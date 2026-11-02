//! Boats, minecarts, item frames, armor stands and paintings for a server's
//! client: single player on its server thread, a LAN guest, a player on a
//! dedicated server.
//!
//! The client aims at them like at mobs (`Minecraft.hitResult`, then
//! `ServerboundInteractPacket`), and rides the vehicle the server seats it in
//! (`ClientPacketListener.handleSetEntityPassengersPacket`). A boat is
//! controlled by its rider's client: it ticks here and its moves go to the
//! server (`LocalPlayer.tick`: `ServerboundMoveVehiclePacket`,
//! `AbstractBoat.tick`: `ServerboundPaddleBoatPacket`). So is a saddled horse,
//! donkey, mule or camel: its rider's client moves it and reports the moves,
//! and tells the server of a riding jump (`START_RIDING_JUMP`). A minecart
//! moves on the server with the rider's input, and the rider sits in it.
use super::{Game, Keys, ray_box};
use horizon_client::mobs::{Mob, equine, happy_ghast};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::packets::play::{self, Serverbound as S};

/// An object under the crosshair: its entity id, how far along the view it
/// was hit, and where (`EntityHitResult.getLocation` minus the entity's
/// position, the interact packet's location).
pub(super) struct ObjectHit {
    pub id: i32,
    pub distance: f64,
    pub location: [f64; 3],
}

