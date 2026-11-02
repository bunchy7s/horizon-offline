//! Boats and the entities around them: what a boat runs into
//! (`AbstractBoat.canVehicleCollide`), the pushes between a boat and what
//! touches it (`AbstractBoat.push`, `LivingEntity.pushEntities`), the mobs
//! it picks up (the end of `AbstractBoat.tick`), where they sit
//! (`positionRider`, `clampRotation`) and how they get out.
//! Saving them under the boat's `Passengers` lives in `persist.rs`.
use super::{Game, boats::Boat, lan_mobs::guest_bounds};
use horizon_client::{
    camera::wrap_degrees,
    mobs::{Mob, Species, Who, drops::is_animal, equine, riders},
    scoreboard::pushable_by,
};
use horizon_core::geometry::{Aabb, Vec3};

/// `Entity.boardingCooldown` once a mob leaves its boat.
pub(super) const BOARDING_COOLDOWN: i32 = 60;
/// `Player.DEFAULT_VEHICLE_ATTACHMENT`: how far below the seat a player's
/// feet are.
const PLAYER_RIDING_OFFSET: f64 = 0.6;

/// `Entity.isVehicle`: a player or a mob rides it.
fn carries(m: &Mob) -> bool {
    m.rider.is_some() || m.extra.rider.is_some()
}
/// `isPushable`, as `pushableBy` and `canVehicleCollide` ask it:
/// `Mob::pushable` plus the shulkers it leaves out (they take no push, but
/// count), less a ridden horse (`AbstractHorse.isPushable`).
fn pushable(m: &Mob) -> bool {
    !m.removed
        && (m.pushable() || m.species() == Species::Shulker && m.alive())
        && !(equine::is_equine(m.species()) && carries(m))
}
/// Whether `Entity.push` moves the mob: it is pushable and carries no one.
fn moved_by_push(m: &Mob) -> bool {
    !m.removed && m.pushable() && !carries(m)
}
/// `Entity.push(Entity)`: the push `other` gets from `this` (which takes the
/// opposite one), or None when they stand on nearly the same spot.
pub(super) fn push_impulse(this: Vec3, other: Vec3) -> Option<Vec3> {
    let (mut xa, mut za) = (other.x - this.x, other.z - this.z);
    let mut dd = xa.abs().max(za.abs());
    if dd < f64::from(0.01_f32) {
        return None;
    }
    dd = dd.sqrt();
    xa /= dd;
    za /= dd;
    let pow = (1.0 / dd).min(1.0);
    let k = f64::from(0.05_f32);
    Some(Vec3::new(xa * pow * k, 0.0, za * pow * k))
}
/// `AbstractBoat.push`: another boat is pushed while it is lower than this
/// boat's top, anything else while its feet are no higher than the boat's.
pub(super) fn boat_pushes(boat: Aabb, other: Aabb, other_is_boat: bool) -> bool {
    if other_is_boat {
        other.min_position().y < boat.max_position().y
    } else {
        other.min_position().y <= boat.min_position().y
    }
}

