//! `ItemPickupParticle`: a taken item entity, experience orb, arrow or trident flies
//! into whoever took it over three ticks (`ClientPacketListener.handleTakeItemEntity`
//! adds one for every `ClientboundTakeItemEntityPacket`), drawn with the render
//! state it had when it was taken (`extractEntity(from, 1.0F)`).
use super::Game;
use horizon_client::{
    entities::{ExperienceOrb, ItemEntity},
    projectiles::Projectile,
};

/// `ItemPickupParticle.LIFE_TIME`.
const LIFE_TIME: u32 = 3;

/// The taken entity, as it was when taken.
pub(super) enum Taken {
    Item(Box<ItemEntity>),
    Orb(Box<ExperienceOrb>),
    Arrow(Box<Projectile>),
    Trident(Box<super::trident::ThrownTrident>),
}
impl Taken {
    /// The render state's position: the entity's at partial tick 1.
    fn position(&self) -> [f64; 3] {
        let p = match self {
            Taken::Item(i) => i.position,
            Taken::Orb(o) => o.position,
            Taken::Arrow(a) => a.position,
            Taken::Trident(t) => t.position,
        };
        [p.x, p.y, p.z]
    }
}

/// Whoever took it: the particle's `target`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Collector {
    Player,
    /// A mob, or another player on a server (both kept as mobs here).
    Mob(u32),
    /// A LAN guest, by entity id (the host draws it as a model).
    Guest(u32),
}

pub(super) struct Pickup {
    pub taken: Taken,
    collector: Collector,
    life: u32,
    target: [f64; 3],
    target_old: [f64; 3],
}

/// `ParticleEngine`'s ITEM_PICKUP group and the particles waiting to join it.
#[derive(Default)]
pub(super) struct Pickups {
    pub live: Vec<Pickup>,
    added: Vec<Pickup>,
    /// The particles made so far, by kind of taken entity (for horizon-ctl).
    made: std::collections::BTreeMap<&'static str, u64>,
}
impl Pickups {
    pub fn clear(&mut self) {
        self.live.clear();
        self.added.clear();
    }
    pub fn made(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.made
    }
}

/// `ItemPickupParticleGroup.ParticleInstance.fromParticle`: the position at
/// `partial`, from the taken entity's to the collector's (lerped between ticks)
/// along the square of the particle's age.
pub(super) fn position(p: &Pickup, partial: f64) -> [f64; 3] {
    let time = (p.life as f32 + partial as f32) / LIFE_TIME as f32;
    let time = f64::from(time * time);
    let from = p.taken.position();
    std::array::from_fn(|i| {
        let target = p.target_old[i] + partial * (p.target[i] - p.target_old[i]);
        from[i] + time * (target - from[i])
    })
}

/// A LAN guest's target, from the model the host draws for it.
pub(super) fn guest_target(host: &super::lan::LanHost, id: u32) -> Option<[f64; 3]> {
    let m = host.models.values().find(|m| m.id == id)?;
    let (p, eye) = (m.position, m.eye_height());
    Some([p.x, (p.y + (p.y + eye)) / 2.0, p.z])
}

impl Game<'_> {
    /// `ItemPickupParticle.updatePosition`: the collector's x and z, halfway up to its eyes.
    fn collector_target(&self, collector: Collector) -> Option<[f64; 3]> {
        let (p, eye) = match collector {
            Collector::Player => {
                let p = self.player.as_ref()?;
                (p.position, f64::from(p.dimensions.eye_height))
            }
            Collector::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                (m.position, m.eye_height())
            }
            Collector::Guest(id) => {
                return guest_target(self.lan.as_ref()?, id);
            }
        };
        Some([p.x, (p.y + (p.y + eye)) / 2.0, p.z])
    }
    /// `handleTakeItemEntity`: `particleEngine.add(new ItemPickupParticle(...))` when the
    /// collector is known; it joins the group at the end of the next particle tick.
    pub(super) fn add_pickup(&mut self, taken: Taken, collector: Collector) {
        if let Some(target) = self.collector_target(collector) {
            self.add_pickup_toward(taken, collector, target);
        }
    }
    /// [`Self::add_pickup`] with the collector's first target given (a LAN
    /// guest's model while the host is taken out for its tick).
    pub(super) fn add_pickup_toward(
        &mut self,
        taken: Taken,
        collector: Collector,
        target: [f64; 3],
    ) {
        let kind = match taken {
            Taken::Item(_) => "item",
            Taken::Orb(_) => "orb",
            Taken::Arrow(_) => "arrow",
            Taken::Trident(_) => "trident",
        };
        *self.pickups.made.entry(kind).or_default() += 1;
        self.pickups.added.push(Pickup {
            taken,
            collector,
            life: 0,
            target,
            target_old: target,
        });
    }
    /// `ParticleEngine.tick` for the ITEM_PICKUP group: age (removed at
    /// `LIFE_TIME`), follow the collector, then let the new particles in.
    pub(super) fn tick_pickups(&mut self) {
        let mut live = std::mem::take(&mut self.pickups.live);
        live.retain_mut(|p| {
            p.life += 1;
            if p.life == LIFE_TIME {
                return false;
            }
            p.target_old = p.target;
            if let Some(t) = self.collector_target(p.collector) {
                p.target = t;
            }
            true
        });
        live.append(&mut self.pickups.added);
        self.pickups.live = live;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orb_at(x: f64, y: f64, z: f64) -> Taken {
        let at = horizon_core::geometry::Vec3::new(x, y, z);
        Taken::Orb(Box::new(ExperienceOrb {
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: horizon_core::geometry::Vec3::new(0.0, 0.0, 0.0),
            value: 1,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            health: 5,
        }))
    }

    #[test]
    fn flies_along_the_square_of_its_age_to_the_moving_target() {
        let mut p = Pickup {
            taken: orb_at(0.0, 64.0, 0.0),
            collector: Collector::Player,
            life: 0,
            target: [3.0, 65.0, 0.0],
            target_old: [3.0, 65.0, 0.0],
        };
        // Added this tick: still at the orb.
        assert_eq!(position(&p, 0.0), [0.0, 64.0, 0.0]);
        // Halfway through the first tick: (0.5 / 3)^2 of the way.
        let t = (0.5f32 / 3.0) * (0.5 / 3.0);
        let mid = position(&p, 0.5);
        assert!((mid[0] - 3.0 * f64::from(t)).abs() < 1e-9, "{mid:?}");
        // Last tick before removal, the target moved: lerped between its ticks.
        p.life = 2;
        p.target_old = [3.0, 65.0, 0.0];
        p.target = [4.0, 65.0, 0.0];
        let end = position(&p, 1.0);
        assert!(
            (end[0] - 4.0).abs() < 1e-9 && (end[1] - 65.0).abs() < 1e-9,
            "{end:?}"
        );
    }
}