impl Game<'_> {
    /// The nearest boat, minecart, item frame, armor stand or painting the
    /// view hits within reach; also End crystals when attacking.
    pub(super) fn remote_object_hit(&self, attacking: bool) -> Option<ObjectHit> {
        let (eye, dir) = self.aim()?;
        let at = |d: f64| {
            [
                eye[0] + dir[0] * d,
                eye[1] + dir[1] * d,
                eye[2] + dir[2] * d,
            ]
        };
        let block_centre = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        let mut hits: Vec<(i32, f64, [f64; 3])> = Vec::new();
        if let Some((i, d)) = self.painting_hit() {
            let p = &self.paintings[i];
            hits.push((p.id as i32, d, block_centre(p.pos)));
        }
        if let Some((i, d)) = self.frame_hit() {
            let f = &self.item_frames[i];
            hits.push((f.id as i32, d, block_centre(f.pos)));
        }
        if let Some((i, d, _)) = self.armor_stand_hit() {
            let a = &self.armor_stands[i];
            let p = a.position;
            hits.push((a.id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d, _)) = self.cushion_hit() {
            let c = &self.cushions[i];
            let p = c.position;
            hits.push((c.id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d)) = self.boat_hit() {
            let p = self.boats[i].position;
            hits.push((self.boats[i].id as i32, d, [p.x, p.y, p.z]));
        }
        if let Some((i, d)) = self.cart_hit() {
            let p = self.carts[i].position;
            hits.push((self.carts[i].id as i32, d, [p.x, p.y, p.z]));
        }
        // Interaction.isPickable: an invisible box the server clicks through
        // to its scripts, for both uses and attacks.
        let reach = self.entity_reach;
        for (&id, i) in &self.interactions {
            if let Some(d) = ray_box(eye, dir, reach, i.bounds()) {
                let p = i.position;
                hits.push((id, d, [p.x, p.y, p.z]));
            }
        }
        if attacking {
            // EndCrystal.isPickable: its 2 x 2 box, with the server's entity
            // id, not special_targets' local projectile target id.
            // Entity.interact passes on a crystal, so right clicks use the item.
            let reach = self.entity_reach;
            for c in self.crystals.iter().filter(|c| c.id != 0) {
                let p = c.position;
                let bounds = Aabb::new(
                    Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
                    Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
                );
                if let Some(d) = ray_box(eye, dir, reach, bounds) {
                    hits.push((c.id as i32, d, [p.x, p.y, p.z]));
                }
            }
            // EnderDragonPart.isPickable: the server's dragon hurts by part.
            hits.extend(self.remote_dragon_part_hit(eye, dir, reach));
        }
        let (id, distance, origin) = hits.into_iter().min_by(|a, b| a.1.total_cmp(&b.1))?;
        let point = at(distance);
        Some(ObjectHit {
            id,
            distance,
            location: [
                point[0] - origin[0],
                point[1] - origin[1],
                point[2] - origin[2],
            ],
        })
    }

    /// `ClientPacketListener.handleSetEntityPassengersPacket`: who rides
    /// `vehicle` now. The local player gets in or out; mobs and other
    /// players sit where the server keeps them.
    pub(super) fn remote_set_passengers(&mut self, vehicle: i32, passengers: &[i32]) {
        let Some(me) = self.remote.as_ref().map(|r| r.entity_id) else {
            return;
        };
        let vehicle_id = vehicle as u32;
        for m in &mut self.mobs {
            // `Entity.isVehicle` (the saddle reins of a ridden horse): the
            // server sends no rider on the mob itself, only this list.
            if m.id == vehicle_id {
                m.extra.rider = passengers.first().map(|&p| p as u32);
            }
            if m.extra.mount == Some(vehicle_id) && !passengers.contains(&(m.id as i32)) {
                m.extra.mount = None;
                m.extra.passenger = false;
            } else if passengers.contains(&(m.id as i32)) {
                m.extra.mount = Some(vehicle_id);
                m.extra.passenger = true;
            }
        }
        for (id, display) in &mut self.text_displays {
            if passengers.contains(id) {
                display.vehicle = Some(vehicle_id);
            } else if display.vehicle == Some(vehicle_id) {
                display.vehicle = None;
            }
        }
        for a in &mut self.armor_stands {
            if passengers.contains(&(a.id as i32)) {
                a.vehicle = Some(vehicle_id);
            } else if a.vehicle == Some(vehicle_id) {
                a.vehicle = None;
            }
        }
        for (&id, i) in &mut self.interactions {
            if passengers.contains(&id) {
                i.vehicle = Some(vehicle_id);
            } else if i.vehicle == Some(vehicle_id) {
                i.vehicle = None;
            }
        }
        if passengers.contains(&me)
            && let Some(r) = self.remote.as_mut()
        {
            r.first_passenger = passengers.first() == Some(&me);
        }
        if passengers.contains(&me) {
            if self.riding != Some(vehicle_id) {
                self.riding = Some(vehicle_id);
                if let Some(p) = self.player.as_mut() {
                    p.velocity = Vec3::ZERO;
                    p.fall_distance = 0.0;
                }
            }
        } else if self.riding == Some(vehicle_id) {
            // The server takes the player out, then teleports it.
            self.riding = None;
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == vehicle_id) {
                m.rider = None;
            }
            // HappyGhast.removePassenger: a still timeout of 10 (the server's
            // STAYS_STILL confirms it), and the rider lands on top of the ghast
            // (`place_ghast_under`).
            if let Some(m) = self.mobs.iter_mut().find(|m| {
                m.id == vehicle_id && m.species() == horizon_client::mobs::Species::HappyGhast
            }) {
                m.extra.counter = 10;
                if let Some(r) = self.remote.as_mut() {
                    r.ghast_dismount = Some(vehicle_id);
                }
            }
        }
    }

    /// The local player's tick in its vehicle, in place of its own movement:
    /// false when it rides nothing.
    pub(super) fn remote_ride(&mut self, keys: Keys) -> bool {
        let Some(id) = self.riding else {
            return false;
        };
        if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            // LocalPlayer.aiStep: the jump bar charges while jump is held.
            let jump = self.charge_mount_jump(i, keys);
            if self.drives(&self.mobs[i]) {
                self.drive_mount(i, keys, jump);
            }
            // Another animal moves on the server with the rider's input
            // (PlayerInput); the rider sits where `positionRider` puts it.
            self.position_rider();
            return true;
        }
        let seat = if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            // The seat does not move; the rider sits on the cushion.
            Some(c.seat())
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            // AbstractBoat.tick on the controlling client: steer and move.
            // It steers with the input `LocalPlayer.rideTick` latched on the
            // previous tick, so the paddle packet describes the motion the
            // server is sent; this tick's keys are latched after it ticked.
            let mut b = self.tick_boat(self.boats[i].clone(), true);
            b.input = [keys.left, keys.right, keys.forward, keys.backward];
            // positionRider: the seat turns with the boat; clampRotation.
            self.yaw += b.delta_rotation;
            let delta = (self.yaw - b.yaw + 180.0).rem_euclid(360.0) - 180.0;
            self.yaw += delta.clamp(-105.0, 105.0) - delta;
            self.boats[i] = b;
            // In front of a mob that shares the boat.
            Some(self.player_boat_seat(i))
        } else {
            // A minecart: the server moves it, the rider sits in it.
            let cart = self.carts.iter().find(|c| c.id == id).map(|c| {
                c.position
                    .add_vector(Vec3::new(0.0, super::minecarts::SEAT, 0.0))
            });
            if cart.is_none() {
                // Entity.baseTick: a passenger whose vehicle is gone stops
                // riding it. Nothing with the id is in the level any more (it
                // died, or the world it rode in was left), so the seat goes.
                self.riding = None;
                return false;
            }
            cart
        };
        if let Some(p) = self.player.as_mut() {
            // The tick a rider takes its seat is a teleport (`Entity.moveTo`,
            // which sets the old position too): the camera must not sweep from
            // where the player stood to the cushion, boat or cart. The ticks
            // after it move with the vehicle and interpolate as usual.
            let first = self.posed_rider != self.riding;
            self.posed_rider = self.riding;
            if let Some(v) = seat {
                self.previous_position = if first { v } else { p.position };
                p.position = v;
            } else {
                self.previous_position = p.position;
            }
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        true
    }

    /// `LocalPlayer.tick` for a passenger: the boat it controls goes to the
    /// server every tick with its paddles (the rider's own position does not).
    pub(super) fn remote_vehicle_packets(&self) -> Vec<S> {
        if let Some(m) = self.driven_mount() {
            return vec![mount_move(m)];
        }
        let Some(b) = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
        else {
            return Vec::new();
        };
        vec![
            S::MoveVehicle(play::MoveVehicleRequest {
                position: [b.position.x, b.position.y, b.position.z],
                yaw: b.yaw,
                pitch: 0.0,
                on_ground: b.on_ground,
            }),
            S::PaddleBoat(play::PaddleBoat {
                left: b.paddling[0],
                right: b.paddling[1],
            }),
        ]
    }

    /// `ClientPacketListener.handleMoveVehicle`: the server put the boat or
    /// the mount the player drives back.
    pub(super) fn remote_move_vehicle(&mut self, position: [f64; 3], yaw: f32) {
        let at = Vec3::new(position[0], position[1], position[2]);
        if let Some(id) = self.driven_mount().map(|m| m.id)
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            m.previous = at;
            m.position = at;
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.velocity = Vec3::ZERO;
            return;
        }
        let Some(b) = self
            .riding
            .and_then(|id| self.boats.iter_mut().find(|b| b.id == id))
        else {
            return;
        };
        b.previous = at;
        b.position = at;
        b.yaw = yaw;
        b.previous_yaw = yaw;
        b.velocity = Vec3::ZERO;
    }

    /// `AbstractBoat.handleEntityEvent`: 71 and 72, a bubble column dragging
    /// the boat down or throwing it up (`handleBubbleColumnEffect`, which the
    /// client applies to the boat it drives).
    pub(super) fn remote_boat_event(&mut self, id: i32, event: i8) {
        let drag_down = event == super::boats::BUBBLE_DOWN_EVENT;
        if !drag_down && event != super::boats::BUBBLE_UP_EVENT {
            return;
        }
        if let Some(i) = self.boats.iter().position(|b| b.id == id as u32) {
            self.handle_bubble_column_effect(i, drag_down);
        }
    }

    /// `AbstractBoat.tick`: the part that reaches bubble columns, for every
    /// boat on a client: `status`, the two `applyEffectsFromBlocks` (a boat
    /// above a column may splash; the boat the player drives is lifted or
    /// dragged by a column it is inside) and `tickBubbleColumn`'s wobble from
    /// the synched bubble time.
    pub(super) fn tick_remote_boat_bubbles(&mut self) {
        for i in 0..self.boats.len() {
            let driven = self.riding == Some(self.boats[i].id);
            if !driven {
                // `AbstractBoat.tick`: the non-authoritative copy carries no
                // cached motion into the next time this player drives it.
                self.boats[i].velocity = Vec3::ZERO;
                self.boats[i].delta_rotation = 0.0;
                self.refresh_boat_status(i);
            }
            let touches = self.boat_bubble_columns(i);
            for _ in 0..2 {
                for &column in &touches {
                    if column.surface {
                        if self.boat_splash_roll(i) {
                            self.boat_bubble_splash(i);
                        }
                    } else if driven {
                        self.boat_inside_bubble_column(i, column);
                    }
                }
            }
            self.boats[i].tick_bubble_wobble();
        }
    }

    /// Whether this client moves `m`, the mount the player rides: as its
    /// first passenger (`AbstractHorse.getControllingPassenger`), on its
    /// saddle (`isLocalInstanceAuthoritative`).
    fn drives(&self, m: &Mob) -> bool {
        self.riding == Some(m.id)
            && client_moves(m, self.remote.as_ref().is_some_and(|r| r.first_passenger))
    }

    /// The mount this client moves, if the player rides one.
    pub(super) fn driven_mount(&self) -> Option<&Mob> {
        let id = self.riding?;
        self.mobs.iter().find(|m| m.id == id && self.drives(m))
    }

    /// `LocalPlayer.aiStep` and `LivingEntity.travelRidden` on the
    /// controlling client: a released jump goes to the server
    /// (`sendRidingJump`), and the mount takes the rider's input and moves
    /// here (`tickRidden`, then `travel`).
    fn drive_mount(&mut self, i: usize, keys: Keys, jump: f32) {
        if jump > 0.0
            && let Some(id) = self.remote.as_ref().map(|r| r.entity_id)
        {
            self.send(riding_jump(id, self.mount.jump_scale));
        }
        let strafe = f32::from(u8::from(keys.left)) - f32::from(u8::from(keys.right));
        let forward = f32::from(u8::from(keys.forward)) - f32::from(u8::from(keys.backward));
        let m = &mut self.mobs[i];
        if jump > 0.0 && equine::is_equine(m.species()) {
            // LocalPlayer.aiStep also runs `onPlayerJump` here: the mount slides
            // on the rider's keys and rears while the server does the same.
            equine::on_player_jump(m);
        }
        // playerJumpPendingScale waits for the ground.
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw: self.yaw,
            pitch: self.pitch,
            jump: if jump > 0.0 { jump } else { pending },
            steer: false,
            sprint: keys.sprint,
            jumping: keys.jump,
            client: false,
        });
        let mut mob = self.mobs.remove(i);
        let mut random = self.world.random.clone();
        let mut events = Vec::new();
        {
            let world = self.mob_world_base(&[]);
            mob.tick(&world, &mut random, &mut events);
        }
        // Only the server's packets take the mount away.
        mob.removed = false;
        self.world.random = random;
        self.mobs.insert(i, mob);
        // #4821: footstep, gallop, breathe, splash and landing sounds from mob.tick
        let _ = self.handle_mob_events(events);
    }
}

