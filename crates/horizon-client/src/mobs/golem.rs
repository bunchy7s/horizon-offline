//! Golems: iron golems (fighting monsters, the heavy swing that throws its
//! target up, cracks as they weaken, iron repairs and poppies for villagers)
//! and snow golems (snowballs at monsters, the snow trail, melting).
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        RandomLookAround, Stroll, goal_clone, reduced,
    },
    villager_brain::{
        closest_section, occupied_pois_in_range, others_tickets, sections_to_village,
        village_sections,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `Enemy` monsters an iron golem goes after (never creepers).
fn golem_prey(s: &super::Seen) -> bool {
    s.hostile && s.species != Species::Creeper
}
/// `IronGolem.registerGoals`, without `ResetUniversalAngerTargetGoal` (that
/// goal is not ported).
pub fn register_iron(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(1, MeleeAttack::new(1.0, true));
    g.add(
        2,
        MoveTowardsTarget {
            speed: 0.9,
            within: 32.0,
        },
    );
    let mut back = Stroll::new(0.6)
        .interval(10)
        .named("MoveBackToVillageGoal")
        .gate(outside_village)
        .position(back_to_village_position);
    back.check_no_action = false;
    g.add(2, back);
    let mut stroll = Stroll::new(0.6)
        .interval(240)
        .named("GolemRandomStrollInVillageGoal")
        .position(village_stroll_position);
    stroll.check_no_action = false;
    g.add(4, stroll);
    g.add(
        5,
        OfferFlower {
            ticks: 0,
            villager: None,
        },
    );
    g.add(7, LookAtPlayer::new(6.0));
    g.add(8, RandomLookAround::default());
    m.ai.targets.add(1, DefendVillage);
    m.ai.targets.add(2, HurtByTarget::new());
    m.ai.targets.add(
        3,
        NearestTarget::player(true)
            .interval(10)
            .when(|m, _| m.extra.anger > 0),
    );
    m.ai.targets.add(
        3,
        NearestTarget::new(Prey::Filter(golem_prey), false).interval(5),
    );
}
/// `SectionPos.of`: the section a position is in.
fn section_of(p: Vec3) -> [i32; 3] {
    [
        (p.x.floor() as i32) >> 4,
        (p.y.floor() as i32) >> 4,
        (p.z.floor() as i32) >> 4,
    ]
}
/// `SectionPos.center` as `Vec3.atBottomCenterOf` reads it.
fn section_center(s: [i32; 3]) -> [f64; 3] {
    [
        f64::from(s[0] * 16 + 8) + 0.5,
        f64::from(s[1] * 16 + 8),
        f64::from(s[2] * 16 + 8) + 0.5,
    ]
}
/// `MoveBackToVillageGoal.canUse`: only a golem outside the village
/// (`ServerLevel.isVillage` at its feet, at most 1 section from a village
/// section) goes back to it.
fn outside_village(m: &Mob, w: &MobWorld) -> bool {
    sections_to_village(&village_sections(m, w), section_of(m.position)) > 1
}
/// `MoveBackToVillageGoal.getPosition`: toward the section within 2 that
/// is closest to a village, a quarter turn either side; none when the
/// golem's own section already is.
fn back_to_village_position(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let villages = village_sections(m, cx.w);
    let here = section_of(m.position);
    let best = closest_section(&villages, here, 2);
    if best == here {
        return None;
    }
    m.default_random_pos_towards(
        cx.w,
        10,
        7,
        section_center(best),
        f64::from(std::f32::consts::FRAC_PI_2),
    )
}
/// `GolemRandomStrollInVillageGoal.getPosition`: now and then anywhere
/// (3 in 10); else toward a villager who wants a golem or toward a spot in
/// a village section (7 in 10 the villager first), and anywhere if neither
/// is found. The picks draw from the level's random.
fn village_stroll_position(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    if cx.lr.next_float() < 0.3 {
        return m.land_random_pos(cx.w, 10, 7);
    }
    let target = if cx.lr.next_float() < 0.7 {
        towards_villager_who_wants_golem(m, cx).or_else(|| towards_poi(m, cx))
    } else {
        towards_poi(m, cx).or_else(|| towards_villager_who_wants_golem(m, cx))
    };
    target.or_else(|| m.land_random_pos(cx.w, 10, 7))
}
/// `GolemRandomStrollInVillageGoal.getPositionTowardsVillagerWhoWantsGolem`:
/// one of the villagers within 32 blocks that want a golem, picked from
/// the level's random.
fn towards_villager_who_wants_golem(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let area = m.bounds().inflate(Vec3::new(32.0, 32.0, 32.0));
    let now = cx.w.game_time;
    let villagers: Vec<Vec3> =
        cx.w.others_near(area)
            .filter(|s| {
                s.species == Species::Villager
                    && s.bounds.intersects(area)
                    && s.villager.as_ref().is_some_and(|v| v.wants_golem(now))
            })
            .map(|s| s.position)
            .collect();
    if villagers.is_empty() {
        return None;
    }
    let at = villagers[cx.lr.next_int_bound(villagers.len() as i32).unwrap_or(0) as usize];
    m.land_random_pos_towards(cx.w, 10, 7, [at.x, at.y, at.z])
}
/// `GolemRandomStrollInVillageGoal.getPositionTowardsPoi`: a taken point of
/// interest within 8 blocks of the centre of a random village section
/// (`getRandomVillageSection`, `getRandomPoiWithinSection`) among those
/// within 2 sections of the golem.
fn towards_poi(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
    let villages = village_sections(m, cx.w);
    let here = section_of(m.position);
    let mut sections = Vec::new();
    for z in here[2] - 2..=here[2] + 2 {
        for y in here[1] - 2..=here[1] + 2 {
            for x in here[0] - 2..=here[0] + 2 {
                if sections_to_village(&villages, [x, y, z]) == 0 {
                    sections.push([x, y, z]);
                }
            }
        }
    }
    if sections.is_empty() {
        return None;
    }
    let s = sections[cx.lr.next_int_bound(sections.len() as i32).unwrap_or(0) as usize];
    let taken = others_tickets(m, cx.w);
    let pois = occupied_pois_in_range(
        cx.w,
        &taken,
        [s[0] * 16 + 8, s[1] * 16 + 8, s[2] * 16 + 8],
        8,
    );
    if pois.is_empty() {
        return None;
    }
    let p = pois[cx.lr.next_int_bound(pois.len() as i32).unwrap_or(0) as usize];
    m.land_random_pos_towards(
        cx.w,
        10,
        7,
        [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ],
    )
}
/// `DefendVillageTargetGoal`: a player that a villager nearby has no
/// good word for (reputation of -100 or worse) is attacked on sight.
#[derive(Clone, Debug)]
struct DefendVillage;
impl Goal for DefendVillage {
    goal_clone!();
    fn flags(&self) -> u8 {
        super::goals::TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = &cx.w.player else { return false };
        if !p.alive || p.creative || p.spectator || m.spares_players(cx.w) {
            return false;
        }
        // The box around the golem: 10 across, 8 up and down.
        let b = m.bounds().inflate(Vec3::new(10.0, 8.0, 10.0));
        if !b.intersects(p.bounds) || m.position.distance_squared(p.position) > 64.0 * 64.0 {
            return false;
        }
        // A position inside `b` puts the box's bottom face there (one block
        // wider, as the box test is strict).
        let near = b.inflate(Vec3::new(1.0, 1.0, 1.0));
        let bad = cx.w.others_near(near).any(|s| {
            s.species == Species::Villager
                && s.alive
                && s.player_rep <= -100
                && b.contains(s.position)
                && s.position.distance_squared(m.position) <= 64.0 * 64.0
        });
        bad && m.sees(cx.w, Who::Player)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let spares = m.spares_players(cx.w);
        m.target_info(cx.w).is_some_and(|t| {
            t.alive
                && !(t.is_player && (t.creative || spares))
                && m.position.distance_squared(t.position) <= 64.0 * 64.0
        })
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.ai.target = Some(Who::Player);
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.ai.target = None;
    }
}
/// `MoveTowardsTargetGoal`: close in on a target within range.
#[derive(Clone, Debug)]
struct MoveTowardsTarget {
    speed: f64,
    within: f64,
}
impl Goal for MoveTowardsTarget {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.ai.target.and_then(|t| cx.w.target_info(t)) else {
            return false;
        };
        if m.distance_sqr(t.position) > self.within * self.within {
            return false;
        }
        m.nav_move_to_entity(
            cx.w,
            [t.position.x, t.position.y, t.position.z],
            0,
            self.speed,
        )
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.ai.nav.is_done()
            && m.ai
                .target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && m.distance_sqr(t.position) < self.within * self.within)
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// `OfferFlowerGoal`: by day, now and then, hold out a poppy to a villager.
#[derive(Clone, Debug)]
struct OfferFlower {
    ticks: i32,
    villager: Option<u32>,
}
impl Goal for OfferFlower {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !cx.w.daylight || m.random.next_int_bound(8000).unwrap_or(1) != 0 {
            return false;
        }
        // The nearest of #candidate_for_iron_golem_gift: villagers and copper golems.
        self.villager =
            cx.w.nearby(m.position, 6.0, |s| {
                matches!(s.species, Species::Villager | Species::CopperGolem) && s.alive
            })
            .map(|s| s.id)
            .next();
        self.villager.is_some()
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        self.ticks > 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        // adjustedTickDelay(400): the goal ticks every other game tick.
        self.ticks = reduced(400);
        m.extra.counter = 400;
        // IronGolem.offerFlower(true): entity event 11.
        entity_event(m, 11, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.counter = 0;
        // IronGolem.offerFlower(false): entity event 34.
        entity_event(m, 34, cx);
        // A copper golem the offer ran out beside takes the poppy on its
        // antenna (#accepts_iron_golem_gift) when it has none.
        if self.ticks == 0
            && let Some(v) = self.villager.and_then(|v| cx.w.seen(v))
            && v.species == Species::CopperGolem
        {
            let b = m.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let (vlo, vhi) = (v.bounds.min_position(), v.bounds.max_position());
            if lo.x - 6.0 < vhi.x
                && hi.x + 6.0 > vlo.x
                && lo.y - 2.0 < vhi.y
                && hi.y + 2.0 > vlo.y
                && lo.z - 6.0 < vhi.z
                && hi.z + 6.0 > vlo.z
            {
                cx.events.push(MobEvent::Gift {
                    target: v.id,
                    item: "minecraft:poppy",
                });
            }
        }
        self.villager = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(v) = self.villager.and_then(|v| cx.w.seen(v)) {
            m.look_at_speed([v.position.x, v.eye_y, v.position.z], 30.0, 30.0);
        }
        self.ticks -= 1;
    }
}
/// `IronGolem.doHurtTarget`: half to one and a half times the damage, and
/// the target is thrown upward.
pub fn iron_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
    entity_event(m, 4, cx);
    let attack = m.attrs.attack_damage;
    let amount = if attack as i32 > 0 {
        attack / 2.0 + m.random.next_int_bound(attack as i32).unwrap_or(0) as f32
    } else {
        attack
    };
    let from = [m.position.x, m.position.y, m.position.z];
    match who {
        Who::Player => cx.events.push(MobEvent::AttackPlayer(amount, from)),
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source: super::Damage::mob(m.id, from),
        }),
    }
    cx.events.push(MobEvent::Launch {
        target: who,
        up: 0.4,
    });
    cx.events.push(MobEvent::Sound(
        "entity.iron_golem.attack",
        m.center(),
        1.0,
        1.0,
    ));
}
/// `Level.broadcastEntityEvent` for an iron golem (guests run `client_event`).
fn entity_event(m: &Mob, event: i8, cx: &mut Cx) {
    cx.events.push(MobEvent::EntityEvent {
        mob: m.id,
        event,
        at: [m.position.x, m.position.y, m.position.z],
    });
}
/// `IronGolem.handleEntityEvent` on a client: 4 the swing, 11 and 34 the
/// poppy offer starting and stopping.
pub fn client_event(m: &mut Mob, event: u8) {
    match event {
        4 => m.extra.anim_ticks = 10,
        11 => m.extra.counter = 400,
        34 => m.extra.counter = 0,
        _ => {}
    }
}
/// The swing and offer clocks of `IronGolem.aiStep` on a client.
pub fn client_tick(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
}
/// `IronGolem.aiStep` timers and anger.
pub fn iron_ai_step(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) {
            m.ai.target = None;
        }
    }
}
/// `Crackiness.GOLEM` texture for the health left.
pub fn crackiness(m: &Mob) -> Option<&'static str> {
    let f = m.health / m.attrs.max_health;
    if f < 0.25 {
        Some("entity/iron_golem/iron_golem_crackiness_high")
    } else if f < 0.5 {
        Some("entity/iron_golem/iron_golem_crackiness_medium")
    } else if f < 0.75 {
        Some("entity/iron_golem/iron_golem_crackiness_low")
    } else {
        None
    }
}
fn triangle_wave(x: f32, period: f32) -> f32 {
    ((x % period) - period * 0.5).abs() / (period * 0.5) * 2.0 - 1.0
}
/// `IronGolemModel.setupAnim`.
pub fn iron_anim(m: &Mob, s: &State, p: &mut Poses) {
    let attack = if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks as f32 - s.partial
    } else {
        0.0
    };
    let (pos, speed) = (s.pos, s.speed);
    if attack > 0.0 {
        p.xrot("right_arm", -2.0 + 1.5 * triangle_wave(attack, 10.0));
        p.xrot("left_arm", -2.0 + 1.5 * triangle_wave(attack, 10.0));
    } else if m.extra.counter > 0 {
        p.xrot(
            "right_arm",
            -0.8 + 0.025 * triangle_wave(m.extra.counter as f32, 70.0),
        );
        p.xrot("left_arm", 0.0);
    } else {
        p.xrot("right_arm", (-0.2 + 1.5 * triangle_wave(pos, 13.0)) * speed);
        p.xrot("left_arm", (-0.2 - 1.5 * triangle_wave(pos, 13.0)) * speed);
    }
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    p.xrot("right_leg", -1.5 * triangle_wave(pos, 13.0) * speed);
    p.xrot("left_leg", 1.5 * triangle_wave(pos, 13.0) * speed);
    p.yrot("right_leg", 0.0);
    p.yrot("left_leg", 0.0);
}

