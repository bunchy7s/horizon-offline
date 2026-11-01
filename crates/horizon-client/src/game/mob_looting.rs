//! `Mob.aiStep`'s looting loop: a mob with `canPickUpLoot` takes what lies
//! within its box grown by `getPickupReach` (1, 0, 1) and wears or holds it
//! when it beats what it has (`equipItemIfPossible`), dropping the replaced
//! stack by its drop chance. The species whose `pickUpItem` does something
//! else keep their own loops (`pandas.rs`, `piglins.rs`, `villagers.rs`,
//! `allays.rs`, `sulfur_cubes.rs`).
use super::{
    Game,
    pickups::{Collector, Taken},
};
use horizon_client::{
    entities::ItemEntity,
    items::ItemStack,
    mobs::{
        Mob, Species,
        equip::{self, Facts, ItemTags, Slot},
        finalize::{self, set_drop_chance, set_item_stack, stored_drop_chance},
    },
};
use horizon_core::{geometry::Vec3, random::RandomSource};

impl Game<'_> {
    /// `Mob.aiStep`'s looting block, once per mob per tick.
    pub(super) fn tick_mob_looting(&mut self) {
        if self.dropped.is_empty() || !self.ui.rules.bool("mob_griefing") {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if !m.can_pick_up_loot() || !m.alive() || m.removed {
                continue;
            }
            // `Mob.ITEM_PICKUP_REACH`.
            let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
            let (species, baby, passenger) = (m.species(), m.is_baby(), m.extra.passenger);
            let mut j = 0;
            while j < self.dropped.len() {
                let e = &self.dropped[j];
                if e.pickup_delay > 0 || !reach.intersects(e.bounds()) {
                    j += 1;
                    continue;
                }
                let stack = e.stack.clone();
                let item = &stack.item;
                let tags = ItemTags {
                    spear: self.items().item_in_tag("minecraft:spears", item),
                    egg: self.items().item_in_tag("minecraft:eggs", item),
                    disliked_weapon: self
                        .items()
                        .item_in_tag("minecraft:wither_skeleton_disliked_weapons", item),
                };
                if !equip::wants_to_pick_up(species, baby, passenger, item, tags) {
                    j += 1;
                    continue;
                }
                // `Mob.pickUpItem`: nothing leaves the ground what it cannot use.
                let Some(taken) = self.equip_item_if_possible(i, &stack) else {
                    j += 1;
                    continue;
                };
                let before = self.dropped[j].clone();
                let id = self.mobs[i].id;
                self.add_pickup(Taken::Item(Box::new(before.clone())), Collector::Mob(id));
                self.mob_item_pickup_trigger(i, &before);
                let e = &mut self.dropped[j];
                e.stack.count -= taken;
                if e.stack.count == 0 {
                    self.remove_dropped(j);
                } else {
                    j += 1;
                }
            }
        }
    }

    /// `Mob.equipItemIfPossible`: what `slot` would take of `stack`, and how
    /// many items that takes. `None` when the mob has no use for it.
    fn equip_item_if_possible(&mut self, index: usize, stack: &ItemStack) -> Option<u32> {
        let item = &*stack.item;
        let equip_slot = self.items().get(item).and_then(|i| i.equip_slot.clone());
        let mut slot = equip::slot_for(equip_slot.as_deref())?;
        let mut replace = self.can_replace(index, stack, slot);
        // An armour piece that beats nothing it is worn with falls back to an
        // empty main hand.
        if slot.is_armor() && !replace {
            slot = Slot::MainHand;
            replace = self.mobs[index].extra.main_hand.is_none();
        }
        if !replace {
            return None;
        }
        // `EquipmentSlot.limit`: an armour slot takes one item, a hand the
        // whole stack.
        let taken = if slot.is_armor() {
            1
        } else {
            stack.count.max(1)
        };
        let m = &mut self.mobs[index];
        let old = finalize::item_stack(m, slot.index());
        let chance = stored_drop_chance(m, slot.index());
        // `DropChances.byEquipment` against the roll the equip makes.
        let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
        // The whole stack comes with it: its enchantments (a Sharpness V sword
        // stays sharp), damage, name and components.
        let mut equipped = stack.clone();
        equipped.count = taken;
        set_item_stack(m, slot.index(), equipped);
        // `setItemSlotAndDropWhenKilled`, then `persistenceRequired`.
        set_drop_chance(m, slot.index(), 2.0);
        m.persistent = true;
        let at = [m.position.x, m.position.y, m.position.z];
        if drop_old && let Some(old) = old {
            self.spawn_item(old, at, super::dispenser::new_item_motion(), 10);
        }
        Some(taken)
    }

    /// `Mob.canReplaceCurrentItem` for what `m` wears or holds in `slot`.
    fn can_replace(&self, index: usize, stack: &ItemStack, slot: Slot) -> bool {
        let m = &self.mobs[index];
        let Some(worn_stack) = finalize::item_stack(m, slot.index()) else {
            return true;
        };
        let current = worn_stack.item.clone();
        let species = m.species();
        // `Drowned.canReplaceCurrentItem` never parts with a nautilus shell.
        if species == Species::Drowned && &*current == "minecraft:nautilus_shell" {
            return false;
        }
        let (armor, toughness, attack) = self.attributes(m, &stack.item, slot);
        let new = Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: attack,
            enchantments: stack.enchantments().len(),
            damage: stack.damage as i32,
            custom_name: stack.custom_name().is_some(),
            // `EnchantmentEffectComponents.PREVENT_ARMOR_CHANGE` is the Curse of
            // Binding's, the only enchantment that carries one.
            prevents_armor_change: stack.enchantment_level("minecraft:binding_curse") > 0,
        };
        let (armor, toughness, attack) = self.attributes(m, &current, slot);
        let worn = Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: attack,
            enchantments: worn_stack.enchantments().len(),
            damage: worn_stack.damage as i32,
            custom_name: worn_stack.custom_name().is_some(),
            prevents_armor_change: worn_stack.enchantment_level("minecraft:binding_curse") > 0,
        };
        let preferred = equip::preferred_weapon_tag(species).map(|tag| {
            (
                self.items().item_in_tag(tag, &stack.item),
                self.items().item_in_tag(tag, &current),
            )
        });
        equip::can_replace_current_item(&new, Some(&worn), slot, preferred)
    }

    /// `Mob.getApproximateAttributeWith` for the item `id` in `slot`: each
    /// attribute starts from the mob's own value. (armour, toughness, damage)
    fn attributes(&self, m: &Mob, id: &str, slot: Slot) -> (f64, f64, f64) {
        let empty: &[(String, String, f64, String, String)] = &[];
        let modifiers = self
            .items()
            .get(id)
            .map_or(empty, |i| i.modifiers.as_slice());
        (
            equip::attribute_with(modifiers, "minecraft:armor", f64::from(m.kind.armor), slot),
            equip::attribute_with(
                modifiers,
                "minecraft:armor_toughness",
                f64::from(m.kind.armor_toughness),
                slot,
            ),
            equip::attribute_with(
                modifiers,
                "minecraft:attack_damage",
                f64::from(m.attrs.attack_damage),
                slot,
            ),
        )
    }

    /// `LivingEntity.onItemPickup`: the advancement of whoever threw the item,
    /// whichever mob took it.
    pub(super) fn mob_item_pickup_trigger(&mut self, index: usize, e: &ItemEntity) {
        let Some(owner) = e.thrower else {
            return;
        };
        let stack = e.stack.clone();
        let target = self.mob_entity(&self.mobs[index]);
        self.for_each_player(|g| {
            if g.acting_uuid() == owner {
                g.trigger_item_entity("minecraft:thrown_item_picked_up_by_entity", &stack, &target);
            }
        });
    }
}
