//! `LivingEntity.breakItem` as a client sees it. A server never plays the
//! break itself: `LivingEntity.onEquippedItemBroken` broadcasts the entity
//! event `entityEventForEquipmentBreak` gives for the slot, and each client's
//! `LivingEntity.handleEntityEvent` breaks the item it still draws in that
//! slot (`getItemBySlot`), with the sound and the crumbs.
use super::{Mob, drops::head_is_body};
use crate::items::ItemStack;
use horizon_world::player_inventory::EquipmentSlot;

/// `LivingEntity.handleEntityEvent` 47 to 52, 65 and 68: the slot whose
/// item broke (the inverse of `entityEventForEquipmentBreak`).
pub fn equipment_break_slot(event: i8) -> Option<EquipmentSlot> {
    Some(match event {
        47 => EquipmentSlot::MainHand,
        48 => EquipmentSlot::OffHand,
        49 => EquipmentSlot::Head,
        50 => EquipmentSlot::Chest,
        51 => EquipmentSlot::Legs,
        52 => EquipmentSlot::Feet,
        65 => EquipmentSlot::Body,
        68 => EquipmentSlot::Saddle,
        _ => return None,
    })
}

/// `LivingEntity.entityEventForEquipmentBreak`: the event whose client side
/// breaks the item in `slot` (the inverse of `equipment_break_slot`).
pub fn equipment_break_event(slot: EquipmentSlot) -> i8 {
    match slot {
        EquipmentSlot::MainHand => 47,
        EquipmentSlot::OffHand => 48,
        EquipmentSlot::Head => 49,
        EquipmentSlot::Chest => 50,
        EquipmentSlot::Legs => 51,
        EquipmentSlot::Feet => 52,
        EquipmentSlot::Body => 65,
        EquipmentSlot::Saddle => 68,
    }
}

impl Mob {
    /// `LivingEntity.getItemBySlot` for a mob or another player a server
    /// sends: the stack it wears or holds in `slot`, with the components
    /// `ClientboundSetEquipmentPacket` gave it where they are kept.
    pub fn item_by_slot(&self, slot: EquipmentSlot) -> Option<ItemStack> {
        let e = &self.extra;
        let body = head_is_body(self.species());
        let plain = |id: &Option<std::sync::Arc<str>>| id.as_deref().map(|i| ItemStack::new(i, 1));
        let worn = |index: usize, id: &Option<std::sync::Arc<str>>| {
            e.armor[index]
                .clone()
                .filter(|s| id.as_ref() == Some(&s.item))
                .or_else(|| plain(id))
        };
        match slot {
            EquipmentSlot::MainHand => e.main_hand_stack().cloned().or_else(|| plain(&e.main_hand)),
            EquipmentSlot::OffHand => plain(&e.off_hand),
            EquipmentSlot::Head if !body => worn(0, &e.head),
            EquipmentSlot::Chest => worn(1, &e.chest),
            EquipmentSlot::Legs => worn(2, &e.legs),
            EquipmentSlot::Feet => worn(3, &e.feet),
            // The body armour (a wolf's, a horse's, a llama's carpet) is
            // kept in `head`, with the durability it has lost.
            EquipmentSlot::Body if body => plain(&e.head).map(|s| ItemStack {
                damage: e.body_damage.max(0) as u32,
                ..s
            }),
            EquipmentSlot::Saddle => self.saddle_item(),
            EquipmentSlot::Head | EquipmentSlot::Body => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::Species;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn mob(id: &str, species: Species) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = id.into();
        k.species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `LivingEntity.entityEventForEquipmentBreak` and its inverse agree for
    /// every slot a mob or another player can wear something in.
    #[test]
    fn the_break_event_and_its_slot_are_inverses() {
        for slot in EquipmentSlot::ALL {
            let event = equipment_break_event(slot);
            assert_eq!(equipment_break_slot(event), Some(slot), "event {event}");
        }
        assert_eq!(equipment_break_event(EquipmentSlot::Body), 65);
        assert_eq!(equipment_break_event(EquipmentSlot::Saddle), 68);
    }

    /// `LivingEntity.entityEventForEquipmentBreak`: 47 main hand, 48 off hand,
    /// 49 to 52 head to feet, 65 body, 68 saddle; other events break nothing.
    #[test]
    fn each_equipment_break_event_names_its_slot() {
        let slots = [
            (47, EquipmentSlot::MainHand),
            (48, EquipmentSlot::OffHand),
            (49, EquipmentSlot::Head),
            (50, EquipmentSlot::Chest),
            (51, EquipmentSlot::Legs),
            (52, EquipmentSlot::Feet),
            (65, EquipmentSlot::Body),
            (68, EquipmentSlot::Saddle),
        ];
        for (event, slot) in slots {
            assert_eq!(equipment_break_slot(event), Some(slot), "event {event}");
        }
        for event in [2, 3, 35, 46, 53, 54, 55, 60, 64, 66, 67, 69] {
            assert_eq!(equipment_break_slot(event), None, "event {event}");
        }
    }

    /// The stack another player (an avatar, as a mannequin) holds or wears
    /// is the one that breaks.
    #[test]
    fn a_player_breaks_what_it_holds_and_wears() {
        let mut p = mob("minecraft:player", Species::Mannequin);
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.damage = 1560;
        p.extra.set_main_hand_stack(Some(pick.clone()));
        p.extra.off_hand = Some("minecraft:shield".into());
        p.extra.head = Some("minecraft:iron_helmet".into());
        let mut boots = ItemStack::new("minecraft:leather_boots", 1);
        boots.damage = 64;
        p.extra.feet = Some(boots.item.clone());
        p.extra.armor[3] = Some(boots.clone());
        let item = |slot| p.item_by_slot(slot).map(|s: ItemStack| (s.item, s.damage));
        assert_eq!(
            item(EquipmentSlot::MainHand),
            Some((pick.item.clone(), 1560))
        );
        assert_eq!(
            item(EquipmentSlot::OffHand),
            Some(("minecraft:shield".into(), 0))
        );
        assert_eq!(
            item(EquipmentSlot::Head),
            Some(("minecraft:iron_helmet".into(), 0))
        );
        assert_eq!(item(EquipmentSlot::Feet), Some((boots.item.clone(), 64)));
        assert_eq!(item(EquipmentSlot::Chest), None);
        assert_eq!(item(EquipmentSlot::Body), None);
        assert_eq!(item(EquipmentSlot::Saddle), None);
    }

    /// A wolf's armour is its body slot (kept in `head`), and a horse's
    /// saddle its saddle slot.
    #[test]
    fn body_armour_and_saddles_break_from_their_slots() {
        let mut wolf = mob("minecraft:wolf", Species::Wolf);
        wolf.extra.head = Some("minecraft:wolf_armor".into());
        wolf.extra.body_damage = 63;
        let armour = wolf.item_by_slot(EquipmentSlot::Body).unwrap();
        assert_eq!((&*armour.item, armour.damage), ("minecraft:wolf_armor", 63));
        assert_eq!(wolf.item_by_slot(EquipmentSlot::Head), None);

        let mut horse = mob("minecraft:horse", Species::Horse);
        horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
        assert_eq!(
            horse.item_by_slot(EquipmentSlot::Saddle).map(|s| s.item),
            Some("minecraft:saddle".into())
        );
    }
}