/// Whether a player's client moves `m`, the mount it rides
/// (`isLocalInstanceAuthoritative`): a saddled horse, donkey, mule or camel
/// it is the first passenger of (`AbstractHorse.getControllingPassenger`),
/// or a harnessed adult Happy Ghast it is the first passenger of
/// (`HappyGhast.getControllingPassenger`: wearing body armor, player on
/// top, not on a still timeout — the rider steers it with `travelRidden`
/// on their own client, and the mount goes to the server as `MoveVehicle`).
fn client_moves(m: &Mob, first_passenger: bool) -> bool {
    if !first_passenger || !m.alive() {
        return false;
    }
    if equine::moved_by_rider(m.species()) && m.extra.saddled {
        return true;
    }
    m.species() == horizon_client::mobs::Species::HappyGhast
        && happy_ghast::harnessed_adult(m)
        && !happy_ghast::on_still_timeout(m)
}

/// `LocalPlayer.sendRidingJump`: START_RIDING_JUMP with
/// `floor(getJumpRidingScale() * 100)`, as `onPlayerJump` gets it.
fn riding_jump(player: i32, scale: f32) -> S {
    S::PlayerCommand(play::PlayerCommand {
        id: player,
        action: 3,
        data: (scale * 100.0).floor() as i32,
    })
}

