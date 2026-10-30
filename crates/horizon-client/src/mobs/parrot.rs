//! Parrots: flying (`FlyingMoveControl`, `FlyingPathNavigation`, the wander
//! that heads for trees), flapping and gliding down (`calculateFlapping`),
//! and what `Parrot.aiStep` adds: imitating the mobs around it
//! (`imitateNearbyMobs`, the ambient sound of `getAmbient`) and dancing near
//! a playing jukebox.
use super::{Mob, MobEvent, MobWorld, Species, control::MoveKind, nav::NavKind, path::PathType};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `Parrot.MOB_SOUND_MAP` (the happy ghast's is `SoundEvents.EMPTY`: "").
const IMITATIONS: [(Species, &str); 41] = [
    (Species::Blaze, "entity.parrot.imitate.blaze"),
    (Species::Bogged, "entity.parrot.imitate.bogged"),
    (Species::Breeze, "entity.parrot.imitate.breeze"),
    (Species::CamelHusk, "entity.parrot.imitate.camel_husk"),
    (Species::CaveSpider, "entity.parrot.imitate.spider"),
    (Species::Creaking, "entity.parrot.imitate.creaking"),
    (Species::Creeper, "entity.parrot.imitate.creeper"),
    (Species::Drowned, "entity.parrot.imitate.drowned"),
    (
        Species::ElderGuardian,
        "entity.parrot.imitate.elder_guardian",
    ),
    (Species::Endermite, "entity.parrot.imitate.endermite"),
    (Species::Evoker, "entity.parrot.imitate.evoker"),
    (Species::Ghast, "entity.parrot.imitate.ghast"),
    (Species::HappyGhast, ""),
    (Species::Guardian, "entity.parrot.imitate.guardian"),
    (Species::Hoglin, "entity.parrot.imitate.hoglin"),
    (Species::Husk, "entity.parrot.imitate.husk"),
    (Species::Illusioner, "entity.parrot.imitate.illusioner"),
    (Species::MagmaCube, "entity.parrot.imitate.magma_cube"),
    (Species::Parched, "entity.parrot.imitate.parched"),
    (Species::Phantom, "entity.parrot.imitate.phantom"),
    (Species::Piglin, "entity.parrot.imitate.piglin"),
    (Species::PiglinBrute, "entity.parrot.imitate.piglin_brute"),
    (Species::Pillager, "entity.parrot.imitate.pillager"),
    (Species::Ravager, "entity.parrot.imitate.ravager"),
    (Species::Shulker, "entity.parrot.imitate.shulker"),
    (Species::Silverfish, "entity.parrot.imitate.silverfish"),
    (Species::Skeleton, "entity.parrot.imitate.skeleton"),
    (Species::Slime, "entity.parrot.imitate.slime"),
    (Species::Spider, "entity.parrot.imitate.spider"),
    (Species::Stray, "entity.parrot.imitate.stray"),
    (Species::Vex, "entity.parrot.imitate.vex"),
    (Species::Vindicator, "entity.parrot.imitate.vindicator"),
    (Species::Warden, "entity.parrot.imitate.warden"),
    (Species::Witch, "entity.parrot.imitate.witch"),
    (Species::Wither, "entity.parrot.imitate.wither"),
    (
        Species::WitherSkeleton,
        "entity.parrot.imitate.wither_skeleton",
    ),
    (Species::Zoglin, "entity.parrot.imitate.zoglin"),
    (Species::Zombie, "entity.parrot.imitate.zombie"),
    (Species::ZombieHorse, "entity.parrot.imitate.zombie_horse"),
    (
        Species::ZombieNautilus,
        "entity.parrot.imitate.zombie_nautilus",
    ),
    (
        Species::ZombieVillager,
        "entity.parrot.imitate.zombie_villager",
    ),
];

fn imitation(species: Species) -> Option<&'static str> {
    IMITATIONS.iter().find(|i| i.0 == species).map(|i| i.1)
}

/// `Parrot.getPitch`.
fn pitch(random: &mut LegacyRandom) -> f32 {
    (random.next_float() - random.next_float()) * 0.2 + 1.0
}

/// The start of `Parrot.aiStep`: a jukebox that is gone or farther than
/// 3.46 blocks stops the dance, and one tick in 400 the parrot imitates a
/// mob nearby.
pub fn ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if let Some(at) = m.extra.party_jukebox {
        let d = |i: usize, v: f64| f64::from(at[i]) + 0.5 - v;
        let (x, y, z) = (d(0, m.position.x), d(1, m.position.y), d(2, m.position.z));
        let jukebox = &*w.facts.get(w.world.block(at[0], at[1], at[2])).name == "minecraft:jukebox";
        if x * x + y * y + z * z >= 3.46 * 3.46 || !jukebox {
            m.extra.party_jukebox = None;
        }
    }
    if level_random.next_int_bound(400).unwrap_or(1) == 0 {
        imitate_nearby_mobs(m, w, level_random, events);
    }
}

