//! Rabbit hopping controls and the clocks consumed by `RabbitModel`.
use super::{
    Mob, MobEvent, MobWorld, Species,
    control::Operation,
    goals::{
        Avoid, AvoidEntity, Breed, ClimbOnTopOfPowderSnow, Cx, Float, Goal, HurtByTarget, JUMP,
        LookAtPlayer, MOVE, MeleeAttack, MoveToBlock, NearestTarget, Panic, Stroll, Tempt,
        goal_clone, reduced,
    },
};
use horizon_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};

// Rabbit's transient state in Extra (none of these clocks are saved).
const JUMP_TICKS: usize = 0;
const JUMP_DURATION: usize = 1;
const MOVE_DELAY: usize = 2;
const IDLE_TIMEOUT: usize = 3;
// Client counters start only when the actual ground jump emits entity event 1.
const CLIENT_JUMP_TICKS: usize = 4;
const CLIENT_JUMP_DURATION: usize = 5;

/// The killer bunny's variant id (`Rabbit.Variant.EVIL`).
const EVIL: u8 = 99;

/// Rabbit.registerGoals.
pub fn register(m: &mut Mob) {
    let food = m.kind.food.clone();
    let monsters = super::species::ALL
        .iter()
        .map(|d| d.species)
        .filter(|s| super::ai::is_monster(*s))
        .collect::<Vec<Species>>();
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(1, ClimbOnTopOfPowderSnow);
    g.add(1, Panic::new(2.2));
    g.add(2, Breed::new(0.8));
    g.add(3, Tempt::new(1.0, food, false));
    g.add(4, RabbitAvoid::new(Avoid::Player, 8.0));
    g.add(
        4,
        RabbitAvoid::new(Avoid::Species(vec![Species::Wolf]), 10.0),
    );
    g.add(4, RabbitAvoid::new(Avoid::Species(monsters), 4.0));
    g.add(5, RaidGarden::new());
    g.add(6, Stroll::avoiding_water(0.6));
    g.add(11, LookAtPlayer::new(10.0));
    m.ai.mov.kind = super::control::MoveKind::Custom;
    m.ai.nav.can_float = true;
    m.extra.ints[IDLE_TIMEOUT] = m.random.next_int_bound(40).unwrap_or(0) + 180;
}

/// `Rabbit.RaidGardenGoal.isValidTarget`: farmland-like ground (`#supports_crops`) under a
/// carrot crop at its maximum age, while the rabbit wants more food and has not found one.
fn raid_target(w: &MobWorld, p: [i32; 3], wants_to_raid: bool, can_raid: bool) -> bool {
    w.has_tag("minecraft:supports_crops", w.block_name(p[0], p[1], p[2]))
        && wants_to_raid
        && !can_raid
        && w.block_name(p[0], p[1] + 1, p[2]) == "minecraft:carrots"
        && w.prop(p[0], p[1] + 1, p[2], "age")
            .and_then(|a| a.parse::<i32>().ok())
            .is_some_and(|age| age >= CARROT_MAX_AGE)
}
/// `CarrotBlock.MAX_AGE`.
const CARROT_MAX_AGE: i32 = 7;