impl Game<'_> {
    /// The players in boat `id`: the window's player (or the server player
    /// acting as it) and the server's other players.
    pub(super) fn boat_players(&self, id: u32) -> usize {
        let guests = self.lan.as_ref().map_or(0, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| Some(g.conn) != self.lan_acting && g.vehicle == Some(id))
                .count()
        });
        usize::from(self.riding == Some(id)) + guests
    }
    /// The mobs in boat `id`, in their seat order after the players
    /// (`Entity.addPassenger` puts a player first).
    pub(super) fn boat_mobs(&self, id: u32) -> Vec<usize> {
        (0..self.mobs.len())
            .filter(|&j| {
                let m = &self.mobs[j];
                m.extra.mount == Some(id) && !m.removed && m.alive()
            })
            .collect()
    }
    /// `getPassengers().size()` of boat `id`.
    pub(super) fn boat_passengers(&self, id: u32) -> usize {
        self.boat_players(id) + self.boat_mobs(id).len()
    }
    /// `AbstractBoat.canAddPassenger`: a seat is free and the boat is not
    /// under water.
    pub(super) fn boat_has_room(&self, i: usize) -> bool {
        let b = &self.boats[i];
        self.boat_passengers(b.id) < b.max_passengers() && !b.under_water()
    }
    /// Where the player in boat `i` sits: in the middle alone, in front of a
    /// mob.
    pub(super) fn player_boat_seat(&self, i: usize) -> Vec3 {
        let b = &self.boats[i];
        let count = self.boat_players(b.id).max(1) + self.boat_mobs(b.id).len();
        b.seat(0, count, false, PLAYER_RIDING_OFFSET, false)
    }
    /// `getRootVehicle` of mob `j`: the mob or boat at the bottom of what it
    /// rides, None when it rides nothing.
    fn mob_root_vehicle(&self, mut j: usize) -> Option<u32> {
        let mut root = None;
        for _ in 0..8 {
            let Some(v) = self.mobs[j].extra.mount else {
                break;
            };
            root = Some(v);
            match self.mobs.iter().position(|m| m.id == v) {
                Some(k) => j = k,
                None => break,
            }
        }
        root
    }
    /// The players outside boat `id` that push and block it (living, not
    /// spectators), with the window player's team.
    fn players_outside(&self, id: u32) -> Vec<(Aabb, bool)> {
        let mut out = Vec::new();
        if let Some(p) = self.player.as_ref()
            && self.riding != Some(id)
            && !self.spectator()
            && !p.health.dead_or_dying()
        {
            out.push((p.bounds(), true));
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                Some(g.conn) != self.lan_acting
                    && host.hub.here(g)
                    && g.loaded
                    && !g.dead
                    && !g.spectator()
                    && g.vehicle != Some(id)
            }) {
                out.push((guest_bounds(g), false));
            }
        }
        out
    }
    /// `getEntityCollisions` of boat `id` within `area`, with
    /// `AbstractBoat.canVehicleCollide`: the other boats, minecarts,
    /// shulkers and pushable mobs and players, but not its own passengers.
    pub(super) fn boat_obstacles(&self, id: u32, area: Aabb) -> Vec<Aabb> {
        let mut out: Vec<Aabb> = (0..self.mobs.len())
            .filter(|&j| {
                let m = &self.mobs[j];
                pushable(m)
                    && m.bounds().intersects(area)
                    && (m.extra.mount.is_none() || self.mob_root_vehicle(j) != Some(id))
            })
            .map(|j| self.mobs[j].bounds())
            .collect();
        out.extend(
            self.boats
                .iter()
                .filter(|b| b.id != id)
                .map(Boat::bounds)
                .chain(self.carts.iter().map(|c| c.bounds()))
                .chain(self.players_outside(id).into_iter().map(|(bb, _)| bb))
                .filter(|bb| bb.intersects(area)),
        );
        out
    }
    /// What boat `b` may run into this tick.
    pub(super) fn boat_move_obstacles(&self, b: &Boat) -> Vec<Aabb> {
        let reach = b.velocity.length() + 1.0;
        self.boat_obstacles(b.id, b.bounds().inflate(Vec3::new(reach, reach, reach)))
    }
    /// `noEntityCollision(boat, box)` for a new boat (`BoatItem.use`).
    pub(super) fn boat_entity_collision(&self, bb: Aabb) -> bool {
        // EntityGetter.getEntityCollisions: the box grown by 1e-7.
        let area = bb.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        !self.boat_obstacles(u32::MAX, area).is_empty()
    }

    /// The end of `AbstractBoat.tick` on the server for boat `i`: each
    /// entity it touches gets in (a mob, while no player steers) or is
    /// pushed apart from it, and `LivingEntity.pushEntities` of each mob and
    /// player touching it pushes the two apart once more. Only the window's
    /// own player is moved here: a server's players are pushed by their
    /// clients, which tick the boats near them (`remote_pushes`).
    pub(super) fn boat_contacts(&mut self, i: usize) {
        let (id, bb, max) = {
            let b = &self.boats[i];
            (b.id, b.bounds(), b.max_passengers())
        };
        let area = bb.inflate(Vec3::new(
            f64::from(0.2_f32),
            f64::from(-0.01_f32),
            f64::from(0.2_f32),
        ));
        let mut count = self.boat_passengers(id);
        // getControllingPassenger is a player: one rides it (players sit first).
        let pick_up = self.boat_players(id) == 0;
        // canAddPassenger, which startRiding asks after the checks below.
        let room = !self.boats[i].under_water();
        // (mob, touched by the boat's sweep, its own pushEntities reaches the boat)
        let teams = self.mob_teams();
        let mobs: Vec<(u32, bool, bool)> = (0..self.mobs.len())
            .filter_map(|j| {
                let m = &self.mobs[j];
                if !pushable(m) || m.extra.mount.is_some() && self.mob_root_vehicle(j) == Some(id) {
                    return None;
                }
                let mb = m.bounds();
                let team = teams.of(Who::Mob(m.id));
                let touched = mb.intersects(area) && pushable_by(None, team, true);
                // Bats push nothing (Bat.pushEntities).
                let pushes = m.species() != Species::Bat
                    && mb.intersects(bb)
                    && pushable_by(team, None, true);
                (touched || pushes).then_some((m.id, touched, pushes))
            })
            .collect();
        let player_team = teams.of(Who::Player);
        let players: Vec<(Aabb, bool, bool, bool)> = self
            .players_outside(id)
            .into_iter()
            .map(|(pb, own)| {
                let team = if own { player_team } else { None };
                (
                    pb,
                    own,
                    pb.intersects(area) && pushable_by(None, team, true),
                    pb.intersects(bb) && pushable_by(team, None, true),
                )
            })
            .collect();
        drop(teams);
        for (mob, touched, pushes) in mobs {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            if touched {
                let m = &self.mobs[j];
                let boards = pick_up
                    && count < max
                    && m.extra.mount.is_none()
                    && m.width() < super::boats::WIDTH as f32
                    && !self
                        .res
                        .mob_data()
                        .entity_has("minecraft:cannot_be_pushed_onto_boats", &m.kind.id);
                if boards {
                    // startRiding fails quietly (no push) while the mob's
                    // boarding cooldown runs (Entity.canRide) or the boat
                    // is under water, or when a mod cancels it
                    // (EntityMountEvent).
                    let ready = m.extra.boarding_cooldown <= 0;
                    if room
                        && ready
                        && self.native_mob_may_ride(mob, id, true)
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob)
                    {
                        m.extra.mount = Some(id);
                        m.extra.passenger = true;
                        m.ai.nav.stop();
                        count += 1;
                        // Mob.startRiding drops its lead.
                        self.break_leash(mob, true, false);
                    }
                    continue;
                }
                self.boat_push_mob(i, j, count);
            }
            if pushes {
                self.boat_push_mob(i, j, count);
            }
        }
        for (pb, own, touched, pushes) in players {
            let Some(v) = push_impulse(self.boats[i].position, pb.center())
                .filter(|_| boat_pushes(bb, pb, false))
            else {
                continue;
            };
            for _ in 0..usize::from(touched) + usize::from(pushes) {
                if count == 0 {
                    let b = &mut self.boats[i];
                    b.velocity = b.velocity.subtract(v);
                }
                if own && let Some(p) = self.player.as_mut() {
                    p.velocity = p.velocity.add_vector(v);
                }
            }
        }
        for k in 0..self.boats.len() {
            let ob = self.boats[k].bounds();
            if k == i || !ob.intersects(area) || !boat_pushes(bb, ob, true) {
                continue;
            }
            let Some(v) = push_impulse(self.boats[i].position, self.boats[k].position) else {
                continue;
            };
            if count == 0 {
                self.boats[i].velocity = self.boats[i].velocity.subtract(v);
            }
            if self.boat_passengers(self.boats[k].id) == 0 {
                self.boats[k].velocity = self.boats[k].velocity.add_vector(v);
            }
        }
        let guest_carts = self.guest_vehicles();
        for c in 0..self.carts.len() {
            let cb = self.carts[c].bounds();
            if !cb.intersects(area) || !boat_pushes(bb, cb, false) {
                continue;
            }
            let Some(v) = push_impulse(self.boats[i].position, self.carts[c].position) else {
                continue;
            };
            if count == 0 {
                self.boats[i].velocity = self.boats[i].velocity.subtract(v);
            }
            let cid = self.carts[c].id;
            let ridden = self.riding == Some(cid)
                || guest_carts.contains(&cid)
                || self.mobs.iter().any(|m| m.extra.mount == Some(cid));
            if !ridden {
                self.carts[c].velocity = self.carts[c].velocity.add_vector(v);
            }
        }
    }
    /// `AbstractBoat.push(mob)` from boat `i`, which has `count` passengers.
    fn boat_push_mob(&mut self, i: usize, j: usize, count: usize) {
        let (b, m) = (&self.boats[i], &self.mobs[j]);
        if !boat_pushes(b.bounds(), m.bounds(), false) {
            return;
        }
        let Some(v) = push_impulse(b.position, m.position) else {
            return;
        };
        if count == 0 {
            let b = &mut self.boats[i];
            b.velocity = b.velocity.subtract(v);
        }
        let m = &mut self.mobs[j];
        if moved_by_push(m) {
            m.velocity = m.velocity.add_vector(v);
            m.needs_sync = true;
        }
    }

    /// `positionRider` and `clampRotation` for the mobs in boats: each sits
    /// in its seat facing the way the boat does, an animal in a full boat
    /// sideways, its head kept within its reach of its body.
    pub(super) fn seat_boat_mobs(&mut self) {
        for i in 0..self.boats.len() {
            let id = self.boats[i].id;
            let mobs = self.boat_mobs(id);
            if mobs.is_empty() {
                continue;
            }
            let players = self.boat_players(id);
            let count = players + mobs.len();
            let b = &self.boats[i];
            let full = count == b.max_passengers();
            let delta = b.delta_rotation;
            let seats: Vec<_> = mobs
                .iter()
                .enumerate()
                .map(|(n, &j)| {
                    let m = &self.mobs[j];
                    let animal = is_animal(m.species());
                    // Another player's too, on a server's client.
                    let offset = riders::vehicle_attachment(m);
                    let index = players + n;
                    // calculatePassengerBodyYRot.
                    let turn = match (animal && full, m.id.is_multiple_of(2)) {
                        (false, _) => 0.0,
                        (true, true) => 90.0,
                        (true, false) => 270.0,
                    };
                    let turns = !self
                        .res
                        .mob_data()
                        .entity_has("minecraft:can_turn_in_boats", &m.kind.id);
                    (
                        j,
                        b.seat(index, count, animal, offset, false),
                        b.seat(index, count, animal, offset, true),
                        turns.then_some((b.yaw + turn, b.previous_yaw + turn)),
                    )
                })
                .collect();
            for (j, at, previous, facing) in seats {
                let m = &mut self.mobs[j];
                m.position = at;
                m.previous = previous;
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
                let Some((body, previous_body)) = facing else {
                    continue;
                };
                m.head_yaw += delta;
                m.body_yaw = body;
                m.previous_body_yaw = previous_body;
                m.yaw = body;
                m.previous_yaw = previous_body;
                // Mob.clampHeadRotationToBody.
                let limit = m.kind.max_head_y_rot;
                let d = wrap_degrees(body - m.head_yaw);
                m.head_yaw += d - d.clamp(-limit, limit);
            }
        }
    }
    /// `ejectPassengers` for the mobs in boat `i`: each gets out where
    /// `getDismountLocationForPassenger` puts it (beside the boat when it
    /// broke: `LivingEntity.dismountVehicle`) and can't board for a while.
    pub(super) fn eject_boat_mobs(&mut self, i: usize, broken: bool) {
        let b = &self.boats[i];
        let (id, y) = (b.id, b.position.y);
        for j in self.boat_mobs(id) {
            let m = &self.mobs[j];
            let at = if broken {
                Vec3::new(m.position.x, m.position.y.max(y), m.position.z)
            } else {
                self.boat_dismount_target(i, f64::from(m.width()), m.yaw)
            };
            let m = &mut self.mobs[j];
            m.extra.mount = None;
            m.extra.passenger = false;
            m.extra.boarding_cooldown = BOARDING_COOLDOWN;
            m.position = at;
            m.previous = at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_is_capped_and_scaled() {
        // One block apart on x: the full 0.05 along x.
        let v = push_impulse(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!((v.x - f64::from(0.05_f32)).abs() < 1e-12 && v.z == 0.0);
        // Almost on the same spot: no push.
        assert!(push_impulse(Vec3::ZERO, Vec3::new(0.005, 0.0, 0.0)).is_none());
    }

    #[test]
    fn boats_push_by_height() {
        let boat = Aabb::new(Vec3::new(0.0, 64.0, 0.0), Vec3::new(1.0, 64.5625, 1.0));
        let at = |y: f64| Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(1.0, y + 1.0, 1.0));
        assert!(boat_pushes(boat, at(64.0), false));
        assert!(!boat_pushes(boat, at(64.1), false));
        assert!(boat_pushes(boat, at(64.5), true));
        assert!(!boat_pushes(boat, at(64.5625), true));
    }
}
