//! `Mob.equipItemIfPossible` and the comparisons behind it: `compareArmor`,
//! `compareWeapons`, `canReplaceEqualItem` and the per-species gates that
//! decide whether a mob takes an item off the ground at all (`wantsToPickUp`,
//! `canHoldItem`). `game/mob_looting.rs` reads the item data and calls these.
use super::Species;

/// An `EquipmentSlot`, in the order `finalize::item` and `drops::SLOTS` keep
/// them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
}

impl Slot {
    /// `EquipmentSlot.isArmor`: the four armour slots, not the hands.
    pub fn is_armor(self) -> bool {
        matches!(self, Slot::Feet | Slot::Legs | Slot::Chest | Slot::Head)
    }
    /// The `drops::SLOTS` index (`set_item`, `item`, `set_drop_chance`).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// `LivingEntity.getEquipmentSlotForItem`: the item's own `equippable` slot,
/// else the main hand. `None` for `body` and `saddle`: those name the wearer
/// they belong to (`Equippable.canBeEquippedBy`), which Horizon does not
/// model, so no mob of this pass puts them on.
pub fn slot_for(equip_slot: Option<&str>) -> Option<Slot> {
    match equip_slot {
        None | Some("mainhand") => Some(Slot::MainHand),
        Some("offhand") => Some(Slot::OffHand),
        Some("head") => Some(Slot::Head),
        Some("chest") => Some(Slot::Chest),
        Some("legs") => Some(Slot::Legs),
        Some("feet") => Some(Slot::Feet),
        Some(_) => None,
    }
}

/// `EquipmentSlotGroup.test`: whether a modifier the item records for `group`
/// counts in `slot`.
fn group_matches(group: &str, slot: Slot) -> bool {
    match group {
        "any" => true,
        "hand" => matches!(slot, Slot::MainHand | Slot::OffHand),
        "armor" => slot.is_armor(),
        "mainhand" => slot == Slot::MainHand,
        "offhand" => slot == Slot::OffHand,
        "head" => slot == Slot::Head,
        "chest" => slot == Slot::Chest,
        "legs" => slot == Slot::Legs,
        "feet" => slot == Slot::Feet,
        // `body` and `saddle` are slots this pass never equips into.
        _ => false,
    }
}

/// `ItemAttributeModifiers.compute(attribute, base, slot)`: `base` (the mob's
/// own attribute value) plus the item's modifiers for `attribute` that apply
/// in `slot`, in the order the item lists them.
pub fn attribute_with(
    modifiers: &[(String, String, f64, String, String)],
    attribute: &str,
    base: f64,
    slot: Slot,
) -> f64 {
    let mut value = base;
    for (kind, _, amount, operation, group) in modifiers {
        if kind != attribute || !group_matches(group, slot) {
            continue;
        }
        value += match operation.as_str() {
            "add_multiplied_base" => amount * base,
            "add_multiplied_total" => amount * value,
            _ => *amount,
        };
    }
    value
}

/// What the comparisons read about one stack: `getApproximateAttributeWith`
/// over its `attribute_modifiers`, and the three facts `canReplaceEqualItem`
/// compares.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    pub armor: f64,
    pub armor_toughness: f64,
    pub attack_damage: f64,
    /// How many enchantments the stack carries (`ENCHANTMENTS`).
    pub enchantments: usize,
    pub damage: i32,
    pub custom_name: bool,
    /// `EnchantmentEffectComponents.PREVENT_ARMOR_CHANGE` (Curse of Binding).
    pub prevents_armor_change: bool,
}

/// `Mob.canReplaceCurrentItem`: whether the stack the mob could take beats
/// what it has in `slot`. `preferred` is `getPreferredWeaponType`'s tag
/// answer for the two stacks, `(new, current)`; `None` when the mob has none.
pub fn can_replace_current_item(
    new: &Facts,
    current: Option<&Facts>,
    slot: Slot,
    preferred: Option<(bool, bool)>,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if slot.is_armor() {
        compare_armor(new, current)
    } else if slot == Slot::MainHand {
        compare_weapons(new, current, preferred)
    } else {
        // The off hand is never replaced.
        false
    }
}

