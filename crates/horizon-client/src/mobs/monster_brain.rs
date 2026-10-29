//! The brains of the breeze (`Breeze.BRAIN_PROVIDER`, `BreezeAi`) and the
//! creaking (`Creaking.BRAIN_PROVIDER`, `CreakingAi`), ported behaviour for
//! behaviour on the villager brain's memories (`villager_brain::Slot`):
//! sensors with vanilla's scan delays, the core, idle and fight activities,
//! and their behaviours started and ticked in `Brain.tick`'s order
//! (priority, then the activity's slot in its hash map, then insertion).
//!
//! Behaviour durations, `DoNothing` and the creaking's look ticker draw from
//! the level random; positions, sounds and jumps from the mob's own random;
//! `RunOne` shuffles with a random of its own (`ShufflingList.random`),
//! seeded when the brain is built.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    brain::{Timed, living_near},
    goals::Cx,
    path::Path,
    villager_brain::{NEVER, Slot, Tracker, Visible, WalkTarget, compute_path, reached},
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{
    geometry::{Vec3, trig},
    random::{LegacyRandom, RandomSource},
};

/// `Activity` (the ones these brains use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Core,
    Idle,
    Fight,
}
impl Act {
    fn name(self) -> &'static str {
        match self {
            Act::Core => "core",
            Act::Idle => "idle",
            Act::Fight => "fight",
        }
    }
}

/// The memories either brain registers.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    pub attack_target: Slot<Who>,
    pub attack_cooling_down: Slot<bool>,
    pub cant_reach_walk_target_since: Slot<i64>,
    /// `HURT_BY`: the damage (its game time).
    pub hurt_by: Slot<i64>,
    pub hurt_by_entity: Slot<Who>,
    pub look_target: Slot<Tracker>,
    pub mobs: Slot<Vec<Who>>,
    pub visible_mobs: Slot<Visible>,
    pub nearest_attackable: Slot<Who>,
    pub nearest_players: Slot<Vec<Who>>,
    pub nearest_visible_player: Slot<Who>,
    pub nearest_visible_targetable_player: Slot<Who>,
    pub nearest_visible_targetable_players: Slot<Vec<Who>>,
    pub path: Slot<Path>,
    pub walk_target: Slot<WalkTarget>,
    pub breeze_shoot: Slot<()>,
    pub breeze_shoot_charging: Slot<()>,
    pub breeze_shoot_recovering: Slot<()>,
    pub breeze_shoot_cooldown: Slot<()>,
    pub breeze_jump_cooldown: Slot<()>,
    pub breeze_jump_inhaling: Slot<()>,
    pub breeze_jump_target: Slot<[i32; 3]>,
    pub breeze_leaving_water: Slot<()>,
}
impl Memories {
    /// `Brain.forgetOutdatedMemories`.
    fn tick(&mut self) {
        self.attack_target.tick();
        self.attack_cooling_down.tick();
        self.cant_reach_walk_target_since.tick();
        self.hurt_by.tick();
        self.hurt_by_entity.tick();
        self.look_target.tick();
        self.mobs.tick();
        self.visible_mobs.tick();
        self.nearest_attackable.tick();
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_targetable_player.tick();
        self.nearest_visible_targetable_players.tick();
        self.path.tick();
        self.walk_target.tick();
        self.breeze_shoot.tick();
        self.breeze_shoot_charging.tick();
        self.breeze_shoot_recovering.tick();
        self.breeze_shoot_cooldown.tick();
        self.breeze_jump_cooldown.tick();
        self.breeze_jump_inhaling.tick();
        self.breeze_jump_target.tick();
        self.breeze_leaving_water.tick();
    }
}

/// The sensors, in `BRAIN_PROVIDER` order (all scan every 20 ticks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sensor {
    NearestLiving,
    HurtBy,
    NearestPlayers,
    BreezeAttack,
}

/// A breeze's or creaking's brain.
#[derive(Clone, Debug)]
pub struct Brain {
    pub mem: Memories,
    sensors: Vec<(Sensor, i64)>,
    /// The active non-core activity (`CORE` is always active).
    pub activity: Act,
    default_activity: Act,
    list: Vec<Entry>,
}
#[derive(Clone, Debug)]
struct Entry {
    act: Act,
    b: B,
}

fn brain(m: &Mob) -> Option<&Brain> {
    m.extra.brain.as_deref()
}
fn brain_mut(m: &mut Mob) -> &mut Brain {
    m.extra.brain.as_deref_mut().expect("brain")
}
fn mem(m: &mut Mob) -> &mut Memories {
    &mut brain_mut(m).mem
}
/// `CANT_REACH_WALK_TARGET_SINCE`, for `compute_path`.
fn cant_reach(m: &mut Mob) -> &mut Slot<i64> {
    &mut mem(m).cant_reach_walk_target_since
}
fn mem_ref(m: &Mob) -> &Memories {
    &brain(m).expect("brain").mem
}
/// Whether the species thinks with this brain.
pub fn has_brain(s: Species) -> bool {
    matches!(s, Species::Breeze | Species::Creaking)
}

/// `Brain.Provider.makeBrain`: the sensors' first scans are delayed by draws
/// from the mob's random; `RunOne` shuffles with `shuffle_seed`.
pub fn init(m: &mut Mob, shuffle_seed: i64) {
    let breeze = m.species() == Species::Breeze;
    let kinds: &[Sensor] = if breeze {
        &[
            Sensor::NearestLiving,
            Sensor::HurtBy,
            Sensor::NearestPlayers,
            Sensor::BreezeAttack,
        ]
    } else {
        &[Sensor::NearestLiving, Sensor::NearestPlayers]
    };
    let sensors = kinds
        .iter()
        .map(|&k| (k, i64::from(m.random.next_int_bound(20).unwrap_or(0))))
        .collect();
    let (list, default_activity) = if breeze {
        (breeze_packages(shuffle_seed), Act::Fight)
    } else {
        (creaking_packages(shuffle_seed), Act::Idle)
    };
    m.extra.brain = Some(Box::new(Brain {
        mem: Memories::default(),
        sensors,
        activity: default_activity,
        default_activity,
        list,
    }));
}

/// What behaviours read and emit during a brain tick.
struct Bx<'a, 'w> {
    w: &'a MobWorld<'w>,
    lr: &'a mut LegacyRandom,
    events: &'a mut Vec<MobEvent>,
    now: i64,
}

