//! Farm animals: pigs, cows, mooshrooms, sheep and chickens (`registerGoals`,
//! chicken flapping and egg laying, sheep wool colours).
use super::{
    Mob, MobEvent, MobWorld, Species,
    goals::{
        Breed, EatBlock, Float, FollowParent, LookAtPlayer, Panic, RandomLookAround, Stroll, Tempt,
    },
};
use horizon_core::random::RandomSource;
use std::sync::Arc;

pub fn register(m: &mut Mob) {
    let food = m.kind.food.clone();
    let tempt = m.kind.tempt.clone();
    let g = &mut m.ai.goals;
    match m.kind.species {
        Species::Pig => {
            g.add(0, Float);
            g.add(1, Panic::new(1.25));
            g.add(3, Breed::new(1.0));
            g.add(
                4,
                Tempt::new(
                    1.2,
                    Arc::from(vec!["minecraft:carrot_on_a_stick".to_owned()]),
                    false,
                ),
            );
            g.add(4, Tempt::new(1.2, tempt, false));
            g.add(5, FollowParent::new(1.1));
            g.add(6, Stroll::avoiding_water(1.0));
            g.add(7, LookAtPlayer::new(6.0));
            g.add(8, RandomLookAround::default());
        }
        Species::Cow | Species::Mooshroom => {
            g.add(0, Float);
            g.add(1, Panic::new(2.0));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.25, food, false));
            g.add(4, FollowParent::new(1.25));
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(6.0));
            g.add(7, RandomLookAround::default());
        }
        Species::Sheep => {
            g.add(0, Float);
            g.add(1, Panic::new(1.25));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.1, food, false));
            g.add(4, FollowParent::new(1.1));
            g.add(5, EatBlock::default());
            g.add(6, Stroll::avoiding_water(1.0));
            g.add(7, LookAtPlayer::new(6.0));
            g.add(8, RandomLookAround::default());
        }
        Species::Chicken => {
            g.add(0, Float);
            g.add(1, Panic::new(1.4));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.0, food, false));
            g.add(4, FollowParent::new(1.1));
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(6.0));
            g.add(7, RandomLookAround::default());
            m.extra.egg_time = m.random.next_int_bound(6000).unwrap_or(0) + 6000;
            // `Chicken()`: water costs nothing.
            m.set_malus(super::path::PathType::Water, 0.0);
        }
        _ => {}
    }
}
/// Chicken.aiStep: wing flapping, slowed falling and laying eggs.
pub fn chicken_ai_step(m: &mut Mob, _w: &MobWorld, events: &mut Vec<MobEvent>) {
    chicken_flap(m);
    if !m.on_ground && m.velocity.y < 0.0 {
        m.velocity.y *= 0.6;
    }
    if m.alive() && !m.is_baby() && !m.extra.jockey {
        m.extra.egg_time -= 1;
        if m.extra.egg_time <= 0 {
            let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
            let at = [m.position.x, m.position.y, m.position.z];
            events.push(MobEvent::Sound("entity.chicken.egg", at, 1.0, pitch));
            events.push(MobEvent::GiftLoot {
                mob: m.id,
                table: "minecraft:gameplay/chicken_lay",
                at,
            });
            m.extra.egg_time = m.random.next_int_bound(6000).unwrap_or(0) + 6000;
        }
    }
}
/// Chicken.aiStep's wings: `flapSpeed` rises in the air and falls on the
/// ground, `flapping` restarts at 1 in the air and decays, and `flap` turns
/// by it. It runs on a client too, for the chickens it only displays (the
/// falling slowdown and the eggs are the server's).
pub fn chicken_flap(m: &mut Mob) {
    let a = &mut m.anim;
    a.flap_o = a.flap;
    a.flap_speed_o = a.flap_speed;
    a.flap_speed = (a.flap_speed + if m.on_ground { -1.0 } else { 4.0 } * 0.3).clamp(0.0, 1.0);
    if !m.on_ground && a.flapping < 1.0 {
        a.flapping = 1.0;
    }
    a.flapping *= 0.9;
    a.flap += a.flapping * 2.0;
}
/// Farm animal climate from the biome tags (`spawns_warm/cold_variant_farm_animals`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Climate {
    #[default]
    Temperate,
    Warm,
    Cold,
}
/// SheepColorSpawnRules.getSheepColor: a weighted list of 100 where the
/// climate's common colour (82) itself rolls pink 1 in 500.
pub fn sheep_color(climate: Climate, random: &mut impl RandomSource) -> u8 {
    // DyeColor ids: white 0, pink 6, gray 7, light gray 8, brown 12, black 15.
    let (list, common): ([(u8, i32); 4], u8) = match climate {
        Climate::Temperate => ([(15, 5), (7, 5), (8, 5), (12, 3)], 0),
        Climate::Warm => ([(7, 5), (8, 5), (0, 5), (15, 3)], 12),
        Climate::Cold => ([(8, 5), (7, 5), (0, 5), (12, 3)], 15),
    };
    let mut r = random.next_int_bound(100).unwrap_or(0);
    for (color, weight) in list {
        if r < weight {
            return color;
        }
        r -= weight;
    }
    if random.next_int_bound(500).unwrap_or(1) < 499 {
        common
    } else {
        6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    /// `chicken_flap` reads only the ground flag: any mob type will do.
    fn chicken() -> Mob {
        Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0)
    }

    #[test]
    fn a_falling_chicken_flaps_and_a_landed_one_folds_its_wings() {
        let mut m = chicken();
        m.on_ground = true;
        chicken_flap(&mut m);
        assert_eq!(m.anim.flap_speed, 0.0);
        assert_eq!(m.anim.flap, 0.0);
        // In the air flapSpeed rises by 1.2 (clamped to 1), flapping restarts
        // at 1 and decays by 0.9, and flap turns by twice flapping.
        m.on_ground = false;
        chicken_flap(&mut m);
        assert_eq!(m.anim.flap_speed, 1.0);
        assert_eq!(m.anim.flapping, 0.9);
        assert_eq!(m.anim.flap, 1.8);
        chicken_flap(&mut m);
        assert_eq!(m.anim.flap_o, 1.8);
        assert_eq!(m.anim.flap_speed_o, 1.0);
        assert_eq!(m.anim.flap, 3.6);
        // Landed, flapSpeed falls by 0.3 a tick and flapping decays.
        m.on_ground = true;
        chicken_flap(&mut m);
        assert!((m.anim.flap_speed - 0.7).abs() < 1.0e-6);
        assert!((m.anim.flapping - 0.81).abs() < 1.0e-6);
    }
}