/// `Parrot.imitateNearbyMobs`: half the time, a random imitable mob within 20
/// blocks is heard through the parrot's own mouth.
fn imitate_nearby_mobs(
    m: &Mob,
    w: &MobWorld,
    random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if !m.alive() || m.silent || random.next_int_bound(2).unwrap_or(1) != 0 {
        return;
    }
    let area = m.bounds().inflate(Vec3::new(20.0, 20.0, 20.0));
    let mobs: Vec<_> = w
        .others
        .iter()
        .filter(|s| s.alive && imitation(s.species).is_some() && s.bounds.intersects(area))
        .collect();
    if mobs.is_empty() {
        return;
    }
    let mob = mobs[random.next_int_bound(mobs.len() as i32).unwrap_or(0) as usize];
    if mob.silent {
        return;
    }
    if let Some(sound) = imitation(mob.species).filter(|s| !s.is_empty()) {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::Sound(sound, at, 0.7, pitch(random)));
    }
}

/// `Parrot.getAmbient`: now and then (outside peaceful) the voice of a random
/// imitable mob instead of the parrot's own.
pub fn ambient(m: &mut Mob, w: &MobWorld) -> &'static str {
    if w.difficulty.effective > 0.0 && m.random.next_int_bound(1000).unwrap_or(1) == 0 {
        let i = m
            .random
            .next_int_bound(IMITATIONS.len() as i32)
            .unwrap_or(0);
        return IMITATIONS[i as usize].1;
    }
    "entity.parrot.ambient"
}

/// `Parrot(EntityType, Level)` with `createNavigation` and `registerGoals`:
/// a `FlyingMoveControl(10, false)` that lets gravity back on when it has
/// nowhere to go, and a `FlyingPathNavigation` that floats and opens no doors.
pub fn register(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Flying {
        max_turn: 10.0,
        hovers: false,
    };
    m.ai.nav.kind = NavKind::Flying;
    m.ai.nav.can_open_doors = false;
    m.ai.nav.can_float = true;
    m.set_malus(PathType::FireInNeighbor, -1.0);
    m.set_malus(PathType::Fire, -1.0);
    m.set_malus(PathType::Cocoa, -1.0);
    super::tame::register_parrot(m);
}

/// The wings of `Parrot.calculateFlapping`: `flapSpeed` rises in the air and
/// falls on the ground or a shoulder, `flapping` restarts at 1 in the air and
/// decays, and `flap` turns by it. A client runs it for the parrots it only
/// displays.
pub fn flap(m: &mut Mob) {
    let grounded = m.on_ground || m.extra.mount.is_some();
    let a = &mut m.anim;
    a.flap_o = a.flap;
    a.flap_speed_o = a.flap_speed;
    a.flap_speed = (a.flap_speed + if grounded { -1.0 } else { 4.0 } * 0.3).clamp(0.0, 1.0);
    if !m.on_ground && a.flapping < 1.0 {
        a.flapping = 1.0;
    }
    a.flapping *= 0.9;
    a.flap += a.flapping * 2.0;
}

/// `Parrot.calculateFlapping`, after the rest of `aiStep`: the wings, and a
/// parrot falling through the air glides down at 0.6 of the speed.
pub fn calculate_flapping(m: &mut Mob) {
    flap(m);
    if !m.on_ground && m.velocity.y < 0.0 {
        m.velocity.y *= 0.6;
    }
}

/// `Parrot.isFlapping` and `onFlap` (from `Entity.processFlappingMovement`,
/// when a move ends in the air): a wing beat each half `flapSpeed` flown.
pub fn process_flapping(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.ai.steps.fly_dist > m.ai.steps.next_flap {
        if !m.silent {
            let at = [m.position.x, m.position.y, m.position.z];
            events.push(MobEvent::Sound("entity.parrot.fly", at, 0.15, 1.0));
        }
        m.ai.steps.next_flap = m.ai.steps.fly_dist + m.anim.flap_speed / 2.0;
    }
}

/// `Parrot.ParrotWanderGoal.getPosition`: out of water onto land; otherwise
/// (all but one time in a thousand) the top of a tree nearby; failing both,
/// `WaterAvoidingRandomFlyingGoal.getPosition`.
pub fn wander_pos(m: &mut Mob, w: &MobWorld) -> Option<[f64; 3]> {
    let mut pos = None;
    if m.fluids.in_water {
        pos = m.land_random_pos(w, 15, 15);
    }
    // WaterAvoidingRandomStrollGoal.probability.
    if m.random.next_float() >= 0.001 {
        pos = tree_pos(m, w);
    }
    pos.or_else(|| m.flying_stroll_pos(w))
}