/// `Brain.tick`, then the species' `updateActivity`
/// (`Breeze.customServerAiStep`, `Creaking.customServerAiStep`).
pub fn tick(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if brain(m).is_none() {
        let seed = i64::from(m.id);
        init(m, seed);
    }
    mem(m).tick();
    tick_sensors(m, w);
    let mut list = std::mem::take(&mut brain_mut(m).list);
    let mut cx = Bx {
        w,
        lr,
        events,
        now: w.game_time,
    };
    for e in list.iter_mut() {
        let active = e.act == Act::Core || e.act == brain_mut(m).activity;
        if active && !e.b.running() {
            e.b.try_start(m, &mut cx);
        }
    }
    for e in list.iter_mut() {
        if e.b.running() {
            e.b.tick_or_stop(m, &mut cx);
        }
    }
    brain_mut(m).list = list;
    update_activity(m);
    sync_target(m);
}
/// `BreezeAi.updateActivity` and `CreakingAi.updateActivity`.
fn update_activity(m: &mut Mob) {
    let frozen = m.species() == Species::Creaking && !super::creaking::can_move(m);
    let b = brain_mut(m);
    if frozen {
        // useDefaultActivity
        b.activity = b.default_activity;
        return;
    }
    if fight_requirements(b) {
        b.activity = Act::Fight;
    } else {
        b.activity = Act::Idle;
    }
}
fn fight_requirements(b: &Brain) -> bool {
    b.mem.attack_target.present()
        && (b.default_activity != Act::Fight || !b.mem.walk_target.present())
}
/// `Mob.getTarget` is `getTargetFromBrain` for both species.
fn sync_target(m: &mut Mob) {
    m.ai.target = mem_ref(m).attack_target.value();
}
/// Sets or erases the attack target (and the mob's view of it).
pub fn set_attack_target(m: &mut Mob, who: Option<Who>) {
    if brain(m).is_none() {
        let seed = i64::from(m.id);
        init(m, seed);
    }
    mem(m).attack_target.set_opt(who);
    m.ai.target = who;
}
/// The attack target the brain remembers.
pub fn attack_target(m: &Mob) -> Option<Who> {
    brain(m).and_then(|b| b.mem.attack_target.value())
}
/// `NEAREST_PLAYERS`.
pub fn nearest_players(m: &Mob) -> Vec<Who> {
    brain(m)
        .and_then(|b| b.mem.nearest_players.value())
        .unwrap_or_default()
}
/// `HURT_BY`'s damage source entity, if a living one.
fn hurt_by_entity(m: &Mob, w: &MobWorld) -> Option<Who> {
    let t = brain(m)?.mem.hurt_by.value()?;
    let (d, at) = m.last_damage?;
    if at != t {
        return None;
    }
    let who = d.attacker?;
    w.target_info(who).map(|_| who)
}

// ---- Sensors ------------------------------------------------------------

fn tick_sensors(m: &mut Mob, w: &MobWorld) {
    for i in 0..brain_mut(m).sensors.len() {
        let (kind, fire) = {
            let s = &mut brain_mut(m).sensors[i];
            s.1 -= 1;
            if s.1 <= 0 {
                s.1 = 20;
                (s.0, true)
            } else {
                (s.0, false)
            }
        };
        if !fire {
            continue;
        }
        match kind {
            Sensor::NearestLiving => sense_living(m, w),
            Sensor::HurtBy => sense_hurt(m, w),
            Sensor::NearestPlayers => sense_players(m, w),
            Sensor::BreezeAttack => {
                sense_living(m, w);
                sense_breeze_attack(m, w);
            }
        }
    }
}
/// `NearestLivingEntitySensor`: everything alive within the follow range,
/// nearest first.
fn sense_living(m: &mut Mob, w: &MobWorld) {
    let list = living_near(m, w);
    let mm = mem(m);
    mm.mobs.set_list(list.clone());
    mm.visible_mobs.set(Visible {
        list,
        cache: Vec::new(),
    });
}
/// `HurtBySensor`: the last damage (forgotten 40 ticks after) and who did it.
fn sense_hurt(m: &mut Mob, w: &MobWorld) {
    let age = i64::from(m.age);
    let last = m
        .last_damage
        .as_ref()
        .filter(|(_, t)| age - t <= 40)
        .map(|(d, t)| (d.attacker, *t));
    let mm = mem(m);
    if let Some((by, t)) = last {
        mm.hurt_by.set(t);
        if let Some(by) = by {
            mm.hurt_by_entity.set(by);
        }
    } else {
        mm.hurt_by.clear();
    }
    if let Some(who) = mm.hurt_by_entity.value()
        && !w.target_info(who).is_some_and(|t| t.alive)
    {
        mm.hurt_by_entity.clear();
    }
}
/// `Sensor.isEntityTargetable`: `TargetingConditions.forNonCombat()` within
/// the follow range, in sight.
fn targetable(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => m.can_see_player(w, range),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive && m.distance_sqr(s.position) <= range * range && m.sees(w, who)
        }
    }
}
/// `LivingEntity.canAttack` with the species' own rule (`Breeze.canAttack`:
/// players and iron golems only).
fn can_attack(m: &Mob, w: &MobWorld, who: Who) -> bool {
    match who {
        Who::Player => w
            .player
            .as_ref()
            .is_some_and(|p| p.alive && !p.creative && !p.spectator),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive
                && !s.playing_dead
                && (m.species() != Species::Breeze || s.species == Species::IronGolem)
        }
    }
}
/// `Sensor.isEntityAttackable`: `TargetingConditions.forCombat()` within the
/// follow range, in sight.
fn attackable(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    // The current ATTACK_TARGET is tested with ignoreInvisibilityTesting.
    let current = mem_ref(m).attack_target.value() == Some(who);
    let range = m.attrs.follow_range;
    if !can_attack(m, w, who) {
        return false;
    }
    match who {
        Who::Player if current => m.can_keep_target_player(w, range),
        Who::Player => m.can_target_player(w, range, true),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            m.distance_sqr(s.position) <= range * range && m.sees(w, who)
        }
    }
}
/// The visible-entities memory's line-of-sight test, cached per memory.
fn visible_test(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    match mem_ref(m).visible_mobs.get() {
        Some(v) => {
            if let Some(&(_, r)) = v.cache.iter().find(|(k, _)| *k == who) {
                return r;
            }
        }
        None => return false,
    }
    let r = targetable(m, w, who);
    if let Some(v) = mem(m).visible_mobs.get_mut() {
        v.cache.push((who, r));
    }
    r
}
/// `NearestVisibleLivingEntities.findClosest`.
fn find_closest(m: &mut Mob, w: &MobWorld, filter: impl Fn(&Mob, Who) -> bool) -> Option<Who> {
    let list = mem_ref(m).visible_mobs.get()?.list.clone();
    list.into_iter()
        .find(|&who| filter(m, who) && visible_test(m, w, who))
}
/// `NearestVisibleLivingEntities.contains(entity)`.
fn visible_contains(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let listed = mem_ref(m)
        .visible_mobs
        .get()
        .is_some_and(|v| v.list.contains(&who));
    listed && visible_test(m, w, who)
}
/// `PlayerSensor`.
fn sense_players(m: &mut Mob, w: &MobWorld) {
    let range = m.attrs.follow_range;
    let near = w
        .player
        .as_ref()
        .is_some_and(|p| !p.spectator && m.distance_sqr(p.position) < range * range);
    let players: Vec<Who> = if near { vec![Who::Player] } else { vec![] };
    let visible: Vec<Who> = players
        .iter()
        .copied()
        .filter(|&p| targetable(m, w, p))
        .collect();
    let attackable: Vec<Who> = visible
        .iter()
        .copied()
        .filter(|&p| attackable(m, w, p))
        .collect();
    let mm = mem(m);
    mm.nearest_players.set_list(players);
    mm.nearest_visible_player.set_opt(visible.first().copied());
    mm.nearest_visible_targetable_players
        .set_list(attackable.clone());
    mm.nearest_visible_targetable_player
        .set_opt(attackable.first().copied());
}
/// `BreezeAttackEntitySensor`: the nearest living entity (not a creative or
/// spectator player) it may attack.
fn sense_breeze_attack(m: &mut Mob, w: &MobWorld) {
    let list = mem_ref(m).mobs.value().unwrap_or_default();
    let found = list.into_iter().find(|&who| {
        let shown = match who {
            Who::Player => w
                .player
                .as_ref()
                .is_some_and(|p| !p.creative && !p.spectator),
            Who::Mob(_) => true,
        };
        shown && attackable(m, w, who)
    });
    mem(m).nearest_attackable.set_opt(found);
}

