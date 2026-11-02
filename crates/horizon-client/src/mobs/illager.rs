//! Illagers outside raids: pillagers (crossbow charging and volleys) and
//! vindicators (axes, Johnny), with `IllagerModel`'s arm poses.
use super::{
    Mob, MobEvent, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        Stroll, goal_clone, reduced,
    },
};
use crate::items::ItemStack;
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::trig::{DEG_TO_RAD, Mth};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::f32::consts::PI;

fn targets(m: &mut Mob, villagers_seen: bool) {
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Witch,
        Species::Evoker,
        Species::Illusioner,
        Species::Vex,
        Species::Ravager,
    ];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets.add(
        3,
        NearestTarget::species(
            &[Species::Villager, Species::WanderingTrader],
            villagers_seen,
        ),
    );
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::IronGolem], true));
}
/// `AvoidEntityGoal<>(this, Creaking.class, 8.0F, walk, sprint)`.
fn avoid_creaking(walk: f64, sprint: f64) -> super::goals::AvoidEntity {
    super::goals::AvoidEntity::new(
        super::goals::Avoid::Species(vec![Species::Creaking]),
        8.0,
        walk,
        sprint,
    )
}
pub fn register_pillager(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, avoid_creaking(1.0, 1.2));
    g.add(2, super::raider::HoldGroundAttack::new(10.0));
    g.add(3, CrossbowAttack::new(1.0, 8.0));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(15.0).probability(1.0));
    g.add(10, LookAtPlayer::mobs(15.0));
    targets(m, false);
}
pub fn register_vindicator(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, avoid_creaking(1.0, 1.2));
    g.add(2, super::doors::BreakDoor::vindicator());
    g.add(3, super::doors::RaiderOpenDoor::default());
    g.add(4, super::raider::HoldGroundAttack::new(10.0));
    g.add(5, MeleeAttack::new(1.0, false));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(3.0).probability(1.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    targets(m, true);
    // VindicatorJohnnyAttackGoal: a vindicator named Johnny attacks any
    // living entity, other raiders included.
    m.ai.targets.add(
        4,
        NearestTarget::new(Prey::Filter(|_| true), true)
            .interval(0)
            .when(|m, _| m.custom_name.as_deref() == Some("Johnny")),
    );
}
/// `Vindicator.customServerAiStep`: its navigation opens doors while a raid
/// is at its position.
pub fn vindicator_server_ai_step(m: &mut Mob) {
    if super::doors::can_navigate_ground(m) {
        m.ai.nav.can_open_doors = super::raider::state(m).is_some_and(|s| s.raided);
    }
}
// Evoker state: `counter` spellCastingTickCount, `variant2` the spell
// being cast (0 none, 1 vexes, 2 fangs, 3 wololo), `counter2` the wololo
// sheep's id.
pub fn register_evoker(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, CastingSpell);
    g.add(
        2,
        super::goals::AvoidEntity::new(super::goals::Avoid::Player, 8.0, 0.6, 1.0),
    );
    g.add(3, avoid_creaking(0.6, 1.0));
    g.add(4, UseSpell::new(Spell::Vexes));
    g.add(5, UseSpell::new(Spell::Fangs));
    g.add(6, UseSpell::new(Spell::Wololo));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(3.0).probability(1.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    targets(m, false);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spell {
    Vexes = 1,
    Fangs = 2,
    Wololo = 3,
}
/// `IllagerSpell.spellColor` as `(float)` red, green and blue, by spell id
/// (1 summon vex, 2 fangs, 3 wololo, 4 disappear, 5 blindness).
fn spell_color(id: u8) -> [f32; 3] {
    match id {
        1 => [0.7, 0.7, 0.8],
        2 => [0.4, 0.3, 0.35],
        3 => [0.7, 0.5, 0.2],
        4 => [0.3, 0.3, 0.8],
        5 => [0.1, 0.1, 0.2],
        _ => [0.0; 3],
    }
}
/// The client half of `SpellcasterIllager.tick`: while the casting-spell data
/// is above 0, two `entity_effect` particles in the spell's colour, one at each
/// hand, which swing with the body and a wobble of the age.
pub fn cast_particles(m: &Mob, events: &mut Vec<MobEvent>) {
    if m.extra.variant2 == 0 {
        return;
    }
    let [r, g, b] = spell_color(m.extra.variant2);
    let angle = m.body_yaw * DEG_TO_RAD + (m.age as f32 * 0.6662).mcos() * 0.25;
    let (cos, sin) = (f64::from(angle.mcos()), f64::from(angle.msin()));
    let scale = super::riders::scale(m);
    let (reach, height) = (0.6 * scale, 1.8 * scale);
    let p = m.position;
    for side in [1.0, -1.0] {
        events.push(MobEvent::ParticleColored {
            at: [
                p.x + side * cos * reach,
                p.y + height,
                p.z + side * sin * reach,
            ],
            color: [r, g, b],
        });
    }
}
/// `SpellcasterCastingSpellGoal`: stand still facing the target while a
/// spell is cast.
#[derive(Clone, Debug)]
pub(super) struct CastingSpell;
impl Goal for CastingSpell {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.counter > 0
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.ai.nav.stop();
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.variant2 = 0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let look = match m.target_info(cx.w) {
            Some(t) => Some([t.position.x, t.eye_y, t.position.z]),
            None => {
                let id = m.extra.counter2 as u32;
                cx.w.others
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| [s.position.x, s.eye_y, s.position.z])
            }
        };
        if let Some(at) = look {
            let (y, x) = (m.kind.head_rot_speed, m.kind.max_head_x_rot);
            m.look_at_speed(at, y, x);
        }
    }
}
/// `SpellcasterUseSpellGoal` with the evoker's three spells.
#[derive(Clone, Debug)]
struct UseSpell {
    spell: Spell,
    warmup: i32,
    next: i64,
}
impl UseSpell {
    fn new(spell: Spell) -> Self {
        Self {
            spell,
            warmup: 0,
            next: 0,
        }
    }
    /// (warm-up, casting time, interval, prepare sound).
    fn timing(&self) -> (i32, i32, i64, &'static str) {
        match self.spell {
            Spell::Vexes => (20, 100, 340, "entity.evoker.prepare_summon"),
            Spell::Fangs => (20, 40, 100, "entity.evoker.prepare_attack"),
            Spell::Wololo => (40, 60, 140, "entity.evoker.prepare_wololo"),
        }
    }
}
impl Goal for UseSpell {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let age = i64::from(m.age);
        if self.spell == Spell::Wololo {
            // EvokerWololoSpellGoal.canUse: a blue sheep within the box
            // (16 sideways, 4 up or down) and `range(16)`, no target, griefing on.
            if m.ai.target.is_some() || m.extra.counter > 0 || age < self.next || !cx.w.mob_griefing
            {
                return false;
            }
            let p = m.position;
            let sheep: Vec<u32> =
                cx.w.others
                    .iter()
                    .filter(|s| {
                        s.species == Species::Sheep
                            && s.alive
                            && s.color == 11
                            && (s.position.x - p.x).abs() <= 16.0
                            && (s.position.y - p.y).abs() <= 4.0
                            && (s.position.z - p.z).abs() <= 16.0
                            && s.position.distance_squared(p) <= 16.0 * 16.0
                    })
                    .map(|s| s.id)
                    .collect();
            if sheep.is_empty() {
                return false;
            }
            let pick = m.random.next_int_bound(sheep.len() as i32).unwrap_or(0) as usize;
            m.extra.counter2 = sheep[pick] as i32;
            return true;
        }
        if !m.target_info(cx.w).is_some_and(|t| t.alive) || m.extra.counter > 0 || age < self.next {
            return false;
        }
        if self.spell == Spell::Vexes {
            // EvokerSummonSpellGoal: fewer vexes within `range(16)` than a
            // roll of 1..8.
            let p = m.position;
            let vexes =
                cx.w.others
                    .iter()
                    .filter(|s| {
                        s.species == Species::Vex
                            && s.alive
                            && s.position.distance_squared(p) <= 16.0 * 16.0
                    })
                    .count() as i32;
            return m.random.next_int_bound(8).unwrap_or(0) + 1 > vexes;
        }
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.spell == Spell::Wololo {
            return m.extra.counter2 != 0 && self.warmup > 0;
        }
        m.target_info(cx.w).is_some_and(|t| t.alive) && self.warmup > 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (warmup, casting, interval, sound) = self.timing();
        self.warmup = reduced(warmup);
        m.extra.counter = casting;
        self.next = i64::from(m.age) + interval;
        cx.events.push(MobEvent::Sound(
            sound,
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
        m.extra.variant2 = self.spell as u8;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        if self.spell == Spell::Wololo {
            m.extra.counter2 = 0;
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.warmup -= 1;
        if self.warmup != 0 {
            return;
        }
        match self.spell {
            Spell::Vexes => summon_vexes(m, cx),
            Spell::Fangs => cast_fangs(m, cx),
            Spell::Wololo => {
                let id = m.extra.counter2 as u32;
                if id != 0 {
                    cx.events.push(MobEvent::Wololo { mob: id });
                }
            }
        }
        cx.events.push(MobEvent::Sound(
            "entity.evoker.cast_spell",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
    }
}
/// `EvokerSummonSpellGoal.performSpellCasting`: three vexes nearby for 30
/// to 120 seconds.
fn summon_vexes(m: &mut Mob, cx: &mut Cx) {
    let base = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    for _ in 0..3 {
        let r = &mut m.random;
        let p = [
            base[0] - 2 + r.next_int_bound(5).unwrap_or(2),
            base[1] + 1,
            base[2] - 2 + r.next_int_bound(5).unwrap_or(2),
        ];
        let life = 20 * (30 + r.next_int_bound(90).unwrap_or(0));
        cx.events.push(MobEvent::SpawnKind {
            kind: "minecraft:vex".into(),
            at: [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
            setup: super::SpawnSetup {
                owner_mob: Some(m.id),
                home: Some(p),
                life: Some(life),
                ..Default::default()
            },
        });
    }
}
/// `EvokerAttackSpellGoal.performSpellCasting`: a ring of fangs when the
/// target is close, else a line of sixteen toward it.
fn cast_fangs(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else {
        return;
    };
    let min_y = t.position.y.min(m.position.y);
    let max_y = t.position.y.max(m.position.y) + 1.0;
    let angle = (t.position.z - m.position.z).atan2(t.position.x - m.position.x) as f32;
    let mut place = |x: f64, z: f64, a: f32, delay: i32| {
        if let Some(y) = fang_floor(cx.w, x, z, min_y, max_y) {
            cx.events.push(MobEvent::Fangs {
                at: [x, y, z],
                yaw: a.to_degrees(),
                delay,
                owner: m.id,
            });
        }
    };
    let (x, z) = (m.position.x, m.position.z);
    if m.distance_sqr(t.position) < 9.0 {
        for i in 0..5 {
            let a = angle + i as f32 * PI * 0.4;
            place(
                x + f64::from(a.mcos()) * 1.5,
                z + f64::from(a.msin()) * 1.5,
                a,
                0,
            );
        }
        for i in 0..8 {
            let a = angle + i as f32 * PI * 2.0 / 8.0 + PI * 2.0 / 5.0;
            place(
                x + f64::from(a.mcos()) * 2.5,
                z + f64::from(a.msin()) * 2.5,
                a,
                3,
            );
        }
    } else {
        for i in 0..16 {
            let reach = 1.25 * f64::from(i + 1);
            place(
                x + f64::from(angle.mcos()) * reach,
                z + f64::from(angle.msin()) * reach,
                angle,
                i,
            );
        }
    }
}
/// `createSpellEntity`: the first block below `max_y` whose up face is
/// sturdy, standing on the top of whatever collision shape is in the way.
///
/// The floor test is `BlockState.isFaceSturdy(.., UP)`, not a full cube: a
/// soul sand or mud block is 14/16 high and still supports a fang, where
/// requiring a full collision shape drops the fang into the block.
fn fang_floor(w: &super::MobWorld, x: f64, z: f64, min_y: f64, max_y: f64) -> Option<f64> {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let mut y = max_y.floor() as i32;
    while y >= min_y.floor() as i32 - 1 {
        let below = w.world.block(bx, y - 1, bz);
        if w.data.state(below).flags & super::data::STURDY_TOP != 0 {
            let here = w.world.block(bx, y, bz);
            let top = if w.facts.is_air(here) {
                0.0
            } else {
                f64::from(w.data.state(here).top)
            };
            return Some(f64::from(y) + top);
        }
        y -= 1;
    }
    None
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Crossbow {
    Uncharged,
    Charging,
    Charged,
    Ready,
}
/// `RangedCrossbowAttackGoal`.
#[derive(Clone, Debug)]
struct CrossbowAttack {
    speed: f64,
    radius_sqr: f64,
    state: Crossbow,
    see_time: i32,
    attack_delay: i32,
    path_delay: i32,
}
impl CrossbowAttack {
    fn new(speed: f64, radius: f64) -> Self {
        Self {
            speed,
            radius_sqr: radius * radius,
            state: Crossbow::Uncharged,
            see_time: 0,
            attack_delay: 0,
            path_delay: 0,
        }
    }
}
/// The crossbow goal for other crossbow users (piglins).
pub fn crossbow_goal(speed: f64, radius: f64) -> impl Goal {
    CrossbowAttack::new(speed, radius)
}
/// `CrossbowItem.getChargeDuration` (25 ticks, 5 fewer per level of Quick Charge).
pub fn charge_ticks(m: &Mob) -> i32 {
    25 - 5 * super::gear::level(m, super::gear::MAIN, "minecraft:quick_charge") as i32
}
fn holding_crossbow(m: &Mob) -> bool {
    m.extra.main_hand.as_deref() == Some("minecraft:crossbow")
}

/// `CrossbowItem.setCharged`: the loaded arrow lives in the stack's
/// `charged_projectiles` component, which is what draws it in the crossbow and
/// what `CrossbowItem.isCharged` reads. A mob's held stack carries the
/// components it was spawned or equipped with, so a pillager that loads its
/// own crossbow has to set them itself.
fn set_crossbow_charged(m: &mut Mob, charged: bool) {
    let mut stack = m
        .extra
        .main_hand_stack()
        .cloned()
        .unwrap_or_else(|| ItemStack::new("minecraft:crossbow", 1));
    if &*stack.item != "minecraft:crossbow" {
        return;
    }
    if charged {
        if stack.data().is_some_and(|d| !d.charged.is_empty()) {
            return;
        }
        stack.data_mut().charged = vec![ItemStack::new("minecraft:arrow", 1)];
    } else if stack.data().is_none_or(|d| d.charged.is_empty()) {
        return;
    } else {
        stack.data_mut().charged.clear();
    }
    m.extra.set_main_hand_stack(Some(stack));
}
impl Goal for CrossbowAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        holding_crossbow(m)
            && m.ai
                .target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && !(t.is_player && t.creative))
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let valid =
            m.ai.target
                .and_then(|t| cx.w.target_info(t))
                .is_some_and(|t| t.alive && !(t.is_player && t.creative));
        valid && holding_crossbow(m)
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = false;
        m.ai.target = None;
        self.see_time = 0;
        m.extra.flag = false;
        m.extra.using_item = 0;
        self.state = Crossbow::Uncharged;
        set_crossbow_charged(m, false);
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
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
        let sees = m.sees(cx.w, who);
        if sees != (self.see_time > 0) {
            self.see_time = 0;
        }
        self.see_time += if sees { 1 } else { -1 };
        let d2 = m.distance_sqr(t.position);
        let needs_to_move = (d2 > self.radius_sqr || self.see_time < 5) && self.attack_delay == 0;
        if needs_to_move {
            self.path_delay -= 1;
            if self.path_delay <= 0 {
                let speed = if self.state == Crossbow::Uncharged {
                    self.speed
                } else {
                    self.speed * 0.5
                };
                m.nav_move_to_entity(cx.w, [t.position.x, t.position.y, t.position.z], 0, speed);
                self.path_delay = 20 + m.random.next_int_bound(21).unwrap_or(0);
            }
        } else {
            self.path_delay = 0;
            m.nav_stop();
        }
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        match self.state {
            Crossbow::Uncharged => {
                if !needs_to_move {
                    self.state = Crossbow::Charging;
                    m.extra.flag = true;
                    m.extra.using_item = 0;
                    let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                    cx.events.push(MobEvent::Sound(
                        "item.crossbow.loading_start",
                        m.center(),
                        1.0,
                        pitch,
                    ));
                }
            }
            Crossbow::Charging => {
                m.extra.using_item += 1;
                if m.extra.using_item >= charge_ticks(m) {
                    m.extra.using_item = 0;
                    m.extra.flag = false;
                    self.state = Crossbow::Charged;
                    set_crossbow_charged(m, true);
                    self.attack_delay = 20 + m.random.next_int_bound(20).unwrap_or(0);
                    cx.events.push(MobEvent::Sound(
                        "item.crossbow.loading_end",
                        m.center(),
                        1.0,
                        1.0 / (m.random.next_float() * 0.5 + 1.0) + 0.2,
                    ));
                }
            }
            Crossbow::Charged => {
                self.attack_delay -= 1;
                if self.attack_delay == 0 {
                    self.state = Crossbow::Ready;
                }
            }
            Crossbow::Ready => {
                if sees {
                    shoot_crossbow(m, who, cx);
                    self.state = Crossbow::Uncharged;
                    set_crossbow_charged(m, false);
                }
            }
        }
    }
}
/// `CrossbowAttackMob.performCrossbowAttack` at power 1.6: one arrow aimed a
/// third of the way up the target, lifted by a fifth of the distance.
fn shoot_crossbow(m: &mut Mob, who: Who, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
    let dx = t.position.x - m.position.x;
    let dz = t.position.z - m.position.z;
    let dist = (dx * dx + dz * dz).sqrt();
    let third =
        t.bounds.min_position().y + (t.bounds.max_position().y - t.bounds.min_position().y) / 3.0;
    let dy = third - start.y + dist * 0.2;
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        start,
        Some(Who::Mob(m.id)),
    );
    p.set_bonus(super::gear::arrow_bonus(m));
    // rangedAttackUncertainty: 14 - 4 * difficulty.
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [dx, dy, dz],
        1.6,
        (14 - i32::from(cx.w.difficulty.id) * 4) as f32,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 / (m.random.next_float() * 0.5 + 1.8) + 0.53;
    cx.events.push(MobEvent::Sound(
        "item.crossbow.shoot",
        m.center(),
        1.0,
        pitch,
    ));
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArmPose {
    BowAndArrow,
    Spellcasting,
    Crossed,
    Celebrating,
    Neutral,
    Attacking,
    CrossbowHold,
    CrossbowCharge,
    Bow,
}
/// `Pillager` / `Vindicator.getArmPose`.
fn arm_pose(m: &Mob) -> ArmPose {
    match m.species() {
        Species::Pillager => {
            if m.extra.flag {
                ArmPose::CrossbowCharge
            } else if holding_crossbow(m) {
                ArmPose::CrossbowHold
            } else if m.ai.aggressive {
                ArmPose::Attacking
            } else {
                ArmPose::Neutral
            }
        }
        Species::Illusioner if m.extra.variant2 != 0 => ArmPose::Spellcasting,
        Species::Illusioner if m.ai.aggressive => ArmPose::Bow,
        Species::Illusioner => ArmPose::Crossed,
        Species::Evoker if m.extra.variant2 != 0 => ArmPose::Spellcasting,
        Species::Evoker if celebrating(m) => ArmPose::Celebrating,
        Species::Evoker => ArmPose::Crossed,
        _ => {
            if m.ai.aggressive {
                ArmPose::Attacking
            } else if celebrating(m) {
                ArmPose::Celebrating
            } else {
                ArmPose::Crossed
            }
        }
    }
}
fn celebrating(m: &Mob) -> bool {
    super::raider::state(m).is_some_and(|s| s.celebrating)
}
/// Whether the held item shows (vindicators only draw their axe when angry).
pub fn shows_item(m: &Mob) -> bool {
    match m.species() {
        Species::Vindicator => m.ai.aggressive,
        Species::Illusioner => m.extra.variant2 != 0 || m.ai.aggressive,
        _ => true,
    }
}
fn hide(p: &mut Poses, part: &str) {
    p.visible(part, false);
}
/// `IllagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    anim_pose(m, s, p, arm_pose(m));
}
/// `IllagerModel.setupAnim` for a given arm pose.
pub fn anim_pose(m: &Mob, s: &State, p: &mut Poses, pose: ArmPose) {
    // IllagerModel hides the hat; the illusioner's renderer shows it.
    if m.species() != Species::Illusioner {
        hide(p, "hat");
    }
    let (hy, hx) = (s.y_rot.to_radians(), s.x_rot.to_radians());
    p.yrot("head", hy);
    p.xrot("head", hx);
    let (pos, sp) = (s.pos, s.speed);
    if m.extra.passenger {
        for arm in ["right_arm", "left_arm"] {
            p.xrot(arm, -PI / 5.0);
            p.yrot(arm, 0.0);
            p.zrot(arm, 0.0);
        }
        p.xrot("right_leg", -1.413_716_7);
        p.yrot("right_leg", PI / 10.0);
        p.zrot("right_leg", 0.078_539_82);
        p.xrot("left_leg", -1.413_716_7);
        p.yrot("left_leg", -PI / 10.0);
        p.zrot("left_leg", -0.078_539_82);
    } else {
        p.xrot("right_arm", (pos * 0.6662 + PI).mcos() * 2.0 * sp * 0.5);
        p.yrot("right_arm", 0.0);
        p.zrot("right_arm", 0.0);
        p.xrot("left_arm", (pos * 0.6662).mcos() * 2.0 * sp * 0.5);
        p.yrot("left_arm", 0.0);
        p.zrot("left_arm", 0.0);
        p.xrot("right_leg", (pos * 0.6662).mcos() * 1.4 * sp * 0.5);
        p.yrot("right_leg", 0.0);
        p.zrot("right_leg", 0.0);
        p.xrot("left_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp * 0.5);
        p.yrot("left_leg", 0.0);
        p.zrot("left_leg", 0.0);
    }
    match pose {
        ArmPose::Attacking => {
            if m.extra.main_hand.is_none() {
                // animateZombieArms + bobArms
                let swing = s.swing;
                let a = (swing * PI).msin();
                let b = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", -(0.1 - a * 0.6));
                p.yrot("left_arm", 0.1 - a * 0.6);
                let x = -PI / 1.5;
                p.xrot("right_arm", x + a * 1.2 - b * 0.4);
                p.xrot("left_arm", x + a * 1.2 - b * 0.4);
            } else {
                // swingWeaponDown: the main arm swings down.
                let (main, off) = if m.main_arm_left() {
                    ("left_arm", "right_arm")
                } else {
                    ("right_arm", "left_arm")
                };
                let swing = s.swing;
                let a2 = (swing * PI).msin();
                let a = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", PI / 20.0);
                p.yrot("left_arm", -PI / 20.0);
                p.xrot(
                    main,
                    -1.884_955_8 + (s.age * 0.09).mcos() * 0.15 + (a2 * 2.2 - a * 0.4),
                );
                p.xrot(
                    off,
                    -0.0 + (s.age * 0.19).mcos() * 0.5 + (a2 * 1.2 - a * 0.4),
                );
            }
            let bob = |p: &mut Poses, part: &str, sign: f32| {
                let v = p.get(part);
                v[5] += sign * ((s.age * 0.09).mcos() * 0.05 + 0.05);
                v[3] += sign * (s.age * 0.067).msin() * 0.05;
            };
            bob(p, "right_arm", 1.0);
            bob(p, "left_arm", -1.0);
        }
        ArmPose::Bow => {
            p.yrot("right_arm", -0.1 + hy);
            p.xrot("right_arm", -PI / 2.0 + hx);
            p.xrot("left_arm", -0.942_477_9 + hx);
            p.yrot("left_arm", hy - 0.4);
            p.zrot("left_arm", PI / 2.0);
        }
        ArmPose::CrossbowHold => {
            p.yrot("right_arm", -0.3 + hy);
            p.yrot("left_arm", 0.6 + hy);
            p.xrot("right_arm", -PI / 2.0 + hx + 0.1);
            p.xrot("left_arm", -1.5 + hx);
        }
        ArmPose::CrossbowCharge => {
            p.yrot("right_arm", -0.8);
            p.xrot("right_arm", -0.970_796_35);
            let charge = charge_ticks(m) as f32;
            let a = (m.extra.using_item as f32 + s.partial).clamp(0.0, charge) / charge;
            p.yrot("left_arm", 0.4 + (0.85 - 0.4) * a);
            p.xrot("left_arm", -0.970_796_35 + (-PI / 2.0 + 0.970_796_35) * a);
        }
        ArmPose::Spellcasting => {
            for (arm, x, z_rot) in [
                ("right_arm", -5.0, PI * 0.75),
                ("left_arm", 5.0, -PI * 0.75),
            ] {
                let v = p.get(arm);
                v[2] = 0.0;
                v[0] = x;
                v[3] = (s.age * 0.6662).mcos() * 0.25;
                v[4] = 0.0;
                v[5] = z_rot;
            }
        }
        ArmPose::Celebrating => {
            for (arm, x, z_rot) in [
                ("right_arm", -5.0, 2.670_354),
                ("left_arm", 5.0, -PI * 0.75),
            ] {
                let v = p.get(arm);
                v[2] = 0.0;
                v[0] = x;
                v[3] = (s.age * 0.6662).mcos() * 0.05;
                v[4] = 0.0;
                v[5] = z_rot;
            }
        }
        ArmPose::BowAndArrow => {
            p.yrot("right_arm", -0.1 + hy);
            p.xrot("right_arm", -PI / 2.0 + hx);
            p.xrot("left_arm", -0.942_477_9 + hx);
            p.yrot("left_arm", hy - 0.4);
            p.zrot("left_arm", PI / 2.0);
        }
        ArmPose::Crossed | ArmPose::Neutral => {}
    }
    if pose == ArmPose::Crossed {
        hide(p, "left_arm");
        hide(p, "right_arm");
    } else {
        hide(p, "arms");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::Species;
    use crate::mobs::sensor_scan_tests::kind;
    use std::sync::Arc;

    fn evoker(spell: u8) -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:evoker".into();
        k.species = Species::Evoker;
        let mut m = Mob::new(1, kind, Vec3::new(10.0, 64.0, -5.0), 0.0);
        m.extra.variant2 = spell;
        m.body_yaw = 90.0;
        m.age = 7;
        m
    }

    /// `SpellcasterIllager.tick`: two coloured particles at the hands while a
    /// spell is cast, none otherwise; the colours are `IllagerSpell.spellColor`.
    #[test]
    fn a_casting_evoker_has_a_particle_in_each_hand() {
        let mut events = Vec::new();
        cast_particles(&evoker(0), &mut events);
        assert!(events.is_empty());
        for (id, want) in [
            (1, [0.7, 0.7, 0.8]),
            (2, [0.4, 0.3, 0.35]),
            (3, [0.7, 0.5, 0.2]),
            (4, [0.3, 0.3, 0.8]),
            (5, [0.1, 0.1, 0.2]),
        ] {
            events.clear();
            cast_particles(&evoker(id), &mut events);
            assert_eq!(events.len(), 2);
            let hands: Vec<([f64; 3], [f32; 3])> = events
                .iter()
                .map(|e| match e {
                    MobEvent::ParticleColored { at, color } => (*at, *color),
                    _ => panic!("not a coloured particle"),
                })
                .collect();
            assert_eq!(hands[0].1, want);
            assert_eq!(hands[1].1, want);
            // Both at y + 1.8, mirrored about the mob, 0.6 away.
            assert!((hands[0].0[1] - 65.8).abs() < 1e-9);
            assert!((hands[0].0[0] + hands[1].0[0] - 20.0).abs() < 1e-9);
            assert!((hands[0].0[2] + hands[1].0[2] + 10.0).abs() < 1e-9);
            let (dx, dz) = (hands[0].0[0] - 10.0, hands[0].0[2] + 5.0);
            assert!((dx.hypot(dz) - 0.6).abs() < 1e-6);
        }
    }

    fn pillager() -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:pillager".into();
        k.species = Species::Pillager;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `CrossbowItem.setCharged`: the arrow lives in the stack's components,
    /// and the model draws whatever the stack carries.
    #[test]
    fn a_charged_crossbow_carries_its_arrow() {
        let mut m = pillager();
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:crossbow", 1)));
        set_crossbow_charged(&mut m, true);
        let stack = m.extra.main_hand_stack().unwrap();
        let charged = &stack.data().unwrap().charged;
        assert_eq!(charged.len(), 1);
        assert_eq!(&*charged[0].item, "minecraft:arrow");
        set_crossbow_charged(&mut m, false);
        assert!(
            m.extra
                .main_hand_stack()
                .unwrap()
                .data()
                .unwrap()
                .charged
                .is_empty()
        );
    }

    /// A shot crossbow keeps its stack; only the loaded arrow goes.
    #[test]
    fn charging_leaves_anything_but_a_crossbow_alone() {
        let mut m = pillager();
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
        set_crossbow_charged(&mut m, true);
        assert!(
            m.extra
                .main_hand_stack()
                .unwrap()
                .data()
                .is_none_or(|d| d.charged.is_empty())
        );
    }
    /// `EvokerAttackSpellGoal.createSpellEntity` asks the block below for a
    /// sturdy up face, not for a full cube. Soul sand is 14/16 high and still
    /// supports a fang, which a full-collision test drops into the block.
    #[test]
    fn a_fang_stands_on_a_soul_sand_floor() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let soul_sand = registry.default_state("minecraft:soul_sand").unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world.set(0, 64, 0, soul_sand).unwrap();
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, _| {
            // An evoker standing on the soul sand has its feet at 64.875, so
            // the cast asks for a floor from 65 down: the soul sand at 64 is
            // the first sturdy block and the fang sits at 65.
            assert_eq!(fang_floor(w, 0.5, 0.5, 64.0, 65.875), Some(65.0));
        });
        let _ = ran;
    }
}