/// `Rabbit.RaidGardenGoal`: a `MoveToBlockGoal` (0.7, 16) to ripe carrots that eats one age
/// of them, then keeps the rabbit full for 40 ticks (`moreCarrotTicks`).
#[derive(Clone, Debug)]
struct RaidGarden {
    block: MoveToBlock,
    wants_to_raid: bool,
    can_raid: bool,
}
impl RaidGarden {
    fn new() -> Self {
        Self {
            block: MoveToBlock::new(0.7, 16, 1),
            wants_to_raid: false,
            can_raid: false,
        }
    }
}
impl Goal for RaidGarden {
    goal_clone!();
    fn name(&self) -> &'static str {
        "RaidGardenGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // MoveToBlockGoal.canUse draws the next delay only once the last one ran out.
        if self.block.next_start > 0 {
            self.block.next_start -= 1;
            return false;
        }
        if !cx.w.mob_griefing {
            return false;
        }
        self.can_raid = false;
        self.wants_to_raid = m.extra.more_carrot_ticks <= 0;
        let next = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        let wants = self.wants_to_raid;
        let found = std::cell::Cell::new(false);
        let target = self.block.can_use(m, cx.w, next, |w, p| {
            let ok = raid_target(w, p, wants, false);
            found.set(found.get() || ok);
            ok
        });
        self.can_raid = found.get();
        target
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        let (wants, raid) = (self.wants_to_raid, self.can_raid);
        self.can_raid
            && self
                .block
                .can_continue(cx.w, |w, p| raid_target(w, p, wants, raid))
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.block.start(m, cx.w);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.block.tick(m, cx.w);
        let b = self.block.block;
        m.ai.look.set_look_at(
            [
                f64::from(b[0]) + 0.5,
                f64::from(b[1] + 1),
                f64::from(b[2]) + 0.5,
            ],
            10.0,
            m.max_head_x_rot(),
        );
        if !self.block.reached {
            return;
        }
        let crop = [b[0], b[1] + 1, b[2]];
        if self.can_raid && cx.w.block_name(crop[0], crop[1], crop[2]) == "minecraft:carrots" {
            let age =
                cx.w.prop(crop[0], crop[1], crop[2], "age")
                    .and_then(|a| a.parse::<i32>().ok())
                    .unwrap_or(0);
            let old = cx.w.world.block(crop[0], crop[1], crop[2]);
            if age == 0 {
                // `setBlock(AIR, 2)`, then `destroyBlock` finds air and does nothing.
                if let Some(air) = cx.w.registry.default_state("minecraft:air") {
                    cx.events.push(MobEvent::CropRaided {
                        pos: crop,
                        state: air,
                        old: None,
                        mob: m.id,
                    });
                }
            } else if let Some(mut s) = cx.w.registry.state(old).cloned() {
                s.properties.insert("age".into(), (age - 1).to_string());
                if let Some(state) = cx.w.registry.id(&s) {
                    cx.events.push(MobEvent::CropRaided {
                        pos: crop,
                        state,
                        old: Some(old),
                        mob: m.id,
                    });
                }
            }
            m.extra.more_carrot_ticks = 40;
        }
        self.can_raid = false;
        self.block.next_start = 10;
    }
}