// ---- Behaviours ---------------------------------------------------------

/// `StartAttacking`'s target finders.
#[derive(Clone, Copy, Debug)]
enum Finder {
    /// The breeze's `NEAREST_ATTACKABLE`.
    NearestAttackable,
    /// The breeze's `HURT_BY` source, if living.
    HurtBy,
    /// The creaking's `NEAREST_VISIBLE_ATTACKABLE_PLAYER`, while active.
    ActiveCreaking,
}

/// The behaviours of both brains.
#[derive(Clone, Debug)]
enum B {
    Swim {
        t: Timed,
        creaking: bool,
    },
    LookAtTargetSink(Timed),
    /// `MoveToTargetSink`, and the breeze's `SlideToTargetSink`.
    MoveToTargetSink {
        t: Timed,
        cooldown: i32,
        path: Option<Path>,
        last_target: Option<[i32; 3]>,
        speed: f32,
        slide: bool,
    },
    StartAttacking(bool, Finder),
    /// `StopAttackingIfTargetInvalid`: the breeze's while the target was
    /// attackable within the last 100 ticks (`positives_left`), the
    /// creaking's while a visible attackable player.
    StopAttacking {
        running: bool,
        breeze: bool,
        positives_left: i32,
    },
    RunOne(Box<RunOne>),
    DoNothing(Timed),
    RandomStroll(bool, f32),
    SetEntityLookTargetSometimes {
        running: bool,
        until_next: i32,
    },
    SetWalkTargetFromLookTarget(bool),
    SetWalkTargetFromAttackTarget(bool),
    MeleeAttack(bool),
    Shoot(Timed),
    LongJump(Timed),
    ShootWhenStuck(Timed),
    Slide(Timed),
}

/// `RunOne`: shuffled by weight, the first that starts runs.
#[derive(Clone, Debug)]
struct RunOne {
    running: bool,
    random: LegacyRandom,
    kids: Vec<(B, i32, f64)>,
}
impl RunOne {
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        self.running = true;
        // ShufflingList.shuffle: a fresh weight for each entry in the current
        // order, then a stable sort.
        for k in &mut self.kids {
            let r = self.random.next_float();
            k.2 = -horizon_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
        }
        self.kids.sort_by(|a, b| a.2.total_cmp(&b.2));
        for k in &mut self.kids {
            if !k.0.running() && k.0.try_start(m, cx) {
                break;
            }
        }
        true
    }
    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        for k in &mut self.kids {
            if k.0.running() {
                k.0.tick_or_stop(m, cx);
            }
        }
        if !self.kids.iter().any(|k| k.0.running()) {
            self.do_stop(m, cx);
        }
    }
    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        self.running = false;
        for k in &mut self.kids {
            if k.0.running() {
                k.0.do_stop(m, cx);
            }
        }
    }
}

