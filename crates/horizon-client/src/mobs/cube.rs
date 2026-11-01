//! Slimes and magma cubes (`AbstractCubeMob`): sized bodies, hopping
//! movement (`CubeMobMoveControl`), random directions, contact damage,
//! squish animation and splitting.
use super::{
    Damage, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    control::{MoveKind, Operation, rotlerp},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, NearestTarget, Prey, goal_clone, reduced},
};
use horizon_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};

// AnimState slots.
const SQUISH: usize = 0;
const TARGET_SQUISH: usize = 1;

pub fn register(m: &mut Mob) {
    if m.species() == Species::SulfurCube {
        return super::sulfur_cube::register(m);
    }
    m.ai.goals.add(1, CubeFloat);
    m.ai.goals.add(2, CubeAttack { tired: 0 });
    m.ai.goals.add(
        4,
        CubeRandomDirection {
            degrees: 0.0,
            next: 0,
        },
    );
    m.ai.goals.add(5, CubeKeepJumping);
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Player, true)
            .interval(10)
            .when(|m, w| {
                w.player
                    .as_ref()
                    .is_some_and(|p| (p.position.y - m.position.y).abs() <= 4.0)
            }),
    );
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::IronGolem], true));
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.can_float = true;
    m.extra.vector[0] = f64::from(180.0 * m.yaw / std::f32::consts::PI);
    set_size(m, 1);
}
/// AbstractCubeMob.setSize(size, true): the max health (size^2), movement
/// speed (0.2 + 0.1 * size, in floats) and attack damage (size) bases, a
/// magma cube's armor (size * 3), then the health back to the max.
pub fn set_size(m: &mut Mob, size: i32) {
    use super::finalize::{Attr, set_base};
    let size = size.clamp(1, 127);
    m.extra.size = size;
    set_base(m, Attr::MaxHealth, f64::from(size * size));
    set_base(
        m,
        Attr::MovementSpeed,
        f64::from(0.2f32 + 0.1f32 * size as f32),
    );
    set_base(m, Attr::AttackDamage, f64::from(size));
    if m.species() == super::Species::MagmaCube {
        set_base(m, Attr::Armor, f64::from(size * 3));
    }
    m.health = super::finalize::max_health(m);
}
/// AbstractCubeMob.setSpawnSize: 1, 2 or 4 (normal difficulty special multiplier 0).
pub fn finalize(m: &mut Mob) {
    let scale = m.random.next_int_bound(3).unwrap_or(0);
    set_size(m, 1 << scale);
}
fn tiny(m: &Mob) -> bool {
    m.extra.size <= 1
}
fn can_deal_damage(m: &Mob) -> bool {
    // SulfurCube.canDealDamage: never.
    m.species() == Species::MagmaCube || !tiny(m) && m.species() != Species::SulfurCube
}
fn jump_delay(m: &mut Mob) -> i32 {
    let base = m.random.next_int_bound(20).unwrap_or(0) + 10;
    if m.species() == Species::MagmaCube {
        base * 4
    } else {
        base
    }
}
fn sound_pitch(m: &mut Mob) -> f32 {
    let adjust = if tiny(m) { 1.4 } else { 0.8 };
    ((m.random.next_float() - m.random.next_float()) * 0.2 + 1.0) * adjust
}
/// CubeMobMoveControl.tick
pub fn move_control(m: &mut Mob, events: &mut Vec<MobEvent>) {
    // SulfurCubeMobMoveControl: still while holding a block.
    if m.species() == Species::SulfurCube && super::sulfur_cube::has_body(m) {
        return;
    }
    let want = m.extra.vector[0] as f32;
    m.yaw = rotlerp(m.yaw, want, 90.0);
    m.head_yaw = m.yaw;
    m.body_yaw = m.yaw;
    if m.ai.mov.operation != Operation::MoveTo {
        m.ai.zza = 0.0;
        return;
    }
    m.ai.mov.operation = Operation::Wait;
    let speed = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
    if m.on_ground {
        m.set_speed(speed);
        let delay = m.extra.counter;
        m.extra.counter -= 1;
        if delay <= 0 {
            m.extra.counter = jump_delay(m);
            if m.extra.flag {
                m.extra.counter /= 3;
            }
            m.ai.jump = true;
            let jump = match (m.species(), tiny(m)) {
                (Species::MagmaCube, _) => "entity.magma_cube.jump",
                (Species::SulfurCube, _) => super::sulfur_cube::jump_sound(m),
                (_, true) => "entity.slime.jump_small",
                _ => "entity.slime.jump",
            };
            let pitch = sound_pitch(m);
            let vol = super::ai::sound_volume(m);
            events.push(MobEvent::Sound(jump, m.center(), vol, pitch));
        } else {
            m.ai.xxa = 0.0;
            m.ai.zza = 0.0;
            m.set_speed(0.0);
        }
    } else {
        m.set_speed(speed);
    }
}
pub(super) fn set_direction(m: &mut Mob, y_rot: f32, aggressive: bool) {
    m.extra.vector[0] = f64::from(y_rot);
    m.extra.flag = aggressive;
}
pub(super) fn set_wanted(m: &mut Mob, speed: f64) {
    m.ai.mov.speed_modifier = speed;
    m.ai.mov.operation = Operation::MoveTo;
}
/// AbstractCubeMob.jumpFromGround (and MagmaCube's size boost).
pub fn jump(m: &mut Mob) {
    let boost = if m.species() == Species::MagmaCube {
        m.extra.size as f32 * 0.1
    } else {
        0.0
    };
    m.velocity.y = f64::from(m.jump_power() + boost);
}
/// `AbstractCubeMob.tick`'s landing splash: crumbs of goo (or flames) around
/// the cube's feet, drawn by the side that ticks the mob.
fn landing_splash(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let size = m.width() * 2.0;
    let radius = size / 2.0;
    let kind = match m.species() {
        Species::MagmaCube => "flame",
        Species::SulfurCube => "sulfur_cube_goo",
        _ => "item_slime",
    };
    let mut i = 0;
    while (i as f32) < size * 16.0 {
        let dir = m.random.next_float() * std::f64::consts::TAU as f32;
        let d = m.random.next_float() * 0.5 + 0.5;
        let xd = trig::sin(f64::from(dir)) * radius * d;
        let zd = trig::cos(f64::from(dir)) * radius * d;
        events.push(MobEvent::Particles {
            kind,
            at: [
                m.position.x + f64::from(xd),
                m.position.y,
                m.position.z + f64::from(zd),
            ],
            count: 0,
        });
        i += 1;
    }
}
/// `AbstractCubeMob.tick` on a client of a server: the landing splash follows
/// the on-ground flag the server sends.
pub fn client_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.species() == Species::SulfurCube {
        super::sulfur_cube::client_tick(m);
    }
    if m.on_ground && !m.extra.flag2 {
        landing_splash(m, events);
    }
    m.extra.flag2 = m.on_ground;
}
/// AbstractCubeMob.tick squish and landing sounds, plus contact damage.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let v = &mut m.anim.v;
    v[SQUISH] += (v[TARGET_SQUISH] - v[SQUISH]) * 0.5;
    let was = m.extra.flag2;
    if m.on_ground && !was {
        landing_splash(m, events);
        let squish = match (m.species(), tiny(m)) {
            (Species::SulfurCube, _) => super::sulfur_cube::squish_sound(m),
            (Species::MagmaCube, true) => "entity.magma_cube.squish_small",
            (Species::MagmaCube, false) => "entity.magma_cube.squish",
            (_, true) => "entity.slime.squish_small",
            _ => "entity.slime.squish",
        };
        let pitch = ((m.random.next_float() - m.random.next_float()) * 0.2 + 1.0) / 0.8;
        let vol = super::ai::sound_volume(m);
        events.push(MobEvent::Sound(squish, m.center(), vol, pitch));
        m.anim.v[TARGET_SQUISH] = -0.5;
    } else if !m.on_ground && was {
        m.anim.v[TARGET_SQUISH] = 1.0;
    }
    m.extra.flag2 = m.on_ground;
    m.anim.v[TARGET_SQUISH] *= if m.species() == Species::MagmaCube {
        0.9
    } else {
        0.6
    };
    // playerTouch -> dealDamage
    if can_deal_damage(m)
        && !m.no_ai
        && m.alive()
        && let Some(p) = &w.player
        && p.alive
        && !p.creative
        && crate::scoreboard::teams_allow_damage(
            w.teams.of(Who::Mob(m.id)),
            w.teams.of(Who::Player),
        )
        // Player.touch: entities within the player's box grown by (1, 0.5, 1).
        && m.bounds().intersects(p.bounds.inflate(Vec3::new(1.0, 0.5, 1.0)))
    {
        let reach = 2.04f32.sqrt() as f64 - 0.6;
        if m.bounds()
            .inflate(Vec3::new(reach, 0.0, reach))
            .intersects(p.bounds)
            && m.sees(w, Who::Player)
        {
            events.push(MobEvent::CubeHurtPlayer {
                cube: m.id,
                amount: attack_damage(m),
                from: [m.position.x, m.position.y, m.position.z],
            });
        }
    }
    hurt_touching_golems(m, w, events);
}
/// `AbstractCubeMob.getAttackDamage`: the attack damage attribute, plus 2 for
/// a magma cube (`MagmaCube.getAttackDamage`).
fn attack_damage(m: &Mob) -> f32 {
    m.attrs.attack_damage
        + if m.species() == Species::MagmaCube {
            2.0
        } else {
            0.0
        }
}
/// `AbstractCubeMob.push(entity)`: an iron golem whose `pushEntities` takes
/// this cube (its box overlaps the cube's, `EntitySelector.pushableBy`) gets
/// `dealDamage`: alive, `doTeamsAllowDamage`, `isWithinMeleeAttackRange`
/// (always true for overlapping boxes) and `hasLineOfSight`, then a
/// `mobAttack` hit for `getAttackDamage`.
fn hurt_touching_golems(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // canDealDamage also needs Mob.isEffectiveAi: a NoAI cube never hurts.
    if !can_deal_damage(m) || m.no_ai || !m.alive() {
        return;
    }
    let bounds = m.bounds();
    let own = w.teams.of(Who::Mob(m.id));
    let pushable = m.pushable();
    let golems: Vec<u32> = w
        .others_near(bounds)
        .filter(|o| {
            let theirs = w.teams.of(Who::Mob(o.id));
            o.id != m.id
                && o.species == Species::IronGolem
                && o.alive
                && o.bounds.intersects(bounds)
                && crate::scoreboard::pushable_by(theirs, own, pushable)
                && crate::scoreboard::teams_allow_damage(own, theirs)
        })
        .map(|o| o.id)
        .collect();
    let from = [m.position.x, m.position.y, m.position.z];
    for golem in golems {
        if m.sees(w, Who::Mob(golem)) {
            events.push(MobEvent::CubeHurtMob {
                cube: m.id,
                target: golem,
                amount: attack_damage(m),
                source: Damage::mob(m.id, from),
            });
        }
    }
}
/// MagmaCubeModel.setupAnim (slimes have no animation).
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    if m.species() == Species::MagmaCube {
        let squish = m.anim.lerp(SQUISH, s.partial).max(0.0);
        for i in 0..8 {
            p.get(&format!("cube{i}"))[1] = -((4 - i) as f32) * squish * 1.7;
        }
    }
}
/// AbstractCubeMobRenderer.scale: size with squash and stretch.
pub fn scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    let size = m.extra.size.max(1) as f32;
    let squish = m.anim.lerp(SQUISH, alpha) / (size * 0.5 + 1.0);
    let w = 1.0 / (squish + 1.0);
    let mut t = glam::Mat4::IDENTITY;
    if m.species() == Species::Slime {
        t = glam::Mat4::from_scale(glam::Vec3::splat(0.999))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.001, 0.0));
    }
    t * glam::Mat4::from_scale(glam::Vec3::new(w * size, 1.0 / w * size, w * size))
}
/// CubeMobFloatGoal
#[derive(Clone, Debug)]
pub(super) struct CubeFloat;
impl Goal for CubeFloat {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.is_in_liquid()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.random.next_float() < 0.8 {
            m.ai.jump = true;
        }
        set_wanted(m, 1.2);
    }
}
/// CubeMobAttackGoal: face the target and hop at it for up to 300 ticks.
#[derive(Clone, Debug)]
struct CubeAttack {
    tired: i32,
}
impl Goal for CubeAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w)
            .is_some_and(|t| t.alive && !(t.is_player && t.creative))
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.tired = reduced(300);
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m
            .target_info(cx.w)
            .is_some_and(|t| t.alive && !(t.is_player && t.creative))
        {
            return false;
        }
        self.tired -= 1;
        self.tired > 0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = m.target_info(cx.w) {
            m.turn_to([t.position.x, t.eye_y, t.position.z], 10.0, 10.0);
        }
        let (yaw, aggressive) = (m.yaw, can_deal_damage(m));
        set_direction(m, yaw, aggressive);
    }
}
/// CubeMobRandomDirectionGoal
#[derive(Clone, Debug, Default)]
pub(super) struct CubeRandomDirection {
    degrees: f32,
    next: i32,
}
impl Goal for CubeRandomDirection {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_none()
            && (m.on_ground || m.is_in_liquid() || m.has_effect("minecraft:levitation"))
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        self.next -= 1;
        if self.next <= 0 {
            self.next = reduced(40 + m.random.next_int_bound(60).unwrap_or(0));
            self.degrees = m.random.next_int_bound(360).unwrap_or(0) as f32;
        }
        set_direction(m, self.degrees, false);
    }
}
/// CubeMobKeepOnJumpingGoal
#[derive(Clone, Debug)]
pub(super) struct CubeKeepJumping;
impl Goal for CubeKeepJumping {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        set_wanted(m, 1.0);
    }
}

