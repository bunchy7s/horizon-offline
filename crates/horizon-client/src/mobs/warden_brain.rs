//! The warden's `Brain` (`Warden.BRAIN_PROVIDER` and `WardenAi`), ported
//! behaviour for behaviour on the villager brain's machinery (`Slot`
//! memories with their time to live, `Tracker`s, `WalkTarget`s and the
//! nearest-visible list): the two sensors and their scan phases, the core,
//! emerge, dig, idle, roar, fight, investigate and sniff activities, and their
//! behaviours started and ticked in vanilla's order (priority, then the
//! activity's slot in `Brain.availableBehaviorsByPriority`'s hash map, then
//! the order the activity lists them in).
//!
//! The level random (`ServerLevel.getRandom`) draws every `Behavior`'s
//! duration and the idle `DoNothing`, the sniff cooldown and the spot near a
//! disturbance; the warden's own random drives strolls, sounds and swimming.
//! `RunOne` shuffles with a random of its own, seeded with the brain's
//! shuffle seed.
use super::{
    Mob, MobEvent, MobWorld, Who,
    brain::Timed,
    goals::Cx,
    path::Path,
    villager_brain::{Slot, Tracker, Visible, WalkTarget, compute_path, reached},
    warden::{self, Pose},
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `WardenAi` timings.
pub const EMERGE_DURATION: i32 = 134;
pub const ROAR_DURATION: i32 = 84;
const SNIFFING_DURATION: i32 = 84;
const DIGGING_DURATION: i32 = 100;
pub const DIGGING_COOLDOWN: i64 = 1200;
const MELEE_ATTACK_COOLDOWN: i64 = 18;
const SONIC_DURATION: i32 = 60;
const SONIC_SOUND_DELAY: i64 = 34;
const ROAR_SOUND_DELAY: i64 = 25;
const DISTURBANCE_EXPIRY: i64 = 100;
/// `Warden.createAttributes`: the follow range the fight's look target uses.
const FOLLOW_RANGE: f64 = 24.0;

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Core,
    Emerge,
    Dig,
    Idle,
    Roar,
    Fight,
    Investigate,
    Sniff,
}
impl Act {
    pub fn name(self) -> &'static str {
        match self {
            Act::Core => "core",
            Act::Emerge => "emerge",
            Act::Dig => "dig",
            Act::Idle => "idle",
            Act::Roar => "roar",
            Act::Fight => "fight",
            Act::Investigate => "investigate",
            Act::Sniff => "sniff",
        }
    }
    /// The bucket of `HashMap<Activity, ...>` (16 buckets, the hash of the
    /// name); roar and investigate share one but never a priority.
    fn slot(self) -> u8 {
        match self {
            Act::Core => 1,
            Act::Dig => 3,
            Act::Idle => 5,
            Act::Sniff => 6,
            Act::Emerge => 10,
            Act::Roar | Act::Investigate => 11,
            Act::Fight => 14,
        }
    }
    /// Its place in `getActivities` (insertion order inside a bucket).
    fn order(self) -> u8 {
        self as u8
    }
}

/// The memories the warden's brain registers.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    pub nearest_players: Slot<Vec<Who>>,
    pub nearest_visible_player: Slot<Who>,
    pub nearest_visible_targetable_player: Slot<Who>,
    pub nearest_visible_targetable_players: Slot<Vec<Who>>,
    pub mobs: Slot<Vec<Who>>,
    pub visible_mobs: Slot<Visible>,
    pub nearest_attackable: Slot<Who>,
    pub look_target: Slot<Tracker>,
    pub walk_target: Slot<WalkTarget>,
    pub path: Slot<Path>,
    pub cant_reach_walk_target_since: Slot<i64>,
    pub attack_target: Slot<Who>,
    pub attack_cooling_down: Slot<bool>,
    pub roar_target: Slot<Who>,
    pub disturbance_location: Slot<[i32; 3]>,
    pub recent_projectile: Slot<()>,
    pub is_sniffing: Slot<()>,
    pub is_emerging: Slot<()>,
    pub roar_sound_delay: Slot<()>,
    pub dig_cooldown: Slot<()>,
    pub roar_sound_cooldown: Slot<()>,
    pub sniff_cooldown: Slot<()>,
    pub touch_cooldown: Slot<()>,
    pub vibration_cooldown: Slot<()>,
    pub sonic_boom_cooldown: Slot<()>,
    pub sonic_boom_sound_cooldown: Slot<()>,
    pub sonic_boom_sound_delay: Slot<()>,
}
impl Memories {
    fn tick(&mut self) {
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_targetable_player.tick();
        self.nearest_visible_targetable_players.tick();
        self.mobs.tick();
        self.visible_mobs.tick();
        self.nearest_attackable.tick();
        self.look_target.tick();
        self.walk_target.tick();
        self.path.tick();
        self.cant_reach_walk_target_since.tick();
        self.attack_target.tick();
        self.attack_cooling_down.tick();
        self.roar_target.tick();
        self.disturbance_location.tick();
        self.recent_projectile.tick();
        self.is_sniffing.tick();
        self.is_emerging.tick();
        self.roar_sound_delay.tick();
        self.dig_cooldown.tick();
        self.roar_sound_cooldown.tick();
        self.sniff_cooldown.tick();
        self.touch_cooldown.tick();
        self.vibration_cooldown.tick();
        self.sonic_boom_cooldown.tick();
        self.sonic_boom_sound_cooldown.tick();
        self.sonic_boom_sound_delay.tick();
    }
}

