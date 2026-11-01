//! Nether flyers: blazes (hovering, three-fireball volleys, the spinning
//! rods) and ghasts (floating about, charging and shooting large fireballs).
use super::{
    Mob, MobEvent, MobWorld, Who,
    anim::{Poses, State},
    control::{MoveKind, Operation},
    goals::{
        Cx, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, NearestTarget, RandomLookAround, Stroll,
        goal_clone,
    },
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::trig::Mth;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::f32::consts::PI;

/// `RandomSource.triangle(mode, deviation)`.
fn triangle(m: &mut Mob, mode: f64, deviation: f64) -> f64 {
    mode + deviation * (m.random.next_double() - m.random.next_double())
}

pub fn register_blaze(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(4, BlazeAttack::default());
    g.add(7, Stroll::avoiding_water(1.0));
    g.add(8, LookAtPlayer::new(8.0));
    g.add(8, RandomLookAround::default());
    m.ai.targets.add(1, HurtByTarget::new().alert());
    m.ai.targets.add(2, NearestTarget::player(true));
}
/// `Blaze.BlazeAttackGoal`: melee up close, otherwise a 3-second charge and
/// three small fireballs, then a five-second rest.
#[derive(Clone, Debug, Default)]
struct BlazeAttack {
    step: i32,
    time: i32,
    last_seen: i32,
}
impl Goal for BlazeAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target
            .and_then(|t| cx.w.target_info(t))
            .is_some_and(|t| t.alive && !(t.is_player && t.creative))
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.step = 0;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = false;
        self.last_seen = 0;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.time -= 1;
        let Some(who) = m.ai.target else {
            return;
        };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let sees = m.sees(cx.w, who);
        if sees {
            self.last_seen = 0;
        } else {
            self.last_seen += 1;
        }
        let d2 = m.distance_sqr(t.position);
        let follow = m.attrs.follow_range;
        let wanted = [t.position.x, t.position.y, t.position.z];
        if d2 < 4.0 {
            if !sees {
                return;
            }
            if self.time <= 0 {
                self.time = 20;
                super::ai::do_hurt_target(m, who, cx);
            }
            m.ai.mov.set_wanted(wanted[0], wanted[1], wanted[2], 1.0);
        } else if d2 < follow * follow && sees {
            let half = f64::from(m.height()) * 0.5;
            let xd = t.position.x - m.position.x;
            let yd = (t.bounds.min_position().y + t.bounds.max_position().y) * 0.5
                - (m.position.y + half);
            let zd = t.position.z - m.position.z;
            if self.time <= 0 {
                self.step += 1;
                if self.step == 1 {
                    self.time = 60;
                    m.extra.flag = true;
                } else if self.step <= 4 {
                    self.time = 6;
                } else {
                    self.time = 100;
                    self.step = 0;
                    m.extra.flag = false;
                }
                if self.step > 1 {
                    let sqd = d2.sqrt().sqrt() * 0.5;
                    cx.events.push(MobEvent::Sound(
                        "entity.blaze.shoot",
                        m.center(),
                        1.0,
                        (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0,
                    ));
                    let dir = [
                        triangle(m, xd, 2.297 * sqd),
                        yd,
                        triangle(m, zd, 2.297 * sqd),
                    ];
                    let start = Vec3::new(m.position.x, m.position.y + half + 0.5, m.position.z);
                    cx.events.push(MobEvent::Projectile(Projectile::fireball(
                        ProjectileKind::SmallFireball,
                        start,
                        Some(Who::Mob(m.id)),
                        dir,
                    )));
                }
            }
            m.look_at_speed([t.position.x, t.eye_y, t.position.z], 10.0, 10.0);
        } else if self.last_seen < 5 {
            m.ai.mov.set_wanted(wanted[0], wanted[1], wanted[2], 1.0);
        }
    }
}
/// `Blaze.aiStep`: slow falling, the burning sound and smoke.
pub fn blaze_ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !m.on_ground && m.velocity.y < 0.0 {
        m.velocity.y *= 0.6;
    }
    if m.random.next_int_bound(24).unwrap_or(1) == 0 {
        let volume = 1.0 + m.random.next_float();
        let pitch = m.random.next_float() * 0.7 + 0.3;
        events.push(MobEvent::Sound(
            "entity.blaze.burn",
            m.center(),
            volume,
            pitch,
        ));
    }
    // getRandomX(0.5), getRandomY(), getRandomZ(0.5), at rest.
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    for _ in 0..2 {
        let at = [
            m.position.x + w * (2.0 * m.random.next_double() - 1.0) * 0.5,
            m.position.y + h * m.random.next_double(),
            m.position.z + w * (2.0 * m.random.next_double() - 1.0) * 0.5,
        ];
        events.push(MobEvent::Particles {
            kind: "large_smoke",
            at,
            count: 0,
        });
    }
}
/// `Blaze.customServerAiStep`: rise toward a target above the allowed offset.
pub fn blaze_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    m.extra.counter -= 1;
    if m.extra.counter <= 0 {
        m.extra.counter = 100;
        m.extra.vector[0] = triangle(m, 0.5, 6.891);
    }
    if let Some(t) = m.ai.target.and_then(|t| cx.w.target_info(t))
        && t.alive
        && !(t.is_player && t.creative)
        && t.eye_y > m.eye_y() + m.extra.vector[0]
    {
        m.velocity.y += (0.3 - m.velocity.y) * 0.3;
    }
}
/// `BlazeModel.setupAnim`: three rings of rods circling and bobbing.
pub fn blaze_anim(s: &State, p: &mut Poses) {
    let age = s.age;
    let mut angle = age * PI * -0.1;
    for i in 0..4 {
        let part = p.get(&format!("part{i}"));
        part[1] = -2.0 + ((i * 2) as f32 + age).mul_add(0.25, 0.0).mcos();
        part[0] = angle.mcos() * 9.0;
        part[2] = angle.msin() * 9.0;
        angle += PI / 2.0;
    }
    angle = PI / 4.0 + age * PI * 0.03;
    for i in 4..8 {
        let part = p.get(&format!("part{i}"));
        part[1] = 2.0 + (((i * 2) as f32 + age) * 0.25).mcos();
        part[0] = angle.mcos() * 7.0;
        part[2] = angle.msin() * 7.0;
        angle += PI / 2.0;
    }
    angle = 0.471_238_94 + age * PI * -0.05;
    for i in 8..12 {
        let part = p.get(&format!("part{i}"));
        part[1] = 11.0 + ((i as f32 * 1.5 + age) * 0.5).mcos();
        part[0] = angle.mcos() * 5.0;
        part[2] = angle.msin() * 5.0;
        angle += PI / 2.0;
    }
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
}

