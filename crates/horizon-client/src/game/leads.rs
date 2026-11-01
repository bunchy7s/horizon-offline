//! Leads on the game side: putting them on and taking them off mobs
//! (`Entity.interact`'s leash part), fence knots (`LeadItem.useOn` and
//! `LeashFenceKnotEntity.interact`), loading and saving who holds what,
//! breaking, and drawing the rope (`LeashFeatureRenderer`) and the knot.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::leash::{self, Holder, Pending},
};
use horizon_world::nbt::Tag;

/// `Vec3.yRot(angle)`.
/// `LocalPlayer.getRopeHoldPosition`'s `Mth.lerp(partialTick * 0.5F, getYRot(),
/// yRotO)` and the same for the pitch, in radians.
fn rope_hold_angles(now: (f32, f32), old: (f32, f32), alpha: f32) -> (f64, f64) {
    let t = alpha * 0.5;
    let lerp = |a: f32, b: f32| a + t * (b - a);
    (
        f64::from(lerp(now.0, old.0) * (std::f32::consts::PI / 180.0)),
        f64::from(lerp(now.1, old.1) * (std::f32::consts::PI / 180.0)),
    )
}
fn rot_y(v: glam::DVec3, angle: f64) -> glam::DVec3 {
    let (s, c) = (angle.sin(), angle.cos());
    glam::DVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}
