//! Boats and minecarts for a server's players: LAN guests, players on a
//! dedicated server, and single player's window on its server thread.
//!
//! A player aims at a vehicle, item frame, armor stand or painting as at a
//! mob (`ServerGamePacketListenerImpl.handleInteract`), and the game's own
//! code for it runs as that player (`AbstractBoat.interact` seats it,
//! `VehicleEntity.hurtServer` breaks the vehicle). A boat is driven by its
//! rider's client (`handleMoveVehicle`, `handlePaddleBoat`); a minecart moves
//! here with the rider's input (`ServerPlayer.getLastClientMoveIntent`).
//! Sneaking gets out (`Player.rideTick`: `wantsToStopRiding`), and the client
//! is teleported where the player lands (`ServerPlayer.dismountTo`).
use super::{Game, Keys, lan::LanHost, minecarts::SEAT};
use horizon_client::collisions::SceneCollisions;
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    text::translatable,
};

/// `PlayerInput` flags, as the server player's client last sent them.
pub(super) fn input_keys(flags: u8) -> Keys {
    Keys {
        forward: flags & 1 != 0,
        backward: flags & 2 != 0,
        left: flags & 4 != 0,
        right: flags & 8 != 0,
        jump: flags & 16 != 0,
        shift: flags & 32 != 0,
        sprint: flags & 64 != 0,
        ..Keys::default()
    }
}

fn corners(b: Aabb) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = (b.min_position(), b.max_position());
    ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
}