/// `RunOne`'s `ShufflingList` over a stroll (weight 2) and `DoNothing` (1).
#[derive(Clone, Debug)]
struct Gate {
    running: bool,
    random: LegacyRandom,
    kids: Vec<(B, i32, f64)>,
}

/// One behaviour of the warden's activities.
#[derive(Clone, Debug)]
enum B {
    Swim(Timed),
    SetWardenLookTarget(bool),
    LookAtTargetSink(Timed),
    MoveToTargetSink {
        t: Timed,
        cooldown: i32,
        path: Option<Path>,
        last_target: Option<[i32; 3]>,
        speed: f32,
    },
    Emerging(Timed),
    ForceUnmount(Timed),
    Digging(Timed),
    SetRoarTarget(bool),
    TryToSniff(bool),
    RunOne(Box<Gate>),
    Stroll(bool),
    DoNothing(Timed),
    GoToDisturbance(bool),
    Sniffing(Timed),
    Roar(Timed),
    DigCooldownSetter(bool),
    StopAttackingIfTargetInvalid(bool),
    SetEntityLookTarget(bool),
    SetWalkTargetFromAttackTarget(bool),
    SonicBoom(Timed),
    MeleeAttack(bool),
}

#[derive(Clone, Debug)]
struct Entry {
    act: Act,
    b: B,
}

/// The warden's brain.
#[derive(Clone, Debug)]
pub struct Brain {
    pub mem: Memories,
    /// `NEAREST_PLAYERS` and `WARDEN_ENTITY_SENSOR`: ticks to the next scan.
    sensors: [i64; 2],
    /// The active non-core activity (`CORE` is always active).
    pub activity: Act,
    list: Vec<Entry>,
    pub shuffle_seed: i64,
}
impl Default for Brain {
    fn default() -> Self {
        Self::new(0)
    }
}
impl Brain {
    pub fn new(shuffle_seed: i64) -> Self {
        Self {
            mem: Memories::default(),
            sensors: [0; 2],
            activity: Act::Idle,
            list: activities(shuffle_seed),
            shuffle_seed,
        }
    }
    pub fn is_active(&self, a: Act) -> bool {
        a == Act::Core || a == self.activity
    }
    fn requirements_met(&self, a: Act) -> bool {
        let m = &self.mem;
        match a {
            Act::Emerge => m.is_emerging.present(),
            Act::Dig => !m.roar_target.present() && !m.dig_cooldown.present(),
            Act::Roar => m.roar_target.present(),
            Act::Fight => m.attack_target.present(),
            Act::Investigate => m.disturbance_location.present(),
            Act::Sniff => m.is_sniffing.present(),
            Act::Core | Act::Idle => true,
        }
    }
    /// `setActiveActivity`: leaving an activity erases the memory it needed.
    fn set_active(&mut self, a: Act) {
        if self.is_active(a) {
            return;
        }
        let m = &mut self.mem;
        match self.activity {
            Act::Emerge => m.is_emerging.clear(),
            Act::Roar => m.roar_target.clear(),
            Act::Fight => m.attack_target.clear(),
            Act::Investigate => m.disturbance_location.clear(),
            Act::Sniff => m.is_sniffing.clear(),
            Act::Core | Act::Dig | Act::Idle => {}
        }
        self.activity = a;
    }
    /// `WardenAi.updateActivity`: `setActiveActivityToFirstValid`.
    pub fn update_activity(&mut self) {
        for a in [
            Act::Emerge,
            Act::Dig,
            Act::Roar,
            Act::Fight,
            Act::Investigate,
            Act::Sniff,
            Act::Idle,
        ] {
            if self.requirements_met(a) {
                self.set_active(a);
                break;
            }
        }
    }
    /// `Sensor.randomlyDelayStart` for both sensors, from the warden's random.
    pub fn delay_sensors(&mut self, random: &mut LegacyRandom) {
        for s in &mut self.sensors {
            *s = i64::from(random.next_int_bound(20).unwrap_or(0));
        }
    }
}