/// `ServerboundMoveVehiclePacket.fromEntity`: where the mount this client
/// moves is, and how it is turned.
fn mount_move(m: &Mob) -> S {
    S::MoveVehicle(play::MoveVehicleRequest {
        position: [m.position.x, m.position.y, m.position.z],
        yaw: m.yaw,
        pitch: m.head_pitch,
        on_ground: m.on_ground,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mobs::MobTypes;

    /// A mob of the prepared resources' type `kind` (None without them).
    fn mob(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        let types = MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(Mob::new(7, t, Vec3::new(1.5, 64.0, -2.5), 0.0))
    }

    /// #2920: on a vanilla server the rider's client moves a saddled horse,
    /// donkey, mule or camel; a pig, a llama, an unsaddled horse or a second
    /// passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_saddled_horse_on_its_client() {
        for kind in ["horse", "donkey", "mule", "camel", "pig", "llama"] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(!client_moves(&m, true), "{kind} without a saddle");
            m.set_saddle(Some(horizon_client::items::ItemStack::new(
                "minecraft:saddle",
                1,
            )));
            let horse = matches!(kind, "horse" | "donkey" | "mule" | "camel");
            assert_eq!(client_moves(&m, true), horse, "{kind}");
            assert!(!client_moves(&m, false), "{kind} behind another rider");
        }
    }

    /// #4107: on a vanilla server the rider's client moves a harnessed adult
    /// Happy Ghast (`getControllingPassenger`); an unharnessed one, a baby
    /// or a second passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_harnessed_ghast_on_its_client() {
        let Some(mut m) = mob("minecraft:happy_ghast") else {
            return;
        };
        assert!(!client_moves(&m, true), "ghast without a harness");
        m.extra.head = Some("minecraft:white_happy_ghast_harness".into());
        assert!(client_moves(&m, true), "harnessed adult");
        assert!(!client_moves(&m, false), "behind another rider");
        m.extra.age = -1;
        assert!(!client_moves(&m, true), "baby ghast");
    }

    /// #2920: the client reports where it moved the mount, and a released
    /// jump with the bar's percentage (`sendRidingJump`).
    #[test]
    fn the_rider_sends_its_mounts_moves_and_jumps() {
        let Some(mut m) = mob("minecraft:horse") else {
            return;
        };
        (m.yaw, m.head_pitch, m.on_ground) = (90.0, 10.0, true);
        match mount_move(&m) {
            S::MoveVehicle(p) => {
                assert_eq!(p.position, [1.5, 64.0, -2.5]);
                assert_eq!((p.yaw, p.pitch, p.on_ground), (90.0, 10.0, true));
            }
            other => panic!("{other:?}"),
        }
        match riding_jump(42, 0.75) {
            S::PlayerCommand(p) => assert_eq!((p.id, p.action, p.data), (42, 3, 75)),
            other => panic!("{other:?}"),
        }
    }
}
