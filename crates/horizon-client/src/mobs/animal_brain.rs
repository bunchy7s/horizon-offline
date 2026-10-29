//! The `Brain`s of the sniffer (`SnifferAi`) and the ghastling
//! (`HappyGhastAi`), ported behaviour for behaviour: memories that expire,
//! the sensors and their scan rates, the activities and their
//! requirements, and the behaviours of each activity, started and ticked in
//! vanilla's order (priority, then the activity's slot in
//! `Brain.availableBehaviorsByPriority`'s hash map, then insertion order).
//!
//! The level random (`ServerLevel.getRandom`) times behaviours; the mob's
//! own random drives positions, sounds and jumps. `RunOne` shuffles with a
//! random of its own, seeded with the brain's `shuffle_seed` when it is
//! built.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    brain::{Timed, living_near},
    path::Path,
    sniffer,
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `MemorySlot.NEVER_EXPIRE`.
const NEVER: i64 = i64::MAX;

/// `MemorySlot`: a value and its time to live.
#[derive(Clone, Debug)]
pub struct Slot<T> {
    v: Option<T>,
    ttl: i64,
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
    fn tick(&mut self) {
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
    pub fn present(&self) -> bool {
        self.v.is_some()
    }
}
impl<T: Clone> Slot<T> {
    pub fn value(&self) -> Option<T> {
        self.v.clone()
    }
}
impl<T> Slot<Vec<T>> {
    /// `Brain.setMemory` of a collection: an empty one erases the memory.
    fn set_list(&mut self, v: Vec<T>) {
        if v.is_empty() {
            self.clear();
        } else {
            self.set(v);
        }
    }
}

/// `PositionTracker`: an entity (`EntityTracker`: at its eyes or feet, and
/// whether its block is the one at its eyes) or a block (`BlockPosTracker`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tracker {
    Entity {
        who: Who,
        eye: bool,
        block_eye: bool,
    },
    Block([i32; 3]),
}
impl Tracker {
    fn entity(who: Who, eye: bool) -> Self {
        Tracker::Entity {
            who,
            eye,
            block_eye: false,
        }
    }
    /// `currentPosition`.
    fn position(&self, w: &MobWorld) -> Option<[f64; 3]> {
        match *self {
            Tracker::Entity { who, eye, .. } => w.target_info(who).map(|t| {
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
        }
    }
    /// `currentBlockPosition`.
    fn block(&self, w: &MobWorld) -> Option<[i32; 3]> {
        match *self {
            Tracker::Entity { who, block_eye, .. } => w.target_info(who).map(|t| {
                if block_eye {
                    block_of(Vec3::new(t.position.x, t.eye_y, t.position.z))
                } else {
                    block_of(t.position)
                }
            }),
            Tracker::Block(p) => Some(p),
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
    /// `new WalkTarget(Vec3, ...)`: the block containing the point.
    fn point(p: [f64; 3], speed: f32, close: i32) -> Self {
        Self {
            target: Tracker::Block([
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]),
            speed,
            close,
        }
    }
}

/// `NearestVisibleLivingEntities`: the nearby entities and a lazily filled
/// cache of `Sensor.isEntityTargetable`.
#[derive(Clone, Debug, Default)]
pub struct Visible {
    list: Vec<Who>,
    cache: Vec<(Who, bool)>,
}

/// The memories these brains register.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    pub mobs: Slot<Vec<Who>>,
    pub visible_mobs: Slot<Visible>,
    pub nearest_players: Slot<Vec<Who>>,
    pub nearest_visible_player: Slot<Who>,
    pub nearest_visible_targetable_player: Slot<Who>,
    pub nearest_visible_targetable_players: Slot<Vec<Who>>,
    pub nearest_visible_adult: Slot<Who>,
    pub walk_target: Slot<WalkTarget>,
    pub look_target: Slot<Tracker>,
    pub breed_target: Slot<Who>,
    pub path: Slot<Path>,
    /// `HURT_BY`: whether the damage is one of `#panic_causes`.
    pub hurt_by: Slot<bool>,
    pub hurt_by_entity: Slot<Who>,
    pub cant_reach_walk_target_since: Slot<i64>,
    pub tempting_player: Slot<Who>,
    pub temptation_cooldown_ticks: Slot<i32>,
    pub is_panicking: Slot<bool>,
    pub sniffer_digging: Slot<bool>,
    pub sniffer_sniffing_target: Slot<[i32; 3]>,
    pub sniffer_happy: Slot<bool>,
    pub sniff_cooldown: Slot<()>,
    pub gaze_cooldown_ticks: Slot<i32>,
    pub transport_items_cooldown_ticks: Slot<i32>,
    pub nearest_attackable: Slot<Who>,
    pub attack_target: Slot<Who>,
    pub attack_cooling_down: Slot<bool>,
    pub hunting_cooldown: Slot<bool>,
    pub play_dead_ticks: Slot<i32>,
    pub visited_block_positions: Slot<Vec<[i32; 3]>>,
    pub unreachable_transport_block_positions: Slot<Vec<[i32; 3]>>,
}
impl Memories {
    fn tick(&mut self) {
        self.nearest_attackable.tick();
        self.attack_target.tick();
        self.attack_cooling_down.tick();
        self.hunting_cooldown.tick();
        self.play_dead_ticks.tick();
        self.mobs.tick();
        self.visible_mobs.tick();
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_targetable_player.tick();
        self.nearest_visible_targetable_players.tick();
        self.nearest_visible_adult.tick();
        self.walk_target.tick();
        self.look_target.tick();
        self.breed_target.tick();
        self.path.tick();
        self.hurt_by.tick();
        self.hurt_by_entity.tick();
        self.cant_reach_walk_target_since.tick();
        self.tempting_player.tick();
        self.temptation_cooldown_ticks.tick();
        self.is_panicking.tick();
        self.sniffer_digging.tick();
        self.sniffer_sniffing_target.tick();
        self.sniffer_happy.tick();
        self.sniff_cooldown.tick();
        self.gaze_cooldown_ticks.tick();
        self.transport_items_cooldown_ticks.tick();
        self.visited_block_positions.tick();
        self.unreachable_transport_block_positions.tick();
    }
}

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Core,
    Fight,
    PlayDead,
    Idle,
    Sniff,
    Dig,
    Panic,
}
impl Act {
    pub fn name(self) -> &'static str {
        match self {
            Act::Core => "core",
            Act::Fight => "fight",
            Act::PlayDead => "play_dead",
            Act::Idle => "idle",
            Act::Sniff => "sniff",
            Act::Dig => "dig",
            Act::Panic => "panic",
        }
    }
}

/// Which brain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Axolotl,
    Sniffer,
    /// A happy ghast (the brain ticks while it is a ghastling).
    Ghastling,
    /// `CopperGolemAi`.
    CopperGolem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sensor {
    Living,
    AxolotlAttackables,
    AdultSameType,
    HurtBy,
    Players,
    Tempting,
    AdultAnyType,
}
impl Kind {
    /// `BRAIN_PROVIDER`'s sensors, in order (all scan every 20 ticks).
    fn sensors(self) -> &'static [Sensor] {
        match self {
            Kind::Axolotl => &[
                Sensor::Living,
                Sensor::AdultSameType,
                Sensor::HurtBy,
                Sensor::AxolotlAttackables,
                Sensor::Tempting,
            ],
            Kind::Sniffer => &[
                Sensor::Living,
                Sensor::HurtBy,
                Sensor::Players,
                Sensor::Tempting,
            ],
            Kind::Ghastling => &[
                Sensor::Living,
                Sensor::HurtBy,
                Sensor::Tempting,
                Sensor::AdultAnyType,
                Sensor::Players,
            ],
            Kind::CopperGolem => &[Sensor::Living, Sensor::HurtBy],
        }
    }
}

/// An animal's brain.
#[derive(Clone, Debug)]
pub struct Brain {
    pub kind: Kind,
    pub mem: Memories,
    sensors: Vec<(Sensor, i64)>,
    /// The active non-core activity (`CORE` is always active).
    pub activity: Act,
    list: Vec<Entry>,
}
#[derive(Clone, Debug)]
struct Entry {
    act: Act,
    b: B,
}

fn brain(m: &Mob) -> Option<&Brain> {
    m.extra.animal_brain.as_deref()
}
fn brain_mut(m: &mut Mob) -> &mut Brain {
    m.extra
        .animal_brain
        .as_deref_mut()
        .expect("the animal brain is built before it is used")
}
pub(super) fn mem(m: &mut Mob) -> &mut Memories {
    &mut brain_mut(m).mem
}
pub(super) fn mem_ref(m: &Mob) -> Option<&Memories> {
    brain(m).map(|b| &b.mem)
}

/// The brain kind a mob has, if any.
pub fn kind_of(m: &Mob) -> Option<Kind> {
    kind_for_species(m.species())
}
fn kind_for_species(species: Species) -> Option<Kind> {
    match species {
        Species::Axolotl => Some(Kind::Axolotl),
        Species::Sniffer => Some(Kind::Sniffer),
        // Every happy ghast has the brain; only the ghastling's ticks.
        Species::HappyGhast => Some(Kind::Ghastling),
        Species::CopperGolem => Some(Kind::CopperGolem),
        _ => None,
    }
}