/// `WardenAi.getActivities`, in the order `Brain.startEachNonRunningBehavior`
/// meets the behaviours.
fn activities(seed: i64) -> Vec<Entry> {
    let gate = Gate {
        running: false,
        random: LegacyRandom::new(seed),
        kids: vec![
            (B::Stroll(false), 2, 0.0),
            (B::DoNothing(Timed::new(30, 60)), 1, 0.0),
        ],
    };
    let mut list = vec![
        (0, Act::Core, B::Swim(Timed::new(60, 60))),
        (1, Act::Core, B::SetWardenLookTarget(false)),
        (2, Act::Core, B::LookAtTargetSink(Timed::new(45, 90))),
        (
            3,
            Act::Core,
            B::MoveToTargetSink {
                t: Timed::new(150, 250),
                cooldown: 0,
                path: None,
                last_target: None,
                speed: 0.0,
            },
        ),
        (
            5,
            Act::Emerge,
            B::Emerging(Timed::new(EMERGE_DURATION, EMERGE_DURATION)),
        ),
        (0, Act::Dig, B::ForceUnmount(Timed::new(60, 60))),
        (
            1,
            Act::Dig,
            B::Digging(Timed::new(DIGGING_DURATION, DIGGING_DURATION)),
        ),
        (10, Act::Idle, B::SetRoarTarget(false)),
        (11, Act::Idle, B::TryToSniff(false)),
        (12, Act::Idle, B::RunOne(Box::new(gate))),
        (
            10,
            Act::Roar,
            B::Roar(Timed::new(ROAR_DURATION, ROAR_DURATION)),
        ),
        (10, Act::Fight, B::DigCooldownSetter(false)),
        (11, Act::Fight, B::StopAttackingIfTargetInvalid(false)),
        (12, Act::Fight, B::SetEntityLookTarget(false)),
        (13, Act::Fight, B::SetWalkTargetFromAttackTarget(false)),
        (
            14,
            Act::Fight,
            B::SonicBoom(Timed::new(SONIC_DURATION, SONIC_DURATION)),
        ),
        (15, Act::Fight, B::MeleeAttack(false)),
        (5, Act::Investigate, B::SetRoarTarget(false)),
        (6, Act::Investigate, B::GoToDisturbance(false)),
        (5, Act::Sniff, B::SetRoarTarget(false)),
        (
            6,
            Act::Sniff,
            B::Sniffing(Timed::new(SNIFFING_DURATION, SNIFFING_DURATION)),
        ),
    ];
    // A stable sort keeps each activity's own order.
    list.sort_by_key(|(prio, act, _)| (*prio, act.slot(), act.order()));
    list.into_iter()
        .map(|(_, act, b)| Entry { act, b })
        .collect()
}

/// What behaviours read and emit during a brain tick.
pub struct Bx<'a, 'w> {
    pub w: &'a MobWorld<'w>,
    pub lr: &'a mut LegacyRandom,
    pub events: &'a mut Vec<MobEvent>,
    pub now: i64,
}

fn brain(m: &Mob) -> Option<&Brain> {
    m.extra.warden.as_ref().map(|w| &w.brain)
}
pub(super) fn mem(m: &mut Mob) -> &mut Memories {
    &mut m.extra.warden.as_mut().expect("a warden").brain.mem
}
/// `CANT_REACH_WALK_TARGET_SINCE`, for `compute_path`.
fn cant_reach(m: &mut Mob) -> &mut Slot<i64> {
    &mut mem(m).cant_reach_walk_target_since
}
pub(super) fn mem_ref(m: &Mob) -> Option<&Memories> {
    brain(m).map(|b| &b.mem)
}

/// `Brain.tick` (from `Warden.customServerAiStep`).
pub fn tick(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if m.extra.warden.is_none() {
        return;
    }
    // forgetOutdatedMemories
    mem(m).tick();
    tick_sensors(m, w);
    let Some(st) = m.extra.warden.as_mut() else {
        return;
    };
    let mut list = std::mem::take(&mut st.brain.list);
    let mut cx = Bx {
        w,
        lr,
        events,
        now: w.game_time,
    };
    for e in &mut list {
        let active = m
            .extra
            .warden
            .as_ref()
            .is_some_and(|st| st.brain.is_active(e.act));
        if active && !e.b.running() {
            e.b.try_start(m, &mut cx);
        }
    }
    for e in &mut list {
        if e.b.running() {
            e.b.tick_or_stop(m, &mut cx);
        }
    }
    if let Some(st) = m.extra.warden.as_mut() {
        st.brain.list = list;
    }
}

// ---- Sensors ------------------------------------------------------------

