//! `TransportItemsBetweenContainers` as the copper golem's brain runs it
//! (`CopperGolemAi`): find the nearest wanted chest (a copper chest with
//! nothing in hand, a chest or trapped chest holding something), walk to it,
//! queue while another golem has it open, open it for 60 ticks and take up
//! to 16 items or put the carried stack down, remembering visited and
//! unreachable chests.
//!
//! The chests are read from the world; what changes them comes out as
//! [`MobEvent::ChestOpen`] and [`MobEvent::ChestSlot`] for the host to apply
//! before the next mob ticks.
use super::{
    Mob, MobEvent, MobWorld,
    animal_brain::{Bx, Tracker, WalkTarget, mem, mem_ref},
    copper_golem as cg,
};
use crate::{containers::BlockEntity, items::ItemStack};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use horizon_world::java_hash_map::{JavaHashMap, JavaKey};

/// `TransportItemState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Travelling,
    Queuing,
    Interacting,
}
/// `ContainerInteractionState`.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interaction {
    PickupItem,
    PickupNoItem,
    PlaceItem,
    PlaceNoItem,
}

/// `TransportItemTarget`: the chest and its block state when it was chosen.
#[derive(Clone, Debug)]
struct Target {
    pos: [i32; 3],
    name: String,
    /// The halves of the container in slot order (the right half first).
    halves: Vec<[i32; 3]>,
}

#[derive(Clone, Debug)]
pub(super) struct Transport {
    pub(super) running: bool,
    end: i64,
    target: Option<Target>,
    state: State,
    interaction: Option<Interaction>,
    ticks: i32,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            running: false,
            end: 0,
            target: None,
            state: State::Travelling,
            interaction: None,
            ticks: 0,
        }
    }
}

/// The golem's held stack (`getMainHandItem`).
pub fn held(m: &Mob) -> Option<&ItemStack> {
    m.extra.items.first().and_then(Option::as_ref)
}
/// `setItemSlot(MAINHAND, ..)`, keeping the rendered copy in step.
pub fn set_held(m: &mut Mob, stack: Option<ItemStack>) {
    let stack = stack.filter(|s| s.count > 0);
    m.extra.set_main_hand_stack(stack.clone());
    m.extra.ints[1] = stack.as_ref().map_or(0, |s| s.count as i32);
    if m.extra.items.is_empty() {
        m.extra.items.push(None);
    }
    m.extra.items[0] = stack;
}