/// `Brain.Provider.makeBrain`: a new brain whose sensors' first scans are
/// delayed by draws from the mob's random, on its default activity (idle).
/// The gates shuffle with a random seeded with `shuffle_seed`.
pub fn init(m: &mut Mob, shuffle_seed: i64) {
    let Some(kind) = kind_of(m) else { return };
    // The copper golem's constructor: TRANSPORT_ITEMS_COOLDOWN_TICKS of
    // nextInt(60, 100).
    let cooldown =
        (kind == Kind::CopperGolem).then(|| 60 + m.random.next_int_bound(40).unwrap_or(0));
    let sensors = kind
        .sensors()
        .iter()
        .map(|s| (*s, i64::from(m.random.next_int_bound(20).unwrap_or(0))))
        .collect();
    let mut b = Brain {
        kind,
        mem: Memories::default(),
        sensors,
        activity: Act::Idle,
        list: Vec::new(),
    };
    b.list = match kind {
        Kind::Axolotl => axolotl_package(shuffle_seed),
        Kind::Sniffer => sniffer_package(shuffle_seed),
        Kind::Ghastling => ghastling_package(shuffle_seed),
        Kind::CopperGolem => copper_golem_package(shuffle_seed),
    };
    if let Some(c) = cooldown {
        b.mem.transport_items_cooldown_ticks.set(c);
    }
    m.extra.animal_brain = Some(Box::new(b));
}

/// A copper golem's `TRANSPORT_ITEMS_COOLDOWN_TICKS`, or none for 0.
pub fn set_transport_cooldown(m: &mut Mob, ticks: i32) {
    if m.extra.animal_brain.is_none() {
        return;
    }
    let slot = &mut mem(m).transport_items_cooldown_ticks;
    if ticks > 0 {
        slot.set(ticks);
    } else {
        slot.clear();
    }
}

/// The happy ghast grew up (`adultGhastSetup`): every running behaviour
/// stops (`Brain.stopAll`) and the memories are cleared; the brain no
/// longer ticks.
pub fn grown_up(m: &mut Mob) {
    let Some(b) = m.extra.animal_brain.as_deref_mut() else {
        return;
    };
    for e in b.list.iter_mut() {
        e.b.reset();
    }
    b.mem = Memories::default();
    m.nav_stop();
}

/// `AnimalMakeLove.start` on the partner's side: it remembers `partner`
/// as its breed target, looks at it and walks to it
/// (`BehaviorUtils.lockGazeAndWalkToEachOther`).
pub fn set_breed_partner(m: &mut Mob, partner: u32) {
    if m.extra.animal_brain.is_none() {
        return;
    }
    let who = Who::Mob(partner);
    mem(m).breed_target.set(who);
    lock_gaze_and_walk(
        m,
        who,
        if m.species() == Species::Axolotl {
            0.2
        } else {
            1.0
        },
        2,
    );
}

/// `AnimalMakeLove.tick` after the birth: the partner forgets this mob.
pub fn forget_breed_target(m: &mut Mob, partner: u32) {
    if m.extra.animal_brain.is_some() && mem(m).breed_target.value() == Some(Who::Mob(partner)) {
        mem(m).breed_target.clear();
    }
}
/// The partner's half of `lockGazeAndWalkToEachOther`.
pub fn lock_gaze(m: &mut Mob, partner: u32) {
    if m.extra.animal_brain.is_none() {
        return;
    }
    lock_gaze_and_walk(
        m,
        Who::Mob(partner),
        if m.species() == Species::Axolotl {
            0.2
        } else {
            1.0
        },
        2,
    );
}

/// What behaviours read and emit during a brain tick.
pub struct Bx<'a, 'w> {
    pub w: &'a MobWorld<'w>,
    pub lr: &'a mut LegacyRandom,
    pub events: &'a mut Vec<MobEvent>,
    pub now: i64,
}

/// `Brain.tick` then `updateActivity` (from `customServerAiStep`).
pub fn tick(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if brain(m).is_none() {
        let seed = i64::from(m.id);
        init(m, seed);
        if brain(m).is_none() {
            return;
        }
    }
    // forgetOutdatedMemories
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
    m.ai.panicking = is_panicking(m);
    if m.species() == Species::Axolotl {
        m.extra.axolotl.playing_dead = mem(m)
            .play_dead_ticks
            .value()
            .is_some_and(|ticks| ticks > 0);
        m.ai.target = mem(m).attack_target.value();
    }
}

/// `SnifferAi.updateActivity` / `HappyGhastAi.updateActivity`:
/// `setActiveActivityToFirstValid`.
fn update_activity(m: &mut Mob) {
    let b = brain_mut(m);
    let mm = &b.mem;
    if b.kind == Kind::Axolotl {
        update_axolotl_activity(b);
        return;
    }
    let order: &[Act] = match b.kind {
        Kind::Sniffer => &[Act::Dig, Act::Sniff, Act::Idle],
        Kind::Ghastling => &[Act::Panic, Act::Idle],
        Kind::CopperGolem => &[Act::Idle],
        Kind::Axolotl => unreachable!(),
    };
    for &a in order {
        let ok = match a {
            Act::Dig => {
                !mm.is_panicking.present()
                    && !mm.walk_target.present()
                    && mm.sniffer_digging.present()
            }
            Act::Sniff => {
                !mm.is_panicking.present()
                    && mm.sniffer_sniffing_target.present()
                    && mm.walk_target.present()
            }
            Act::Idle => b.kind != Kind::Sniffer || !mm.sniffer_digging.present(),
            Act::Panic => mm.is_panicking.present(),
            Act::Core => true,
            Act::Fight | Act::PlayDead => false,
        };
        if ok {
            b.activity = a;
            break;
        }
    }
}

/// `PathfinderMob.isPanicking` for a mob with this brain.
pub fn is_panicking(m: &Mob) -> bool {
    mem_ref(m).is_some_and(|mm| mm.is_panicking.present())
}
/// `Sniffer.isTempted`.
pub fn is_tempted(m: &Mob) -> bool {
    mem_ref(m).is_some_and(|mm| mm.tempting_player.present())
}
/// The explored positions are the sniffer's `SNIFFER_EXPLORED_POSITIONS`.
const EXPLORED_MAX: usize = 20;

// ---- Sensors ------------------------------------------------------------