fn tick_sensors(m: &mut Mob, w: &MobWorld) {
    for i in 0..2 {
        let fire = {
            let b = &mut m.extra.warden.as_mut().expect("a warden").brain;
            b.sensors[i] -= 1;
            if b.sensors[i] <= 0 {
                b.sensors[i] = 20;
                true
            } else {
                false
            }
        };
        if fire {
            if i == 0 {
                sense_players(m, w);
            } else {
                sense_entities(m, w);
            }
        }
    }
}
/// `TargetingConditions.forNonCombat().range(followRange)` (sensors use the
/// follow range of the mob that scanned last).
fn targetable(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => m.can_see_player(w, range),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive && m.distance_sqr(s.position) <= range.max(2.0).powi(2) && m.sees(w, who)
        }
    }
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
        .filter(|_| m.can_target_player(w, range, true))
        .collect();
    let mm = mem(m);
    mm.nearest_players.set_list(players);
    mm.nearest_visible_player.set_opt(visible.first().copied());
    mm.nearest_visible_targetable_players
        .set_list(attackable.clone());
    mm.nearest_visible_targetable_player
        .set_opt(attackable.first().copied());
}
/// `WardenEntitySensor`: `NearestLivingEntitySensor`, then the nearest
/// targetable player, else the nearest other targetable entity.
fn sense_entities(m: &mut Mob, w: &MobWorld) {
    let r = m.attrs.follow_range;
    let area = m.bounds().inflate(Vec3::new(r, r, r));
    let mut found: Vec<(Who, f64)> = Vec::new();
    for o in w.others_near(area) {
        if o.id != m.id && o.alive && o.bounds.intersects(area) {
            found.push((Who::Mob(o.id), m.distance_sqr(o.position)));
        }
    }
    if let Some(p) = &w.player
        && p.alive
        && p.bounds.intersects(area)
    {
        found.push((Who::Player, m.distance_sqr(p.position)));
    }
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    let list: Vec<Who> = found.into_iter().map(|(w, _)| w).collect();
    let attackable = list
        .iter()
        .copied()
        .find(|&e| e == Who::Player && warden::can_target(w, e))
        .or_else(|| {
            list.iter()
                .copied()
                .find(|&e| e != Who::Player && warden::can_target(w, e))
        });
    let mm = mem(m);
    mm.mobs.set_list(list.clone());
    mm.visible_mobs.set(Visible {
        list,
        cache: Vec::new(),
    });
    mm.nearest_attackable.set_opt(attackable);
}
/// The visible-entities memory's line-of-sight test, cached per memory.
fn visible_test(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    match mem_ref(m).and_then(|mm| mm.visible_mobs.get()) {
        Some(v) => {
            if let Some(&(_, r)) = v.cache.iter().find(|(k, _)| *k == who) {
                return r;
            }
        }
        None => return false,
    }
    // Sensor.isEntityTargetable: the attack target ignores invisibility.
    let r = targetable(m, w, who);
    if let Some(v) = mem(m).visible_mobs.get_mut() {
        v.cache.push((who, r));
    }
    r
}
/// `NearestVisibleLivingEntities.contains(entity)`.
fn visible_contains(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let listed = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
        .is_some_and(|v| v.list.contains(&who));
    listed && visible_test(m, w, who)
}
/// `NearestVisibleLivingEntities.findClosest`.
fn find_closest(m: &mut Mob, w: &MobWorld, filter: impl Fn(&Mob, Who) -> bool) -> Option<Who> {
    let list = mem_ref(m)?.visible_mobs.get()?.list.clone();
    list.into_iter()
        .find(|&who| filter(m, who) && visible_test(m, w, who))
}

// ---- Shared warden actions ------------------------------------------------

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> i64 {
    (0..3).map(|i| i64::from(a[i] - b[i]).pow(2)).sum()
}
/// `WardenAi.setDigCooldown`.
pub(super) fn set_dig_cooldown(m: &mut Mob) {
    let mm = mem(m);
    if mm.dig_cooldown.present() {
        mm.dig_cooldown.set_ttl((), DIGGING_COOLDOWN);
    }
}
/// `WardenAi.setDisturbanceLocation`.
pub(super) fn set_disturbance(m: &mut Mob, w: &MobWorld, position: [i32; 3]) {
    if warden::entity_angry_at(m, w).is_some() || mem(m).attack_target.present() {
        return;
    }
    set_dig_cooldown(m);
    let mm = mem(m);
    mm.sniff_cooldown.set_ttl((), DISTURBANCE_EXPIRY);
    mm.look_target
        .set_ttl(Tracker::Block(position), DISTURBANCE_EXPIRY);
    mm.disturbance_location
        .set_ttl(position, DISTURBANCE_EXPIRY);
    mm.walk_target.clear();
}
/// `Warden.setAttackTarget`.
pub(super) fn set_attack_target(m: &mut Mob, target: Who) {
    let mm = mem(m);
    mm.roar_target.clear();
    mm.attack_target.set(target);
    mm.cant_reach_walk_target_since.clear();
    mm.sonic_boom_cooldown.set_ttl((), 200);
}
/// `Entity.closerThan(target, horizontal, vertical)`.
fn closer_than(m: &Mob, p: Vec3, horizontal: f64, vertical: f64) -> bool {
    let (dx, dy, dz) = (p.x - m.position.x, p.y - m.position.y, p.z - m.position.z);
    dx * dx + dz * dz < horizontal * horizontal && dy * dy < vertical * vertical
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
fn same_path(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.nodes == b.nodes && a.target == b.target,
        _ => false,
    }
}
fn sound(m: &Mob, name: &'static str, volume: f32, pitch: f32, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(
        name,
        [m.position.x, m.position.y, m.position.z],
        volume,
        pitch,
    ));
}

