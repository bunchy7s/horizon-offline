//! The villager's `Brain` (`Villager.BRAIN_PROVIDER` and
//! `VillagerGoalPackages`), ported behaviour for behaviour: memories that
//! expire, the nine sensors and their scan rates, activities from the
//! `villager_activity` timeline, and the behaviours of each activity, started
//! and ticked in vanilla's order (priority, then the activity's slot in
//! `Brain.availableBehaviorsByPriority`'s hash map, then insertion order).
//!
//! The level random (`ServerLevel.getRandom`) times behaviours and POI scans;
//! the mob's own random drives positions, sounds and jumps. `RunOne` and the
//! other `GateBehavior`s shuffle with a random of their own, seeded with
//! `Brain::shuffle_seed` when the brain is built.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    brain::{Timed, living_near},
    path::{Path, PathNode},
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `MemorySlot.NEVER_EXPIRE`.
pub(super) const NEVER: i64 = i64::MAX;

/// `MemorySlot`: a value and its time to live.
#[derive(Clone, Debug)]
pub struct Slot<T> {
    v: Option<T>,
    pub(super) ttl: i64,
}
impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            v: None,
            ttl: NEVER,
        }
    }
}
impl<T> Slot<T> {
    /// `MemorySlot.tick` (`Brain.forgetOutdatedMemories`).
    pub(super) fn tick(&mut self) {
        if self.v.is_some() && self.ttl != NEVER {
            if self.ttl <= 0 {
                self.clear();
            } else {
                self.ttl -= 1;
            }
        }
    }
    pub fn set(&mut self, v: T) {
        self.v = Some(v);
        self.ttl = NEVER;
    }
    pub fn set_ttl(&mut self, v: T, ttl: i64) {
        self.v = Some(v);
        self.ttl = ttl;
    }
    pub fn set_opt(&mut self, v: Option<T>) {
        match v {
            Some(v) => self.set(v),
            None => self.clear(),
        }
    }
    pub fn clear(&mut self) {
        self.v = None;
        self.ttl = NEVER;
    }
    pub fn get(&self) -> Option<&T> {
        self.v.as_ref()
    }
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.v.as_mut()
    }
    pub fn present(&self) -> bool {
        self.v.is_some()
    }
    /// The time to live (`i64::MAX`: never expires).
    pub fn ttl(&self) -> i64 {
        self.ttl
    }
}
impl<T: Clone> Slot<T> {
    pub fn value(&self) -> Option<T> {
        self.v.clone()
    }
}
impl<T> Slot<Vec<T>> {
    /// `Brain.setMemory` of a collection: an empty one erases the memory.
    pub(super) fn set_list(&mut self, v: Vec<T>) {
        if v.is_empty() {
            self.clear();
        } else {
            self.set(v);
        }
    }
}

/// `PositionTracker`: an entity (`EntityTracker`, at its eyes or feet) or a
/// block (`BlockPosTracker`, at its centre or at a given point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tracker {
    Entity(Who, bool),
    Block([i32; 3]),
    Point([f64; 3]),
}
impl Tracker {
    /// `currentPosition`.
    pub fn position(&self, w: &MobWorld) -> Option<[f64; 3]> {
        match *self {
            Tracker::Entity(who, eye) => w.target_info(who).map(|t| {
                [
                    t.position.x,
                    if eye { t.eye_y } else { t.position.y },
                    t.position.z,
                ]
            }),
            Tracker::Block(p) => Some([
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]),
            Tracker::Point(v) => Some(v),
        }
    }
    /// `currentBlockPosition`.
    pub fn block(&self, w: &MobWorld) -> Option<[i32; 3]> {
        match *self {
            Tracker::Entity(who, _) => w.target_info(who).map(|t| block_of(t.position)),
            Tracker::Block(p) => Some(p),
            Tracker::Point(v) => Some([
                v[0].floor() as i32,
                v[1].floor() as i32,
                v[2].floor() as i32,
            ]),
        }
    }
}
fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
/// `WalkTarget`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTarget {
    pub target: Tracker,
    pub speed: f32,
    pub close: i32,
}
impl WalkTarget {
    pub(super) fn block(p: [i32; 3], speed: f32, close: i32) -> Self {
        Self {
            target: Tracker::Block(p),
            speed,
            close,
        }
    }
    /// `new WalkTarget(Vec3, ...)`: the block containing the point.
    pub(super) fn point(p: [f64; 3], speed: f32, close: i32) -> Self {
        Self::block(
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ],
            speed,
            close,
        )
    }
}

/// `NearestVisibleLivingEntities`: the nearby entities and a lazily filled
/// cache of `Sensor.isEntityTargetable`.
#[derive(Clone, Debug, Default)]
pub struct Visible {
    pub(super) list: Vec<Who>,
    pub(super) cache: Vec<(Who, bool)>,
}

/// The memories a villager's brain registers.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    pub home: Slot<[i32; 3]>,
    pub job_site: Slot<[i32; 3]>,
    pub potential_job_site: Slot<[i32; 3]>,
    pub meeting_point: Slot<[i32; 3]>,
    pub secondary_job_site: Slot<Vec<[i32; 3]>>,
    pub mobs: Slot<Vec<Who>>,
    pub visible_mobs: Slot<Visible>,
    pub visible_villager_babies: Slot<Vec<Who>>,
    pub nearest_players: Slot<Vec<Who>>,
    pub nearest_visible_player: Slot<Who>,
    pub nearest_visible_targetable_player: Slot<Who>,
    pub nearest_visible_targetable_players: Slot<Vec<Who>>,
    pub walk_target: Slot<WalkTarget>,
    pub look_target: Slot<Tracker>,
    pub interaction_target: Slot<Who>,
    pub breed_target: Slot<Who>,
    /// `PATH`: the navigation's path (its target).
    pub path: Slot<Path>,
    pub doors_to_close: Slot<Vec<[i32; 3]>>,
    pub nearest_bed: Slot<[i32; 3]>,
    /// `HURT_BY`: the damage (its game time).
    pub hurt_by: Slot<i64>,
    pub hurt_by_entity: Slot<Who>,
    pub nearest_hostile: Slot<Who>,
    pub hiding_place: Slot<[i32; 3]>,
    pub heard_bell_time: Slot<i64>,
    /// `NEAREST_VISIBLE_WANTED_ITEM`: where the item entity lies.
    pub nearest_wanted_item: Slot<[f64; 3]>,
    pub cant_reach_walk_target_since: Slot<i64>,
    pub golem_detected_recently: Slot<bool>,
    pub last_slept: Slot<i64>,
    pub last_woken: Slot<i64>,
    pub last_worked_at_poi: Slot<i64>,
}
impl Memories {
    fn tick(&mut self) {
        self.home.tick();
        self.job_site.tick();
        self.potential_job_site.tick();
        self.meeting_point.tick();
        self.secondary_job_site.tick();
        self.mobs.tick();
        self.visible_mobs.tick();
        self.visible_villager_babies.tick();
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_targetable_player.tick();
        self.nearest_visible_targetable_players.tick();
        self.walk_target.tick();
        self.look_target.tick();
        self.interaction_target.tick();
        self.breed_target.tick();
        self.path.tick();
        self.doors_to_close.tick();
        self.nearest_bed.tick();
        self.hurt_by.tick();
        self.hurt_by_entity.tick();
        self.nearest_hostile.tick();
        self.hiding_place.tick();
        self.heard_bell_time.tick();
        self.nearest_wanted_item.tick();
        self.cant_reach_walk_target_since.tick();
        self.golem_detected_recently.tick();
        self.last_slept.tick();
        self.last_woken.tick();
        self.last_worked_at_poi.tick();
    }
    /// `Brain.pack`: the memories with a codec, which a rebuilt brain keeps.
    fn pack(&self) -> Memories {
        Memories {
            home: self.home.clone(),
            job_site: self.job_site.clone(),
            potential_job_site: self.potential_job_site.clone(),
            meeting_point: self.meeting_point.clone(),
            golem_detected_recently: self.golem_detected_recently.clone(),
            last_slept: self.last_slept.clone(),
            last_woken: self.last_woken.clone(),
            last_worked_at_poi: self.last_worked_at_poi.clone(),
            ..Memories::default()
        }
    }
}

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Core,
    Idle,
    Work,
    Play,
    Rest,
    Meet,
    Panic,
    Raid,
    PreRaid,
    Hide,
}
impl Act {
    pub fn name(self) -> &'static str {
        match self {
            Act::Core => "core",
            Act::Idle => "idle",
            Act::Work => "work",
            Act::Play => "play",
            Act::Rest => "rest",
            Act::Meet => "meet",
            Act::Panic => "panic",
            Act::Raid => "raid",
            Act::PreRaid => "pre_raid",
            Act::Hide => "hide",
        }
    }
    /// The bucket order of `HashMap<Activity, ...>` (16 buckets, the hash
    /// of the name; equal buckets keep the villager's insertion order).
    fn slot(self) -> u8 {
        match self {
            Act::Play => 0,
            Act::Core => 1,
            Act::Rest => 2,
            Act::Hide => 3,
            Act::Meet => 4,
            Act::Idle => 5,
            Act::PreRaid => 6,
            Act::Work => 7,
            Act::Panic => 8,
            Act::Raid => 9,
        }
    }
}

/// The scan rate of each sensor, in `BRAIN_PROVIDER` order.
const SENSOR_RATES: [i64; 9] = [20, 20, 20, 20, 20, 20, 20, 40, 200];

/// A villager's brain.
#[derive(Clone, Debug)]
pub struct Brain {
    pub mem: Memories,
    sensors: [i64; 9],
    /// The active non-core activity (`CORE` is always active).
    pub activity: Act,
    default_activity: Act,
    last_schedule_update: i64,
    list: Vec<Entry>,
    pub shuffle_seed: i64,
    baby: bool,
    /// Whether the nearest-bed sensor's batch cache is used (`NearestBedSensor`).
    bed_cache: Vec<([i32; 3], i64)>,
    /// `NearestItemSensor` is due; the game, which knows the items, answers.
    scan_items: bool,
}
#[derive(Clone, Debug)]
struct Entry {
    prio: i32,
    act: Act,
    b: B,
}

fn brain(m: &Mob) -> Option<&Brain> {
    super::villager::ext(m).and_then(|e| e.brain.as_deref())
}
fn brain_mut(m: &mut Mob) -> &mut Brain {
    let (baby, farmer) = (m.is_baby(), super::villager::is_farmer(m));
    let e = super::villager::ext_mut(m);
    e.brain
        .get_or_insert_with(|| Box::new(Brain::new(baby, 0, farmer)))
}
fn mem(m: &mut Mob) -> &mut Memories {
    &mut brain_mut(m).mem
}
/// `CANT_REACH_WALK_TARGET_SINCE`, for `compute_path`.
fn cant_reach(m: &mut Mob) -> &mut Slot<i64> {
    &mut mem(m).cant_reach_walk_target_since
}
fn mem_ref(m: &Mob) -> Option<&Memories> {
    brain(m).map(|b| &b.mem)
}

impl Brain {
    fn new(baby: bool, shuffle_seed: i64, farmer: bool) -> Self {
        let mut b = Brain {
            mem: Memories::default(),
            sensors: [0; 9],
            activity: Act::Idle,
            default_activity: Act::Idle,
            last_schedule_update: -9999,
            list: Vec::new(),
            shuffle_seed,
            baby,
            bed_cache: Vec::new(),
            scan_items: false,
        };
        b.list = packages(baby, shuffle_seed, farmer);
        b
    }
    fn requirements_met(&self, a: Act) -> bool {
        match a {
            Act::Work => !self.baby && self.mem.job_site.present(),
            Act::Play => self.baby,
            Act::Meet => self.mem.meeting_point.present(),
            Act::Core
            | Act::Idle
            | Act::Rest
            | Act::Panic
            | Act::Raid
            | Act::PreRaid
            | Act::Hide => true,
        }
    }
    pub fn is_active(&self, a: Act) -> bool {
        a == Act::Core || a == self.activity
    }
    /// `setActiveActivityIfPossible`.
    pub fn set_active_if_possible(&mut self, a: Act) {
        if self.requirements_met(a) {
            self.activity = a;
        } else {
            self.activity = self.default_activity;
        }
    }
    /// `updateActivityFromSchedule`.
    fn update_from_schedule(&mut self, game_time: i64, day_time: i64) {
        if game_time - self.last_schedule_update > 20 {
            self.last_schedule_update = game_time;
            let scheduled = schedule(self.baby, day_time);
            if !self.is_active(scheduled) {
                self.set_active_if_possible(scheduled);
            }
        }
    }
}
/// The `villager_activity` and `baby_villager_activity` timelines.
fn schedule(baby: bool, day_time: i64) -> Act {
    match super::villager::activity_at(baby, day_time) {
        super::villager::Activity::Idle => Act::Idle,
        super::villager::Activity::Work => Act::Work,
        super::villager::Activity::Play => Act::Play,
        super::villager::Activity::Meet => Act::Meet,
        super::villager::Activity::Rest => Act::Rest,
    }
}

/// `Brain.Provider.makeBrain` and `Villager.registerBrainGoals`: a new brain
/// from the packed memories (the sensors' first scans are delayed by draws
/// from the mob's random), on the timeline's activity.
fn make_brain(m: &mut Mob, packed: Memories, shuffle_seed: i64, w_times: (i64, i64)) -> Brain {
    let mut b = Brain::new(m.is_baby(), shuffle_seed, super::villager::is_farmer(m));
    for (i, rate) in SENSOR_RATES.iter().enumerate() {
        b.sensors[i] = i64::from(m.random.next_int_bound(*rate as i32).unwrap_or(0));
    }
    b.mem = packed;
    b.update_from_schedule(w_times.0, w_times.1);
    b
}

/// Builds the villager's brain as its constructor does, at this game time:
/// the memories it remembers (job site, bed, bell) come from the mob.
pub fn init(m: &mut Mob, game_time: i64, day_time: i64, shuffle_seed: i64) {
    let mut packed = Memories::default();
    packed.job_site.set_opt(m.extra.home);
    packed.home.set_opt(m.extra.bed);
    packed.meeting_point.set_opt(m.extra.meeting);
    if let Some(t) = super::villager::ext(m).and_then(|e| e.last_slept) {
        packed.last_slept.set(t);
    }
    let b = make_brain(m, packed, shuffle_seed, (game_time, day_time));
    super::villager::ext_mut(m).brain = Some(Box::new(b));
}

/// What behaviours read and emit during a brain tick.
pub struct Bx<'a, 'w> {
    pub w: &'a MobWorld<'w>,
    pub lr: &'a mut LegacyRandom,
    pub events: &'a mut Vec<MobEvent>,
    pub now: i64,
    /// `Villager.refreshBrain` was asked for.
    refresh: bool,
}

/// `Brain.tick` (from `Villager.customServerAiStep`).
pub fn tick(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if brain(m).is_none() {
        let seed = i64::from(m.id);
        init(m, w.game_time, w.day_time, seed);
    }
    // forgetOutdatedMemories
    mem(m).tick();
    tick_sensors(m, w, lr);
    let mut list = std::mem::take(&mut brain_mut(m).list);
    let mut cx = Bx {
        w,
        lr,
        events,
        now: w.game_time,
        refresh: false,
    };
    // The brain whose behaviours these are: after a refresh they keep running
    // against the new brain's memories, gated by the old brain's activities.
    let mut old_activity: Option<Act> = None;
    let mut refreshed = false;
    for i in 0..list.len() {
        let active = match old_activity {
            Some(a) => list[i].act == Act::Core || list[i].act == a,
            None => brain_mut(m).is_active(list[i].act),
        };
        if active && !list[i].b.running() {
            list[i].b.try_start(m, &mut cx);
            if cx.refresh {
                cx.refresh = false;
                old_activity = Some(brain_mut(m).activity);
                refresh(m, &mut list, i, &mut cx);
                refreshed = true;
            }
        }
    }
    for e in list.iter_mut() {
        if e.b.running() {
            e.b.tick_or_stop(m, &mut cx);
        }
    }
    if !refreshed {
        brain_mut(m).list = list;
    }
    sync(m, w.game_time);
}