fn tick_sensors(m: &mut Mob, w: &MobWorld) {
    let n = brain_mut(m).sensors.len();
    for i in 0..n {
        let (sensor, fire) = {
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
        match sensor {
            Sensor::Living => sense_living(m, w),
            Sensor::AxolotlAttackables => sense_axolotl_attackables(m, w),
            Sensor::AdultSameType => {
                let found = find_closest(m, w, |m, who| match who {
                    Who::Mob(id) => w
                        .seen(id)
                        .is_some_and(|s| !s.baby && s.species == m.species()),
                    _ => false,
                });
                mem(m).nearest_visible_adult.set_opt(found);
            }
            Sensor::HurtBy => sense_hurt(m, w),
            Sensor::Players => sense_players(m, w),
            Sensor::Tempting => sense_tempting(m, w),
            Sensor::AdultAnyType => sense_adult(m, w),
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
/// `Sensor.isEntityTargetable`: `TargetingConditions.forNonCombat()` within
/// the follow range, in sight.
fn targetable(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => {
            // TargetingConditions.forNonCombat(): in range (scaled by the
            // player's visibility, at least 2) and in sight.
            let Some(p) = &w.player else { return false };
            let r = (range * p.visibility_percent()).max(2.0);
            p.alive && !p.spectator && m.distance_sqr(p.position) <= r * r && m.sees(w, who)
        }
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive && m.distance_sqr(s.position) <= range * range && m.sees(w, who)
        }
    }
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
    let listed = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
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
/// `HurtBySensor`: the last damage (forgotten 40 ticks after) and who did it.
fn sense_hurt(m: &mut Mob, w: &MobWorld) {
    let age = i64::from(m.age);
    let last = m
        .last_damage
        .as_ref()
        .filter(|(_, t)| age - t <= 40)
        .map(|(d, _)| (d.attacker, d.causes_panic()));
    let mm = mem(m);
    if let Some((by, panic)) = last {
        mm.hurt_by.set(panic);
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
/// `TemptingSensor.forAnimal`: the nearest player within the tempt range
/// holding the animal's food (and not riding it).
fn sense_tempting(m: &mut Mob, w: &MobWorld) {
    // Attributes.TEMPT_RANGE: 10 for animals, 16 for the happy ghast.
    let range = if m.species() == Species::HappyGhast {
        16.0
    } else {
        10.0
    };
    let food = &m.kind.food;
    let holds = |item: &Option<std::sync::Arc<str>>| {
        item.as_deref()
            .is_some_and(|i| food.iter().any(|f| f.as_str() == i))
    };
    let tempting = w.player.as_ref().is_some_and(|p| {
        p.alive
            && !p.spectator
            && m.distance_sqr(p.position) <= range * range
            && (holds(&p.main_hand) || holds(&p.off_hand))
            && m.rider.is_none()
    });
    mem(m)
        .tempting_player
        .set_opt(tempting.then_some(Who::Player));
}
/// `AdultSensorAnyType`: the nearest visible adult of a followable friendly
/// type.
fn sense_adult(m: &mut Mob, w: &MobWorld) {
    if !mem(m).visible_mobs.present() {
        return;
    }
    let found = find_closest(m, w, |_, who| match who {
        Who::Mob(id) => w
            .seen(id)
            .is_some_and(|s| !s.baby && followable_friendly(s.species)),
        Who::Player => false,
    });
    mem(m).nearest_visible_adult.set_opt(found);
}
/// `#followable_friendly_mobs`.
fn followable_friendly(s: Species) -> bool {
    matches!(
        s,
        Species::Camel
            | Species::Cat
            | Species::Chicken
            | Species::Cow
            | Species::Donkey
            | Species::Fox
            | Species::Goat
            | Species::HappyGhast
            | Species::Horse
            | Species::SkeletonHorse
            | Species::Llama
            | Species::Mule
            | Species::Ocelot
            | Species::Panda
            | Species::Parrot
            | Species::Pig
            | Species::PolarBear
            | Species::Rabbit
            | Species::Sheep
            | Species::Sniffer
            | Species::Strider
            | Species::Villager
            | Species::Wolf
    )
}

// ---- Behaviours ------------------------------------------------------------

/// `RunOne`: shuffles its children by weight and starts the first that can.
#[derive(Clone, Debug)]
struct Gate {
    running: bool,
    shuffle: bool,
    try_all: bool,
    random: LegacyRandom,
    kids: Vec<(B, i32, f64)>,
    /// The gate's memory requirements.
    when: Option<fn(&Memories) -> bool>,
}
impl Gate {
    fn run_one(seed: i64, kids: Vec<(B, i32)>) -> Self {
        Self {
            running: false,
            shuffle: true,
            try_all: false,
            random: LegacyRandom::new(seed),
            kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
            when: None,
        }
    }
    fn when(mut self, f: fn(&Memories) -> bool) -> Self {
        self.when = Some(f);
        self
    }
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        if let Some(f) = self.when
            && !f(mem(m))
        {
            return false;
        }
        self.running = true;
        // ShufflingList.shuffle: a fresh weight for each entry in the
        // current order, then a stable sort.
        if self.shuffle {
            for k in &mut self.kids {
                let r = self.random.next_float();
                k.2 = -horizon_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
            }
            self.kids.sort_by(|a, b| a.2.total_cmp(&b.2));
        }
        for k in &mut self.kids {
            if !k.0.running() && k.0.try_start(m, cx) && !self.try_all {
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

#[derive(Clone, Debug)]
enum B {
    AxolotlAction {
        action: AxolotlAction,
        running: bool,
        until: i64,
    },
    PlayDead(Timed),
    Swim(Timed),
    /// `AnimalPanic`: flying (`AirAndWaterRandomPos`) or on land, and
    /// whether the sniffer's override resets its sniffing first.
    AnimalPanic {
        t: Timed,
        fly: bool,
        speed: f32,
    },
    MoveToTargetSink {
        t: Timed,
        cooldown: i32,
        path: Option<Path>,
        last_target: Option<[i32; 3]>,
        speed: f32,
    },
    CountDownTemptation(Timed),
    MakeLove {
        t: Timed,
        spawn_at: i64,
    },
    /// `FollowTemptation` and how close is close enough.
    FollowTemptation {
        t: Timed,
        look_in_the_eyes: bool,
    },
    LookAtTargetSink(Timed),
    FeelingHappy(Timed),
    Digging(Timed),
    FinishedDigging(Timed),
    Searching(Timed),
    Scenting(Timed),
    Sniffing(Timed),
    DoNothing(Timed),
    /// `SetWalkTargetFromLookTarget.create(speed, close)`.
    WalkToLookTarget {
        running: bool,
        speed: f32,
        close: i32,
    },
    /// `SetEntityLookTarget.create(EntityType.PLAYER, 6)`.
    LookAtPlayer(bool),
    /// `RandomStroll.stroll` or `RandomStroll.fly`.
    Stroll {
        running: bool,
        fly: bool,
        speed: f32,
        /// Horizontal and vertical range (land strolls).
        range: (i32, i32),
    },
    /// `BabyFollowAdult.create(3..16, 1.1, memory, true)`: the nearest
    /// visible player or adult.
    FollowAdult {
        running: bool,
        player: bool,
    },
    Gate(Box<Gate>),
    /// `CountDownCooldownTicks(GAZE_COOLDOWN_TICKS)` and
    /// `(TRANSPORT_ITEMS_COOLDOWN_TICKS)`.
    CountDownGaze(Timed),
    CountDownTransport(Timed),
    /// `SetEntityLookTargetSometimes.create(PLAYER, 6, 40..80)`: running,
    /// and the ticker's ticks until the next start.
    LookAtPlayerSometimes(bool, i32),
    /// `TransportItemsBetweenContainers`.
    Transport(Box<super::transport_items::Transport>),
}

impl B {
    fn running(&self) -> bool {
        match self {
            B::PlayDead(t)
            | B::Swim(t)
            | B::AnimalPanic { t, .. }
            | B::MoveToTargetSink { t, .. }
            | B::CountDownTemptation(t)
            | B::MakeLove { t, .. }
            | B::FollowTemptation { t, .. }
            | B::LookAtTargetSink(t)
            | B::FeelingHappy(t)
            | B::Digging(t)
            | B::FinishedDigging(t)
            | B::Searching(t)
            | B::Scenting(t)
            | B::Sniffing(t)
            | B::DoNothing(t)
            | B::CountDownGaze(t)
            | B::CountDownTransport(t) => t.running,
            B::Transport(x) => x.running,
            B::LookAtPlayerSometimes(running, _) => *running,
            B::AxolotlAction { running, .. }
            | B::WalkToLookTarget { running, .. }
            | B::LookAtPlayer(running)
            | B::Stroll { running, .. }
            | B::FollowAdult { running, .. } => *running,
            B::Gate(g) => g.running,
        }
    }
    /// Stopped without its stop effects (the memories they would touch are
    /// cleared with the rest).
    fn reset(&mut self) {
        match self {
            B::PlayDead(t)
            | B::Swim(t)
            | B::AnimalPanic { t, .. }
            | B::CountDownTemptation(t)
            | B::MakeLove { t, .. }
            | B::FollowTemptation { t, .. }
            | B::LookAtTargetSink(t)
            | B::FeelingHappy(t)
            | B::Digging(t)
            | B::FinishedDigging(t)
            | B::Searching(t)
            | B::Scenting(t)
            | B::Sniffing(t)
            | B::DoNothing(t)
            | B::CountDownGaze(t)
            | B::CountDownTransport(t) => t.running = false,
            B::Transport(x) => x.running = false,
            B::MoveToTargetSink { t, path, .. } => {
                t.running = false;
                *path = None;
            }
            B::Gate(g) => {
                g.running = false;
                for k in &mut g.kids {
                    k.0.reset();
                }
            }
            _ => self.set_one_shot(false),
        }
    }
    /// A one-shot (`OneShot`) runs only until its first tick.
    fn one_shot(&self) -> bool {
        matches!(
            self,
            B::AxolotlAction { .. }
                | B::WalkToLookTarget { .. }
                | B::LookAtPlayer(_)
                | B::LookAtPlayerSometimes(..)
                | B::Stroll { .. }
                | B::FollowAdult { .. }
        )
    }
    fn set_one_shot(&mut self, on: bool) {
        match self {
            B::AxolotlAction { running, .. }
            | B::WalkToLookTarget { running, .. }
            | B::LookAtPlayer(running)
            | B::LookAtPlayerSometimes(running, _)
            | B::Stroll { running, .. }
            | B::FollowAdult { running, .. } => *running = on,
            _ => {}
        }
    }

    /// `BehaviorControl.tryStart`.
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        if self.one_shot() {
            let ok = self.trigger(m, cx);
            if ok {
                self.set_one_shot(true);
            }
            return ok;
        }
        let now = cx.now;
        match self {
            B::Gate(g) => g.try_start(m, cx),
            B::PlayDead(t) => {
                if !m.fluids.in_water
                    || !mem(m).play_dead_ticks.present()
                    || !mem(m).hurt_by_entity.present()
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                mem(m).walk_target.clear();
                mem(m).look_target.clear();
                m.add_effect("minecraft:regeneration", 200, 0);
                true
            }
            B::Swim(t) => {
                if !swim_condition(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::AnimalPanic { t, .. } => {
                let mm = mem(m);
                if !(mm.hurt_by.get() == Some(&true) || mm.is_panicking.present()) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                if brain_mut(m).kind == Kind::Sniffer {
                    reset_sniffing(m, cx);
                }
                let mm = mem(m);
                mm.is_panicking.set(true);
                mm.walk_target.clear();
                m.nav_stop();
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
                if *cooldown > 0 {
                    *cooldown -= 1;
                    return false;
                }
                let walk = mm.walk_target.value().unwrap();
                let reached = reached(m, cx.w, &walk);
                if !reached && compute_path(m, cx, &walk, path, speed) {
                    *last_target = walk.target.block(cx.w);
                    t.begin(cx.lr, cx.now);
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
            B::CountDownTemptation(t) => {
                if !mem(m).temptation_cooldown_ticks.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::CountDownGaze(t) => {
                if !mem(m).gaze_cooldown_ticks.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::CountDownTransport(t) => {
                if !mem(m).transport_items_cooldown_ticks.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::Transport(x) => x.try_start(m, cx),
            B::MakeLove { t, spawn_at } => {
                let mm = mem(m);
                if !mm.visible_mobs.present()
                    || mm.breed_target.present()
                    || mm.is_panicking.present()
                {
                    return false;
                }
                if m.extra.in_love <= 0 {
                    return false;
                }
                let Some(partner) = breed_partner(m, cx.w) else {
                    return false;
                };
                t.begin(cx.lr, cx.now);
                if brain_mut(m).kind == Kind::Sniffer {
                    reset_sniffing(m, cx);
                }
                mem(m).breed_target.set(partner);
                if let Who::Mob(id) = partner {
                    cx.events.push(MobEvent::BreedTarget {
                        mob: id,
                        partner: m.id,
                    });
                }
                let speed = if m.species() == Species::Axolotl {
                    0.2
                } else {
                    1.0
                };
                lock_gaze_and_walk(m, partner, speed, 2);
                *spawn_at = now + 60 + i64::from(m.random.next_int_bound(50).unwrap_or(0));
                true
            }
            B::FollowTemptation { t, .. } => {
                let mm = mem(m);
                if mm.temptation_cooldown_ticks.present()
                    || !mm.tempting_player.present()
                    || mm.breed_target.present()
                    || mm.is_panicking.present()
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                if brain_mut(m).kind == Kind::Sniffer {
                    reset_sniffing(m, cx);
                }
                true
            }
            B::LookAtTargetSink(t) => {
                if !mem(m).look_target.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                true
            }
            B::FeelingHappy(t) => {
                if !mem(m).sniffer_happy.present() {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::FEELING_HAPPY);
                true
            }
            B::Digging(t) => {
                let mm = mem(m);
                if mm.is_panicking.present()
                    || mm.walk_target.present()
                    || !mm.sniffer_digging.present()
                    || mm.sniff_cooldown.present()
                {
                    return false;
                }
                if !can_sniff(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::DIGGING);
                true
            }
            B::FinishedDigging(t) => {
                let mm = mem(m);
                if mm.is_panicking.present()
                    || mm.walk_target.present()
                    || !mm.sniffer_digging.present()
                    || !mm.sniff_cooldown.present()
                {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::RISING);
                true
            }
            B::Searching(t) => {
                let mm = mem(m);
                if !mm.walk_target.present()
                    || mm.is_panicking.present()
                    || !mm.sniffer_sniffing_target.present()
                {
                    return false;
                }
                if !can_sniff(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::SEARCHING);
                true
            }
            B::Scenting(t) => {
                let mm = mem(m);
                if mm.is_panicking.present()
                    || mm.sniffer_digging.present()
                    || mm.sniffer_sniffing_target.present()
                    || mm.sniffer_happy.present()
                    || mm.breed_target.present()
                {
                    return false;
                }
                if is_tempted(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::SCENTING);
                true
            }
            B::Sniffing(t) => {
                let mm = mem(m);
                if mm.walk_target.present()
                    || mm.sniffer_sniffing_target.present()
                    || mm.sniff_cooldown.present()
                {
                    return false;
                }
                if m.is_baby() || !can_sniff(m) {
                    return false;
                }
                t.begin(cx.lr, cx.now);
                transition(m, cx, sniffer::SNIFFING);
                true
            }
            B::DoNothing(t) => {
                t.begin(cx.lr, cx.now);
                true
            }
            B::AxolotlAction { .. }
            | B::WalkToLookTarget { .. }
            | B::LookAtPlayer(_)
            | B::LookAtPlayerSometimes(..)
            | B::Stroll { .. }
            | B::FollowAdult { .. } => unreachable!("one-shots trigger"),
        }
    }

    /// A one-shot's trigger.
    fn trigger(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        match *self {
            B::AxolotlAction {
                action,
                ref mut until,
                ..
            } => axolotl_action(m, cx, action, until),
            B::WalkToLookTarget { speed, close, .. } => {
                let mm = mem(m);
                if mm.walk_target.present() {
                    return false;
                }
                let Some(look) = mm.look_target.value() else {
                    return false;
                };
                mm.walk_target.set(WalkTarget {
                    target: look,
                    speed,
                    close,
                });
                true
            }
            B::LookAtPlayer(_) => {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    who == Who::Player
                        && w.target_info(who)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 36.0)
                        && m.rider.is_none()
                });
                let Some(t) = found else { return false };
                mem(m).look_target.set(Tracker::entity(t, true));
                true
            }
            B::LookAtPlayerSometimes(_, ref mut ticks) => {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    who == Who::Player
                        && w.target_info(who)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 36.0)
                });
                let Some(t) = found else { return false };
                // Ticker.tickDownAndCheck (UniformInt 40..80 on the level random).
                if *ticks == 0 {
                    *ticks = if m.species() == Species::Axolotl {
                        30 + cx.lr.next_int_bound(31).unwrap_or(0) - 1
                    } else {
                        40 + cx.lr.next_int_bound(41).unwrap_or(0) - 1
                    };
                    return false;
                }
                *ticks -= 1;
                if *ticks != 0 {
                    return false;
                }
                mem(m).look_target.set(Tracker::entity(t, true));
                true
            }
            B::Stroll {
                fly, speed, range, ..
            } => {
                if m.species() == Species::Axolotl && m.fluids.in_water {
                    return false;
                }
                if mem(m).walk_target.present() {
                    return false;
                }
                let pos = if fly {
                    let view = m.view_vector();
                    m.air_and_water_random_pos(cx.w, 10, 7, -2, view[0], view[2])
                } else {
                    m.land_random_pos(cx.w, range.0, range.1)
                };
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, speed, 0)));
                true
            }
            B::FollowAdult { player, .. } => {
                let mm = mem(m);
                let adult = if player {
                    mm.nearest_visible_player.value()
                } else {
                    mm.nearest_visible_adult.value()
                };
                let Some(adult) = adult else { return false };
                if mm.walk_target.present() || !m.is_baby() {
                    return false;
                }
                let Some(at) = cx.w.target_info(adult) else {
                    return false;
                };
                // closerThan(adult, 17) && !closerThan(adult, 3)
                let d = m.distance_sqr(at.position);
                let min = if m.species() == Species::Axolotl {
                    5.0
                } else {
                    3.0
                };
                if !(min * min..17.0 * 17.0).contains(&d) {
                    return false;
                }
                let speed = if m.species() == Species::Axolotl {
                    axolotl_speed(m, true)
                } else {
                    1.1
                };
                let close = if m.species() == Species::Axolotl {
                    4
                } else {
                    2
                };
                let eyes = m.species() != Species::Axolotl;
                let mm = mem(m);
                mm.walk_target.set(WalkTarget {
                    target: Tracker::Entity {
                        who: adult,
                        eye: eyes,
                        block_eye: eyes,
                    },
                    speed,
                    close,
                });
                mm.look_target.set(Tracker::Entity {
                    who: adult,
                    eye: true,
                    block_eye: eyes,
                });
                true
            }
            _ => false,
        }
    }

    /// `BehaviorControl.tickOrStop`.
    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot() {
            self.set_one_shot(false);
            return;
        }
        let now = cx.now;
        match self {
            B::AxolotlAction { .. } => {}
            B::PlayDead(t) => {
                if t.timed_out(now) || !m.fluids.in_water || !mem(m).play_dead_ticks.present() {
                    self.do_stop(m, cx);
                }
            }
            B::Gate(g) => g.tick_or_stop(m, cx),
            B::Swim(t) => {
                if t.timed_out(now) || !swim_condition(m) {
                    self.do_stop(m, cx);
                } else if m.random.next_float() < 0.8 {
                    m.ai.jump = true;
                }
            }
            B::AnimalPanic { t, fly, speed } => {
                if t.timed_out(now) {
                    self.do_stop(m, cx);
                    return;
                }
                if m.ai.nav.is_done() {
                    let pos = panic_pos(m, cx.w, *fly);
                    if let Some(p) = pos {
                        mem(m).walk_target.set(WalkTarget::point(p, *speed, 0));
                    }
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
                if t.timed_out(now) || !still {
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
                    let d = dist_sqr(now_target, last);
                    if d > 4 && compute_path(m, cx, &walk, path, speed) {
                        *last_target = Some(now_target);
                        mem(m).path.set_opt(path.clone());
                        m.nav_move_to_path(path.clone(), f64::from(*speed));
                    }
                }
            }
            B::CountDownTemptation(_) => {
                let v = mem(m).temptation_cooldown_ticks.value();
                match v {
                    Some(v) if v > 0 => mem(m).temptation_cooldown_ticks.set(v - 1),
                    _ => self.do_stop(m, cx),
                }
            }
            B::CountDownGaze(_) => {
                let v = mem(m).gaze_cooldown_ticks.value();
                match v {
                    Some(v) if v > 0 => mem(m).gaze_cooldown_ticks.set(v - 1),
                    _ => self.do_stop(m, cx),
                }
            }
            B::CountDownTransport(_) => {
                let v = mem(m).transport_items_cooldown_ticks.value();
                match v {
                    Some(v) if v > 0 => mem(m).transport_items_cooldown_ticks.set(v - 1),
                    _ => self.do_stop(m, cx),
                }
            }
            B::Transport(x) => x.tick_or_stop(m, cx),
            B::MakeLove { spawn_at, .. } => {
                let Some(partner) = mem(m).breed_target.value() else {
                    self.do_stop(m, cx);
                    return;
                };
                let ok = cx.w.target_info(partner).is_some_and(|t| t.alive)
                    && can_mate_with(m, cx.w, partner)
                    && visible_contains(m, cx.w, partner)
                    && now <= *spawn_at
                    && !is_panicking(m)
                    && !partner_panicking(cx.w, partner);
                if !ok {
                    self.do_stop(m, cx);
                    return;
                }
                lock_gaze_and_walk(
                    m,
                    partner,
                    if m.species() == Species::Axolotl {
                        0.2
                    } else {
                        1.0
                    },
                    2,
                );
                if let Who::Mob(id) = partner {
                    cx.events.push(MobEvent::LockGaze {
                        mob: id,
                        partner: m.id,
                    });
                }
                if let Some(t) = cx.w.target_info(partner)
                    && m.distance_sqr(t.position) < 9.0
                    && now >= *spawn_at
                {
                    // spawnChildFromBreeding: the egg (or baby), and both
                    // parents rest (finalizeSpawnChildFromBreeding).
                    let Who::Mob(id) = partner else { return };
                    m.extra.in_love = 0;
                    m.extra.age = 6000;
                    cx.events.push(MobEvent::Breed {
                        parent: m.id,
                        partner: id,
                        llama: None,
                    });
                    if brain_mut(m).kind == Kind::Sniffer {
                        // Sniffer.spawnChildFromBreeding: the experience
                        // (finalizeSpawnChildFromBreeding), then the plop,
                        // both from the parent's random; the game lays the
                        // egg.
                        let at = [m.position.x, m.position.y, m.position.z];
                        let xp = 1 + m.random.next_int_bound(7).unwrap_or(0);
                        cx.events.push(MobEvent::Experience {
                            at,
                            amount: xp as u32,
                        });
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 0.5;
                        cx.events
                            .push(MobEvent::Sound("block.sniffer_egg.plop", at, 1.0, pitch));
                    }
                    mem(m).breed_target.clear();
                }
            }
            B::FollowTemptation {
                look_in_the_eyes, ..
            } => {
                let mm = mem(m);
                let still = mm.tempting_player.present()
                    && !mm.breed_target.present()
                    && !mm.is_panicking.present();
                if !still {
                    self.do_stop(m, cx);
                    return;
                }
                let eyes = *look_in_the_eyes;
                let player = mm.tempting_player.value().unwrap();
                mm.look_target.set(Tracker::entity(player, true));
                let close = follow_temptation_close(m);
                let near =
                    cx.w.target_info(player)
                        .is_some_and(|t| m.distance_sqr(t.position) < close * close);
                let speed = if m.species() == Species::Axolotl {
                    axolotl_speed(m, false)
                } else {
                    1.25
                };
                let mm = mem(m);
                if near {
                    mm.walk_target.clear();
                } else {
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity {
                            who: player,
                            eye: eyes,
                            block_eye: eyes,
                        },
                        speed,
                        close: 2,
                    });
                }
            }
            B::LookAtTargetSink(t) => {
                let visible = match mem(m).look_target.value() {
                    Some(look) => tracker_visible(m, cx.w, look),
                    None => false,
                };
                if t.timed_out(now) || !visible {
                    self.do_stop(m, cx);
                } else if let Some(at) = mem(m).look_target.value().and_then(|l| l.position(cx.w)) {
                    m.look_at(at);
                }
            }
            B::FeelingHappy(t) | B::Scenting(t) | B::DoNothing(t) => {
                if t.timed_out(now) {
                    self.do_stop(m, cx);
                }
            }
            B::Digging(t) => {
                let ok =
                    mem(m).sniffer_digging.present() && can_dig(m, cx.w) && m.extra.in_love <= 0;
                if t.timed_out(now) || !ok {
                    self.do_stop(m, cx);
                }
            }
            B::FinishedDigging(t) => {
                if t.timed_out(now) || !mem(m).sniffer_digging.present() {
                    self.do_stop(m, cx);
                }
            }
            B::Searching(t) => {
                let still = if !can_sniff(m) {
                    transition(m, cx, sniffer::IDLING);
                    false
                } else {
                    let w = cx.w;
                    let mm = mem(m);
                    match (
                        mm.walk_target.value().and_then(|t| t.target.block(w)),
                        mm.sniffer_sniffing_target.value(),
                    ) {
                        (Some(a), Some(b)) => a == b,
                        _ => false,
                    }
                };
                if t.timed_out(now) || !still {
                    self.do_stop(m, cx);
                }
            }
            B::Sniffing(t) => {
                if t.timed_out(now) || !can_sniff(m) {
                    self.do_stop(m, cx);
                }
            }
            B::WalkToLookTarget { .. }
            | B::LookAtPlayer(_)
            | B::LookAtPlayerSometimes(..)
            | B::Stroll { .. }
            | B::FollowAdult { .. } => {}
        }
    }

    /// `BehaviorControl.doStop`.
    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot() {
            self.set_one_shot(false);
            return;
        }
        let now = cx.now;
        match self {
            B::AxolotlAction { .. } => {}
            B::PlayDead(t) => t.running = false,
            B::Gate(g) => g.do_stop(m, cx),
            B::Swim(t) | B::DoNothing(t) => t.running = false,
            B::AnimalPanic { t, .. } => {
                t.running = false;
                mem(m).is_panicking.clear();
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
            B::CountDownTemptation(t) => {
                t.running = false;
                mem(m).temptation_cooldown_ticks.clear();
            }
            B::CountDownGaze(t) => {
                t.running = false;
                mem(m).gaze_cooldown_ticks.clear();
            }
            B::CountDownTransport(t) => {
                t.running = false;
                mem(m).transport_items_cooldown_ticks.clear();
            }
            B::Transport(x) => x.do_stop(m, cx),
            B::MakeLove { t, spawn_at } => {
                t.running = false;
                let mm = mem(m);
                mm.breed_target.clear();
                mm.walk_target.clear();
                mm.look_target.clear();
                *spawn_at = 0;
            }
            B::FollowTemptation { t, .. } => {
                t.running = false;
                let mm = mem(m);
                mm.temptation_cooldown_ticks.set(100);
                mm.walk_target.clear();
                mm.look_target.clear();
            }
            B::LookAtTargetSink(t) => {
                t.running = false;
                mem(m).look_target.clear();
            }
            B::FeelingHappy(t) => {
                t.running = false;
                transition(m, cx, sniffer::IDLING);
                mem(m).sniffer_happy.clear();
            }
            B::Digging(t) => {
                t.running = false;
                if t.timed_out(now) {
                    mem(m).sniff_cooldown.set_ttl((), 9600);
                } else {
                    reset_sniffing(m, cx);
                }
            }
            B::FinishedDigging(t) => {
                t.running = false;
                let finished = t.timed_out(now);
                transition(m, cx, sniffer::IDLING);
                if finished {
                    // onDiggingComplete: storeExploredPosition(getOnPos()).
                    let p = sniffer::on_pos(m);
                    m.extra.positions.truncate(EXPLORED_MAX);
                    m.extra.positions.insert(0, p);
                }
                let mm = mem(m);
                mm.sniffer_digging.clear();
                mm.sniffer_happy.set(true);
            }
            B::Searching(t) => {
                t.running = false;
                if can_dig(m, cx.w) && can_sniff(m) {
                    mem(m).sniffer_digging.set(true);
                }
                let mm = mem(m);
                mm.walk_target.clear();
                mm.sniffer_sniffing_target.clear();
            }
            B::Scenting(t) => {
                t.running = false;
                transition(m, cx, sniffer::IDLING);
            }
            B::Sniffing(t) => {
                t.running = false;
                let finished = t.timed_out(now);
                transition(m, cx, sniffer::IDLING);
                if finished && let Some(p) = dig_position(m, cx.w) {
                    let mm = mem(m);
                    mm.sniffer_sniffing_target.set(p);
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Block(p),
                        speed: 1.25,
                        close: 0,
                    });
                }
            }
            B::WalkToLookTarget { .. }
            | B::LookAtPlayer(_)
            | B::LookAtPlayerSometimes(..)
            | B::Stroll { .. }
            | B::FollowAdult { .. } => {}
        }
    }
}

fn same_path(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.nodes == b.nodes && a.target == b.target,
        _ => false,
    }
}
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> i64 {
    let d = |i: usize| i64::from(a[i] - b[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}
fn manhattan(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}
/// `Swim`: deeper than the jump threshold, or in lava.
fn swim_condition(m: &Mob) -> bool {
    m.fluids.water > m.fluid_jump_threshold() || m.fluids.in_lava
}
/// `PositionTracker.isVisibleBy`.
fn tracker_visible(m: &mut Mob, w: &MobWorld, t: Tracker) -> bool {
    match t {
        Tracker::Entity { who, .. } => {
            w.target_info(who).is_some_and(|i| i.alive) && visible_contains(m, w, who)
        }
        Tracker::Block(_) => true,
    }
}
/// `MoveToTargetSink.reachedTarget`.
fn reached(m: &Mob, w: &MobWorld, walk: &WalkTarget) -> bool {
    walk.target
        .block(w)
        .is_some_and(|p| manhattan(p, block_of(m.position)) <= walk.close)
}
/// `MoveToTargetSink.tryComputePath`.
fn compute_path(
    m: &mut Mob,
    cx: &mut Bx,
    walk: &WalkTarget,
    path: &mut Option<Path>,
    speed: &mut f32,
) -> bool {
    let Some(target) = walk.target.block(cx.w) else {
        return false;
    };
    *path = m.create_path_to(cx.w, target, 0);
    *speed = walk.speed;
    if reached(m, cx.w, walk) {
        mem(m).cant_reach_walk_target_since.clear();
    } else {
        let can_reach = path.as_ref().is_some_and(|p| p.reached);
        let mm = mem(m);
        if can_reach {
            mm.cant_reach_walk_target_since.clear();
        } else if !mm.cant_reach_walk_target_since.present() {
            mm.cant_reach_walk_target_since.set(cx.now);
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
            m.default_random_pos_towards(cx.w, 10, 7, to, f64::from(std::f32::consts::FRAC_PI_2))
        {
            let p = [
                step[0].floor() as i32,
                step[1].floor() as i32,
                step[2].floor() as i32,
            ];
            *path = m.create_path_to(cx.w, p, 0);
            return path.is_some();
        }
    }
    false
}
/// `AnimalPanic.getPanicPos`: on land 5 blocks out and 4 up or down, or
/// for a flyer the air and water around it ahead (water first when burning
/// is not reproduced: no fire reaches these scenarios).
fn panic_pos(m: &mut Mob, w: &MobWorld, fly: bool) -> Option<[f64; 3]> {
    if fly {
        let view = m.view_vector();
        m.air_and_water_random_pos(w, 5, 4, 0, view[0], view[2])
    } else {
        m.land_random_pos(w, 5, 4)
    }
}
/// `BehaviorUtils.lockGazeAndWalkToEachOther` (this mob's half; the
/// partner's comes with the breed-target event).
fn lock_gaze_and_walk(m: &mut Mob, partner: Who, speed: f32, close: i32) {
    let t = Tracker::entity(partner, true);
    let mm = mem(m);
    mm.look_target.set(t);
    mm.walk_target.set(WalkTarget {
        target: t,
        speed,
        close,
    });
}
/// `FollowTemptation`'s close-enough distance.
fn follow_temptation_close(m: &mut Mob) -> f64 {
    match brain_mut(m).kind {
        Kind::Sniffer => {
            if m.is_baby() {
                2.5
            } else {
                3.5
            }
        }
        Kind::Axolotl => 2.5,
        Kind::Ghastling | Kind::CopperGolem => 3.0,
    }
}
fn can_mate_with(m: &Mob, w: &MobWorld, partner: Who) -> bool {
    let Who::Mob(id) = partner else { return false };
    let Some(s) = w.seen(id) else { return false };
    s.species == m.species()
        && s.in_love
        && m.extra.in_love > 0
        && (m.species() != Species::Sniffer
            || sniffer::can_mate_state(sniffer::state(m)) && sniffer::can_mate_state(s.variant))
}
fn partner_panicking(_w: &MobWorld, _partner: Who) -> bool {
    false
}
/// `AnimalMakeLove.findValidBreedPartner`.
fn breed_partner(m: &mut Mob, w: &MobWorld) -> Option<Who> {
    find_closest(m, w, |m, who| can_mate_with(m, w, who))
}

// ---- The sniffer --------------------------------------------------------

/// `Sniffer.transitionTo`: the state and its sound.
fn transition(m: &mut Mob, cx: &mut Bx, to: u8) {
    let at = [m.position.x, m.position.y, m.position.z];
    let sound = |name: &'static str, pitch: f32| MobEvent::Sound(name, at, 1.0, pitch);
    match to {
        sniffer::FEELING_HAPPY => cx.events.push(sound("entity.sniffer.happy", 1.0)),
        sniffer::SCENTING => {
            let pitch = if m.is_baby() { 1.3 } else { 1.0 };
            cx.events.push(sound("entity.sniffer.scenting", pitch));
        }
        sniffer::SNIFFING => cx.events.push(sound("entity.sniffer.sniffing", 1.0)),
        // onDiggingStart: the seed drops 120 ticks in.
        sniffer::DIGGING => m.extra.ints[3] = m.age as i32 + 120,
        sniffer::RISING => cx.events.push(sound("entity.sniffer.digging_stop", 1.0)),
        _ => {}
    }
    sniffer::set_state(m, to);
}
/// `SnifferAi.resetSniffing`.
fn reset_sniffing(m: &mut Mob, cx: &mut Bx) {
    let mm = mem(m);
    mm.sniffer_digging.clear();
    mm.sniffer_sniffing_target.clear();
    transition(m, cx, sniffer::IDLING);
}
/// `Sniffer.canSniff`.
fn can_sniff(m: &Mob) -> bool {
    !is_tempted(m)
        && !is_panicking(m)
        && !m.fluids.in_water
        && m.extra.in_love <= 0
        && m.on_ground
        && !m.extra.passenger
        && m.extra.leash.is_none()
}
/// `Sniffer.canDig(BlockPos)`: diggable, unexplored and reachable.
fn can_dig_at(m: &mut Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    w.has_tag(
        "minecraft:sniffer_diggable_block",
        w.block_name(p[0], p[1], p[2]),
    ) && !m.extra.positions.contains(&p)
        && m.create_path_to(w, p, 1).is_some_and(|path| path.reached)
}
/// `Sniffer.canDig()`.
fn can_dig(m: &mut Mob, w: &MobWorld) -> bool {
    if is_panicking(m)
        || is_tempted(m)
        || m.is_baby()
        || m.fluids.in_water
        || !m.on_ground
        || m.extra.passenger
    {
        return false;
    }
    let head = sniffer::head_block(m);
    can_dig_at(m, w, [head[0], head[1] - 1, head[2]])
}
/// `Sniffer.calculateDigPosition`: the first of five ever wider land
/// positions whose block below can be dug.
fn dig_position(m: &mut Mob, w: &MobWorld) -> Option<[i32; 3]> {
    for idx in 0..5 {
        let Some(p) = m.land_random_pos(w, 10 + 2 * idx, 3) else {
            continue;
        };
        let below = [
            p[0].floor() as i32,
            p[1].floor() as i32 - 1,
            p[2].floor() as i32,
        ];
        if can_dig_at(m, w, below) {
            return Some(below);
        }
    }
    None
}

/// `SnifferAi.getActivities`, in `availableBehaviorsByPriority` order: by
/// priority (`ActivityData.create(CORE, 0, ...)` numbers the core
/// behaviours 0, 1, 2...), then by the activity's bucket in the 16-bucket
/// `HashMap<Activity, ...>` (core 1, dig 3, idle 5, sniff 6, panic 15),
/// then in insertion order.
fn sniffer_package(seed: i64) -> Vec<Entry> {
    let e = |act: Act, b: B| Entry { act, b };
    let mut l = vec![
        // Priority 0: core, dig, idle, sniff.
        e(Act::Core, B::Swim(Timed::new(60, 60))),
        e(Act::Dig, B::Digging(Timed::new(160, 180))),
        e(Act::Dig, B::FinishedDigging(Timed::new(40, 40))),
        e(
            Act::Idle,
            B::MakeLove {
                t: Timed::new(110, 110),
                spawn_at: 0,
            },
        ),
        e(Act::Sniff, B::Searching(Timed::new(600, 600))),
        // Priority 1.
        e(
            Act::Core,
            B::AnimalPanic {
                t: Timed::new(100, 120),
                fly: false,
                speed: 2.0,
            },
        ),
        e(
            Act::Idle,
            B::FollowTemptation {
                t: Timed::new(60, 60),
                look_in_the_eyes: false,
            },
        ),
        // Priority 2.
        e(Act::Core, move_to_target_sink(500, 700)),
        e(Act::Idle, B::LookAtTargetSink(Timed::new(45, 90))),
        // Priority 3.
        e(Act::Core, B::CountDownTemptation(Timed::new(60, 60))),
        e(Act::Idle, B::FeelingHappy(Timed::new(40, 100))),
    ];
    // Priority 4.
    l.push(e(
        Act::Idle,
        B::Gate(Box::new(Gate::run_one(
            seed,
            vec![
                (
                    B::WalkToLookTarget {
                        running: false,
                        speed: 1.0,
                        close: 3,
                    },
                    2,
                ),
                (B::Scenting(Timed::new(40, 80)), 1),
                (B::Sniffing(Timed::new(40, 80)), 1),
                (B::LookAtPlayer(false), 1),
                (
                    B::Stroll {
                        running: false,
                        fly: false,
                        speed: 1.0,
                        range: (10, 7),
                    },
                    1,
                ),
                (B::DoNothing(Timed::new(5, 20)), 2),
            ],
        ))),
    ));
    l
}
/// `HappyGhastAi.getActivities`, in `availableBehaviorsByPriority` order.
/// `CopperGolemAi.getActivities`, in tick order: the core behaviours take
/// priorities 0 to 5 in turn (`ActivityData.create(CORE, 0, ..)`), the idle
/// ones 0, 1 and 2.
fn copper_golem_package(seed: i64) -> Vec<Entry> {
    let e = |act: Act, b: B| Entry { act, b };
    vec![
        // Priority 0.
        e(
            Act::Core,
            B::AnimalPanic {
                t: Timed::new(100, 120),
                fly: false,
                speed: 1.5,
            },
        ),
        e(Act::Idle, B::Transport(Box::default())),
        // Priority 1.
        e(Act::Core, B::LookAtTargetSink(Timed::new(45, 90))),
        e(Act::Idle, B::LookAtPlayerSometimes(false, 0)),
        // Priority 2: RunOne(stroll or do nothing) while there is no walk
        // target and the transport cools down.
        e(Act::Core, move_to_target_sink(150, 250)),
        e(
            Act::Idle,
            B::Gate(Box::new(
                Gate::run_one(
                    seed,
                    vec![
                        (
                            B::Stroll {
                                running: false,
                                fly: false,
                                speed: 1.0,
                                range: (2, 2),
                            },
                            1,
                        ),
                        (B::DoNothing(Timed::new(30, 60)), 1),
                    ],
                )
                .when(|mm| {
                    !mm.walk_target.present() && mm.transport_items_cooldown_ticks.present()
                }),
            )),
        ),
        // Priority 3 is InteractWithDoor, which only acts on doors along the
        // path; these golems pass none.
        // Priorities 4 and 5.
        e(Act::Core, B::CountDownGaze(Timed::new(60, 60))),
        e(Act::Core, B::CountDownTransport(Timed::new(60, 60))),
    ]
}

fn ghastling_package(seed: i64) -> Vec<Entry> {
    let e = |act: Act, b: B| Entry { act, b };
    vec![
        // Priority 0.
        e(Act::Core, B::Swim(Timed::new(60, 60))),
        // Priority 1.
        e(
            Act::Core,
            B::AnimalPanic {
                t: Timed::new(100, 120),
                fly: true,
                speed: 2.0,
            },
        ),
        e(
            Act::Idle,
            B::FollowTemptation {
                t: Timed::new(60, 60),
                look_in_the_eyes: true,
            },
        ),
        // Priority 2.
        e(Act::Core, B::LookAtTargetSink(Timed::new(45, 90))),
        e(
            Act::Idle,
            B::FollowAdult {
                running: false,
                player: true,
            },
        ),
        // Priority 3.
        e(Act::Core, move_to_target_sink(150, 250)),
        e(
            Act::Idle,
            B::FollowAdult {
                running: false,
                player: false,
            },
        ),
        // Priority 4.
        e(Act::Core, B::CountDownTemptation(Timed::new(60, 60))),
        e(
            Act::Idle,
            B::Gate(Box::new(Gate::run_one(
                seed,
                vec![
                    (
                        B::Stroll {
                            running: false,
                            fly: true,
                            speed: 1.0,
                            range: (10, 7),
                        },
                        1,
                    ),
                    (
                        B::WalkToLookTarget {
                            running: false,
                            speed: 1.0,
                            close: 3,
                        },
                        1,
                    ),
                ],
            ))),
        ),
    ]
}
fn move_to_target_sink(min: i32, max: i32) -> B {
    B::MoveToTargetSink {
        t: Timed::new(min, max),
        cooldown: 0,
        path: None,
        last_target: None,
        speed: 0.0,
    }
}

// ---- AxolotlAi -----------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum AxolotlAction {
    ValidateDead,
    StartAttack,
    StopAttack,
    Chase,
    Bite,
    EraseAttackBreeding,
    EraseDeadBreeding,
    FindLiquid,
    SwimStroll,
    WalkFromLook,
    StayWater,
    StayGround,
}
fn axolotl_speed(m: &Mob, chase: bool) -> f32 {
    if m.fluids.in_water {
        if chase { 0.6 } else { 0.5 }
    } else {
        0.15
    }
}
pub fn set_play_dead(m: &mut Mob) {
    if m.extra.animal_brain.is_none() {
        init(m, i64::from(m.id));
    }
    mem(m).play_dead_ticks.set(super::axolotl::PLAY_DEAD_TICKS);
}
pub fn hunting_cooldown(m: &Mob) -> Option<i64> {
    mem_ref(m).and_then(|mm| {
        mm.hunting_cooldown
            .get()
            .and_then(|_| (mm.hunting_cooldown.ttl > 0).then_some(mm.hunting_cooldown.ttl))
    })
}
pub fn set_hunting_cooldown(m: &mut Mob, ticks: i64) {
    if m.extra.animal_brain.is_none() {
        init(m, i64::from(m.id));
    }
    if ticks > 0 {
        mem(m).hunting_cooldown.set_ttl(true, ticks);
    } else {
        mem(m).hunting_cooldown.clear();
    }
}
fn validate_play_dead(b: &mut Brain) -> bool {
    let Some(ticks) = b.mem.play_dead_ticks.value() else {
        return false;
    };
    if ticks <= 0 {
        b.mem.play_dead_ticks.clear();
        b.mem.hurt_by_entity.clear();
        b.activity = Act::Idle;
    } else {
        b.mem.play_dead_ticks.set(ticks - 1);
    }
    true
}
fn update_axolotl_activity(b: &mut Brain) {
    let old = b.activity;
    // PLAY_DEAD remains active until ValidatePlayDead resets to default IDLE.
    if old == Act::PlayDead {
        return;
    }
    b.activity = if b.mem.play_dead_ticks.present() {
        Act::PlayDead
    } else if b.mem.attack_target.present() {
        Act::Fight
    } else {
        Act::Idle
    };
    if old != b.activity && old == Act::Fight {
        b.mem
            .hunting_cooldown
            .set_ttl(true, super::axolotl::HUNTING_COOLDOWN);
        b.mem.attack_target.clear();
    }
}
fn sense_axolotl_attackables(m: &mut Mob, w: &MobWorld) {
    let cooldown = mem(m).hunting_cooldown.present();
    let target = find_closest(m, w, |m, who| {
        let Who::Mob(id) = who else { return false };
        let Some(s) = w.seen(id) else { return false };
        let id = format!("minecraft:{}", super::species::def(s.species).name);
        super::axolotl::prey_eligible(
            m.distance_sqr(s.position),
            s.in_water,
            w.data.entity_has("minecraft:axolotl_always_hostiles", &id),
            w.data.entity_has("minecraft:axolotl_hunt_targets", &id),
            cooldown,
            s.alive && !s.playing_dead,
        )
    });
    mem(m).nearest_attackable.set_opt(target);
}
fn axolotl_action(m: &mut Mob, cx: &mut Bx, action: AxolotlAction, until: &mut i64) -> bool {
    use AxolotlAction::*;
    match action {
        ValidateDead => validate_play_dead(brain_mut(m)),
        StartAttack => {
            let mm = mem(m);
            if mm.attack_target.present() || mm.breed_target.present() {
                return false;
            }
            let Some(target) = mm.nearest_attackable.value() else {
                return false;
            };
            if !cx
                .w
                .target_info(target)
                .is_some_and(|t| t.alive && !t.creative)
            {
                return false;
            }
            mem(m).attack_target.set(target);
            mem(m).cant_reach_walk_target_since.clear();
            m.ai.target = Some(target);
            true
        }
        StopAttack => {
            let Some(target) = mem(m).attack_target.value() else {
                return false;
            };
            let tired = mem(m)
                .cant_reach_walk_target_since
                .value()
                .is_some_and(|t| cx.now - t > 200);
            let valid =
                cx.w.target_info(target)
                    .is_some_and(|t| t.alive && !t.creative)
                    && match target {
                        Who::Mob(id) => cx.w.seen(id).is_some_and(|s| !s.playing_dead),
                        Who::Player => true,
                    };
            if tired || !valid {
                super::axolotl::on_stop_attacking(m, cx.w, target, cx.events);
                mem(m).attack_target.clear();
                m.ai.target = None;
            }
            true
        }
        Chase => {
            let Some(target) = mem(m).attack_target.value() else {
                return false;
            };
            let speed = axolotl_speed(m, true);
            if cx
                .w
                .target_info(target)
                .is_some_and(|t| m.within_melee_range(&t))
                && visible_contains(m, cx.w, target)
            {
                mem(m).walk_target.clear();
            } else {
                let tracker = Tracker::entity(target, true);
                mem(m).look_target.set(tracker);
                mem(m).walk_target.set(WalkTarget {
                    target: Tracker::entity(target, false),
                    speed,
                    close: 0,
                });
            }
            true
        }
        Bite => {
            let mm = mem(m);
            if mm.attack_cooling_down.present() {
                return false;
            }
            let Some(target) = mm.attack_target.value() else {
                return false;
            };
            if !visible_contains(m, cx.w, target)
                || !cx
                    .w
                    .target_info(target)
                    .is_some_and(|t| m.within_melee_range(&t))
            {
                return false;
            }
            mem(m).look_target.set(Tracker::entity(target, true));
            m.swing();
            super::ai::do_hurt_target(
                m,
                target,
                &mut super::goals::Cx {
                    w: cx.w,
                    lr: cx.lr,
                    events: cx.events,
                },
            );
            mem(m).attack_cooling_down.set_ttl(true, 20);
            true
        }
        EraseAttackBreeding | EraseDeadBreeding => {
            if !mem(m).breed_target.present() {
                return false;
            }
            if matches!(action, EraseAttackBreeding) {
                mem(m).attack_target.clear();
            } else {
                mem(m).play_dead_ticks.clear();
            }
            true
        }
        FindLiquid => {
            if mem(m).walk_target.present() || mem(m).attack_target.present() {
                return false;
            }
            let p = [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ];
            if cx.w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water {
                return false;
            }
            if cx.now < *until {
                *until = cx.now + 22;
                return true;
            }
            let mut positions = Vec::new();
            // BlockPos.withinManhattan: distance, x, y, then positive/negative z.
            for distance in 0..=18i32 {
                for x in -6..=6i32 {
                    for y in -6..=6i32 {
                        let z = distance - x.abs() - y.abs();
                        if !(0..=6).contains(&z) {
                            continue;
                        }
                        positions.push([p[0] + x, p[1] + y, p[2] + z]);
                        if z != 0 {
                            positions.push([p[0] + x, p[1] + y, p[2] - z]);
                        }
                    }
                }
            }
            let mut found = None;
            for q in positions {
                if (q[0] == p[0] && q[2] == p[2])
                    || cx.w.fluid(q[0], q[1], q[2]) != crate::blocks::Fluid::Water
                {
                    continue;
                }
                let name = cx.w.block_name(q[0], q[1], q[2]);
                if name != "minecraft:water" {
                    continue;
                }
                if cx.w.block_name(q[0], q[1] + 1, q[2]).ends_with("air") {
                    found = Some(q);
                    break;
                }
                let v = Vec3::new(
                    f64::from(q[0]) + 0.5,
                    f64::from(q[1]) + 0.5,
                    f64::from(q[2]) + 0.5,
                );
                if found.is_none() && m.distance_sqr(v) >= 2.25 {
                    found = Some(q);
                }
            }
            if let Some(q) = found {
                mem(m).look_target.set(Tracker::Block(q));
                mem(m).walk_target.set(WalkTarget {
                    target: Tracker::Block(q),
                    speed: 0.15,
                    close: 0,
                });
            }
            *until = cx.now + 40;
            true
        }
        SwimStroll => {
            if mem(m).walk_target.present() || !m.fluids.in_water {
                return false;
            }
            let mut fallback: Option<Vec3> = None;
            for (h, v) in [(1, 1), (3, 3), (5, 5), (6, 5), (7, 7), (10, 7)] {
                let point = if let Some(p) = fallback {
                    let delta =
                        Vec3::new(p.x - m.position.x, p.y - m.position.y, p.z - m.position.z);
                    let length = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
                    let n = if length < 1.0e-5 {
                        Vec3::ZERO
                    } else {
                        Vec3::new(delta.x / length, delta.y / length, delta.z / length)
                    };
                    Some(Vec3::new(
                        m.position.x + n.x * f64::from(h),
                        m.position.y + n.y * f64::from(v),
                        m.position.z + n.z * f64::from(h),
                    ))
                } else {
                    let mut p = m.default_random_pos(cx.w, h, v);
                    let mut count = 0;
                    while let Some(q) = p {
                        let state = cx.w.world.block(
                            q[0].floor() as i32,
                            q[1].floor() as i32,
                            q[2].floor() as i32,
                        );
                        if cx.w.data.state(state).flags & super::data::WATER != 0 || count >= 10 {
                            break;
                        }
                        count += 1;
                        p = m.default_random_pos(cx.w, h, v);
                    }
                    p.map(|q| Vec3::new(q[0], q[1], q[2]))
                };
                let Some(point) = point else { break };
                let q = [
                    point.x.floor() as i32,
                    point.y.floor() as i32,
                    point.z.floor() as i32,
                ];
                let restricted = m.restricted(f64::from(h), q);
                if cx.w.fluid(q[0], q[1], q[2]) == crate::blocks::Fluid::None || restricted {
                    break;
                }
                fallback = Some(point);
            }
            mem(m)
                .walk_target
                .set_opt(fallback.map(|p| WalkTarget::point([p.x, p.y, p.z], 0.5, 0)));
            true
        }
        WalkFromLook => {
            if mem(m).walk_target.present() {
                return false;
            }
            let Some(t) = mem(m).look_target.value() else {
                return false;
            };
            let Some(p) = t.block(cx.w) else { return false };
            if (cx.w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water) != m.fluids.in_water {
                return false;
            }
            let speed = axolotl_speed(m, false);
            mem(m).walk_target.set(WalkTarget {
                target: t,
                speed,
                close: 3,
            });
            true
        }
        StayWater => m.fluids.in_water,
        StayGround => m.on_ground,
    }
}
fn axolotl_package(seed: i64) -> Vec<Entry> {
    use Act::*;
    use AxolotlAction::*;
    let e = |act, b| Entry { act, b };
    let action = |action| B::AxolotlAction {
        action,
        running: false,
        until: 0,
    };
    vec![
        // Native activity slots, merged by behavior priority.
        e(Core, B::LookAtTargetSink(Timed::new(45, 90))),
        e(Idle, B::LookAtPlayerSometimes(false, 0)),
        e(Fight, action(StopAttack)),
        e(PlayDead, B::PlayDead(Timed::new(200, 200))),
        e(Core, move_to_target_sink(150, 250)),
        e(
            Idle,
            B::MakeLove {
                t: Timed::new(110, 110),
                spawn_at: 0,
            },
        ),
        e(Fight, action(Chase)),
        e(PlayDead, action(EraseDeadBreeding)),
        e(Core, action(ValidateDead)),
        e(
            Idle,
            B::Gate(Box::new(Gate::run_one(
                seed,
                vec![
                    (
                        B::FollowTemptation {
                            t: Timed::new(60, 60),
                            look_in_the_eyes: false,
                        },
                        1,
                    ),
                    (
                        B::FollowAdult {
                            running: false,
                            player: false,
                        },
                        1,
                    ),
                ],
            ))),
        ),
        e(Fight, action(Bite)),
        e(Core, B::CountDownTemptation(Timed::new(60, 60))),
        e(Idle, action(StartAttack)),
        e(Idle, action(FindLiquid)),
        e(Fight, action(EraseAttackBreeding)),
        e(
            Idle,
            B::Gate(Box::new(Gate {
                running: false,
                shuffle: false,
                try_all: true,
                random: horizon_core::random::LegacyRandom::new(seed),
                kids: vec![
                    (action(SwimStroll), 2, 0.0),
                    (
                        B::Stroll {
                            running: false,
                            fly: false,
                            speed: 0.15,
                            range: (10, 7),
                        },
                        2,
                        0.0,
                    ),
                    (action(WalkFromLook), 3, 0.0),
                    (action(StayWater), 5, 0.0),
                    (action(StayGround), 5, 0.0),
                ],
                when: Some(|mm| !mm.walk_target.present()),
            })),
        ),
    ]
}

// ---- Debug output (the mob AI harness) ------------------------------------

fn pos(p: [i32; 3]) -> String {
    format!("{},{},{}", p[0], p[1], p[2])
}
fn who(w: Who) -> String {
    match w {
        Who::Player => "player".into(),
        Who::Mob(id) => format!("e{id}"),
    }
}
fn tracker(t: Tracker) -> String {
    match t {
        Tracker::Entity { who: e, .. } => who(e),
        Tracker::Block(p) => pos(p),
    }
}

/// The active activities and the memories, as the mob AI oracle prints
/// them.
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
    let n = |l: &Vec<Who>| format!("n{}", l.len());
    slot!("nearest_attackable", mm.nearest_attackable, |v: &Who| who(
        *v
    ));
    slot!("attack_target", mm.attack_target, |v: &Who| who(*v));
    slot!("attack_cooling_down", mm.attack_cooling_down, |v: &bool| v
        .to_string());
    slot!("has_hunting_cooldown", mm.hunting_cooldown, |v: &bool| v
        .to_string());
    slot!("play_dead_ticks", mm.play_dead_ticks, |v: &i32| v
        .to_string());
    slot!("mobs", mm.mobs, n);
    slot!("visible_mobs", mm.visible_mobs, |_: &Visible| {
        "NearestVisibleLivingEntities".to_string()
    });
    slot!("nearest_players", mm.nearest_players, n);
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
        n
    );
    slot!(
        "nearest_visible_adult",
        mm.nearest_visible_adult,
        |e: &Who| who(*e)
    );
    slot!("walk_target", mm.walk_target, |t: &WalkTarget| format!(
        "{}/{:x}/{}",
        tracker(t.target),
        t.speed.to_bits(),
        t.close
    ));
    slot!("look_target", mm.look_target, |t: &Tracker| tracker(*t));
    slot!("breed_target", mm.breed_target, |e: &Who| who(*e));
    slot!("path", mm.path, |p: &Path| format!("path{}", pos(p.target)));
    slot!("hurt_by", mm.hurt_by, |_: &bool| "DamageSource".to_string());
    slot!("hurt_by_entity", mm.hurt_by_entity, |e: &Who| who(*e));
    slot!(
        "cant_reach_walk_target_since",
        mm.cant_reach_walk_target_since,
        |t: &i64| t.to_string()
    );
    slot!("tempting_player", mm.tempting_player, |e: &Who| who(*e));
    slot!(
        "temptation_cooldown_ticks",
        mm.temptation_cooldown_ticks,
        |t: &i32| t.to_string()
    );
    slot!("is_panicking", mm.is_panicking, |b: &bool| b.to_string());
    slot!("sniffer_digging", mm.sniffer_digging, |b: &bool| b
        .to_string());
    slot!(
        "sniffer_sniffing_target",
        mm.sniffer_sniffing_target,
        |p: &[i32; 3]| pos(*p)
    );
    slot!("sniffer_happy", mm.sniffer_happy, |b: &bool| b.to_string());
    slot!("sniff_cooldown", mm.sniff_cooldown, |_: &()| "u"
        .to_string());
    slot!("gaze_cooldown_ticks", mm.gaze_cooldown_ticks, |t: &i32| t
        .to_string());
    slot!(
        "transport_items_cooldown_ticks",
        mm.transport_items_cooldown_ticks,
        |t: &i32| t.to_string()
    );
    slot!(
        "visited_block_positions",
        mm.visited_block_positions,
        |l: &Vec<[i32; 3]>| format!("n{}", l.len())
    );
    slot!(
        "unreachable_transport_block_positions",
        mm.unreachable_transport_block_positions,
        |l: &Vec<[i32; 3]>| format!("n{}", l.len())
    );
    if b.kind == Kind::Sniffer && !m.extra.positions.is_empty() {
        put(
            "sniffer_explored_positions",
            format!("n{}", m.extra.positions.len()),
            NEVER,
        );
    }
    out.sort();
    Some((acts.join("|"), out.join(" ")))
}