/// `Parrot.ParrotWanderGoal.getTreePos`: within 3 blocks across and 6 up or
/// down (x fastest, then y, then z, as `BlockPos.betweenClosed` walks), the
/// first empty block with an empty one above over leaves or a log.
fn tree_pos(m: &Mob, w: &MobWorld) -> Option<[f64; 3]> {
    let (x, y, z) = (m.position.x, m.position.y, m.position.z);
    let at = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
    let air = |p: [i32; 3]| {
        matches!(
            w.block_name(p[0], p[1], p[2]),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    };
    for bz in (z - 3.0).floor() as i32..=(z + 3.0).floor() as i32 {
        for by in (y - 6.0).floor() as i32..=(y + 6.0).floor() as i32 {
            for bx in (x - 3.0).floor() as i32..=(x + 3.0).floor() as i32 {
                let p = [bx, by, bz];
                if p == at {
                    continue;
                }
                let below = w.block_name(bx, by - 1, bz);
                let tree = below.ends_with("_leaves") || w.has_tag("minecraft:logs", below);
                if tree && air(p) && air([bx, by + 1, bz]) {
                    return Some([f64::from(bx) + 0.5, f64::from(by), f64::from(bz) + 0.5]);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use std::sync::Arc;

    fn parrot() -> Mob {
        let mut k = (*kind()).clone();
        k.id = "minecraft:parrot".into();
        k.species = Species::Parrot;
        k.flags.omnidirectional = true;
        Mob::new(1, Arc::new(k), Vec3::ZERO, 0.0)
    }

    /// `Parrot()`: `FlyingMoveControl(this, 10, false)`, the path maluses,
    /// and `createNavigation`'s `FlyingPathNavigation` (no doors, floats).
    #[test]
    fn a_parrot_flies_by_its_navigation_and_move_control() {
        let m = parrot();
        assert_eq!(
            m.ai.mov.kind,
            MoveKind::Flying {
                max_turn: 10.0,
                hovers: false
            }
        );
        assert_eq!(m.ai.nav.kind, NavKind::Flying);
        assert!(!m.ai.nav.can_open_doors);
        assert!(m.ai.nav.can_float);
        for t in [PathType::FireInNeighbor, PathType::Fire, PathType::Cocoa] {
            assert_eq!(m.malus(t), -1.0);
        }
        assert!(m.ai.goals.has("ParrotWander"));
    }

    /// `Parrot.calculateFlapping`: in the air the wings beat and a fall is
    /// slowed to 0.6; on the ground or a shoulder they fold.
    #[test]
    fn a_falling_parrot_flaps_and_glides() {
        let mut m = parrot();
        m.on_ground = false;
        m.velocity = Vec3::new(0.1, -0.5, 0.0);
        calculate_flapping(&mut m);
        assert!((m.velocity.y + 0.3).abs() < 1.0e-12);
        assert_eq!(m.velocity.x, 0.1);
        assert_eq!(m.anim.flap_speed, 1.0);
        assert_eq!(m.anim.flapping, 0.9);
        assert_eq!(m.anim.flap, 1.8);
        // Rising is not slowed.
        m.velocity = Vec3::new(0.0, 0.2, 0.0);
        calculate_flapping(&mut m);
        assert_eq!(m.velocity.y, 0.2);
        assert_eq!(m.anim.flap_o, 1.8);
        // On a shoulder (a passenger), flapSpeed falls by 0.3 a tick.
        m.extra.mount = Some(7);
        calculate_flapping(&mut m);
        assert!((m.anim.flap_speed - 0.7).abs() < 1.0e-6);
        m.extra.mount = None;
        m.on_ground = true;
        m.velocity = Vec3::new(0.0, -0.0784, 0.0);
        calculate_flapping(&mut m);
        assert!((m.anim.flap_speed - 0.4).abs() < 1.0e-6);
        assert_eq!(m.velocity.y, -0.0784, "no glide on the ground");
    }

    /// `Parrot.onFlap`: a wing beat (`entity.parrot.fly`, 0.15) each half
    /// `flapSpeed` flown.
    #[test]
    fn it_beats_its_wings_as_it_flies() {
        let mut m = parrot();
        m.anim.flap_speed = 1.0;
        let mut events = Vec::new();
        m.ai.steps.fly_dist = 0.5;
        process_flapping(&mut m, &mut events);
        assert!(events.is_empty(), "nextFlap starts at 1");
        m.ai.steps.fly_dist = 1.2;
        process_flapping(&mut m, &mut events);
        assert!(matches!(
            events.as_slice(),
            [MobEvent::Sound("entity.parrot.fly", _, v, _)] if *v == 0.15
        ));
        assert!((m.ai.steps.next_flap - 1.7).abs() < 1.0e-6);
    }
}
