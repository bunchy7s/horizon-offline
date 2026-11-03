//! The game side of piglins (`PiglinAi`): spotting and picking up gold,
//! bartering when the admiring is over, and anger at players who open
//! their chests or mine their gold.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{
        Mob, Species, Who,
        finalize::{
            SLOT_NAMES, item_stack, set_drop_chance, set_item, set_item_stack,
            set_slot_enchantments, slot_enchantments, stored_drop_chance,
        },
        piglin::{ADMIRE, ATE_RECENTLY},
    },
};
use horizon_core::{geometry::Vec3, random::RandomSource};

impl Game<'_> {
    fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        self.res
            .items()
            .item_tags
            .get(tag)
            .is_some_and(|t| t.contains(item))
    }
    fn piglin_loved(&self, item: &str) -> bool {
        self.item_in_tag("minecraft:piglin_loved", item)
    }
    /// `PiglinAi.isLovedItem` or a preferred weapon (`Piglin.canReplaceCurrentItem`).
    fn piglin_wanted(&self, m: &Mob, item: &str) -> bool {
        self.piglin_loved(item)
            || !m.is_baby() && self.item_in_tag("minecraft:piglin_preferred_weapons", item)
    }
    /// The `SLOT_NAMES` index `getEquipmentSlotForItem` picks: armour goes
    /// to its slot, anything else to the main hand.
    fn piglin_slot(&self, item: &str) -> usize {
        let slot = self
            .res
            .items()
            .get(item)
            .and_then(|i| i.equip_slot.as_deref())
            .filter(|s| matches!(*s, "head" | "chest" | "legs" | "feet"))
            .unwrap_or("mainhand");
        SLOT_NAMES.iter().position(|n| *n == slot).unwrap_or(0)
    }
    /// `Piglin.canReplaceCurrentItem`: an empty slot takes anything, a wanted
    /// item replaces an unwanted one and never the other way round. Vanilla's
    /// armour and weapon comparison for two equal-interest items is left out
    /// (the piglin keeps what it has).
    fn piglin_can_replace(&self, m: &Mob, item: &str, slot: usize) -> bool {
        let Some(current) = horizon_client::mobs::finalize::item(m, slot) else {
            return true;
        };
        self.piglin_wanted(m, item) && !self.piglin_wanted(m, &current)
    }
    /// `SimpleContainer.canAddItem` on the piglin's eight slots.
    fn piglin_can_add(&self, m: &Mob, stack: &ItemStack) -> bool {
        m.extra.inventory.len() < 8
            || m.extra.inventory.iter().any(|s| {
                s.same_item_same_components(stack) && s.count < self.piglin_max_stack(&s.item)
            })
    }
    fn piglin_max_stack(&self, item: &str) -> u32 {
        self.items().get(item).map_or(64, |i| i.max_stack)
    }
    /// `PiglinAi.wantsToPickup`: babies ignore their own item tag, but can
    /// pick up currency and other loved items just like adults.
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool {
        let item = &*stack.item;
        if m.is_baby() && self.item_in_tag("minecraft:ignored_by_piglin_babies", item)
            || self.item_in_tag("minecraft:piglin_repellents", item)
        {
            return false;
        }
        let holds_loved = m
            .extra
            .off_hand
            .as_deref()
            .is_some_and(|o| self.piglin_loved(o));
        if item == "minecraft:gold_ingot" {
            return !holds_loved;
        }
        let space = self.piglin_can_add(m, stack);
        if item == "minecraft:gold_nugget" {
            space
        } else if self.item_in_tag("minecraft:piglin_food", item) {
            m.extra.ints[0] <= 0 && space
        } else if !self.piglin_loved(item) {
            self.piglin_can_replace(m, item, self.piglin_slot(item))
        } else {
            !holds_loved && space
        }
    }
    /// `Mob.equipItemIfPossible` for a piglin: put it in its slot when that
    /// slot takes it (an armour piece that cannot replace what is worn goes
    /// to an empty main hand), dropping what it replaces by drop chance.
    fn piglin_equip(&mut self, index: usize, stack: &ItemStack) -> bool {
        let item = &*stack.item;
        let mut slot = self.piglin_slot(item);
        let m = &self.mobs[index];
        let mut replace = self.piglin_can_replace(m, item, slot);
        if slot > 1 && !replace {
            slot = 0;
            replace = m.extra.main_hand.is_none();
        }
        if !replace {
            return false;
        }
        let m = &mut self.mobs[index];
        let old = horizon_client::mobs::finalize::item(m, slot);
        let chance = stored_drop_chance(m, slot);
        let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
        let old_enchantments = slot_enchantments(m, slot);
        set_item(m, slot, item);
        // The piglin keeps what the stack carried (Protection IV stays on).
        set_slot_enchantments(m, slot, stack.enchantments().to_vec());
        // setItemSlotAndDropWhenKilled, then persistenceRequired.
        set_drop_chance(m, slot, 2.0);
        m.persistent = true;
        let at = [m.position.x, m.position.y, m.position.z];
        if let (true, Some(old)) = (drop_old, old) {
            let mut dropped = ItemStack::new(&old, 1);
            dropped.set_crafting_enchantments(old_enchantments);
            self.spawn_item(dropped, at, super::dispenser::new_item_motion(), 10);
        }
        true
    }
    /// `PiglinAi.putInInventory`: what does not fit is thrown at a random
    /// spot nearby.
    fn piglin_stash(&mut self, index: usize, mut stack: ItemStack) {
        let max = self.piglin_max_stack(&stack.item);
        let m = &mut self.mobs[index];
        for s in &mut m.extra.inventory {
            if stack.count > 0 && s.same_item_same_components(&stack) && s.count < max {
                let n = (max - s.count).min(stack.count);
                s.count += n;
                stack.count -= n;
            }
        }
        while stack.count > 0 && m.extra.inventory.len() < 8 {
            let n = stack.count.min(max);
            m.extra.inventory.push(ItemStack {
                count: n,
                ..stack.clone()
            });
            stack.count -= n;
        }
        if stack.count == 0 {
            return;
        }
        let from = [m.position.x, m.position.y + 1.3, m.position.z];
        let r = &mut self.world.random;
        let to = [
            from[0] + f64::from(r.next_float() * 8.0 - 4.0),
            from[1],
            from[2] + f64::from(r.next_float() * 8.0 - 4.0),
        ];
        self.piglin_throw(from, to, stack);
    }
    /// `PiglinAi.stopHoldingOffHandItem` for something that is not
    /// bartering currency: wear it if it can, else keep it in the inventory.
    fn piglin_keep(&mut self, index: usize, stack: ItemStack) {
        // Nuggets keep their whole stack in the inventory (a hand slot here
        // holds one item id only).
        if &*stack.item == "minecraft:gold_nugget" || !self.piglin_equip(index, &stack) {
            self.piglin_stash(index, stack);
        }
    }
    /// `BehaviorUtils.throwItem`: toss a stack from a point toward another.
    fn piglin_throw(&mut self, from: [f64; 3], to: [f64; 3], stack: ItemStack) {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        let v = [d[0] / len * 0.3, d[1] / len * 0.3 + 0.1, d[2] / len * 0.3];
        self.spawn_item(stack, from, v, 40);
    }
    /// `PiglinAi.isWearingSafeArmor`.
    pub(super) fn wearing_gold(&self) -> bool {
        let tag = self
            .res
            .items()
            .item_tags
            .get("minecraft:piglin_safe_armor");
        self.armor
            .iter()
            .flatten()
            .any(|s| tag.is_some_and(|t| t.contains(&*s.item)))
    }
    /// Piglins notice wanted items on the ground within 9 blocks (`PiglinAi.wantsToPickup`:
    /// gold and other loved items, nuggets, food, gear), walk to them and
    /// take them (`GoToWantedItem`, `Piglin.pickUpItem`).
    pub(super) fn piglin_pickups(&mut self) {
        if !self.ui.rules.bool("mob_griefing") {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Piglin || !m.alive() || m.extra.using_item > 0 {
                continue;
            }
            let pos = m.position;
            // NearestItemSensor: the nearest wanted item within 32 that the piglin
            // sees (eye to eye); GoToWantedItem then walks to it within 9 blocks.
            let w = self.mob_world_base(&[]);
            let eye = Vec3::new(pos.x, m.eye_y(), pos.z);
            let nearest = self
                .dropped
                .iter()
                .enumerate()
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .filter(|(_, d)| *d < 32.0 * 32.0)
                .filter(|&(j, _)| self.piglin_wants(m, &self.dropped[j].stack))
                .filter(|&(j, _)| {
                    // ItemEntity's eye height is 0.2125.
                    let e = &self.dropped[j];
                    let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
                    w.line_of_sight(eye, at)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let loved =
                nearest.is_some_and(|(j, _)| self.piglin_loved(&self.dropped[j].stack.item));
            self.admire_seen_item(i, nearest.map(|(_, d)| d), loved);
            let best = nearest.filter(|(_, d)| *d <= 81.0);
            let Some((j, _)) = best else {
                self.mobs[i].extra.point = None;
                continue;
            };
            let e = &self.dropped[j];
            let close = (e.position.x - pos.x).abs() <= 1.3
                && (e.position.z - pos.z).abs() <= 1.3
                && (e.position.y - pos.y).abs() <= 1.0;
            if !close || e.pickup_delay > 0 {
                self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
                continue;
            }
            // Nuggets come whole, anything else one at a time.
            let (taken_item, id) = (e.clone(), self.mobs[i].id);
            let item = e.stack.item.clone();
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(taken_item.clone())),
                super::pickups::Collector::Mob(id),
            );
            // LivingEntity.onItemPickup, before PiglinAi.pickUpItem splits the stack.
            self.mob_item_pickup_trigger(i, &taken_item);
            let stack = if &*item == "minecraft:gold_nugget" {
                self.remove_dropped(j).stack
            } else {
                let e = &mut self.dropped[j];
                e.stack.count -= 1;
                let one = ItemStack {
                    count: 1,
                    ..e.stack.clone()
                };
                if e.stack.count == 0 {
                    self.remove_dropped(j);
                }
                one
            };
            if self.piglin_loved(&item) {
                self.start_admiring(i, item);
            } else if self.item_in_tag("minecraft:piglin_food", &item)
                && self.mobs[i].extra.ints[0] <= 0
            {
                // PiglinAi.eat: the food is gone and it will not eat for a while.
                self.mobs[i].extra.ints[0] = ATE_RECENTLY;
                self.mobs[i].nav_stop();
            } else {
                self.mobs[i].nav_stop();
                self.piglin_keep(i, stack);
            }
        }
    }
    /// `StartAdmiringItemIfSeen` and the admire activity's stop behaviours, from the
    /// distance squared to `NEAREST_VISIBLE_WANTED_ITEM` and whether that item is loved.
    fn admire_seen_item(&mut self, index: usize, nearest: Option<f64>, loved: bool) {
        let m = &mut self.mobs[index];
        let holding = m.extra.off_hand.is_some();
        m.extra.piglin.admire_seen(holding, nearest, loved);
    }
    fn start_admiring(&mut self, index: usize, item: std::sync::Arc<str>) {
        let m = &mut self.mobs[index];
        // The memory is set again, for the item it holds now.
        m.extra.piglin.seen_admire = 0;
        m.extra.off_hand = Some(item);
        m.extra.using_item = ADMIRE;
        m.extra.point = None;
        m.nav_stop();
        // The admiring grunt is `PiglinAi.updateActivity`'s: `piglin::brain_step`
        // plays it when the activity changes.
    }
    /// `PiglinAi.mobInteract`: hand a piglin a gold ingot (only piglin
    /// currency is accepted from the hand; other loved items are passed over).
    pub(super) fn give_piglin(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::Piglin
            || m.is_baby()
            || m.extra.using_item > 0
            || m.extra.piglin.seen_admire > 0
            || m.extra.piglin.admire_disabled > 0
            || item != "minecraft:gold_ingot"
        {
            return false;
        }
        self.start_admiring(index, item.into());
        true
    }
    /// `PiglinAi.stopHoldingOffHandItem` after admiring: gold ingots buy a
    /// roll of the bartering table for adults. Babies equip the admired item,
    /// or hold it in the main hand after stashing or throwing the old item.
    pub(super) fn finish_admiring(&mut self, mob: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(item) = self.mobs[i].extra.off_hand.take() else {
            return;
        };
        if self.mobs[i].is_baby() {
            let stack = ItemStack::new(&item, 1);
            if !self.piglin_equip(i, &stack) {
                if let Some(old) = item_stack(&self.mobs[i], 0) {
                    if self.piglin_loved(&old.item) {
                        self.piglin_stash(i, old);
                    } else {
                        self.piglin_throw_items(i, vec![old]);
                    }
                }
                let m = &mut self.mobs[i];
                set_item_stack(m, 0, stack);
                set_drop_chance(m, 0, 2.0);
                m.persistent = true;
            }
            return;
        }
        if &*item != "minecraft:gold_ingot" {
            self.piglin_keep(i, ItemStack::new(&item, 1));
            return;
        }
        let items = world_items!(self).loot_table(
            "minecraft:gameplay/piglin_bartering",
            false,
            None,
            &mut self.world.random,
        );
        self.piglin_throw_items(i, items);
    }
    /// `PiglinAi.throwItems`: barters and a baby's unwanted main-hand item
    /// use the same player-directed throw.
    fn piglin_throw_items(&mut self, i: usize, items: Vec<ItemStack>) {
        let m = &self.mobs[i];
        let from = [m.position.x, m.position.y + 1.3, m.position.z];
        // BehaviorUtils.throwItem toward the nearest visible player
        // (NEAREST_VISIBLE_PLAYER: within 16, alive, no spectators; the
        // guests count as much as the host), else nearby.
        let at = m.position;
        let host = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !self.spectator())
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let guests = self.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .map(|g| g.position)
        });
        let d = |p: &[f64; 3]| at.distance_squared(Vec3::new(p[0], p[1], p[2]));
        let target = host
            .into_iter()
            .chain(guests)
            .filter(|p| d(p) < 256.0)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
            .map(|p| [p[0], p[1] + 1.0, p[2]]);
        for stack in items {
            let to = target.unwrap_or_else(|| {
                let r = &mut self.world.random;
                [
                    from[0] + f64::from(r.next_float() * 8.0 - 4.0),
                    from[1],
                    from[2] + f64::from(r.next_float() * 8.0 - 4.0),
                ]
            });
            self.piglin_throw(from, to, stack);
        }
    }
    /// `PiglinAi.angerNearbyPiglins`.
    pub(super) fn anger_piglins(&mut self, only_if_seen: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if self.creative || self.spectator() {
            return;
        }
        let at = p.position;
        let eye = [at.x, at.y + 1.62, at.z];
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Piglin
                || m.is_baby()
                || !m.alive()
                || m.ai.target.is_some()
                || (m.position.x - at.x).abs() > 16.0
                || (m.position.y - at.y).abs() > 16.0
                || (m.position.z - at.z).abs() > 16.0
            {
                continue;
            }
            if only_if_seen && !self.can_see(m.center(), eye) {
                continue;
            }
            let m = &mut self.mobs[i];
            m.extra.anger = 600;
            m.ai.target = Some(Who::Player);
        }
    }
    /// A clear line of sight between two points (blocks with collision stop it).
    fn can_see(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let steps = 32;
        (1..steps).all(|i| {
            let f = f64::from(i) / f64::from(steps);
            let p = [
                from[0] + (to[0] - from[0]) * f,
                from[1] + (to[1] - from[1]) * f,
                from[2] + (to[2] - from[2]) * f,
            ];
            !self
                .res
                .facts()
                .get(self.world.block(
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ))
                .full_collision
        })
    }
    /// Whether breaking or opening this block angers piglins.
    pub(super) fn guarded_by_piglins(&self, name: &str) -> bool {
        self.res
            .items()
            .block_tags
            .get("minecraft:guarded_by_piglins")
            .is_some_and(|t| t.contains(name))
    }
}
