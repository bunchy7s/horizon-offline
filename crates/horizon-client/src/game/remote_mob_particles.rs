//! Particles a client of a server draws for a mob's entity event, which the
//! server does not send as a particle packet.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::Species,
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{
    geometry::{
        Vec3,
        trig::{DEG_TO_RAD, Mth},
    },
    random::RandomSource,
};

impl Game<'_> {
    /// `Fox.handleEntityEvent(45)`: eight crumbs of the item in its mouth,
    /// thrown from in front of it along `Entity.getLookAngle`.
    pub(super) fn fox_eat_particles(&mut self, id: u32) {
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == Species::Fox)
        else {
            return;
        };
        let Some(item) = m.extra.main_hand.clone() else {
            return;
        };
        // Entity.calculateViewVector(xRot, yRot): the look's x and z.
        let x_rad = m.head_pitch * DEG_TO_RAD;
        let y_rad = -m.yaw * DEG_TO_RAD;
        let look = [
            f64::from(y_rad.msin() * x_rad.mcos()),
            f64::from(y_rad.mcos() * x_rad.mcos()),
        ];
        let at = [m.position.x, m.position.y, m.position.z];
        let mut spawned = Vec::new();
        for _ in 0..8 {
            let d = Vec3::new(
                (f64::from(m.random.next_float()) - 0.5) * 0.1,
                f64::from(m.random.next_float()) * 0.1 + 0.1,
                0.0,
            )
            .rotate_x(-m.head_pitch * DEG_TO_RAD)
            .rotate_y(-m.yaw * DEG_TO_RAD);
            spawned.push((
                [at[0] + look[0] / 2.0, at[1], at[2] + look[1] / 2.0],
                [d.x, d.y + 0.05, d.z],
            ));
        }
        let options = self.item_particle(&ItemStack::new(&item, 1));
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `Witch.handleEntityEvent(15)`: `nextInt(35) + 10` purple puffs about
    /// half a block over its head; the count is drawn again every pass, as
    /// the Java loop condition does.
    pub(super) fn witch_magic_particles(&mut self, id: u32) {
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == Species::Witch)
        else {
            return;
        };
        let (x, z) = (m.position.x, m.position.z);
        let top = m.bounds().max_position().y;
        let spread = f64::from(0.13_f32);
        let mut spawned = Vec::new();
        let mut i = 0;
        while i < m.random.next_int_bound(35).unwrap_or(0) + 10 {
            spawned.push([
                x + m.random.next_gaussian() * spread,
                top + 0.5 + m.random.next_gaussian() * spread,
                z + m.random.next_gaussian() * spread,
            ]);
            i += 1;
        }
        let options = ParticleOptions::simple(ParticleType::Witch);
        for p in spawned {
            self.particles.add(&options, p, [0.0, 0.0, 0.0]);
        }
    }
    /// `Ravager.handleEntityEvent(69)`: forty puffs of smoke from the middle
    /// of its box, then `applyRoarKnockbackClient`: this client's own player
    /// within 4 blocks is thrown away from it.
    pub(super) fn remote_ravager_roar(&mut self, id: i32) {
        let Some(m) = self
            .mobs
            .iter()
            .find(|m| m.id == id as u32 && m.species() == Species::Ravager)
        else {
            return;
        };
        let (center, at, area) = (
            m.center(),
            m.position,
            m.bounds().inflate(Vec3::new(4.0, 4.0, 4.0)),
        );
        let options = ParticleOptions::simple(ParticleType::Poof);
        for _ in 0..40 {
            let r = self.particles.random();
            let v = [
                r.next_gaussian() * 0.2,
                r.next_gaussian() * 0.2,
                r.next_gaussian() * 0.2,
            ];
            self.particles.add(&options, center, v);
        }
        if let Some(view) = self.player_view()
            && view.alive
            && view.bounds.intersects(area)
            && let Some(p) = self.player.as_mut()
        {
            // strongKnockback.
            let (xd, zd) = (p.position.x - at.x, p.position.z - at.z);
            let dd = (xd * xd + zd * zd).max(0.001);
            p.velocity = p
                .velocity
                .add_vector(Vec3::new(xd / dd * 4.0, 0.2, zd / dd * 4.0));
        }
    }
}