/// `Entity.getRopeHoldPosition` of a boat: 0.7 of its eye height (0.5625) up.
fn boat_hold() -> glam::DVec3 {
    glam::DVec3::new(0.0, f64::from(0.5625_f32) * 0.7, 0.0)
}
fn pack_light(light: u8) -> u32 {
    (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8)
}
fn abgr(r: f32, g: f32, b: f32) -> u32 {
    let c = |v: f32| (v * 255.0) as u32 & 255;
    0xFF00_0000 | c(b) << 16 | c(g) << 8 | c(r)
}
/// One leashed entity's rope, as `EntityRenderer.extractRenderState` reads it.
struct Rope {
    holder: Holder,
    /// `getPosition(partialTicks)`.
    here: glam::DVec3,
    /// `getPreciseBodyRotation` in radians.
    body: f64,
    /// `getLeashOffset`.
    offset: [f64; 3],
    /// `getQuadLeashOffsets`, when the entity supports a quad lead.
    quad: Option<[[f64; 3]; 4]>,
}
impl Game<'_> {
    pub(super) fn lead_sound(&mut self, sound: &str, at: [f64; 3]) {
        self.audio
            .play(&format!("minecraft:item.lead.{sound}"), Some(at), 1.0, 1.0);
    }
    /// `Leashable.canHaveALeashAttachedTo` for the mob at `index` and a
    /// holder whose box centre is `center`: within snapping range and
    /// leashable.
    pub(super) fn can_attach_lead(&self, index: usize, center: glam::DVec3) -> bool {
        let m = &self.mobs[index];
        if !leash::can_be_leashed(m) || !m.alive() {
            return false;
        }
        let b = m.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let mine = glam::DVec3::new(
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        );
        mine.distance(center) <= leash::SNAP
    }
    /// `Entity.gameEvent(event, player)` for a lead: the acting player is the source.
    pub(super) fn lead_game_event(&mut self, event: &'static str, at: [f64; 3]) {
        let ctx = self.player_context(None);
        self.post_game_event(event, at, ctx);
    }
    /// `Entity.interact`'s sneak branch: the mobs and boats on the player's
    /// lead within reach of `center` that `canHaveALeashAttachedTo` the
    /// entity (`skip` is the entity itself) are re-tied to `to`. Whether any
    /// was.
    pub(super) fn tie_player_leads(
        &mut self,
        center: glam::DVec3,
        to: Holder,
        skip: Option<usize>,
    ) -> bool {
        let mut any = false;
        for j in self.player_leashed(center) {
            if Some(j) != skip && self.can_attach_lead(j, center) {
                self.mobs[j].extra.leash = Some(to);
                any = true;
            }
        }
        // The boats on the player's lead too (game/boat_leads.rs).
        any | self.tie_player_boats(center, to)
    }
    pub(super) fn player_center(&self) -> glam::DVec3 {
        self.player.as_ref().map_or(glam::DVec3::ZERO, |p| {
            glam::DVec3::new(
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            )
        })
    }
    /// The mobs on the acting player's lead within 32 blocks of a point
    /// (`Leashable.leashableInArea` with `getLeashHolder() == player`).
    pub(super) fn player_leashed(&self, around: glam::DVec3) -> Vec<usize> {
        let acting = self.acting_uuid();
        (0..self.mobs.len())
            .filter(|&i| {
                let m = &self.mobs[i];
                m.extra.leash == Some(Holder::Player)
                    && m.extra.leash_player.is_none_or(|p| p == acting)
                    && (m.position.x - around.x).abs() <= 16.0
                    && (m.position.y - around.y).abs() <= 16.0
                    && (m.position.z - around.z).abs() <= 16.0
            })
            .collect()
    }
    /// `Entity.spawnAtLocation(level, Items.LEAD)`: the item entity draws its
    /// motion from its own random, not the level's.
    pub(super) fn drop_lead_item(&mut self, at: [f64; 3]) {
        let v = super::dispenser::new_item_motion();
        self.spawn_item(ItemStack::new("minecraft:lead", 1), at, v, 10);
    }
    /// `Leashable.dropLeash` / `removeLeash`: the lead falls as an item.
    pub(super) fn break_leash(&mut self, id: u32, drop: bool, sound: bool) {
        self.drop_leash(id, drop, sound, false);
    }
    /// `break_leash` by the acting player's use (untying it, shears).
    fn break_leash_used(&mut self, id: u32, drop: bool) {
        self.drop_leash(id, drop, false, true);
    }
    fn drop_leash(&mut self, id: u32, drop: bool, sound: bool, used: bool) {
        let killer = self.lead_killers.remove(&id).map(|(killer, _)| killer);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if horizon_client::mobs::leash::remove(m).is_none() {
            return;
        }
        // Leashable.tickLeash gates a dying entity's lead drop on entity_drops.
        let drop = drop && (m.alive() || self.ui.rules.bool("entity_drops"));
        let at = [m.position.x, m.position.y, m.position.z];
        let kind = super::native_hooks::mob_kind(m);
        if sound {
            self.lead_sound("break", at);
        }
        if !drop {
            return;
        }
        // The mods' say when a player is behind it: the user, else the
        // killer of a mob that died leashed.
        let player = if used {
            Some(self.native_acting_id())
        } else {
            killer
        };
        let Some(player) = player else {
            self.drop_lead_item(at);
            return;
        };
        let mut drops = vec![ItemStack::new("minecraft:lead", 1)];
        let context = super::native_hooks::NativeLootDrops {
            table: "",
            dimension: self.dim.id(),
            origin: at,
            block: None,
            entity: Some((id, &kind)),
            player: Some(player),
            tool: None,
            explosion: None,
            used,
        };
        self.native_loot_drops(&context, &mut drops);
        for stack in drops {
            if stack.count > 0 && &*stack.item != "minecraft:air" {
                let v = super::dispenser::new_item_motion();
                self.spawn_item(stack, at, v, 10);
            }
        }
    }
    /// Holders read from a save become real once the entity is found; a
    /// holder that never shows up drops the lead after 100 ticks.
    pub(super) fn tick_leads(&mut self) {
        // A corpse gone without dropping its lead takes its killer along.
        // Only this level's: the others' mobs aren't in place.
        if !self.lead_killers.is_empty() {
            let (mobs, dim) = (&self.mobs, self.dim);
            self.lead_killers
                .retain(|id, (_, at)| *at != dim || mobs.iter().any(|m| m.id == *id && !m.removed));
        }
        let player_uuid = self.level.player_uuid.unwrap_or([0, 0, 0, 1]);
        for i in 0..self.mobs.len() {
            let Some(pending) = self.mobs[i].extra.leash_pending else {
                continue;
            };
            let holder = match pending {
                Pending::Knot(p) => Some(Holder::Knot(p)),
                Pending::Uuid(u) if u == player_uuid => Some(Holder::Player),
                Pending::Uuid(u) => self
                    .mob_by_uuid(u)
                    .map(Holder::Mob)
                    .or_else(|| self.boat_by_uuid(u).map(Holder::Boat)),
            };
            let m = &mut self.mobs[i];
            if let Some(h) = holder {
                if h == Holder::Player {
                    m.extra.leash_player = Some(player_uuid);
                }
                m.extra.leash = Some(h);
                m.extra.leash_pending = None;
            } else if m.age > 100 {
                m.extra.leash_pending = None;
                let at = [m.position.x, m.position.y, m.position.z];
                self.drop_lead_item(at);
            }
        }
    }
    /// The `leash` tag of a mob about to be saved.
    pub(super) fn write_leash(&mut self, tag: &mut Tag, index: usize) {
        let Tag::Compound(map) = tag else { return };
        map.remove(&"leash".into());
        let m = &self.mobs[index];
        let holder = m.extra.leash;
        let pending = m.extra.leash_pending;
        let uuid = match (holder, pending) {
            (Some(Holder::Player), _) => Some(self.level.player_uuid.unwrap_or([0, 0, 0, 1])),
            (Some(Holder::Mob(id) | Holder::Boat(id)), _) => Some(self.mob_uuid(id)),
            (_, Some(Pending::Uuid(u))) => Some(u),
            _ => None,
        };
        let value = match (holder, pending) {
            (Some(h), _) => leash::to_nbt(h, uuid),
            (None, Some(Pending::Knot(p))) => leash::to_nbt(Holder::Knot(p), None),
            (None, Some(Pending::Uuid(u))) => leash::to_nbt(Holder::Player, Some(u)),
            _ => None,
        };
        if let Some(v) = value {
            map.insert("leash".into(), v);
        }
    }
    /// `Entity.interact`'s leash part for a mob. Returns whether it was used.
    pub(super) fn interact_leash(&mut self, index: usize, item: &str) -> bool {
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let id = self.mobs[index].id;
        let at = self.mobs[index].center();
        if !self.mobs[index].alive() {
            return false;
        }
        // Sneaking with mobs in hand: tie them to this one.
        if sneaking && leash::can_be_leashed(&self.mobs[index]) && !self.mobs[index].is_baby() {
            let b = self.mobs[index].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let center = glam::DVec3::new(
                (lo.x + hi.x) / 2.0,
                (lo.y + hi.y) / 2.0,
                (lo.z + hi.z) / 2.0,
            );
            if self.tie_player_leads(center, Holder::Mob(id), Some(index)) {
                self.lead_game_event("minecraft:entity_action", at);
                self.lead_sound("tied", at);
                return true;
            }
        }
        if item == "minecraft:shears" {
            let mut dropped = false;
            if self.mobs[index].extra.leash.is_some() {
                self.break_leash_used(id, true);
                dropped = true;
            }
            let others: Vec<u32> = self
                .mobs
                .iter()
                .filter(|m| m.extra.leash == Some(Holder::Mob(id)))
                .map(|m| m.id)
                .collect();
            for o in others {
                self.break_leash_used(o, true);
                dropped = true;
            }
            let boats = self.boats_on(Holder::Mob(id));
            dropped |= !boats.is_empty();
            self.drop_boat_leashes(&boats);
            if dropped {
                self.lead_game_event("minecraft:shear", at);
                self.audio
                    .play("minecraft:item.shears.snip", Some(at), 1.0, 1.0);
                self.damage_held_tool();
                return true;
            }
        }
        // Mob.checkAndHandleImportantInteractions: only the player the lead
        // is tied to unties it (`getLeashHolder() == player`).
        if self.mobs[index].extra.leash == Some(Holder::Player)
            && self.mobs[index]
                .extra
                .leash_player
                .is_none_or(|p| p == self.acting_player_uuid())
        {
            let creative = self.creative;
            self.break_leash_used(id, !creative);
            self.lead_game_event("minecraft:entity_interact", at);
            self.lead_sound("untied", at);
            return true;
        }
        if item == "minecraft:lead" {
            let center = self.player_center();
            if self.can_attach_lead(index, center) {
                if self.mobs[index].extra.leash.is_some() {
                    self.break_leash_used(id, true);
                }
                let holder = self.acting_player_uuid();
                self.mobs[index].extra.leash = Some(Holder::Player);
                self.mobs[index].extra.leash_player = Some(holder);
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
    /// `LeadItem.useOn` and `LeashFenceKnotEntity.interact` on a fence.
    pub(super) fn interact_knot(&mut self) -> bool {
        let Some(hit) = self.pick() else {
            return false;
        };
        let name = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        if !self
            .res
            .items()
            .block_tags
            .get("minecraft:fences")
            .is_some_and(|s| s.contains(&*name.name))
        {
            return false;
        }
        let knot_pos = hit.pos;
        let knot = leash::knot_state(knot_pos).center;
        let center = glam::DVec3::new(knot.x, knot.y, knot.z);
        let at = [center.x, center.y, center.z];
        let existing: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| self.mobs[i].extra.leash == Some(Holder::Knot(knot_pos)))
            .collect();
        let existing_boats = self.boats_on(Holder::Knot(knot_pos));
        // `LeashFenceKnotEntity.interact` runs only when the ray meets the knot
        // before the post: the knot's box (`recalculateBoundingBox`, 0.375 wide,
        // y + 0.375 to y + 0.875) is wider than the post, so a hit point on the
        // post inside that box went through the knot first. Any other click on
        // the fence is `FenceBlock.useWithoutItem` / `LeadItem.useOn`, which only
        // ties the player's own mobs (`LeadItem.bindPlayerMobs`).
        let knot_hit = !(existing.is_empty() && existing_boats.is_empty()) && {
            let box_ = leash::knot_state(knot_pos);
            let half = box_.width / 2.0 + 1.0e-6;
            (hit.point[0] - box_.position.x).abs() <= half
                && (hit.point[2] - box_.position.z).abs() <= half
                && hit.point[1] >= box_.position.y - 1.0e-6
                && hit.point[1] <= box_.position.y + box_.height + 1.0e-6
                // The knot is an entity: a hit on it counts only within the
                // entity interaction range (`filterHitResult`), shorter than
                // the block range the fence was hit at.
                && self.eye_position().is_some_and(|eye| {
                    let d = [0, 1, 2].map(|i| hit.point[i] - eye[i]);
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= self.entity_reach * self.entity_reach
                })
        };
        let held_item = self.held().map(|s| s.item.to_string());
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        // Shears snip every lead on the knot.
        if knot_hit && held_item.as_deref() == Some("minecraft:shears") {
            for i in existing {
                let id = self.mobs[i].id;
                self.break_leash_used(id, true);
            }
            self.drop_boat_leashes(&existing_boats);
            self.audio
                .play("minecraft:item.shears.snip", Some(at), 1.0, 1.0);
            self.damage_held_tool();
            return true;
        }
        let in_hand = self.player_leashed(center);
        let boats_in_hand = self.player_led_boats(center);
        // The knot is only made for the lead item, or when mobs already hang on it.
        if held_item.as_deref() != Some("minecraft:lead")
            && existing.is_empty()
            && existing_boats.is_empty()
            && in_hand.is_empty()
            && boats_in_hand.is_empty()
        {
            return false;
        }
        let mut attached = self.tie_player_boats(center, Holder::Knot(knot_pos));
        for j in in_hand {
            if self.can_attach_lead(j, center) {
                self.mobs[j].extra.leash = Some(Holder::Knot(knot_pos));
                attached = true;
            }
        }
        if !attached && !sneaking && knot_hit {
            let player = self.player_center();
            let holder = self.acting_player_uuid();
            for i in existing {
                if self.can_attach_lead(i, player) {
                    self.mobs[i].extra.leash = Some(Holder::Player);
                    self.mobs[i].extra.leash_player = Some(holder);
                    attached = true;
                }
            }
            attached |= self.take_knot_boats(knot_pos);
        }
        if attached {
            self.lead_sound("tied", at);
            self.swing_arm();
        }
        attached
    }
    /// `LeashFeatureRenderer` for every lead and `LeashKnotRenderer` for the knots.
    pub(super) fn leash_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let atlas = &self.gui.atlas;
        let white = atlas.font.white();
        let uv = [
            [white.u0, white.v1],
            [white.u1, white.v1],
            [white.u1, white.v0],
            [white.u0, white.v0],
        ];
        let lerp3 = |a: horizon_core::geometry::Vec3, b: horizon_core::geometry::Vec3| {
            glam::DVec3::new(
                a.x + (b.x - a.x) * alpha,
                a.y + (b.y - a.y) * alpha,
                a.z + (b.z - a.z) * alpha,
            )
        };
        let mut knots: Vec<[i32; 3]> = Vec::new();
        // EntityRenderer.extractRenderState: a leashed mob or boat (an
        // `AbstractBoat` is `Leashable` too) with where it hangs.
        let body_of = |previous: f32, current: f32| {
            f64::from(
                previous + horizon_client::mobs::living::wrap(current - previous) * alpha as f32,
            )
            .to_radians()
        };
        let mut ropes: Vec<Rope> = Vec::new();
        for m in &self.mobs {
            if let Some(holder) = m.extra.leash {
                ropes.push(Rope {
                    holder,
                    here: lerp3(m.previous, m.position),
                    body: body_of(m.previous_body_yaw, m.body_yaw),
                    offset: leash::leash_offset(m),
                    quad: leash::supports_quad_leash(m.species())
                        .then(|| leash::quad_leash_offsets(m)),
                });
            }
        }
        for b in &self.boats {
            if let Some(holder) = b.leash {
                ropes.push(Rope {
                    holder,
                    here: lerp3(b.previous, b.position),
                    // Entity.getPreciseBodyRotation: the boat's yaw.
                    body: body_of(b.previous_yaw, b.yaw),
                    offset: leash::boat_leash_offset(),
                    quad: Some(leash::boat_quad_leash_offsets()),
                });
            }
        }
        for rope in ropes {
            let Rope {
                holder,
                here,
                body,
                offset,
                quad,
            } = rope;
            // One lead from the leash offset to the holder's rope hold
            // position, or, when the holder is a happy ghast and the leashee
            // supports a quad lead, four from `getQuadLeashOffsets` to
            // `getQuadLeashHolderOffsets`.
            let mut ends: Vec<(glam::DVec3, glam::DVec3, bool)> = Vec::new();
            let quad = match (holder, quad) {
                (Holder::Mob(id), Some(offsets)) => self
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .and_then(|o| leash::quad_holder_offsets(o).map(|h| (o, h, offsets))),
                _ => None,
            };
            if let Some((roper, holder_points, own)) = quad {
                let roper_body = body_of(roper.previous_body_yaw, roper.body_yaw);
                let holder_pos = lerp3(roper.previous, roper.position);
                for (mine, theirs) in own.iter().zip(holder_points) {
                    let start = here + rot_y(glam::DVec3::from_array(*mine), -body);
                    let end = holder_pos + rot_y(glam::DVec3::from_array(theirs), -roper_body);
                    ends.push((start, end, false));
                }
            } else {
                let offset = rot_y(glam::DVec3::from_array(offset), -body);
                let start = here + offset;
                let end = match holder {
                    Holder::Knot(p) => {
                        if !knots.contains(&p) {
                            knots.push(p);
                        }
                        glam::DVec3::new(
                            f64::from(p[0]) + 0.5,
                            f64::from(p[1]) + 0.375 + 0.2,
                            f64::from(p[2]) + 0.5,
                        )
                    }
                    Holder::Mob(id) => {
                        let Some(o) = self.mobs.iter().find(|o| o.id == id) else {
                            continue;
                        };
                        lerp3(o.previous, o.position)
                            + glam::DVec3::new(0.0, o.eye_height() * 0.7, 0.0)
                    }
                    Holder::Boat(id) => {
                        let Some(o) = self.boats.iter().find(|o| o.id == id) else {
                            continue;
                        };
                        lerp3(o.previous, o.position) + boat_hold()
                    }
                    Holder::Player => {
                        let Some(p) = self.player.as_ref() else {
                            continue;
                        };
                        self.player_rope_hold(p, alpha)
                    }
                };
                ends.push((start, end, true));
            }
            let light = |p: glam::DVec3| {
                self.world
                    .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
            };
            // Block and sky light averaged between the two ends.
            let (a, b) = (light(ends[0].0), light(ends[0].1));
            let packed = pack_light((((a >> 4) + (b >> 4)) / 2) << 4 | (((a & 15) + (b & 15)) / 2));
            for (start, end, slack) in ends {
                let d = end - start;
                let k = 1.0 / (d.x * d.x + d.z * d.z).sqrt().max(1e-6) * 0.05 / 2.0;
                let (dx_off, dz_off) = (d.z * k, d.x * k);
                let point = |step: i32, fudge: f64| {
                    let progress = f64::from(step) / 24.0;
                    // LeashFeatureRenderer.addVertexPair: a quad lead is straight.
                    let y = if !slack {
                        d.y * progress
                    } else if d.y > 0.0 {
                        d.y * progress * progress
                    } else {
                        d.y - d.y * (1.0 - progress) * (1.0 - progress)
                    };
                    let base = start + glam::DVec3::new(d.x * progress, y, d.z * progress);
                    (
                        base + glam::DVec3::new(-dx_off, fudge, dz_off),
                        base + glam::DVec3::new(dx_off, 0.05 - fudge, -dz_off),
                    )
                };
                for (fudge, backwards) in [(0.05, false), (0.0, true)] {
                    for step in 0..24 {
                        let shade = |s: i32| {
                            let dark = s % 2 == i32::from(backwards);
                            let f = if dark { 0.7 } else { 1.0 };
                            abgr(0.5 * f, 0.4 * f, 0.3 * f)
                        };
                        let (a0, b0) = point(step, fudge);
                        let (a1, b1) = point(step + 1, fudge);
                        let color = shade(step);
                        // Both faces: the leash is not culled.
                        for quad in [[a0, b0, b1, a1], [a1, b1, b0, a0]] {
                            for (v, uv) in quad.iter().zip(uv) {
                                out.push(Vertex {
                                    normal: [0; 4],
                                    pos: (*v - camera).as_vec3().to_array(),
                                    uv,
                                    layer: 0,
                                    color,
                                    light: packed,

                                    glint_uv: [0.0, 0.0],
                                });
                            }
                        }
                    }
                }
            }
        }
        // The knots: a 6 by 8 by 6 pixel block on the fence post.
        static KNOT: std::sync::OnceLock<Option<horizon_client::mobs::Part>> =
            std::sync::OnceLock::new();
        let model = KNOT.get_or_init(|| {
            horizon_client::mobs::load_model(&self.options.resources, "minecraft:leash_knot#main")
        });
        let Some(model) = model else { return };
        let Some(r) = atlas.sprites.get("entity/lead_knot/lead_knot").copied() else {
            return;
        };
        for p in knots {
            let state = leash::knot_state(p);
            let at =
                glam::DVec3::new(state.position.x, state.position.y, state.position.z) - camera;
            let base = glam::Mat4::from_translation(at.as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = horizon_client::mobs::anim::Poses::new(model);
            let light = pack_light(self.world.light(p[0], p[1], p[2]));
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                [r.u0, r.v0, r.u1, r.v1],
                0xFFFF_FFFF,
                light,
                0,
                out,
            );
        }
    }
    /// `Player.getRopeHoldPosition` (and `LocalPlayer`'s in first person).
    fn player_rope_hold(
        &self,
        p: &horizon_client::motion::PlayerMotion,
        alpha: f64,
    ) -> glam::DVec3 {
        let pos = glam::DVec3::new(p.position.x, p.position.y, p.position.z);
        // Player.getRopeHoldPosition: `0.22 * (RIGHT ? -1 : 1)`.
        let hand_dir = if self.settings.main_hand_left {
            1.0
        } else {
            -1.0
        };
        if self.camera_type.first_person() {
            let (yaw, pitch) =
                rope_hold_angles((self.yaw, self.pitch), self.rope_rot_old, alpha as f32);
            let v = glam::DVec3::new(0.39 * hand_dir, -0.6, 0.3);
            // Vec3.xRot(-pitch) then yRot(-yaw).
            let (s, c) = ((-pitch).sin(), (-pitch).cos());
            let v = glam::DVec3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s);
            // Entity.getEyePosition(partialTick).
            let feet = self.previous_position.lerp(p.position, alpha);
            let eye = glam::DVec3::new(feet.x, feet.y + f64::from(p.dimensions.eye_height), feet.z);
            return rot_y(v, -yaw) + eye;
        }
        let yaw = f64::from(self.yaw.to_radians());
        let crouching = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let v = glam::DVec3::new(
            0.22 * hand_dir,
            f64::from(p.dimensions.height) - 1.0,
            if crouching { -0.2 } else { 0.07 },
        );
        pos + rot_y(v, -yaw)
    }
}

#[cfg(test)]
mod rope_hold_tests {
    use super::rope_hold_angles;

    #[test]
    fn rope_hold_angles_lerp_by_half_the_partial_tick() {
        // Mth.lerp(pt * 0.5, yRot, yRotO): at pt 1 the view is halfway to the old one.
        let (y, x) = rope_hold_angles((90.0, 20.0), (50.0, 0.0), 1.0);
        assert!((y - 70.0_f64.to_radians()).abs() < 1e-6);
        assert!((x - 10.0_f64.to_radians()).abs() < 1e-6);
        let (y, _) = rope_hold_angles((90.0, 20.0), (50.0, 0.0), 0.0);
        assert!((y - 90.0_f64.to_radians()).abs() < 1e-6);
    }
}