// ---- Behaviours -----------------------------------------------------------

impl B {
    fn running(&self) -> bool {
        match self {
            B::Swim(t)
            | B::LookAtTargetSink(t)
            | B::Emerging(t)
            | B::ForceUnmount(t)
            | B::Digging(t)
            | B::DoNothing(t)
            | B::Sniffing(t)
            | B::Roar(t)
            | B::SonicBoom(t) => t.running,
            B::MoveToTargetSink { t, .. } => t.running,
            B::RunOne(g) => g.running,
            B::SetWardenLookTarget(r)
            | B::SetRoarTarget(r)
            | B::TryToSniff(r)
            | B::Stroll(r)
            | B::GoToDisturbance(r)
            | B::DigCooldownSetter(r)
            | B::StopAttackingIfTargetInvalid(r)
            | B::SetEntityLookTarget(r)
            | B::SetWalkTargetFromAttackTarget(r)
            | B::MeleeAttack(r) => *r,
        }
    }

    /// `BehaviorControl.tryStart`.
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        match self {
            B::Swim(t) => {
                if !swim_condition(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::SetWardenLookTarget(r) => one(r, || {
                let mm = mem(m);
                if mm.attack_target.present() {
                    return false;
                }
                let target = mm
                    .roar_target
                    .value()
                    .and_then(|who| cx.w.target_info(who))
                    .map(|t| block_of(t.position))
                    .or_else(|| mm.disturbance_location.value());
                let Some(p) = target else { return false };
                mem(m).look_target.set(Tracker::Block(p));
                true
            }),
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
                let Some(walk) = mm.walk_target.value() else {
                    return false;
                };
                let reached = reached(m, cx.w, &walk);
                if !reached && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach) {
                    *last_target = walk.target.block(cx.w);
                    t.begin(cx.lr, cx.now);
                    // start
                    mem(m).path.set_opt(path.clone());
                    m.nav_move_to_path(path.clone(), f64::from(*speed));
                    return true;
                }
                let mm = mem(m);
                mm.walk_target.clear();
                if reached {
                    mm.cant_reach_walk_target_since.clear();
                }
                false
            }
            B::Emerging(t) => {
                let mm = mem(m);
                if !mm.is_emerging.present() || mm.walk_target.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                warden::set_pose(m, Pose::Emerging);
                sound(m, "entity.warden.emerge", 5.0, 1.0, cx.events);
                true
            }
            // ForceUnmount: a warden never rides (Warden.canRide).
            B::ForceUnmount(_) => false,
            B::Digging(t) => {
                let mm = mem(m);
                if mm.attack_target.present() || mm.walk_target.present() {
                    return false;
                }
                let liquid = m.fluids.in_water || m.fluids.in_lava;
                if !(m.on_ground || liquid) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                if m.on_ground {
                    warden::set_pose(m, Pose::Digging);
                    sound(m, "entity.warden.dig", 5.0, 1.0, cx.events);
                } else {
                    sound(m, "entity.warden.agitated", 5.0, 1.0, cx.events);
                    // `stop` without `doStop`: discarded, still running.
                    m.removed = true;
                }
                true
            }
            B::SetRoarTarget(r) => one(r, || {
                let mm = mem(m);
                if mm.roar_target.present() || mm.attack_target.present() {
                    return false;
                }
                let Some(target) = warden::entity_angry_at(m, cx.w) else {
                    return false;
                };
                if !warden::can_target(cx.w, target) {
                    return false;
                }
                let mm = mem(m);
                mm.roar_target.set(target);
                mm.cant_reach_walk_target_since.clear();
                true
            }),
            B::TryToSniff(r) => one(r, || {
                let mm = mem(m);
                if mm.sniff_cooldown.present()
                    || !mm.nearest_attackable.present()
                    || mm.disturbance_location.present()
                {
                    return false;
                }
                mm.is_sniffing.set(());
                // UniformInt.of(100, 200).sample(level.getRandom())
                let cooldown = 100 + cx.lr.next_int_bound(101).unwrap_or(0);
                let mm = mem(m);
                mm.sniff_cooldown.set_ttl((), i64::from(cooldown));
                mm.walk_target.clear();
                warden::set_pose(m, Pose::Sniffing);
                true
            }),
            B::RunOne(g) => {
                if mem(m).is_sniffing.present() {
                    return false;
                }
                g.running = true;
                // ShufflingList.shuffle: a fresh weight for each entry in the
                // current order, then a stable sort.
                for k in &mut g.kids {
                    let r = g.random.next_float();
                    k.2 = -horizon_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
                }
                g.kids.sort_by(|a, b| a.2.total_cmp(&b.2));
                for k in &mut g.kids {
                    if !k.0.running() && k.0.try_start(m, cx) {
                        break;
                    }
                }
                true
            }
            B::Stroll(r) => one(r, || {
                if mem(m).walk_target.present() {
                    return false;
                }
                // RandomStroll.stroll(0.5): LandRandomPos.getPos(body, 10, 7).
                let target = m.land_random_pos(cx.w, 10, 7);
                mem(m).walk_target.set_opt(target.map(|p| WalkTarget {
                    target: Tracker::Block([
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ]),
                    speed: 0.5,
                    close: 0,
                }));
                true
            }),
            B::DoNothing(t) => {
                t.begin(cx.lr, cx.now);
                true
            }
            B::GoToDisturbance(r) => one(r, || {
                let mm = mem(m);
                if mm.attack_target.present() || mm.walk_target.present() {
                    return false;
                }
                let Some(p) = mm.disturbance_location.value() else {
                    return false;
                };
                // Vec3i.closerThan(body.blockPosition(), 2)
                if dist_sqr(p, block_of(m.position)) >= 4 {
                    // GoToTargetLocation.getNearbyPos: the level random.
                    let dx = cx.lr.next_int_bound(3).unwrap_or(0) - 1;
                    let dz = cx.lr.next_int_bound(3).unwrap_or(0) - 1;
                    let near = [p[0] + dx, p[1], p[2] + dz];
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Block(near));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Block(near),
                        speed: 0.7,
                        close: 2,
                    });
                }
                true
            }),
            B::Sniffing(t) => {
                let mm = mem(m);
                if !mm.is_sniffing.present()
                    || mm.attack_target.present()
                    || mm.walk_target.present()
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                sound(m, "entity.warden.sniff", 5.0, 1.0, cx.events);
                true
            }
            B::Roar(t) => {
                let mm = mem(m);
                if mm.attack_target.present() {
                    return false;
                }
                let Some(target) = mm.roar_target.value() else {
                    return false;
                };
                t.begin(cx.lr, cx.now);
                let mm = mem(m);
                mm.roar_sound_delay.set_ttl((), ROAR_SOUND_DELAY);
                mm.walk_target.clear();
                // BehaviorUtils.lookAtEntity
                mm.look_target.set(Tracker::Entity(target, true));
                warden::set_pose(m, Pose::Roaring);
                warden::increase_anger(m, cx.w, Some(target), 20, false, cx.events);
                true
            }
            B::DigCooldownSetter(r) => one(r, || {
                set_dig_cooldown(m);
                true
            }),
            B::StopAttackingIfTargetInvalid(r) => one(r, || {
                let Some(target) = mem(m).attack_target.value() else {
                    return false;
                };
                let alive = cx.w.target_info(target).is_some_and(|t| t.alive);
                let can = warden::can_target(cx.w, target);
                let angry = warden::anger_level(m).is_angry();
                if can && alive && angry {
                    return true;
                }
                // WardenAi.onTargetInvalid
                if !can && let Some(st) = m.extra.warden.as_mut() {
                    st.anger.clear(target);
                }
                set_dig_cooldown(m);
                mem(m).attack_target.clear();
                true
            }),
            B::SetEntityLookTarget(r) => one(r, || {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let target = mm.attack_target.value();
                let max = (FOLLOW_RANGE as f32 * FOLLOW_RANGE as f32) as f64;
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    Some(who) == target
                        && w.target_info(who)
                            .is_some_and(|t| m.distance_sqr(t.position) <= max)
                });
                let Some(found) = found else { return false };
                mem(m).look_target.set(Tracker::Entity(found, true));
                true
            }),
            B::SetWalkTargetFromAttackTarget(r) => one(r, || {
                let Some(target) = mem(m).attack_target.value() else {
                    return false;
                };
                let w = cx.w;
                let in_reach = mem(m).visible_mobs.present()
                    && visible_contains(m, w, target)
                    && w.target_info(target)
                        .is_some_and(|t| m.within_melee_range(&t));
                let mm = mem(m);
                if in_reach {
                    mm.walk_target.clear();
                } else {
                    mm.look_target.set(Tracker::Entity(target, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(target, false),
                        speed: 1.2,
                        close: 0,
                    });
                }
                true
            }),
            B::SonicBoom(t) => {
                let mm = mem(m);
                if mm.sonic_boom_cooldown.present() {
                    return false;
                }
                let Some(target) = mm.attack_target.value() else {
                    return false;
                };
                let close =
                    cx.w.target_info(target)
                        .is_some_and(|i| closer_than(m, i.position, 15.0, 20.0));
                if !close {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                let mm = mem(m);
                mm.attack_cooling_down
                    .set_ttl(true, i64::from(SONIC_DURATION));
                mm.sonic_boom_sound_delay.set_ttl((), SONIC_SOUND_DELAY);
                // Entity event 62: the sonic boom keyframes start.
                m.anim.states[warden::SONIC_BOOM].start(m.age as f32);
                cx.events.push(MobEvent::EntityEvent {
                    mob: m.id,
                    event: 62,
                    at: [m.position.x, m.position.y, m.position.z],
                });
                sound(m, "entity.warden.sonic_charge", 3.0, 1.0, cx.events);
                true
            }
            B::MeleeAttack(r) => one(r, || {
                let mm = mem(m);
                if mm.attack_cooling_down.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let Some(target) = mm.attack_target.value() else {
                    return false;
                };
                let w = cx.w;
                let Some(info) = w.target_info(target) else {
                    return false;
                };
                if !(m.within_melee_range(&info) && visible_contains(m, w, target)) {
                    return false;
                }
                mem(m).look_target.set(Tracker::Entity(target, true));
                m.swing();
                warden::do_hurt_target(m, target, w, cx.lr, cx.events);
                mem(m)
                    .attack_cooling_down
                    .set_ttl(true, MELEE_ATTACK_COOLDOWN);
                true
            }),
        }
    }

    /// `BehaviorControl.tickOrStop`.
    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        let now = cx.now;
        match self {
            B::Swim(t) => {
                if now > t.end || !swim_condition(m) {
                    self.do_stop(m, cx);
                } else if m.random.next_float() < 0.8 {
                    m.ai.jumping = true;
                }
            }
            B::LookAtTargetSink(t) => {
                let target = mem(m).look_target.value();
                let visible = match target {
                    Some(tr) => tracker_visible(m, cx.w, tr),
                    None => false,
                };
                if now > t.end || !visible {
                    self.do_stop(m, cx);
                } else if let Some(at) = target.and_then(|tr| tr.position(cx.w)) {
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
                    && mem(m).walk_target.value().is_some_and(|walk| {
                        !m.ai.nav.is_done()
                            && !reached(m, cx.w, &walk)
                            && !matches!(walk.target, Tracker::Entity(Who::Player, _)
                                if cx.w.player.as_ref().is_some_and(|p| p.spectator))
                    });
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
                    && dist_sqr(now_target, last) > 4
                    && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach)
                {
                    *last_target = Some(now_target);
                    mem(m).path.set_opt(path.clone());
                    m.nav_move_to_path(path.clone(), f64::from(*speed));
                }
            }
            // canStillUse is true: they run their duration.
            B::Emerging(t) | B::Sniffing(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::ForceUnmount(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::Digging(t) => {
                if now > t.end || m.removed {
                    self.do_stop(m, cx);
                }
            }
            B::DoNothing(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::RunOne(g) => {
                for k in &mut g.kids {
                    if k.0.running() {
                        k.0.tick_or_stop(m, cx);
                    }
                }
                if !g.kids.iter().any(|k| k.0.running()) {
                    self.do_stop(m, cx);
                }
            }
            B::Roar(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                    return;
                }
                let mm = mem(m);
                if !mm.roar_sound_delay.present() && !mm.roar_sound_cooldown.present() {
                    mm.roar_sound_cooldown
                        .set_ttl((), i64::from(ROAR_DURATION) - ROAR_SOUND_DELAY);
                    sound(m, "entity.warden.roar", 3.0, 1.0, cx.events);
                }
            }
            B::SonicBoom(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                    return;
                }
                warden::tick_sonic_boom(m, cx.w, cx.events);
            }
            // OneShot.tickOrStop: they stop the tick they ran.
            _ => self.do_stop(m, cx),
        }
    }

    /// `BehaviorControl.doStop`.
    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        match self {
            B::Swim(t) | B::ForceUnmount(t) | B::DoNothing(t) => t.running = false,
            B::LookAtTargetSink(t) => {
                t.running = false;
                mem(m).look_target.clear();
            }
            B::MoveToTargetSink {
                t, cooldown, path, ..
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
            }
            B::Emerging(t) => {
                t.running = false;
                if warden::pose(m) == Pose::Emerging {
                    warden::set_pose(m, Pose::Standing);
                }
            }
            B::Digging(t) => {
                t.running = false;
                m.removed = true;
            }
            B::RunOne(g) => {
                g.running = false;
                for k in &mut g.kids {
                    if k.0.running() {
                        k.0.do_stop(m, cx);
                    }
                }
            }
            B::Sniffing(t) => {
                t.running = false;
                if warden::pose(m) == Pose::Sniffing {
                    warden::set_pose(m, Pose::Standing);
                }
                mem(m).is_sniffing.clear();
                let w = cx.w;
                if let Some(entity) = mem(m).nearest_attackable.value()
                    && warden::can_target(w, entity)
                    && let Some(info) = w.target_info(entity)
                {
                    if closer_than(m, info.position, 6.0, 20.0) {
                        warden::increase_anger(m, w, Some(entity), 35, true, cx.events);
                    }
                    if !mem(m).disturbance_location.present() {
                        set_disturbance(m, w, block_of(info.position));
                    }
                }
            }
            B::Roar(t) => {
                t.running = false;
                if warden::pose(m) == Pose::Roaring {
                    warden::set_pose(m, Pose::Standing);
                }
                if let Some(target) = mem(m).roar_target.value() {
                    set_attack_target(m, target);
                }
                mem(m).roar_target.clear();
            }
            B::SonicBoom(t) => {
                t.running = false;
                mem(m).sonic_boom_cooldown.set_ttl((), 40);
            }
            B::SetWardenLookTarget(r)
            | B::SetRoarTarget(r)
            | B::TryToSniff(r)
            | B::Stroll(r)
            | B::GoToDisturbance(r)
            | B::DigCooldownSetter(r)
            | B::StopAttackingIfTargetInvalid(r)
            | B::SetEntityLookTarget(r)
            | B::SetWalkTargetFromAttackTarget(r)
            | B::MeleeAttack(r) => *r = false,
        }
    }
}