/// `Villager.refreshBrain`: stop every running behaviour, then rebuild the
/// brain from the remembered memories.
fn refresh(m: &mut Mob, list: &mut [Entry], current: usize, cx: &mut Bx) {
    for (i, e) in list.iter_mut().enumerate() {
        if i != current && e.b.running() {
            e.b.do_stop(m, cx);
        }
    }
    let b = brain_mut(m);
    let packed = b.mem.pack();
    let seed = b.shuffle_seed;
    let nb = make_brain(m, packed, seed, (cx.w.game_time, cx.w.day_time));
    super::villager::ext_mut(m).brain = Some(Box::new(nb));
}

/// Mirrors the remembered positions onto the mob (saved, seen by others),
/// and the sleep and golem memories onto `villager::Ext`.
fn sync(m: &mut Mob, now: i64) {
    let (job, home, meeting, slept, golem) = {
        let b = brain_mut(m);
        (
            b.mem.job_site.value(),
            b.mem.home.value(),
            b.mem.meeting_point.value(),
            b.mem.last_slept.value(),
            b.mem
                .golem_detected_recently
                .get()
                .map(|_| now + b.mem.golem_detected_recently.ttl + 1),
        )
    };
    m.extra.home = job;
    m.extra.bed = home;
    m.extra.meeting = meeting;
    let e = super::villager::ext_mut(m);
    e.last_slept = slept;
    e.golem_until = golem.unwrap_or(0);
}

// ---- Sensors ------------------------------------------------------------

fn tick_sensors(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
    for i in 0..9 {
        let fire = {
            let b = brain_mut(m);
            b.sensors[i] -= 1;
            if b.sensors[i] <= 0 {
                b.sensors[i] = SENSOR_RATES[i];
                true
            } else {
                false
            }
        };
        if !fire {
            continue;
        }
        match i {
            0 => sense_living(m, w),
            1 => sense_players(m, w),
            2 => brain_mut(m).scan_items = true,
            3 => sense_bed(m, w, lr),
            4 => sense_hurt(m, w),
            5 => sense_hostiles(m, w),
            6 => sense_babies(m, w),
            7 => sense_secondary(m, w),
            _ => sense_golem(m, w),
        }
    }
}
const FOLLOW_RANGE: f64 = 16.0;
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
/// `Sensor.isEntityTargetable`: `TargetingConditions.forNonCombat()` within
/// the follow range, in sight.
fn targetable(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    match who {
        Who::Player => m.can_see_player(w, FOLLOW_RANGE),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive && m.distance_sqr(s.position) <= FOLLOW_RANGE * FOLLOW_RANGE && m.sees(w, who)
        }
    }
}
/// The visible-entities memory's line-of-sight test, cached per memory.
fn visible_test(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    if let Some(v) = mem_ref(m).and_then(|mm| mm.visible_mobs.get()) {
        if let Some(&(_, r)) = v.cache.iter().find(|(k, _)| *k == who) {
            return r;
        }
    } else {
        return false;
    }
    let r = targetable(m, w, who);
    if let Some(v) = mem(m).visible_mobs.v.as_mut() {
        v.cache.push((who, r));
    }
    r
}
/// `NearestVisibleLivingEntities.findClosest`.
fn find_closest(m: &mut Mob, w: &MobWorld, filter: impl Fn(&Mob, Who) -> bool) -> Option<Who> {
    let list = mem_ref(m)?.visible_mobs.get()?.list.clone();
    list.into_iter()
        .find(|&who| filter(m, who) && visible_test(m, w, who))
}
/// `NearestVisibleLivingEntities.contains(entity)`.
fn visible_contains(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let Some(list) = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
        .map(|v| v.list.contains(&who))
    else {
        return false;
    };
    list && visible_test(m, w, who)
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
        .filter(|_| m.can_target_player(w, FOLLOW_RANGE, true))
        .collect();
    let mm = mem(m);
    mm.nearest_players.set_list(players);
    mm.nearest_visible_player.set_opt(visible.first().copied());
    mm.nearest_visible_targetable_players
        .set_list(attackable.clone());
    mm.nearest_visible_targetable_player
        .set_opt(attackable.first().copied());
}
/// `NearestBedSensor`: babies remember the nearest reachable bed, trying
/// at most four beds a scan and skipping the ones tried lately.
fn sense_bed(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
    if !m.is_baby() {
        return;
    }
    let last_update = w.game_time + i64::from(lr.next_int_bound(20).unwrap_or(0));
    let homes = homes_in_range(m, w, 48);
    let mut cache = std::mem::take(&mut brain_mut(m).bed_cache);
    let mut tried = 0;
    let mut targets = Vec::new();
    for &p in &homes {
        if cache.iter().any(|(q, _)| *q == p) {
            continue;
        }
        tried += 1;
        if tried >= 5 {
            continue;
        }
        cache.push((p, last_update + 40));
        targets.push(p);
    }
    let path = if targets.is_empty() {
        None
    } else {
        m.create_path_to_any(w, &targets, 1)
    };
    // createPath hands back the current path when it still heads for a
    // target position it was asked: only a bed (a POI) counts.
    if let Some(path) = path.filter(|p| p.reached) {
        if homes.contains(&path.target) {
            mem(m).nearest_bed.set(path.target);
        }
    } else if tried < 5 {
        cache.retain(|(_, until)| *until >= last_update);
    }
    brain_mut(m).bed_cache = cache;
}
/// `HurtBySensor`: the last damage (forgotten 40 ticks after) and who did it.
fn sense_hurt(m: &mut Mob, w: &MobWorld) {
    // LivingEntity.getLastDamageSource: forgotten 40 ticks after the hit.
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
/// `VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`.
fn hostile_distance(s: Species) -> Option<f32> {
    Some(match s {
        Species::Drowned
        | Species::Husk
        | Species::Vex
        | Species::Zombie
        | Species::ZombieVillager => 8.0,
        Species::Evoker | Species::Illusioner | Species::Ravager => 12.0,
        Species::Pillager => 15.0,
        Species::Vindicator | Species::Zoglin => 10.0,
        _ => return None,
    })
}
fn sense_hostiles(m: &mut Mob, w: &MobWorld) {
    let hostile = find_closest(m, w, |m, who| {
        let Who::Mob(id) = who else { return false };
        let Some(s) = w.seen(id) else { return false };
        hostile_distance(s.species).is_some_and(|d| m.distance_sqr(s.position) <= f64::from(d * d))
    });
    mem(m).nearest_hostile.set_opt(hostile);
}
fn sense_babies(m: &mut Mob, w: &MobWorld) {
    let list = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
        .map(|v| v.list.clone())
        .unwrap_or_default();
    let mut babies = Vec::new();
    for who in list {
        let Who::Mob(id) = who else { continue };
        if w.seen(id)
            .is_some_and(|s| s.species == Species::Villager && s.baby)
            && visible_test(m, w, who)
        {
            babies.push(who);
        }
    }
    mem(m).visible_villager_babies.set_list(babies);
}
/// `SecondaryPoiSensor`: a farmer's farmland within four blocks.
fn sense_secondary(m: &mut Mob, w: &MobWorld) {
    let mut sites = Vec::new();
    if super::villager::is_farmer(m) {
        let c = block_of(m.position);
        for x in -4..=4 {
            for y in -2..=2 {
                for z in -4..=4 {
                    let p = [c[0] + x, c[1] + y, c[2] + z];
                    if w.block_name(p[0], p[1], p[2]) == "minecraft:farmland" {
                        sites.push(p);
                    }
                }
            }
        }
    }
    mem(m).secondary_job_site.set_list(sites);
}
/// `GolemSensor`.
fn sense_golem(m: &mut Mob, w: &MobWorld) {
    let golem = mem_ref(m)
        .and_then(|mm| mm.mobs.get())
        .is_some_and(|list| {
            list.iter().any(|who| {
                matches!(who, Who::Mob(id) if w.seen(*id).is_some_and(|s| s.species == Species::IronGolem))
            })
        });
    if golem {
        mem(m).golem_detected_recently.set_ttl(true, 599);
    }
}

/// `GolemSensor.golemDetected`: a golem was summoned near this villager;
/// for 599 ticks it wants no other.
pub fn golem_detected(m: &mut Mob, now: i64) {
    mem(m).golem_detected_recently.set_ttl(true, 599);
    // What `sync` derives from the memory.
    super::villager::ext_mut(m).golem_until = now + 599 + 1;
}

// ---- Behaviours ---------------------------------------------------------

pub use crate::poi_sections::Poi;
/// The POI a block state is (`PoiTypes`): job site blocks, bed heads, bells.
pub fn poi_at(w: &MobWorld, x: i32, y: i32, z: i32) -> Option<Poi> {
    poi_of_state(w, w.world.block(x, y, z))
}
/// The POI a block state is, if any (`PoiTypes.forState`), from the table
/// of every state built on first use.
fn poi_of_state(w: &MobWorld, state: horizon_world::registry::BlockStateId) -> Option<Poi> {
    let table = w.data.pois.get_or_init(|| {
        (0..w.registry.len())
            .map(|i| state_poi(w, horizon_world::registry::BlockStateId(i as u32)))
            .collect()
    });
    match table.get(state.0 as usize) {
        Some(poi) => *poi,
        None => state_poi(w, state),
    }
}
fn state_poi(w: &MobWorld, state: horizon_world::registry::BlockStateId) -> Option<Poi> {
    let name = &*w.facts.get(state).name;
    if name == "minecraft:bell" {
        return Some(Poi::Meeting);
    }
    if name.ends_with("_bed") {
        let part = w
            .registry
            .state(state)
            .and_then(|s| s.properties.get("part"));
        return (part.map(String::as_str) == Some("head")).then_some(Poi::Home);
    }
    super::villager::job_site_profession(name).map(Poi::Job)
}
/// Which POI types a predicate accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PoiFilter {
    /// `VillagerProfession.heldJobSite` of the current profession.
    Held,
    /// `acquirableJobSite` of the current profession.
    Acquirable,
    Home,
    Meeting,
}
fn poi_matches(m: &Mob, f: PoiFilter, p: Poi) -> bool {
    let profession = m.extra.variant2;
    let employed = profession != super::villager::NONE && profession != super::villager::NITWIT;
    match (f, p) {
        (PoiFilter::Held, Poi::Job(j)) => employed && j == profession,
        (PoiFilter::Acquirable, Poi::Job(j)) => {
            if profession == super::villager::NONE {
                true
            } else {
                employed && j == profession
            }
        }
        (PoiFilter::Home, Poi::Home) | (PoiFilter::Meeting, Poi::Meeting) => true,
        _ => false,
    }
}
/// Tickets taken on a POI: by this villager's memories and by others'
/// (counted once per search by [`others_tickets`]).
fn tickets(m: &Mob, others: &crate::fast_hash::FxHashMap<[i32; 3], usize>, p: [i32; 3]) -> usize {
    let mut n = 0;
    if let Some(mm) = mem_ref(m) {
        for s in [
            &mm.job_site,
            &mm.potential_job_site,
            &mm.home,
            &mm.meeting_point,
        ] {
            if s.get() == Some(&p) {
                n += 1;
            }
        }
    }
    n + others.get(&p).copied().unwrap_or(0)
}
/// The tickets the other living villagers' POIs take, by POI.
fn others_tickets(m: &Mob, w: &MobWorld) -> crate::fast_hash::FxHashMap<[i32; 3], usize> {
    let mut taken = crate::fast_hash::FxHashMap::default();
    for o in w.others {
        if o.id != m.id && o.alive && o.species == Species::Villager {
            for q in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                *taken.entry(q).or_insert(0) += 1;
            }
        }
    }
    taken
}
/// The POIs of chunk `(qx, qz)` from `y0` to `y1` (within the world), in
/// scan order: y, then z, then x. Only the indexed POI blocks of the sections
/// are visited (`PoiManager.getInChunk`).
fn chunk_pois(
    w: &MobWorld,
    (qx, qz): (i32, i32),
    y0: i32,
    y1: i32,
    mut visit: impl FnMut(Poi, [i32; 3]),
) {
    let Some(chunk) = w.world.chunks.get(&(qx, qz)) else {
        return;
    };
    let kind = |state: u32| poi_of_state(w, horizon_world::registry::BlockStateId(state));
    let min = crate::chunk::MIN_Y;
    for s in (y0 - min) >> 4..=(y1 - min) >> 4 {
        let section = &chunk.sections[s as usize];
        for &(i, poi) in w.world.pois.get((qx, s, qz), section, kind).iter() {
            let i = i32::from(i);
            let y = min + s * 16 + (i >> 8);
            if (y0..=y1).contains(&y) {
                visit(poi, [qx * 16 + (i & 15), y, qz * 16 + (i >> 4 & 15)]);
            }
        }
    }
}
/// `PoiManager.getInRange` with `HAS_SPACE`, nearest first (`distSqr` to the
/// block, ties in scan order: chunk by chunk, z then x, and y, z, x within),
/// filtered by `accept`.
fn pois_in_range(
    m: &Mob,
    w: &MobWorld,
    f: PoiFilter,
    radius: i32,
    mut accept: impl FnMut([i32; 3]) -> bool,
) -> Vec<(Poi, [i32; 3])> {
    let c = block_of(m.position);
    let mut found = Vec::new();
    // Every candidate asks how taken it is: count the others' once.
    let mut taken = None;
    let r2 = i64::from(radius) * i64::from(radius);
    let (cx, cz) = (c[0] >> 4, c[2] >> 4);
    let chunks = radius.div_euclid(16) + 1;
    let y0 = (c[1] - radius).max(crate::chunk::MIN_Y);
    let y1 = (c[1] + radius).min(crate::chunk::MIN_Y + crate::chunk::HEIGHT - 1);
    for qz in cz - chunks..=cz + chunks {
        for qx in cx - chunks..=cx + chunks {
            chunk_pois(w, (qx, qz), y0, y1, |poi, p| {
                if (p[0] - c[0]).abs() > radius
                    || (p[2] - c[2]).abs() > radius
                    || dist_sqr(p, c) > r2
                    || !poi_matches(m, f, poi)
                    || tickets(m, taken.get_or_insert_with(|| others_tickets(m, w)), p)
                        >= poi.max_tickets()
                {
                    return;
                }
                if accept(p) {
                    found.push((poi, p));
                }
            });
        }
    }
    found.sort_by_key(|&(_, p)| dist_sqr(p, c));
    found
}
/// `PoiManager.exists(pos, predicate)`.
fn poi_exists(m: &Mob, w: &MobWorld, p: [i32; 3], f: PoiFilter) -> bool {
    poi_at(w, p[0], p[1], p[2]).is_some_and(|poi| poi_matches(m, f, poi))
}
/// `BlockPos.closerToCenterThan(position, d)`.
fn closer_to_center(p: [i32; 3], at: Vec3, d: f64) -> bool {
    let c = Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    );
    c.distance_squared(at) < d * d
}
fn manhattan(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}
/// The village sections: sections holding a taken village POI.
fn village_sections(m: &Mob, w: &MobWorld) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    let mut add = |p: [i32; 3]| {
        let s = [p[0] >> 4, p[1] >> 4, p[2] >> 4];
        if !out.contains(&s) {
            out.push(s);
        }
    };
    if let Some(mm) = mem_ref(m) {
        for s in [
            &mm.job_site,
            &mm.potential_job_site,
            &mm.home,
            &mm.meeting_point,
        ] {
            if let Some(&p) = s.get()
                && poi_at(w, p[0], p[1], p[2]).is_some()
            {
                add(p);
            }
        }
    }
    for o in w.others {
        if o.alive && o.species == Species::Villager {
            for p in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                if poi_at(w, p[0], p[1], p[2]).is_some() {
                    add(p);
                }
            }
        }
    }
    out
}
/// `PoiManager.sectionsToVillage`: the Chebyshev distance in sections to a
/// village section, at most 7.
fn sections_to_village(villages: &[[i32; 3]], s: [i32; 3]) -> i32 {
    villages
        .iter()
        .map(|v| {
            (v[0] - s[0])
                .abs()
                .max((v[1] - s[1]).abs())
                .max((v[2] - s[2]).abs())
        })
        .min()
        .unwrap_or(7)
        .min(7)
}