/// `Mob.compareArmor`: more armour wins, then more toughness, then
/// `canReplaceEqualItem`. A piece with `PREVENT_ARMOR_CHANGE` is never taken
/// off.
pub fn compare_armor(new: &Facts, current: &Facts) -> bool {
    if current.prevents_armor_change {
        return false;
    }
    if new.armor != current.armor {
        return new.armor > current.armor;
    }
    if new.armor_toughness != current.armor_toughness {
        return new.armor_toughness > current.armor_toughness;
    }
    can_replace_equal_item(new, current)
}

/// `Mob.compareWeapons`: a weapon in the mob's preferred tag beats one
/// outside it, then more attack damage wins, then `canReplaceEqualItem`.
pub fn compare_weapons(new: &Facts, current: &Facts, preferred: Option<(bool, bool)>) -> bool {
    if let Some((new_is, current_is)) = preferred {
        if current_is && !new_is {
            return false;
        }
        if new_is && !current_is {
            return true;
        }
    }
    if new.attack_damage != current.attack_damage {
        return new.attack_damage > current.attack_damage;
    }
    can_replace_equal_item(new, current)
}

/// `Mob.canReplaceEqualItem`: more enchantments wins, then less damage, then a
/// custom name.
pub fn can_replace_equal_item(new: &Facts, current: &Facts) -> bool {
    if new.enchantments != current.enchantments {
        return new.enchantments > current.enchantments;
    }
    if new.damage != current.damage {
        return new.damage < current.damage;
    }
    new.custom_name && !current.custom_name
}

/// `Mob.getPreferredWeaponType` of the species that have one; the wither
/// skeleton overrides it back to null.
pub fn preferred_weapon_tag(species: Species) -> Option<&'static str> {
    match species {
        Species::Skeleton | Species::Stray | Species::Bogged | Species::Parched => {
            Some("minecraft:skeleton_preferred_weapons")
        }
        Species::Drowned => Some("minecraft:drowned_preferred_weapons"),
        _ => None,
    }
}

fn is_skeleton(s: Species) -> bool {
    matches!(
        s,
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    )
}
fn is_zombie(s: Species) -> bool {
    matches!(
        s,
        Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager
    )
}

/// The tag facts `wants_to_pick_up` reads about one item.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemTags {
    /// `#minecraft:spears`.
    pub spear: bool,
    /// `#minecraft:eggs`.
    pub egg: bool,
    /// `#minecraft:wither_skeleton_disliked_weapons`.
    pub disliked_weapon: bool,
}