pub fn register_ghast(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Custom;
    let g = &mut m.ai.goals;
    g.add(5, RandomFloatAround);
    g.add(7, GhastLook);
    g.add(7, GhastShoot { charge: 0 });
    m.ai.targets.add(
        1,
        NearestTarget::player(true).when(|m, w| {
            w.player
                .as_ref()
                .is_some_and(|p| (p.position.y - m.position.y).abs() <= 4.0)
        }),
    );
}
/// `Ghast.GhastMoveControl`: a push toward the wanted point every few ticks.
pub fn ghast_move_control(m: &mut Mob, w: &MobWorld) {
    if m.ai.mov.operation != Operation::MoveTo {
        return;
    }
    // `floatDuration-- <= 0`: the test is on the value before the decrement.
    let before = m.extra.counter2;
    m.extra.counter2 -= 1;
    if before > 0 {
        return;
    }
    m.extra.counter2 += m.random.next_int_bound(5).unwrap_or(0) + 2;
    let [wx, wy, wz] = m.ai.mov.wanted;
    let travel = Vec3::new(wx - m.position.x, wy - m.position.y, wz - m.position.z);
    let len = travel.length();
    if len < 1.0e-6 {
        return;
    }
    // canReach: the swept box stays clear along the way.
    let b = m.bounds();
    let steps = len.ceil() as i32;
    let clear = (1..=steps).all(|i| {
        let f = f64::from(i) / f64::from(steps);
        let d = Vec3::new(travel.x * f, travel.y * f, travel.z * f);
        w.no_collision(Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        ))
    });
    if clear {
        // FLYING_SPEED 0.06 * 5 / 3.
        let speed = 0.06 * 5.0 / 3.0;
        m.velocity = m.velocity.add_vector(Vec3::new(
            travel.x / len * speed,
            travel.y / len * speed,
            travel.z / len * speed,
        ));
    } else {
        m.ai.mov.operation = Operation::Wait;
    }
}
/// `(180.0F / (float)Math.PI)` of `GhastLookGoal.tick`.
fn ghast_deg() -> f32 {
    180.0_f32 / std::f32::consts::PI
}
/// `Ghast.RandomFloatAroundGoal`.
#[derive(Clone, Debug)]
struct RandomFloatAround;
impl Goal for RandomFloatAround {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        if !m.ai.mov.has_wanted() {
            return true;
        }
        let [x, y, z] = m.ai.mov.wanted;
        let d =
            (x - m.position.x).powi(2) + (y - m.position.y).powi(2) + (z - m.position.z).powi(2);
        !(1.0..=3600.0).contains(&d)
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let c = m.position;
        let mut r = || f64::from(m.random.next_float() * 2.0 - 1.0) * 16.0;
        // `getSuitableFlyToPosition` with distanceToBlocks 0: the first random
        // position is the target (the mirror under the heightmap only follows
        // 64 rejected tries, which a ghast without a home never has).
        let t = [c.x + r(), c.y + r(), c.z + r()];
        m.ai.mov.set_wanted(t[0], t[1], t[2], 1.0);
    }
}
/// `Ghast.GhastLookGoal`: face the motion, or the target within 64 blocks.
#[derive(Clone, Debug)]
struct GhastLook;
impl Goal for GhastLook {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        match m.ai.target.and_then(|t| cx.w.target_info(t)) {
            None => {
                let v = m.velocity;
                m.yaw = -(horizon_core::geometry::trig::atan2(v.x, v.z) as f32) * ghast_deg();
                m.body_yaw = m.yaw;
            }
            Some(t) => {
                if m.distance_sqr(t.position) < 4096.0 {
                    let (dx, dz) = (t.position.x - m.position.x, t.position.z - m.position.z);
                    m.yaw = -(horizon_core::geometry::trig::atan2(dx, dz) as f32) * ghast_deg();
                    m.body_yaw = m.yaw;
                }
            }
        }
    }
}
/// `Ghast.GhastShootFireballGoal`: a second of charging, then a fireball.
#[derive(Clone, Debug)]
struct GhastShoot {
    charge: i32,
}
impl Goal for GhastShoot {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some()
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.charge = 0;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = false;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else {
            return;
        };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        if m.distance_sqr(t.position) < 4096.0 && m.sees(cx.w, who) {
            self.charge += 1;
            if self.charge == 10 {
                // LevelEvent 1015: the warning cry, its pitch the client's.
                if !m.silent {
                    cx.events.push(MobEvent::LevelEventSound {
                        sound: "entity.ghast.warn",
                        at: m.center(),
                        volume: 10.0,
                        spread: 0.2,
                    });
                }
            }
            if self.charge == 20 {
                let yaw = m.yaw.to_radians();
                let view = [-f64::from(yaw.msin()), f64::from(yaw.mcos())];
                let half = f64::from(m.height()) * 0.5;
                let ty = (t.bounds.min_position().y + t.bounds.max_position().y) * 0.5;
                let dir = [
                    t.position.x - (m.position.x + view[0] * 4.0),
                    ty - (0.5 + m.position.y + half),
                    t.position.z - (m.position.z + view[1] * 4.0),
                ];
                // LevelEvent 1016: the shot, its pitch the client's.
                if !m.silent {
                    cx.events.push(MobEvent::LevelEventSound {
                        sound: "entity.ghast.shoot",
                        at: m.center(),
                        volume: 10.0,
                        spread: 0.2,
                    });
                }
                let start = Vec3::new(
                    m.position.x + view[0] * 4.0,
                    m.position.y + half + 0.5,
                    m.position.z + view[1] * 4.0,
                );
                cx.events.push(MobEvent::Projectile(Projectile::fireball(
                    ProjectileKind::LargeFireball { power: 1 },
                    start,
                    Some(Who::Mob(m.id)),
                    dir,
                )));
                self.charge = -40;
            }
        } else if self.charge > 0 {
            self.charge -= 1;
        }
        m.extra.flag = self.charge > 10;
    }
}
/// `GhastModel.animateTentacles`.
pub fn ghast_anim(s: &State, p: &mut Poses) {
    for i in 0..9 {
        let name = format!("tentacle{i}");
        if p.has(&name) {
            p.xrot(&name, 0.2 * (s.age * 0.3 + i as f32).msin() + 0.4);
        }
    }
}