/// A memory a gate or behaviour conditions on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mem {
    VisibleBabies,
    InteractionTarget,
    BreedTarget,
    Home,
}
fn mem_present(m: &Mob, which: Mem) -> bool {
    let Some(mm) = mem_ref(m) else { return false };
    match which {
        Mem::VisibleBabies => mm.visible_villager_babies.present(),
        Mem::InteractionTarget => mm.interaction_target.present(),
        Mem::BreedTarget => mm.breed_target.present(),
        Mem::Home => mm.home.present(),
    }
}
fn mem_erase(m: &mut Mob, which: Mem) {
    let mm = mem(m);
    match which {
        Mem::VisibleBabies => mm.visible_villager_babies.clear(),
        Mem::InteractionTarget => mm.interaction_target.clear(),
        Mem::BreedTarget => mm.breed_target.clear(),
        Mem::Home => mm.home.clear(),
    }
}

/// `GateBehavior`: `RunOne` (shuffled, run one) and the ordered gates.
#[derive(Clone, Debug)]
struct Gate {
    running: bool,
    /// Entry condition: this memory must be absent.
    absent: Option<Mem>,
    /// Erased when the gate stops.
    erase: Option<Mem>,
    shuffled: bool,
    random: LegacyRandom,
    kids: Vec<(B, i32, f64)>,
}
impl Gate {
    fn run_one(seed: i64, kids: Vec<(B, i32)>) -> Self {
        Self {
            running: false,
            absent: None,
            erase: None,
            shuffled: true,
            random: LegacyRandom::new(seed),
            kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
        }
    }
    fn ordered(seed: i64, erase: Mem, kid: B) -> Self {
        Self {
            running: false,
            absent: None,
            erase: Some(erase),
            shuffled: false,
            random: LegacyRandom::new(seed),
            kids: vec![(kid, 1, 0.0)],
        }
    }
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        if self.absent.is_some_and(|a| mem_present(m, a)) {
            return false;
        }
        self.running = true;
        if self.shuffled {
            // ShufflingList.shuffle: a fresh weight for each entry in the
            // current order, then a stable sort.
            for k in &mut self.kids {
                let r = self.random.next_float();
                k.2 = -horizon_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
            }
            self.kids.sort_by(|a, b| a.2.total_cmp(&b.2));
        }
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
        if let Some(e) = self.erase {
            mem_erase(m, e);
        }
    }
}

/// `SetEntityLookTarget`'s predicates.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LookAt {
    Species(Species),
    Player,
    Category(&'static str),
}
/// `InteractWith`'s targets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Interact {
    Villager,
    BreedVillager,
    Cat,
}
/// The fixed behaviours whose only state is their running flag (OneShots
/// stop in the same tick).
#[derive(Clone, Debug)]
enum B {
    Swim(Timed),
    InteractWithDoor {
        running: bool,
        last_node: Option<PathNode>,
        cooldown: i32,
    },
    LookAtTargetSink(Timed),
    PanicTrigger(Timed),
    WakeUp(bool),
    ReactToBell(bool),
    SetRaidStatus(bool),
    ValidatePoi {
        running: bool,
        filter: PoiFilter,
        which: PoiMem,
    },
    MoveToTargetSink {
        t: Timed,
        cooldown: i32,
        path: Option<Path>,
        last_target: Option<[i32; 3]>,
        speed: f32,
    },
    PoiCompetitorScan(bool),
    FollowTradingPlayer(Timed),
    GoToWantedItem(bool),
    AcquirePoi {
        running: bool,
        filter: PoiFilter,
        /// The memory filled (and checked absent): job site, home, meeting.
        poi: Poi,
        only_adult: bool,
        event: bool,
        next: i64,
        retries: Vec<([i32; 3], Retry)>,
    },
    GoToPotentialJobSite(Timed),
    YieldJobSite(bool),
    AssignProfession(bool),
    ResetProfession(bool),
    Gate(Box<Gate>),
    InteractWith {
        running: bool,
        kind: Interact,
    },
    VillageBoundStroll {
        running: bool,
        speed: f32,
        xz: i32,
        y: i32,
    },
    WalkToLookTarget(bool),
    JumpOnBed {
        t: Timed,
        bed: Option<[i32; 3]>,
        time_left: i32,
        jumps: i32,
        cooldown: i32,
    },
    DoNothing(Timed),
    GiveGiftToHero {
        t: Timed,
        until_next: i32,
        given: bool,
        started: i64,
    },
    SetLookAndInteract(bool),
    ShowTrades {
        t: Timed,
        look_time: i32,
    },
    TradeWithVillager(Timed),
    MakeLove {
        t: Timed,
        birth: i64,
    },
    LookAtEntity {
        running: bool,
        what: LookAt,
    },
    UpdateActivity(bool),
    WorkAtPoi {
        t: Timed,
        last_check: i64,
        /// `WorkAtComposter` (a farmer's package) rather than `WorkAtPoi`.
        composter: bool,
    },
    StrollAroundPoi {
        running: bool,
        next_ok: i64,
        max_dist: i32,
        which: PoiMem,
    },
    StrollToPoi {
        running: bool,
        next_ok: i64,
    },
    StrollToPoiList {
        running: bool,
        next_ok: i64,
    },
    HarvestFarmland {
        t: Timed,
        next_ok: i64,
        target: Option<[i32; 3]>,
        valid: Vec<[i32; 3]>,
        worked: i32,
    },
    UseBonemeal {
        t: Timed,
        next_work: i64,
        last_session: i64,
        worked: i32,
        crop: Option<[i32; 3]>,
        /// The next crop is picked on the next tick, when the level shows the
        /// age the meal gave (vanilla grows the crop at once).
        repick: bool,
    },
    WalkToBlockMemory {
        running: bool,
        which: PoiMem,
        speed: f32,
        close: i32,
        too_far: i32,
        too_long: i64,
    },
    SleepInBed {
        t: Timed,
        next_ok: i64,
    },
    ClosestHomeAsWalkTarget {
        running: bool,
        last_update: i64,
        cache: Vec<([i32; 3], i64)>,
    },
    InsideBrownianWalk {
        running: bool,
        random: LegacyRandom,
    },
    GoToClosestVillage(bool),
    TriggerGate {
        running: bool,
        random: LegacyRandom,
        kids: Vec<(B, i32, f64)>,
    },
    SocializeAtBell(bool),
    CalmDown(bool),
    WalkAwayFrom {
        running: bool,
        hostile: bool,
    },
    RingBell(bool),
    ResetRaidStatus(bool),
    /// `BehaviorBuilder.sequence(triggerIf(raid condition), then)`.
    RaidSequence {
        running: bool,
        victory: bool,
        then: Box<B>,
    },
    MoveToSkySeeingSpot {
        running: bool,
        speed: f32,
    },
    Celebrate(Timed),
    LocateHidingPlace {
        running: bool,
        radius: i32,
        speed: f32,
        close: i32,
    },
    SetHiddenState {
        running: bool,
        hidden: i32,
    },
    PlayTag(bool),
}
/// `AcquirePoi.JitteredLinearRetry`.
#[derive(Clone, Copy, Debug)]
struct Retry {
    previous: i64,
    next: i64,
    delay: i32,
}
impl Retry {
    fn new(cx: &mut Bx, now: i64) -> Self {
        let mut r = Retry {
            previous: 0,
            next: 0,
            delay: 0,
        };
        r.mark(cx, now);
        r
    }
    fn mark(&mut self, cx: &mut Bx, now: i64) {
        self.previous = now;
        let suggested = self.delay + cx.lr.next_int_bound(40).unwrap_or(0) + 40;
        self.delay = suggested.min(400);
        self.next = now + i64::from(self.delay);
    }
}

fn one(f: impl FnOnce() -> bool, running: &mut bool) -> bool {
    let r = f();
    if r {
        *running = true;
    }
    r
}