impl B {
    fn running(&self) -> bool {
        match self {
            B::Swim { t, .. }
            | B::LookAtTargetSink(t)
            | B::MoveToTargetSink { t, .. }
            | B::DoNothing(t)
            | B::Shoot(t)
            | B::LongJump(t)
            | B::ShootWhenStuck(t)
            | B::Slide(t) => t.running,
            B::StartAttacking(r, _)
            | B::StopAttacking { running: r, .. }
            | B::RandomStroll(r, _)
            | B::SetEntityLookTargetSometimes { running: r, .. }
            | B::SetWalkTargetFromLookTarget(r)
            | B::SetWalkTargetFromAttackTarget(r)
            | B::MeleeAttack(r) => *r,
            B::RunOne(g) => g.running,
        }
    }

    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        match self {
            B::RunOne(g) => g.try_start(m, cx),
            B::Swim { t, creaking } => {
                if *creaking && !super::creaking::can_move(m) || !swim_condition(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::LookAtTargetSink(t) => {
                if !mem(m).look_target.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::MoveToTargetSink {
                t,
                cooldown,
                path,
                last_target,
                speed,
                slide,
            } => {
                let mm = mem(m);
                if mm.path.present() || !mm.walk_target.present() {
                    return false;
                }
                // checkExtraStartConditions
                if *cooldown > 0 {
                    *cooldown -= 1;
                    return false;
                }
                let walk = mm.walk_target.value().unwrap();
                let reached = reached(m, cx.w, &walk);
                if !reached && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach) {
                    *last_target = walk.target.block(cx.w);
                    t.begin(cx.lr, cx.now);
                    // start
                    mem(m).path.set_opt(path.clone());
                    m.nav_move_to_path(path.clone(), f64::from(*speed));
                    if *slide {
                        // SlideToTargetSink.start
                        let at = m.center();
                        cx.events
                            .push(MobEvent::Sound("entity.breeze.slide", at, 1.0, 1.0));
                        super::breeze::set_pose(m, super::breeze::SLIDING);
                    }
                    return true;
                }
                let mm = mem(m);
                mm.walk_target.clear();
                if reached {
                    mm.cant_reach_walk_target_since.clear();
                }
                false
            }
            B::StartAttacking(running, finder) => {
                if mem(m).attack_target.present() {
                    return false;
                }
                let target = match finder {
                    Finder::NearestAttackable => mem(m).nearest_attackable.value(),
                    Finder::HurtBy => hurt_by_entity(m, cx.w),
                    Finder::ActiveCreaking => {
                        if !super::creaking::is_active(m) {
                            return false;
                        }
                        mem(m).nearest_visible_targetable_player.value()
                    }
                };
                let Some(target) = target else { return false };
                if !can_attack(m, cx.w, target) {
                    return false;
                }
                set_attack_target(m, Some(target));
                mem(m).cant_reach_walk_target_since.clear();
                *running = true;
                true
            }
            B::StopAttacking {
                running,
                breeze,
                positives_left,
            } => {
                let Some(target) = mem(m).attack_target.value() else {
                    return false;
                };
                let alive = cx.w.target_info(target).is_some_and(|t| t.alive);
                let tired = mem(m)
                    .cant_reach_walk_target_since
                    .value()
                    .is_some_and(|since| cx.now - since > 200);
                let stop_when = if *breeze {
                    // Sensor.wasEntityAttackableLastNTicks(100).negate()
                    if attackable(m, cx.w, target) {
                        *positives_left = 100;
                        false
                    } else {
                        *positives_left -= 1;
                        *positives_left < 0
                    }
                } else {
                    // CreakingAi.isAttackTargetStillReachable
                    let reachable = target == Who::Player
                        && mem(m)
                            .nearest_visible_targetable_players
                            .get()
                            .is_some_and(|l| l.contains(&Who::Player));
                    !reachable
                };
                if !can_attack(m, cx.w, target) || tired || !alive || stop_when {
                    set_attack_target(m, None);
                }
                *running = true;
                true
            }
            B::DoNothing(t) => {
                t.begin(cx.lr, cx.now);
                true
            }
            B::RandomStroll(running, speed) => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let pos = m.land_random_pos(cx.w, 10, 7);
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, *speed, 0)));
                *running = true;
                true
            }
            B::SetEntityLookTargetSometimes {
                running,
                until_next,
            } => {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    w.target_info(who)
                        .is_some_and(|t| t.position.distance_squared(m.position) <= 64.0)
                });
                let Some(t) = found else { return false };
                // Ticker.tickDownAndCheck with UniformInt.of(30, 60).
                if *until_next == 0 {
                    *until_next = 30 + cx.lr.next_int_bound(31).unwrap_or(0) - 1;
                    return false;
                }
                *until_next -= 1;
                if *until_next != 0 {
                    return false;
                }
                mem(m).look_target.set(Tracker::Entity(t, true));
                *running = true;
                true
            }
            B::SetWalkTargetFromLookTarget(running) => {
                let mm = mem(m);
                if mm.walk_target.present() {
                    return false;
                }
                let Some(look) = mm.look_target.value() else {
                    return false;
                };
                mm.walk_target.set(WalkTarget {
                    target: look,
                    speed: 0.3,
                    close: 3,
                });
                *running = true;
                true
            }
            B::SetWalkTargetFromAttackTarget(running) => {
                let Some(target) = mem(m).attack_target.value() else {
                    return false;
                };
                let w = cx.w;
                let seen = mem_ref(m).visible_mobs.present() && visible_contains(m, w, target);
                if seen && within_attack_range(m, w, target) {
                    mem(m).walk_target.clear();
                } else {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Entity(target, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(target, false),
                        speed: 1.0,
                        close: 0,
                    });
                }
                *running = true;
                true
            }
            B::MeleeAttack(running) => {
                let mm = mem(m);
                let Some(target) = mm.attack_target.value() else {
                    return false;
                };
                if mm.attack_cooling_down.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let w = cx.w;
                let in_reach = w
                    .target_info(target)
                    .is_some_and(|t| m.within_melee_range(&t));
                if !super::creaking::can_move(m) || !in_reach || !visible_contains(m, w, target) {
                    return false;
                }
                mem(m).look_target.set(Tracker::Entity(target, true));
                m.swing();
                let mut gcx = Cx {
                    w,
                    lr: cx.lr,
                    events: cx.events,
                };
                super::ai::do_hurt_target(m, target, &mut gcx);
                mem(m).attack_cooling_down.set_ttl(true, 40);
                *running = true;
                true
            }
            B::Shoot(t) => {
                let mm = mem(m);
                if !mm.attack_target.present()
                    || mm.breeze_shoot_cooldown.present()
                    || mm.breeze_shoot_charging.present()
                    || mm.breeze_shoot_recovering.present()
                    || !mm.breeze_shoot.present()
                    || mm.walk_target.present()
                    || mm.breeze_jump_target.present()
                {
                    return false;
                }
                // checkExtraStartConditions
                if super::breeze::pose(m) != super::breeze::STANDING {
                    return false;
                }
                let Some(target) = mem(m).attack_target.value() else {
                    return false;
                };
                let within =
                    cx.w.target_info(target)
                        .is_some_and(|t| m.position.distance_squared(t.position) < 256.0);
                if !within {
                    mem(m).breeze_shoot.clear();
                    return false;
                }
                t.begin(cx.lr, cx.now);
                // start
                super::breeze::set_pose(m, super::breeze::SHOOTING);
                mem(m).breeze_shoot_charging.set_ttl((), 15);
                breeze_sound(m, cx, "entity.breeze.inhale", 1.0);
                true
            }
            B::LongJump(t) => {
                let mm = mem(m);
                if !mm.attack_target.present()
                    || mm.breeze_jump_cooldown.present()
                    || mm.breeze_shoot.present()
                    || mm.walk_target.present()
                {
                    return false;
                }
                if !long_jump_can_run(m, cx) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                // start
                if !mem(m).breeze_jump_inhaling.present() {
                    mem(m).breeze_jump_inhaling.set_ttl((), 10);
                }
                super::breeze::set_pose(m, super::breeze::INHALING);
                // level.playSound(null, breeze, BREEZE_CHARGE, HOSTILE, 1, 1)
                let at = [m.position.x, m.position.y, m.position.z];
                cx.events
                    .push(MobEvent::Sound("entity.breeze.charge", at, 1.0, 1.0));
                if let Some(p) = mem(m).breeze_jump_target.value() {
                    look_at_eyes(
                        m,
                        [
                            f64::from(p[0]) + 0.5,
                            f64::from(p[1]) + 0.5,
                            f64::from(p[2]) + 0.5,
                        ],
                    );
                }
                true
            }
            B::ShootWhenStuck(t) => {
                let mm = mem(m);
                if !mm.attack_target.present()
                    || mm.breeze_jump_inhaling.present()
                    || mm.breeze_jump_target.present()
                    || mm.walk_target.present()
                    || mm.breeze_shoot.present()
                {
                    return false;
                }
                let stuck = m.extra.mount.is_some()
                    || m.fluids.in_water
                    || m.has_effect("minecraft:levitation");
                if !stuck {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                mem(m).breeze_shoot.set_ttl((), 60);
                true
            }
            B::Slide(t) => {
                let mm = mem(m);
                if !mm.attack_target.present()
                    || mm.walk_target.present()
                    || mm.breeze_jump_cooldown.present()
                    || mm.breeze_shoot.present()
                {
                    return false;
                }
                if !m.on_ground
                    || m.fluids.in_water
                    || super::breeze::pose(m) != super::breeze::STANDING
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                slide_start(m, cx);
                true
            }
        }
    }

    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        let now = cx.now;
        match self {
            B::RunOne(g) => g.tick_or_stop(m, cx),
            // OneShots stop in the tick they started.
            B::StartAttacking(r, _)
            | B::StopAttacking { running: r, .. }
            | B::RandomStroll(r, _)
            | B::SetEntityLookTargetSometimes { running: r, .. }
            | B::SetWalkTargetFromLookTarget(r)
            | B::SetWalkTargetFromAttackTarget(r)
            | B::MeleeAttack(r) => *r = false,
            B::Swim { t, .. } => {
                if now > t.end || !swim_condition(m) {
                    self.do_stop(m, cx);
                } else if m.random.next_float() < 0.8 {
                    m.ai.jump = true;
                }
            }
            B::LookAtTargetSink(t) => {
                let visible = match mem(m).look_target.value() {
                    Some(look) => tracker_visible(m, cx.w, look),
                    None => false,
                };
                if now > t.end || !visible {
                    self.do_stop(m, cx);
                } else if let Some(at) = mem(m).look_target.value().and_then(|l| l.position(cx.w)) {
                    m.look_at(at);
                }
            }
            B::MoveToTargetSink {
                t,
                path,
                last_target,
                speed,
                ..
            } => {
                let still = path.is_some()
                    && last_target.is_some()
                    && mem(m)
                        .walk_target
                        .value()
                        .is_some_and(|walk| !m.ai.nav.is_done() && !reached(m, cx.w, &walk));
                if now > t.end || !still {
                    self.do_stop(m, cx);
                    return;
                }
                let new_path = m.ai.nav.path.clone();
                if !same_path(path.as_ref(), new_path.as_ref()) {
                    *path = new_path.clone();
                    mem(m).path.set_opt(new_path.clone());
                }
                if new_path.is_some()
                    && let Some(last) = *last_target
                    && let Some(walk) = mem(m).walk_target.value()
                    && let Some(now_target) = walk.target.block(cx.w)
                {
                    let d = manhattan_sq(now_target, last);
                    if d > 4 && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach) {
                        *last_target = Some(now_target);
                        mem(m).path.set_opt(path.clone());
                        m.nav_move_to_path(path.clone(), f64::from(*speed));
                    }
                }
            }
            B::DoNothing(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::Shoot(t) => {
                let mm = mem(m);
                if now > t.end || !mm.attack_target.present() || !mm.breeze_shoot.present() {
                    self.do_stop(m, cx);
                    return;
                }
                shoot_tick(m, cx);
            }
            B::LongJump(t) => {
                let still = super::breeze::pose(m) != super::breeze::STANDING
                    && !mem(m).breeze_jump_cooldown.present();
                if now > t.end || !still {
                    self.do_stop(m, cx);
                    return;
                }
                long_jump_tick(m, cx);
            }
            // canStillUse is false: stops at once.
            B::ShootWhenStuck(_) | B::Slide(_) => self.do_stop(m, cx),
        }
    }

    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        match self {
            B::RunOne(g) => g.do_stop(m, cx),
            B::StartAttacking(r, _)
            | B::StopAttacking { running: r, .. }
            | B::RandomStroll(r, _)
            | B::SetEntityLookTargetSometimes { running: r, .. }
            | B::SetWalkTargetFromLookTarget(r)
            | B::SetWalkTargetFromAttackTarget(r)
            | B::MeleeAttack(r) => *r = false,
            B::Swim { t, .. } | B::DoNothing(t) | B::ShootWhenStuck(t) | B::Slide(t) => {
                t.running = false
            }
            B::LookAtTargetSink(t) => {
                t.running = false;
                mem(m).look_target.clear();
            }
            B::MoveToTargetSink {
                t,
                cooldown,
                path,
                slide,
                ..
            } => {
                t.running = false;
                if let Some(walk) = mem(m).walk_target.value()
                    && !reached(m, cx.w, &walk)
                    && m.ai.nav.stuck
                {
                    *cooldown = cx.lr.next_int_bound(40).unwrap_or(0);
                }
                m.nav_stop();
                let mm = mem(m);
                mm.walk_target.clear();
                mm.path.clear();
                *path = None;
                if *slide {
                    // SlideToTargetSink.stop
                    super::breeze::set_pose(m, super::breeze::STANDING);
                    if mem(m).attack_target.present() {
                        mem(m).breeze_shoot.set_ttl((), 60);
                    }
                }
            }
            B::Shoot(t) => {
                t.running = false;
                if super::breeze::pose(m) == super::breeze::SHOOTING {
                    super::breeze::set_pose(m, super::breeze::STANDING);
                }
                let mm = mem(m);
                mm.breeze_shoot_cooldown.set_ttl((), 10);
                mm.breeze_shoot.clear();
            }
            B::LongJump(t) => {
                t.running = false;
                let pose = super::breeze::pose(m);
                if pose == super::breeze::LONG_JUMPING || pose == super::breeze::INHALING {
                    super::breeze::set_pose(m, super::breeze::STANDING);
                }
                let mm = mem(m);
                mm.breeze_jump_target.clear();
                mm.breeze_jump_inhaling.clear();
                mm.breeze_leaving_water.clear();
            }
        }
    }
}

