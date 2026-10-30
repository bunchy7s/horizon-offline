//! Witches (potion drinking and splash potion attacks) and the shared
//! `RangedAttackGoal` (also used by snow golems).
use super::{
    Mob, MobEvent, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, NearestTarget, RandomLookAround,
        Stroll, goal_clone,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::trig::Mth;
use horizon_core::{geometry::Vec3, random::RandomSource};

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(2, RangedAttack::new(1.0, 60, 60, 10.0, witch_attack));
    g.add(2, Stroll::avoiding_water(1.0));
    g.add(3, LookAtPlayer::new(8.0));
    g.add(3, RandomLookAround::default());
    m.ai.targets.add(1, HurtByTarget::new());
    m.ai.targets.add(2, HealRaiders::default());
    // NearestAttackableWitchTargetGoal: only while not healing raiders.
    m.ai.targets.add(
        3,
        NearestTarget::player(true)
            .interval(10)
            .gate(|m, _| m.extra.counter2 <= 0)
            .named("NearestAttackableWitchTargetGoal"),
    );
}
/// `NearestHealableRaiderTargetGoal`: in a raid, target another raider to
/// heal, then wait 10 seconds (the cooldown lives in `extra.counter2`).
#[derive(Clone, Debug)]
struct HealRaiders(NearestTarget);
impl Default for HealRaiders {
    fn default() -> Self {
        Self(
            NearestTarget::new(
                super::goals::Prey::Filter(|s| {
                    super::raider::is_raider(s.species) && s.species != Species::Witch
                }),
                true,
            )
            .interval(0),
        )
    }
}
impl Goal for HealRaiders {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.counter2 > 0 || !m.random.next_boolean() {
            return false;
        }
        if !super::raider::has_active_raid(m) {
            return false;
        }
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.counter2 = super::goals::reduced(200);
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
/// The callback a ranged mob uses to fire (`RangedAttackMob.performRangedAttack`).
pub type Fire = fn(&mut Mob, Who, f32, &mut Cx);

/// RangedAttackGoal(speed, intervalMin, intervalMax, radius).
#[derive(Clone, Debug)]
pub struct RangedAttack {
    speed: f64,
    min: i32,
    max: i32,
    radius: f32,
    fire: Fire,
    attack_time: i32,
    see_time: i32,
}
impl RangedAttack {
    pub fn new(speed: f64, min: i32, max: i32, radius: f32, fire: Fire) -> Self {
        Self {
            speed,
            min,
            max,
            radius,
            fire,
            attack_time: -1,
            see_time: 0,
        }
    }
}
impl Goal for RangedAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx) || m.target_info(cx.w).is_some_and(|t| t.alive) && !m.ai.nav.is_done()
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.see_time = 0;
        self.attack_time = -1;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let d2 = m.distance_sqr(t.position);
        let sees = m.sees(cx.w, who);
        if sees {
            self.see_time += 1;
        } else {
            self.see_time = 0;
        }
        if d2 <= f64::from(self.radius * self.radius) && self.see_time >= 5 {
            m.nav_stop();
        } else {
            m.nav_move_to_entity(
                cx.w,
                [t.position.x, t.position.y, t.position.z],
                1,
                self.speed,
            );
        }
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = (d2.sqrt() as f32) / self.radius;
            let power = dist.clamp(0.1, 1.0);
            (self.fire)(m, who, power, cx);
            self.attack_time =
                (dist * (self.max - self.min) as f32 + self.min as f32).floor() as i32;
        } else if self.attack_time < 0 {
            let f = d2.sqrt() / f64::from(self.radius);
            self.attack_time =
                (f64::from(self.min) + f * f64::from(self.max - self.min)).floor() as i32;
        }
    }
}
/// Witch.performRangedAttack: harming by default, slowness at range, poison
/// on healthy targets, weakness up close.
fn witch_attack(m: &mut Mob, who: Who, _power: f32, cx: &mut Cx) {
    // isDrinkingPotion
    if m.extra.flag {
        return;
    }
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let motion = match who {
        Who::Player => cx.w.player.as_ref().map_or(Vec3::ZERO, |p| p.velocity),
        Who::Mob(id) => cx.w.seen(id).map_or(Vec3::ZERO, |s| s.velocity),
    };
    let xd = t.position.x + motion.x - m.position.x;
    let yd = t.eye_y - f64::from(1.1_f32) - m.position.y;
    let zd = t.position.z + motion.z - m.position.z;
    let dist = (xd * xd + zd * zd).sqrt();
    let health = if t.is_player {
        20.0
    } else {
        cx.w.seen(match who {
            Who::Mob(id) => id,
            Who::Player => 0,
        })
        .map_or(20.0, |s| s.health)
    };
    // A raider target is healed instead: Healing when nearly dead, else
    // Regeneration, and the witch drops the target afterwards.
    let heals = match who {
        Who::Mob(id) => {
            cx.w.seen(id)
                .is_some_and(|s| super::raider::is_raider(s.species))
        }
        Who::Player => false,
    };
    // `target.hasEffect`: a target that already has the effect gets the next one.
    let has = |effect: &str| match who {
        Who::Player => cx.w.player.as_ref().is_some_and(|p| p.has_effect(effect)),
        Who::Mob(id) => cx.w.seen(id).is_some_and(|s| s.has_effect(effect)),
    };
    let (effect, ticks, amplifier) = if heals {
        m.ai.target = None;
        if health <= 4.0 {
            ("minecraft:instant_health", 1, 0)
        } else {
            ("minecraft:regeneration", 900, 0)
        }
    } else if dist >= 8.0 && !has("slowness") {
        ("minecraft:slowness", 1800, 0)
    } else if health >= 8.0 && !has("poison") {
        ("minecraft:poison", 900, 0)
    } else if dist <= 3.0 && !has("weakness") && m.random.next_float() < 0.25 {
        ("minecraft:weakness", 1800, 0)
    } else {
        ("minecraft:instant_damage", 1, 0)
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let mut p = Projectile::new(
        ProjectileKind::Potion {
            effect,
            ticks,
            amplifier,
            color: 0,
        },
        start,
        Some(Who::Mob(m.id)),
    );
    // Splash potions throw with gravity 0.05.
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [xd, yd + dist * 0.2, zd],
        if dist <= 2.0 { 0.45 } else { 0.75 },
        8.0,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 0.8 + m.random.next_float() * 0.4;
    cx.events.push(MobEvent::Sound(
        "entity.witch.throw",
        m.center(),
        1.0,
        pitch,
    ));
}
/// The movement speed of a witch with `SPEED_MODIFIER_DRINKING` (-0.25,
/// `ADD_VALUE`) while it drinks: it stands still for the 32 ticks.
pub fn speed(m: &Mob, base: f64) -> f64 {
    if m.species() == Species::Witch && m.extra.flag {
        (base - 0.25).max(0.0)
    } else {
        base
    }
}
/// Witch.aiStep before `super.aiStep()`: the healing cooldown, then drink
/// water breathing, fire resistance, healing or swiftness.
pub fn ai_step(m: &mut Mob, cx_events: &mut Vec<MobEvent>, target_far: bool) {
    if !m.alive() {
        return;
    }
    // healRaidersGoal.decrementCooldown (the attack goal's canAttack reads it).
    m.extra.counter2 -= 1;
    drink(m, cx_events, target_far);
    // The angry-witch particles (entity event 15).
    let _ = m.random.next_float();
}
fn drink(m: &mut Mob, cx_events: &mut Vec<MobEvent>, target_far: bool) {
    if m.extra.flag {
        // `usingTime-- <= 0`
        m.extra.using_item -= 1;
        if m.extra.using_item < 0 {
            m.extra.using_item = 0;
            if let Some(potion) = m.extra.take_main_hand() {
                match potion.as_ref() {
                    "minecraft:water_breathing" => {
                        m.add_effect("minecraft:water_breathing", 3600, 0)
                    }
                    "minecraft:fire_resistance" => {
                        m.add_effect("minecraft:fire_resistance", 3600, 0)
                    }
                    "minecraft:healing" => m.health = (m.health + 4.0).min(m.attrs.max_health),
                    "minecraft:swiftness" => m.add_effect("minecraft:speed", 3600, 0),
                    _ => {}
                }
            }
            m.extra.flag = false;
        }
        return;
    }
    let potion = if m.random.next_float() < 0.15
        && m.fluids.eye_in_water
        && !m.has_effect("minecraft:water_breathing")
    {
        Some("minecraft:water_breathing")
    } else if m.random.next_float() < 0.15
        && (m.fire_ticks > 0 || m.last_damage.is_some_and(|(d, _)| d.is_fire()))
        && !m.has_effect("minecraft:fire_resistance")
    {
        Some("minecraft:fire_resistance")
    } else if m.random.next_float() < 0.05 && m.health < m.attrs.max_health {
        Some("minecraft:healing")
    } else if m.random.next_float() < 0.5
        && m.ai.target.is_some()
        && !m.has_effect("minecraft:speed")
        && target_far
    {
        Some("minecraft:swiftness")
    } else {
        None
    };
    if let Some(p) = potion {
        m.extra.set_main_hand(Some(p.into()));
        // PotionItem use duration 32 ticks.
        m.extra.using_item = 32;
        m.extra.flag = true;
        let pitch = 0.8 + m.random.next_float() * 0.4;
        cx_events.push(MobEvent::Sound(
            "entity.witch.drink",
            m.center(),
            1.0,
            pitch,
        ));
    }
}
/// WitchModel.setupAnim (VillagerModel base with a wobbling nose).
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    p.xrot("right_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed * 0.5);
    p.xrot(
        "left_leg",
        (s.pos * 0.6662 + std::f32::consts::PI).mcos() * 1.4 * s.speed * 0.5,
    );
    let speed = 0.01 * (m.id % 10) as f32;
    let deg = horizon_core::geometry::trig::DEG_TO_RAD;
    p.xrot("nose", (s.age * speed).msin() * 4.5 * deg);
    p.zrot("nose", (s.age * speed).mcos() * 2.5 * deg);
    if m.extra.main_hand.is_some() {
        let n = p.get("nose");
        n[0] = 0.0;
        n[1] = 1.0;
        n[2] = -1.5;
        n[3] = -0.9;
    }
    let _ = Species::Witch;
}