fn name_at(w: &MobWorld, p: [i32; 3]) -> String {
    w.facts
        .get(w.world.block(p[0], p[1], p[2]))
        .name
        .to_string()
}
fn is_chest_block(name: &str) -> bool {
    matches!(name, "minecraft:chest" | "minecraft:trapped_chest") || name.ends_with("copper_chest")
}
/// `CopperGolemAi.TRANSPORT_ITEM_SOURCE_BLOCK` / `_DESTINATION_BLOCK`.
fn wanted(w: &MobWorld, m: &Mob, name: &str) -> bool {
    if held(m).is_none() {
        w.has_tag("minecraft:copper_chests", name) || w.has_tag("copper_chests", name)
    } else {
        matches!(name, "minecraft:chest" | "minecraft:trapped_chest")
    }
}
/// `ChestBlock.getConnectedBlockPos`: the other half of a double chest.
fn connected(w: &MobWorld, p: [i32; 3]) -> Option<[i32; 3]> {
    let kind = w.prop(p[0], p[1], p[2], "type")?;
    if kind == "single" {
        return None;
    }
    let facing = w.prop(p[0], p[1], p[2], "facing")?;
    // LEFT: facing.getClockWise(), else getCounterClockWise().
    let order = ["north", "east", "south", "west"];
    let i = order.iter().position(|f| *f == facing)?;
    let dir = order[if kind == "left" {
        (i + 1) % 4
    } else {
        (i + 3) % 4
    }];
    let (dx, dz) = match dir {
        "north" => (0, -1),
        "south" => (0, 1),
        "east" => (1, 0),
        _ => (-1, 0),
    };
    Some([p[0] + dx, p[1], p[2] + dz])
}
/// `ChestBlock.isChestBlockedAt` (a cat sitting on it aside): a redstone
/// conductor above.
fn blocked(w: &MobWorld, p: [i32; 3]) -> bool {
    let above = w.world.block(p[0], p[1] + 1, p[2]);
    w.data.state(above).flags & super::data::REDSTONE_CONDUCTOR != 0
}
/// `ChestBlock.getContainer(.., false)`: the chest's halves in slot order,
/// none when either half is blocked.
fn container(w: &MobWorld, p: [i32; 3]) -> Option<Vec<[i32; 3]>> {
    if blocked(w, p) {
        return None;
    }
    let mut halves = vec![p];
    if let Some(other) = connected(w, p) {
        let same = w.world.block(other[0], other[1], other[2]) == w.world.block(p[0], p[1], p[2])
            || name_at(w, other) == name_at(w, p)
                && w.prop(other[0], other[1], other[2], "facing")
                    == w.prop(p[0], p[1], p[2], "facing")
                && w.prop(other[0], other[1], other[2], "type")
                    .is_some_and(|t| t != "single")
                && w.prop(other[0], other[1], other[2], "type") != w.prop(p[0], p[1], p[2], "type");
        if same
            && matches!(
                w.world.block_entities.get(&(other[0], other[1], other[2])),
                Some(BlockEntity::Chest(_))
            )
        {
            if blocked(w, other) {
                return None;
            }
            if w.prop(p[0], p[1], p[2], "type") == Some("right") {
                halves.push(other);
            } else {
                halves.insert(0, other);
            }
        }
    }
    Some(halves)
}
fn slots(w: &MobWorld, halves: &[[i32; 3]]) -> Vec<Option<ItemStack>> {
    halves
        .iter()
        .filter_map(|h| w.world.block_entities.get(&(h[0], h[1], h[2])))
        .flat_map(|e| e.slots().to_vec())
        .collect()
}
/// Where slot `n` of the container is: the half and its own slot.
fn slot_at(w: &MobWorld, halves: &[[i32; 3]], mut n: usize) -> Option<([i32; 3], usize)> {
    for h in halves {
        let len = w
            .world
            .block_entities
            .get(&(h[0], h[1], h[2]))?
            .slots()
            .len();
        if n < len {
            return Some((*h, n));
        }
        n -= len;
    }
    None
}
fn max_stack(w: &MobWorld, s: &ItemStack) -> u32 {
    w.items
        .and_then(|i| i.get(&s.item))
        .map_or(64, |i| i.max_stack)
        .max(1)
}