#[cfg(test)]
mod golem_touch_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// The golems a cube of this kind and size, `x` blocks east of an iron
    /// golem, hurts in one `ai_step`, and the damage.
    fn hits(name: &str, size: i32, x: f64) -> Option<Vec<(u32, f32)>> {
        hits_with(name, size, x, false)
    }
    fn hits_with(name: &str, size: i32, x: f64, no_ai: bool) -> Option<Vec<(u32, f32)>> {
        let mut out = None;
        with_test_world(&[], |_, types| {
            let mob = |id: u32, name: &str, at: Vec3| {
                Mob::new(id, Arc::new(types.types[name].clone()), at, 0.0)
            };
            let golem = mob(2, "minecraft:iron_golem", Vec3::new(0.5, 64.0, 0.5));
            let mut cube = mob(1, name, Vec3::new(0.5 + x, 64.0, 0.5));
            set_size(&mut cube, size);
            cube.no_ai = no_ai;
            out = Some((cube, golem.seen(None)));
        });
        let (mut cube, golem) = out?;
        let mut events = Vec::new();
        with_test_world(&[golem], |w, _| ai_step(&mut cube, w, &mut events));
        Some(
            events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::CubeHurtMob {
                        cube: 1,
                        target,
                        amount,
                        source,
                    } => {
                        assert_eq!(source.attacker, Some(Who::Mob(1)));
                        Some((*target, *amount))
                    }
                    _ => None,
                })
                .collect(),
        )
    }

    /// `AbstractCubeMob.push`: a slime or magma cube the iron golem bumps
    /// into hurts it like a touching player (#3513), for `getAttackDamage`;
    /// a tiny slime (`canDealDamage`) and a cube clear of the golem do not.
    #[test]
    fn a_cube_touching_an_iron_golem_hurts_it() {
        let Some(slime) = hits("minecraft:slime", 2, 1.0) else {
            return;
        };
        assert_eq!(slime, vec![(2, 2.0)], "a size 2 slime hits the golem for 2");
        let magma = hits("minecraft:magma_cube", 4, 1.5).unwrap();
        assert_eq!(magma, vec![(2, 6.0)], "a size 4 magma cube hits for 4 + 2");
        let tiny = hits("minecraft:magma_cube", 1, 0.9).unwrap();
        assert_eq!(
            tiny,
            vec![(2, 3.0)],
            "a tiny magma cube still hits for 1 + 2"
        );
        assert!(
            hits("minecraft:slime", 1, 0.9).unwrap().is_empty(),
            "a tiny slime never hurts"
        );
        assert!(
            hits("minecraft:slime", 2, 3.0).unwrap().is_empty(),
            "the golem is out of touch"
        );
        assert!(
            hits_with("minecraft:slime", 2, 1.0, true)
                .unwrap()
                .is_empty(),
            "a NoAI cube is not isEffectiveAi, so it cannot deal damage"
        );
    }

    /// #4569: `AbstractCubeMob.dealDamage` on a touching player emits
    /// `CubeHurtPlayer` without queueing `entity.slime.attack` in `ai_step`
    /// (the sound only follows a hit that `hurtServer` accepts).
    #[test]
    fn a_cube_touching_a_player_defers_its_attack_sound_to_hurt_server() {
        use horizon_core::geometry::Aabb;
        let mut events = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            let mut w = w.clone();
            let at = Vec3::new(0.5, 64.0, 0.5);
            w.player = Some(crate::mobs::PlayerView {
                position: at,
                bounds: Aabb::new(
                    Vec3::new(at.x - 0.3, at.y, at.z - 0.3),
                    Vec3::new(at.x + 0.3, at.y + 1.8, at.z + 0.3),
                ),
                eye: [at.x, at.y + 1.62, at.z],
                alive: true,
                creative: false,
                sneaking: false,
                swimming: false,
                look: [0.0, 0.0, 1.0],
                main_hand: None,
                off_hand: None,
                head: None,
                armor_cover: 0.0,
                invisible: false,
                spectator: false,
                gold_armor: false,
                hurt_by: None,
                last_hurt: None,
                velocity: Vec3::ZERO,
                yaw: 0.0,
                sprinting: false,
                riding: false,
                hero: false,
                effects: Vec::new(),
                uuid: [0; 4],
                allies: Default::default(),
                remote: false,
                sleeping: false,
                sleep_timer: 0,
                passenger: false,
                vehicle: None,
            });
            let mut slime = Mob::new(
                1,
                Arc::new(types.types["minecraft:slime"].clone()),
                Vec3::new(1.2, 64.0, 0.5),
                0.0,
            );
            set_size(&mut slime, 2);
            slime.on_ground = true;
            slime.extra.flag2 = true;
            ai_step(&mut slime, &w, &mut events);
        });
        if !ran {
            return;
        }
        assert!(
            events.iter().any(|e| matches!(
                e,
                MobEvent::CubeHurtPlayer {
                    cube: 1,
                    amount,
                    ..
                } if (*amount - 2.0).abs() < f32::EPSILON
            )),
            "expected CubeHurtPlayer for size-2 slime, got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, MobEvent::Sound("entity.slime.attack", ..))),
            "ai_step must not emit entity.slime.attack before hurtServer: {events:?}"
        );
    }
}