/// `Mob.aiStep`'s looting gate, `Mob.wantsToPickUp` with `canHoldItem`, for
/// the species whose pickup ends in the base `Mob.pickUpItem` (the equip of
/// `equipItemIfPossible`; a piglin brute takes only a golden axe). A villager, piglin, allay, panda, sulfur cube,
/// fox, dolphin or pillager overrides the pickup itself and has its own loop
/// elsewhere; an illager's `Raider.pickUpItem` puts the ominous banner on the
/// head, which Horizon does not model, so they answer `false` here too.
pub fn wants_to_pick_up(
    species: Species,
    baby: bool,
    passenger: bool,
    item: &str,
    tags: ItemTags,
) -> bool {
    if !matches!(
        species,
        Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
            | Species::PiglinBrute
    ) && !is_skeleton(species)
    {
        return false;
    }
    // `PiglinBrute.wantsToPickUp`: only a golden axe, then `Mob.wantsToPickUp`.
    if species == Species::PiglinBrute && item != "minecraft:golden_axe" {
        return false;
    }
    // `AbstractSkeleton.wantsToPickUp` and `Drowned.wantsToPickUp`.
    if (is_skeleton(species) || species == Species::Drowned) && tags.spear {
        return false;
    }
    // `Zombie.wantsToPickUp` (the other zombies) refuses glow ink;
    // `ZombifiedPiglin.wantsToPickUp` is a bare `canHoldItem`.
    if is_zombie(species) && item == "minecraft:glow_ink_sac" {
        return false;
    }
    // `Zombie.canHoldItem`: a baby riding something leaves eggs alone.
    if (is_zombie(species) || species == Species::ZombifiedPiglin) && baby && passenger && tags.egg
    {
        return false;
    }
    // `WitherSkeleton.canHoldItem`.
    if species == Species::WitherSkeleton && tags.disliked_weapon {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(armor: f64, toughness: f64, damage: f64) -> Facts {
        Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: damage,
            ..Facts::default()
        }
    }
    fn sword(damage: f64) -> Facts {
        facts(0.0, 0.0, damage)
    }
    /// `canReplaceCurrentItem` for an item that goes in the main hand, from a
    /// mob with no preferred weapon tag.
    fn replaces_weapon(new: &Facts, current: &Facts) -> bool {
        can_replace_current_item(new, Some(current), Slot::MainHand, None)
    }
    /// The same for a chest piece.
    fn replaces_armour(new: &Facts, current: &Facts) -> bool {
        can_replace_current_item(new, Some(current), Slot::Chest, None)
    }

    #[test]
    fn a_better_weapon_replaces_a_worse_one_and_not_the_reverse() {
        let (iron, wood) = (sword(7.0), sword(5.0));
        assert!(replaces_weapon(&iron, &wood));
        assert!(!replaces_weapon(&wood, &iron));
        // An empty hand takes anything.
        assert!(can_replace_current_item(&wood, None, Slot::MainHand, None));
        // A hand already holding it keeps it: equal damage, equal everything.
        assert!(!replaces_weapon(&iron, &iron));
    }

    #[test]
    fn armour_with_more_armour_or_toughness_wins() {
        let (diamond, iron) = (facts(8.0, 2.0, 0.0), facts(6.0, 0.0, 0.0));
        assert!(replaces_armour(&diamond, &iron));
        assert!(!replaces_armour(&iron, &diamond));
        // Equal armour: the tougher piece wins.
        let tough = facts(8.0, 3.0, 0.0);
        assert!(replaces_armour(&tough, &diamond));
        assert!(!replaces_armour(&diamond, &tough));
    }

    #[test]
    fn an_equal_weapon_with_enchantments_wins() {
        let plain = sword(7.0);
        let sharp = Facts {
            enchantments: 2,
            ..sword(7.0)
        };
        assert!(replaces_weapon(&sharp, &plain));
        assert!(!replaces_weapon(&plain, &sharp));
        // Same enchantment count: the less damaged one wins, then a named one.
        let worn = Facts {
            damage: 40,
            ..sharp
        };
        assert!(replaces_weapon(&sharp, &worn));
        let named = Facts {
            custom_name: true,
            ..sharp
        };
        assert!(replaces_weapon(&named, &sharp));
        assert!(!replaces_weapon(&sharp, &named));
    }

    #[test]
    fn a_piece_that_prevents_armour_change_is_never_replaced() {
        let plain = facts(2.0, 0.0, 0.0);
        let cursed = Facts {
            prevents_armor_change: true,
            ..plain
        };
        let better = facts(8.0, 3.0, 0.0);
        assert!(!replaces_armour(&better, &cursed));
        assert!(replaces_armour(&better, &plain));
        // The weapon comparison does not look at it.
        let cursed_sword = Facts {
            prevents_armor_change: true,
            ..sword(1.0)
        };
        assert!(replaces_weapon(&sword(7.0), &cursed_sword));
    }

    #[test]
    fn a_preferred_weapon_beats_a_stronger_one_outside_the_tag() {
        let (axe, bow) = (sword(9.0), sword(1.0));
        // The skeleton's bow counts as a preferred weapon, both ways round.
        let main = Slot::MainHand;
        assert!(!can_replace_current_item(
            &axe,
            Some(&bow),
            main,
            Some((false, true))
        ));
        assert!(can_replace_current_item(
            &bow,
            Some(&axe),
            main,
            Some((true, false))
        ));
        // Both in the tag: the damage decides.
        assert!(can_replace_current_item(
            &axe,
            Some(&bow),
            main,
            Some((true, true))
        ));
        assert!(!can_replace_current_item(
            &bow,
            Some(&axe),
            main,
            Some((true, true))
        ));
    }

    #[test]
    fn attribute_modifiers_apply_by_slot_group_and_operation() {
        let entry = |attribute: &str, amount: f64, operation: &str, group: &str| {
            (
                attribute.to_owned(),
                "minecraft:test".to_owned(),
                amount,
                operation.to_owned(),
                group.to_owned(),
            )
        };
        let modifiers = [
            entry("minecraft:armor", 8.0, "add_value", "chest"),
            entry("minecraft:attack_damage", 7.0, "add_value", "mainhand"),
            entry("minecraft:attack_damage", 1.0, "add_multiplied_base", "any"),
        ];
        // A chestplate's armour does not count in the feet slot.
        assert_eq!(
            attribute_with(&modifiers, "minecraft:armor", 0.0, Slot::Chest),
            8.0
        );
        assert_eq!(
            attribute_with(&modifiers, "minecraft:armor", 0.0, Slot::Feet),
            0.0
        );
        // The mob's own base damage is the start, and `any` applies.
        let damage = |slot| attribute_with(&modifiers, "minecraft:attack_damage", 1.0, slot);
        assert_eq!(damage(Slot::MainHand), 1.0 + 7.0 + 1.0);
        assert_eq!(damage(Slot::OffHand), 2.0);
    }

    #[test]
    fn items_go_in_their_own_slot() {
        assert_eq!(slot_for(None), Some(Slot::MainHand));
        assert_eq!(slot_for(Some("head")), Some(Slot::Head));
        assert_eq!(slot_for(Some("feet")), Some(Slot::Feet));
        // A body or saddle item names its wearer, which no mob of this pass is.
        assert_eq!(slot_for(Some("body")), None);
    }

    #[test]
    fn the_pickup_gate_follows_the_species_overrides() {
        let tags = |spear, egg, disliked_weapon| ItemTags {
            spear,
            egg,
            disliked_weapon,
        };
        let none = tags(false, false, false);
        let gate = |species, baby, item, t| wants_to_pick_up(species, baby, false, item, t);
        // A zombie takes a sword; a skeleton refuses a spear but takes a bow.
        assert!(gate(
            Species::Zombie,
            false,
            "minecraft:diamond_sword",
            none
        ));
        assert!(!gate(
            Species::Skeleton,
            false,
            "minecraft:iron_spear",
            tags(true, false, false)
        ));
        assert!(gate(Species::Skeleton, false, "minecraft:bow", none));
        // `Zombie.wantsToPickUp` refuses glow ink; a zombified piglin does not.
        assert!(!gate(Species::Husk, false, "minecraft:glow_ink_sac", none));
        assert!(gate(
            Species::ZombifiedPiglin,
            false,
            "minecraft:glow_ink_sac",
            none
        ));
        // A baby zombie riding a chicken leaves eggs alone; on foot it takes one.
        let eggs = tags(false, true, false);
        assert!(!wants_to_pick_up(
            Species::Zombie,
            true,
            true,
            "minecraft:egg",
            eggs
        ));
        assert!(gate(Species::Zombie, true, "minecraft:egg", eggs));
        assert!(gate(Species::Husk, false, "minecraft:egg", eggs));
        // The wither skeleton and its disliked weapons.
        assert!(!gate(
            Species::WitherSkeleton,
            false,
            "minecraft:iron_sword",
            tags(false, false, true)
        ));
        // `PiglinBrute.wantsToPickUp`: a golden axe and nothing else.
        assert!(gate(
            Species::PiglinBrute,
            false,
            "minecraft:golden_axe",
            none
        ));
        assert!(!gate(
            Species::PiglinBrute,
            false,
            "minecraft:iron_sword",
            none
        ));
        assert!(!gate(
            Species::PiglinBrute,
            false,
            "minecraft:gold_ingot",
            none
        ));
        // The species whose pickup is elsewhere never answer yes.
        for s in [
            Species::Villager,
            Species::Piglin,
            Species::Panda,
            Species::Allay,
            Species::Fox,
            Species::Vindicator,
        ] {
            assert!(!gate(s, false, "minecraft:diamond_sword", none));
        }
    }
}