#[cfg(test)]
mod axolotl_parity_tests {
    use super::*;
    fn empty_brain() -> Brain {
        Brain {
            kind: Kind::Axolotl,
            mem: Memories::default(),
            sensors: Vec::new(),
            activity: Act::Idle,
            list: Vec::new(),
        }
    }
    #[test]
    fn fight_exit_sets_hunting_cooldown_and_play_dead_has_priority() {
        let mut b = empty_brain();
        b.mem.attack_target.set(Who::Mob(1));
        update_axolotl_activity(&mut b);
        assert_eq!(b.activity, Act::Fight);
        b.mem.play_dead_ticks.set(200);
        update_axolotl_activity(&mut b);
        assert_eq!(b.activity, Act::PlayDead);
        assert_eq!(b.mem.hunting_cooldown.ttl, 2400);
        assert!(!b.mem.attack_target.present());
        for _ in 0..2400 {
            b.mem.hunting_cooldown.tick();
        }
        assert!(b.mem.hunting_cooldown.present());
        b.mem.hunting_cooldown.tick();
        assert!(!b.mem.hunting_cooldown.present());
    }
    #[test]
    fn play_dead_flag_ends_at_zero_before_memory_is_erased() {
        let mut b = empty_brain();
        b.activity = Act::PlayDead;
        b.mem.play_dead_ticks.set(200);
        b.mem.hurt_by_entity.set(Who::Mob(7));
        for _ in 0..200 {
            assert!(validate_play_dead(&mut b));
        }
        assert_eq!(b.mem.play_dead_ticks.value(), Some(0));
        assert_eq!(b.activity, Act::PlayDead);
        assert!(validate_play_dead(&mut b));
        assert!(!b.mem.play_dead_ticks.present());
        assert!(!b.mem.hurt_by_entity.present());
        assert_eq!(b.activity, Act::Idle);
    }
    #[test]
    fn axolotl_has_native_activity_brain_not_fallback_goals() {
        assert!(kind_for_species(Species::Axolotl).is_some());
    }
}

#[cfg(test)]
mod breeding_duration_tests {
    use super::*;
    #[test]
    fn axolotl_breeding_uses_native_110_tick_timeout() {
        let package = axolotl_package(1);
        let timed = package
            .iter()
            .find_map(|e| {
                if let B::MakeLove { t, .. } = &e.b {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!((timed.min, timed.max), (110, 110));
    }
}
