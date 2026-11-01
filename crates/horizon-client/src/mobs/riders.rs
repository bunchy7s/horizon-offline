//! Mobs riding mobs (`Entity.getPassengers`): where a rider sits on its
//! vehicle (`EntityAttachments`), the rider steering the vehicle toward its
//! target (`Mob.getControllingPassenger`, whose navigation and move control
//! the rider's goals use), and the saved jockey fields. The game links the
//! two (`game/passengers.rs`) and spawns the jockeys.
use super::{
    Mob, MobWorld, Seen, Species,
    goals::{JUMP, LOOK, MOVE},
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::nbt::{Compound, Tag};

/// The goal flags a controlling passenger takes from its vehicle.
const CONTROLLED: u8 = MOVE | JUMP | LOOK;
/// `Entity.removePassenger`: a passenger that got off cannot board for 60 ticks.
const BOARDING_COOLDOWN: i32 = 60;
/// `Piglin.startRiding`: how many baby piglins stack on one hoglin.
const MAX_PASSENGERS_ON_ONE_HOGLIN: i32 = 3;

/// `LivingEntity.getScale`: the `minecraft:scale` the server sent on any
/// living entity (`extra.model_scale`, 0 when none came), 1 when unset.
fn scale(m: &Mob) -> f64 {
    if m.extra.model_scale > 0.0 {
        f64::from(m.extra.model_scale)
    } else {
        1.0
    }
}
/// Players and mannequins (`Avatar`).
fn avatar(m: &Mob) -> bool {
    matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin")
}
/// `EntityType` passenger attachment of a vehicle (before rotation): where a
/// rider's attachment point sits relative to the vehicle's feet, in its
/// scale (`getDimensions` scales the attachments with the box).
pub fn passenger_attachment(v: &Mob) -> [f64; 3] {
    let [x, y, z] = base_attachment(v);
    let s = scale(v);
    // AbstractHorse.getPassengerAttachmentPoint: a rearing horse slides it,
    // after the scaled attachment (`rear_offset` scales with age itself).
    let rear = super::equine::rear_offset(v);
    [x * s + rear[0], y * s + rear[1], z * s + rear[2]]
}
fn base_attachment(v: &Mob) -> [f64; 3] {
    match v.species() {
        Species::Chicken => [0.0, 0.7, -0.1],
        Species::Spider => [0.0, 0.765, 0.0],
        Species::Ravager => [0.0, 2.2625, -0.0625],
        Species::Horse => [0.0, 1.44375, 0.0],
        Species::Donkey => [0.0, 1.1125, 0.0],
        Species::Mule => [0.0, 1.2125, 0.0],
        Species::SkeletonHorse | Species::ZombieHorse => [0.0, 1.31875, 0.0],
        Species::Pig => [0.0, 0.86875, 0.0],
        // `Hoglin.BABY_DIMENSIONS` and `EntityType.HOGLIN`.
        Species::Hoglin if v.is_baby() => [0.0, 0.875, 0.0],
        Species::Hoglin => [0.0, 1.49375, 0.0],
        Species::Nautilus | Species::ZombieNautilus => [0.0, 1.1375, 0.0],
        Species::Camel | Species::CamelHusk => super::camel::passenger_attachment(v),
        // The default attachments: the top of the bounding box, before
        // `passenger_attachment`'s scale — a player's or mannequin's
        // `height()` carries `minecraft:scale` since `getDimensions` grew it;
        // any other mob's `height()` stays unscaled (`awake_dims_in`).
        _ if avatar(v) => [0.0, f64::from(v.height()) / scale(v), 0.0],
        _ => [0.0, f64::from(v.height()), 0.0],
    }
}
/// The rider's own `VEHICLE` attachment height (`ridingOffset`): how far
/// below the vehicle's attachment point its feet hang, in its scale.
pub fn vehicle_attachment(r: &Mob) -> f64 {
    unscaled_vehicle_attachment(r) * scale(r)
}
fn unscaled_vehicle_attachment(r: &Mob) -> f64 {
    use Species as S;
    // Avatar.DEFAULT_VEHICLE_ATTACHMENT.
    if avatar(r) {
        return 0.6;
    }
    match r.species() {
        S::Zombie | S::Husk | S::Drowned if r.is_baby() => 0.1875,
        S::ZombieVillager if r.is_baby() => 0.125,
        // `Piglin.BABY_DIMENSIONS`' vehicle attachment.
        S::Piglin if r.is_baby() => 0.1875,
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

/// `Entity.startRiding`: the link it makes, `rider` on `vehicle` as its first
/// passenger (`addPassenger`).
pub fn link(mobs: &mut [Mob], rider: u32, vehicle: u32) {
    if rider == vehicle {
        return;
    }
    if let Some(m) = mobs.iter_mut().find(|m| m.id == rider) {
        m.extra.mount = Some(vehicle);
        m.extra.passenger = true;
    }
    if let Some(m) = mobs.iter_mut().find(|m| m.id == vehicle) {
        m.extra.rider = Some(rider);
    }
}
/// `Piglin.getTopPassenger`: down the chain of first passengers from
/// `vehicle`, stopping at the `counter`th seat.
fn top_passenger(mobs: &[Mob], vehicle: u32, counter: i32) -> u32 {
    let first = mobs
        .iter()
        .find(|m| m.id == vehicle)
        .and_then(|m| m.extra.rider);
    match first {
        Some(passenger) if counter != 1 => top_passenger(mobs, passenger, counter - 1),
        _ => vehicle,
    }
}
/// `Piglin.startRiding` over `Entity.startRiding` for two mobs: a baby piglin
/// that boards a hoglin sits on the top of up to three piglins stacked on it.
/// Not the vehicle it is on already, nor one it carries itself, and only
/// after its boarding cooldown, onto a vehicle with no passenger
/// (`canRide`, `canAddPassenger`). Returns whether it boarded.
pub fn start_riding(mobs: &mut [Mob], rider: u32, vehicle: u32) -> bool {
    let Some(r) = mobs.iter().find(|m| m.id == rider) else {
        return false;
    };
    let on_hoglin = mobs
        .iter()
        .any(|m| m.id == vehicle && m.species() == Species::Hoglin);
    let vehicle = if r.species() == Species::Piglin && r.is_baby() && on_hoglin {
        top_passenger(mobs, vehicle, MAX_PASSENGERS_ON_ONE_HOGLIN)
    } else {
        vehicle
    };
    if rider == vehicle || r.extra.mount == Some(vehicle) {
        return false;
    }
    // Nothing that rides the rider (through any number of vehicles) can be
    // its vehicle.
    let mut up = Some(vehicle);
    while let Some(v) = up {
        up = mobs.iter().find(|m| m.id == v).and_then(|m| m.extra.mount);
        if up == Some(rider) {
            return false;
        }
    }
    let Some(v) = mobs.iter().find(|m| m.id == vehicle) else {
        return false;
    };
    if r.extra.boarding_cooldown > 0 || v.extra.rider.is_some() || v.rider.is_some() {
        return false;
    }
    link(mobs, rider, vehicle);
    true
}
/// `LivingEntity.stopRiding` for the mob itself: it leaves its vehicle (which
/// forgets it when the riders are next synced), cannot board again for 60
/// ticks (`Entity.removePassenger`) and is set down at `at`, where
/// `dismountVehicle` puts it (`dismountTo`), or stays where it is.
pub fn stop_riding(m: &mut Mob, at: Option<Vec3>) {
    if m.extra.mount.take().is_none() {
        return;
    }
    m.extra.passenger = false;
    m.extra.boarding_cooldown = BOARDING_COOLDOWN;
    if let Some(at) = at {
        m.position = at;
        m.previous = at;
    }
}
/// `Animal.getDismountLocationForPassenger` for a standing passenger of
/// `width` by `height`: on the floor of the first block around the
/// vehicle's, going right, left, behind right, behind left, ahead right,
/// ahead left, behind and ahead of its facing, where the passenger's box
/// fits (`DismountHelper.canDismountTo`, without the world border); else on
/// top of the vehicle. A piglin is no `Animal`: it keeps
/// `Entity.getDismountLocationForPassenger`, on top of it.
pub fn dismount_location(w: &MobWorld, vehicle: &Seen, width: f32, height: f32) -> Vec3 {
    // `Entity.getDismountLocationForPassenger`.
    let on_top = Vec3::new(
        vehicle.position.x,
        vehicle.bounds.max_position().y,
        vehicle.position.z,
    );
    if vehicle.species == Species::Piglin {
        return on_top;
    }
    let (bx, by, bz) = (
        vehicle.position.x.floor() as i32,
        vehicle.position.y.floor() as i32,
        vehicle.position.z.floor() as i32,
    );
    // `Direction.fromYRot` of the vehicle's yaw: south, west, north, east.
    let facing = (f64::from(vehicle.yaw) / 90.0 + 0.5).floor() as i32;
    let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(facing & 3) as usize];
    // `DismountHelper.offsetsForDirection`; `right` is `Direction.getClockWise`.
    let (rx, rz) = (-fz, fx);
    let offsets = [
        (rx, rz),
        (-rx, -rz),
        (rx - fx, rz - fz),
        (-rx - fx, -rz - fz),
        (fx + rx, fz + rz),
        (fx - rx, fz - rz),
        (-fx, -fz),
        (fx, fz),
    ];
    // `EntityDimensions.makeBoundingBox`: the half width is a float.
    let half = f64::from(width / 2.0);
    for (dx, dz) in offsets {
        let (x, z) = (bx + dx, bz + dz);
        let floor = w.block_floor_height(x, by, z);
        // `DismountHelper.isBlockFloorValid`.
        if floor.is_infinite() || floor >= 1.0 {
            continue;
        }
        // `Vec3.upFromBottomCenterOf`.
        let at = Vec3::new(
            f64::from(x) + 0.5,
            f64::from(by) + floor,
            f64::from(z) + 0.5,
        );
        let bounds = Aabb::new(
            Vec3::new(at.x - half, at.y, at.z - half),
            Vec3::new(at.x + half, at.y + f64::from(height), at.z + half),
        );
        if w.no_collision(bounds) {
            return at;
        }
    }
    on_top
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
    fn npc(scale: f32) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:player".into();
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.model_scale = scale;
        m
    }
    /// An NPC plugin's enlarged fake player: the name it carries sits on the
    /// top of its scaled box, and on a seat it hangs `Avatar`'s 0.6 lower, scaled too.
    #[test]
    fn a_scaled_player_carries_and_sits_in_its_scale() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(close(passenger_attachment(&npc(0.0))[1], 1.8));
        assert!(close(passenger_attachment(&npc(1.5))[1], 2.7));
        assert!(close(vehicle_attachment(&npc(0.0)), 0.6));
        assert!(close(vehicle_attachment(&npc(1.5)), 0.9));
        // On a seat at y 55 (a text display, whose attachment is its feet):
        // a 1.5 NPC's feet at 54.1, where the server puts it.
        let seat = seat(
            [0.0; 3],
            vehicle_attachment(&npc(1.5)),
            Vec3::new(0.0, 55.0, 0.0),
            0.0,
        );
        assert!(close(seat.y, 54.1));
    }
    /// A scaled vehicle that is not an avatar: `remote_mob_attributes` puts
    /// `minecraft:scale` on any living entity, but `awake_dims_in` grows only
    /// avatars' boxes, so the seat still rides at the scaled box height.
    #[test]
    fn a_scaled_vehicle_seat_stays_scaled() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        let mut cow = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        assert!(close(passenger_attachment(&cow)[1], 1.4));
        cow.extra.model_scale = 2.0;
        assert!(close(passenger_attachment(&cow)[1], 2.8));
    }
    #[test]
    fn archers_keep_their_distance_and_brawlers_close_in() {
        assert!(ranged(Species::Skeleton) && ranged(Species::Pillager));
        assert!(!ranged(Species::Zombie) && !ranged(Species::Vindicator));
    }
}