/// `BlockPos` keys in a chunk's block entity map.
#[derive(PartialEq, Eq)]
struct PosKey([i32; 3]);
impl JavaKey for PosKey {
    fn java_hash(&self) -> i32 {
        // Vec3i.hashCode.
        (self.0[1].wrapping_add(self.0[2].wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(self.0[0])
    }
    fn tie_break(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Less
    }
}

/// `AABB.ofSize(center, ..)` of the mob's box, and whether the chest's
/// collision box grown by `distance` (and 0.5 up and down) reaches it.
fn within(w: &MobWorld, m: &Mob, distance: f64, t: &Target, from: [f64; 3]) -> bool {
    let size = m.bounds().size();
    let moved = Aabb::new(
        Vec3::new(
            from[0] - size.x / 2.0,
            from[1] - size.y / 2.0,
            from[2] - size.z / 2.0,
        ),
        Vec3::new(
            from[0] + size.x / 2.0,
            from[1] + size.y / 2.0,
            from[2] + size.z / 2.0,
        ),
    );
    // getCollisionShape of the state the target was chosen with: the block
    // there now (the target is dropped once it changes).
    let Some(shape) = w.collision_bounds(t.pos) else {
        return false;
    };
    let p = Vec3::new(
        f64::from(t.pos[0]),
        f64::from(t.pos[1]),
        f64::from(t.pos[2]),
    );
    shape
        .inflate(Vec3::new(distance, 0.5, distance))
        .move_by(p)
        .intersects(moved)
}
fn center(m: &Mob) -> [f64; 3] {
    [
        m.position.x,
        m.position.y + m.bounds().size().y / 2.0,
        m.position.z,
    ]
}
fn finished_path(m: &Mob) -> bool {
    m.ai.nav.path.as_ref().is_some_and(|p| p.is_done())
}
/// `getInteractionRange`.
fn range(m: &Mob) -> f64 {
    if finished_path(m) { 1.0 } else { 0.5 }
}
/// `stopInPlace`.
fn stop_in_place(m: &mut Mob) {
    m.nav_stop();
    m.ai.xxa = 0.0;
    m.ai.yya = 0.0;
    m.ai.speed = 0.0;
    m.velocity.x = 0.0;
    m.velocity.z = 0.0;
}
/// `ContainerOpenersCounter.getEntitiesWithContainerOpen`: the golems with
/// either half of the chest open, within the counter's interaction range
/// plus 4.
fn openers(w: &MobWorld, m: &Mob, p: [i32; 3]) -> usize {
    let area = opener_area(w, p);
    let me = usize::from(has_open(w, m, p));
    me + w
        .others
        .iter()
        .filter(|s| {
            s.id != m.id && s.alive && holds(w, s.opened_chest, p) && s.bounds.intersects(area)
        })
        .count()
}
/// `CopperGolem.hasContainerOpen`: its opened chest is `p` or `p`'s other half.
fn holds(w: &MobWorld, opened: Option<[i32; 3]>, p: [i32; 3]) -> bool {
    opened.is_some_and(|o| o == p || connected(w, o) == Some(p))
}
/// Whether the counter at `p` lists this golem.
fn has_open(w: &MobWorld, m: &Mob, p: [i32; 3]) -> bool {
    holds(w, m.extra.opened_chest, p) && m.bounds().intersects(opener_area(w, p))
}
fn opener_area(w: &MobWorld, p: [i32; 3]) -> Aabb {
    let range = w
        .world
        .chest_openers
        .get(&(p[0], p[1], p[2]))
        .map_or(0.0, |o| o.1)
        + 4.0;
    Aabb::new(
        Vec3::new(
            f64::from(p[0]) - range,
            f64::from(p[1]) - range,
            f64::from(p[2]) - range,
        ),
        Vec3::new(
            f64::from(p[0]) + 1.0 + range,
            f64::from(p[1]) + 1.0 + range,
            f64::from(p[2]) + 1.0 + range,
        ),
    )
}
/// `ChestBlockEntity.startOpen` / `stopOpen` by a golem (interaction range
/// 3): the opener count changes and the first opener or the last closer
/// plays the chest's sound (`ChestBlockEntity.playSound`, at the middle of
/// a double chest from its right half, none from its left).
fn open_or_close(cx: &mut Bx, p: [i32; 3], open: bool) {
    let w = cx.w;
    let count = w
        .world
        .chest_openers
        .get(&(p[0], p[1], p[2]))
        .map_or(0, |o| o.0);
    let sounds = if open { count == 0 } else { count == 1 };
    if sounds
        && let Some(kind) = w.prop(p[0], p[1], p[2], "type")
        && kind != "left"
    {
        let mut at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        if kind == "right"
            && let Some(o) = connected(w, p)
        {
            at[0] += f64::from(o[0] - p[0]) * 0.5;
            at[2] += f64::from(o[2] - p[2]) * 0.5;
        }
        let name = name_at(w, p);
        let base = name
            .trim_start_matches("minecraft:")
            .trim_start_matches("waxed_");
        let sound = match (base.ends_with("copper_chest"), base.split('_').next(), open) {
            (false, _, true) => "block.chest.open",
            (false, _, false) => "block.chest.close",
            (true, Some("weathered"), true) => "block.copper_chest_weathered.open",
            (true, Some("weathered"), false) => "block.copper_chest_weathered.close",
            (true, Some("oxidized"), true) => "block.copper_chest_oxidized.open",
            (true, Some("oxidized"), false) => "block.copper_chest_oxidized.close",
            (true, _, true) => "block.copper_chest.open",
            (true, _, false) => "block.copper_chest.close",
        };
        let pitch = cx.lr.next_float() * 0.1 + 0.9;
        cx.events.push(MobEvent::Sound(sound, at, 0.5, pitch));
    }
    cx.events.push(MobEvent::ChestOpen { chest: p, open });
}

impl Transport {
    /// `Behavior.tryStart`: no transport cooldown, not panicking, not
    /// leashed; the duration draw, then `start`.
    pub(super) fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        let mm = mem(m);
        if mm.transport_items_cooldown_ticks.present() || mm.is_panicking.present() {
            return false;
        }
        if m.extra.leash.is_some() {
            return false;
        }
        self.running = true;
        self.end = cx.now + i64::from(cx.lr.next_int_bound(1).unwrap_or(0)) + 60;
        m.ai.nav.below_surface = true;
        true
    }
    pub(super) fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        let mm = mem(m);
        let can_still = !mm.transport_items_cooldown_ticks.present()
            && !mm.is_panicking.present()
            && m.extra.leash.is_none();
        if !can_still {
            self.do_stop(m, cx);
            return;
        }
        self.tick(m, cx);
    }
    pub(super) fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        self.running = false;
        self.on_start_travelling(m, cx);
        m.ai.nav.below_surface = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Bx) {
        let updated = self.update_invalid_target(m, cx);
        let Some(target) = self.target.clone() else {
            // `this.stop(..)`: the stop effects, the behaviour still running.
            self.on_start_travelling(m, cx);
            m.ai.nav.below_surface = false;
            return;
        };
        if updated {
            return;
        }
        if self.state == State::Queuing && !self.another_interacting(m, cx.w, &target) {
            self.state = State::Travelling;
            self.walk_towards(m);
        }
        if self.state == State::Travelling {
            let c = center(m);
            if within(cx.w, m, 3.0, &target, c) && self.another_interacting(m, cx.w, &target) {
                stop_in_place(m);
                self.state = State::Queuing;
            } else if within(cx.w, m, range(m), &target, c) {
                self.interaction = Some(reached_interaction(m, cx.w, &target.halves));
                self.state = State::Interacting;
            } else {
                self.walk_towards(m);
            }
        }
        if self.state == State::Interacting {
            self.on_reached(m, cx, &target);
        }
    }
    fn update_invalid_target(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        if self.has_valid_target(m, cx) {
            return false;
        }
        self.stop_targeting(m);
        match self.find_target(m, cx.w) {
            Some(t) => {
                let pos = t.pos;
                self.target = Some(t);
                self.on_start_travelling(m, cx);
                self.set_visited(m, pos);
            }
            None => self.enter_cooldown(m),
        }
        true
    }
    fn has_valid_target(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        let Some(t) = self.target.clone() else {
            return false;
        };
        let w = cx.w;
        let unchanged = matches!(
            w.world.block_entities.get(&(t.pos[0], t.pos[1], t.pos[2])),
            Some(BlockEntity::Chest(_))
        ) && is_chest_block(&name_at(w, t.pos));
        if !(wanted(w, m, &t.name) && unchanged) || blocked(w, t.pos) {
            return false;
        }
        if self.state != State::Travelling {
            return true;
        }
        if self.valid_travelling_path(m, w, &t) {
            return true;
        }
        self.mark_unreachable(m, t.pos);
        false
    }
    fn valid_travelling_path(&mut self, m: &mut Mob, w: &MobWorld, t: &Target) -> bool {
        let path = match &m.ai.nav.path {
            Some(p) => Some(p.clone()),
            None => m.create_path_to(w, t.pos, 0),
        };
        let from = match path.as_ref().and_then(|p| p.end()) {
            Some(n) => [f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5],
            None => [m.position.x, m.position.y, m.position.z],
        };
        let from = [from[0], from[1] + m.bounds().size().y / 2.0, from[2]];
        let reach = within(w, m, range(m), t, from);
        (path.is_none() && !reach) || reach && sees_a_side(w, t.pos, from)
    }
    fn find_target(&self, m: &Mob, w: &MobWorld) -> Option<Target> {
        let bx = m.position.x.floor() as i32;
        let by = m.position.y.floor() as i32;
        let bz = m.position.z.floor() as i32;
        let (h, v): (i32, i32) = if m.extra.mount.is_some() {
            (1, 1)
        } else {
            (32, 8)
        };
        let visited = mem_list(m, false);
        let unreachable = mem_list(m, true);
        let (cx, cz) = (bx >> 4, bz >> 4);
        let r = h.div_euclid(16) + 1;
        // Each chunk's block entities in their HashMap's order.
        let mut by_chunk: std::collections::HashMap<(i32, i32), Vec<[i32; 3]>> = Default::default();
        for (k, e) in &w.world.block_entities {
            if matches!(e, BlockEntity::Chest(_)) {
                by_chunk
                    .entry((k.0 >> 4, k.2 >> 4))
                    .or_default()
                    .push([k.0, k.1, k.2]);
            }
        }
        let mut best: Option<(f64, Target)> = None;
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                let Some(list) = by_chunk.get_mut(&(x, z)) else {
                    continue;
                };
                list.sort();
                let mut map: JavaHashMap<PosKey, ()> = JavaHashMap::default();
                for p in list.iter() {
                    map.put(PosKey(*p), ());
                }
                for (key, _) in map.entries() {
                    let p = key.0;
                    let d = (f64::from(p[0]) + 0.5 - m.position.x).powi(2)
                        + (f64::from(p[1]) + 0.5 - m.position.y).powi(2)
                        + (f64::from(p[2]) + 0.5 - m.position.z).powi(2);
                    if best.as_ref().map_or(f64::from(f32::MAX), |b| b.0) <= d {
                        continue;
                    }
                    // The search box: the mob's block grown by h, v, h.
                    let inside = (bx - h..bx + 1 + h).contains(&p[0])
                        && (by - v..by + 1 + v).contains(&p[1])
                        && (bz - h..bz + 1 + h).contains(&p[2]);
                    if !inside {
                        continue;
                    }
                    let Some(halves) = container(w, p) else {
                        continue;
                    };
                    let name = name_at(w, p);
                    let seen =
                        std::iter::once(p)
                            .chain(connected(w, p).filter(|o| {
                                w.world.block_entities.contains_key(&(o[0], o[1], o[2]))
                            }))
                            .any(|q| visited.contains(&q) || unreachable.contains(&q));
                    if wanted(w, m, &name) && !seen {
                        best = Some((
                            d,
                            Target {
                                pos: p,
                                name,
                                halves,
                            },
                        ));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
    /// `isAnotherMobInteractingWithTarget`: either half has an opener.
    fn another_interacting(&self, m: &Mob, w: &MobWorld, t: &Target) -> bool {
        let mut chests = vec![t.pos];
        if let Some(o) = connected(w, t.pos)
            && w.world.block_entities.contains_key(&(o[0], o[1], o[2]))
        {
            chests.push(o);
        }
        chests.iter().any(|p| openers(w, m, *p) > 0)
    }
    fn walk_towards(&self, m: &mut Mob) {
        if let Some(t) = &self.target {
            // BehaviorUtils.setWalkAndLookTargetMemories(body, pos, 1, 0).
            let mm = mem(m);
            mm.look_target.set(Tracker::Block(t.pos));
            mm.walk_target.set(WalkTarget {
                target: Tracker::Block(t.pos),
                speed: 1.0,
                close: 0,
            });
        }
    }
    fn on_reached(&mut self, m: &mut Mob, cx: &mut Bx, t: &Target) {
        if !within(cx.w, m, 2.0, t, center(m)) {
            self.on_start_travelling(m, cx);
            return;
        }
        self.ticks += 1;
        // onTargetInteraction.
        mem(m).look_target.set(Tracker::Block(t.pos));
        stop_in_place(m);
        if let Some(i) = self.interaction {
            let (state, sound) = match i {
                Interaction::PickupItem => (cg::GETTING_ITEM, "entity.copper_golem.no_item_get"),
                Interaction::PickupNoItem => {
                    (cg::GETTING_NO_ITEM, "entity.copper_golem.no_item_no_get")
                }
                Interaction::PlaceItem => (cg::DROPPING_ITEM, "entity.copper_golem.item_drop"),
                Interaction::PlaceNoItem => {
                    (cg::DROPPING_NO_ITEM, "entity.copper_golem.item_no_drop")
                }
            };
            if self.ticks == 1 {
                // container.startOpen, before setOpenedChestPos.
                for h in &t.halves {
                    open_or_close(cx, *h, true);
                }
                m.extra.opened_chest = Some(t.pos);
                m.extra.variant3 = state;
            }
            if self.ticks == 9 && !m.silent {
                let at = [m.position.x, m.position.y, m.position.z];
                cx.events.push(MobEvent::Sound(sound, at, 1.0, 1.0));
            }
            if self.ticks == 60 {
                // container.getEntitiesWithContainerOpen().contains(body):
                // only a single chest lists its openers (CompoundContainer
                // keeps Container's empty list, so a double chest is left
                // to the openers recheck).
                if t.halves.len() == 1 && has_open(cx.w, m, t.pos) {
                    open_or_close(cx, t.pos, false);
                }
                m.extra.opened_chest = None;
            }
        }
        if self.ticks >= 60 {
            let items = slots(cx.w, &t.halves);
            if held(m).is_none() {
                if items.iter().any(Option::is_some) {
                    self.pick_up(m, cx, &t.halves, &items);
                } else {
                    self.stop_targeting(m);
                }
            } else if leaving_ok(m, &items) {
                self.put_down(m, cx, &t.halves, items);
            } else {
                self.stop_targeting(m);
            }
            self.on_start_travelling(m, cx);
        }
    }
    /// `pickUpItems`: up to 16 of the first non-empty slot.
    fn pick_up(
        &mut self,
        m: &mut Mob,
        cx: &mut Bx,
        halves: &[[i32; 3]],
        items: &[Option<ItemStack>],
    ) {
        if let Some((n, s)) = items
            .iter()
            .enumerate()
            .find_map(|(n, s)| s.as_ref().map(|s| (n, s.clone())))
        {
            let take = s.count.min(16);
            let mut left = s.clone();
            left.count -= take;
            let mut got = s;
            got.count = take;
            if let Some((h, slot)) = slot_at(cx.w, halves, n) {
                cx.events.push(MobEvent::ChestSlot {
                    chest: h,
                    slot,
                    stack: (left.count > 0).then_some(left),
                });
            }
            set_held(m, Some(got));
        }
        self.clear_memories(m);
    }
    /// `putDownItem` (`addItemsToContainer`, which takes from the carried
    /// stack all the room a slot had, not what it put there).
    fn put_down(
        &mut self,
        m: &mut Mob,
        cx: &mut Bx,
        halves: &[[i32; 3]],
        items: Vec<Option<ItemStack>>,
    ) {
        let mut carried = held(m).cloned();
        for (n, slot) in items.into_iter().enumerate() {
            let Some(stack) = carried.clone() else { break };
            match slot {
                None => {
                    if let Some((h, s)) = slot_at(cx.w, halves, n) {
                        cx.events.push(MobEvent::ChestSlot {
                            chest: h,
                            slot: s,
                            stack: Some(stack),
                        });
                    }
                    carried = None;
                    break;
                }
                Some(mut there) => {
                    let max = max_stack(cx.w, &there);
                    if there.item == stack.item
                        && there.extra == stack.extra
                        && there.damage == stack.damage
                        && there.count < max
                    {
                        let room = max - there.count;
                        let add = room.min(stack.count);
                        there.count += add;
                        if let Some((h, s)) = slot_at(cx.w, halves, n) {
                            cx.events.push(MobEvent::ChestSlot {
                                chest: h,
                                slot: s,
                                stack: Some(there),
                            });
                        }
                        let left = i64::from(stack.count) - i64::from(room);
                        if left <= 0 {
                            carried = None;
                            break;
                        }
                        let mut rest = stack;
                        rest.count = left as u32;
                        carried = Some(rest);
                    }
                }
            }
        }
        let emptied = carried.is_none();
        set_held(m, carried);
        if emptied {
            self.clear_memories(m);
        } else {
            self.stop_targeting(m);
        }
    }
    fn on_start_travelling(&mut self, m: &mut Mob, _: &mut Bx) {
        // CopperGolemAi.onTravelling: clearOpenedChestPos, IDLE.
        m.extra.opened_chest = None;
        m.extra.variant3 = cg::IDLE;
        self.state = State::Travelling;
        self.interaction = None;
        self.ticks = 0;
    }
    fn stop_targeting(&mut self, m: &mut Mob) {
        self.ticks = 0;
        self.target = None;
        m.nav_stop();
        mem(m).walk_target.clear();
    }
    fn clear_memories(&mut self, m: &mut Mob) {
        self.stop_targeting(m);
        let mm = mem(m);
        mm.visited_block_positions.clear();
        mm.unreachable_transport_block_positions.clear();
    }
    fn enter_cooldown(&mut self, m: &mut Mob) {
        self.stop_targeting(m);
        let mm = mem(m);
        mm.transport_items_cooldown_ticks.set(140);
        mm.visited_block_positions.clear();
        mm.unreachable_transport_block_positions.clear();
    }
    fn set_visited(&mut self, m: &mut Mob, p: [i32; 3]) {
        let mut list = mem_list(m, false);
        if !list.contains(&p) {
            list.push(p);
        }
        if list.len() > 10 {
            self.enter_cooldown(m);
        } else {
            mem(m).visited_block_positions.set_ttl(list, 6000);
        }
    }
    fn mark_unreachable(&mut self, m: &mut Mob, p: [i32; 3]) {
        let mut visited = mem_list(m, false);
        visited.retain(|q| *q != p);
        let mut unreachable = mem_list(m, true);
        if !unreachable.contains(&p) {
            unreachable.push(p);
        }
        if unreachable.len() > 50 {
            self.enter_cooldown(m);
        } else {
            let mm = mem(m);
            mm.visited_block_positions.set_ttl(visited, 6000);
            mm.unreachable_transport_block_positions
                .set_ttl(unreachable, 6000);
        }
    }
}
fn mem_list(m: &Mob, unreachable: bool) -> Vec<[i32; 3]> {
    let Some(mm) = mem_ref(m) else {
        return Vec::new();
    };
    let slot = if unreachable {
        &mm.unreachable_transport_block_positions
    } else {
        &mm.visited_block_positions
    };
    slot.get().cloned().unwrap_or_default()
}
/// `matchesLeavingItemsRequirement`: an empty container, or one holding the
/// carried item.
fn leaving_ok(m: &Mob, items: &[Option<ItemStack>]) -> bool {
    let carried = held(m);
    items.iter().all(Option::is_none)
        || items
            .iter()
            .flatten()
            .any(|s| carried.is_some_and(|c| c.item == s.item))
}
fn reached_interaction(m: &Mob, w: &MobWorld, halves: &[[i32; 3]]) -> Interaction {
    let items = slots(w, halves);
    if held(m).is_none() {
        if items.iter().any(Option::is_some) {
            Interaction::PickupItem
        } else {
            Interaction::PickupNoItem
        }
    } else if leaving_ok(m, &items) {
        Interaction::PlaceItem
    } else {
        Interaction::PlaceNoItem
    }
}
/// `canSeeAnyTargetSide`: a clip from `eye` to the middle of any face of the
/// chest's block hits that block.
fn sees_a_side(w: &MobWorld, p: [i32; 3], eye: [f64; 3]) -> bool {
    let c = Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    );
    // Direction order: down, up, north, south, west, east.
    let steps = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ];
    let from = Vec3::new(eye[0], eye[1], eye[2]);
    steps.iter().any(|s| {
        let to = Vec3::new(
            c.x + 0.5 * f64::from(s[0]),
            c.y + 0.5 * f64::from(s[1]),
            c.z + 0.5 * f64::from(s[2]),
        );
        w.clip_block_at(from, to) == Some(p)
    })
}