/// `OneShot.tryStart`: running when the trigger fires.
fn one(running: &mut bool, f: impl FnOnce() -> bool) -> bool {
    if f() {
        *running = true;
        true
    } else {
        false
    }
}
/// `Swim`: deeper than the jump threshold, or in lava.
fn swim_condition(m: &Mob) -> bool {
    m.fluids.water > m.fluid_jump_threshold() || m.fluids.in_lava
}
/// The `Cx` the mob library's attacks take.
pub(super) fn cx<'a, 'w>(
    w: &'a MobWorld<'w>,
    lr: &'a mut LegacyRandom,
    events: &'a mut Vec<MobEvent>,
) -> Cx<'a, 'w> {
    Cx { w, lr, events }
}

// ---- Printing ---------------------------------------------------------------

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
/// The active activities and the memories with values, as the mob AI oracle
/// prints them (`act core|idle`, `mem name=value...` sorted).
pub fn describe(m: &Mob) -> Option<(String, String)> {
    let b = brain(m)?;
    let mut acts = ["core", b.activity.name()];
    acts.sort_unstable();
    let mm = &b.mem;
    let mut out: Vec<String> = Vec::new();
    let mut put = |name: &str, value: String, ttl: i64| {
        if ttl == i64::MAX {
            out.push(format!("{name}={value}"));
        } else {
            out.push(format!("{name}={value}@{ttl}"));
        }
    };
    macro_rules! slot {
        ($name:expr, $s:expr, $f:expr) => {
            if let Some(v) = $s.get() {
                put($name, $f(v), $s.ttl());
            }
        };
    }
    let unit = |_: &()| "u".to_string();
    let list = |l: &Vec<Who>| format!("n{}", l.len());
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
    slot!("mobs", mm.mobs, list);
    slot!("visible_mobs", mm.visible_mobs, |_: &Visible| {
        "NearestVisibleLivingEntities".to_string()
    });
    slot!("nearest_attackable", mm.nearest_attackable, |e: &Who| who(
        *e
    ));
    slot!("look_target", mm.look_target, |t: &Tracker| tracker(*t));
    slot!("walk_target", mm.walk_target, |t: &WalkTarget| format!(
        "{}/{:x}/{}",
        tracker(t.target),
        t.speed.to_bits(),
        t.close
    ));
    slot!("path", mm.path, |p: &Path| format!("path{}", pos(p.target)));
    slot!(
        "cant_reach_walk_target_since",
        mm.cant_reach_walk_target_since,
        |t: &i64| t.to_string()
    );
    slot!("attack_target", mm.attack_target, |e: &Who| who(*e));
    slot!("attack_cooling_down", mm.attack_cooling_down, |t: &bool| t
        .to_string());
    slot!("roar_target", mm.roar_target, |e: &Who| who(*e));
    slot!(
        "disturbance_location",
        mm.disturbance_location,
        |p: &[i32; 3]| pos(*p)
    );
    slot!("recent_projectile", mm.recent_projectile, unit);
    slot!("is_sniffing", mm.is_sniffing, unit);
    slot!("is_emerging", mm.is_emerging, unit);
    slot!("roar_sound_delay", mm.roar_sound_delay, unit);
    slot!("dig_cooldown", mm.dig_cooldown, unit);
    slot!("roar_sound_cooldown", mm.roar_sound_cooldown, unit);
    slot!("sniff_cooldown", mm.sniff_cooldown, unit);
    slot!("touch_cooldown", mm.touch_cooldown, unit);
    slot!("vibration_cooldown", mm.vibration_cooldown, unit);
    slot!("sonic_boom_cooldown", mm.sonic_boom_cooldown, unit);
    slot!(
        "sonic_boom_sound_cooldown",
        mm.sonic_boom_sound_cooldown,
        unit
    );
    slot!("sonic_boom_sound_delay", mm.sonic_boom_sound_delay, unit);
    out.sort();
    Some((acts.join("|"), out.join(" ")))
}
