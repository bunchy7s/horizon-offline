//! Leads on boats and rafts (`AbstractBoat implements Leashable`): putting a
//! lead on one, taking it off, `Leashable.tickLeash` (the pull of the rope,
//! the lead snapping past twelve blocks), saving who holds it, and dropping
//! the lead when the boat breaks. The rope is drawn by `leads.rs`
//! (`LeashFeatureRenderer`, four ropes from a boat to a happy ghast).
//! A boat is also a holder (`Holder::Boat`): `Entity.interact`'s sneak click
//! ties the player's mobs and boats to it.
//!
//! The elastic pull of a ridden boat: `Leashable.tickLeash` runs on the
//! server only, and the server's boat that its rider's client drives takes
//! that client's position back (`handleMoveVehicle`), so vanilla never moves
//! it by the rope either. Here the pull lands on the server's copy and the
//! rider's client overwrites it the same way (`tick_boats`).
use super::{Game, boats::Boat};
use horizon_client::mobs::leash::{self, Holder, HolderState, Pending};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;

/// `AbstractBoat`'s `getBbWidth` and `getBbHeight`.
const WIDTH: f32 = 1.375;
const HEIGHT: f32 = 0.5625;

/// Where the other end of a boat's lead is.
enum Held {
    /// The holder is gone: the lead comes off (`canInteractWithLevel`).
    Gone,
    /// In another level or not loaded: nothing happens (`level() ==`).
    Elsewhere,
    /// Its state, and whether it holds a quad lead.
    At(HolderState, bool),
}