fn manhattan_sq(a: [i32; 3], b: [i32; 3]) -> i64 {
    let d = [
        i64::from(a[0] - b[0]),
        i64::from(a[1] - b[1]),
        i64::from(a[2] - b[2]),
    ];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}
fn same_path(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.nodes == b.nodes && a.target == b.target,
        _ => false,
    }
}
fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
/// `Swim`: deeper than the jump threshold, or in lava.
fn swim_condition(m: &Mob) -> bool {
    m.fluids.water > m.fluid_jump_threshold() || m.fluids.in_lava
}
/// `PositionTracker.isVisibleBy`.
fn tracker_visible(m: &mut Mob, w: &MobWorld, t: Tracker) -> bool {
    match t {
        Tracker::Entity(who, _) => {
            w.target_info(who).is_some_and(|i| i.alive) && visible_contains(m, w, who)
        }
        _ => true,
    }
}
/// `BehaviorUtils.isWithinAttackRange(body, target, 1)` for a mob without a
/// ranged weapon: `Mob.isWithinMeleeAttackRange`.
fn within_attack_range(m: &Mob, w: &MobWorld, target: Who) -> bool {
    w.target_info(target)
        .is_some_and(|t| m.within_melee_range(&t))
}

// ---- The breeze's behaviours --------------------------------------------

