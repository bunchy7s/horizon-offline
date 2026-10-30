//! A server's players between dimensions.
//!
//! A server holds a level per dimension (game/levels.rs). A player who steps
//! into a Nether or End portal moves to the destination's level, which opens
//! if nobody was there: the swap is the in-frame path's portal plans and
//! arrival search, and the player hears it as `ServerPlayer.teleport` into
//! another level: a Respawn packet keeping all data, then a teleport to the
//! exit once the destination has loaded around them.

use super::end::GatewayJob;
use super::nether_portal::{PLAYER_DELAY, Plan};
use super::respawn_anchor::RespawnConfig;
use super::{Dim, Game, Mob, boats, lan::LanHost, minecarts};
use horizon_client::{lan::hub::TickCtx, text::Text, world_spawn::PlayerSpawnFinder};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A trip under way: the player is in the destination's level, whose
/// chunks are loading around `target`.
pub(super) struct Trip {
    conn: u32,
    dim: Dim,
    /// The level the player came from: the vehicle it rides waits there
    /// until the exit is found.
    source: Dim,
    target: Vec3,
    /// Through an End portal.
    end: bool,
    /// The Nether portal plan, whose exit is found or built on arrival.
    plan: Option<Plan>,
    /// The vehicle the player rides, which travels with it
    /// (`Entity.teleportCrossDimension` carries the passengers).
    vehicle: Option<u32>,
    /// Through an End gateway, within the level: its exit, found or built
    /// on arrival.
    gateway: Option<GatewayJob>,
    /// A respawn at this point outside the Overworld, resolved through its
    /// block on arrival.
    respawn: Option<RespawnConfig>,
    /// A respawn at the world spawn, moved onto a free column within the
    /// respawn radius once the chunks around it have loaded.
    world_spawn: bool,
    /// The search for that column while a candidate's chunk loads.
    finder: Option<PlayerSpawnFinder>,
    /// Sent by a command: the player is already where it goes.
    placed: bool,
}

/// A vehicle a player rides: an animal, a boat or a minecart. It travels
/// with the player through a portal (`Entity.teleportCrossDimension` carries
/// the passengers, and they ride the vehicle that is recreated in the other
/// dimension).
trait Rideable {
    fn id(&self) -> u32;
    /// The vehicle stands where its rider arrives, with no motion.
    fn arrive(&mut self, at: Vec3, yaw: f32);
}

impl Rideable for Mob {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.fall_distance = 0.0;
    }
}

impl Rideable for boats::Boat {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.delta_rotation = 0.0;
    }
}

impl Rideable for minecarts::Minecart {
    fn id(&self) -> u32 {
        self.id
    }
    fn arrive(&mut self, at: Vec3, yaw: f32) {
        self.position = at;
        self.previous = at;
        self.velocity = Vec3::ZERO;
        self.yaw = yaw;
        self.previous_yaw = yaw;
    }
}

/// The vehicle `id` leaves the level it is in, at the point its rider
/// arrives. It keeps its entity id, so the ride is never broken.
fn take_vehicle<T: Rideable>(list: &mut Vec<T>, id: u32, at: Vec3, yaw: f32) -> Option<T> {
    let i = list.iter().position(|v| v.id() == id)?;
    let mut v = list.remove(i);
    v.arrive(at, yaw);
    Some(v)
}

/// `TeleportCommand` into another level for a server's player: where, and
/// the rotation it sets (none keeps the player's).
pub(super) struct LevelTeleport {
    pub conn: u32,
    pub dim: Dim,
    pub at: [f64; 3],
    pub rotation: Option<[f32; 2]>,
}

/// A player's box: Player's standing dimensions.
const WIDTH: f64 = 0.6;
const HEIGHT: f64 = 1.8;

impl Trip {
    pub(super) fn is(&self, conn: u32) -> bool {
        self.conn == conn
    }
}

