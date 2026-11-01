//! `Mob.convertTo` with `ConversionType.SINGLE`: one mob turns into another
//! of a different type in its place, keeping what the old one had.
use super::Mob;

/// `ConversionParams` of a `SINGLE` conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// `keepEquipment`: the worn and held items move to the new mob, with
    /// their drop chances.
    pub keep_equipment: bool,
    /// `preserveCanPickUpLoot`.
    pub preserve_can_pick_up_loot: bool,
}
impl Params {
    /// `ConversionParams.single(mob, true, true)`, what `ConversionTracker`
    /// uses (zombie to drowned, husk to zombie, skeleton to stray).
    pub const KEEP_ALL: Self = Self {
        keep_equipment: true,
        preserve_can_pick_up_loot: true,
    };
}

/// `ConversionType.SINGLE.convert` and `ConversionType.convertCommon`: `to`
/// is a fresh mob (`EntityType.create(level, CONVERSION)`, no spawn
/// finalization) that takes `from`'s place, motion, ride, lead, equipment,
/// effects, age, name and flags. Its health stays its own type's.
pub fn single(from: &Mob, to: &mut Mob, params: Params) {
    // copyPosition, setDeltaMovement and the old mob's vehicle.
    to.position = from.position;
    to.previous = from.position;
    to.yaw = from.yaw;
    to.previous_yaw = from.yaw;
    to.head_yaw = from.head_yaw;
    to.previous_head_yaw = from.head_yaw;
    to.head_pitch = from.head_pitch;
    to.previous_head_pitch = from.head_pitch;
    to.velocity = from.velocity;
    to.extra.mount = from.extra.mount;
    to.extra.passenger = from.extra.passenger;
    if params.keep_equipment {
        // Every slot, copyAndClear, and setDropChance from the old mob.
        to.extra.set_main_hand(from.extra.main_hand.clone());
        if let Some(stack) = from.extra.main_hand_stack() {
            to.extra.set_main_hand_stack(Some(stack.clone()));
        }
        to.extra.off_hand = from.extra.off_hand.clone();
        to.extra.head = from.extra.head.clone();
        to.extra.chest = from.extra.chest.clone();
        to.extra.legs = from.extra.legs.clone();
        to.extra.feet = from.extra.feet.clone();
        to.extra.armor = from.extra.armor.clone();
        to.extra.gear = from.extra.gear.clone();
        to.extra.drop_chances = from.extra.drop_chances.clone();
    }
    to.fall_distance = from.fall_distance;
    to.extra.fall_flying = from.extra.fall_flying;
    to.ai.last_hurt_by_player = from.ai.last_hurt_by_player;
    to.hurt_time = from.hurt_time;
    to.body_yaw = from.body_yaw;
    to.previous_body_yaw = from.body_yaw;
    to.on_ground = from.on_ground;
    to.extra.leash = from.extra.leash;
    to.extra.leash_pending = from.extra.leash_pending;
    to.extra.leash_player = from.extra.leash_player;
    // convertCommon.
    to.extra.effects = from.extra.effects.clone();
    to.extra.hidden_effects = from.extra.hidden_effects.clone();
    to.extra.ambient_effects = from.extra.ambient_effects.clone();
    if from.is_baby() {
        super::finalize::set_baby(to, true);
    }
    // An ageable pair keeps the exact age, an adult's breeding cooldown too.
    if super::finalize::ageable(from.species()) && super::finalize::ageable(to.species()) {
        super::finalize::set_age(to, from.extra.age);
        to.extra.forced_age = from.extra.forced_age;
        to.extra.forced_age_timer = from.extra.forced_age_timer;
    }
    if params.preserve_can_pick_up_loot {
        to.extra.pick_up_loot = from.extra.pick_up_loot;
    }
    to.extra.left_handed = from.extra.left_handed;
    to.no_ai = from.no_ai;
    to.persistent |= from.persistent;
    to.name_visible = from.name_visible;
    to.invulnerable_flag = from.invulnerable_flag;
    to.invulnerable = from.invulnerable;
    to.no_gravity = from.no_gravity;
    to.silent = from.silent;
    // COMPONENTS_TO_COPY: the custom name.
    to.custom_name = from.custom_name.clone();
    to.custom_name_text = from.custom_name_text.clone();
    // A zombie that turns into another zombie keeps canBreakDoors.
    if super::doors::is_zombie(from.species()) && super::doors::is_zombie(to.species()) {
        super::doors::set_can_break_doors(to, from.extra.can_break_doors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{items::ItemStack, mobs::world::with_test_world};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    #[test]
    fn a_frozen_skeleton_becomes_a_stray_that_keeps_its_bow_and_name() {
        with_test_world(&[], |_, types| {
            let mob = |id, kind: &str| {
                Mob::new(
                    id,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(3.5, 70.0, -2.5),
                    0.0,
                )
            };
            let mut s = mob(1, "minecraft:skeleton");
            s.yaw = 90.0;
            s.custom_name = Some("Frosty".into());
            s.no_ai = true;
            s.extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:bow", 1)));
            s.extra.chest = Some("minecraft:chainmail_chestplate".into());
            s.extra.leash = Some(crate::mobs::leash::Holder::Knot([3, 71, -3]));
            s.extra.mount = Some(7);
            s.extra.passenger = true;
            let mut st = mob(2, "minecraft:stray");
            single(&s, &mut st, Params::KEEP_ALL);
            assert_eq!(st.custom_name.as_deref(), Some("Frosty"));
            assert!(st.no_ai);
            assert_eq!(st.yaw, 90.0);
            assert_eq!(
                st.extra.main_hand_stack().map(|s| &*s.item),
                Some("minecraft:bow")
            );
            assert_eq!(
                st.extra.chest.as_deref(),
                Some("minecraft:chainmail_chestplate")
            );
            assert_eq!(
                st.extra.leash,
                Some(crate::mobs::leash::Holder::Knot([3, 71, -3]))
            );
            assert_eq!(st.extra.mount, Some(7));
            // Health is the new type's own, not copied.
            assert_eq!(st.health, mob(3, "minecraft:stray").health);
            // Without keepEquipment the gear stays behind.
            let mut bare = mob(4, "minecraft:stray");
            single(
                &s,
                &mut bare,
                Params {
                    keep_equipment: false,
                    preserve_can_pick_up_loot: false,
                },
            );
            assert!(bare.extra.main_hand.is_none() && bare.extra.chest.is_none());
            assert_eq!(bare.custom_name.as_deref(), Some("Frosty"));
        });
    }
}
