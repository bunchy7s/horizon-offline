//! Effective mob armor and toughness at the damage boundary.
use super::{Mob, Species};
use horizon_world::attributes::{Definition, Instance, Observer, entity::EntityAttributes};
use std::convert::Infallible;

struct Calculation;
impl Observer for Calculation {
    type Error = Infallible;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Infallible> {
        Ok(())
    }
}

fn effective(
    map: Option<&EntityAttributes>,
    name: &str,
    base: f64,
    bonus: f64,
    maximum: f64,
) -> f64 {
    if let Some(map) = map
        && let Some(key) = map.catalog().id(name)
    {
        if let Some(source) = map.instance(key) {
            // Equipment and the covered shell contribute in the additive
            // phase, before multiplicative modifiers and effective clamping.
            let mut value = source.clone();
            value
                .set_base_value(source.base_value() + bonus, &mut Calculation)
                .unwrap();
            return value.value();
        }
        let definition = map.catalog().attribute(key).definition;
        return definition.sanitize(map.base_value(key).unwrap_or(base) + bonus);
    }
    Definition::ranged(0.0, 0.0, maximum)
        .unwrap()
        .sanitize(base + bonus)
}

/// LivingEntity.getArmorValue floors the effective double before converting
/// its integer result to float. Toughness converts its effective double directly.
pub fn values(mob: &Mob) -> (f32, f32) {
    let (worn, worn_toughness) = mob.worn_armor();
    let covered = if mob.species() == Species::Shulker && mob.extra.counter == 0 {
        20.0
    } else {
        0.0
    };
    let map = mob.extra.armor_attributes.as_deref();
    let armor = effective(
        map,
        "minecraft:armor",
        f64::from(mob.kind.armor),
        f64::from(worn) + covered,
        30.0,
    );
    let toughness = effective(
        map,
        "minecraft:armor_toughness",
        f64::from(mob.kind.armor_toughness),
        f64::from(worn_toughness),
        20.0,
    );
    (armor.floor() as i32 as f32, toughness as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{Damage, DamageKind};
    use horizon_core::geometry::Vec3;
    use horizon_world::attributes::entity::Catalog;
    use std::sync::Arc;

    fn pig(armor: f64, toughness: f64) -> Mob {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "attributes": [
                {"id":"minecraft:armor", "default":bits(0.0), "min":bits(0.0), "max":bits(30.0), "syncable":true},
                {"id":"minecraft:armor_toughness", "default":bits(0.0), "min":bits(0.0), "max":bits(20.0), "syncable":true}
            ],
            "entities": {"minecraft:pig":{"minecraft:armor":bits(0.0), "minecraft:armor_toughness":bits(0.0)}}
        })).unwrap());
        let mut attributes = catalog.create("minecraft:pig").unwrap();
        for (name, base) in [
            ("minecraft:armor", armor),
            ("minecraft:armor_toughness", toughness),
        ] {
            let key = catalog.id(name).unwrap();
            attributes.edit(key, |value, observer| {
                value.set_base_value(base, observer).unwrap()
            });
        }
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:pig".into();
        ty.species = Species::Pig;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.armor_attributes = Some(Box::new(attributes));
        mob.attrs.max_health = 1000.0;
        mob.health = 1000.0;
        mob
    }

    #[test]
    fn effective_mob_armor_controls_damage_and_fractional_armor_floors() {
        for (armor, toughness, source, expected) in [
            (20.0, 8.0, DamageKind::Cactus, 999039),
            (0.0, 8.0, DamageKind::Cactus, 996000),
            (0.5, 8.0, DamageKind::Cactus, 996000),
            (20.0, 8.0, DamageKind::SonicBoom, 996000),
        ] {
            let mut mob = pig(armor, toughness);
            assert!(mob.damage(4.0, Damage::of(source)));
            assert_eq!((f64::from(mob.health) * 1000.0) as i32, expected);
        }
    }

    #[test]
    fn equipment_and_covered_shell_are_counted_once_before_effective_limits() {
        let mut mob = pig(20.0, 8.0);
        mob.extra.head = Some("minecraft:diamond_helmet".into());
        assert_eq!(values(&mob), (23.0, 10.0));
        mob.extra.head = None;
        Arc::make_mut(&mut mob.kind).species = Species::Shulker;
        let map = mob.extra.armor_attributes.as_deref_mut().unwrap();
        let key = map.catalog().id("minecraft:armor").unwrap();
        map.edit(key, |value, observer| {
            value.set_base_value(5.0, observer).unwrap()
        });
        mob.extra.counter = 0;
        assert_eq!(values(&mob), (25.0, 8.0));
        mob.extra.counter = 1;
        assert_eq!(values(&mob), (5.0, 8.0));
        let mob = pig(100.0, 100.0);
        assert_eq!(values(&mob), (30.0, 20.0));
    }
}
