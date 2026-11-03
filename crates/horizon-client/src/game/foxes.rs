//! Foxes and dolphins on the game side: what they take from the ground (`Fox.pickUpItem`,
//! `Fox.canHoldItem`, `Dolphin.pickUpItem`) and where `FoxSearchForItemsGoal` and
//! `Dolphin.MoveToItemGoal` walk to.
use super::{
    Game,
    pickups::{Collector, Taken},
};
use horizon_client::mobs::{
    Species, dolphin,
    equip::Slot,
    finalize::{self, set_drop_chance, set_item_stack},
    fox,
};
use horizon_core::geometry::Vec3;

impl Game<'_> {
    /// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
    pub(super) fn tick_foxes(&mut self) {
        let griefing = self.ui.rules.bool("mob_griefing");
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Fox || !m.alive() || m.removed {
                continue;
            }
            // `Mob.aiStep`: the items in the box grown by (1, 0, 1), in order.
            if griefing && m.can_pick_up_loot() {
                let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
                let mut j = 0;
                while j < self.dropped.len() {
                    let e = &self.dropped[j];
                    if e.pickup_delay > 0 || e.stack.count == 0 || !reach.intersects(e.bounds()) {
                        j += 1;
                        continue;
                    }
                    let item_food = self.fox_food(&e.stack.item);
                    let held_food = self.mobs[i]
                        .extra
                        .main_hand
                        .as_deref()
                        .is_some_and(|held| self.fox_food(held));
                    if !fox::can_hold_item(&self.mobs[i], item_food, held_food) {
                        j += 1;
                        continue;
                    }
                    // The entity is discarded: the next one is at `j` now.
                    self.fox_pick_up(i, j);
                }
            }
            // `FoxSearchForItemsGoal`: the first allowed item within 8 blocks.
            let m = &self.mobs[i];
            let area = m.bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
            let at = self
                .dropped
                .iter()
                .find(|e| e.pickup_delay == 0 && e.health > 0 && area.intersects(e.bounds()))
                .map(|e| [e.position.x, e.position.y, e.position.z]);
            fox::set_item_at(&mut self.mobs[i], at);
        }
    }
    /// `Dolphin.pickUpItem` in `Mob.aiStep`'s looting block, and the item
    /// `Dolphin.MoveToItemGoal` swims to (`Dolphin.ALLOWED_ITEMS`: no pickup delay, alive, in water).
    pub(super) fn tick_dolphins(&mut self) {
        let griefing = self.ui.rules.bool("mob_griefing");
        let in_water = |g: &Self, e: &horizon_client::entities::ItemEntity| {
            g.res
                .facts()
                .get(g.world.block(
                    e.position.x.floor() as i32,
                    e.position.y.floor() as i32,
                    e.position.z.floor() as i32,
                ))
                .fluid
                == horizon_client::blocks::Fluid::Water
        };
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Dolphin || !m.alive() || m.removed {
                continue;
            }
            // `Mob.aiStep`: the items in the box grown by (1, 0, 1); `Dolphin.pickUpItem`
            // takes the whole stack when the mouth is empty (`Mob.canHoldItem` is true).
            if griefing && m.can_pick_up_loot() {
                let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
                let mut j = 0;
                while j < self.dropped.len() {
                    let e = &self.dropped[j];
                    if e.pickup_delay > 0
                        || e.stack.count == 0
                        || !reach.intersects(e.bounds())
                        || self.mobs[i].extra.main_hand.is_some()
                    {
                        j += 1;
                        continue;
                    }
                    let before = self.dropped[j].clone();
                    self.mob_item_pickup_trigger(i, &before);
                    let m = &mut self.mobs[i];
                    set_item_stack(m, Slot::MainHand.index(), before.stack.clone());
                    set_drop_chance(m, Slot::MainHand.index(), 2.0);
                    let id = m.id;
                    self.add_pickup(Taken::Item(Box::new(before)), Collector::Mob(id));
                    self.remove_dropped(j);
                }
            }
            let area = self.mobs[i].bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
            let at = self
                .dropped
                .iter()
                .find(|e| {
                    e.pickup_delay == 0
                        && e.health > 0
                        && area.intersects(e.bounds())
                        && in_water(self, e)
                })
                .map(|e| [e.position.x, e.position.y, e.position.z]);
            dolphin::set_item_at(&mut self.mobs[i], at);
        }
    }
    /// `Fox.isConsumableFood`.
    fn fox_food(&self, item: &str) -> bool {
        self.items()
            .get(item)
            .is_some_and(|i| i.food.is_some() && i.consumable.is_some())
    }
    /// `Fox.pickUpItem`: the extra items drop where it stands, what it held is spat out
    /// ahead of it, and it holds one of the item.
    fn fox_pick_up(&mut self, fox_index: usize, item_index: usize) {
        let before = self.dropped[item_index].clone();
        let mut stack = before.stack.clone();
        let count = stack.count;
        let at = {
            let m = &self.mobs[fox_index];
            [m.position.x, m.position.y, m.position.z]
        };
        // `dropItemStack(itemStack.split(count - 1))`: a default ItemEntity at its feet.
        if count > 1 {
            let mut rest = stack.clone();
            rest.count = count - 1;
            stack.count = 1;
            self.spawn_item(rest, at, super::dispenser::new_item_motion(), 10);
        }
        // `spitOutItem(getItemBySlot(MAINHAND))`.
        let held = finalize::item_stack(&self.mobs[fox_index], Slot::MainHand.index());
        if let Some(held) = held {
            let look = self.mobs[fox_index].view_vector();
            let from = [at[0] + look[0], at[1] + 1.0, at[2] + look[2]];
            self.spawn_item(held, from, super::dispenser::new_item_motion(), 40);
            let id = self.mob_uuid(self.mobs[fox_index].id);
            if let Some(thrown) = self.just_spawned_item() {
                thrown.thrower = Some(id);
            }
            self.audio
                .play("minecraft:entity.fox.spit", Some(at), 1.0, 1.0);
        }
        // `onItemPickup`, then `setItemSlot(MAINHAND, split(1))`, `setGuaranteedDrop`.
        self.mob_item_pickup_trigger(fox_index, &before);
        let m = &mut self.mobs[fox_index];
        set_item_stack(m, Slot::MainHand.index(), stack);
        set_drop_chance(m, Slot::MainHand.index(), 2.0);
        fox::took_item(m);
        let id = m.id;
        // `take(entity, count)` and `entity.discard()`.
        self.add_pickup(Taken::Item(Box::new(before)), Collector::Mob(id));
        self.remove_dropped(item_index);
    }
}