impl Game<'_> {
    /// Whether `conn` is between two levels: the server leaves its body
    /// alone until it arrives.
    pub(super) fn on_server_trip(&self, conn: u32) -> bool {
        self.server_trips.iter().any(|t| t.conn == conn)
    }

    /// Whether a vehicle waits in `dim` for its rider to arrive elsewhere:
    /// that level stays open until it has moved.
    pub(super) fn vehicle_waits_in(&self, dim: Dim) -> bool {
        self.server_trips
            .iter()
            .any(|t| t.vehicle.is_some() && t.source == dim)
    }

    /// Per server tick in each level, after the guests' own: the arrivals of
    /// the trips into this level, then `Entity.handlePortal` for each player
    /// in it (Nether portals count up their transition time, End portals
    /// take them at once).
    pub(super) fn tick_server_travel(&mut self, host: &mut LanHost) {
        // Outside a server only the respawns at the world spawn wait for chunks.
        self.arrive_server_trips(host);
        if self.options.dedicated.is_none() {
            return;
        }
        self.carry_riders_through_portals(host);
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.spectator())
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            if self.on_server_trip(conn) {
                continue;
            }
            // `Entity.portalProcess` and `portalCooldown` are the player's own.
            let mut portal = host.play.entry(conn).or_default().portal;
            std::mem::swap(&mut self.portals.player, &mut portal);
            self.handle_server_portal(host, conn);
            std::mem::swap(&mut self.portals.player, &mut portal);
            host.play.entry(conn).or_default().portal = portal;
        }
    }

    /// `Entity.handlePortal` for one player, whose portal state is in place.
    fn handle_server_portal(&mut self, host: &mut LanHost, conn: u32) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (at, invulnerable, sleeping, riding) = (
            Vec3::new(g.position[0], g.position[1], g.position[2]),
            g.creative(),
            g.sleeping,
            g.vehicle.is_some(),
        );
        // Entity.checkInsideBlocks: the Nether portal blocks the box is in.
        // `canUsePortal(false)` is false for a passenger: its vehicle goes
        // through and carries it (`carry_riders_through_portals`).
        if !riding && let Some(pos) = self.nether_portal_inside(at) {
            self.portals.player.set_inside(pos, PLAYER_DELAY);
        }
        // `ServerPlayer.showEndCredits`: the player stays out of the world while the
        // poem rolls, then PERFORM_RESPAWN takes it to its respawn point.
        let play = host.play.entry(conn).or_default();
        if play.won_game {
            if std::mem::take(&mut play.respawn_requested) {
                play.won_game = false;
                self.start_server_trip(host, conn, Dim::Overworld, None);
                self.portals.player.cooldown = 10;
            }
            return;
        }
        let (end_portal, gateway) = self.inside_end_portals(at, WIDTH, HEIGHT);
        if end_portal && !riding {
            // EndPortalBlock.entityInside: the first exit from the End shows the credits.
            if self.dim == Dim::End && !host.hub.guest(conn).is_some_and(|g| g.seen_credits) {
                if let Some(g) = host.hub.guests.iter_mut().find(|g| g.conn == conn) {
                    g.seen_credits = true;
                }
                host.play.entry(conn).or_default().won_game = true;
                // ClientboundGameEventPacket.WIN_GAME.
                host.hub.send_to(
                    conn,
                    &C::GameEvent(play::GameEvent {
                        event: 4,
                        param: 0.0,
                    }),
                );
                return;
            }
            if self.portals.player.cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                self.start_server_trip(host, conn, target, None);
            }
            // Entity.setAsInsidePortal while cooling down restarts the cooldown.
            self.portals.player.cooldown = 10;
            return;
        }
        // EndGatewayBlock.entityInside: a gateway that is not cooling down
        // takes the player at once (no transition time) and cools down
        // (triggerCooldown), even for a player still on its own cooldown.
        if let Some(g) = gateway
            && self.gateway_enter(g)
        {
            if self.portals.player.cooldown == 0 {
                self.start_gateway_trip(host, conn, g);
            }
            // setPortalCooldown: after the trip, or again while cooling down.
            self.portals.player.cooldown = 10;
            return;
        }
        let rule = if invulnerable {
            "players_nether_portal_creative_delay"
        } else {
            "players_nether_portal_default_delay"
        };
        let transition = self.ui.rules.int(rule).max(0);
        let Some(entry) = self
            .portals
            .player
            .handle(!sleeping, transition, PLAYER_DELAY)
        else {
            return;
        };
        let plan = self.portal_plan(entry, at, WIDTH as f32, HEIGHT as f32, true);
        if !self.may_enter(plan.dim) {
            return;
        }
        self.start_server_trip(host, conn, plan.dim, Some(plan));
    }

    /// `Entity.handlePortal` of a vehicle server players ride, once through
    /// a portal (a vehicle's transition time is 0): the riders travel with
    /// it and ride it again on arrival (`Entity.teleportCrossDimension`), so
    /// a rider never waits for its own portal delay.
    fn carry_riders_through_portals(&mut self, host: &mut LanHost) {
        for (vehicle, entry) in self.take_carrying(self.dim) {
            let Some(plan) = self.vehicle_plan(vehicle, entry) else {
                continue;
            };
            if !self.may_enter(plan.dim) {
                continue;
            }
            let riders: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| g.vehicle == Some(vehicle) && !self.on_server_trip(g.conn))
                .map(|g| g.conn)
                .collect();
            // Entity.canTeleport: out of the End, every rider has seen the
            // credits.
            if plan.end
                && self.dim == Dim::End
                && riders
                    .iter()
                    .any(|c| !host.hub.guest(*c).is_some_and(|g| g.seen_credits))
            {
                continue;
            }
            // An End portal's exit is found as a player's (its own respawn
            // point out of the End).
            let (target, plan) = (plan.dim, (!plan.end).then_some(plan));
            for conn in riders {
                self.start_server_trip(host, conn, target, plan);
            }
        }
    }

    /// A Nether portal block a player's box is in.
    fn nether_portal_inside(&self, at: Vec3) -> Option<BlockPos> {
        let w = WIDTH / 2.0 - 1.0e-5;
        let lo = [at.x - w, at.y + 1.0e-5, at.z - w];
        let hi = [at.x + w, at.y + HEIGHT - 1.0e-5, at.z + w];
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    if &*self.block_name([x, y, z]) == "minecraft:nether_portal" {
                        return Some(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        None
    }

    /// The player moves to `target`'s level (opened if nobody was there)
    /// and is told it is there, at the approximate exit, while it loads. The
    /// level whose tick is running is put back in place afterwards.
    pub(super) fn start_server_trip(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        target: Dim,
        plan: Option<Plan>,
    ) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[i];
        let from = Vec3::new(g.position[0], g.position[1], g.position[2]);
        let yaw = g.yaw;
        // The vehicle the player rides travels with it: the player is its
        // passenger, and the vehicle is recreated in the other dimension,
        // where they ride it again (`Entity.teleportCrossDimension`). It
        // keeps its entity id, so the ride is never broken.
        let vehicle = g.vehicle.filter(|id| {
            self.mobs.iter().any(|m| m.id == *id)
                || self.boats.iter().any(|b| b.id == *id)
                || self.carts.iter().any(|c| c.id == *id)
        });
        let source = self.dim;
        let end = plan.is_none();
        // Coordinates scale by 8 between the Overworld and the Nether; an
        // End portal has fixed destinations.
        let scale = match (source, target) {
            (Dim::Overworld, Dim::Nether) | (Dim::End, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            _ => 1.0,
        };
        self.end_portal_travel = end;
        // Leaving the End: the player's own respawn point.
        let own = if end {
            self.as_guest(host, conn, |g| g.end_destination(target))
                .flatten()
        } else {
            None
        };
        let scaled = own
            .or_else(|| self.end_destination(target))
            .unwrap_or(Vec3::new(from.x * scale, from.y, from.z * scale));
        self.end_portal_travel = false;
        if let Err(e) = self.enter_level(Some(host), target) {
            log::error!("could not open {}: {e}", target.id());
            return;
        }
        let destination = match plan {
            Some(plan) => self.arrival_centre(&plan),
            None => scaled,
        };
        let at = [destination.x, destination.y, destination.z];
        let ctx = self.server_ctx();
        // Through a portal: the client waits on its loading screen until
        // the exit is found (`arrive_server_trips`). The cooldown is the one
        // Entity.handlePortal set before the teleport.
        host.hub
            .change_level(i, at, yaw, None, true, PLAYER_DELAY, &ctx);
        self.as_guest(host, conn, |g| {
            g.trigger_dimension(source, target, [from.x, from.y, from.z])
        });
        self.server_trips.push(Trip {
            conn,
            dim: target,
            source,
            target: destination,
            end,
            plan,
            vehicle,
            gateway: None,
            respawn: None,
            world_spawn: false,
            finder: None,
            placed: false,
        });
        if let Err(e) = self.enter_level(Some(host), source) {
            log::error!("could not reopen {}: {e}", source.id());
        }
    }

    /// A respawned player waits at its point outside the Overworld for the
    /// level to load around it.
    pub(super) fn respawn_trip(&mut self, conn: u32, config: RespawnConfig) {
        let [x, y, z] = config.pos;
        self.server_trips.push(Trip {
            conn,
            dim: config.dim,
            source: self.dim,
            target: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            end: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: Some(config),
            world_spawn: false,
            finder: None,
            placed: false,
        });
    }

    /// `findRespawnAndUseSpawnBlock` for the player in place, at its point
    /// in the level in place: where it stands up, facing the anchor (with
    /// the deplete sound) or the bed; else the point is forgotten
    /// (`block.minecraft.spawn.not_valid`).
    pub(super) fn resolve_server_respawn(&mut self, config: RespawnConfig) -> Option<(Vec3, f32)> {
        let [x, y, z] = config.pos;
        let anchor = self.is_anchor(self.world.block(x, y, z));
        match self.find_respawn_and_use(config, true) {
            Ok(Some((at, yaw))) => {
                if !config.forced && anchor {
                    let [x, y, z] = config.pos.map(f64::from);
                    self.audio.play(
                        "minecraft:block.respawn_anchor.deplete",
                        Some([x, y, z]),
                        1.0,
                        1.0,
                    );
                }
                Some((at, yaw))
            }
            Ok(None) | Err(_) => {
                self.respawn_config = None;
                self.system_message(Text::translate("block.minecraft.spawn.not_valid", &[]));
                None
            }
        }
    }

    /// TeleportTransition.createDefault and missingRespawnBlock: the player
    /// waits at the world spawn for its chunks, then PlayerSpawnFinder puts
    /// it on a free column within the respawn radius.
    pub(super) fn world_spawn_trip(&mut self, conn: u32) {
        let Some([x, y, z]) = self.level.world_spawn else {
            return;
        };
        self.server_trips.retain(|t| !t.is(conn));
        self.server_trips.push(Trip {
            conn,
            dim: Dim::Overworld,
            source: self.dim,
            target: Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            end: false,
            plan: None,
            vehicle: None,
            gateway: None,
            respawn: None,
            world_spawn: true,
            finder: None,
            placed: false,
        });
    }

    /// TeleportTransition.missingRespawnBlock: the world spawn, in the
    /// Overworld.
    fn respawn_at_world_spawn(&mut self, host: &mut LanHost, conn: u32) {
        let at = self.level.world_spawn.map_or(
            self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]),
            |[x, y, z]| [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5],
        );
        self.level_teleports.push(LevelTeleport {
            conn,
            dim: Dim::Overworld,
            at,
            rotation: None,
        });
        self.run_level_teleports(host);
        self.world_spawn_trip(conn);
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` and the teleport within
    /// the End (keeping the player's motion and look): straight to an exact
    /// exit, else once the destination has loaded around the player, who
    /// waits there (its exit is found among those blocks, or built).
    fn start_gateway_trip(&mut self, host: &mut LanHost, conn: u32, gateway: [i32; 3]) {
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let (job, destination) = self.gateway_job(gateway);
        let at = [destination.x, destination.y, destination.z];
        // Relative.union(DELTA, ROTATION).
        host.hub.teleport_keeping_motion(i, at);
        if let GatewayJob::Known { exact: true, .. } = job {
            return;
        }
        self.server_trips.push(Trip {
            conn,
            dim: self.dim,
            source: self.dim,
            target: destination,
            end: false,
            plan: None,
            vehicle: None,
            gateway: Some(job),
            respawn: None,
            world_spawn: false,
            finder: None,
            placed: false,
        });
    }

    /// The trips into the level in place whose destination has loaded
    /// around their target: the exit (the Nether portal found or built, the
    /// End platform, the Overworld's spawn column) and the teleport there.
    fn arrive_server_trips(&mut self, host: &mut LanHost) {
        let mut k = 0;
        while k < self.server_trips.len() {
            let trip = &self.server_trips[k];
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == trip.conn) else {
                // They left on the way.
                self.server_trips.remove(k);
                continue;
            };
            let (cx, cz) = (
                (trip.target.x.floor() as i32) >> 4,
                (trip.target.z.floor() as i32) >> 4,
            );
            if trip.dim != self.dim
                || !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz)))
            {
                k += 1;
                continue;
            }
            let mut trip = self.server_trips.remove(k);
            if trip.placed {
                continue;
            }
            // A portal's trip (`start_server_trip`): its client waits to land.
            let portal = trip.end || trip.plan.is_some();
            self.resolve_arrivals();
            let yaw = host.hub.guests[i].yaw;
            let (position, yaw) = if let Some(config) = trip.respawn {
                match self.as_guest(host, trip.conn, |g| g.resolve_server_respawn(config)) {
                    Some(Some(found)) => found,
                    _ => {
                        self.respawn_at_world_spawn(host, trip.conn);
                        continue;
                    }
                }
            } else if trip.world_spawn {
                // PlayerSpawnFinder.findSpawn around the world spawn.
                let suggestion =
                    [trip.target.x, trip.target.y, trip.target.z].map(|v| v.floor() as i32);
                match self.search_spawn(suggestion, &mut trip.finder) {
                    // The respawn already put the player on this column:
                    // vanilla teleports once, so no second move.
                    Ok(at) if host.hub.guests[i].position == [at.x, at.y, at.z] => continue,
                    Ok(at) => (at, yaw),
                    Err(chunk) => {
                        self.spawn_wait.push(chunk);
                        self.server_trips.insert(k, trip);
                        k += 1;
                        continue;
                    }
                }
            } else if let Some(job) = trip.gateway {
                match self.resolve_gateway(job, trip.target) {
                    Ok(at) => (at, yaw),
                    Err(e) => {
                        log::error!("End gateway exit: {e}");
                        (trip.target, yaw)
                    }
                }
            } else if trip.end {
                let arrival = match self.as_guest(host, trip.conn, |g| {
                    g.end_arrival(trip.target, &mut trip.finder)
                }) {
                    Some(arrival) => arrival,
                    None => self.end_arrival(trip.target, &mut trip.finder),
                };
                match arrival {
                    Ok(Some((at, turned))) => (at, turned.unwrap_or(yaw)),
                    Ok(None) => {
                        self.server_trips.insert(k, trip);
                        k += 1;
                        continue;
                    }
                    Err(e) => {
                        log::error!("End arrival: {e}");
                        (trip.target, yaw)
                    }
                }
            } else {
                let exit = trip
                    .plan
                    .as_ref()
                    .and_then(|plan| self.nether_exit_portal(plan));
                match (trip.plan, exit) {
                    (Some(plan), Some(exit)) => {
                        let (at, rotation) = self.arrival(&plan, exit);
                        (at, yaw + rotation)
                    }
                    _ => (trip.target, yaw),
                }
            };
            // The vehicle the player rode comes with it, and waits for it
            // where it arrives (`Entity.teleportCrossDimension` carries the
            // passengers).
            if let Some(vehicle) = trip.vehicle {
                self.arrive_vehicle(trip.source, vehicle, position, yaw);
            }
            // Player.getDimensionChangingDelay.
            host.play.entry(trip.conn).or_default().portal.cooldown = 10;
            let g = &mut host.hub.guests[i];
            g.yaw = yaw;
            // A new body at the exit: no fall distance or motion carried over.
            g.motion = None;
            let at = [position.x, position.y, position.z];
            if portal {
                host.hub.land(i, at);
            } else {
                host.hub.teleport(i, Some(at));
            }
        }
    }

    /// The vehicle the player rode arrives with it, at the point the player
    /// arrives: it keeps its entity id, so the ride is never broken
    /// (`Entity.teleportCrossDimension` recreates the vehicle in the other
    /// dimension, and its passengers ride it again).
    fn arrive_vehicle(&mut self, source: Dim, id: u32, at: Vec3, yaw: f32) {
        let Some(parked) = self.levels.parked_mut(source) else {
            return;
        };
        if let Some(b) = take_vehicle(&mut parked.boats, id, at, yaw) {
            self.boats.push(b);
        } else if let Some(c) = take_vehicle(&mut parked.carts, id, at, yaw) {
            self.carts.push(c);
        } else if let Some(m) = take_vehicle(&mut parked.mobs, id, at, yaw) {
            self.mobs.push(m);
        } else {
            return;
        }
        // Its memory (its UUID and the compound it was loaded with) moves
        // with it, so it is saved in this level as it was in the other.
        self.move_entity_memory(source, id);
        // The vehicle that came through a portal does not go back through it
        // for a while (`Entity.setPortalCooldown`).
        self.set_portal_cooldown(id);
    }

    /// The players the command just run sends to another level
    /// (`ServerPlayer.teleport` from TeleportCommand): a Respawn packet
    /// keeping all data and the teleport, with no portal sound. The server
    /// leaves the body alone until the level has loaded around it.
    pub(super) fn run_level_teleports(&mut self, host: &mut LanHost) {
        for t in std::mem::take(&mut self.level_teleports) {
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == t.conn) else {
                continue;
            };
            let source = self.dim;
            if let Err(e) = self.enter_level(Some(host), t.dim) {
                log::error!("could not open {}: {e}", t.dim.id());
                continue;
            }
            let g = &mut host.hub.guests[i];
            let [yaw, pitch] = t.rotation.unwrap_or([g.yaw, g.pitch]);
            let origin = g.position;
            // A new body there: no fall distance or motion carried over.
            g.motion = None;
            let cooldown = host.play.get(&t.conn).map_or(0, |p| p.portal.cooldown);
            let ctx = self.server_ctx();
            host.hub
                .change_level(i, t.at, yaw, Some(pitch), false, cooldown, &ctx);
            self.as_guest(host, t.conn, |g| g.trigger_dimension(source, t.dim, origin));
            self.server_trips.retain(|trip| !trip.is(t.conn));
            self.server_trips.push(Trip {
                conn: t.conn,
                dim: t.dim,
                source,
                target: Vec3::new(t.at[0], t.at[1], t.at[2]),
                end: false,
                plan: None,
                // A command teleports the player alone: it is not a
                // passenger any more (`ServerPlayer.teleport`).
                vehicle: None,
                gateway: None,
                respawn: None,
                world_spawn: false,
                finder: None,
                placed: true,
            });
            if let Err(e) = self.enter_level(Some(host), source) {
                log::error!("could not reopen {}: {e}", source.id());
            }
        }
    }

    /// The hub's view of the server for packets sent outside its own tick.
    pub(super) fn server_ctx(&mut self) -> TickCtx<'_> {
        let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
        let clock_rate = self.network_clock_rate();
        TickCtx {
            world: &mut self.world,
            host: None,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The vehicle a player rides travels with it through a portal: it
    /// leaves the level the player came from, keeps its entity id (so the
    /// ride is never broken) and stands where its rider arrives
    /// (`Entity.teleportCrossDimension` carries the passengers, and they
    /// ride the vehicle that is recreated in the other dimension).
    #[test]
    fn the_vehicle_a_player_rides_travels_with_it() {
        let mut boats = vec![boats::Boat::new(
            7,
            Arc::from("oak"),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        )];
        let mut carts: Vec<minecarts::Minecart> = Vec::new();
        let mut mobs: Vec<Mob> = Vec::new();
        let at = Vec3::new(1.5, 65.0, -2.5);
        let boat = take_vehicle(&mut boats, 7, at, 90.0)
            .expect("the vehicle the player rides travels with it");
        assert!(
            boats.is_empty(),
            "the vehicle left the level the player came from"
        );
        assert_eq!(boat.id, 7, "the vehicle keeps its entity id");
        assert_eq!(boat.position, at, "it stands where its rider arrives");
        assert_eq!(boat.yaw, 90.0, "it looks where its rider looks");
        assert_eq!(boat.velocity, Vec3::ZERO, "it comes with no motion");
        // A vehicle that is not in the level does not travel.
        assert!(take_vehicle(&mut carts, 9, at, 0.0).is_none());
        assert!(carts.is_empty());
        assert!(take_vehicle(&mut mobs, 9, at, 0.0).is_none());
        assert!(mobs.is_empty());
    }
}