pub fn register_snow(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(
        1,
        super::witch::RangedAttack::new(1.25, 20, 20, 10.0, snowball_attack),
    );
    g.add(2, Stroll::avoiding_water(1.0));
    g.add(3, LookAtPlayer::new(6.0));
    g.add(4, RandomLookAround::default());
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Filter(|s| s.hostile), true).interval(10),
    );
}
/// `SnowGolem.performRangedAttack`.
fn snowball_attack(m: &mut Mob, who: Who, _power: f32, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let xd = t.position.x - m.position.x;
    let yd = t.eye_y - 1.1;
    let zd = t.position.z - m.position.z;
    let yo = (xd * xd + zd * zd).sqrt() * 0.2;
    let mut p = Projectile::new(ProjectileKind::Snowball, start, Some(Who::Mob(m.id)));
    // SnowGolem.rangedAttackUncertainty: 12 on every difficulty.
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [xd, yd + yo - start.y, zd],
        1.6,
        12.0,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 0.4 / (m.random.next_float() * 0.4 + 0.8);
    cx.events.push(MobEvent::Sound(
        "entity.snow_golem.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
/// `SnowGolem.aiStep`: melting in hot biomes, the snow trail.
pub fn snow_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    // EnvironmentAttributes.SNOW_GOLEM_MELTS: the hot biomes and the Nether.
    if w.nether || w.temperature(x, y, z) > 1.0 {
        events.push(MobEvent::HurtSelf {
            mob: m.id,
            kind: super::DamageKind::OnFire,
            amount: 1.0,
        });
    }
    if !w.mob_griefing {
        return;
    }
    for i in 0..4 {
        let xx = (m.position.x + f64::from((i % 2 * 2 - 1) as f32 * 0.25)).floor() as i32;
        let zz = (m.position.z + f64::from((i / 2 % 2 * 2 - 1) as f32 * 0.25)).floor() as i32;
        let here = w.facts.get(w.world.block(xx, y, zz));
        let below = w.facts.get(w.world.block(xx, y - 1, zz));
        // SnowLayerBlock.canSurvive: a full top face, not ice.
        let ice = matches!(
            below.name.as_ref(),
            "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier"
        );
        if here.name.as_ref() == "minecraft:air" && below.full_collision && !ice {
            events.push(MobEvent::SetBlockName {
                pos: [xx, y, zz],
                name: "minecraft:snow",
                effect: false,
                flags: horizon_world::level::flags::UPDATE_ALL,
            });
        }
    }
    let _ = z;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use std::sync::Arc;

    fn mob(id: u32, species: Species, at: Vec3) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        Mob::new(id, Arc::new(kind), at, 0.0)
    }

    /// `IronGolem.canAttack` refuses creepers, and `HurtByTargetGoal.canUse`
    /// asks it: the creeper that hurt the golem is never fought back, while
    /// a zombie that hurts it is.
    #[test]
    fn a_creeper_that_hurts_an_iron_golem_is_never_fought_back() {
        let creeper = mob(2, Species::Creeper, Vec3::new(1.5, 64.0, 0.5));
        let zombie = mob(3, Species::Zombie, Vec3::new(1.5, 64.0, 0.5));
        with_test_world(&[creeper.seen(None), zombie.seen(None)], |w, _| {
            let mut golem = mob(1, Species::IronGolem, Vec3::new(0.5, 64.0, 0.5));
            let mut goal = HurtByTarget::new();
            let mut events = Vec::new();
            let mut lr = horizon_core::random::LegacyRandom::new(0);
            let mut cx = Cx {
                w,
                lr: &mut lr,
                events: &mut events,
            };

            golem.ai.last_hurt_by = Some(Who::Mob(2));
            golem.ai.last_hurt_by_time = 1;
            assert!(!goal.can_use(&mut golem, &mut cx));
            assert_eq!(golem.ai.target, None, "a creeper is refused");

            golem.ai.last_hurt_by = Some(Who::Mob(3));
            golem.ai.last_hurt_by_time = 2;
            assert!(goal.can_use(&mut golem, &mut cx));
            goal.start(&mut golem, &mut cx);
            assert_eq!(golem.ai.target, Some(Who::Mob(3)));
        });
    }

    /// A village golem (not built by a player) still goes after the player
    /// who angered it.
    #[test]
    fn a_village_golem_still_targets_the_player_who_angered_it() {
        with_test_world(&[], |w, _| {
            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(3.5, 64.0, 0.5)));
            let mut golem = mob(1, Species::IronGolem, Vec3::new(0.5, 64.0, 0.5));
            golem.extra.anger = 100;
            let mut events = Vec::new();
            let mut lr = horizon_core::random::LegacyRandom::new(0);
            let mut cx = Cx {
                w: &w,
                lr: &mut lr,
                events: &mut events,
            };
            let mut goal = NearestTarget::player(true)
                .interval(0)
                .when(|m, _| m.extra.anger > 0);
            assert!(goal.can_use(&mut golem, &mut cx));
            goal.start(&mut golem, &mut cx);
            assert_eq!(golem.ai.target, Some(Who::Player));
        });
    }
}

