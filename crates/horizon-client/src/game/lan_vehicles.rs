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
use horizon_client::lan::hub::no_blocks_in;
use horizon_client::mobs::equine;
use horizon_core::geometry::{Aabb, Axis, Vec3};
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
        let at = |v: Vec3| [v.x, v.y, v.z];
        let centre = |p: [i32; 3]| p.map(|v| f64::from(v) + 0.5);
        if let Some(i) = self.interactions.get(&id) {
            let (lo, hi) = corners(i.bounds());
            return Some((lo, hi, at(i.position)));
        }
        let id = u32::try_from(id).ok()?;
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
        let mut ejected = false;
        if let Some(i) = boat {
            // AbstractBoat.tick on the server for a boat its rider drives:
            // only the hurt animation and the damage wear off here.
            let b = &mut self.boats[i];
            b.hurt_time = (b.hurt_time - 1).max(0);
            b.damage = (b.damage - 1.0).max(0.0);
            // `Entity.ejectPassengers`: every player in the boat must see
            // the request; the common boat tick clears it once nobody rides.
            ejected = b.eject_riders;
        }
        // ServerGamePacketListenerImpl.tick: the vehicle's last good position
        // is where it is now, so a move the server made (a piston, a push)
        // is not held against the rider's next move, which would fail, be
        // sent back, and fail again on every answer.
        let at = boat.map(|i| self.boats[i].position).or_else(|| {
            self.mobs
                .iter()
                .find(|m| m.id == vehicle && equine::moved_by_client(m))
                .map(|m| m.position)
        });
        if let Some(p) = at {
            host.hub.guests[gi].vehicle_good = Some([p.x, p.y, p.z]);
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
                if keys.shift || ejected {
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

    /// `ServerGamePacketListenerImpl.handleMoveVehicle`: the boat or the
    /// saddled horse its rider drives goes where the client says, unless it
    /// moved too quickly or wrongly; then it stays, and the client is told
    /// where it is.
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
        if g.awaiting_teleport.is_some() || !g.client_loaded() {
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
        // Only a boat, and a horse its rider's client moves, are driven by
        // their rider (getControllingPassenger); a minecart that is elsewhere
        // on the client is put back.
        let boat = vehicle.and_then(|v| self.boats.iter().position(|b| b.id == v));
        let mount = vehicle.and_then(|v| {
            self.mobs
                .iter()
                .position(|m| m.id == v && equine::moved_by_client(m))
        });
        // The vehicle's id, position, rotation, motion, box, step height
        // and ground.
        let (id, old, yaw, velocity, bounds, step, on_ground) = if let Some(i) = boat {
            let b = &self.boats[i];
            (
                b.id,
                b.position,
                b.yaw,
                b.velocity,
                b.bounds(),
                0.0,
                b.on_ground,
            )
        } else if let Some(i) = mount {
            let m = &self.mobs[i];
            let step = m.kind.step_height as f32;
            (
                m.id,
                m.position,
                m.yaw,
                m.velocity,
                m.bounds(),
                step,
                m.on_ground,
            )
        } else {
            if let Some(c) = vehicle.and_then(|v| self.carts.iter().find(|c| c.id == v))
                && c.position.distance_squared(Vec3::new(x, y, z)) > 1.0
            {
                back(host, c.position, c.yaw);
            }
            return;
        };
        let good = good.map_or(old, |g| Vec3::new(g[0], g[1], g[2]));
        let target = Vec3::new(
            x.clamp(-3.0E7, 3.0E7),
            y.clamp(-2.0E7, 2.0E7),
            z.clamp(-3.0E7, 3.0E7),
        );
        // Moved too quickly: more than 10 blocks past its own motion.
        let moved = target.distance_squared(old) - velocity.length_squared();
        if moved > 100.0 && !owner {
            log::warn!("vehicle {id} moved too quickly! {moved}");
            back(host, old, yaw);
            return;
        }
        // vehicle.move(PLAYER, target - lastGood) with the vehicle's
        // collisions: where it can't follow the client (a quarter block off,
        // the height aside), it moved wrongly.
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let wanted = target.subtract(good);
        let resolved = scene
            .resolve(bounds, wanted, step, on_ground)
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
            log::warn!("vehicle {id} moved wrongly");
            back(host, old, yaw);
            return;
        }
        // clientVehicleIsFloating: the move didn't go down a step, the vehicle hadn't come down onto
        // something on its last move (`verticalCollisionBelow` before this one), and nothing but air is
        // around its box grown by 0.0625 and stretched 0.55 down (water holds a boat up). A flying or
        // no-gravity mount, and a server that allows flight, never float.
        let (gravity, no_gravity) = match mount {
            Some(i) => (0.08, self.mobs[i].no_gravity),
            None => (0.04, false),
        };
        let around = moved_box
            .inflate(Vec3::new(0.0625, 0.0625, 0.0625))
            .expand_towards(Vec3::new(0.0, -0.55, 0.0));
        let corner = |max: bool| {
            [Axis::X, Axis::Y, Axis::Z].map(|a| if max { around.max(a) } else { around.min(a) })
        };
        let no_blocks = no_blocks_in(&self.world, corner(false), corner(true));
        let allow_flight = host.hub.config.allow_flight;
        let g = &mut host.hub.guests[gi];
        g.vehicle_good = Some([target.x, target.y, target.z]);
        let rested = std::mem::replace(
            &mut g.vehicle_rests,
            wanted.y < 0.0 && resolved.y != wanted.y,
        );
        g.vehicle_floating =
            wanted.y >= -0.03125 && !rested && !allow_flight && !no_gravity && no_blocks;
        g.vehicle_gravity = gravity;
        if let Some(i) = mount {
            // absSnapTo: where and how its rider's client turned it; the
            // rider sits on it from its own ride tick.
            let m = &mut self.mobs[i];
            m.position = target;
            m.yaw = p.yaw;
            m.head_pitch = p.pitch;
            m.on_ground = p.on_ground;
            return;
        }
        let Some(i) = boat else {
            return;
        };
        let b = &mut self.boats[i];
        b.previous = old;
        b.velocity = target.subtract(old);
        b.position = target;
        b.previous_yaw = b.yaw;
        b.yaw = p.yaw;
        b.on_ground = p.on_ground;
        let seat = self.player_boat_seat(i);
        let g = &mut host.hub.guests[gi];
        g.previous_position = g.position;
        g.position = [seat.x, seat.y, seat.z];
    }

    /// `Entity.getControllingPassenger` of a vehicle a server player rides: a
    /// boat has two seats, and only its first player passenger drives it. The
    /// window player sits first when it rides; of the server's players it is
    /// the one that got in first.
    pub(super) fn guest_controls_vehicle(&self, host: &LanHost, conn: u32, vehicle: u32) -> bool {
        if self.riding == Some(vehicle) {
            return false;
        }
        host.hub
            .guests
            .iter()
            .filter(|g| g.vehicle == Some(vehicle))
            .min_by_key(|g| g.vehicle_order)
            .is_some_and(|g| g.conn == conn)
    }

    /// `handlePaddleBoat`: the paddles of the boat the player controls
    /// (`player.getControlledVehicle()`); a passenger in the second seat moves
    /// no paddle.
    pub(super) fn guest_paddle_boat(&mut self, host: &LanHost, conn: u32, paddles: [bool; 2]) {
        let vehicle = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .and_then(|g| g.vehicle);
        let Some(vehicle) = vehicle.filter(|&v| self.guest_controls_vehicle(host, conn, v)) else {
            return;
        };
        if let Some(b) = self.boats.iter_mut().find(|b| b.id == vehicle) {
            b.paddling = paddles;
        }
    }
}