fn center(b: &Boat) -> Vec3 {
    Vec3::new(
        b.position.x,
        b.position.y + f64::from(HEIGHT) / 2.0,
        b.position.z,
    )
}
/// A boat as a lead's holder (`HolderState`), with its id.
pub(super) fn boat_state(b: &Boat) -> (u32, HolderState) {
    (
        b.id,
        HolderState {
            position: b.position,
            center: center(b),
            width: f64::from(WIDTH),
            height: f64::from(HEIGHT),
            yaw: b.yaw,
            velocity: b.velocity,
        },
    )
}
fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl Game<'_> {
    /// The player whose lead this is: the lead's `leash_player`, else the
    /// local player.
    fn boat_leash_player(&self, b: &Boat) -> Option<[i32; 4]> {
        b.leash_player.or(self.level.player_uuid)
    }
    fn boat_holder(&self, holder: Holder, player: Option<[i32; 4]>) -> Held {
        match holder {
            Holder::Player => {
                let want = player.or(self.level.player_uuid);
                // A server's guest (single player's window included) first.
                let guest = self.lan.as_ref().and_then(|host| {
                    let g = host.hub.guests.iter().find(|g| {
                        Some(horizon_client::lan::playerdata::uuid_ints(g.uuid)) == want
                    })?;
                    Some((host, g))
                });
                if let Some((host, g)) = guest {
                    if !host.hub.here(g) || !g.loaded {
                        return Held::Elsewhere;
                    }
                    if g.dead || g.spectator() {
                        return Held::Gone;
                    }
                    let bounds = g.bounds();
                    let (lo, hi) = (bounds.min_position(), bounds.max_position());
                    let moved = [0, 1, 2].map(|k| g.position[k] - g.previous_position[k]);
                    return Held::At(
                        HolderState {
                            position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                            center: Vec3::new(
                                (lo.x + hi.x) / 2.0,
                                (lo.y + hi.y) / 2.0,
                                (lo.z + hi.z) / 2.0,
                            ),
                            width: hi.x - lo.x,
                            height: hi.y - lo.y,
                            yaw: g.yaw,
                            velocity: Vec3::new(moved[0], moved[1], moved[2]),
                        },
                        false,
                    );
                }
                if want != self.level.player_uuid {
                    // A player who left: Entity.remove drops what they led.
                    return Held::Gone;
                }
                let Some(v) = self.player_view() else {
                    return Held::Elsewhere;
                };
                if !v.alive || v.spectator {
                    return Held::Gone;
                }
                let lo = v.bounds.min_position();
                let size = v.bounds.max_position().subtract(lo);
                Held::At(
                    HolderState {
                        position: v.position,
                        center: Vec3::new(v.position.x, lo.y + size.y / 2.0, v.position.z),
                        width: size.x,
                        height: size.y,
                        yaw: v.yaw,
                        velocity: v.velocity,
                    },
                    false,
                )
            }
            Holder::Mob(id) => {
                let Some(m) = self.mobs.iter().find(|m| m.id == id && m.alive()) else {
                    return Held::Gone;
                };
                let (lo, hi) = (m.bounds().min_position(), m.bounds().max_position());
                Held::At(
                    HolderState {
                        position: m.position,
                        center: Vec3::new(
                            (lo.x + hi.x) / 2.0,
                            (lo.y + hi.y) / 2.0,
                            (lo.z + hi.z) / 2.0,
                        ),
                        width: hi.x - lo.x,
                        height: hi.y - lo.y,
                        yaw: m.yaw,
                        velocity: m.velocity,
                    },
                    leash::quad_holder_offsets(m).is_some(),
                )
            }
            Holder::Boat(id) => match self.boats.iter().find(|o| o.id == id) {
                // supportQuadLeashAsHolder is the ghasts' alone.
                Some(o) => Held::At(boat_state(o).1, false),
                None => Held::Gone,
            },
            Holder::Knot(pos) => {
                let name = self
                    .res
                    .facts()
                    .get(self.world.block(pos[0], pos[1], pos[2]));
                let fence = self
                    .res
                    .items()
                    .block_tags
                    .get("minecraft:fences")
                    .is_some_and(|s| s.contains(&*name.name));
                if fence {
                    Held::At(leash::knot_state(pos), false)
                } else {
                    Held::Gone
                }
            }
        }
    }

    /// `Leashable.dropLeash` / `removeLeash` on a boat: the lead falls as an
    /// item (`drop`). The change reaches clients through the entity tracker.
    fn release_boat_leash(&mut self, b: &mut Boat, drop: bool) {
        if b.leash.take().is_none() {
            return;
        }
        b.leash_player = None;
        if drop {
            self.drop_lead_item([b.position.x, b.position.y, b.position.z]);
        }
    }

    /// `AbstractBoat.remove`: a boat that is destroyed drops its lead.
    pub(super) fn boat_removed(&mut self, b: &Boat) {
        let mut b = b.clone();
        let pending = b.leash_pending.take().is_some();
        if b.leash.is_some() || pending {
            let at = [b.position.x, b.position.y, b.position.z];
            b.leash = None;
            self.drop_lead_item(at);
        }
    }

    /// `Leashable.tickLeash` for a boat, at the end of `Entity.baseTick`.
    pub(super) fn tick_boat_leash(&mut self, b: &mut Boat) {
        b.age += 1;
        if let Some(pending) = b.leash_pending {
            self.restore_boat_leash(b, pending);
        }
        let Some(holder) = b.leash else {
            return;
        };
        let (state, quad) = match self.boat_holder(holder, b.leash_player) {
            Held::Elsewhere => return,
            // !leashHolder.canInteractWithLevel(): dropLeash with entity_drops
            // on, else removeLeash.
            Held::Gone => {
                let drops = self.ui.rules.bool("entity_drops");
                self.release_boat_leash(b, drops);
                return;
            }
            Held::At(state, quad) => (state, quad),
        };
        // Leashable.whenLeashedTo: `leashHolder.notifyLeashHolder(this)`; a
        // boat supports a quad lead.
        if let Holder::Mob(id) = holder
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            horizon_client::mobs::happy_ghast::notify_leash_holder(m);
        }
        let away = distance(state.center, center(b));
        if away > leash::SNAP {
            let p = state.position;
            self.lead_sound("break", [p.x, p.y, p.z]);
            self.release_boat_leash(b, true);
            return;
        }
        if away > leash::ELASTIC - state.width - f64::from(WIDTH)
            && let Some((force, torque)) = leash::elastic_pull(
                b.position,
                b.yaw,
                (f64::from(WIDTH), f64::from(HEIGHT)),
                &state,
                b.velocity,
                quad,
            )
        {
            b.leash_spin += 10.0 * torque;
            let relative = Vec3::new(
                state.velocity.x - b.velocity.x,
                state.velocity.y - b.velocity.y,
                state.velocity.z - b.velocity.z,
            );
            b.velocity = Vec3::new(
                b.velocity.x + force[0] * 0.8 + relative.x * 0.11,
                b.velocity.y + force[1] * 0.2 + relative.y * 0.11,
                b.velocity.z + force[2] * 0.8 + relative.z * 0.11,
            );
        }
        b.yaw = (f64::from(b.yaw) - b.leash_spin) as f32;
        b.leash_spin *= f64::from(self.boat_angular_friction(b));
    }

    /// `Leashable.angularFriction`.
    fn boat_angular_friction(&self, b: &Boat) -> f32 {
        if b.on_ground {
            let (x, z) = (b.position.x.floor() as i32, b.position.z.floor() as i32);
            let y = (b.position.y - 0.500_001).floor() as i32;
            self.res.facts().get(self.world.block(x, y, z)).friction * 0.91
        } else if b.in_liquid() {
            0.8
        } else {
            0.91
        }
    }

    /// `Leashable.restoreLeashFromSave`: a holder read from a save becomes
    /// real once its entity is found; one that never shows up in 100 ticks
    /// drops the lead.
    fn restore_boat_leash(&mut self, b: &mut Boat, pending: Pending) {
        let found = match pending {
            Pending::Knot(p) => Some((Holder::Knot(p), None)),
            Pending::Uuid(u) => {
                let guest = self.lan.as_ref().is_some_and(|h| {
                    h.hub
                        .guests
                        .iter()
                        .any(|g| horizon_client::lan::playerdata::uuid_ints(g.uuid) == u)
                });
                if guest || (self.level.player_uuid == Some(u) && self.player.is_some()) {
                    Some((Holder::Player, Some(u)))
                } else {
                    self.mob_by_uuid(u)
                        .map(|id| (Holder::Mob(id), None))
                        .or_else(|| self.boat_by_uuid(u).map(|id| (Holder::Boat(id), None)))
                }
            }
        };
        if let Some((holder, player)) = found {
            b.leash = Some(holder);
            b.leash_player = player;
            b.leash_pending = None;
        } else if b.age > 100 {
            b.leash_pending = None;
            self.drop_lead_item([b.position.x, b.position.y, b.position.z]);
        }
    }

    /// The `leash` tag of a boat about to be saved.
    pub(super) fn write_boat_leash(&mut self, tag: &mut Tag, index: usize) {
        let Tag::Compound(map) = tag else { return };
        map.remove(&"leash".into());
        let b = &self.boats[index];
        let (holder, pending, player) = (b.leash, b.leash_pending, self.boat_leash_player(b));
        let value = match (holder, pending) {
            (Some(Holder::Mob(id)), _) => {
                let uuid = self.mob_uuid(id);
                leash::to_nbt(Holder::Mob(id), Some(uuid))
            }
            (Some(Holder::Boat(id)), _) => {
                let uuid = self.uuid_for(id);
                leash::to_nbt(Holder::Boat(id), Some(uuid))
            }
            (Some(h), _) => leash::to_nbt(h, player),
            (None, Some(Pending::Knot(p))) => leash::to_nbt(Holder::Knot(p), None),
            (None, Some(Pending::Uuid(u))) => leash::to_nbt(Holder::Player, Some(u)),
            _ => None,
        };
        if let Some(v) = value {
            map.insert("leash".into(), v);
        }
    }

    /// Whether the player acting holds this boat's lead.
    fn boat_led_by_player(&mut self, i: usize) -> bool {
        let me = self.acting_player_uuid();
        let b = &self.boats[i];
        b.leash == Some(Holder::Player) && self.boat_leash_player(b) == Some(me)
    }

    /// The boat with this UUID (`ServerLevel.getEntity`).
    pub(super) fn boat_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.persist
            .uuids
            .iter()
            .find(|(id, u)| **u == uuid && self.boats.iter().any(|b| b.id == **id))
            .map(|(id, _)| *id)
    }

    /// The boats on the acting player's lead within 32 blocks of a point
    /// (`Leashable.leashableInArea`).
    pub(super) fn player_led_boats(&mut self, around: glam::DVec3) -> Vec<usize> {
        (0..self.boats.len())
            .filter(|&i| {
                let c = center(&self.boats[i]);
                self.boat_led_by_player(i)
                    && (c.x - around.x).abs() <= 16.0 + f64::from(WIDTH) / 2.0
                    && (c.y - around.y).abs() <= 16.0
                    && (c.z - around.z).abs() <= 16.0 + f64::from(WIDTH) / 2.0
            })
            .collect()
    }

    /// The boats whose lead hangs on `holder`.
    pub(super) fn boats_on(&self, holder: Holder) -> Vec<usize> {
        (0..self.boats.len())
            .filter(|&i| self.boats[i].leash == Some(holder))
            .collect()
    }

    /// `Leashable.canHaveALeashAttachedTo` for the boat at `i` and an entity
    /// whose box centre is `at`.
    fn boat_can_attach(&self, i: usize, at: glam::DVec3) -> bool {
        let c = center(&self.boats[i]);
        distance(Vec3::new(at.x, at.y, at.z), c) <= leash::SNAP
    }

    /// The acting player's boats on a lead go to `to` (`Entity.interact` of a
    /// sneak click on a mob, `LeadItem.bindPlayerMobs`, the knot's
    /// `interact`), the ones in reach of `at`. Whether any did.
    pub(super) fn tie_player_boats(&mut self, at: glam::DVec3, to: Holder) -> bool {
        let mut any = false;
        for i in self.player_led_boats(at) {
            // Leashable.setLeashedTo: `this != holder`.
            if to == Holder::Boat(self.boats[i].id) {
                continue;
            }
            if self.boat_can_attach(i, at) {
                self.boats[i].leash = Some(to);
                self.boats[i].leash_player = None;
                any = true;
            }
        }
        any
    }

    /// `LeashFenceKnotEntity.interact`: the boats on the knot go back on the
    /// acting player's lead. Whether any did.
    pub(super) fn take_knot_boats(&mut self, knot: [i32; 3]) -> bool {
        let me = self.acting_player_uuid();
        let player = self.player_center();
        let mut any = false;
        for i in self.boats_on(Holder::Knot(knot)) {
            if self.boat_can_attach(i, player) {
                self.boats[i].leash = Some(Holder::Player);
                self.boats[i].leash_player = Some(me);
                any = true;
            }
        }
        any
    }

    /// `Leashable.dropLeash` for each boat in `list`.
    pub(super) fn drop_boat_leashes(&mut self, list: &[usize]) {
        for &i in list {
            let mut b = self.boats[i].clone();
            self.release_boat_leash(&mut b, true);
            self.boats[i] = b;
        }
    }

    /// `Entity.interact`'s lead part for the boat at `i`; whether it took
    /// the click.
    pub(super) fn interact_boat_leash(&mut self, i: usize) -> bool {
        let item = self.held().map(|s| s.item.to_string());
        let c = center(&self.boats[i]);
        let at = [c.x, c.y, c.z];
        let id = self.boats[i].id;
        // The sneak branch: the player's mobs and boats on leads are re-tied
        // to this boat.
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        if sneaking
            && self.tie_player_leads(glam::DVec3::new(c.x, c.y, c.z), Holder::Boat(id), None)
        {
            self.lead_game_event("minecraft:entity_action", at);
            self.lead_sound("tied", at);
            return true;
        }
        // Entity.shearOffAllLeashConnections: its own lead and every one
        // hanging on it.
        if item.as_deref() == Some("minecraft:shears") {
            let held_mobs: Vec<u32> = self
                .mobs
                .iter()
                .filter(|m| m.extra.leash == Some(Holder::Boat(id)))
                .map(|m| m.id)
                .collect();
            let mut held_boats = self.boats_on(Holder::Boat(id));
            if self.boats[i].leash.is_some() {
                held_boats.push(i);
            }
            if !held_mobs.is_empty() || !held_boats.is_empty() {
                for m in held_mobs {
                    self.break_leash(m, true, false);
                }
                self.drop_boat_leashes(&held_boats);
                self.lead_game_event("minecraft:shear", at);
                self.audio
                    .play("minecraft:item.shears.snip", Some(at), 1.0, 1.0);
                self.damage_held_tool();
                return true;
            }
        }
        if self.boat_led_by_player(i) {
            // hasInfiniteMaterials: removeLeash, else dropLeash.
            let drop = !self.creative;
            let mut b = self.boats[i].clone();
            self.release_boat_leash(&mut b, drop);
            self.boats[i] = b;
            self.lead_game_event("minecraft:entity_interact", at);
            self.lead_sound("untied", at);
            return true;
        }
        let taken_by_player = self.boats[i].leash == Some(Holder::Player);
        if item.as_deref() == Some("minecraft:lead") && !taken_by_player {
            let player = self.player_center();
            if self.boat_can_attach(i, player) {
                let mut b = self.boats[i].clone();
                self.release_boat_leash(&mut b, true);
                b.leash = Some(Holder::Player);
                b.leash_player = Some(self.acting_player_uuid());
                self.boats[i] = b;
                self.lead_sound("tied", at);
                // The lead is always used up.
                if let Some(stack) = self.held_mut().as_mut() {
                    stack.count -= 1;
                    if stack.count == 0 {
                        *self.held_mut() = None;
                    }
                }
                return true;
            }
        }
        false
    }

    /// The leads on boats that hang on a player or a mob, for the entity
    /// tracker: the holder's entity id (`ClientboundSetEntityLinkPacket`).
    pub(super) fn boat_leash_link(
        &self,
        b: &Boat,
        host_entity: Option<i32>,
        guests: &[(u128, i32)],
    ) -> Option<i32> {
        match b.leash? {
            Holder::Player => {
                let uuid = self.boat_leash_player(b);
                match guests
                    .iter()
                    .find(|(u, _)| Some(horizon_client::lan::playerdata::uuid_ints(*u)) == uuid)
                {
                    Some(&(_, id)) => Some(id),
                    None if uuid == self.level.player_uuid => host_entity,
                    None => None,
                }
            }
            Holder::Mob(id) | Holder::Boat(id) => Some(id as i32),
            Holder::Knot(p) => self.knot_ids.get(&p).map(|&id| id as i32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boat_is_centred_half_its_height_up() {
        let b = Boat::new(1, "oak".into(), Vec3::new(1.0, 64.0, 2.0), 0.0);
        let c = center(&b);
        assert_eq!((c.x, c.z), (1.0, 2.0));
        assert!((c.y - 64.28125).abs() < 1e-9);
    }
    #[test]
    fn a_boat_holds_a_lead_from_its_centre() {
        let mut b = Boat::new(7, "oak".into(), Vec3::new(1.0, 64.0, 2.0), 90.0);
        b.velocity = Vec3::new(0.1, 0.0, 0.0);
        let (id, s) = boat_state(&b);
        assert_eq!(id, 7);
        assert!((s.center.y - 64.28125).abs() < 1e-9);
        assert!((s.width - 1.375).abs() < 1e-9 && (s.height - 0.5625).abs() < 1e-9);
        assert_eq!((s.yaw, s.velocity.x), (90.0, 0.1));
    }
    #[test]
    fn a_boat_holder_is_an_entity_not_a_player() {
        assert_eq!(Holder::Boat(7).who(), None);
        assert_ne!(Holder::Boat(7), Holder::Mob(7));
    }
    #[test]
    fn distances_are_euclidean() {
        let d = distance(Vec3::new(0.0, 0.0, 0.0), Vec3::new(3.0, 4.0, 0.0));
        assert!((d - 5.0).abs() < 1e-12);
    }
}