#[cfg(test)]
mod created_golem_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use std::sync::Arc;

    /// The target `m`'s target goals pick over `ticks` ticks in `w`.
    fn target(m: &mut Mob, w: &MobWorld<'_>, ticks: u32) -> Option<Who> {
        let mut events = Vec::new();
        let mut lr = horizon_core::random::LegacyRandom::new(0);
        let mut goals = std::mem::take(&mut m.ai.targets);
        {
            let mut cx = Cx {
                w,
                lr: &mut lr,
                events: &mut events,
            };
            for _ in 0..ticks {
                goals.tick(m, &mut cx);
            }
        }
        m.ai.targets = goals;
        m.ai.target
    }
    /// A player-built and a village iron golem, both angry at a player, and a
    /// zombie; `None` without the extracted vanilla data.
    fn angry_golems() -> Option<(Mob, Mob, crate::mobs::Seen)> {
        let mut out = None;
        with_test_world(&[], |_, types| {
            let golem = |id: u32, created: bool| {
                let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
                let mut m = Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                // `CarvedPumpkinBlock.trySpawnGolem`: setPlayerCreated(true).
                m.extra.player_created = created;
                // The anger a player's hit gives it (`after_hurt`).
                m.extra.anger = 400;
                m
            };
            let zombie = Mob::new(
                3,
                Arc::new(types.types["minecraft:zombie"].clone()),
                Vec3::new(0.5, 64.0, 2.5),
                0.0,
            );
            out = Some((golem(1, true), golem(2, false), zombie.seen(None)));
        });
        out
    }

    /// `IronGolem.canAttack`: a golem a player built never takes a player as
    /// its target, though a village golem does and both still go after
    /// monsters. The world holds one player, so "its builder" and "a player"
    /// are the same check, as they are in vanilla.
    #[test]
    fn a_golem_a_player_built_never_targets_one() {
        let Some((mut built, mut village, zombie)) = angry_golems() else {
            return;
        };
        with_test_world(&[], |w, _| {
            let mut w = w.clone();
            w.player = Some(test_player(Vec3::new(2.5, 64.0, 0.5)));
            assert_eq!(
                target(&mut built, &w, 200),
                None,
                "a golem a player built turned on its builder"
            );
            assert_eq!(
                target(&mut village, &w, 200),
                Some(Who::Player),
                "a village golem goes after the player"
            );
        });
        with_test_world(&[zombie], |w, _| {
            assert_eq!(
                target(&mut built, w, 200),
                Some(Who::Mob(3)),
                "a golem a player built still fights monsters"
            );
        });
    }
}

