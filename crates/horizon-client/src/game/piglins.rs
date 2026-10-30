//! The game side of piglins (`PiglinAi`): spotting and picking up gold,
//! bartering when the admiring is over, and anger at players who open
//! their chests or mine their gold.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{
        Mob, Species, Who,
        finalize::{
            SLOT_NAMES, set_drop_chance, set_item, set_slot_enchantments, slot_enchantments,
            stored_drop_chance,
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
    fn piglin_wanted(&self, item: &str) -> bool {
        self.piglin_loved(item) || self.item_in_tag("minecraft:piglin_preferred_weapons", item)
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
        self.piglin_wanted(item) && !self.piglin_wanted(&current)
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
    /// `PiglinAi.wantsToPickup` (an adult that is not hurt-shy), without
    /// the baby rule, which babies skip entirely here.
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool {
        let item = &*stack.item;
        if self.item_in_tag("minecraft:piglin_repellents", item) {
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
            if m.species() != Species::Piglin
                || !m.alive()
                || m.is_baby()
                || m.extra.using_item > 0
                || m.extra.egg_time > 0
            {
                continue;
            }
            let pos = m.position;
            let best = self
                .dropped
                .iter()
                .enumerate()
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .filter(|(_, d)| *d <= 81.0)
                .filter(|&(j, _)| self.piglin_wants(m, &self.dropped[j].stack))
                .min_by(|a, b| a.1.total_cmp(&b.1));
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
            let carried = e.stack.clone();
            let owner = e.thrower;
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(taken_item)),
                super::pickups::Collector::Mob(id),
            );
            // LivingEntity.onItemPickup, before PiglinAi.pickUpItem splits the stack.
            // The owner is the host or a guest: whoever it is gets the trigger.
            if let Some(owner) = owner {
                let target = self.mob_entity(&self.mobs[i]);
                self.for_each_player(|g| {
                    if g.acting_uuid() == owner {
                        g.trigger_item_entity(
                            "minecraft:thrown_item_picked_up_by_entity",
                            &carried,
                            &target,
                        );
                    }
                });
            }
            let stack = if &*item == "minecraft:gold_nugget" {
                self.dropped.remove(j).stack
            } else {
                let e = &mut self.dropped[j];
                e.stack.count -= 1;
                let one = ItemStack {
                    count: 1,
                    ..e.stack.clone()
                };
                if e.stack.count == 0 {
                    self.dropped.remove(j);
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
    fn start_admiring(&mut self, index: usize, item: std::sync::Arc<str>) {
        let m = &mut self.mobs[index];
        m.extra.off_hand = Some(item);
        m.extra.using_item = ADMIRE;
        m.extra.point = None;
        m.nav_stop();
        let at = m.center();
        self.audio
            .play("minecraft:entity.piglin.admiring_item", Some(at), 1.0, 1.0);
    }
    /// `PiglinAi.mobInteract`: hand a piglin a gold ingot (only piglin
    /// currency is accepted from the hand; other loved items are passed over).
    pub(super) fn give_piglin(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::Piglin
            || m.is_baby()
            || m.extra.using_item > 0
            || m.extra.egg_time > 0
            || item != "minecraft:gold_ingot"
        {
            return false;
        }
        self.start_admiring(index, item.into());
        true
    }
    /// `PiglinAi.stopHoldingOffHandItem` after admiring: gold ingots buy a
    /// roll of the bartering table, thrown toward the player; any other loved
    /// item it picked up is kept (worn if it can be, else stored).
    pub(super) fn finish_admiring(&mut self, mob: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(item) = self.mobs[i].extra.off_hand.take() else {
            return;
        };
        if &*item != "minecraft:gold_ingot" {
            self.piglin_keep(i, ItemStack::new(&item, 1));
            return;
        }
        let m = &self.mobs[i];
        let from = [m.position.x, m.position.y + 1.3, m.position.z];
        let items = if !m.is_baby() {
            world_items!(self).loot_table(
                "minecraft:gameplay/piglin_bartering",
                false,
                None,
                &mut self.world.random,
            )
        } else {
            vec![ItemStack::new(&item, 1)]
        };
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
