//! Mobs riding mobs (`Entity.getPassengers`): where a rider sits on its
//! vehicle (`EntityAttachments`), the rider steering the vehicle toward its
//! target (`Mob.getControllingPassenger`, whose navigation and move control
//! the rider's goals use), and the saved jockey fields. The game links the
//! two (`game/passengers.rs`) and spawns the jockeys.
use super::{
    Mob, MobWorld, Species,
    goals::{JUMP, LOOK, MOVE},
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::{Compound, Tag};

/// The goal flags a controlling passenger takes from its vehicle.
const CONTROLLED: u8 = MOVE | JUMP | LOOK;

/// `EntityType` passenger attachment of a vehicle (before rotation): where a
/// rider's attachment point sits relative to the vehicle's feet.
pub fn passenger_attachment(v: &Mob) -> [f64; 3] {
    match v.species() {
        Species::Chicken => [0.0, 0.7, -0.1],
        Species::Spider => [0.0, 0.765, 0.0],
        Species::Ravager => [0.0, 2.2625, -0.0625],
        Species::Horse => [0.0, 1.44375, 0.0],
        Species::Donkey => [0.0, 1.1125, 0.0],
        Species::Mule => [0.0, 1.2125, 0.0],
        Species::SkeletonHorse | Species::ZombieHorse => [0.0, 1.31875, 0.0],
        Species::Pig => [0.0, 0.86875, 0.0],
        Species::Nautilus | Species::ZombieNautilus => [0.0, 1.1375, 0.0],
        // Camel.getPassengerAttachmentPoint for a lone standing driver.
        Species::Camel | Species::CamelHusk if !super::camel::sitting(v) => {
            [0.0, f64::from(v.height()) - 0.375, 0.5]
        }
        // The default attachments: the top of the bounding box.
        _ => [0.0, f64::from(v.height()), 0.0],
    }
}
/// The rider's own `VEHICLE` attachment height (`ridingOffset`): how far
/// below the vehicle's attachment point its feet hang.
pub fn vehicle_attachment(r: &Mob) -> f64 {
    use Species as S;
    match r.species() {
        S::Zombie | S::Husk | S::Drowned if r.is_baby() => 0.1875,
        S::ZombieVillager if r.is_baby() => 0.125,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::ZombifiedPiglin
        | S::Piglin
        | S::PiglinBrute
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton => 0.7,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner => 0.6,
        _ => 0.0,
    }
}
/// `Vec3.yRot(-yaw)` of an attachment point.
fn rotated([x, y, z]: [f64; 3], yaw: f32) -> [f64; 3] {
    let a = -f64::from(yaw).to_radians();
    let (sin, cos) = (a.sin(), a.cos());
    [x * cos + z * sin, y, z * cos - x * sin]
}
/// `Entity.positionRider`: the vehicle's position plus its rotated passenger
/// attachment, less the rider's own vehicle attachment.
pub fn seat(attachment: [f64; 3], rider_offset: f64, at: Vec3, yaw: f32) -> Vec3 {
    let [x, y, z] = rotated(attachment, yaw);
    Vec3::new(at.x + x, at.y + y - rider_offset, at.z + z)
}

/// Ranged riders keep this far from their target instead of closing in.
const RANGED_DISTANCE: f64 = 8.0;
fn ranged(s: Species) -> bool {
    matches!(
        s,
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::Pillager
            | Species::Witch
            | Species::Piglin
    )
}
/// A mob rider steers: while it has a target, its vehicle gives up its own
/// movement goals and carries it toward the target (melee riders to contact,
/// archers to a shooting distance). Without a target the vehicle wanders.
pub fn drive(m: &mut Mob, w: &MobWorld) {
    let Some(rider) = m.extra.rider else {
        m.ai.goals.disabled &= !CONTROLLED;
        return;
    };
    let seen = w.seen(rider).filter(|s| s.alive);
    let info = seen
        .and_then(|s| s.target)
        .and_then(|t| w.target_info(t))
        .filter(|t| t.alive && !(t.is_player && t.creative));
    let (Some(seen), Some(t)) = (seen, info) else {
        m.ai.goals.disabled &= !CONTROLLED;
        return;
    };
    m.ai.goals.disabled |= CONTROLLED;
    m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
    let close =
        ranged(seen.species) && m.distance_sqr(t.position) < RANGED_DISTANCE * RANGED_DISTANCE;
    if close {
        m.nav_stop();
    } else if m.ai.nav.is_done() || (m.age + m.id).is_multiple_of(10) {
        m.nav_move_to_entity(w, [t.position.x, t.position.y, t.position.z], 0, 1.0);
    }
}
/// `Mob.getControllingPassenger` for a mob on its back: the rider, unless it
/// is in `#non_controlling_rider` (`Entity.canControlVehicle`).
pub fn mob_controlled(m: &Mob, w: &MobWorld) -> bool {
    m.extra.rider.and_then(|r| w.seen(r)).is_some_and(|r| {
        !matches!(
            r.species,
            Species::Slime | Species::MagmaCube | Species::SulfurCube
        )
    })
}
/// `Mob.requiresCustomPersistence`: a passenger never despawns.
pub fn keeps_alive(m: &Mob) -> bool {
    m.extra.mount.is_some()
}

/// `IsChickenJockey`, `SkeletonTrap` and `SkeletonTrapTime`.
pub fn write_nbt(m: &Mob, map: &mut Compound) {
    match m.species() {
        Species::Chicken => {
            map.insert(
                "IsChickenJockey".into(),
                Tag::Byte(i8::from(m.extra.jockey)),
            );
        }
        Species::SkeletonHorse => {
            map.insert("SkeletonTrap".into(), Tag::Byte(i8::from(m.extra.trap)));
            map.insert("SkeletonTrapTime".into(), Tag::Int(m.extra.trap_time));
        }
        _ => {}
    }
}
pub fn read_nbt(m: &mut Mob, tag: &Tag) {
    let int = |key: &str| match tag.get(key) {
        Some(Tag::Int(v)) => *v,
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        _ => 0,
    };
    match m.species() {
        Species::Chicken => m.extra.jockey = int("IsChickenJockey") != 0,
        Species::SkeletonHorse => {
            m.extra.trap = int("SkeletonTrap") != 0;
            m.extra.trap_time = int("SkeletonTrapTime");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_turn_with_the_vehicle() {
        // A ravager's seat sits a sixteenth behind its centre.
        let behind = [0.0, 2.2625, -0.0625];
        let [x, y, z] = rotated(behind, 0.0);
        assert!(x.abs() < 1e-9 && (z + 0.0625).abs() < 1e-9 && y == 2.2625);
        // Facing west (yaw 90), behind is to the east.
        let [x, _, z] = rotated(behind, 90.0);
        assert!((x - 0.0625).abs() < 1e-9 && z.abs() < 1e-9);
    }
    #[test]
    fn archers_keep_their_distance_and_brawlers_close_in() {
        assert!(ranged(Species::Skeleton) && ranged(Species::Pillager));
        assert!(!ranged(Species::Zombie) && !ranged(Species::Vindicator));
    }
}