impl B {
    fn running(&self) -> bool {
        match self {
            B::Swim(t)
            | B::LookAtTargetSink(t)
            | B::PanicTrigger(t)
            | B::FollowTradingPlayer(t)
            | B::GoToPotentialJobSite(t)
            | B::DoNothing(t)
            | B::TradeWithVillager(t)
            | B::Celebrate(t) => t.running,
            B::MoveToTargetSink { t, .. }
            | B::GiveGiftToHero { t, .. }
            | B::ShowTrades { t, .. }
            | B::WorkAtPoi { t, .. }
            | B::HarvestFarmland { t, .. }
            | B::UseBonemeal { t, .. }
            | B::JumpOnBed { t, .. }
            | B::MakeLove { t, .. }
            | B::SleepInBed { t, .. } => t.running,
            B::InteractWithDoor { running, .. }
            | B::ValidatePoi { running, .. }
            | B::AcquirePoi { running, .. }
            | B::InteractWith { running, .. }
            | B::VillageBoundStroll { running, .. }
            | B::LookAtEntity { running, .. }
            | B::StrollAroundPoi { running, .. }
            | B::StrollToPoi { running, .. }
            | B::StrollToPoiList { running, .. }
            | B::WalkToBlockMemory { running, .. }
            | B::ClosestHomeAsWalkTarget { running, .. }
            | B::InsideBrownianWalk { running, .. }
            | B::TriggerGate { running, .. }
            | B::WalkAwayFrom { running, .. }
            | B::RaidSequence { running, .. }
            | B::MoveToSkySeeingSpot { running, .. }
            | B::LocateHidingPlace { running, .. }
            | B::SetHiddenState { running, .. } => *running,
            B::WakeUp(r)
            | B::ReactToBell(r)
            | B::SetRaidStatus(r)
            | B::PoiCompetitorScan(r)
            | B::GoToWantedItem(r)
            | B::YieldJobSite(r)
            | B::AssignProfession(r)
            | B::ResetProfession(r)
            | B::WalkToLookTarget(r)
            | B::SetLookAndInteract(r)
            | B::UpdateActivity(r)
            | B::GoToClosestVillage(r)
            | B::SocializeAtBell(r)
            | B::CalmDown(r)
            | B::RingBell(r)
            | B::ResetRaidStatus(r)
            | B::PlayTag(r) => *r,
            B::Gate(g) => g.running,
        }
    }
    /// `OneShot` behaviours stop at their first tick.
    fn one_shot_stop(&mut self) -> bool {
        match self {
            B::InteractWithDoor { running, .. }
            | B::ValidatePoi { running, .. }
            | B::AcquirePoi { running, .. }
            | B::InteractWith { running, .. }
            | B::VillageBoundStroll { running, .. }
            | B::LookAtEntity { running, .. }
            | B::StrollAroundPoi { running, .. }
            | B::StrollToPoi { running, .. }
            | B::StrollToPoiList { running, .. }
            | B::WalkToBlockMemory { running, .. }
            | B::ClosestHomeAsWalkTarget { running, .. }
            | B::InsideBrownianWalk { running, .. }
            | B::TriggerGate { running, .. }
            | B::WalkAwayFrom { running, .. }
            | B::RaidSequence { running, .. }
            | B::MoveToSkySeeingSpot { running, .. }
            | B::LocateHidingPlace { running, .. }
            | B::SetHiddenState { running, .. }
            | B::GoToClosestVillage(running)
            | B::SocializeAtBell(running)
            | B::CalmDown(running)
            | B::RingBell(running)
            | B::ResetRaidStatus(running)
            | B::PlayTag(running)
            | B::WakeUp(running)
            | B::ReactToBell(running)
            | B::SetRaidStatus(running)
            | B::PoiCompetitorScan(running)
            | B::GoToWantedItem(running)
            | B::YieldJobSite(running)
            | B::AssignProfession(running)
            | B::ResetProfession(running)
            | B::WalkToLookTarget(running)
            | B::SetLookAndInteract(running)
            | B::UpdateActivity(running) => {
                *running = false;
                true
            }
            _ => false,
        }
    }

    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        let now = cx.now;
        match self {
            B::Gate(g) => g.try_start(m, cx),
            // -- Behaviours with a duration.
            B::Swim(t) => {
                if !swim_condition(m) {
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
            B::PanicTrigger(t) => {
                t.begin(cx.lr, cx.now);
                // VillagerPanicTrigger.start
                if panic_condition(m) {
                    if !brain_mut(m).is_active(Act::Panic) {
                        let mm = mem(m);
                        mm.path.clear();
                        mm.walk_target.clear();
                        mm.look_target.clear();
                        mm.breed_target.clear();
                        mm.interaction_target.clear();
                    }
                    brain_mut(m).set_active_if_possible(Act::Panic);
                }
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
                let walk = mm.walk_target.value().unwrap();
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
            B::FollowTradingPlayer(t) => {
                if !follow_trading_condition(m, cx.w) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                follow_trading_player(m);
                true
            }
            B::GoToPotentialJobSite(t) => {
                if !mem(m).potential_job_site.present() {
                    return false;
                }
                let a = brain_mut(m).activity;
                if !matches!(a, Act::Idle | Act::Work | Act::Play) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::JumpOnBed {
                t,
                bed,
                time_left,
                jumps,
                cooldown,
            } => {
                let mm = mem(m);
                let Some(nearest) = mm.nearest_bed.value() else {
                    return false;
                };
                if mm.walk_target.present() || !m.is_baby() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                *bed = Some(nearest);
                *time_left = 100;
                *jumps = 3 + cx.lr.next_int_bound(4).unwrap_or(0);
                *cooldown = 0;
                mem(m).walk_target.set(WalkTarget::block(nearest, 0.5, 0));
                true
            }
            B::DoNothing(t) => {
                t.begin(cx.lr, cx.now);
                true
            }
            B::GiveGiftToHero {
                t,
                until_next,
                given,
                started,
            } => {
                if !mem(m).nearest_visible_player.present() {
                    return false;
                }
                if !hero_visible(m, cx.w) {
                    return false;
                }
                if *until_next > 0 {
                    *until_next -= 1;
                    return false;
                }
                t.begin(cx.lr, cx.now);
                *given = false;
                *started = now;
                let mm = mem(m);
                mm.interaction_target.set(Who::Player);
                mm.look_target.set(Tracker::Entity(Who::Player, true));
                true
            }
            B::ShowTrades { t, look_time } => {
                if !mem(m).interaction_target.present() || !show_trades_condition(m, cx.w) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                // start
                if let Some(target) = mem(m).interaction_target.value() {
                    mem(m).look_target.set(Tracker::Entity(target, true));
                }
                *look_time = 40;
                true
            }
            B::TradeWithVillager(t) => {
                let mm = mem(m);
                if !mm.interaction_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                if !villager_target_valid(m, cx.w, Mem::InteractionTarget) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                let target = mem(m).interaction_target.value().unwrap();
                lock_gaze(m, target, cx);
                true
            }
            B::MakeLove { t, birth } => {
                let mm = mem(m);
                if !mm.breed_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                if !breeding_possible(m, cx.w) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                let target = mem(m).breed_target.value().unwrap();
                lock_gaze(m, target, cx);
                *birth = now + 275 + i64::from(m.random.next_int_bound(50).unwrap_or(0));
                true
            }
            B::WorkAtPoi {
                t,
                last_check,
                composter,
            } => {
                let mm = mem(m);
                let Some(site) = mm.job_site.value() else {
                    return false;
                };
                if now - *last_check < 300 {
                    return false;
                }
                if cx.lr.next_int_bound(2).unwrap_or(0) != 0 {
                    return false;
                }
                *last_check = now;
                if !closer_to_center(site, m.position, 1.73) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                // start
                let mm = mem(m);
                mm.last_worked_at_poi.set(now);
                mm.look_target.set(Tracker::Block(site));
                play_work_sound(m, cx);
                // WorkAtComposter.useWorkstation, before the restock.
                if *composter {
                    cx.events.push(MobEvent::WorkAtComposter {
                        mob: m.id,
                        pos: site,
                    });
                }
                restock_if_due(m, cx.w);
                true
            }
            B::HarvestFarmland {
                t,
                next_ok,
                target,
                valid,
                ..
            } => {
                let mm = mem(m);
                if mm.look_target.present()
                    || mm.walk_target.present()
                    || !mm.secondary_job_site.present()
                {
                    return false;
                }
                if !cx.w.mob_griefing || !super::villager::is_farmer(m) {
                    return false;
                }
                *valid = farmland_around(m, cx.w);
                *target = pick_farmland(valid, cx);
                let Some(p) = *target else { return false };
                t.begin(cx.lr, cx.now);
                if now > *next_ok {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Block(p));
                    mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
                }
                true
            }
            B::UseBonemeal {
                t,
                next_work,
                last_session,
                worked,
                crop,
                repick,
            } => {
                let mm = mem(m);
                if mm.look_target.present() || mm.walk_target.present() {
                    return false;
                }
                // Checked every 10 ticks, and not within 160 ticks of the last session.
                let age = i64::from(m.age);
                if age % 10 != 0 || (*last_session != 0 && *last_session + 160 > age) {
                    return false;
                }
                if super::villager::ext(m).is_none_or(|e| e.count_item("minecraft:bone_meal") == 0)
                {
                    return false;
                }
                *crop = pick_bonemeal_crop(m, cx);
                if crop.is_none() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                bonemeal_target(m, *crop);
                m.extra.main_hand = Some("minecraft:bone_meal".into());
                *next_work = cx.now;
                *worked = 0;
                *repick = false;
                true
            }
            // -- OneShots.
            B::InteractWithDoor {
                running,
                last_node,
                cooldown,
            } => {
                if !mem(m).path.present() {
                    return false;
                }
                let Some(path) = m.ai.nav.path.clone() else {
                    return false;
                };
                if path.next == 0 || path.is_done() {
                    return false;
                }
                let next = path.next_node();
                if *last_node == Some(next) {
                    *cooldown = 20;
                } else {
                    *cooldown -= 1;
                    if *cooldown > 0 {
                        return false;
                    }
                }
                *last_node = Some(next);
                let from = path.nodes[path.next - 1];
                interact_with_doors(m, cx, from, next);
                *running = true;
                true
            }
            B::WakeUp(r) => one(
                || {
                    if !brain_mut(m).is_active(Act::Rest) && super::villager::sleeping(m) {
                        super::villager::stop_sleeping(m, cx.w, cx.events);
                        mem(m).last_woken.set(now);
                        true
                    } else {
                        false
                    }
                },
                r,
            ),
            B::ReactToBell(r) => one(
                || {
                    if !mem(m).heard_bell_time.present() {
                        return false;
                    }
                    if !super::villager::raid_nearby(m) {
                        brain_mut(m).set_active_if_possible(Act::Hide);
                    }
                    true
                },
                r,
            ),
            B::SetRaidStatus(r) => one(
                || {
                    if cx.lr.next_int_bound(20).unwrap_or(0) != 0 {
                        return false;
                    }
                    match super::villager::raid_state(m) {
                        Some(true) => {
                            let b = brain_mut(m);
                            b.default_activity = Act::Raid;
                            b.set_active_if_possible(Act::Raid);
                        }
                        Some(false) => {
                            let b = brain_mut(m);
                            b.default_activity = Act::PreRaid;
                            b.set_active_if_possible(Act::PreRaid);
                        }
                        None => {}
                    }
                    true
                },
                r,
            ),
            B::ValidatePoi {
                running,
                filter,
                which,
            } => {
                let Some(p) = which.get(m) else { return false };
                if !closer_to_center(p, m.position, 16.0) {
                    return false;
                }
                if !poi_exists(m, cx.w, p, *filter) {
                    which.clear(m);
                } else if cx.w.block_name(p[0], p[1], p[2]).ends_with("_bed")
                    && cx.w.prop(p[0], p[1], p[2], "occupied") == Some("true")
                    && !super::villager::sleeping(m)
                {
                    // ValidateNearbyPoi.bedIsOccupied: someone else sleeps there.
                    which.clear(m);
                }
                *running = true;
                true
            }
            B::PoiCompetitorScan(r) => one(
                || {
                    let mm = mem(m);
                    let (Some(site), true) = (mm.job_site.value(), mm.mobs.present()) else {
                        return false;
                    };
                    if poi_at(cx.w, site[0], site[1], site[2]).is_some() {
                        let list = mem(m).mobs.value().unwrap_or_default();
                        let my_xp = super::villager::xp(m);
                        let mut winner_xp = my_xp;
                        let mut me_winner = true;
                        for who in list {
                            let Who::Mob(id) = who else { continue };
                            let Some(s) = cx.w.seen(id) else { continue };
                            if s.species != Species::Villager || !s.alive || s.job != Some(site) {
                                continue;
                            }
                            if s.profession != m.extra.variant2 {
                                continue;
                            }
                            // selectWinner(first, second): the second wins ties.
                            if winner_xp > s.xp {
                                // The other villager loses its site (it finds out itself).
                            } else {
                                if me_winner {
                                    mem(m).job_site.clear();
                                }
                                me_winner = false;
                                winner_xp = s.xp;
                            }
                        }
                    }
                    true
                },
                r,
            ),
            B::GoToWantedItem(r) => one(
                || {
                    let mm = mem(m);
                    if mm.walk_target.present() {
                        return false;
                    }
                    let Some(at) = mm.nearest_wanted_item.value() else {
                        return false;
                    };
                    // item.closerThan(body, 4); a villager can pick up loot.
                    if m.position.distance_squared(Vec3::new(at[0], at[1], at[2])) >= 16.0 {
                        return false;
                    }
                    let mm = mem(m);
                    // The item's eye height is 0.2125.
                    mm.look_target
                        .set(Tracker::Point([at[0], at[1] + 0.2125, at[2]]));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Point(at),
                        speed: 0.5,
                        close: 0,
                    });
                    true
                },
                r,
            ),
            B::AcquirePoi {
                running,
                filter,
                poi,
                only_adult,
                event,
                next,
                retries,
            } => {
                let mm = mem(m);
                let (target_present, validate_present) = match poi {
                    Poi::Job(_) => (mm.potential_job_site.present(), mm.job_site.present()),
                    Poi::Home => (mm.home.present(), false),
                    Poi::Meeting => (mm.meeting_point.present(), false),
                };
                if target_present || validate_present {
                    return false;
                }
                if *only_adult && m.is_baby() {
                    return false;
                }
                if *next == 0 {
                    *next = cx.w.game_time + i64::from(cx.lr.next_int_bound(20).unwrap_or(0));
                    return false;
                }
                if cx.w.game_time < *next {
                    return false;
                }
                *next = now + 20 + i64::from(cx.lr.next_int_bound(20).unwrap_or(0));
                retries.retain(|(_, r)| now - r.previous < 400);
                let w = cx.w;
                let mut found = pois_in_range(m, w, *filter, 48, |_| true);
                // The cache test runs on every candidate, in scan order.
                let mut keep = Vec::new();
                {
                    let order = pois_scan_order(m, &found);
                    for idx in order {
                        let p = found[idx].1;
                        let ok = match retries.iter_mut().find(|(q, _)| *q == p) {
                            None => true,
                            Some((_, r)) => {
                                if now >= r.next {
                                    r.mark(cx, now);
                                    true
                                } else {
                                    false
                                }
                            }
                        };
                        if ok {
                            keep.push(idx);
                        }
                    }
                }
                found = found
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| keep.contains(i))
                    .map(|(_, f)| f)
                    .collect();
                found.truncate(5);
                if *poi == Poi::Home {
                    found.retain(|(_, p)| w.prop(p[0], p[1], p[2], "occupied") != Some("true"));
                }
                let path = if found.is_empty() {
                    None
                } else {
                    let range = found
                        .iter()
                        .map(|(t, _)| t.valid_range())
                        .max()
                        .unwrap_or(1)
                        .max(1);
                    let targets: Vec<[i32; 3]> = found.iter().map(|(_, p)| *p).collect();
                    m.create_path_to_any(w, &targets, range)
                };
                if let Some(path) = path.filter(|p| p.reached) {
                    let target = path.target;
                    if poi_at(w, target[0], target[1], target[2]).is_some() {
                        let mm = mem(m);
                        match poi {
                            Poi::Job(_) => mm.potential_job_site.set(target),
                            Poi::Home => mm.home.set(target),
                            Poi::Meeting => mm.meeting_point.set(target),
                        }
                        if *event {
                            happy_particles(m, cx);
                        }
                        retries.clear();
                    }
                } else {
                    for (_, p) in &found {
                        if !retries.iter().any(|(q, _)| q == p) {
                            let r = Retry::new(cx, now);
                            retries.push((*p, r));
                        }
                    }
                }
                *running = true;
                true
            }
            B::YieldJobSite(r) => one(
                || {
                    let mm = mem(m);
                    let Some(site) = mm.potential_job_site.value() else {
                        return false;
                    };
                    if mm.job_site.present() || !mm.mobs.present() {
                        return false;
                    }
                    if m.is_baby() || m.extra.variant2 != super::villager::NONE {
                        return false;
                    }
                    let Some(poi) = poi_at(cx.w, site[0], site[1], site[2]) else {
                        return true;
                    };
                    let list = mem(m).mobs.value().unwrap_or_default();
                    for who in list {
                        let Who::Mob(id) = who else { continue };
                        let Some(s) = cx.w.seen(id) else { continue };
                        if s.species != Species::Villager || !s.alive || s.potential_job.is_some() {
                            continue;
                        }
                        let Poi::Job(j) = poi else { continue };
                        if s.profession != j {
                            continue;
                        }
                        // A nearby villager of that profession takes it over.
                        if s.job.is_none_or(|p| p == site) {
                            let mm = mem(m);
                            mm.walk_target.clear();
                            mm.look_target.clear();
                            mm.potential_job_site.clear();
                            break;
                        }
                    }
                    true
                },
                r,
            ),
            B::AssignProfession(r) => {
                let Some(p) = mem(m).potential_job_site.value() else {
                    return false;
                };
                if !closer_to_center(p, m.position, 2.0) {
                    return false;
                }
                let mm = mem(m);
                mm.potential_job_site.clear();
                mm.job_site.set(p);
                happy_particles(m, cx);
                *r = true;
                if m.extra.variant2 != super::villager::NONE {
                    return true;
                }
                if let Some(Poi::Job(j)) = poi_at(cx.w, p[0], p[1], p[2]) {
                    super::villager::set_profession(m, j);
                    cx.refresh = true;
                }
                true
            }
            B::ResetProfession(r) => one(
                || {
                    if mem(m).job_site.present() {
                        return false;
                    }
                    let prof = m.extra.variant2;
                    let can_be_fired =
                        prof != super::villager::NONE && prof != super::villager::NITWIT;
                    if can_be_fired && super::villager::xp(m) == 0 && m.extra.variant3 <= 1 {
                        super::villager::set_profession(m, super::villager::NONE);
                        cx.refresh = true;
                        true
                    } else {
                        false
                    }
                },
                r,
            ),
            B::InteractWith { running, kind } => {
                let mm = mem(m);
                if mm.walk_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let kind = *kind;
                if kind == Interact::BreedVillager && !super::villager::can_breed(m) {
                    return false;
                }
                let w = cx.w;
                let valid = move |who: Who| -> bool {
                    let Who::Mob(id) = who else { return false };
                    let Some(s) = w.seen(id) else { return false };
                    match kind {
                        Interact::Villager => s.species == Species::Villager,
                        Interact::BreedVillager => s.species == Species::Villager && s.in_love,
                        Interact::Cat => s.species == Species::Cat,
                    }
                };
                if find_closest(m, w, |_, who| valid(who)).is_none() {
                    return false;
                }
                if let Some(t) = find_closest(m, w, |m, who| {
                    valid(who)
                        && w.target_info(who)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 64.0)
                }) {
                    let mm = mem(m);
                    if kind == Interact::BreedVillager {
                        mm.breed_target.set(t);
                    } else {
                        mm.interaction_target.set(t);
                    }
                    mm.look_target.set(Tracker::Entity(t, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(t, false),
                        speed: 0.5,
                        close: 2,
                    });
                }
                *running = true;
                true
            }
            B::VillageBoundStroll {
                running,
                speed,
                xz,
                y,
            } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                let section = [here[0] >> 4, here[1] >> 4, here[2] >> 4];
                let villages = village_sections(m, w);
                let pos = if sections_to_village(&villages, section) <= 1 {
                    m.land_random_pos(w, *xz, *y)
                } else {
                    let best = closest_section(&villages, section, 2);
                    if best != section {
                        let to = [
                            f64::from(best[0] * 16 + 8) + 0.5,
                            f64::from(best[1] * 16 + 8),
                            f64::from(best[2] * 16 + 8) + 0.5,
                        ];
                        m.default_random_pos_towards(
                            w,
                            *xz,
                            *y,
                            to,
                            f64::from(std::f32::consts::FRAC_PI_2),
                        )
                    } else {
                        m.land_random_pos(w, *xz, *y)
                    }
                };
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, *speed, 0)));
                *running = true;
                true
            }
            B::WalkToLookTarget(r) => one(
                || {
                    let mm = mem(m);
                    if mm.walk_target.present() {
                        return false;
                    }
                    let Some(look) = mm.look_target.value() else {
                        return false;
                    };
                    mm.walk_target.set(WalkTarget {
                        target: look,
                        speed: 0.5,
                        close: 2,
                    });
                    true
                },
                r,
            ),
            B::SetLookAndInteract(r) => one(
                || {
                    let mm = mem(m);
                    if mm.interaction_target.present() || !mm.visible_mobs.present() {
                        return false;
                    }
                    let w = cx.w;
                    let found = find_closest(m, w, |m, who| {
                        who == Who::Player
                            && w.target_info(who)
                                .is_some_and(|t| t.position.distance_squared(m.position) <= 16.0)
                    });
                    let Some(t) = found else { return false };
                    let mm = mem(m);
                    mm.interaction_target.set(t);
                    mm.look_target.set(Tracker::Entity(t, true));
                    true
                },
                r,
            ),
            B::LookAtEntity { running, what } => {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let what = *what;
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    let ok = match (what, who) {
                        (LookAt::Player, Who::Player) => true,
                        (LookAt::Species(sp), Who::Mob(id)) => {
                            w.seen(id).is_some_and(|s| s.species == sp)
                        }
                        (LookAt::Category(c), Who::Mob(id)) => {
                            w.seen(id).is_some_and(|s| s.category == c)
                        }
                        _ => false,
                    };
                    ok && w
                        .target_info(who)
                        .is_some_and(|t| t.position.distance_squared(m.position) <= 64.0)
                });
                let Some(t) = found else { return false };
                mem(m).look_target.set(Tracker::Entity(t, true));
                *running = true;
                true
            }
            B::UpdateActivity(r) => {
                brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
                *r = true;
                true
            }
            B::StrollAroundPoi {
                running,
                next_ok,
                max_dist,
                which,
            } => {
                let Some(site) = which.get(m) else {
                    return false;
                };
                if !closer_to_center(site, m.position, f64::from(*max_dist)) {
                    return false;
                }
                if now <= *next_ok {
                    *running = true;
                    return true;
                }
                let pos = m.land_random_pos(cx.w, 8, 6);
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, 0.4, 1)));
                *next_ok = now + 180;
                *running = true;
                true
            }
            B::StrollToPoi { running, next_ok } => {
                let Some(site) = mem(m).job_site.value() else {
                    return false;
                };
                if !closer_to_center(site, m.position, 10.0) {
                    return false;
                }
                if now <= *next_ok {
                    *running = true;
                    return true;
                }
                mem(m).walk_target.set(WalkTarget::block(site, 0.4, 1));
                *next_ok = now + 80;
                *running = true;
                true
            }
            B::StrollToPoiList { running, next_ok } => {
                let mm = mem(m);
                let (Some(list), Some(site)) = (mm.secondary_job_site.value(), mm.job_site.value())
                else {
                    return false;
                };
                if list.is_empty() {
                    return false;
                }
                let target = list[cx.lr.next_int_bound(list.len() as i32).unwrap_or(0) as usize];
                if !closer_to_center(site, m.position, 6.0) {
                    return false;
                }
                if now > *next_ok {
                    mem(m).walk_target.set(WalkTarget::block(target, 0.5, 1));
                    *next_ok = now + 100;
                }
                *running = true;
                true
            }
            B::WalkToBlockMemory {
                running,
                which,
                speed,
                close,
                too_far,
                too_long,
            } => {
                let r = walk_to_block_memory(m, cx, *which, *speed, *close, *too_far, *too_long);
                *running = r;
                r
            }
            B::SleepInBed { t, next_ok } => {
                if m.extra.mount.is_some() {
                    return false;
                }
                let mm = mem(m);
                let Some(home) = mm.home.value() else {
                    return false;
                };
                if let Some(woken) = mm.last_woken.value() {
                    let since = cx.w.game_time - woken;
                    if since > 0 && since < 100 {
                        return false;
                    }
                }
                let w = cx.w;
                if !(closer_to_center(home, m.position, 2.0)
                    && w.block_name(home[0], home[1], home[2]).ends_with("_bed")
                    && w.prop(home[0], home[1], home[2], "occupied") != Some("true"))
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                if now > *next_ok {
                    close_doors(m, cx, None, None);
                    if super::villager::start_sleeping(m, home, w, cx.events) {
                        mem(m).last_slept.set(now);
                    }
                    let mm = mem(m);
                    mm.walk_target.clear();
                    mm.cant_reach_walk_target_since.clear();
                }
                true
            }
            B::ClosestHomeAsWalkTarget {
                running,
                last_update,
                cache,
            } => {
                let mm = mem(m);
                if mm.walk_target.present() || mm.home.present() {
                    return false;
                }
                if cx.w.game_time - *last_update < 20 {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                let homes = homes_in_range(m, w, 48);
                let Some(closest) = homes.iter().min_by_key(|p| dist_sqr(**p, here)).copied()
                else {
                    return false;
                };
                if dist_sqr(closest, here) <= 4 {
                    return false;
                }
                let mut tried = 0;
                *last_update = cx.w.game_time + i64::from(cx.lr.next_int_bound(20).unwrap_or(0));
                let mut targets = Vec::new();
                for p in homes {
                    if cache.iter().any(|(q, _)| *q == p) {
                        continue;
                    }
                    tried += 1;
                    if tried >= 5 {
                        continue;
                    }
                    cache.push((p, *last_update + 40));
                    targets.push(p);
                }
                let path = if targets.is_empty() {
                    None
                } else {
                    m.create_path_to_any(w, &targets, 1)
                };
                if let Some(path) = path.filter(|p| p.reached) {
                    mem(m)
                        .walk_target
                        .set(WalkTarget::block(path.target, 0.5, 1));
                } else if tried < 5 {
                    let limit = *last_update;
                    cache.retain(|(_, until)| *until >= limit);
                }
                *running = true;
                true
            }
            B::InsideBrownianWalk { running, random } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                if w.can_see_sky(here[0], here[1], here[2]) {
                    return false;
                }
                let mut poses = Vec::new();
                for z in -1..=1 {
                    for y in -1..=1 {
                        for x in -1..=1 {
                            poses.push([here[0] + x, here[1] + y, here[2] + z]);
                        }
                    }
                }
                // Collections.shuffle (its own random).
                for i in (1..poses.len()).rev() {
                    let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                    poses.swap(i, j);
                }
                if let Some(p) = poses
                    .into_iter()
                    .find(|p| !w.can_see_sky(p[0], p[1], p[2]) && w.solid(p[0], p[1], p[2]))
                {
                    mem(m).walk_target.set(WalkTarget::block(p, 0.5, 0));
                }
                *running = true;
                true
            }
            B::GoToClosestVillage(r) => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let villages = village_sections(m, w);
                let section_of = |p: [i32; 3]| [p[0] >> 4, p[1] >> 4, p[2] >> 4];
                let here = sections_to_village(&villages, section_of(block_of(m.position)));
                if here <= 1 {
                    return false;
                }
                let mut target = None;
                for _ in 0..5 {
                    let pos = m.land_random_pos_scored(w, 15, 7, |p| {
                        -f64::from(sections_to_village(&villages, section_of(p)))
                    });
                    if let Some(p) = pos {
                        let b = [
                            p[0].floor() as i32,
                            p[1].floor() as i32,
                            p[2].floor() as i32,
                        ];
                        let d = sections_to_village(&villages, section_of(b));
                        if d < here {
                            target = Some(p);
                            break;
                        }
                        if d == here {
                            target = Some(p);
                        }
                    }
                }
                if let Some(p) = target {
                    mem(m).walk_target.set(WalkTarget::point(p, 0.5, 4));
                }
                *r = true;
                true
            }
            B::TriggerGate {
                running,
                random,
                kids,
            } => {
                for k in kids.iter_mut() {
                    let r = random.next_float();
                    k.2 = -horizon_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
                }
                kids.sort_by(|a, b| a.2.total_cmp(&b.2));
                for k in kids.iter_mut() {
                    let fired = k.0.try_start(m, cx);
                    k.0.one_shot_stop();
                    if fired {
                        break;
                    }
                }
                *running = true;
                true
            }
            B::SocializeAtBell(r) => one(
                || {
                    let mm = mem(m);
                    let Some(bell) = mm.meeting_point.value() else {
                        return false;
                    };
                    if !mm.visible_mobs.present() || mm.interaction_target.present() {
                        return false;
                    }
                    let w = cx.w;
                    let is_villager = |who: Who| matches!(who, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Villager));
                    if cx.lr.next_int_bound(100).unwrap_or(1) != 0
                        || !closer_to_center(bell, m.position, 4.0)
                        || find_closest(m, w, |_, who| is_villager(who)).is_none()
                    {
                        return false;
                    }
                    if let Some(t) = find_closest(m, w, |m, who| {
                        is_villager(who)
                            && w.target_info(who)
                                .is_some_and(|i| i.position.distance_squared(m.position) <= 32.0)
                    }) {
                        let mm = mem(m);
                        mm.interaction_target.set(t);
                        mm.look_target.set(Tracker::Entity(t, true));
                        mm.walk_target.set(WalkTarget {
                            target: Tracker::Entity(t, false),
                            speed: 0.3,
                            close: 1,
                        });
                    }
                    true
                },
                r,
            ),
            B::CalmDown(r) => {
                let w = cx.w;
                let mm = mem(m);
                let scared = mm.hurt_by.present()
                    || mm.nearest_hostile.present()
                    || mm.hurt_by_entity.value().is_some_and(|e| {
                        w.target_info(e)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 36.0)
                    });
                if !scared {
                    let mm = mem(m);
                    mm.hurt_by.clear();
                    mm.hurt_by_entity.clear();
                    brain_mut(m).update_from_schedule(w.game_time, w.day_time);
                }
                *r = true;
                true
            }
            B::WalkAwayFrom { running, hostile } => {
                let w = cx.w;
                let mm = mem(m);
                let from = if *hostile {
                    mm.nearest_hostile.value()
                } else {
                    mm.hurt_by_entity.value()
                };
                let Some(from) = from else { return false };
                if mm.walk_target.present() {
                    return false;
                }
                let Some(avoid) = w.target_info(from).map(|t| t.position) else {
                    return false;
                };
                if m.position.distance_squared(avoid) >= 36.0 {
                    return false;
                }
                for _ in 0..10 {
                    if let Some(p) = m.land_random_pos_away(w, 16, 7, [avoid.x, avoid.y, avoid.z]) {
                        mem(m).walk_target.set(WalkTarget::point(p, 0.75, 0));
                        break;
                    }
                }
                *running = true;
                true
            }
            B::RingBell(r) => one(
                || {
                    let Some(bell) = mem(m).meeting_point.value() else {
                        return false;
                    };
                    if cx.lr.next_float() <= 0.95 {
                        return false;
                    }
                    if dist_sqr(bell, block_of(m.position)) < 9
                        && cx.w.block_name(bell[0], bell[1], bell[2]) == "minecraft:bell"
                    {
                        cx.events.push(MobEvent::RingBell { bell });
                    }
                    true
                },
                r,
            ),
            B::ResetRaidStatus(r) => one(
                || {
                    if cx.lr.next_int_bound(20).unwrap_or(0) != 0 {
                        return false;
                    }
                    if super::villager::raid_over_or_lost(m) {
                        let b = brain_mut(m);
                        b.default_activity = Act::Idle;
                        b.update_from_schedule(cx.w.game_time, cx.w.day_time);
                    }
                    true
                },
                r,
            ),
            B::RaidSequence {
                running,
                victory,
                then,
            } => {
                let state = super::villager::raid_status(m);
                let ok = if *victory {
                    state == super::villager::RAID_VICTORY
                } else {
                    state == super::villager::RAID || state == super::villager::PRE_RAID
                };
                if !ok {
                    return false;
                }
                let fired = then.try_start(m, cx);
                then.one_shot_stop();
                *running = fired;
                fired
            }
            B::MoveToSkySeeingSpot { running, speed } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                if w.can_see_sky(here[0], here[1], here[2]) {
                    return false;
                }
                for _ in 0..10 {
                    let p = [
                        here[0] + m.random.next_int_bound(20).unwrap_or(0) - 10,
                        here[1] + m.random.next_int_bound(6).unwrap_or(0) - 3,
                        here[2] + m.random.next_int_bound(20).unwrap_or(0) - 10,
                    ];
                    if no_blocks_above(m, w, p) {
                        mem(m).walk_target.set(WalkTarget::block(p, *speed, 0));
                        break;
                    }
                }
                *running = true;
                true
            }
            B::Celebrate(t) => {
                let here = block_of(m.position);
                if super::villager::raid_status(m) != super::villager::RAID_VICTORY
                    || !no_blocks_above(m, cx.w, here)
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::LocateHidingPlace {
                running,
                radius,
                speed,
                close,
            } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let near = homes_in_range(m, w, *close + 1)
                    .into_iter()
                    .next()
                    .filter(|p| closer_to_center(*p, m.position, f64::from(*close)));
                let place = near
                    .or_else(|| {
                        let mut all = homes_in_range(m, w, *radius);
                        // Util.toShuffledList with the mob's random.
                        for i in (2..=all.len()).rev() {
                            let j = m.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                            all.swap(i - 1, j);
                        }
                        all.into_iter().next()
                    })
                    .or_else(|| mem(m).home.value());
                if let Some(p) = place {
                    let far = !closer_to_center(p, m.position, f64::from(*close));
                    let mm = mem(m);
                    mm.path.clear();
                    mm.look_target.clear();
                    mm.breed_target.clear();
                    mm.interaction_target.clear();
                    mm.hiding_place.set(p);
                    if far {
                        mm.walk_target.set(WalkTarget::block(p, *speed, *close));
                    }
                }
                *running = true;
                true
            }
            B::SetHiddenState { running, hidden } => {
                let mm = mem(m);
                let (Some(place), Some(heard)) =
                    (mm.hiding_place.value(), mm.heard_bell_time.value())
                else {
                    return false;
                };
                let timed_out = heard + 300 <= now;
                if *hidden <= 300 && !timed_out {
                    if dist_sqr(place, block_of(m.position)) < 9 {
                        *hidden += 1;
                    }
                } else {
                    let mm = mem(m);
                    mm.heard_bell_time.clear();
                    mm.hiding_place.clear();
                    brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
                    *hidden = 0;
                }
                *running = true;
                true
            }
            B::PlayTag(r) => one(
                || {
                    let mm = mem(m);
                    let Some(friends) = mm.visible_villager_babies.value() else {
                        return false;
                    };
                    if mm.walk_target.present() {
                        return false;
                    }
                    if cx.lr.next_int_bound(10).unwrap_or(0) != 0 {
                        return false;
                    }
                    // What a friend chases is its INTERACTION_TARGET.
                    let chasing = |who: Who| match who {
                        Who::Mob(id) => {
                            cx.w.seen(id)
                                .and_then(|s| s.villager.as_ref())
                                .and_then(|v| v.interaction_target)
                        }
                        Who::Player => None,
                    };
                    let me = Who::Mob(m.id);
                    if !friends.iter().any(|&f| chasing(f) == Some(me)) {
                        // The kid chased by the fewest (one to five) chasers, else any friend.
                        let mut chased: Vec<(Who, i32)> = Vec::new();
                        for &f in &friends {
                            if let Some(c) = chasing(f) {
                                match chased.iter_mut().find(|(k, _)| *k == c) {
                                    Some(e) => e.1 += 1,
                                    None => chased.push((c, 1)),
                                }
                            }
                        }
                        let target = chased
                            .iter()
                            .filter(|(_, n)| *n > 0 && *n <= 5)
                            .min_by_key(|(_, n)| *n)
                            .map(|&(k, _)| k)
                            .or_else(|| friends.first().copied());
                        if let Some(kid) = target {
                            let mm = mem(m);
                            mm.interaction_target.set(kid);
                            mm.look_target.set(Tracker::Entity(kid, true));
                            mm.walk_target.set(WalkTarget {
                                target: Tracker::Entity(kid, false),
                                speed: 0.6,
                                close: 1,
                            });
                        }
                    } else {
                        // Chased: run to a spot inside the village.
                        let villages = village_sections(m, cx.w);
                        for _ in 0..10 {
                            let Some(p) = m.land_random_pos(cx.w, 20, 8) else {
                                continue;
                            };
                            let at = [
                                p[0].floor() as i32,
                                p[1].floor() as i32,
                                p[2].floor() as i32,
                            ];
                            let section = [at[0] >> 4, at[1] >> 4, at[2] >> 4];
                            if sections_to_village(&villages, section) <= 1 {
                                mem(m).walk_target.set(WalkTarget::point(p, 0.6, 0));
                                break;
                            }
                        }
                    }
                    true
                },
                r,
            ),
        }
    }

    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot_stop() {
            return;
        }
        let now = cx.now;
        match self {
            B::Gate(g) => g.tick_or_stop(m, cx),
            B::Swim(t) => {
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
            B::PanicTrigger(t) => {
                if now > t.end || !panic_condition(m) {
                    self.do_stop(m, cx);
                } else if now % 100 == 0 {
                    cx.events.push(MobEvent::GolemCheck {
                        mob: m.id,
                        needed: 3,
                    });
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
                // tick
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
            B::FollowTradingPlayer(_) => {
                if !follow_trading_condition(m, cx.w) {
                    self.do_stop(m, cx);
                } else {
                    follow_trading_player(m);
                }
            }
            B::GoToPotentialJobSite(t) => {
                let site = mem(m).potential_job_site.value();
                if now > t.end || site.is_none() {
                    self.do_stop(m, cx);
                } else if let Some(p) = site {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Block(p));
                    mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
                }
            }
            B::JumpOnBed {
                bed,
                time_left,
                jumps,
                cooldown,
                ..
            } => {
                let w = cx.w;
                let here = block_of(m.position);
                let jumpable = |p: [i32; 3]| w.block_name(p[0], p[1], p[2]).ends_with("_bed");
                let on_or_over = jumpable(here) || jumpable([here[0], here[1] - 1, here[2]]);
                let still = m.is_baby()
                    && bed.is_some_and(jumpable)
                    && (on_or_over || *time_left > 0)
                    && (!on_or_over || *jumps > 0);
                if !still {
                    self.do_stop(m, cx);
                    return;
                }
                if !on_or_over {
                    *time_left -= 1;
                } else if *cooldown > 0 {
                    *cooldown -= 1;
                } else if jumpable(here) {
                    m.ai.jump = true;
                    *jumps -= 1;
                    *cooldown = 5;
                }
            }
            B::DoNothing(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::GiveGiftToHero {
                t, given, started, ..
            } => {
                if now > t.end || !hero_visible(m, cx.w) || *given {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(p) = cx.w.player.clone() else { return };
                mem(m).look_target.set(Tracker::Entity(Who::Player, true));
                let close = {
                    let a = block_of(p.position);
                    let b = block_of(m.position);
                    let d = (a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2);
                    f64::from(d) < 25.0
                };
                if close {
                    if now - *started > 20 {
                        cx.events.push(MobEvent::ThrowGift {
                            table: super::villager::gift_table(m),
                            from: [m.position.x, m.eye_y() - 0.3, m.position.z],
                            to: [p.position.x, p.position.y, p.position.z],
                        });
                        *given = true;
                    }
                } else {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Entity(Who::Player, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(Who::Player, true),
                        speed: 0.5,
                        close: 5,
                    });
                }
            }
            B::ShowTrades { t, look_time } => {
                let still = show_trades_condition(m, cx.w)
                    && *look_time > 0
                    && mem(m).interaction_target.present();
                if now > t.end || !still {
                    self.do_stop(m, cx);
                    return;
                }
                if let Some(target) = mem(m).interaction_target.value() {
                    mem(m).look_target.set(Tracker::Entity(target, true));
                }
                // No offer matches the player's item: look for a while only.
                *look_time = (*look_time).min(40);
                *look_time -= 1;
            }
            B::TradeWithVillager(t) => {
                if now > t.end || !villager_target_valid(m, cx.w, Mem::InteractionTarget) {
                    self.do_stop(m, cx);
                    return;
                }
                if let Some(target) = mem(m).interaction_target.value()
                    && let Some(i) = cx.w.target_info(target)
                    && i.position.distance_squared(m.position) <= 5.0
                {
                    let Who::Mob(other) = target else { return };
                    lock_gaze(m, target, cx);
                    cx.events.push(MobEvent::VillagerMeet { mob: m.id, other });
                }
            }
            B::MakeLove { birth, .. } => {
                if now > *birth || !breeding_possible(m, cx.w) {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(target) = mem(m).breed_target.value() else {
                    return;
                };
                let Who::Mob(partner) = target else { return };
                if cx
                    .w
                    .target_info(target)
                    .is_some_and(|t| t.position.distance_squared(m.position) <= 5.0)
                {
                    lock_gaze(m, target, cx);
                    if now >= *birth {
                        // The parent's half happens in its own tick: it eats,
                        // takes a bed for the child, picks its type and rests.
                        super::villager::ext_mut(m).eat_and_digest();
                        let child = take_bed_for_child(m, cx.w).map(|bed| {
                            // Villager.getBreedOffspring: half the biome's type,
                            // a quarter each parent's.
                            let roll = m.random.next_double();
                            let variant = if roll < 0.5 {
                                None
                            } else if roll < 0.75 {
                                Some(m.extra.variant)
                            } else {
                                Some(cx.w.seen(partner).map_or(m.extra.variant, |s| s.variant))
                            };
                            m.extra.age = 6000;
                            (bed, variant)
                        });
                        cx.events.push(MobEvent::VillagerBirth {
                            parent: m.id,
                            partner,
                            child,
                        });
                    } else if m.random.next_int_bound(35).unwrap_or(1) == 0 {
                        // Entity event 12 for the partner, then this villager:
                        // AbstractVillager.addParticlesAroundSelf(HEART).
                        let mut bodies = Vec::new();
                        if let Some(t) = cx.w.target_info(target) {
                            let (min, max) = (t.bounds.min_position(), t.bounds.max_position());
                            bodies.push((partner, t.position, max.x - min.x, max.y - min.y));
                        }
                        bodies.push((
                            m.id,
                            m.position,
                            f64::from(m.width()),
                            f64::from(m.height()),
                        ));
                        for (mob, feet, width, height) in bodies {
                            cx.events.push(MobEvent::AroundSelf {
                                mob,
                                event: 12,
                                kind: "heart",
                                feet: [feet.x, feet.y, feet.z],
                                width,
                                height,
                                burst: crate::particles::Burst::VILLAGER,
                            });
                        }
                    }
                }
            }
            B::WorkAtPoi { t, .. } => {
                let still = mem(m)
                    .job_site
                    .value()
                    .is_some_and(|site| closer_to_center(site, m.position, 1.73));
                if now > t.end || !still {
                    self.do_stop(m, cx);
                }
            }
            B::HarvestFarmland {
                t,
                next_ok,
                target,
                valid,
                worked,
            } => {
                if now > t.end || *worked >= 200 {
                    self.do_stop(m, cx);
                    return;
                }
                if target.is_none_or(|p| closer_to_center(p, m.position, 1.0)) {
                    harvest_tick(m, cx, next_ok, target, valid);
                    *worked += 1;
                }
            }
            B::SleepInBed { .. } => {
                let still = mem(m).home.value().is_some_and(|bed| {
                    brain_mut(m).is_active(Act::Rest)
                        && m.position.y > f64::from(bed[1]) + 0.4
                        && closer_to_center(bed, m.position, 1.14)
                });
                if !still {
                    self.do_stop(m, cx);
                }
            }
            B::Celebrate(t) => {
                if now > t.end || !super::villager::raid_nearby(m) {
                    self.do_stop(m, cx);
                    return;
                }
                if m.random.next_int_bound(100).unwrap_or(1) == 0 {
                    m.voice("entity.villager.celebrate", cx.events);
                }
                if m.random.next_int_bound(200).unwrap_or(1) == 0
                    && no_blocks_above(m, cx.w, block_of(m.position))
                {
                    let color = m.random.next_int_bound(16).unwrap_or(0) as usize;
                    let flight = m.random.next_int_bound(3).unwrap_or(0) as u8;
                    cx.events.push(MobEvent::Firework {
                        at: [m.position.x, m.eye_y(), m.position.z],
                        color,
                        flight,
                    });
                }
            }
            B::UseBonemeal {
                t,
                next_work,
                worked,
                crop,
                repick,
                ..
            } => {
                if *repick {
                    *repick = false;
                    *crop = pick_bonemeal_crop(m, cx);
                    bonemeal_target(m, *crop);
                }
                // canStillUse: fewer than 80 work cycles and a crop to work on.
                if now > t.end || *worked >= 80 || crop.is_none() {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(p) = *crop else { return };
                if now >= *next_work && closer_to_center(p, m.position, 1.0) {
                    let e = super::villager::ext_mut(m);
                    // BoneMealItem.growCrop: a CropBlock below max age takes the meal.
                    if let Some(slot) = e
                        .inventory
                        .iter()
                        .position(|s| s.count > 0 && &*s.item == "minecraft:bone_meal")
                        && bonemeal_valid(cx.w, p)
                    {
                        e.inventory[slot].count -= 1;
                        if e.inventory[slot].count == 0 {
                            e.inventory.remove(slot);
                        }
                        cx.events.push(MobEvent::Bonemeal { pos: p });
                        *repick = true;
                        *next_work = now + 40;
                    }
                    *worked += 1;
                }
            }
            _ => {}
        }
    }

    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot_stop() {
            return;
        }
        let now = cx.now;
        match self {
            B::Gate(g) => g.do_stop(m, cx),
            B::Swim(t) | B::PanicTrigger(t) | B::DoNothing(t) => t.running = false,
            B::JumpOnBed {
                t,
                bed,
                time_left,
                jumps,
                cooldown,
            } => {
                t.running = false;
                *bed = None;
                *time_left = 0;
                *jumps = 0;
                *cooldown = 0;
            }
            B::SleepInBed { t, next_ok } => {
                t.running = false;
                if super::villager::sleeping(m) {
                    super::villager::stop_sleeping(m, cx.w, cx.events);
                    mem(m).last_woken.set(now);
                    *next_ok = now + 40;
                }
            }
            B::Celebrate(t) => {
                t.running = false;
                brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
            }
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
            B::FollowTradingPlayer(t) => {
                t.running = false;
                let mm = mem(m);
                mm.walk_target.clear();
                mm.look_target.clear();
            }
            B::GoToPotentialJobSite(t) => {
                t.running = false;
                mem(m).potential_job_site.clear();
            }
            B::GiveGiftToHero { t, until_next, .. } => {
                t.running = false;
                *until_next = 600 + cx.lr.next_int_bound(6001).unwrap_or(0);
                let mm = mem(m);
                mm.interaction_target.clear();
                mm.walk_target.clear();
                mm.look_target.clear();
            }
            B::ShowTrades { t, .. } => {
                t.running = false;
                mem(m).interaction_target.clear();
            }
            B::TradeWithVillager(t) => {
                t.running = false;
                mem(m).interaction_target.clear();
            }
            B::MakeLove { t, .. } => {
                t.running = false;
                mem(m).breed_target.clear();
            }
            B::UseBonemeal {
                t, last_session, ..
            } => {
                t.running = false;
                m.extra.main_hand = None;
                *last_session = i64::from(m.age);
            }
            B::WorkAtPoi { t, .. } => t.running = false,
            B::HarvestFarmland {
                t, next_ok, worked, ..
            } => {
                t.running = false;
                *worked = 0;
                let mm = mem(m);
                mm.look_target.clear();
                mm.walk_target.clear();
                *next_ok = now + 40;
            }
            _ => {}
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
/// `Swim`: deeper than the jump threshold, or in lava.
fn swim_condition(m: &Mob) -> bool {
    m.fluids.water > m.fluid_jump_threshold() || m.fluids.in_lava
}
fn panic_condition(m: &Mob) -> bool {
    mem_ref(m).is_some_and(|mm| mm.hurt_by.present() || mm.nearest_hostile.present())
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
/// `MoveToTargetSink.reachedTarget`.
pub(super) fn reached(m: &Mob, w: &MobWorld, walk: &WalkTarget) -> bool {
    walk.target
        .block(w)
        .is_some_and(|p| manhattan(p, block_of(m.position)) <= walk.close)
}
/// `MoveToTargetSink.tryComputePath`, for the brains on these memories
/// (villager, monster, warden): `cant_reach` is the brain's
/// `CANT_REACH_WALK_TARGET_SINCE`.
pub(super) fn compute_path(
    m: &mut Mob,
    w: &MobWorld,
    now: i64,
    walk: &WalkTarget,
    path: &mut Option<Path>,
    speed: &mut f32,
    cant_reach: fn(&mut Mob) -> &mut Slot<i64>,
) -> bool {
    let Some(target) = walk.target.block(w) else {
        return false;
    };
    *path = m.create_path_to(w, target, 0);
    *speed = walk.speed;
    if reached(m, w, walk) {
        cant_reach(m).clear();
    } else {
        let can_reach = path.as_ref().is_some_and(|p| p.reached);
        let since = cant_reach(m);
        if can_reach {
            since.clear();
        } else if !since.present() {
            since.set(now);
        }
        if path.is_some() {
            return true;
        }
        let to = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]),
            f64::from(target[2]) + 0.5,
        ];
        if let Some(step) =
            m.default_random_pos_towards(w, 10, 7, to, f64::from(std::f32::consts::FRAC_PI_2))
        {
            let p = [
                step[0].floor() as i32,
                step[1].floor() as i32,
                step[2].floor() as i32,
            ];
            *path = m.create_path_to(w, p, 0);
            return path.is_some();
        }
    }
    false
}
/// `LookAndFollowTradingPlayerSink.checkExtraStartConditions`.
fn follow_trading_condition(m: &Mob, w: &MobWorld) -> bool {
    m.extra.flag2
        && m.alive()
        && !m.fluids.in_water
        && m.hurt_time <= 0
        && w.player
            .as_ref()
            .is_some_and(|p| m.distance_sqr(p.position) <= 16.0)
}
fn follow_trading_player(m: &mut Mob) {
    let mm = mem(m);
    mm.walk_target.set(WalkTarget {
        target: Tracker::Entity(Who::Player, false),
        speed: 0.5,
        close: 2,
    });
    mm.look_target.set(Tracker::Entity(Who::Player, true));
}
/// `GiveGiftToHero.isHeroVisible`.
fn hero_visible(m: &Mob, w: &MobWorld) -> bool {
    mem_ref(m).is_some_and(|mm| mm.nearest_visible_player.get() == Some(&Who::Player))
        && w.player.as_ref().is_some_and(|p| p.hero)
}
/// `ShowTradesToPlayer.checkExtraStartConditions`.
fn show_trades_condition(m: &Mob, w: &MobWorld) -> bool {
    let Some(target) = mem_ref(m).and_then(|mm| mm.interaction_target.value()) else {
        return false;
    };
    target == Who::Player
        && m.alive()
        && !m.is_baby()
        && w.target_info(target)
            .is_some_and(|t| t.alive && m.distance_sqr(t.position) <= 17.0)
}
/// `BehaviorUtils.targetIsValid` for a villager.
fn villager_target_valid(m: &mut Mob, w: &MobWorld, which: Mem) -> bool {
    let target = mem_ref(m).and_then(|mm| match which {
        Mem::InteractionTarget => mm.interaction_target.value(),
        Mem::BreedTarget => mm.breed_target.value(),
        _ => None,
    });
    let Some(Who::Mob(id)) = target else {
        return false;
    };
    w.seen(id)
        .is_some_and(|s| s.species == Species::Villager && s.alive)
        && visible_contains(m, w, Who::Mob(id))
}
/// `Villager.playWorkSound`.
fn play_work_sound(m: &mut Mob, cx: &mut Bx) {
    if let Some(sound) = super::villager::work_sound(m.extra.variant2) {
        m.voice(sound, cx.events);
    }
}
/// `WorkAtPoi.start`'s restock.
fn restock_if_due(m: &mut Mob, w: &MobWorld) {
    let day = w.day_time.div_euclid(24000);
    if let Some(t) = m.extra.trade.as_mut()
        && t.should_restock(w.game_time, day)
    {
        t.restock(w.game_time);
    }
}
/// `HarvestFarmland.validFarmlandAroundVillager`: ripe crops and bare
/// farmland around the villager.
fn farmland_around(m: &Mob, w: &MobWorld) -> Vec<[i32; 3]> {
    let mut valid = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let p = [
                    (m.position.x + f64::from(x)).floor() as i32,
                    (m.position.y + f64::from(y)).floor() as i32,
                    (m.position.z + f64::from(z)).floor() as i32,
                ];
                if farmland_valid(w, p) {
                    valid.push(p);
                }
            }
        }
    }
    valid
}
fn farmland_valid(w: &MobWorld, p: [i32; 3]) -> bool {
    super::villager::ripe_crop(w, p)
        || w.facts.is_air(w.world.block(p[0], p[1], p[2]))
            && w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:farmland"
}
/// `HarvestFarmland.getValidFarmland`: one picked by the level random.
fn pick_farmland(valid: &[[i32; 3]], cx: &mut Bx) -> Option<[i32; 3]> {
    if valid.is_empty() {
        return None;
    }
    let i = cx.lr.next_int_bound(valid.len() as i32).unwrap_or(0) as usize;
    Some(valid[i])
}
/// The seeds a villager plants (`VILLAGER_PLANTABLE_SEEDS`) and their crops.
fn seed_crop(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:wheat_seeds" => "minecraft:wheat",
        "minecraft:potato" => "minecraft:potatoes",
        "minecraft:carrot" => "minecraft:carrots",
        "minecraft:beetroot_seeds" => "minecraft:beetroots",
        "minecraft:torchflower_seeds" => "minecraft:torchflower_crop",
        "minecraft:pitcher_pod" => "minecraft:pitcher_crop",
        _ => return None,
    })
}
/// `HarvestFarmland.tick` at the farmland: harvest a ripe crop, sow bare
/// farmland, and move on from a growing crop.
fn harvest_tick(
    m: &mut Mob,
    cx: &mut Bx,
    next_ok: &mut i64,
    target: &mut Option<[i32; 3]>,
    valid: &mut Vec<[i32; 3]>,
) {
    let Some(p) = *target else { return };
    if cx.now <= *next_ok {
        return;
    }
    let w = cx.w;
    let ripe = super::villager::ripe_crop(w, p);
    let air = w.facts.is_air(w.world.block(p[0], p[1], p[2]));
    let growing = !ripe && super::villager::crop(w, p);
    if ripe {
        cx.events.push(MobEvent::BreakBlock { pos: p, drop: true });
    }
    if air && w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:farmland" {
        let e = super::villager::ext_mut(m);
        if let Some(slot) = e
            .inventory
            .iter()
            .position(|s| s.count > 0 && seed_crop(&s.item).is_some())
        {
            let crop = seed_crop(&e.inventory[slot].item).unwrap_or("minecraft:wheat");
            e.inventory[slot].count -= 1;
            if e.inventory[slot].count == 0 {
                e.inventory.remove(slot);
            }
            cx.events.push(MobEvent::SetBlockName {
                pos: p,
                name: crop,
                effect: false,
            });
            cx.events.push(MobEvent::Sound(
                "item.crop.plant",
                [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
                1.0,
                1.0,
            ));
        }
    }
    if growing {
        valid.retain(|q| *q != p);
        *target = pick_farmland(valid, cx);
        if let Some(q) = *target {
            *next_ok = cx.now + 20;
            let mm = mem(m);
            mm.walk_target.set(WalkTarget::block(q, 0.5, 1));
            mm.look_target.set(Tracker::Block(q));
        }
    }
}
/// `UseBonemeal.validPos`: a `CropBlock` below its max age.
fn bonemeal_valid(w: &MobWorld, p: [i32; 3]) -> bool {
    super::villager::crop(w, p) && !super::villager::ripe_crop(w, p)
}
/// `UseBonemeal.pickNextTarget`: one of the valid blocks around the villager,
/// each equally likely (reservoir sampling with the level random).
fn pick_bonemeal_crop(m: &Mob, cx: &mut Bx) -> Option<[i32; 3]> {
    let c = block_of(m.position);
    let mut result = None;
    let mut count = 0;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let p = [c[0] + x, c[1] + y, c[2] + z];
                if bonemeal_valid(cx.w, p) {
                    count += 1;
                    if cx.lr.next_int_bound(count).unwrap_or(1) == 0 {
                        result = Some(p);
                    }
                }
            }
        }
    }
    result
}
/// `UseBonemeal.setCurrentCropAsTarget`.
fn bonemeal_target(m: &mut Mob, crop: Option<[i32; 3]>) {
    if let Some(p) = crop {
        let mm = mem(m);
        mm.look_target.set(Tracker::Block(p));
        mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
    }
}
/// The remembered POI positions a behaviour reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoiMem {
    Job,
    Potential,
    Home,
    Meeting,
}
impl PoiMem {
    fn get(self, m: &Mob) -> Option<[i32; 3]> {
        let mm = mem_ref(m)?;
        match self {
            PoiMem::Job => mm.job_site.value(),
            PoiMem::Potential => mm.potential_job_site.value(),
            PoiMem::Home => mm.home.value(),
            PoiMem::Meeting => mm.meeting_point.value(),
        }
    }
    fn clear(self, m: &mut Mob) {
        let mm = mem(m);
        match self {
            PoiMem::Job => mm.job_site.clear(),
            PoiMem::Potential => mm.potential_job_site.clear(),
            PoiMem::Home => mm.home.clear(),
            PoiMem::Meeting => mm.meeting_point.clear(),
        }
    }
}
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> i64 {
    manhattan_sq(a, b)
}
/// Every bed head within `radius` (`PoiManager.getInRange(HOME, ..., ANY)`),
/// in the order of a scan of the cube: y, then z, then x.
fn homes_in_range(m: &Mob, w: &MobWorld, radius: i32) -> Vec<[i32; 3]> {
    let c = block_of(m.position);
    let r2 = i64::from(radius) * i64::from(radius);
    let mut out = Vec::new();
    let y0 = (c[1] - radius).max(crate::chunk::MIN_Y);
    let y1 = (c[1] + radius).min(crate::chunk::MIN_Y + crate::chunk::HEIGHT - 1);
    for qz in (c[2] - radius) >> 4..=(c[2] + radius) >> 4 {
        for qx in (c[0] - radius) >> 4..=(c[0] + radius) >> 4 {
            chunk_pois(w, (qx, qz), y0, y1, |poi, p| {
                if poi == Poi::Home && dist_sqr(p, c) <= r2 {
                    out.push(p);
                }
            });
        }
    }
    out.sort_unstable_by_key(|p| (p[1], p[2], p[0]));
    out
}
/// `MoveToSkySeeingSpot.hasNoBlocksAbove`.
fn no_blocks_above(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    w.can_see_sky(p[0], p[1], p[2])
        && f64::from(w.motion_blocking_height(p[0], p[2])) <= m.position.y
}
/// `VillagerMakeLove.isBreedingPossible`.
fn breeding_possible(m: &mut Mob, w: &MobWorld) -> bool {
    let Some(Who::Mob(id)) = mem_ref(m).and_then(|mm| mm.breed_target.value()) else {
        return false;
    };
    villager_target_valid(m, w, Mem::BreedTarget)
        && super::villager::can_breed(m)
        && w.seen(id).is_some_and(|s| s.in_love)
}
/// `VillagerMakeLove.takeVacantBed`: `PoiManager.take(HOME, ..., 48)` over a
/// sphere of 48 blocks, the nearest bed head no villager has claimed that the
/// villager can walk to (`canReach`: a path that gets there).
fn take_bed_for_child(m: &mut Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let claimed: Vec<[i32; 3]> = w
        .others
        .iter()
        .filter(|s| s.species == Species::Villager && s.id != m.id)
        .filter_map(|s| s.bed)
        .chain(m.extra.bed)
        .collect();
    let from = block_of(m.position);
    let mut beds: Vec<[i32; 3]> = homes_in_range(m, w, 48)
        .into_iter()
        .filter(|p| {
            w.prop(p[0], p[1], p[2], "part") == Some("head")
                && w.prop(p[0], p[1], p[2], "occupied") != Some("true")
                && !claimed.contains(p)
        })
        .collect();
    // Nearest first; equally near beds stay in scan order.
    beds.sort_by_key(|p| dist_sqr(*p, from));
    beds.into_iter().find(|&p| {
        m.create_path_to(w, p, Poi::Home.valid_range())
            .is_some_and(|path| path.reached)
    })
}
/// `BehaviorUtils.lockGazeAndWalkToEachOther`: this villager's half, and
/// the other villager's through a `LockGaze` event.
fn lock_gaze(m: &mut Mob, target: Who, cx: &mut Bx) {
    lock_gaze_half(m, target);
    if let Who::Mob(other) = target {
        cx.events.push(MobEvent::LockGaze {
            mob: other,
            partner: m.id,
        });
    }
}
/// One villager's half of `lockGazeAndWalkToEachOther`: look at the other
/// and walk to it.
pub fn lock_gaze_half(m: &mut Mob, target: Who) {
    if super::villager::ext(m).is_none_or(|e| e.brain.is_none()) {
        return;
    }
    let mm = mem(m);
    mm.look_target.set(Tracker::Entity(target, true));
    mm.walk_target.set(WalkTarget {
        target: Tracker::Entity(target, true),
        speed: 0.5,
        close: 2,
    });
}
/// `SetWalkTargetFromBlockMemory`.
fn walk_to_block_memory(
    m: &mut Mob,
    cx: &mut Bx,
    which: PoiMem,
    speed: f32,
    close: i32,
    too_far: i32,
    too_long: i64,
) -> bool {
    if mem(m).walk_target.present() {
        return false;
    }
    let Some(target) = which.get(m) else {
        return false;
    };
    let since = mem(m).cant_reach_walk_target_since.value();
    let here = block_of(m.position);
    if since.is_none_or(|s| cx.w.game_time - s <= too_long) {
        if manhattan(target, here) > too_far {
            let to = [
                f64::from(target[0]) + 0.5,
                f64::from(target[1]),
                f64::from(target[2]) + 0.5,
            ];
            let mut tries = 0;
            loop {
                let step = m.default_random_pos_towards(
                    cx.w,
                    15,
                    7,
                    to,
                    f64::from(std::f32::consts::FRAC_PI_2),
                );
                if let Some(s) = step {
                    let b = [
                        s[0].floor() as i32,
                        s[1].floor() as i32,
                        s[2].floor() as i32,
                    ];
                    if manhattan(b, block_of(m.position)) <= too_far {
                        mem(m).walk_target.set(WalkTarget::point(s, speed, close));
                        break;
                    }
                }
                tries += 1;
                if tries == 1000 {
                    which.clear(m);
                    mem(m).cant_reach_walk_target_since.set(cx.now);
                    return true;
                }
            }
        } else if manhattan(target, here) > close {
            mem(m)
                .walk_target
                .set(WalkTarget::block(target, speed, close));
        }
    } else {
        which.clear(m);
        mem(m).cant_reach_walk_target_since.set(cx.now);
    }
    true
}
/// `InteractWithDoor`: open the doors on the path, close the ones passed.
fn interact_with_doors(m: &mut Mob, cx: &mut Bx, from: PathNode, to: PathNode) {
    let (from, to) = ([from.x, from.y, from.z], [to.x, to.y, to.z]);
    // The door being left is remembered even when it is open already; the
    // one ahead only when this villager opens it.
    for (p, always) in [(from, true), (to, false)] {
        let Some(open) = door_open(cx.w, p) else {
            continue;
        };
        if !open {
            cx.events.push(MobEvent::SetDoorOpen { pos: p, open: true });
        }
        if always || !open {
            remember_door(m, p);
        }
    }
    close_doors(m, cx, Some(from), Some(to));
}
/// Whether the block at `p` is a door mobs open (`MOB_INTERACTABLE_DOORS`), and
/// then whether it is open.
fn door_open(w: &MobWorld, p: [i32; 3]) -> Option<bool> {
    let name = w.block_name(p[0], p[1], p[2]);
    (name.ends_with("_door") && w.has_tag("minecraft:mob_interactable_doors", name))
        .then(|| w.prop(p[0], p[1], p[2], "open") == Some("true"))
}
fn remember_door(m: &mut Mob, p: [i32; 3]) {
    let doors = &mut mem(m).doors_to_close;
    match doors.get_mut() {
        Some(list) if list.contains(&p) => {}
        Some(list) => list.push(p),
        None => doors.set(vec![p]),
    }
}
/// `InteractWithDoor.closeDoorsThatIHaveOpenedOrPassedThrough`: the remembered
/// doors other than the two of the step being taken are forgotten, and closed
/// when still open, near (3 blocks) and no other villager within 2 blocks of
/// one is passing through it.
fn close_doors(m: &mut Mob, cx: &mut Bx, from: Option<[i32; 3]>, to: Option<[i32; 3]>) {
    let Some(doors) = mem(m).doors_to_close.get().cloned() else {
        return;
    };
    let near = mem(m).mobs.value().unwrap_or_default();
    let mut kept = Vec::new();
    for door in doors {
        if from == Some(door) || to == Some(door) {
            kept.push(door);
            continue;
        }
        if !closer_to_center(door, m.position, 3.0) || door_open(cx.w, door) != Some(true) {
            continue;
        }
        let passing = near.iter().any(|&who| {
            let Who::Mob(id) = who else { return false };
            cx.w.seen(id).is_some_and(|s| {
                s.species == Species::Villager
                    && closer_to_center(door, s.position, 2.0)
                    && s.villager
                        .as_ref()
                        .and_then(|v| v.path_nodes)
                        .is_some_and(|(prev, next)| prev.is_some_and(|p| p == door || next == door))
            })
        });
        if !passing {
            cx.events.push(MobEvent::SetDoorOpen {
                pos: door,
                open: false,
            });
        }
    }
    mem(m).doors_to_close.set(kept);
}
/// The scan order of POI candidates (before the nearest-first sort).
fn pois_scan_order(_m: &Mob, found: &[(Poi, [i32; 3])]) -> Vec<usize> {
    (0..found.len()).collect()
}
/// Entity event 14: the client shows happy particles (with its own random,
/// not the mob's).
/// Entity event 14 (`AcquirePoi`, `AssignProfessionFromJobSite`):
/// `AbstractVillager.addParticlesAroundSelf(HAPPY_VILLAGER)` on the client.
fn happy_particles(m: &Mob, cx: &mut Bx) {
    cx.events.push(MobEvent::AroundSelf {
        mob: m.id,
        event: 14,
        kind: "happy_villager",
        feet: [m.position.x, m.position.y, m.position.z],
        width: f64::from(m.width()),
        height: f64::from(m.height()),
        burst: crate::particles::Burst::VILLAGER,
    });
}
/// `BehaviorUtils.findSectionClosestToVillage`.
fn closest_section(villages: &[[i32; 3]], center: [i32; 3], radius: i32) -> [i32; 3] {
    let here = sections_to_village(villages, center);
    let mut best: Option<([i32; 3], i32)> = None;
    for z in center[2] - radius..=center[2] + radius {
        for y in center[1] - radius..=center[1] + radius {
            for x in center[0] - radius..=center[0] + radius {
                let s = [x, y, z];
                let d = sections_to_village(villages, s);
                if d < here && best.is_none_or(|(_, b)| d < b) {
                    best = Some((s, d));
                }
            }
        }
    }
    best.map_or(center, |(s, _)| s)
}