#[cfg(test)]
mod crack_sound_tests {
    use super::*;
    use crate::mobs::Damage;
    use std::sync::Arc;

    /// An iron golem of its 100 health, from a made-up type.
    fn golem() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:iron_golem".into();
        k.species = Species::IronGolem;
        k.max_health = 100.0;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }
    /// One hit of `amount` past the golem's hurt frames: how often it played
    /// `entity.iron_golem.damage`.
    fn cracks(m: &mut Mob, amount: f32) -> usize {
        m.outbox.clear();
        m.invulnerable = 0;
        assert!(m.damage(amount, Damage::player([0.0, 0.0, 1.0])));
        m.outbox
            .iter()
            .filter(|e| matches!(e, MobEvent::Sound("entity.iron_golem.damage", ..)))
            .count()
    }

    /// `IronGolem.hurtServer`: a hit that leaves the golem in a new
    /// `Crackiness.Level` (`Crackiness.GOLEM` is 0.75, 0.5, 0.25) plays
    /// `entity.iron_golem.damage` once, and hits inside one level play none.
    #[test]
    fn a_crack_is_heard_once_at_the_threshold() {
        let mut m = golem();
        assert_eq!(cracks(&mut m, 30.0), 1, "100 to 70 crosses 0.75");
        assert_eq!(cracks(&mut m, 10.0), 0, "70 to 60 stays at the same crack");
        assert_eq!(cracks(&mut m, 20.0), 1, "60 to 40 crosses 0.5");
        assert_eq!(cracks(&mut m, 10.0), 0, "40 to 30 stays at the same crack");
        assert_eq!(cracks(&mut m, 10.0), 1, "30 to 20 crosses 0.25");
    }
}