/// `Rabbit.RabbitAvoidEntityGoal(rabbit, class, maxDist, 2.2, 2.2)`: the
/// killer bunny flees nothing.
#[derive(Clone, Debug)]
struct RabbitAvoid(AvoidEntity);
impl RabbitAvoid {
    fn new(what: Avoid, max_dist: f32) -> Self {
        Self(AvoidEntity::new(what, max_dist, 2.2, 2.2))
    }
}
impl Goal for RabbitAvoid {
    goal_clone!();
    fn name(&self) -> &'static str {
        "RabbitAvoidEntityGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.variant != EVIL && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}

/// Rabbit.baseTick: the idle timeout branch precedes the hop clock branch.
pub fn base_tick(m: &mut Mob) {
    let age = m.age as f32;
    let leashed = m.extra.leash.is_some();
    if m.extra.ints[IDLE_TIMEOUT] <= 0 && !leashed && !m.no_ai {
        m.extra.ints[IDLE_TIMEOUT] = m.random.next_int_bound(40).unwrap_or(0) + 180;
        m.anim.states[1].start(age);
    } else if m.extra.ints[CLIENT_JUMP_TICKS] > 0 {
        m.anim.states[0].start_if_stopped(age);
        m.anim.states[1].stop();
    } else {
        m.extra.ints[IDLE_TIMEOUT] -= 1;
        m.anim.states[0].stop();
    }
    if leashed {
        m.anim.states[1].stop();
    }
}

/// `Rabbit.facePoint`.
fn face_point(m: &mut Mob, x: f64, z: f64) {
    m.yaw = trig::atan2_degrees(z - m.position.z, x - m.position.x) - 90.0;
}

/// `Rabbit.setVariant(EVIL)`, run the first time a killer bunny ticks (its
/// variant comes from `/summon`, a save or a spawn): armor 8, the bite and
/// target goals, attack damage plus 5 and the "Killer Bunny" name.
fn set_evil(m: &mut Mob) {
    use super::finalize::{Attr, Op, add_modifier, set_base};
    if m.ai.goals.has("MeleeAttack") {
        return;
    }
    set_base(m, Attr::Armor, 8.0);
    m.ai.goals.add(4, MeleeAttack::new(1.4, true));
    m.ai.targets.add(1, HurtByTarget::new().alert());
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets
        .add(2, NearestTarget::species(&[Species::Wolf], true));
    add_modifier(m, Attr::AttackDamage, "minecraft:evil", 5.0, Op::AddValue);
    if m.custom_name.is_none() && m.custom_name_text.is_none() {
        let name = crate::text::Text::translate("entity.minecraft.killer_bunny", &[]);
        m.custom_name = Some(name.plain());
        m.custom_name_text = Some(name);
    }
}

fn start_jumping(m: &mut Mob) {
    m.ai.jumping = true;
    m.extra.ints[JUMP_DURATION] = 15;
    m.extra.ints[JUMP_TICKS] = 0;
    let pitch = ((m.random.next_float() - m.random.next_float()) * 0.2 + 1.0) * 0.8;
    if !m.silent {
        m.outbox.push(MobEvent::Sound(
            "entity.rabbit.jump",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            pitch,
        ));
    }
}

/// Rabbit.customServerAiStep: start a hop toward the current path waypoint;
/// `target` is where the rabbit's target stands, if it has one.
pub fn server_ai_step(m: &mut Mob, target: Option<Vec3>) {
    if m.extra.variant == EVIL {
        set_evil(m);
    }
    // RabbitMoveControl.setWantedPosition applies the water override before landing checks.
    if m.fluids.in_water && m.ai.mov.speed_modifier > 0.0 {
        m.ai.mov.speed_modifier = 1.5;
    }
    if m.extra.ints[MOVE_DELAY] > 0 {
        m.extra.ints[MOVE_DELAY] -= 1;
    }
    // `moreCarrotTicks` runs down by up to 2 a tick.
    if m.extra.more_carrot_ticks > 0 {
        m.extra.more_carrot_ticks =
            (m.extra.more_carrot_ticks - m.random.next_int_bound(3).unwrap_or(0)).max(0);
    }
    if m.on_ground {
        if !m.extra.flag {
            m.ai.jumping = false;
            m.extra.ints[MOVE_DELAY] = if m.ai.mov.speed_modifier < 2.2 { 10 } else { 3 };
        }
        // A killer bunny within 4 blocks of its target faces it and hops at it.
        if m.extra.variant == EVIL
            && m.extra.ints[MOVE_DELAY] == 0
            && let Some(t) = target
            && m.distance_sqr(t) < 16.0
        {
            face_point(m, t.x, t.z);
            let speed = m.ai.mov.speed_modifier;
            m.ai.mov.set_wanted(t.x, t.y, t.z, speed);
            start_jumping(m);
        }
        if !m.ai.jump && m.ai.mov.has_wanted() && m.extra.ints[MOVE_DELAY] == 0 {
            let mut at = m.ai.mov.wanted;
            if let Some(path) = m.ai.nav.path.as_ref().filter(|p| !p.is_done()) {
                let p = path.entity_pos_at(path.next, m.width());
                at = [p.x, p.y, p.z];
            }
            face_point(m, at[0], at[2]);
            start_jumping(m);
        }
    }
    m.extra.flag = m.on_ground;
}

/// RabbitMoveControl.tick: wait on the ground and restore the last wanted speed in flight.
pub fn move_control(m: &mut Mob, w: &MobWorld) {
    prepare_move(m);
    m.ground_move_tick(w, m.movement_speed());
}
fn prepare_move(m: &mut Mob) {
    if m.ai.mov.speed_modifier > 0.0 {
        m.extra.vector[0] = if m.fluids.in_water {
            1.5
        } else {
            m.ai.mov.speed_modifier
        };
    }
    if m.on_ground && !m.ai.jumping && !m.ai.jump {
        m.ai.mov.speed_modifier = 0.0;
        m.set_speed(0.0);
    } else if m.ai.mov.has_wanted() || m.ai.mov.operation == Operation::Jumping {
        m.ai.mov.speed_modifier = if m.fluids.in_water {
            1.5
        } else {
            m.extra.vector[0]
        };
    }
}

/// RabbitJumpControl.tick: explicit jump requests do not wait for the landing cooldown.
pub fn jump_control(m: &mut Mob) {
    if m.ai.jump {
        start_jumping(m);
        m.ai.jump = false;
    }
}

/// Rabbit.aiStep runs after the living physics; the fifteenth tick still renders the hop.
pub fn post_tick(m: &mut Mob) {
    for (ticks, duration) in [
        (JUMP_TICKS, JUMP_DURATION),
        (CLIENT_JUMP_TICKS, CLIENT_JUMP_DURATION),
    ] {
        if m.extra.ints[ticks] != m.extra.ints[duration] {
            m.extra.ints[ticks] += 1;
        } else if m.extra.ints[duration] != 0 {
            m.extra.ints[ticks] = 0;
            m.extra.ints[duration] = 0;
            if ticks == JUMP_TICKS {
                m.ai.jumping = false;
            }
        }
    }
}

fn jump_scale(m: &Mob) -> f32 {
    let step =
        m.ai.nav
            .path
            .as_ref()
            .filter(|p| !p.is_done())
            .is_some_and(|p| f64::from(p.next_node().y) > m.position.y + 0.5);
    let high =
        m.horizontal_collision || step || m.ai.jumping && m.ai.mov.wanted[1] > m.position.y + 0.5;
    let base = if high {
        0.5
    } else if m.ai.mov.speed_modifier <= 0.6 {
        0.2
    } else {
        0.3
    };
    base / 0.42
}

/// Rabbit.jumpFromGround: ordinary small hops, larger hops toward raised terrain.
pub fn jump(m: &mut Mob, w: &MobWorld) {
    let power = jump_power(m, block_jump_factor(m, w));
    apply_jump(m, power);
}
fn jump_power(m: &Mob, block_factor: f32) -> f32 {
    let boost = m
        .extra
        .effects
        .iter()
        .find(|e| e.0 == "minecraft:jump_boost")
        .map_or(0.0, |e| 0.1 * (e.2 + 1) as f32);
    m.attrs.jump_strength as f32 * jump_scale(m) * block_factor + boost
}
fn apply_jump(m: &mut Mob, power: f32) {
    if power > 1.0e-5 {
        m.velocity.y = f64::from(power).max(m.velocity.y);
    }
    if m.ai.mov.speed_modifier > 0.0
        && m.velocity.x * m.velocity.x + m.velocity.z * m.velocity.z < 0.01
    {
        m.move_relative(
            0.1,
            Vec3::new(0.0, if m.is_baby() { 0.5 } else { 1.5 }, 1.0),
        );
    }
    hop_event(m);
    m.outbox.push(MobEvent::RabbitHop {
        mob: m.id,
        at: [m.position.x, m.position.y, m.position.z],
    });
}

/// Rabbit.handleEntityEvent(1): reset the counter without starting the clock early.
pub fn hop_event(m: &mut Mob) {
    m.extra.ints[CLIENT_JUMP_DURATION] = 15;
    m.extra.ints[CLIENT_JUMP_TICKS] = 0;
}

fn block_jump_factor(m: &Mob, w: &MobWorld) -> f32 {
    let properties = w.movement;
    let current = w.world.block(
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let below = m.on_pos(0.500_001);
    // shortcut: use mobs' existing on_pos until they track supporting blocks like the player.
    let lower = w.world.block(below[0], below[1], below[2]);
    let current = properties
        .get(current)
        .expect("validated movement registry")
        .coefficients
        .jump_factor;
    let lower = properties
        .get(lower)
        .expect("validated movement registry")
        .coefficients
        .jump_factor;
    if current == 1.0 { lower } else { current }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{
        Anim, Brain, Flags, MobType, Part, Species,
        anim::{self, Poses},
        body_anim,
        keyframes::Animation,
    };
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    fn rabbit() -> Mob {
        let model = Arc::new(Part::parse(
            "root",
            &json!({"children": {"head": {}, "left_hind_foot": {}}}),
        ));
        let kind = Arc::new(MobType {
            id: "minecraft:rabbit".into(),
            species: Species::Rabbit,
            width: 0.4,
            height: 0.5,
            eye_height: 0.4,
            category: "creature".into(),
            max_health: 3.0,
            speed: 0.3,
            flying_speed: 0.4,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 120,
            flags: Flags {
                animal: true,
                ..Default::default()
            },
            model,
            layers: HashMap::new(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Rabbit,
            sounds: [""; 3],
            step_sound: None,
            food: Arc::default(),
            tempt: Arc::default(),
        });
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1.0e-6, "{a} != {b}");
    }

    #[test]
    fn summoned_age_selects_the_baby_model_and_texture() {
        let mut template = rabbit();
        let kind = Arc::make_mut(&mut template.kind);
        kind.texture = "entity/rabbit/rabbit_brown".into();
        kind.layers
            .insert("rabbit_baby#main".into(), kind.model.clone());
        let types = HashMap::from([(kind.id.clone(), template.kind.clone())]);
        for (age, baby) in [(-24000, true), (-1, true), (0, false), (6000, false)] {
            let (tag, _) = horizon_world::nbt::parse_snbt(&format!(
                "{{id:\"minecraft:rabbit\",Pos:[0.0d,0.0d,0.0d],Age:{age}}}"
            ))
            .unwrap();
            let m = crate::entity_nbt::mob_from_nbt(&tag, &types, 2, None).unwrap();
            assert_eq!(m.extra.age, age);
            assert_eq!(m.is_baby(), baby);
            assert_eq!(
                crate::mobs::render::body(&m),
                if baby {
                    (
                        "rabbit_baby#main".into(),
                        "entity/rabbit/rabbit_brown_baby".into(),
                    )
                } else {
                    ("rabbit#main".into(), "entity/rabbit/rabbit_brown".into())
                }
            );
        }
    }

    #[test]
    fn registers_vanillas_goals_with_the_three_avoidances() {
        // Rabbit.registerGoals: players 8, wolves 10, monsters 4 (#2780).
        let mut m = rabbit();
        register(&mut m);
        for name in [
            "Float",
            "Panic",
            "Breed",
            "Tempt",
            "RabbitAvoidEntityGoal",
            "Stroll",
            "LookAtPlayer",
        ] {
            assert!(m.ai.goals.has(name), "{name}");
        }
        assert!(!m.ai.goals.has("RandomLookAround"));
    }

    #[test]
    fn love_ticks_survive_a_save() {
        // Animal.addAdditionalSaveData / readAdditionalSaveData: "InLove" (#1396).
        let mut m = rabbit();
        m.extra.in_love = 570;
        let tag = crate::entity_nbt::mob_to_nbt(&m, [0, 0, 0, 1], None, None);
        assert_eq!(tag.get("InLove"), Some(&horizon_world::nbt::Tag::Int(570)));
        let types = HashMap::from([(m.kind.id.clone(), m.kind.clone())]);
        let back = crate::entity_nbt::mob_from_nbt(&tag, &types, 2, None).unwrap();
        assert_eq!(back.extra.in_love, 570);
    }

    #[test]
    fn landing_waits_then_real_hop_clock_moves_the_model() {
        let mut m = rabbit();
        m.on_ground = true;
        m.ai.mov.set_wanted(0.0, 0.0, 5.0, 0.6);
        server_ai_step(&mut m, None);
        assert_eq!(m.extra.ints[MOVE_DELAY], 10);
        assert!(!m.ai.jumping);
        for _ in 0..9 {
            server_ai_step(&mut m, None);
        }
        assert!(!m.ai.jumping);
        server_ai_step(&mut m, None);
        assert!(m.ai.jumping);
        assert_eq!(m.extra.ints[JUMP_DURATION], 15);
        assert_eq!(m.anim.states[0].started, None);
        apply_jump(&mut m, 0.2);
        post_tick(&mut m);
        m.age = 1;
        base_tick(&mut m);
        assert_eq!(m.anim.states[0].started, Some(1.0));
        let curve = Animation::parse(&json!({"length": 0.75, "looping": true,
            "bones": {"left_hind_foot": [{"target": "rotation", "keyframes": [[0,[0,0,0],[0,0,0],"linear"],[0.75,[1,0,0],[1,0,0],"linear"]]}]}}));
        let animations = HashMap::from([("RabbitAnimation.HOP".into(), curve)]);
        let mut poses = Poses::new(&m.kind.model);
        let state = anim::state(&m, 0.5);
        body_anim::rabbit(
            &mut poses,
            &state,
            &m.anim.states[0],
            &m.anim.states[1],
            &animations,
        );
        assert!(poses.get("left_hind_foot")[3] > 0.0);
        for _ in 1..15 {
            post_tick(&mut m);
        }
        assert_eq!(m.extra.ints[JUMP_TICKS], 15);
        assert!(m.ai.jumping);
        post_tick(&mut m);
        base_tick(&mut m);
        assert!(!m.ai.jumping);
        assert_eq!(m.anim.states[0].started, None);
    }

    #[test]
    fn panic_and_explicit_requests_use_their_own_hop_timing() {
        let mut m = rabbit();
        m.on_ground = true;
        m.ai.mov.set_wanted(0.0, 0.0, 5.0, 2.2);
        server_ai_step(&mut m, None);
        assert_eq!(m.extra.ints[MOVE_DELAY], 3);
        m.ai.jump = true;
        m.jump_control_tick();
        assert!(m.ai.jumping);
        assert!(!m.ai.jump);
        assert_eq!(m.extra.ints[MOVE_DELAY], 3);
        m.jump_control_tick();
        assert!(m.ai.jumping);
    }

    #[test]
    fn a_new_hop_preserves_a_running_clock_until_the_counter_finishes() {
        let mut m = rabbit();
        m.anim.states[0].start(4.0);
        m.extra.ints[CLIENT_JUMP_TICKS] = 8;
        hop_event(&mut m);
        assert_eq!(m.anim.states[0].started, Some(4.0));
        assert_eq!(m.extra.ints[CLIENT_JUMP_TICKS], 0);
        assert_eq!(m.extra.ints[CLIENT_JUMP_DURATION], 15);
        post_tick(&mut m);
        m.age = 12;
        base_tick(&mut m);
        assert_eq!(m.anim.states[0].started, Some(4.0));
        assert!(m.outbox.is_empty());
    }

    #[test]
    fn deep_water_jump_requests_do_not_start_a_client_hop() {
        let mut m = rabbit();
        m.fluids.in_water = true;
        m.fluids.water = 1.0;
        m.ai.jump = true;
        jump_control(&mut m);
        assert!(m.ai.jumping);
        assert_eq!(m.extra.ints[JUMP_DURATION], 15);
        post_tick(&mut m);
        base_tick(&mut m);
        assert_eq!(m.extra.ints[JUMP_TICKS], 1);
        assert_eq!(m.extra.ints[CLIENT_JUMP_DURATION], 0);
        assert!(m.anim.states[0].started.is_none());
        assert!(
            !m.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::RabbitHop { .. }))
        );
        hop_event(&mut m);
        post_tick(&mut m);
        base_tick(&mut m);
        assert!(m.anim.states[0].started.is_some());
    }