/// `Mob.playSound(sound, volume, 1)`: at the mob, no pitch drawn.
fn breeze_sound(m: &Mob, cx: &mut Bx, sound: &'static str, volume: f32) {
    let at = [m.position.x, m.position.y, m.position.z];
    cx.events.push(MobEvent::Sound(sound, at, volume, 1.0));
}
/// `Entity.lookAt(EntityAnchorArgument.Anchor.EYES, pos)`.
pub fn look_at_eyes(m: &mut Mob, at: [f64; 3]) {
    let (xd, yd, zd) = (
        at[0] - m.position.x,
        at[1] - m.eye_y(),
        at[2] - m.position.z,
    );
    let sd = (xd * xd + zd * zd).sqrt();
    // The compiled code multiplies by the folded `Mth.RAD_TO_DEG`.
    let deg = f64::from(trig::RAD_TO_DEG);
    m.head_pitch = wrap_degrees((-(trig::atan2(yd, sd) * deg)) as f32);
    m.yaw = wrap_degrees((trig::atan2(zd, xd) * deg) as f32 - 90.0);
    m.head_yaw = m.yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_yaw = m.yaw;
    // LivingEntity.lookAt: the body follows the head.
    m.previous_head_yaw = m.head_yaw;
    m.body_yaw = m.head_yaw;
    m.previous_body_yaw = m.body_yaw;
}
/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(angle: f32) -> f32 {
    let mut w = angle % 360.0;
    if w >= 180.0 {
        w -= 360.0;
    }
    if w < -180.0 {
        w += 360.0;
    }
    w
}
/// The attack target's position, eye height, facing and height.
struct Quarry {
    position: Vec3,
    head_yaw: f32,
    height: f64,
    passenger: bool,
}
fn quarry(w: &MobWorld, who: Who) -> Option<Quarry> {
    let t = w.target_info(who)?;
    let head_yaw = match who {
        Who::Player => w.player.as_ref().map_or(0.0, |p| p.yaw),
        Who::Mob(id) => w.seen(id).map_or(0.0, |s| s.yaw),
    };
    Some(Quarry {
        position: t.position,
        head_yaw,
        height: t.bounds.max_position().y - t.bounds.min_position().y,
        passenger: t.passenger,
    })
}
/// `BreezeUtil.randomPointBehindTarget`.
fn point_behind(m: &mut Mob, q: &Quarry) -> Vec3 {
    let view = q.head_yaw + 180.0 + m.random.next_gaussian() as f32 * 90.0 / 2.0;
    let f = m.random.next_float();
    let r = 4.0 + f * (8.0 - 4.0);
    let d = Vec3::direction_from_rotation(0.0, view).scale(f64::from(r));
    q.position.add_vector(d)
}
/// `BreezeUtil.hasLineOfSight`.
fn line_of_sight(m: &Mob, w: &MobWorld, target: Vec3) -> bool {
    let from = m.position;
    let range = 50.0_f64.max(m.attrs.follow_range);
    target.distance(from) <= range && w.clear_between(from, target, false)
}
/// `Slide.start`.
fn slide_start(m: &mut Mob, cx: &mut Bx) {
    let Some(target) = mem(m).attack_target.value() else {
        return;
    };
    let Some(q) = quarry(cx.w, target) else {
        return;
    };
    // Breeze.withinInnerCircleRange
    let centre = Vec3::new(
        m.position.x.floor() + 0.5,
        m.position.y.floor() + 0.5,
        m.position.z.floor() + 0.5,
    );
    let (dx, dy, dz) = (
        q.position.x - centre.x,
        q.position.y - centre.y,
        q.position.z - centre.z,
    );
    let inner = dx * dx + dz * dz < 16.0 && dy.abs() < 10.0;
    let mut position: Option<Vec3> = None;
    if inner
        && let Some(away) =
            m.default_random_pos_away(cx.w, 5, 5, [q.position.x, q.position.y, q.position.z])
    {
        let away = Vec3::new(away[0], away[1], away[2]);
        if line_of_sight(m, cx.w, away)
            && q.position.distance_squared(away) > q.position.distance_squared(m.position)
        {
            position = Some(away);
        }
    }
    let position = match position {
        Some(p) => p,
        None => {
            if m.random.next_boolean() {
                point_behind(m, &q)
            } else {
                // randomPointInMiddleCircle
                let d = q.position.subtract(m.position);
                let a = m.random.next_double();
                let distance = d.length() - (8.0 + a * (4.0 - 8.0));
                let n = d.normalize();
                m.position
                    .add_vector(Vec3::new(n.x * distance, n.y * distance, n.z * distance))
            }
        }
    };
    mem(m)
        .walk_target
        .set(WalkTarget::block(block_of(position), 0.6, 1));
}
/// `Shoot.tick`.
fn shoot_tick(m: &mut Mob, cx: &mut Bx) {
    let Some(target) = mem(m).attack_target.value() else {
        return;
    };
    let Some(q) = quarry(cx.w, target) else {
        return;
    };
    look_at_eyes(m, [q.position.x, q.position.y, q.position.z]);
    let mm = mem(m);
    if mm.breeze_shoot_charging.present() || mm.breeze_shoot_recovering.present() {
        return;
    }
    mm.breeze_shoot_recovering.set_ttl((), 4);
    let firing = super::breeze::firing_y(m);
    let (xd, yd, zd) = (
        q.position.x - m.position.x,
        // target.getY(target.isPassenger() ? 0.8 : 0.3).
        q.position.y + q.height * if q.passenger { 0.8 } else { 0.3 } - firing,
        q.position.z - m.position.z,
    );
    let start = Vec3::new(m.position.x, firing, m.position.z);
    let mut p = Projectile::new(
        ProjectileKind::BreezeWindCharge,
        start,
        Some(Who::Mob(m.id)),
    );
    // The new entity's own random spreads the shot (5 - difficulty id * 4):
    // not the breeze's.
    let mut own = LegacyRandom::new(
        cx.now
            .wrapping_mul(0x5DEE_CE66)
            .wrapping_add(i64::from(m.id)),
    );
    p.shoot(
        &mut own,
        [xd, yd, zd],
        0.7,
        5.0 - f32::from(cx.w.difficulty.id) * 4.0,
    );
    cx.events.push(MobEvent::Projectile(p));
    breeze_sound(m, cx, "entity.breeze.shoot", 1.5);
}
/// `LongJump.canRun`.
fn long_jump_can_run(m: &mut Mob, cx: &mut Bx) -> bool {
    if !m.on_ground && !m.fluids.in_water {
        return false;
    }
    if m.fluids.water > m.fluid_jump_threshold() || m.fluids.in_lava {
        return false;
    }
    if mem(m).breeze_jump_target.present() {
        return true;
    }
    let Some(target) = mem(m).attack_target.value() else {
        return false;
    };
    let Some(q) = quarry(cx.w, target) else {
        return false;
    };
    // outOfAggroRange: !target.closerThan(breeze, follow range)
    let follow = m.attrs.follow_range;
    if q.position.distance_squared(m.position) >= follow * follow {
        set_attack_target(m, None);
        return false;
    }
    // tooCloseForJump: (float) distance - 4 <= 0
    if q.position.distance(m.position) as f32 - 4.0 <= 0.0 {
        return false;
    }
    if !super::breeze::can_jump_from_here(m, cx.w) {
        return false;
    }
    let behind = point_behind(m, &q);
    let Some(target_pos) = super::breeze::snap_to_surface(cx.w, behind) else {
        return false;
    };
    let below =
        cx.w.block_name(target_pos[0], target_pos[1] - 1, target_pos[2]);
    if super::breeze::dangerous_block(below) {
        return false;
    }
    let centre = Vec3::new(
        f64::from(target_pos[0]) + 0.5,
        f64::from(target_pos[1]) + 0.5,
        f64::from(target_pos[2]) + 0.5,
    );
    if !line_of_sight(m, cx.w, centre)
        && !line_of_sight(m, cx.w, centre.add_vector(Vec3::new(0.0, 4.0, 0.0)))
    {
        return false;
    }
    mem(m).breeze_jump_target.set(target_pos);
    true
}
/// `LongJump.tick`.
fn long_jump_tick(m: &mut Mob, cx: &mut Bx) {
    let in_water = m.fluids.in_water;
    if !in_water && mem(m).breeze_leaving_water.present() {
        mem(m).breeze_leaving_water.clear();
    }
    let pose = super::breeze::pose(m);
    let finished_inhaling =
        !mem(m).breeze_jump_inhaling.present() && pose == super::breeze::INHALING;
    if finished_inhaling {
        let target = mem(m).breeze_jump_target.value();
        let v = target.and_then(|t| {
            super::breeze::optimal_jump_vector(
                m,
                Vec3::new(
                    f64::from(t[0]) + 0.5,
                    f64::from(t[1]),
                    f64::from(t[2]) + 0.5,
                ),
            )
        });
        let Some(v) = v else {
            super::breeze::set_pose(m, super::breeze::STANDING);
            return;
        };
        if in_water {
            mem(m).breeze_leaving_water.set(());
        }
        breeze_sound(m, cx, "entity.breeze.jump", 1.0);
        super::breeze::set_pose(m, super::breeze::LONG_JUMPING);
        m.yaw = m.body_yaw;
        m.velocity = v;
        return;
    }
    let landed_in_water = m.fluids.in_water && !mem(m).breeze_leaving_water.present();
    if pose == super::breeze::LONG_JUMPING && (m.on_ground || landed_in_water) {
        breeze_sound(m, cx, "entity.breeze.land", 1.0);
        super::breeze::set_pose(m, super::breeze::STANDING);
        let hurt = mem(m).hurt_by.present();
        let mm = mem(m);
        mm.breeze_jump_cooldown
            .set_ttl((), if hurt { 2 } else { 10 });
        mm.breeze_shoot.set_ttl((), 100);
    }
}

