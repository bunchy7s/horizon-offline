//! Zombies (zombie, husk, drowned, zombie villager) and skeletons (skeleton,
//! stray, bogged, wither skeleton): goals, bow attacks, water conversion.
use super::{
    Damage, Mob, MobEvent, Species, Who,
    doors::BreakDoor,
    goals::{
        Avoid, AvoidEntity, Cx, FleeSun, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack,
        NearestTarget, RandomLookAround, RestrictSun, Stroll, goal_clone,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `Zombie.hurtServer`, once the hit landed: a zombie with a target (its own,
/// else a living attacker) asks the game to try calling a reinforcement; the
/// difficulty, chance and spawn tests are the game's (`call_reinforcements`).
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    if let Some(target) = m.ai.target.or(source.attacker) {
        m.outbox
            .push(MobEvent::Reinforcements { mob: m.id, target });
    }
}

pub fn register(m: &mut Mob) {
    use Species as S;
    match m.species() {
        S::Zombie | S::Husk | S::ZombieVillager => {
            let g = &mut m.ai.goals;
            g.add(1, BreakDoor::zombie());
            g.add(4, AttackTurtleEgg::default());
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            g.add(3, MeleeAttack::new(1.0, false).zombie());
            g.add(7, Stroll::avoiding_water(1.0));
            let t = &mut m.ai.targets;
            let mut hurt = HurtByTarget::new().alert();
            hurt.alert_except = vec![S::ZombifiedPiglin];
            t.add(1, hurt);
            t.add(2, NearestTarget::player(true));
            t.add(
                3,
                NearestTarget::species(&[S::Villager, S::WanderingTrader], false),
            );
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(
                5,
                NearestTarget::new(
                    super::goals::Prey::Filter(|s| s.species == Species::Turtle && s.baby),
                    true,
                )
                .interval(10),
            );
        }
        S::ZombifiedPiglin => {
            // ZombifiedPiglin.addBehaviourGoals: neutral until provoked.
            let g = &mut m.ai.goals;
            g.add(1, BreakDoor::zombie());
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            g.add(2, MeleeAttack::new(1.0, false).zombie());
            g.add(7, Stroll::avoiding_water(1.0));
            let t = &mut m.ai.targets;
            t.add(1, HurtByTarget::new().alert());
            t.add(
                2,
                NearestTarget::player(true)
                    .interval(10)
                    .when(|m, _| m.extra.anger > 0),
            );
        }
        S::Drowned => {
            // Zombie.registerGoals, then Drowned.addBehaviourGoals.
            let g = &mut m.ai.goals;
            g.add(1, BreakDoor::zombie());
            g.add(4, AttackTurtleEgg::default());
            g.add(8, LookAtPlayer::new(8.0));
            g.add(8, RandomLookAround::default());
            g.add(1, DrownedGoToWater::default());
            g.add(
                2,
                DrownedTridentAttack(super::witch::RangedAttack::new(
                    1.0,
                    40,
                    40,
                    10.0,
                    drowned_throw,
                )),
            );
            g.add(2, DrownedAttack(MeleeAttack::new(1.0, false).zombie()));
            g.add(
                5,
                DrownedGoToBeach(super::goals::MoveToBlock::new(1.0, 8, 2)),
            );
            g.add(6, DrownedSwimUp { stuck: false });
            g.add(7, Stroll::new(1.0));
            let t = &mut m.ai.targets;
            let mut hurt = HurtByTarget::new().alert().ignoring(&[S::Drowned]);
            hurt.alert_except = vec![S::ZombifiedPiglin];
            t.add(1, hurt);
            t.add(2, NearestTarget::player(true).when(drowned_can_attack));
            t.add(
                3,
                NearestTarget::species(&[S::Villager, S::WanderingTrader], false),
            );
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(3, NearestTarget::species(&[S::Axolotl], true));
            t.add(
                5,
                NearestTarget::new(
                    super::goals::Prey::Filter(|s| s.species == Species::Turtle && s.baby),
                    true,
                )
                .interval(10),
            );
            // DrownedMoveControl, AmphibiousPathNavigation, no water malus.
            m.ai.mov.kind = super::control::MoveKind::Custom;
            m.ai.nav.kind = super::nav::NavKind::Amphibious;
            m.ai.nav.can_float = false;
            m.set_malus(super::path::PathType::Water, 0.0);
        }
        S::Skeleton | S::Stray | S::Bogged | S::Parched | S::WitherSkeleton => {
            let species = m.species();
            let g = &mut m.ai.goals;
            g.add(2, RestrictSun);
            g.add(3, FleeSun::new(1.0));
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Wolf]), 6.0, 1.0, 1.2),
            );
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            let t = &mut m.ai.targets;
            t.add(1, HurtByTarget::new());
            t.add(2, NearestTarget::player(true));
            t.add(3, NearestTarget::species(&[S::IronGolem], true));
            t.add(
                3,
                NearestTarget::new(
                    super::goals::Prey::Filter(|s| s.species == Species::Turtle && s.baby),
                    true,
                )
                .interval(10),
            );
            if species == S::WitherSkeleton {
                t.add(
                    3,
                    NearestTarget::species(&[S::Piglin, S::PiglinBrute], true),
                );
            }
            // The AbstractSkeleton constructor's reassessWeaponGoal, after
            // registerGoals; the weapon itself comes with finalizeSpawn.
            // getHardAttackInterval / getAttackInterval.
            let (hard, normal) = if matches!(species, S::Bogged | S::Parched) {
                (50, 70)
            } else {
                (20, 40)
            };
            m.ai.goals.add(4, SkeletonWeapon::new(hard, normal));
        }
        _ => {}
    }
}
/// Drowned.okTarget: at night, or when the target is in water.
fn drowned_can_attack(_m: &Mob, w: &super::MobWorld) -> bool {
    let Some(p) = &w.player else { return false };
    !w.daylight || in_water_at(w, p.position)
}
/// The target's isInWater, from the water at its feet.
fn in_water_at(w: &super::MobWorld, p: Vec3) -> bool {
    let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    w.fluid(x, y, z) == crate::blocks::Fluid::Water
}
/// Drowned.okTarget for the current target.
fn drowned_ok_target(m: &Mob, w: &super::MobWorld) -> bool {
    m.target_info(w)
        .is_some_and(|t| !w.daylight || in_water_at(w, t.position))
}
// Drowned state: `flag` wantsToSwim (target in water), `flag2` searchingForLand.
/// Drowned.wantsToSwim
pub fn drowned_wants_to_swim(m: &Mob, w: &super::MobWorld) -> bool {
    m.extra.flag2 || m.target_info(w).is_some_and(|t| in_water_at(w, t.position))
}
/// `Drowned.updateSwimming`: swimming under water while it wants to. `flag`
/// holds `isSwimming`, which clients take from the entity's shared flags.
pub fn drowned_update_swimming(m: &mut Mob, w: &super::MobWorld) {
    m.extra.flag =
        !m.no_ai && m.was_eye_in_water && m.fluids.in_water && drowned_wants_to_swim(m, w);
}
/// A drowned on a client: `LivingEntity.updateSwimAmount` follows the
/// swimming flag (`Drowned.isVisuallySwimming`).
pub fn drowned_client_tick(m: &mut Mob) {
    m.extra.swim.tick(m.extra.flag, m.fluids.in_water);
}
/// Drowned.DrownedMoveControl
pub fn drowned_move_control(m: &mut Mob, w: &super::MobWorld) {
    let target = m.target_info(w);
    if drowned_wants_to_swim(m, w) && m.fluids.in_water {
        if target.is_some_and(|t| t.position.y > m.position.y) || m.extra.flag2 {
            m.velocity = Vec3::new(m.velocity.x, m.velocity.y + 0.002, m.velocity.z);
        }
        if m.ai.mov.operation != super::control::Operation::MoveTo || m.ai.nav.is_done() {
            m.set_speed(0.0);
            return;
        }
        let [wx, wy, wz] = m.ai.mov.wanted;
        let (xd, mut yd, zd) = (wx - m.position.x, wy - m.position.y, wz - m.position.z);
        let dd = (xd * xd + yd * yd + zd * zd).sqrt();
        yd /= dd;
        let y_rot = horizon_core::geometry::trig::atan2_degrees(zd, xd) - 90.0;
        m.yaw = super::control::rotlerp(m.yaw, y_rot, 90.0);
        m.body_yaw = m.yaw;
        let target_speed = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
        let speed = m.ai.speed + 0.125 * (target_speed - m.ai.speed);
        m.set_speed(speed);
        let s = f64::from(speed);
        m.velocity = Vec3::new(
            m.velocity.x + s * xd * 0.005,
            m.velocity.y + s * yd * 0.1,
            m.velocity.z + s * zd * 0.005,
        );
    } else {
        if !m.on_ground {
            m.velocity = Vec3::new(m.velocity.x, m.velocity.y - 0.008, m.velocity.z);
        }
        let speed = m.movement_speed();
        m.ground_move_tick(w, speed);
    }
}
/// Drowned.travelInWater: swimming under water slowly when it wants to.
pub fn drowned_travel(
    m: &mut Mob,
    w: &super::MobWorld,
    input: Vec3,
    events: &mut Vec<MobEvent>,
) -> bool {
    // isUnderWater: wasEyeInWater && isInWater.
    if !(m.was_eye_in_water && m.fluids.in_water && drowned_wants_to_swim(m, w)) {
        return false;
    }
    m.move_relative(0.01, input);
    m.do_move(w, events);
    m.velocity = Vec3::new(m.velocity.x * 0.9, m.velocity.y * 0.9, m.velocity.z * 0.9);
    true
}
/// The trident throw of `Drowned.performRangedAttack`: a `ThrownTrident` of
/// the held trident (a plain one when it holds none), owned by the drowned,
/// which cannot be picked up and does not return (`AbstractArrow` constructor:
/// only a player's projectile is `Pickup.ALLOWED`).
fn drowned_throw(m: &mut Mob, who: Who, _power: f32, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let xd = t.position.x - m.position.x;
    let yd = t.position.y + (t.bounds.max_position().y - t.bounds.min_position().y) / 3.0 - start.y;
    let zd = t.position.z - m.position.z;
    let dist = (xd * xd + zd * zd).sqrt();
    let mut trident = crate::items::ItemStack::new("minecraft:trident", 1);
    if m.extra.main_hand.as_deref() == Some("minecraft:trident")
        && let Some(g) = super::gear::gear(m)
    {
        for (e, l) in &g.slots[super::gear::MAIN] {
            trident.enchant(e, *l);
        }
    }
    let mut p = Projectile::new(ProjectileKind::Trident, start, Some(Who::Mob(m.id)));
    if let Some(a) = p.arrow.as_mut() {
        a.set_item(&trident);
    }
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [xd, yd + dist * f64::from(0.2_f32), zd],
        1.6,
        (14 - 2 * 4) as f32,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 / (m.random.next_float() * 0.4 + 0.8);
    cx.events.push(MobEvent::Sound(
        "entity.drowned.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
/// Drowned.DrownedTridentAttackGoal: the ranged goal while holding a trident.
#[derive(Clone, Debug)]
struct DrownedTridentAttack(super::witch::RangedAttack);
impl Goal for DrownedTridentAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        "DrownedTridentAttackGoal"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx) && m.extra.main_hand.as_deref() == Some("minecraft:trident")
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
        m.ai.aggressive = true;
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
        m.ai.aggressive = false;
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
/// Drowned.DrownedAttackGoal: ZombieAttackGoal on a target it may attack.
#[derive(Clone, Debug)]
struct DrownedAttack(MeleeAttack);
impl Goal for DrownedAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        "DrownedAttackGoal"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx) && drowned_ok_target(m, cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx) && drowned_ok_target(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
/// Drowned.DrownedGoToBeachGoal: at night, swim near the surface to a
/// standable block with air above.
#[derive(Clone, Debug)]
struct DrownedGoToBeach(super::goals::MoveToBlock);
fn beach_target(w: &super::MobWorld, p: [i32; 3]) -> bool {
    let empty = |y: i32| {
        let state = w.world.block(p[0], y, p[2]);
        w.facts.is_air(state) && w.facts.get(state).fluid == crate::blocks::Fluid::None
    };
    // BlockState.entityCanStandOn: a full top face.
    empty(p[1] + 1)
        && empty(p[1] + 2)
        && w.facts.get(w.world.block(p[0], p[1], p[2])).sturdy_full & (1 << 1) != 0
}
impl Goal for DrownedGoToBeach {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | super::goals::JUMP
    }
    fn name(&self) -> &'static str {
        "DrownedGoToBeachGoal"
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // MoveToBlockGoal.canUse, then the drowned's conditions.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        self.0.next_start = super::goals::reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.find_nearest(m, cx.w, beach_target)
            && !cx.w.daylight
            && m.fluids.in_water
            && m.position.y >= f64::from(cx.w.sea_level - 3)
    }
    fn can_continue(&mut self, _m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(cx.w, beach_target)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.flag2 = false;
        self.0.start(m, cx.w);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx.w);
    }
}
/// Drowned.DrownedSwimUpGoal: at night, deep in water, head for the surface.
#[derive(Clone, Debug)]
struct DrownedSwimUp {
    stuck: bool,
}
impl Goal for DrownedSwimUp {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn name(&self) -> &'static str {
        "DrownedSwimUpGoal"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !cx.w.daylight && m.fluids.in_water && m.position.y < f64::from(cx.w.sea_level - 2)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx) && !self.stuck
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.flag2 = true;
        self.stuck = false;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.flag2 = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let sea = f64::from(cx.w.sea_level);
        if m.position.y < sea - 1.0 && (m.ai.nav.is_done() || drowned_close_to_next_pos(m)) {
            let to = [m.position.x, sea - 1.0, m.position.z];
            match m.default_random_pos_towards(cx.w, 4, 8, to, std::f64::consts::FRAC_PI_2) {
                None => self.stuck = true,
                Some(p) => {
                    m.nav_move_to(cx.w, p, 1.0);
                }
            }
        }
    }
}
/// Drowned.closeToNextPos: within two blocks of the path's target.
fn drowned_close_to_next_pos(m: &Mob) -> bool {
    m.ai.nav.path.as_ref().is_some_and(|p| {
        let t = p.target;
        m.distance_sqr(Vec3::new(f64::from(t[0]), f64::from(t[1]), f64::from(t[2]))) < 4.0
    })
}
/// Zombie drowning conversion (Zombie.DrowningTracker) and husk -> zombie.
/// `ZombifiedPiglin.customServerAiStep`: faster and loud while angry,
/// forgetting the player when the anger runs out, rallying the others.
fn zombified_piglin_step(m: &mut Mob, cx: &mut Cx) {
    let base = 0.23;
    // NeutralMob.updatePersistentAnger: a new target starts 20-39 s of anger.
    if m.ai.target.is_some() && !m.extra.flag2 {
        m.extra.flag2 = true;
        m.extra.anger = 400 + m.random.next_int_bound(380).unwrap_or(0);
        m.extra.counter = m.random.next_int_bound(21).unwrap_or(0).max(1);
        m.extra.counter2 = 80 + m.random.next_int_bound(41).unwrap_or(0);
    } else if m.ai.target.is_none() {
        m.extra.flag2 = false;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if !m.is_baby() {
            m.attrs.speed = base + 0.05;
        }
        // playFirstAngerSoundIn
        if m.extra.counter > 0 {
            m.extra.counter -= 1;
            if m.extra.counter == 0 {
                let base = if m.is_baby() { 1.5 } else { 1.0 };
                let pitch = ((m.random.next_float() - m.random.next_float()) * 0.2 + base) * 1.8;
                cx.events.push(MobEvent::Sound(
                    "entity.zombified_piglin.angry",
                    m.center(),
                    2.0,
                    pitch,
                ));
            }
        }
        if m.extra.anger == 0 {
            m.ai.target = None;
            m.extra.flag2 = false;
        }
    } else {
        m.attrs.speed = base;
    }
    // maybeAlertOthers every 4 to 6 seconds while seeing the target.
    if let Some(target) = m.ai.target {
        if m.extra.counter2 > 0 {
            m.extra.counter2 -= 1;
        } else {
            if m.sees(cx.w, target) {
                let range = m.attrs.follow_range;
                let (id, pos) = (m.id, m.position);
                let others: Vec<u32> =
                    cx.w.others
                        .iter()
                        .filter(|s| {
                            s.id != id
                                && s.alive
                                && s.species == Species::ZombifiedPiglin
                                && s.target.is_none()
                                && (s.position.x - pos.x).abs() <= range + 1.0
                                && (s.position.y - pos.y).abs() <= 11.0
                                && (s.position.z - pos.z).abs() <= range + 1.0
                        })
                        .map(|s| s.id)
                        .collect();
                for other in others {
                    cx.events.push(MobEvent::SetTarget { mob: other, target });
                }
            }
            m.extra.counter2 = 80 + m.random.next_int_bound(41).unwrap_or(0);
        }
    }
}
/// `Entity.isInPowderSnow`: the mob's box, shrunk by `1.0E-5` as in
/// `checkInsideBlocks`, touches powder snow.
fn in_powder_snow(m: &Mob, w: &super::MobWorld) -> bool {
    let b = m.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let (min, max) = (b.min_position(), b.max_position());
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for y in min.y.floor() as i32..=max.y.floor() as i32 {
            for z in min.z.floor() as i32..=max.z.floor() as i32 {
                if &*w.facts.get(w.world.block(x, y, z)).name == "minecraft:powder_snow" {
                    return true;
                }
            }
        }
    }
    false
}
/// `Skeleton.freezingTracker` (`ConversionTracker.tick`): 140 ticks in
/// powder snow start a conversion into a stray that takes 300 more; leaving
/// the snow stops it. `counter` is the affliction time (-1 outside the snow)
/// and `counter2` the conversion time plus one while converting, so that
/// reaching 0 is vanilla's `conversionTime < 0`.
fn skeleton_freezing(m: &mut Mob, cx: &mut Cx) {
    if !in_powder_snow(m, cx.w) {
        m.extra.counter = -1;
        m.extra.counter2 = 0;
    } else if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
        if m.extra.counter2 == 0 {
            // levelEvent 1048: the conversion sound at the stray.
            let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
            cx.events.push(MobEvent::Sound(
                "entity.skeleton.converted_to_stray",
                m.center(),
                2.0,
                pitch,
            ));
            cx.events.push(MobEvent::SpawnKind {
                kind: "minecraft:stray".into(),
                at: [m.position.x, m.position.y, m.position.z],
                setup: super::SpawnSetup {
                    yaw: m.yaw,
                    persistent: m.persistent,
                    ..Default::default()
                },
            });
            m.removed = true;
        }
    } else {
        m.extra.counter += 1;
        if m.extra.counter >= 140 {
            m.extra.counter2 = 301;
        }
    }
}
pub fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    if m.species() == Species::Skeleton && m.alive() && !m.no_ai {
        skeleton_freezing(m, cx);
    }
    if m.species() == Species::ZombifiedPiglin {
        return zombified_piglin_step(m, cx);
    }
    if matches!(m.species(), Species::Zombie | Species::Husk) && m.alive() {
        if m.extra.counter2 > 0 && !m.fluids.eye_in_water {
            // ConversionTracker.tick: leaving the water ends a started
            // conversion too.
            m.extra.counter = -1;
            m.extra.counter2 = 0;
        } else if m.extra.counter2 > 0 {
            m.extra.counter2 -= 1;
            if m.extra.counter2 == 0 {
                let (kind, sound) = if m.species() == Species::Husk {
                    ("minecraft:zombie", "entity.husk.converted_to_zombie")
                } else {
                    ("minecraft:drowned", "entity.zombie.converted_to_drowned")
                };
                cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
                cx.events.push(MobEvent::SpawnKind {
                    kind: kind.into(),
                    at: [m.position.x, m.position.y, m.position.z],
                    setup: super::SpawnSetup {
                        baby: m.is_baby(),
                        yaw: m.yaw,
                        persistent: m.persistent,
                        ..Default::default()
                    },
                });
                m.removed = true;
            }
        } else if m.fluids.eye_in_water {
            m.extra.counter += 1;
            if m.extra.counter >= 600 {
                m.extra.counter2 = 300;
            }
        } else {
            m.extra.counter = -1;
        }
    }
}
/// `AbstractSkeleton.reassessWeaponGoal`'s goal: the bow goal while holding a
/// bow, otherwise the melee goal (the anonymous `AbstractSkeleton$1`).
#[derive(Clone, Debug)]
pub struct SkeletonWeapon {
    bow: BowAttack,
    melee: MeleeAttack,
    use_bow: bool,
    /// The weapon the goal was last reassessed for.
    assessed: Option<bool>,
    hard_interval: i32,
    normal_interval: i32,
}
impl SkeletonWeapon {
    pub fn new(hard_interval: i32, normal_interval: i32) -> Self {
        Self {
            bow: BowAttack::new(1.0, normal_interval, 15.0),
            melee: MeleeAttack::new(1.2, false),
            use_bow: false,
            assessed: None,
            hard_interval,
            normal_interval,
        }
    }
}
impl Goal for SkeletonWeapon {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn name(&self) -> &'static str {
        if self.use_bow {
            "BowAttack"
        } else {
            "AbstractSkeleton$1"
        }
    }
    fn every_tick(&self) -> bool {
        if self.use_bow {
            self.bow.every_tick()
        } else {
            self.melee.every_tick()
        }
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // The goal registered for the weapon in hand.
        self.use_bow = holding_bow(m);
        // reassessWeaponGoal: a bow gets the hard interval only on Hard; it
        // is picked again when the weapon changes (the first check after
        // spawning or loading stands for finalizeSpawn and
        // readAdditionalSaveData).
        if self.assessed != Some(self.use_bow) {
            self.assessed = Some(self.use_bow);
            self.bow.interval_min = if cx.w.difficulty.hard {
                self.hard_interval
            } else {
                self.normal_interval
            };
        }
        if self.use_bow {
            self.bow.can_use(m, cx)
        } else {
            self.melee.can_use(m, cx)
        }
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // Changing weapons removes the running goal.
        if self.use_bow != holding_bow(m) {
            return false;
        }
        if self.use_bow {
            self.bow.can_continue(m, cx)
        } else {
            self.melee.can_continue(m, cx)
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.use_bow {
            self.bow.start(m, cx)
        } else {
            self.melee.start(m, cx)
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.use_bow {
            self.bow.stop(m, cx)
        } else {
            self.melee.stop(m, cx)
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.use_bow {
            self.bow.tick(m, cx)
        } else {
            self.melee.tick(m, cx)
        }
    }
}
/// RangedBowAttackGoal for skeletons.
#[derive(Clone, Debug)]
pub struct BowAttack {
    speed: f64,
    interval_min: i32,
    radius_sqr: f32,
    attack_time: i32,
    see_time: i32,
    strafe_clockwise: bool,
    strafe_back: bool,
    strafe_time: i32,
}
impl BowAttack {
    pub fn new(speed: f64, interval_min: i32, radius: f32) -> Self {
        Self {
            speed,
            interval_min,
            radius_sqr: radius * radius,
            attack_time: -1,
            see_time: 0,
            strafe_clockwise: false,
            strafe_back: false,
            strafe_time: -1,
        }
    }
}
/// `LivingEntity.isHolding(Items.BOW)`: the bow can sit in either hand.
fn holding_bow(m: &Mob) -> bool {
    m.extra.main_hand.as_deref() == Some("minecraft:bow")
        || m.extra.off_hand.as_deref() == Some("minecraft:bow")
}
impl Goal for BowAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some() && holding_bow(m)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        (self.can_use(m, cx) || !m.ai.nav.is_done()) && holding_bow(m)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = false;
        self.see_time = 0;
        self.attack_time = -1;
        m.extra.using_item = 0;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let d = m.distance_sqr(t.position);
        let sees = m.sees(cx.w, who);
        if sees != (self.see_time > 0) {
            self.see_time = 0;
        }
        if sees {
            self.see_time += 1;
        } else {
            self.see_time -= 1;
        }
        if d <= f64::from(self.radius_sqr) && self.see_time >= 20 {
            m.nav_stop();
            self.strafe_time += 1;
        } else {
            m.nav_move_to_entity(
                cx.w,
                [t.position.x, t.position.y, t.position.z],
                1,
                self.speed,
            );
            self.strafe_time = -1;
        }
        if self.strafe_time >= 20 {
            if m.random.next_float() < 0.3 {
                self.strafe_clockwise = !self.strafe_clockwise;
            }
            if m.random.next_float() < 0.3 {
                self.strafe_back = !self.strafe_back;
            }
            self.strafe_time = 0;
        }
        let eye = [t.position.x, t.eye_y, t.position.z];
        if self.strafe_time > -1 {
            if d > f64::from(self.radius_sqr * 0.75) {
                self.strafe_back = false;
            } else if d < f64::from(self.radius_sqr * 0.25) {
                self.strafe_back = true;
            }
            m.ai.mov.strafe(
                if self.strafe_back { -0.5 } else { 0.5 },
                if self.strafe_clockwise { 0.5 } else { -0.5 },
            );
            m.turn_to(eye, 30.0, 30.0);
        } else {
            m.look_at_speed(eye, 30.0, 30.0);
        }
        if m.extra.using_item > 0 {
            if !sees && self.see_time < -60 {
                m.extra.using_item = 0;
            } else if sees {
                let pull = m.extra.using_item;
                if pull >= 20 {
                    m.extra.using_item = 0;
                    perform_ranged_attack(m, who, cx, bow_power(pull));
                    self.attack_time = self.interval_min;
                }
            }
            if m.extra.using_item > 0 {
                m.extra.using_item += 1;
            }
        } else {
            self.attack_time -= 1;
            if self.attack_time <= 0 && self.see_time >= -60 {
                // mob.startUsingItem(ProjectileUtil.getWeaponHoldingHand):
                // the off hand when the main one doesn't hold the bow.
                m.extra.using_item = 1;
                m.extra.using_off_hand = m.extra.main_hand.as_deref() != Some("minecraft:bow");
            }
        }
    }
}
/// BowItem.getPowerForTime
pub fn bow_power(ticks: i32) -> f32 {
    let t = ticks as f32 / 20.0;
    ((t * t + t * 2.0) / 3.0).min(1.0)
}
/// AbstractSkeleton.performRangedAttack: aim 20% above the target's lower
/// third, speed 1.6, uncertainty 14 - 4 * difficulty.
pub fn perform_ranged_attack(m: &mut Mob, who: Who, cx: &mut Cx, power: f32) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let height = t.bounds.max_position().y - t.bounds.min_position().y;
    let xd = t.position.x - m.position.x;
    let yd = t.position.y + height / 3.0 - start.y;
    let zd = t.position.z - m.position.z;
    let dist = (xd * xd + zd * zd).sqrt();
    // AbstractArrow.setBaseDamageFromMob: power * 2 + triangle(difficulty * 0.11, 0.57425).
    let id = i32::from(cx.w.difficulty.id);
    let mut arrow_random = crate::projectiles::own_random(m);
    let base = f64::from(power) * 2.0 + arrow_random.triangle(f64::from(id) * 0.11, 0.574_25);
    let effect = match m.species() {
        Species::Stray => Some(("minecraft:slowness", 600)),
        Species::Bogged => Some(("minecraft:poison", 100)),
        Species::Parched => Some(("minecraft:weakness", 600)),
        _ => None,
    };
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: base,
            critical: false,
            effect,
        },
        start,
        Some(Who::Mob(m.id)),
    );
    p.set_bonus(super::gear::arrow_bonus(m));
    p.shoot(
        &mut arrow_random,
        [xd, yd + dist * 0.2, zd],
        1.6,
        (14 - id * 4) as f32,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 / (m.random.next_float() * 0.4 + 0.8);
    cx.events.push(MobEvent::Sound(
        "entity.skeleton.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
/// Drowned.DrownedGoToWaterGoal (simplified to "head for water by day").
#[derive(Clone, Debug, Default)]
pub struct DrownedGoToWater {
    wanted: [f64; 3],
}
impl Goal for DrownedGoToWater {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !cx.w.daylight || m.fluids.in_water {
            return false;
        }
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        for _ in 0..10 {
            let p = [
                bx + m.random.next_int_bound(20).unwrap_or(0) - 10,
                by + 2 - m.random.next_int_bound(8).unwrap_or(0),
                bz + m.random.next_int_bound(20).unwrap_or(0) - 10,
            ];
            if cx.w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water {
                self.wanted = [
                    f64::from(p[0]) + 0.5,
                    f64::from(p[1]),
                    f64::from(p[2]) + 0.5,
                ];
                return true;
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, 1.0);
    }
}

/// `Zombie.ZombieAttackTurtleEggGoal` (a `RemoveBlockGoal` on turtle eggs):
/// walk to a turtle egg within 24 blocks and trample it.
#[derive(Clone, Debug)]
pub struct AttackTurtleEgg {
    base: super::goals::MoveToBlock,
    since_reached: i32,
}
impl Default for AttackTurtleEgg {
    fn default() -> Self {
        Self {
            base: super::goals::MoveToBlock::new(1.0, 24, 3),
            since_reached: 0,
        }
    }
}
/// RemoveBlockGoal.isValidTarget: the block with two air blocks above, in a loaded chunk.
fn turtle_egg_target(w: &super::MobWorld, p: [i32; 3]) -> bool {
    w.world.loaded(p[0], p[2])
        && w.block_name(p[0], p[1], p[2]) == "minecraft:turtle_egg"
        && w.facts.is_air(w.world.block(p[0], p[1] + 1, p[2]))
        && w.facts.is_air(w.world.block(p[0], p[1] + 2, p[2]))
}
impl Goal for AttackTurtleEgg {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | super::goals::JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    /// RemoveBlockGoal.canUse
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !cx.w.mob_griefing {
            return false;
        }
        if self.base.next_start > 0 {
            self.base.next_start -= 1;
            return false;
        }
        if self.base.find_nearest(m, cx.w, turtle_egg_target) {
            self.base.next_start = super::goals::reduced(20);
            return true;
        }
        // MoveToBlockGoal.nextStartTick
        self.base.next_start =
            super::goals::reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        false
    }
    fn can_continue(&mut self, _m: &mut Mob, cx: &mut Cx) -> bool {
        self.base.can_continue(cx.w, turtle_egg_target)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.base.start(m, cx.w);
        self.since_reached = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.fall_distance = 1.0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.base.tick_within(m, cx.w, 1.14);
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let egg = |p: [i32; 3]| cx.w.block_name(p[0], p[1], p[2]) == "minecraft:turtle_egg";
        let eat = [
            at,
            [at[0], at[1] - 1, at[2]],
            [at[0] - 1, at[1], at[2]],
            [at[0] + 1, at[1], at[2]],
            [at[0], at[1], at[2] - 1],
            [at[0], at[1], at[2] + 1],
            [at[0], at[1] - 2, at[2]],
        ]
        .into_iter()
        .find(|p| egg(*p));
        let Some(eat) = eat.filter(|_| self.base.reached) else {
            return;
        };
        let center = [
            f64::from(eat[0]) + 0.5,
            f64::from(eat[1]) + 0.7,
            f64::from(eat[2]) + 0.5,
        ];
        if self.since_reached > 0 {
            m.velocity.y = 0.3;
            // The egg crumb particles' spread.
            for _ in 0..3 {
                m.random.next_float();
            }
            cx.events.push(MobEvent::Particles {
                kind: "item_egg",
                at: center,
                count: 3,
            });
        }
        if self.since_reached % 2 == 0 {
            m.velocity.y = -0.3;
            if self.since_reached % 6 == 0 {
                let pitch = 0.9 + m.random.next_float() * 0.2;
                let b = self.base.block;
                cx.events.push(MobEvent::Sound(
                    "entity.zombie.destroy_egg",
                    [
                        f64::from(b[0]) + 0.5,
                        f64::from(b[1]) + 0.5,
                        f64::from(b[2]) + 0.5,
                    ],
                    0.5,
                    pitch,
                ));
            }
        }
        if self.since_reached > 60 {
            cx.events.push(MobEvent::BreakBlock {
                pos: eat,
                drop: false,
            });
            // `sendParticles(POOF, ..., 1, xa, ya, za, 0.15)`, twenty times.
            for _ in 0..20 {
                let spread = [
                    m.random.next_gaussian() * 0.02,
                    m.random.next_gaussian() * 0.02,
                    m.random.next_gaussian() * 0.02,
                ];
                cx.events.push(MobEvent::LevelParticles {
                    kind: "poof",
                    at: [center[0], f64::from(eat[1]), center[2]],
                    spread,
                    speed: 0.15,
                    count: 1,
                });
            }
            // The break sound's pitch draws from the level random.
            cx.events.push(MobEvent::Sound(
                "block.turtle_egg.break",
                [
                    f64::from(eat[0]) + 0.5,
                    f64::from(eat[1]) + 0.5,
                    f64::from(eat[2]) + 0.5,
                ],
                0.7,
                1.0,
            ));
        }
        self.since_reached += 1;
    }
}