    #[test]
    fn grounded_movement_waits_and_water_uses_its_hop_speed() {
        let mut m = rabbit();
        m.on_ground = true;
        m.ai.mov.set_wanted(0.0, 0.0, 5.0, 0.6);
        prepare_move(&mut m);
        assert_eq!(m.ai.mov.speed_modifier, 0.0);
        assert_eq!(m.ai.zza, 0.0);
        m.ai.jumping = true;
        prepare_move(&mut m);
        assert_eq!(m.ai.mov.speed_modifier, 0.6);
        m.fluids.in_water = true;
        prepare_move(&mut m);
        assert_eq!(m.ai.mov.speed_modifier, 1.5);
        assert_eq!(m.extra.vector[0], 1.5);
        assert!(m.ai.nav.can_float);
        m.extra.flag = false;
        m.ai.mov.set_wanted(0.0, 0.0, 5.0, 2.2);
        server_ai_step(&mut m, None);
        assert_eq!(m.ai.mov.speed_modifier, 1.5);
        assert_eq!(m.extra.ints[MOVE_DELAY], 10);
    }

    #[test]
    fn jump_impulse_scales_strength_terrain_boost_and_baby_kick() {
        let mut m = rabbit();
        m.ai.mov.speed_modifier = 0.6;
        near(f64::from(jump_power(&m, 1.0)), 0.2);
        m.ai.mov.speed_modifier = 2.2;
        near(f64::from(jump_power(&m, 1.0)), 0.3);
        m.horizontal_collision = true;
        near(f64::from(jump_power(&m, 1.0)), 0.5);
        m.attrs.jump_strength = 0.84;
        m.extra.effects.push(("minecraft:jump_boost", 100, 1));
        near(f64::from(jump_power(&m, 0.5)), 0.7);
        apply_jump(&mut m, 0.2);
        near(m.velocity.z, 0.1 / 3.25_f64.sqrt());
        near(m.velocity.y, 0.2 + 0.15 / 3.25_f64.sqrt());
        m.extra.age = -100;
        m.velocity = Vec3::ZERO;
        apply_jump(&mut m, 0.2);
        near(m.velocity.z, 0.1 / 1.25_f64.sqrt());
        near(m.velocity.y, 0.2 + 0.05 / 1.25_f64.sqrt());
        assert_eq!(m.extra.ints[CLIENT_JUMP_DURATION], 15);
    }

    #[test]
    fn idle_timeout_hop_priority_no_ai_and_leash_follow_tick_order() {
        let mut m = rabbit();
        m.age = 7;
        m.extra.ints[IDLE_TIMEOUT] = 0;
        m.extra.ints[CLIENT_JUMP_TICKS] = 2;
        base_tick(&mut m);
        assert_eq!(m.anim.states[1].started, Some(7.0));
        assert!(m.anim.states[0].started.is_none());
        base_tick(&mut m);
        assert_eq!(m.anim.states[0].started, Some(7.0));
        assert!(m.anim.states[1].started.is_none());
        m.extra.ints[CLIENT_JUMP_TICKS] = 0;
        m.extra.ints[IDLE_TIMEOUT] = 0;
        m.no_ai = true;
        base_tick(&mut m);
        assert_eq!(m.extra.ints[IDLE_TIMEOUT], -1);
        assert!(m.anim.states[1].started.is_none());
        m.no_ai = false;
        base_tick(&mut m);
        assert!(m.anim.states[1].started.is_some());
        m.extra.leash = Some(super::super::leash::Holder::Knot([0, 0, 0]));
        base_tick(&mut m);
        assert!(m.anim.states[1].started.is_none());
    }
}