impl Game<'_> {
    /// The box of a boat, minecart, armor stand, item frame or painting, and
    /// the point an interact packet's location is measured from (the
    /// client's `remote_object_hit` uses the same points).
    pub(super) fn object_target(&self, id: i32) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        let id = u32::try_from(id).ok()?;
        let at = |v: Vec3| [v.x, v.y, v.z];
        let centre = |p: [i32; 3]| p.map(|v| f64::from(v) + 0.5);
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            let (lo, hi) = corners(b.bounds());
            return Some((lo, hi, at(b.position)));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(c.bounds());
            return Some((lo, hi, at(c.position)));
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == id) {
            let (lo, hi) = corners(a.bounds());
            return Some((lo, hi, at(a.position)));
        }
        if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(Aabb::new(
                Vec3::new(c.position.x - 0.5, c.position.y, c.position.z - 0.5),
                Vec3::new(c.position.x + 0.5, c.position.y + 0.25, c.position.z + 0.5),
            ));
            return Some((lo, hi, at(c.position)));
        }
        if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            let (lo, hi) = f.bounds();
            return Some((lo, hi, centre(f.pos)));
        }
        let p = self.paintings.iter().find(|p| p.id == id)?;
        let (lo, hi) = p.bounds();
        Some((lo, hi, centre(p.pos)))
    }

    /// The vehicles the server's players ride: they move with their riders,
    /// not in `tick_boats` and `tick_carts`.
    pub(super) fn guest_vehicles(&self) -> Vec<u32> {
        self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub.guests.iter().filter_map(|g| g.vehicle).collect()
        })
    }

    /// Whether a server player other than the one acting rides `vehicle`
    /// (`Entity.canAddPassenger`: a boat or minecart seats one player here).
    pub(super) fn ridden_by_guest(&self, vehicle: u32) -> bool {
        self.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| Some(g.conn) != self.lan_acting && g.vehicle == Some(vehicle))
        })
    }
    /// Whether a server player rides `vehicle` (`Entity.isPassenger`: a
    /// passenger does not use a portal on its own, the vehicle carries it
    /// across, `Entity.teleportCrossDimension`).
    pub(super) fn guest_rides(&self, vehicle: u32) -> bool {
        self.lan
            .as_ref()
            .is_some_and(|h| h.hub.guests.iter().any(|g| g.vehicle == Some(vehicle)))
    }

    /// `Player.rideTick` for a server player, before its moves this tick:
    /// it stays in its seat, steers its minecart, or gets out.
    pub(super) fn guest_vehicle_tick(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        let Some(vehicle) = g.vehicle else {
            return;
        };
        let (keys, dead) = (input_keys(g.input), g.dead || g.health <= 0.0);
        let boat = self.boats.iter().position(|b| b.id == vehicle);
        let cart = self.carts.iter().any(|c| c.id == vehicle);
        let mount = self.mobs.iter().any(|m| m.id == vehicle);
        let cushion = self.cushions.iter().any(|c| c.id == vehicle);
        // The vehicle is gone (broken, in another level) or the rider died:
        // its client drops it with the entity or the death screen.
        if dead || (boat.is_none() && !cart && !mount && !cushion) {
            let g = &mut host.hub.guests[gi];
            (g.vehicle, g.vehicle_good) = (None, None);
            return;
        }
        if let Some(i) = boat {
            // AbstractBoat.tick on the server for a boat its rider drives:
            // only the hurt animation and the damage wear off here.
            let b = &mut self.boats[i];
            b.hurt_time = (b.hurt_time - 1).max(0);
            b.damage = (b.damage - 1.0).max(0.0);
        }
        // Getting out goes through leave_guest, which tells the client and
        // teleports it (ServerPlayer.dismountTo).
        let seat = self.as_guest(host, conn, |game| {
            // A cushion does not move: the rider sits on it until it sneaks
            // (`Cushion.removePassenger` plays the get-up sound).
            if game.cushions.iter().any(|c| Some(c.id) == game.riding) {
                if keys.shift {
                    game.tick_cushion_riding(keys);
                    return None;
                }
                let c = game.cushions.iter().find(|c| Some(c.id) == game.riding)?;
                let p = c.seat();
                return Some([p.x, p.y, p.z]);
            }
            if let Some(i) = game.boats.iter().position(|b| Some(b.id) == game.riding) {
                if keys.shift {
                    game.dismount(i);
                    return None;
                }
                let p = game.player_boat_seat(i);
                return Some([p.x, p.y, p.z]);
            }
            // The minecart moves with the rider's input, and lets it out when
            // it sneaks or the cart breaks. An animal takes the rider's input
            // for its own tick (`travelRidden`), or lets it off.
            if !game.tick_cart_riding(keys) && game.tick_mount_riding(keys) {
                game.riding?;
                game.position_rider();
            }
            game.riding?;
            let p = game.player.as_ref()?.position;
            Some([p.x, p.y, p.z])
        });
        if let Some(Some(at)) = seat {
            let g = &mut host.hub.guests[gi];
            g.previous_position = g.position;
            g.position = at;
        }
    }

    /// After an interaction: a player that got into a vehicle sits in it,
    /// and its last good vehicle position is where the vehicle is.
    pub(super) fn guest_boarded(&mut self, host: &mut LanHost, conn: u32, before: Option<u32>) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(vehicle) = host.hub.guests[gi].vehicle else {
            return;
        };
        if before == Some(vehicle) {
            return;
        }
        // The host in the window rides it already.
        if self.riding == Some(vehicle) {
            host.hub.guests[gi].vehicle = before;
            return;
        }
        let at = self
            .boats
            .iter()
            .position(|b| b.id == vehicle)
            .map(|i| (self.boats[i].position, self.player_boat_seat(i)))
            .or_else(|| {
                self.carts
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.position.add_vector(Vec3::new(0.0, SEAT, 0.0))))
            })
            .or_else(|| {
                self.cushions
                    .iter()
                    .find(|c| c.id == vehicle)
                    .map(|c| (c.position, c.seat()))
            });
        if let Some((p, seat)) = at {
            let g = &mut host.hub.guests[gi];
            g.vehicle_good = Some([p.x, p.y, p.z]);
            g.previous_position = g.position;
            g.position = [seat.x, seat.y, seat.z];
            g.on_ground = false;
            return;
        }
        // An animal: the rider sits where `positionRider` puts it.
        let seat = self.as_guest(host, conn, |game| {
            game.position_rider();
            game.player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z])
        });
        if let Some(Some(at)) = seat {
            let g = &mut host.hub.guests[gi];
            g.previous_position = g.position;
            g.position = at;
            g.on_ground = false;
        }
    }

    /// `ServerGamePacketListenerImpl.handleMoveVehicle`: the boat its rider
    /// drives goes where the client says, unless it moved too quickly or
    /// wrongly; then it stays, and the client is told where it is.
    pub(super) fn guest_move_vehicle(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::MoveVehicleRequest,
    ) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let [x, y, z] = p.position;
        if !x.is_finite()
            || !y.is_finite()
            || !z.is_finite()
            || !p.yaw.is_finite()
            || !p.pitch.is_finite()
        {
            host.hub.kick(
                conn,
                translatable("multiplayer.disconnect.invalid_vehicle_movement", &[]),
            );
            return;
        }
        let g = &host.hub.guests[gi];
        if g.awaiting_teleport.is_some() || !g.loaded {
            return;
        }
        let owner = host.hub.singleplayer_owner == Some(g.uuid);
        let (vehicle, good) = (g.vehicle, g.vehicle_good);
        let back = |host: &mut LanHost, position: Vec3, yaw: f32| {
            let packet = play::MoveVehicle {
                position: [position.x, position.y, position.z],
                yaw,
                pitch: 0.0,
            };
            host.hub.send_to(conn, &C::MoveVehicle(packet));
        };
        // Only a boat is driven by its rider (getControllingPassenger); a
        // minecart that is elsewhere on the client is put back.
        let Some(i) = vehicle.and_then(|v| self.boats.iter().position(|b| b.id == v)) else {
            if let Some(c) = vehicle.and_then(|v| self.carts.iter().find(|c| c.id == v))
                && c.position.distance_squared(Vec3::new(x, y, z)) > 1.0
            {
                back(host, c.position, c.yaw);
            }
            return;
        };
        let b = &self.boats[i];
        let old = b.position;
        let good = good.map_or(old, |g| Vec3::new(g[0], g[1], g[2]));
        let target = Vec3::new(
            x.clamp(-3.0E7, 3.0E7),
            y.clamp(-2.0E7, 2.0E7),
            z.clamp(-3.0E7, 3.0E7),
        );
        // Moved too quickly: more than 10 blocks past its own motion.
        let moved = target.distance_squared(old) - b.velocity.length_squared();
        if moved > 100.0 && !owner {
            log::warn!("boat {} moved too quickly! {moved}", b.id);
            back(host, old, b.yaw);
            return;
        }
        // vehicle.move(PLAYER, target - lastGood) with the boat's collisions:
        // where it can't follow the client (a quarter block off, the height
        // aside), it moved wrongly.
        let bounds = b.bounds();
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let wanted = target.subtract(good);
        let resolved = scene
            .resolve(bounds, wanted, 0.0, b.on_ground)
            .unwrap_or(wanted);
        let reached = old.add_vector(resolved);
        let wrongly = (target.x - reached.x).powi(2) + (target.z - reached.z).powi(2) > 0.0625;
        let free_before = scene.unobstructed(bounds).unwrap_or(true);
        // isEntityCollidingWithAnythingNew: a box that was free is not there.
        let moved_box = bounds.move_by(target.subtract(old));
        let free_after = scene
            .unobstructed(moved_box.deflate(Vec3::new(1.0E-5, 1.0E-5, 1.0E-5)))
            .unwrap_or(true);
        if free_before && (wrongly || !free_after) {
            log::warn!("boat {} moved wrongly", b.id);
            back(host, old, b.yaw);
            return;
        }
        let b = &mut self.boats[i];
        b.previous = old;
        b.velocity = target.subtract(old);
        b.position = target;
        b.previous_yaw = b.yaw;
        b.yaw = p.yaw;
        b.on_ground = p.on_ground;
        let seat = self.player_boat_seat(i);
        let g = &mut host.hub.guests[gi];
        g.vehicle_good = Some([target.x, target.y, target.z]);
        g.previous_position = g.position;
        g.position = [seat.x, seat.y, seat.z];
    }

    /// `handlePaddleBoat`: the paddles of the boat the player drives.
    pub(super) fn guest_paddle_boat(&mut self, host: &LanHost, conn: u32, paddles: [bool; 2]) {
        let vehicle = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .and_then(|g| g.vehicle);
        if let Some(b) = self.boats.iter_mut().find(|b| Some(b.id) == vehicle) {
            b.paddling = paddles;
        }
    }
}