// ---- The packages -------------------------------------------------------

fn timed(min: i32, max: i32) -> Timed {
    Timed::new(min, max)
}
/// `availableBehaviorsByPriority`'s order: priority, then the activity's
/// hash slot (core 1, idle 5, fight 14), then insertion order.
fn sorted(mut v: Vec<(i32, Act, B)>) -> Vec<Entry> {
    let slot = |a: Act| match a {
        Act::Core => 1,
        Act::Idle => 5,
        Act::Fight => 14,
    };
    v.sort_by_key(|(p, a, _)| (*p, slot(*a)));
    v.into_iter().map(|(_, act, b)| Entry { act, b }).collect()
}
fn run_one(seed: i64, kids: Vec<(B, i32)>) -> B {
    B::RunOne(Box::new(RunOne {
        running: false,
        random: LegacyRandom::new(seed),
        kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
    }))
}
fn move_sink(min: i32, max: i32, slide: bool) -> B {
    B::MoveToTargetSink {
        t: timed(min, max),
        cooldown: 0,
        path: None,
        last_target: None,
        speed: 0.0,
        slide,
    }
}
/// `BreezeAi.getActivities`.
fn breeze_packages(seed: i64) -> Vec<Entry> {
    use Act::*;
    sorted(vec![
        (
            0,
            Core,
            B::Swim {
                t: timed(60, 60),
                creaking: false,
            },
        ),
        (1, Core, B::LookAtTargetSink(timed(45, 90))),
        (0, Idle, B::StartAttacking(false, Finder::NearestAttackable)),
        (1, Idle, B::StartAttacking(false, Finder::HurtBy)),
        (2, Idle, move_sink(20, 40, true)),
        (
            3,
            Idle,
            run_one(
                seed,
                vec![
                    (B::DoNothing(timed(20, 100)), 1),
                    (B::RandomStroll(false, 0.6), 2),
                ],
            ),
        ),
        (
            0,
            Fight,
            B::StopAttacking {
                running: false,
                breeze: true,
                positives_left: 0,
            },
        ),
        (1, Fight, B::Shoot(timed(20, 20))),
        (2, Fight, B::LongJump(timed(200, 200))),
        (3, Fight, B::ShootWhenStuck(timed(60, 60))),
        (4, Fight, B::Slide(timed(60, 60))),
    ])
}
/// `CreakingAi.getActivities`.
fn creaking_packages(seed: i64) -> Vec<Entry> {
    use Act::*;
    sorted(vec![
        (
            0,
            Core,
            B::Swim {
                t: timed(60, 60),
                creaking: true,
            },
        ),
        (1, Core, B::LookAtTargetSink(timed(45, 90))),
        (2, Core, move_sink(150, 250, false)),
        (10, Idle, B::StartAttacking(false, Finder::ActiveCreaking)),
        (
            11,
            Idle,
            B::SetEntityLookTargetSometimes {
                running: false,
                until_next: 0,
            },
        ),
        (
            12,
            Idle,
            run_one(
                seed,
                vec![
                    (B::RandomStroll(false, 0.3), 2),
                    (B::SetWalkTargetFromLookTarget(false), 2),
                    (B::DoNothing(timed(30, 60)), 1),
                ],
            ),
        ),
        (10, Fight, B::SetWalkTargetFromAttackTarget(false)),
        (11, Fight, B::MeleeAttack(false)),
        (
            12,
            Fight,
            B::StopAttacking {
                running: false,
                breeze: false,
                positives_left: 0,
            },
        ),
    ])
}