// ---- Packages -----------------------------------------------------------

fn timed(min: i32, max: i32) -> Timed {
    Timed::new(min, max)
}
fn run_one(seed: i64, kids: Vec<(B, i32)>) -> B {
    B::Gate(Box::new(Gate::run_one(seed, kids)))
}
fn look_at(what: LookAt) -> B {
    B::LookAtEntity {
        running: false,
        what,
    }
}
/// `getMinimalLookBehavior`.
fn look_minimal(seed: i64) -> B {
    run_one(
        seed,
        vec![
            (look_at(LookAt::Species(Species::Villager)), 2),
            (look_at(LookAt::Player), 2),
            (B::DoNothing(timed(30, 60)), 8),
        ],
    )
}
/// `getFullLookBehavior`.
fn look_full(seed: i64) -> B {
    run_one(
        seed,
        vec![
            (look_at(LookAt::Species(Species::Cat)), 8),
            (look_at(LookAt::Species(Species::Villager)), 2),
            (look_at(LookAt::Player), 2),
            (look_at(LookAt::Category("creature")), 1),
            (look_at(LookAt::Category("water_creature")), 1),
            (look_at(LookAt::Category("axolotls")), 1),
            (look_at(LookAt::Category("underground_water_creature")), 1),
            (look_at(LookAt::Category("water_ambient")), 1),
            (look_at(LookAt::Category("monster")), 1),
            (B::DoNothing(timed(30, 60)), 2),
        ],
    )
}
fn stroll(speed: f32, xz: i32, y: i32) -> B {
    B::VillageBoundStroll {
        running: false,
        speed,
        xz,
        y,
    }
}
fn walk_to(which: PoiMem, speed: f32, close: i32, too_far: i32, too_long: i64) -> B {
    B::WalkToBlockMemory {
        running: false,
        which,
        speed,
        close,
        too_far,
        too_long,
    }
}
fn trigger_gate(seed: i64, kids: Vec<(B, i32)>) -> B {
    B::TriggerGate {
        running: false,
        random: LegacyRandom::new(seed),
        kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
    }
}
fn gift() -> B {
    B::GiveGiftToHero {
        t: timed(100, 100),
        until_next: 600,
        given: false,
        started: 0,
    }
}
fn show_trades() -> B {
    B::ShowTrades {
        t: timed(400, 1600),
        look_time: 0,
    }
}
fn interact(kind: Interact) -> B {
    B::InteractWith {
        running: false,
        kind,
    }
}
fn trade_gate(seed: i64) -> B {
    B::Gate(Box::new(Gate::ordered(
        seed,
        Mem::InteractionTarget,
        B::TradeWithVillager(timed(60, 60)),
    )))
}
fn jump_on_bed() -> B {
    B::JumpOnBed {
        t: timed(60, 60),
        bed: None,
        time_left: 0,
        jumps: 0,
        cooldown: 0,
    }
}
fn move_to_target_sink(min: i32, max: i32) -> B {
    B::MoveToTargetSink {
        t: timed(min, max),
        cooldown: 0,
        path: None,
        last_target: None,
        speed: 0.0,
    }
}
fn acquire(filter: PoiFilter, poi: Poi, only_adult: bool, event: bool) -> B {
    B::AcquirePoi {
        running: false,
        filter,
        poi,
        only_adult,
        event,
        next: 0,
        retries: Vec::new(),
    }
}
fn validate(filter: PoiFilter, which: PoiMem) -> B {
    B::ValidatePoi {
        running: false,
        filter,
        which,
    }
}
/// `VillagerGoalPackages`: the behaviours of every activity, in iteration
/// order. `speedModifier` is 0.5 throughout.
fn packages(baby: bool, seed: i64, farmer: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    let mut add = |prio: i32, act: Act, b: B| v.push(Entry { prio, act, b });
    if baby {
        // PLAY
        let p = Act::Play;
        add(0, p, move_to_target_sink(80, 120));
        add(5, p, look_full(seed));
        add(5, p, B::PlayTag(false));
        let mut play = Gate::run_one(
            seed,
            vec![
                (interact(Interact::Villager), 2),
                (interact(Interact::Cat), 1),
                (stroll(0.5, 10, 7), 1),
                (B::WalkToLookTarget(false), 1),
                (jump_on_bed(), 2),
                (B::DoNothing(timed(20, 40)), 2),
            ],
        );
        play.absent = Some(Mem::VisibleBabies);
        add(5, p, B::Gate(Box::new(play)));
        add(99, p, B::UpdateActivity(false));
    } else {
        // WORK
        let w = Act::Work;
        add(5, w, look_minimal(seed));
        add(
            5,
            w,
            run_one(
                seed,
                vec![
                    (
                        B::WorkAtPoi {
                            t: timed(60, 60),
                            last_check: 0,
                            composter: farmer,
                        },
                        7,
                    ),
                    (
                        B::StrollAroundPoi {
                            running: false,
                            next_ok: 0,
                            max_dist: 4,
                            which: PoiMem::Job,
                        },
                        2,
                    ),
                    (
                        B::StrollToPoi {
                            running: false,
                            next_ok: 0,
                        },
                        5,
                    ),
                    (
                        B::StrollToPoiList {
                            running: false,
                            next_ok: 0,
                        },
                        5,
                    ),
                    (
                        B::HarvestFarmland {
                            t: timed(60, 60),
                            next_ok: 0,
                            target: None,
                            valid: Vec::new(),
                            worked: 0,
                        },
                        if farmer { 2 } else { 5 },
                    ),
                    (
                        B::UseBonemeal {
                            t: timed(60, 60),
                            next_work: 0,
                            last_session: 0,
                            worked: 0,
                            crop: None,
                            repick: false,
                        },
                        if farmer { 4 } else { 7 },
                    ),
                ],
            ),
        );
        add(10, w, show_trades());
        add(10, w, B::SetLookAndInteract(false));
        add(2, w, walk_to(PoiMem::Job, 0.5, 9, 100, 1200));
        add(3, w, gift());
        add(99, w, B::UpdateActivity(false));
    }
    // CORE
    let c = Act::Core;
    add(0, c, B::Swim(timed(60, 60)));
    add(
        0,
        c,
        B::InteractWithDoor {
            running: false,
            last_node: None,
            cooldown: 0,
        },
    );
    add(0, c, B::LookAtTargetSink(timed(45, 90)));
    add(0, c, B::PanicTrigger(timed(60, 60)));
    add(0, c, B::WakeUp(false));
    add(0, c, B::ReactToBell(false));
    add(0, c, B::SetRaidStatus(false));
    add(0, c, validate(PoiFilter::Held, PoiMem::Job));
    add(0, c, validate(PoiFilter::Acquirable, PoiMem::Potential));
    add(1, c, move_to_target_sink(150, 250));
    add(2, c, B::PoiCompetitorScan(false));
    add(3, c, B::FollowTradingPlayer(timed(i32::MAX, i32::MAX)));
    add(5, c, B::GoToWantedItem(false));
    add(
        6,
        c,
        acquire(PoiFilter::Acquirable, Poi::Job(0), true, false),
    );
    add(7, c, B::GoToPotentialJobSite(timed(1200, 1200)));
    add(8, c, B::YieldJobSite(false));
    add(10, c, acquire(PoiFilter::Home, Poi::Home, false, true));
    add(10, c, acquire(PoiFilter::Meeting, Poi::Meeting, true, true));
    add(10, c, B::AssignProfession(false));
    add(10, c, B::ResetProfession(false));
    // MEET
    let me = Act::Meet;
    add(
        2,
        me,
        trigger_gate(
            seed,
            vec![
                (
                    B::StrollAroundPoi {
                        running: false,
                        next_ok: 0,
                        max_dist: 40,
                        which: PoiMem::Meeting,
                    },
                    2,
                ),
                (B::SocializeAtBell(false), 2),
            ],
        ),
    );
    add(10, me, show_trades());
    add(10, me, B::SetLookAndInteract(false));
    add(2, me, walk_to(PoiMem::Meeting, 0.5, 6, 100, 200));
    add(3, me, gift());
    add(3, me, validate(PoiFilter::Meeting, PoiMem::Meeting));
    add(3, me, trade_gate(seed));
    add(5, me, look_full(seed));
    add(99, me, B::UpdateActivity(false));
    // REST
    let r = Act::Rest;
    add(2, r, walk_to(PoiMem::Home, 0.5, 1, 150, 1200));
    add(3, r, validate(PoiFilter::Home, PoiMem::Home));
    add(
        3,
        r,
        B::SleepInBed {
            t: timed(60, 60),
            next_ok: 0,
        },
    );
    let mut homeless = Gate::run_one(
        seed,
        vec![
            (
                B::ClosestHomeAsWalkTarget {
                    running: false,
                    last_update: 0,
                    cache: Vec::new(),
                },
                1,
            ),
            (
                B::InsideBrownianWalk {
                    running: false,
                    random: LegacyRandom::new(seed),
                },
                4,
            ),
            (B::GoToClosestVillage(false), 2),
            (B::DoNothing(timed(20, 40)), 2),
        ],
    );
    homeless.absent = Some(Mem::Home);
    add(5, r, B::Gate(Box::new(homeless)));
    add(5, r, look_minimal(seed));
    add(99, r, B::UpdateActivity(false));
    // IDLE
    let i = Act::Idle;
    add(
        2,
        i,
        run_one(
            seed,
            vec![
                (interact(Interact::Villager), 2),
                (interact(Interact::BreedVillager), 1),
                (interact(Interact::Cat), 1),
                (stroll(0.5, 10, 7), 1),
                (B::WalkToLookTarget(false), 1),
                (jump_on_bed(), 1),
                (B::DoNothing(timed(30, 60)), 1),
            ],
        ),
    );
    add(3, i, gift());
    add(3, i, B::SetLookAndInteract(false));
    add(3, i, show_trades());
    add(3, i, trade_gate(seed));
    add(
        3,
        i,
        B::Gate(Box::new(Gate::ordered(
            seed,
            Mem::BreedTarget,
            B::MakeLove {
                t: timed(350, 350),
                birth: 0,
            },
        ))),
    );
    add(5, i, look_full(seed));
    add(99, i, B::UpdateActivity(false));
    // PANIC
    let pa = Act::Panic;
    add(0, pa, B::CalmDown(false));
    add(
        1,
        pa,
        B::WalkAwayFrom {
            running: false,
            hostile: true,
        },
    );
    add(
        1,
        pa,
        B::WalkAwayFrom {
            running: false,
            hostile: false,
        },
    );
    add(3, pa, stroll(0.75, 2, 2));
    add(5, pa, look_minimal(seed));
    // PRE_RAID
    let pr = Act::PreRaid;
    add(0, pr, B::RingBell(false));
    add(
        0,
        pr,
        trigger_gate(
            seed,
            vec![
                (walk_to(PoiMem::Meeting, 0.75, 2, 150, 200), 6),
                (stroll(0.75, 10, 7), 2),
            ],
        ),
    );
    add(5, pr, look_minimal(seed));
    add(99, pr, B::ResetRaidStatus(false));
    // RAID
    let ra = Act::Raid;
    add(
        0,
        ra,
        B::RaidSequence {
            running: false,
            victory: true,
            then: Box::new(trigger_gate(
                seed,
                vec![
                    (
                        B::MoveToSkySeeingSpot {
                            running: false,
                            speed: 0.5,
                        },
                        5,
                    ),
                    (stroll(0.55, 10, 7), 2),
                ],
            )),
        },
    );
    add(0, ra, B::Celebrate(timed(600, 600)));
    add(
        2,
        ra,
        B::RaidSequence {
            running: false,
            victory: false,
            then: Box::new(B::LocateHidingPlace {
                running: false,
                radius: 24,
                speed: 0.7,
                close: 1,
            }),
        },
    );
    add(5, ra, look_minimal(seed));
    add(99, ra, B::ResetRaidStatus(false));
    // HIDE
    let h = Act::Hide;
    add(
        0,
        h,
        B::SetHiddenState {
            running: false,
            hidden: 0,
        },
    );
    add(
        1,
        h,
        B::LocateHidingPlace {
            running: false,
            radius: 32,
            speed: 0.625,
            close: 2,
        },
    );
    add(5, h, look_minimal(seed));
    v.sort_by_key(|e| (e.prio, e.act.slot()));
    v
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
/// The active activities and the memories with values, as the oracle
/// prints them (`act core|idle`, `mem name=value...` sorted).
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
    slot!("home", mm.home, |p: &[i32; 3]| pos(*p));
    slot!("job_site", mm.job_site, |p: &[i32; 3]| pos(*p));
    slot!(
        "potential_job_site",
        mm.potential_job_site,
        |p: &[i32; 3]| pos(*p)
    );
    slot!("meeting_point", mm.meeting_point, |p: &[i32; 3]| pos(*p));
    slot!("secondary_job_site", mm.secondary_job_site, |l: &Vec<
        [i32; 3],
    >| format!(
        "n{}",
        l.len()
    ));
    slot!("mobs", mm.mobs, |l: &Vec<Who>| format!("n{}", l.len()));
    slot!("visible_mobs", mm.visible_mobs, |_: &Visible| {
        "NearestVisibleLivingEntities".to_string()
    });
    slot!(
        "visible_villager_babies",
        mm.visible_villager_babies,
        |l: &Vec<Who>| format!("n{}", l.len())
    );
    slot!(
        "nearest_players",
        mm.nearest_players,
        |l: &Vec<Who>| format!("n{}", l.len())
    );
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
        |l: &Vec<Who>| format!("n{}", l.len())
    );
    slot!("walk_target", mm.walk_target, |t: &WalkTarget| format!(
        "{}/{:x}/{}",
        tracker(t.target),
        t.speed.to_bits(),
        t.close
    ));
    slot!("look_target", mm.look_target, |t: &Tracker| tracker(*t));
    slot!("interaction_target", mm.interaction_target, |e: &Who| who(
        *e
    ));
    slot!("breed_target", mm.breed_target, |e: &Who| who(*e));
    slot!("path", mm.path, |p: &Path| format!("path{}", pos(p.target)));
    slot!(
        "doors_to_close",
        mm.doors_to_close,
        |l: &Vec<[i32; 3]>| format!("n{}", l.len())
    );
    slot!("nearest_bed", mm.nearest_bed, |p: &[i32; 3]| pos(*p));
    slot!("hurt_by", mm.hurt_by, |_: &i64| "DamageSource".to_string());
    slot!("hurt_by_entity", mm.hurt_by_entity, |e: &Who| who(*e));
    slot!("nearest_hostile", mm.nearest_hostile, |e: &Who| who(*e));
    slot!("hiding_place", mm.hiding_place, |p: &[i32; 3]| pos(*p));
    slot!("heard_bell_time", mm.heard_bell_time, |t: &i64| t
        .to_string());
    slot!(
        "cant_reach_walk_target_since",
        mm.cant_reach_walk_target_since,
        |t: &i64| t.to_string()
    );
    slot!(
        "golem_detected_recently",
        mm.golem_detected_recently,
        |t: &bool| t.to_string()
    );
    slot!("last_slept", mm.last_slept, |t: &i64| t.to_string());
    slot!("last_woken", mm.last_woken, |t: &i64| t.to_string());
    slot!("last_worked_at_poi", mm.last_worked_at_poi, |t: &i64| t
        .to_string());
    out.sort();
    Some((acts.join("|"), out.join(" ")))
}
/// `Villager.stopSleeping`: remember waking.
pub fn woken(m: &mut Mob, now: i64) {
    if brain(m).is_some() {
        mem(m).last_woken.set(now);
    }
}
/// `BellBlockEntity.updateEntities`: a bell rung within 32 blocks sets
/// `HEARD_BELL_TIME` to the game time.
pub fn heard_bell(m: &mut Mob, now: i64) {
    if brain(m).is_some() {
        mem(m).heard_bell_time.set(now);
    }
}
/// Whether `NearestItemSensor` is due for this villager (it is then reset).
pub fn take_item_scan(m: &mut Mob) -> bool {
    brain(m).is_some() && std::mem::take(&mut brain_mut(m).scan_items)
}
/// `NearestItemSensor.doTick`: the nearest wanted item in sight, or none.
pub fn set_wanted_item(m: &mut Mob, at: Option<[f64; 3]>) {
    mem(m).nearest_wanted_item.set_opt(at);
}
/// What other villagers see of this one's brain.
pub fn potential_job_site(m: &Mob) -> Option<[i32; 3]> {
    mem_ref(m).and_then(|mm| mm.potential_job_site.value())
}
/// The memories other villagers read: the interaction target, the path's
/// nodes (while it is not done), whether a golem was seen lately and the
/// last sleep.
#[allow(clippy::type_complexity)]
pub fn seen_memories(
    m: &Mob,
) -> (
    Option<Who>,
    Option<(Option<[i32; 3]>, [i32; 3])>,
    bool,
    Option<i64>,
) {
    let Some(mm) = mem_ref(m) else {
        return (None, None, false, None);
    };
    let path = mm
        .path
        .present()
        .then_some(m.ai.nav.path.as_ref())
        .flatten();
    let nodes = path.filter(|p| !p.is_done()).map(|p| {
        let n = |i: usize| [p.nodes[i].x, p.nodes[i].y, p.nodes[i].z];
        ((p.next > 0).then(|| n(p.next - 1)), n(p.next))
    });
    (
        mm.interaction_target.value(),
        nodes,
        mm.golem_detected_recently.present(),
        mm.last_slept.value(),
    )
}