// ---- The probe's view ---------------------------------------------------

fn who(w: Who) -> String {
    match w {
        Who::Player => "player".into(),
        Who::Mob(id) => format!("e{id}"),
    }
}
fn pos(p: [i32; 3]) -> String {
    format!("{},{},{}", p[0], p[1], p[2])
}
fn tracker(t: Tracker) -> String {
    match t {
        Tracker::Entity(e, _) => who(e),
        Tracker::Block(p) => pos(p),
        Tracker::Point(v) => pos([
            v[0].floor() as i32,
            v[1].floor() as i32,
            v[2].floor() as i32,
        ]),
    }
}
/// The active activities and the memories with values, as the mob AI
/// oracle prints them.
pub fn describe(m: &Mob) -> Option<(String, String)> {
    let b = brain(m)?;
    let mut acts = ["core", b.activity.name()];
    acts.sort_unstable();
    let mm = &b.mem;
    let mut out: Vec<String> = Vec::new();
    let mut put = |name: &str, value: String, ttl: i64| {
        if ttl == NEVER {
            out.push(format!("{name}={value}"));
        } else {
            out.push(format!("{name}={value}@{ttl}"));
        }
    };
    macro_rules! slot {
        ($name:expr, $s:expr, $f:expr) => {
            if let Some(v) = $s.get() {
                put($name, $f(v), $s.ttl);
            }
        };
    }
    let unit = |_: &()| "u".to_string();
    let list = |l: &Vec<Who>| format!("n{}", l.len());
    slot!("attack_target", mm.attack_target, |e: &Who| who(*e));
    slot!("attack_cooling_down", mm.attack_cooling_down, |b: &bool| b
        .to_string());
    slot!(
        "cant_reach_walk_target_since",
        mm.cant_reach_walk_target_since,
        |t: &i64| t.to_string()
    );
    slot!("hurt_by", mm.hurt_by, |_: &i64| "DamageSource".to_string());
    slot!("hurt_by_entity", mm.hurt_by_entity, |e: &Who| who(*e));
    slot!("look_target", mm.look_target, |t: &Tracker| tracker(*t));
    slot!("mobs", mm.mobs, list);
    slot!("visible_mobs", mm.visible_mobs, |_: &Visible| {
        "NearestVisibleLivingEntities".to_string()
    });
    slot!("nearest_attackable", mm.nearest_attackable, |e: &Who| who(
        *e
    ));
    slot!("nearest_players", mm.nearest_players, list);
    slot!(
        "nearest_visible_player",
        mm.nearest_visible_player,
        |e: &Who| who(*e)
    );
    slot!(
        "nearest_visible_targetable_player",
        mm.nearest_visible_targetable_player,
        |e: &Who| who(*e)
    );
    slot!(
        "nearest_visible_targetable_players",
        mm.nearest_visible_targetable_players,
        list
    );
    slot!("path", mm.path, |p: &Path| format!("path{}", pos(p.target)));
    slot!("walk_target", mm.walk_target, |t: &WalkTarget| format!(
        "{}/{:x}/{}",
        tracker(t.target),
        t.speed.to_bits(),
        t.close
    ));
    slot!("breeze_shoot", mm.breeze_shoot, unit);
    slot!("breeze_shoot_charging", mm.breeze_shoot_charging, unit);
    slot!("breeze_shoot_recover", mm.breeze_shoot_recovering, unit);
    slot!("breeze_shoot_cooldown", mm.breeze_shoot_cooldown, unit);
    slot!("breeze_jump_cooldown", mm.breeze_jump_cooldown, unit);
    slot!("breeze_jump_inhaling", mm.breeze_jump_inhaling, unit);
    slot!(
        "breeze_jump_target",
        mm.breeze_jump_target,
        |p: &[i32; 3]| { pos(*p) }
    );
    slot!("breeze_leaving_water", mm.breeze_leaving_water, unit);
    out.sort();
    Some((acts.join("|"), out.join(" ")))
}
