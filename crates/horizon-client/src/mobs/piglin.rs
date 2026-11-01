//! Piglins, piglin brutes, hoglins and zoglins: gold-wearing players are
//! left alone, gold is admired and bartered, piglins and hoglins zombify
//! outside the Nether, hoglins headbutt their targets into the air.
use super::{
    Mob, MobEvent, MobWorld, Seen, Species, Who,
    anim::{Poses, State},
    brain::{Ttl, living_near},
    goals::{
        Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack, NearestTarget, Prey,
        RandomLookAround, Stroll, TARGET, TargetState, goal_clone, target_continue,
    },
};
use horizon_core::geometry::{Vec3, trig::Mth};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::f32::consts::PI;

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE: i32 = 119;
/// `PiglinAi.eat`: the `ATE_RECENTLY` memory lasts 200 ticks (kept in
/// `extra.ints[0]` of a piglin).
pub const ATE_RECENTLY: i32 = 200;
/// `AbstractPiglin.CONVERSION_TIME` (and the hoglin's).
const CONVERSION: i32 = 300;
/// `PiglinAi.CELEBRATION_TIME`: how long the `CELEBRATE_LOCATION` and
/// `DANCING` memories last.
const CELEBRATION: i32 = 300;

/// `Activity`: what a piglin's or hoglin's brain is busy with. `Admire`,
/// `Celebrate` and `Ride` are a piglin's only; the others are both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Idle,
    Fight,
    Avoid,
    Admire,
    Celebrate,
    Ride,
}
/// `MemoryModuleType.AVOID_TARGET`: who to keep away from, where it was last
/// seen (a target that is gone leaves its spot behind) and when the memory
/// expires.
#[derive(Clone, Copy, Debug)]
pub struct AvoidTarget {
    pub who: Who,
    pub at: Vec3,
    pub ttl: Ttl,
}
/// `MemoryModuleType.RIDE_TARGET`: the mob a baby piglin means to ride, when
/// the memory expires and the block the walk to it was last aimed at.
#[derive(Clone, Copy, Debug)]
pub struct RideTarget {
    pub vehicle: u32,
    pub ttl: Ttl,
    pub aimed: Option<[i32; 3]>,
}
/// What a piglin's or hoglin's brain remembers of its surroundings and of
/// what it retreats from. None of it is saved: these memories have no codec.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    /// `AVOID_TARGET`.
    pub avoid: Option<AvoidTarget>,
    /// `RIDE_TARGET`.
    pub ride: Option<RideTarget>,
    /// A piglin's `HURT_BY` is set: the `HurtBySensor` saw damage taken
    /// within the last 40 ticks.
    pub hurt: bool,
    /// A piglin's `NEAREST_VISIBLE_BABY_HOGLIN`.
    pub baby_hoglin: Option<u32>,
    /// `SetEntityLookTargetSometimes.Ticker.ticksUntilNextStart` of
    /// `babySometimesRideBabyHoglin`.
    pub ride_wait: i32,
    /// `Brain.getActiveNonCoreActivity` as `updateActivity` last left it.
    pub activity: Activity,
    /// The `last_hurt_by_time` of the hit `wasHurtBy` last reacted to.
    pub hurt_seen: i32,
    /// `VISIBLE_ADULT_PIGLIN_COUNT`: a piglin's counts its brutes too, a
    /// hoglin's only the piglins.
    pub piglins: i32,
    /// `VISIBLE_ADULT_HOGLIN_COUNT`.
    pub hoglins: i32,
    /// A hoglin's `NEAREST_VISIBLE_ADULT_PIGLIN`.
    pub nearest_piglin: Option<u32>,
    /// A hoglin's `NEAREST_VISIBLE_ADULT_HOGLINS` and a piglin's
    /// `NEAREST_VISIBLE_ADULT_PIGLINS` (brutes too), nearest first.
    pub adults: Vec<u32>,
    /// A piglin's `NEAREST_VISIBLE_ZOMBIFIED`.
    pub zombified: Option<Who>,
    /// A piglin's `NEAREST_VISIBLE_NEMESIS`: a wither skeleton or a wither.
    pub nemesis: Option<Who>,
    /// A piglin's `NEAREST_PLAYER_HOLDING_WANTED_ITEM` is set.
    pub sees_loved: bool,
    /// Retreats other mobs' `broadcastRetreat` asked of this one, which its
    /// next tick starts with: who to retreat from and where it stands.
    pub retreats: Vec<(Who, Vec3)>,
    /// The spear behaviours' memories (`SPEAR_STATUS`...).
    pub spear: super::spear_use::Memories,
    /// `ADMIRING_ITEM` set by `StartAdmiringItemIfSeen` (nothing in the off
    /// hand yet): the ticks left before it expires.
    pub seen_admire: i32,
    /// `TIME_TRYING_TO_REACH_ADMIRE_ITEM`, when present.
    pub reach_time: Option<i32>,
    /// `DISABLE_WALK_TO_ADMIRE_ITEM`: the ticks left.
    pub walk_off: i32,
    /// `ADMIRING_DISABLED`: the ticks left.
    pub admire_disabled: i32,
}
impl Memories {
    /// `StartAdmiringItemIfSeen.create(119)` (core activity) and the admire
    /// activity's `StopAdmiringIfItemTooFarAway(9)` and
    /// `StopAdmiringIfTiredOfTryingToReachItem(200, 200)`, from the distance
    /// squared to `NEAREST_VISIBLE_WANTED_ITEM` and whether that item is loved.
    /// `holding`: the off hand is not empty, which both stop behaviours check.
    pub fn admire_seen(&mut self, holding: bool, nearest: Option<f64>, loved: bool) {
        let absent = self.seen_admire == 0;
        if !absent && !holding {
            match nearest {
                Some(d) if d < 81.0 => match self.reach_time {
                    None => self.reach_time = Some(0),
                    Some(t) if t > 200 => {
                        self.seen_admire = 0;
                        self.reach_time = None;
                        self.walk_off = 200;
                    }
                    Some(t) => self.reach_time = Some(t + 1),
                },
                _ => self.seen_admire = 0,
            }
        }
        if absent && loved && self.admire_disabled == 0 && self.walk_off == 0 {
            self.seen_admire = ADMIRE;
        }
    }
}
/// `Sensor.isEntityTargetable`: `TargetingConditions.forNonCombat()` within
/// the follow range and in sight, which a `NearestVisibleLivingEntities`
/// applies to every entity it lists.
fn visible(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => {
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
/// `PiglinAi.isPlayerHoldingLovedItem`: gold or anything else in
/// `#piglin_loved`, in either hand.
fn holds_loved(w: &MobWorld, p: &super::PlayerView) -> bool {
    let Some(tag) = w
        .items
        .and_then(|i| i.item_tags.get("minecraft:piglin_loved"))
    else {
        return false;
    };
    [&p.main_hand, &p.off_hand]
        .into_iter()
        .flatten()
        .any(|item| tag.contains(&**item))
}
/// `HoglinSpecificSensor.doTick`: the adult piglins and hoglins in sight,
/// counted, with the nearest piglin and the hoglins themselves. (The sensor
/// also looks up the nearest repellent; `hoglin_repellent` does that.)
fn hoglin_sense(m: &mut Mob, w: &MobWorld) {
    let mut piglins = 0;
    let mut nearest = None;
    let mut hoglins = Vec::new();
    for who in living_near(m, w) {
        let Who::Mob(id) = who else { continue };
        let Some(s) = w.seen(id) else { continue };
        if s.baby || !matches!(s.species, Species::Piglin | Species::Hoglin) || !visible(m, w, who)
        {
            continue;
        }
        if s.species == Species::Piglin {
            piglins += 1;
            nearest = nearest.or(Some(id));
        } else {
            hoglins.push(id);
        }
    }
    let b = &mut m.extra.piglin;
    b.piglins = piglins;
    b.hoglins = hoglins.len() as i32;
    b.nearest_piglin = nearest;
    b.adults = hoglins;
}
/// `PiglinSpecificSensor.doTick`: what a piglin sees: adult piglins (brutes
/// too) and hoglins counted, the nearest baby hoglin, the nearest zombified
/// piglin or zoglin, the nearest wither skeleton or wither, and a player
/// holding gold.
fn piglin_sense(m: &mut Mob, w: &MobWorld) {
    let mut adults = Vec::new();
    let mut hoglins = 0;
    let mut baby_hoglin = None;
    let mut zombified = None;
    let mut nemesis = None;
    let mut loved = false;
    for who in living_near(m, w) {
        match who {
            Who::Player => {
                if !loved
                    && w.player
                        .as_ref()
                        .is_some_and(|p| !p.spectator && holds_loved(w, p))
                    && visible(m, w, who)
                {
                    loved = true;
                }
            }
            Who::Mob(id) => {
                let Some(s) = w.seen(id) else { continue };
                let (species, baby) = (s.species, s.baby);
                match species {
                    Species::Hoglin if baby && baby_hoglin.is_none() && visible(m, w, who) => {
                        baby_hoglin = Some(id);
                    }
                    Species::Hoglin if !baby && visible(m, w, who) => {
                        hoglins += 1;
                    }
                    Species::Piglin | Species::PiglinBrute if !baby && visible(m, w, who) => {
                        adults.push(id);
                    }
                    Species::WitherSkeleton | Species::Wither
                        if nemesis.is_none() && visible(m, w, who) =>
                    {
                        nemesis = Some(who);
                    }
                    Species::ZombifiedPiglin | Species::Zoglin
                        if zombified.is_none() && visible(m, w, who) =>
                    {
                        zombified = Some(who);
                    }
                    _ => {}
                }
            }
        }
    }
    // `HurtBySensor.doTick`, on the piglin sensor's schedule: damage is
    // forgotten 40 ticks after it was taken.
    let age = i64::from(m.age);
    let hurt = m.last_damage.as_ref().is_some_and(|(_, at)| age - at <= 40);
    let b = &mut m.extra.piglin;
    b.hurt = hurt;
    b.piglins = adults.len() as i32;
    b.adults = adults;
    b.hoglins = hoglins;
    b.baby_hoglin = baby_hoglin;
    b.zombified = zombified;
    b.nemesis = nemesis;
    b.sees_loved = loved;
}
/// `Brain.setActiveActivityToFirstValid`: the first valid activity of a
/// piglin's list (admiring, fighting, avoiding, celebrating, riding, idle), of
/// a brute's (fighting, idle) or of a hoglin's (fighting, avoiding, idle). An
/// activity is valid while the memory it requires is there.
pub fn activity(m: &Mob) -> Activity {
    let b = &m.extra.piglin;
    let piglin = m.species() == Species::Piglin;
    // `ADMIRING_ITEM`: a gold item in the off hand being admired (a charging
    // crossbow counts `using_item` up too), or one `StartAdmiringItemIfSeen`
    // saw lying around.
    if piglin && ((m.extra.off_hand.is_some() && m.extra.using_item > 0) || b.seen_admire > 0) {
        Activity::Admire
    } else if m.ai.target.is_some() {
        Activity::Fight
    } else if b.avoid.is_some() {
        Activity::Avoid
    } else if piglin && m.extra.scratch[1] > 0 && m.extra.home.is_some() {
        Activity::Celebrate
    } else if piglin && b.ride.is_some() {
        Activity::Ride
    } else {
        Activity::Idle
    }
}
/// `Brain.forgetOutdatedMemories`: the avoid and ride targets expire (the
/// avoid target keeps track of where it stands), then come the core
/// activity's copies into the avoid target, the retreat activity's
/// `EraseMemoryIf` and `updateActivity`, at the end of `customServerAiStep`'s
/// brain.
pub fn brain_step(m: &mut Mob, cx: &mut Cx) {
    let expired = match m.extra.piglin.avoid.as_mut() {
        Some(a) => {
            a.ttl.tick();
            !a.ttl.present()
        }
        None => false,
    };
    if expired {
        m.extra.piglin.avoid = None;
    }
    // `RIDE_TARGET` expires the same way.
    let ride_expired = match m.extra.piglin.ride.as_mut() {
        Some(r) => {
            r.ttl.tick();
            !r.ttl.present()
        }
        None => false,
    };
    if ride_expired {
        m.extra.piglin.ride = None;
    }
    if let Some(a) = m.extra.piglin.avoid.as_mut()
        && let Some(p) = cx.w.target_info(a.who)
    {
        a.at = p.position;
    }
    if m.species() == Species::Piglin {
        copy_to_avoid(m, cx);
        // The idle activity's behaviours: `babySometimesRideBabyHoglin`.
        if m.extra.piglin.activity == Activity::Idle {
            baby_sometimes_ride_baby_hoglin(m, cx);
        }
    }
    if m.extra.piglin.activity == Activity::Avoid && wants_to_stop_fleeing(m, cx.w) {
        m.extra.piglin.avoid = None;
    }
    // The ride activity's behaviours.
    if m.species() == Species::Piglin && m.extra.piglin.activity == Activity::Ride {
        mount(m, cx);
        dismount_or_skip_mounting(m, cx.w);
    }
    update_activity(m, cx.w, cx.events);
}
/// `Zoglin.customServerAiStep` after the brain: `Zoglin.updateActivity` plays
/// the angry grunt when the fight activity starts.
pub fn zoglin_step(m: &mut Mob, cx: &mut Cx) {
    let now = if m.ai.target.is_some() {
        Activity::Fight
    } else {
        Activity::Idle
    };
    let before = std::mem::replace(&mut m.extra.piglin.activity, now);
    if now == Activity::Fight && before != Activity::Fight {
        m.voice("entity.zoglin.angry", cx.events);
    }
}
/// `HoglinAi.updateActivity`, `PiglinAi.updateActivity` and
/// `PiglinBruteAi.updateActivity`: take the first valid activity and, when it
/// is not the one it was in, make the sound that goes with the new one; a
/// piglin then gets off a baby it rides once it has no ride target
/// (`isBabyRidingBaby`).
fn update_activity(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let now = activity(m);
    let before = std::mem::replace(&mut m.extra.piglin.activity, now);
    if before != now {
        match m.species() {
            Species::PiglinBrute => play_brute_sound(m, events),
            Species::Hoglin => m.voice(hoglin_sound(m, w, now), events),
            _ => m.voice(piglin_sound(m, w, now), events),
        }
    }
    // A baby that rides a baby but has no ride target (any more) gets off.
    if m.species() == Species::Piglin && m.extra.piglin.ride.is_none() && is_baby_riding_baby(m, w)
    {
        dismount(m, w);
    }
}
/// `PiglinAi.isBabyRidingBaby`: a baby piglin on a baby piglin or a baby
/// hoglin.
fn is_baby_riding_baby(m: &Mob, w: &MobWorld) -> bool {
    if !m.is_baby() {
        return false;
    }
    m.extra
        .mount
        .and_then(|id| w.seen(id))
        .is_some_and(|v| v.baby && matches!(v.species, Species::Piglin | Species::Hoglin))
}
/// `Hoglin.isConverting` and `AbstractPiglin.isConverting`: in a dimension
/// that is not piglin safe, a mob that is not immune and has AI.
fn converting(m: &Mob, w: &MobWorld) -> bool {
    !w.nether && !m.extra.flag2 && !m.no_ai
}
/// `Hoglin.getAmbientSound`: the sound of the activity it is in
/// (`HoglinAi.getSoundForCurrentActivity`).
pub fn hoglin_ambient(m: &Mob, w: &MobWorld) -> &'static str {
    hoglin_sound(m, w, m.extra.piglin.activity)
}
/// `Piglin.getAmbientSound`: the sound of the activity it is in
/// (`PiglinAi.getSoundForCurrentActivity`).
pub fn piglin_ambient(m: &Mob, w: &MobWorld) -> &'static str {
    piglin_sound(m, w, m.extra.piglin.activity)
}
/// `HoglinAi.getSoundForActivity`: a retreat grunt while retreating,
/// converting or near a repellent, an angry one while fighting.
fn hoglin_sound(m: &Mob, w: &MobWorld, activity: Activity) -> &'static str {
    if activity == Activity::Avoid || converting(m, w) {
        "entity.hoglin.retreat"
    } else if activity == Activity::Fight {
        "entity.hoglin.angry"
    } else if m.extra.repellent.is_some() {
        "entity.hoglin.retreat"
    } else {
        "entity.hoglin.ambient"
    }
}
/// `PiglinAi.getSoundForActivity`: angry while fighting, a retreat grunt
/// while converting, retreating from something within 12 blocks or near a
/// repellent, then admiring, celebrating, jealous of a player holding gold,
/// or the plain grunt.
fn piglin_sound(m: &Mob, w: &MobWorld, activity: Activity) -> &'static str {
    let b = &m.extra.piglin;
    if activity == Activity::Fight {
        "entity.piglin.angry"
    } else if converting(m, w)
        || (activity == Activity::Avoid && b.avoid.is_some_and(|a| m.distance_sqr(a.at) < 144.0))
    {
        "entity.piglin.retreat"
    } else if activity == Activity::Admire {
        "entity.piglin.admiring_item"
    } else if activity == Activity::Celebrate {
        "entity.piglin.celebrate"
    } else if b.sees_loved {
        "entity.piglin.jealous"
    } else if m.extra.repellent.is_some() {
        "entity.piglin.retreat"
    } else {
        "entity.piglin.ambient"
    }
}
/// `PiglinBruteAi.playActivitySound`: a brute's `PiglinBrute.playAngrySound`
/// while the fight activity is the active one; its other activity (idle) says
/// nothing.
fn play_brute_sound(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.extra.piglin.activity == Activity::Fight {
        m.voice("entity.piglin_brute.angry", events);
    }
}
/// `PiglinBruteAi.maybePlayActivitySound`: the level random is drawn on every
/// tick, fighting or not; a hit only growls in a fight.
fn maybe_play_brute_sound(m: &mut Mob, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if lr.next_float() < 0.0125 {
        play_brute_sound(m, events);
    }
}
/// `PiglinAi.getAvoidTarget`: the avoid target while the memory is there.
pub fn avoid_target(m: &Mob) -> Option<Who> {
    m.extra.piglin.avoid.map(|a| a.who)
}
/// `Brain.setMemoryWithExpiry`: `AVOID_TARGET` for `ticks` more ticks.
fn remember_avoid(m: &mut Mob, who: Who, at: Vec3, ticks: i32) {
    let mut ttl = Ttl::default();
    ttl.set(ticks);
    m.extra.piglin.avoid = Some(AvoidTarget { who, at, ttl });
}
/// `TimeUtil.rangeOfSeconds(5, 7)` of `PiglinAi.AVOID_ZOMBIFIED_DURATION` and
/// `BABY_AVOID_NEMESIS_DURATION`, sampled from the level random.
fn avoid_duration(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(100, 140).unwrap_or(100)
}
/// `PiglinAi.isNearZombified`: the nearest visible zombified piglin or zoglin
/// when it is closer than 6 blocks, and where it stands.
fn near_zombified(m: &Mob, w: &MobWorld) -> Option<(Who, Vec3)> {
    let who = m.extra.piglin.zombified?;
    let at = w.target_info(who)?.position;
    (m.distance_sqr(at) < 36.0).then_some((who, at))
}
/// `PiglinAi.babyAvoidNemesis` and `PiglinAi.avoidZombified`: the core
/// activity's `CopyMemoryWithExpiry` into `AVOID_TARGET`. A baby avoids a
/// wither skeleton or wither it sees, anyone a zombified piglin or zoglin
/// that is near. Each tick it applies draws its expiry from the level random.
fn copy_to_avoid(m: &mut Mob, cx: &mut Cx) {
    if m.is_baby()
        && let Some(who) = m.extra.piglin.nemesis
        && let Some(p) = cx.w.target_info(who)
    {
        let ticks = avoid_duration(cx.lr);
        remember_avoid(m, who, p.position, ticks);
    }
    if let Some((who, at)) = near_zombified(m, cx.w) {
        let ticks = avoid_duration(cx.lr);
        remember_avoid(m, who, at, ticks);
    }
}
/// `SetEntityLookTargetSometimes.Ticker.tickDownAndCheck`: with nothing left
/// to wait for, draw the next wait from `interval` and say no; otherwise
/// count down and say yes when it reaches nothing.
fn tick_down_and_check(wait: &mut i32, lr: &mut LegacyRandom, interval: (i32, i32)) -> bool {
    if *wait == 0 {
        *wait = lr
            .next_int_between_inclusive(interval.0, interval.1)
            .unwrap_or(interval.0)
            - 1;
        false
    } else {
        *wait -= 1;
        *wait == 0
    }
}
/// `PiglinAi.babySometimesRideBabyHoglin`: the idle activity's
/// `CopyMemoryWithExpiry` of the nearest baby hoglin it sees into
/// `RIDE_TARGET` for 10 to 30 seconds. It asks only while the target is
/// empty, and a baby's ticker lets it through once every 10 to 40 seconds
/// of asking (`RIDE_START_INTERVAL`, `RIDE_DURATION`, both drawn from the
/// level random).
fn baby_sometimes_ride_baby_hoglin(m: &mut Mob, cx: &mut Cx) {
    let b = &m.extra.piglin;
    let Some(hoglin) = b.baby_hoglin else { return };
    if b.ride.is_some() || !m.is_baby() {
        return;
    }
    if !tick_down_and_check(&mut m.extra.piglin.ride_wait, cx.lr, (200, 800)) {
        return;
    }
    let ticks = cx.lr.next_int_between_inclusive(200, 600).unwrap_or(200);
    let mut ttl = Ttl::default();
    ttl.set(ticks);
    m.extra.piglin.ride = Some(RideTarget {
        vehicle: hoglin,
        ttl,
        aimed: None,
    });
}
/// `PiglinAi.SPEED_MULTIPLIER_WHEN_MOUNTING`.
const MOUNTING_SPEED: f64 = 0.8;
/// The block under `at` (`BlockPos.containing`).
fn block_of(at: Vec3) -> [i32; 3] {
    [
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    ]
}
/// `Mount.create`: with no walk under way and a `RIDE_TARGET`, a piglin that
/// is not a passenger already boards the ride target once it is closer than a
/// block (`startRiding`), and otherwise looks at it and walks to it at 0.8
/// (`WalkTarget` on an `EntityTracker`). `MoveToTargetSink` carries the walk:
/// it ends once the target's block is a block away (`reachedTarget`) and aims
/// again when that block has moved more than 2 blocks.
fn mount(m: &mut Mob, cx: &mut Cx) {
    if m.extra.mount.is_some() {
        return;
    }
    let Some(ride) = m.extra.piglin.ride else {
        return;
    };
    let Some((at, eye_y)) = cx.w.seen(ride.vehicle).map(|s| (s.position, s.eye_y)) else {
        return;
    };
    let target = block_of(at);
    let here = block_of(m.position);
    let reached = (0..3).map(|i| (target[i] - here[i]).abs()).sum::<i32>() <= 1;
    if !m.ai.nav.is_done() {
        let moved = ride.aimed.is_none_or(|a| {
            (0..3)
                .map(|i| i64::from(target[i] - a[i]).pow(2))
                .sum::<i64>()
                > 4
        });
        if reached {
            m.nav_stop();
        } else if moved {
            aim_at_ride_target(m, cx, at);
        }
        return;
    }
    if m.distance_sqr(at) < 1.0 {
        cx.events.push(MobEvent::StartRiding {
            rider: m.id,
            vehicle: ride.vehicle,
        });
        return;
    }
    m.look_at([at.x, eye_y, at.z]);
    if !reached {
        aim_at_ride_target(m, cx, at);
    }
}
/// `MoveToTargetSink.tryComputePath` to the block the ride target stands in:
/// a path there at `SPEED_MULTIPLIER_WHEN_MOUNTING`, remembering where it was
/// aimed.
fn aim_at_ride_target(m: &mut Mob, cx: &mut Cx, at: Vec3) {
    if m.nav_move_to_entity(cx.w, [at.x, at.y, at.z], 0, MOUNTING_SPEED)
        && let Some(r) = m.extra.piglin.ride.as_mut()
    {
        r.aimed = Some(block_of(at));
    }
}
/// `PiglinAi.wantsToStopRiding`: a piglin wants off a mob that is not a baby
/// or not alive any more, when it was hurt recently, or when it is a piglin
/// that is not riding anything itself. (`wasHurtRecently` of the mob it rides
/// is `HURT_BY` in that mob's brain, which a hoglin's does not have.)
fn wants_to_stop_riding(m: &Mob, vehicle: &Seen) -> bool {
    !vehicle.baby
        || !vehicle.alive
        || m.extra.piglin.hurt
        || vehicle.hurt_recently
        || (vehicle.species == Species::Piglin && !vehicle.passenger)
}
/// `DismountOrSkipMounting.create(8, wantsToStopRiding)`: the vehicle it sits
/// on, or else the ride target, stays only while it is alive, closer than 8
/// blocks and the piglin does not want off it; if not, the piglin gets off
/// and forgets the ride target. A vehicle gone from the level counts as dead.
fn dismount_or_skip_mounting(m: &mut Mob, w: &MobWorld) {
    let Some(vehicle) = m.extra.mount.or(m.extra.piglin.ride.map(|r| r.vehicle)) else {
        return;
    };
    let keep = match w.seen(vehicle) {
        Some(s) => s.alive && m.distance_sqr(s.position) < 64.0 && !wants_to_stop_riding(m, s),
        None => false,
    };
    if keep {
        return;
    }
    dismount(m, w);
    m.extra.piglin.ride = None;
}
/// `LivingEntity.stopRiding`: get off the vehicle, onto the spot
/// `Animal.getDismountLocationForPassenger` finds beside it.
fn dismount(m: &mut Mob, w: &MobWorld) {
    let Some(vehicle) = m.extra.mount else { return };
    let at = w
        .seen(vehicle)
        .map(|s| super::riders::dismount_location(w, s, m.width(), m.height()));
    super::riders::stop_riding(m, at);
}
/// `HoglinAi.piglinsOutnumberHoglins`: an adult hoglin counts itself and
/// sees more adult piglins than hoglins.
pub fn piglins_outnumber_hoglins(m: &Mob) -> bool {
    !m.is_baby() && m.extra.piglin.piglins > m.extra.piglin.hoglins + 1
}
/// `PiglinAi.hoglinsOutnumberPiglins`: a piglin counts itself and sees more
/// adult hoglins than adult piglins.
pub fn hoglins_outnumber_piglins(m: &Mob) -> bool {
    m.extra.piglin.hoglins > m.extra.piglin.piglins + 1
}
/// `HoglinAi.wantsToStopFleeing` and `PiglinAi.wantsToStopFleeing`: the
/// retreat activity's `EraseMemoryIf`. A hoglin stops once it is an adult
/// and piglins no longer outnumber hoglins; a piglin once the hoglins it
/// avoids no longer outnumber piglins, or the zombified mob it avoids is not
/// the nearest one it sees any more. Any other avoid target waits out its
/// expiry.
fn wants_to_stop_fleeing(m: &Mob, w: &MobWorld) -> bool {
    let b = &m.extra.piglin;
    let Some(a) = b.avoid else { return true };
    if m.species() == Species::Hoglin {
        return !m.is_baby() && !piglins_outnumber_hoglins(m);
    }
    match species_of(w, a.who) {
        Some(Species::Hoglin) => !hoglins_outnumber_piglins(m),
        Some(Species::ZombifiedPiglin | Species::Zoglin) => b.zombified != Some(a.who),
        _ => false,
    }
}
/// `PiglinAi.RETREAT_DURATION` and `HoglinAi.RETREAT_DURATION`:
/// `TimeUtil.rangeOfSeconds(5, 20)`, sampled from the level random.
fn retreat_duration(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(100, 400).unwrap_or(100)
}
/// `PiglinAi.TIME_BETWEEN_HUNTS` (30 to 120 seconds) sampled from the level
/// random.
fn hunt_wait(lr: &mut LegacyRandom) -> i32 {
    lr.next_int_between_inclusive(600, 2400).unwrap_or(600)
}
/// `HoglinAi.setAvoidTarget`: forget the attack target and the walk, and
/// avoid `who`, standing at `at`, for a while.
pub fn set_avoid_target(m: &mut Mob, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    m.ai.target = None;
    m.nav_stop();
    let ticks = retreat_duration(lr);
    remember_avoid(m, who, at, ticks);
}
/// `PiglinAi.setAvoidTargetAndDontHuntForAWhile`: forget the anger, the
/// attack target and the walk, avoid `who` for a while and start the
/// `HUNTED_RECENTLY` wait (`dontKillAnyMoreHoglinsForAWhile`).
pub fn set_avoid_target_and_dont_hunt(m: &mut Mob, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    m.extra.anger = 0;
    set_avoid_target(m, lr, who, at);
    m.extra.ints[0] = hunt_wait(lr);
}
/// `HoglinAi.retreatFromNearestTarget` and `PiglinAi.retreatFromNearestTarget`:
/// retreat from the nearest of the new target, the avoid target and the
/// attack target (`BehaviorUtils.getNearestTarget`: the first one only when
/// it is strictly closer).
pub fn retreat_from_nearest(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, who: Who, at: Vec3) {
    let mut nearest = (who, at);
    let avoid = m.extra.piglin.avoid.map(|a| (a.who, a.at));
    let attack =
        m.ai.target
            .and_then(|t| w.target_info(t).map(|i| (t, i.position)));
    for other in [avoid, attack].into_iter().flatten() {
        if m.distance_sqr(other.1) < m.distance_sqr(nearest.1) {
            nearest = other;
        }
    }
    if m.species() == Species::Hoglin {
        set_avoid_target(m, lr, nearest.0, nearest.1);
    } else {
        set_avoid_target_and_dont_hunt(m, lr, nearest.0, nearest.1);
    }
}
/// `HoglinAi.broadcastRetreat` and `PiglinAi.broadcastRetreat`: every adult
/// hoglin (for a piglin, every adult piglin but the brutes) this one sees
/// retreats from `who` too, once its tick starts.
pub fn broadcast_retreat(m: &Mob, w: &MobWorld, who: Who, at: Vec3, events: &mut Vec<MobEvent>) {
    for &id in &m.extra.piglin.adults {
        if m.species() == Species::Piglin
            && !w.seen(id).is_some_and(|s| s.species == Species::Piglin)
        {
            continue;
        }
        events.push(MobEvent::Retreat {
            mob: id,
            from: who,
            at: [at.x, at.y, at.z],
        });
    }
}
/// What `broadcastRetreat` asks of a mob: it handles the retreat when its
/// next tick starts.
pub fn receive_retreat(m: &mut Mob, from: Who, at: [f64; 3]) {
    m.extra
        .piglin
        .retreats
        .push((from, Vec3::new(at[0], at[1], at[2])));
}
/// `BehaviorUtils.isOtherTargetMuchFurtherAwayThanCurrentAttackTarget` with
/// 4 blocks: `other` is more than 4 blocks further away than the attack
/// target.
fn much_further_than_target(m: &Mob, w: &MobWorld, other: Vec3) -> bool {
    let Some(current) =
        m.ai.target
            .and_then(|t| w.target_info(t).map(|i| i.position))
    else {
        return false;
    };
    m.distance_sqr(other) > m.distance_sqr(current) + 16.0
}
/// `Sensor.isEntityAttackable`, or `isEntityAttackableIgnoringLineOfSight`
/// without `check_sight`: `TargetingConditions.forCombat()` within the
/// follow range.
fn attackable(m: &mut Mob, w: &MobWorld, who: Who, check_sight: bool) -> bool {
    let range = m.attrs.follow_range;
    match who {
        Who::Player => m.can_target_player(w, range, check_sight),
        Who::Mob(id) => {
            let Some(s) = w.seen(id) else { return false };
            s.alive
                && !s.playing_dead
                && m.distance_sqr(s.position) <= range * range
                && (!check_sight || m.sees(w, who))
        }
    }
}
/// `HoglinAi.maybeRetaliate`: the tests for an adult. It does not take on a
/// piglin while it retreats, nor a hoglin, nor an attacker more than 4
/// blocks further away than its target, nor one it cannot attack.
fn hoglin_retaliates(m: &mut Mob, w: &MobWorld, by: Who, at: Vec3) -> bool {
    let species = species_of(w, by);
    if m.extra.piglin.activity == Activity::Avoid && species == Some(Species::Piglin) {
        return false;
    }
    species != Some(Species::Hoglin)
        && !much_further_than_target(m, w, at)
        && attackable(m, w, by, true)
}
/// `HoglinAi.wasHurtBy`, which `Hoglin.hurtServer` calls: a hit ends the
/// pacified state; a baby retreats from the attacker, an adult retaliates
/// when `maybeRetaliate` allows. The hurt-by goal does the retaliating, so a
/// hit that does not get it is used up.
fn hoglin_hurt(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, by: Who) {
    m.extra.ints[0] = 0;
    let Some(at) = w.target_info(by).map(|t| t.position) else {
        return;
    };
    if m.is_baby() {
        retreat_from_nearest(m, w, lr, by, at);
        m.ai.last_hurt_by = None;
    } else if !hoglin_retaliates(m, w, by, at) {
        m.ai.last_hurt_by = None;
    }
}
/// `PiglinAi.stopHoldingOffHandItem` without bartering (`wasHurtBy`): a gold
/// ingot is not bartered and is gone; anything else ends its admiring next
/// tick, as if the time ran out, and the game side puts it away.
pub fn stop_admiring(m: &mut Mob) {
    if m.extra.off_hand.as_deref() == Some("minecraft:gold_ingot") {
        m.extra.off_hand = None;
        m.extra.using_item = 0;
    } else {
        m.extra.using_item = i32::from(m.extra.off_hand.is_some() && m.extra.using_item > 0);
    }
}
/// Whether two entities are of one type: both the player, or both mobs of
/// one species.
fn same_type(w: &MobWorld, a: Who, b: Who) -> bool {
    match (a, b) {
        (Who::Player, Who::Player) => true,
        (Who::Mob(_), Who::Mob(_)) => {
            species_of(w, a).is_some() && species_of(w, a) == species_of(w, b)
        }
        _ => false,
    }
}
/// `PiglinAi.broadcastAngerTarget`: as a baby's hit calls it, the adult
/// piglins around it that have no target (brutes too, unless the attacker is
/// a hoglin, which they cannot hunt) take the attacker as theirs.
fn broadcast_anger(m: &Mob, w: &MobWorld, by: Who, at: Vec3, events: &mut Vec<MobEvent>) {
    let range = m.attrs.follow_range;
    let hoglin = species_of(w, by) == Some(Species::Hoglin);
    // Hoglin.canBeHunted: an adult.
    let huntable = matches!(by, Who::Mob(id) if w.seen(id).is_some_and(|s| !s.baby));
    if hoglin && !huntable {
        return;
    }
    let id = m.id;
    let others: Vec<u32> = w
        .nearby(m.position, range, move |s| {
            s.id != id
                && !s.baby
                && s.target.is_none()
                && (s.species == Species::Piglin || (s.species == Species::PiglinBrute && !hoglin))
                && s.position.distance_squared(at) <= range * range
        })
        .map(|s| s.id)
        .collect();
    for mob in others {
        events.push(MobEvent::SetTarget { mob, target: by });
    }
}
/// `PiglinAi.maybeRetaliate`: the tests. Not while retreating, only against
/// an attacker it can attack and that is not more than 4 blocks further away
/// than its target.
fn piglin_retaliates(m: &mut Mob, w: &MobWorld, by: Who, at: Vec3) -> bool {
    m.extra.piglin.activity != Activity::Avoid
        && attackable(m, w, by, false)
        && !much_further_than_target(m, w, at)
}
/// `PiglinAi.wasHurtBy`, which `Piglin.hurtServer` calls: another piglin's
/// hit changes nothing. Any other ends the admiring, the celebration and the
/// dance, and an avoid target of another kind. A baby flees the attacker for 5 seconds
/// and calls the adults around it; an adult retreats from a hoglin when
/// hoglins outnumber piglins and otherwise retaliates when `maybeRetaliate`
/// allows. The hurt-by goal does the retaliating, so a hit that does not get
/// it is used up.
fn piglin_hurt(
    m: &mut Mob,
    w: &MobWorld,
    lr: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
    by: Who,
) {
    let species = species_of(w, by);
    if species == Some(Species::Piglin) {
        return;
    }
    stop_admiring(m);
    // `brain.eraseMemory(ADMIRING_ITEM)`, and a player's blow disables
    // admiring for 400 ticks.
    m.extra.piglin.seen_admire = 0;
    if by == Who::Player {
        m.extra.piglin.admire_disabled = 400;
    }
    m.extra.scratch[0] = 0;
    m.extra.scratch[1] = 0;
    m.extra.home = None;
    if let Some(avoided) = avoid_target(m)
        && !same_type(w, avoided, by)
    {
        m.extra.piglin.avoid = None;
    }
    let Some(at) = w.target_info(by).map(|t| t.position) else {
        return;
    };
    if m.is_baby() {
        remember_avoid(m, by, at, 100);
        if attackable(m, w, by, false) {
            broadcast_anger(m, w, by, at, events);
        }
        m.ai.last_hurt_by = None;
    } else if species == Some(Species::Hoglin) && hoglins_outnumber_piglins(m) {
        set_avoid_target_and_dont_hunt(m, lr, by, at);
        broadcast_retreat(m, w, by, at, events);
        m.ai.last_hurt_by = None;
    } else if !piglin_retaliates(m, w, by, at) {
        m.ai.last_hurt_by = None;
    }
}
/// What a piglin or hoglin handles before its goals run: the retreats other
/// mobs broadcast to it, the hit `wasHurtBy` reacts to and, for a piglin,
/// its celebration.
pub fn before_goals(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    for (from, at) in std::mem::take(&mut m.extra.piglin.retreats) {
        retreat_from_nearest(m, w, lr, from, at);
    }
    if let Some(by) = m.ai.last_hurt_by
        && m.ai.last_hurt_by_time != m.extra.piglin.hurt_seen
    {
        m.extra.piglin.hurt_seen = m.ai.last_hurt_by_time;
        if m.species() == Species::Hoglin {
            hoglin_hurt(m, w, lr, by);
        } else {
            piglin_hurt(m, w, lr, events, by);
        }
    }
    if m.species() == Species::Piglin {
        celebrate(m, w);
    }
}
/// What kind of mob `who` is: none for the player.
fn species_of(w: &MobWorld, who: Who) -> Option<Species> {
    match who {
        Who::Player => None,
        Who::Mob(id) => w.seen(id).map(|s| s.species),
    }
}
/// What `SetWalkTargetAwayFrom.entity` walks away from.
#[derive(Clone, Copy, Debug)]
enum AwayFrom {
    /// The retreat activity's `AVOID_TARGET`.
    AvoidTarget,
    /// The idle activity's `NEAREST_VISIBLE_ADULT_PIGLIN` (a hoglin's).
    AdultPiglin,
}
/// `SetWalkTargetAwayFrom.entity(memory, speed, desiredDistance, interruptCurrentWalk)`:
/// closer than the desired distance to what it avoids, pick a spot away
/// from it and walk there, unless a walk is on and may not be interrupted.
/// A walk at the same speed whose spot lies away from it is left alone.
#[derive(Clone, Debug)]
struct WalkAway {
    from: AwayFrom,
    speed: f64,
    desired: f64,
    interrupt: bool,
}
impl WalkAway {
    fn retreat(speed: f64, desired: f64, interrupt: bool) -> Self {
        Self {
            from: AwayFrom::AvoidTarget,
            speed,
            desired,
            interrupt,
        }
    }
    fn adult_piglin(speed: f64, desired: f64, interrupt: bool) -> Self {
        Self {
            from: AwayFrom::AdultPiglin,
            speed,
            desired,
            interrupt,
        }
    }
    /// Where the avoided entity stands, while the activity this behaviour
    /// belongs to is the active one.
    fn source(&self, m: &Mob, w: &MobWorld) -> Option<Vec3> {
        let b = &m.extra.piglin;
        match self.from {
            AwayFrom::AvoidTarget if b.activity == Activity::Avoid => b.avoid.map(|a| a.at),
            AwayFrom::AdultPiglin if b.activity == Activity::Idle && !m.is_baby() => b
                .nearest_piglin
                .and_then(|id| w.seen(id))
                .map(|s| s.position),
            _ => None,
        }
    }
    /// One run of the behaviour: whether it sets a new walk target, which
    /// it leaves as the path in `pending_path`.
    fn step(&self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(from) = self.source(m, cx.w) else {
            return false;
        };
        let walking = !m.ai.nav.is_done();
        if walking && !self.interrupt {
            return false;
        }
        if m.distance_sqr(from) >= self.desired * self.desired {
            return false;
        }
        if walking
            && m.ai.nav.speed_modifier == self.speed
            && let Some(end) = m.ai.nav.path.as_ref().and_then(|p| p.end())
        {
            let (cx0, cy0, cz0) = (
                f64::from(end.x) + 0.5 - m.position.x,
                f64::from(end.y) - m.position.y,
                f64::from(end.z) + 0.5 - m.position.z,
            );
            let (ax, ay, az) = (
                from.x - m.position.x,
                from.y - m.position.y,
                from.z - m.position.z,
            );
            if cx0 * ax + cy0 * ay + cz0 * az < 0.0 {
                return false;
            }
        }
        let away = [from.x, from.y, from.z];
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, away) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
}
impl Goal for WalkAway {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.step(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.source(m, cx.w).is_some() && !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, self.speed);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.step(m, cx) {
            let path = m.ai.pending_path.take();
            m.nav_move_to_path(cx.w, path, self.speed);
        }
    }
}
/// A goal of the idle and fight activities: it waits while the brain is in
/// the retreat activity.
#[derive(Clone, Debug)]
struct NotRetreating<G>(G);
impl<G: Goal + Clone + 'static> Goal for NotRetreating<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Avoid && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Avoid && self.0.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.0.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
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

/// A goal of the idle and avoid activities' movement: the ride activity has
/// none of it (`PiglinAi.initRideHoglinActivity`), so it waits while the
/// brain is in the ride activity.
#[derive(Clone, Debug)]
struct NotRiding<G>(G);
impl<G: Goal + Clone + 'static> Goal for NotRiding<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Ride && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.piglin.activity != Activity::Ride && self.0.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.0.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
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

fn hostile_to_player(m: &Mob, w: &MobWorld) -> bool {
    // `StartAttacking` is in the idle and celebrate activities, not in the retreat.
    if m.is_baby() || m.extra.piglin.activity == Activity::Avoid {
        return false;
    }
    match m.species() {
        Species::Piglin => m.extra.anger > 0 || w.player.as_ref().is_some_and(|p| !p.gold_armor),
        _ => true,
    }
}
/// `BackUpIfTooClose.create(5, 0.75)` of `PiglinAi.initFightActivity`, behind
/// `hasCrossbow`: with no walk target and the attack target in sight within 5
/// blocks, look at it and strafe backwards every tick. It takes no flags so the
/// crossbow goal keeps aiming meanwhile.
#[derive(Clone, Debug)]
struct BackUpIfTooClose;
impl BackUpIfTooClose {
    fn too_close(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
        if m.extra.main_hand.as_deref() != Some("minecraft:crossbow") || !m.ai.nav.is_done() {
            return None;
        }
        let who = m.ai.target?;
        let t = cx.w.target_info(who)?;
        if !t.position.closer_than(m.position, 5.0) || !m.sees(cx.w, who) {
            return None;
        }
        Some([t.position.x, t.eye_y, t.position.z])
    }
}
impl Goal for BackUpIfTooClose {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        Self::too_close(m, cx).is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(eye) = Self::too_close(m, cx) else {
            return;
        };
        m.look_at(eye);
        m.ai.mov.strafe(-0.75, 0.0);
        m.yaw = super::control::rotate_if_necessary(m.yaw, m.head_yaw, 0.0);
    }
}
pub fn register_piglin(m: &mut Mob) {
    // `AbstractPiglin.<init>`: a piglin's navigation opens doors.
    m.ai.nav.can_open_doors = true;
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // `PiglinAi.initCoreActivity`: `InteractWithDoor.create()`.
    g.add(0, super::doors::InteractWithDoor::default());
    g.add(1, Admire);
    g.add(1, AdmireSeen);
    // `PiglinAi.initRetreatActivity`: `SetWalkTargetAwayFrom.entity(AVOID_TARGET, 1.0, 12, true)`.
    g.add(2, WalkAway::retreat(1.0, 12.0, true));
    // Registered before the spawn weapon is equipped, so the goal checks the
    // held item itself (it only starts while a crossbow is in the main hand).
    g.add(3, super::illager::crossbow_goal(1.0, 8.0));
    g.add(3, BackUpIfTooClose);
    // `PiglinAi.initFightActivity`: the spear behaviours come before the melee one.
    g.add(3, super::spear_use::PiglinSpear::new());
    g.add(
        3,
        super::spear_use::NoMeleeWithSpear(MeleeAttack::new(1.0, true)),
    );
    g.add(4, NotRiding(PiglinAvoidRepellent));
    g.add(4, GoToWantedItem);
    g.add(4, Celebrate);
    g.add(5, NotRiding(Stroll::avoiding_water(0.6)));
    g.add(5, LookAtLoved);
    g.add(6, LookAtPlayer::new(8.0));
    g.add(7, RandomLookAround::default());
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets
        .add(2, NearestTarget::player(true).when(hostile_to_player));
    m.ai.targets.add(3, HuntHoglin::default());
    // `PiglinAi.initMemories`: the first hunt waits too.
    m.extra.ints[0] = time_between_hunts(m);
}
/// `PiglinAi.TIME_BETWEEN_HUNTS`: 30 to 120 seconds.
fn time_between_hunts(m: &mut Mob) -> i32 {
    m.random
        .next_int_between_inclusive(600, 2400)
        .unwrap_or(600)
}
/// `StartHuntingHoglin`: an adult piglin that is not angry, has not hunted
/// recently and sees no adult piglin that has, goes for the nearest adult
/// hoglin it sees; then it and the adult piglins near it wait before the next
/// hunt, and the others join in (`broadcastAngerTarget`).
#[derive(Clone, Debug)]
struct HuntHoglin {
    found: Option<u32>,
    st: TargetState,
}
impl Default for HuntHoglin {
    fn default() -> Self {
        let mut st = TargetState::default();
        st.unseen_memory = 60;
        Self { found: None, st }
    }
}
impl Goal for HuntHoglin {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.is_baby()
            || m.extra.ints[0] > 0
            || m.extra.anger > 0
            || m.ai.target.is_some()
            || m.extra.piglin.activity == Activity::Avoid
        {
            return false;
        }
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        if cx
            .w
            .nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby && s.hunted_recently
            })
            .next()
            .is_some()
        {
            return false;
        }
        let hoglins: Vec<(u32, Vec3)> =
            cx.w.nearby(pos, range, |s| s.species == Species::Hoglin && !s.baby)
                .map(|s| (s.id, s.position))
                .collect();
        self.found = hoglins
            .into_iter()
            .find(|&(hoglin, at)| {
                m.distance_sqr(at) <= range * range && m.sees(cx.w, Who::Mob(hoglin))
            })
            .map(|(hoglin, _)| hoglin);
        self.found.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        target_continue(&mut self.st, true, m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(hoglin) = self.found else { return };
        m.ai.target = Some(Who::Mob(hoglin));
        m.extra.ints[0] = time_between_hunts(m);
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        let others: Vec<u32> =
            cx.w.nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby
            })
            .map(|s| s.id)
            .collect();
        for mob in others {
            let ticks = time_between_hunts(m);
            cx.events.push(MobEvent::HuntHoglin {
                mob,
                target: hoglin,
                ticks,
            });
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
pub fn register_brute(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, MeleeAttack::new(1.0, true));
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::WitherSkeleton], true));
}
pub fn register_hoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // `HoglinAi.initRetreatActivity`: `SetWalkTargetAwayFrom.entity(AVOID_TARGET, 1.3, 15, false)`.
    g.add(1, WalkAway::retreat(1.3, 15.0, false));
    // HoglinAi: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(3, NotRetreating(super::goals::Breed::new(1.0)));
    g.add(3, HoglinAvoidRepellent);
    // `HoglinAi.initIdleActivity`: an adult keeps away from the nearest adult
    // piglin it sees, `SetWalkTargetAwayFrom.entity(NEAREST_VISIBLE_ADULT_PIGLIN, 0.4, 8, false)`.
    g.add(3, WalkAway::adult_piglin(0.4, 8.0, false));
    g.add(4, NotRetreating(super::goals::FollowParent::new(1.1)));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new().alert());
    // `HoglinAi.findNearestValidAttackTarget`: nothing while pacified, and
    // `StartAttacking` is not in the retreat activity.
    m.ai.targets.add(
        2,
        NearestTarget::player(true).when(|m, _| {
            !m.is_baby() && m.extra.ints[0] <= 0 && m.extra.piglin.activity != Activity::Avoid
        }),
    );
}
pub fn register_zoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // Zoglin.initMemories: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new());
    m.ai.targets.add(2, NearestTarget::player(true));
    // Zoglin.isTargetable: anything but creepers and other zoglins.
    m.ai.targets.add(
        3,
        NearestTarget::new(
            Prey::Filter(|s| !matches!(s.species, Species::Creeper | Species::Zoglin)),
            true,
        )
        .interval(10),
    );
}
/// `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, true)` of the hoglin's
/// idle activity: closer than 8 blocks to a repellent, walk away from it.
#[derive(Clone, Debug)]
struct HoglinAvoidRepellent;
impl Goal for HoglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // The idle activity's behaviour: not in the retreat.
        if m.extra.piglin.activity == Activity::Avoid {
            return false;
        }
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, 1.0);
    }
}
/// `PiglinAi.avoidRepellent`: `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, false)`
/// of the idle activity. Closer than 8 blocks to a repellent and not walking
/// anywhere, walk away from it.
#[derive(Clone, Debug)]
struct PiglinAvoidRepellent;
impl Goal for PiglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // The idle and celebrate activities: not fighting, admiring or
        // retreating, and no walk target yet.
        if m.ai.target.is_some()
            || m.extra.using_item > 0
            || m.extra.piglin.activity == Activity::Avoid
            || !m.ai.nav.is_done()
        {
            return false;
        }
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, 1.0);
    }
}
/// `PiglinAi.isPlayerHoldingLovedItem`: the idle and celebrate activities'
/// `SetEntityLookTarget.create(.., 14.0)` and the ride activity's, with 8.0,
/// turn to look at the player in sight within that many blocks who holds gold.
#[derive(Clone, Debug)]
struct LookAtLoved;
impl LookAtLoved {
    /// The eye of the player to watch, if there is one.
    fn watched(m: &mut Mob, cx: &mut Cx) -> Option<[f64; 3]> {
        let range_sqr = match m.extra.piglin.activity {
            Activity::Idle | Activity::Celebrate => 196.0,
            Activity::Ride => 64.0,
            _ => return None,
        };
        let p = cx.w.player.as_ref()?;
        if !holds_loved(cx.w, p) || m.distance_sqr(p.position) > range_sqr {
            return None;
        }
        let eye = p.eye;
        let range = m.attrs.follow_range;
        m.can_see_player(cx.w, range).then_some(eye)
    }
}
impl Goal for LookAtLoved {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        Self::watched(m, cx).is_some()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(eye) = Self::watched(m, cx) {
            m.look_at(eye);
        }
    }
}
/// Admiring a gold item held in the off hand: stand and look at it.
#[derive(Clone, Debug)]
struct Admire;
/// `initAdmireItemActivity` for an item only seen: the idle and fight behaviours
/// stand by while `GoToWantedItem` (the game sets the point) walks to it.
#[derive(Clone, Debug)]
struct AdmireSeen;
impl Goal for AdmireSeen {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.piglin.seen_admire > 0 && m.extra.off_hand.is_none()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = m.extra.point
            && m.ai.nav.is_done()
        {
            m.nav_move_to(cx.w, p, 1.0);
        }
    }
}
impl Goal for Admire {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.using_item > 0 && m.extra.off_hand.is_some()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// The celebrate activity: `GoToTargetLocation` on the spot a killed target
/// fell (within 2 blocks, or 4 and slower while dancing).
#[derive(Clone, Debug)]
struct Celebrate;
impl Celebrate {
    fn walk(m: &mut Mob, cx: &mut Cx) {
        let Some(at) = m.extra.home else { return };
        let (close, speed) = if m.extra.scratch[0] > 0 {
            (4.0, 0.6)
        } else {
            (2.0, 1.0)
        };
        let center = Vec3::new(
            f64::from(at[0]) + 0.5,
            f64::from(at[1]),
            f64::from(at[2]) + 0.5,
        );
        if m.distance_sqr(center) >= close * close {
            m.nav_move_to(cx.w, [center.x, center.y, center.z], speed);
        }
    }
}
impl Goal for Celebrate {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.scratch[1] > 0 && m.extra.home.is_some() && m.extra.using_item == 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        Self::walk(m, cx);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.ai.nav.is_done() {
            Self::walk(m, cx);
        }
    }
}
/// `GoToWantedItem`: walk to the gold the game spotted.
#[derive(Clone, Debug)]
struct GoToWantedItem;
impl Goal for GoToWantedItem {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = m.extra.point else {
            return false;
        };
        m.extra.using_item == 0 && m.nav_move_to(cx.w, p, 1.0)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.point.is_some() && !m.ai.nav.is_done()
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.point = None;
    }
}
/// `HoglinSpecificSensor.findNearestRepellent`: the nearest `#hoglin_repellents`
/// block within 8 blocks sideways and 4 up or down.
fn find_hoglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:hoglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |_, name| tag.contains(name))
}
/// The hoglin's repellent memories: the sensors (every 20 ticks), the end of
/// `PACIFIED` (`ints[0]`) and `BecomePassiveIfMemoryPresent(NEAREST_REPELLENT, 200)`.
fn hoglin_repellent(m: &mut Mob, w: &MobWorld) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    if m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_hoglin_repellent(m, w);
        hoglin_sense(m, w);
    }
    // `BecomePassiveIfMemoryPresent` is in the idle and fight activities.
    if m.extra.ints[0] <= 0
        && m.extra.repellent.is_some()
        && m.extra.piglin.activity != Activity::Avoid
    {
        m.extra.ints[0] = 200;
        m.ai.target = None;
    }
}
/// `StartCelebratingIfTargetDead` for a piglin (`scratch[0]` dancing,
/// `scratch[1]` ticks of celebration left, `home` the `CELEBRATE_LOCATION`):
/// a killed target starts a celebration at its spot, and a hoglin's death
/// makes the piglin dance one time in ten. `piglin_hurt` ends both. Runs
/// before the target goals drop the dead target.
pub fn celebrate(m: &mut Mob, w: &MobWorld) {
    if m.extra.scratch[1] > 0 {
        m.extra.scratch[1] -= 1;
        if m.extra.scratch[1] == 0 {
            m.extra.scratch[0] = 0;
            m.extra.home = None;
        }
    }
    if m.extra.scratch[1] > 0 {
        return;
    }
    let Some(target) = m.ai.target else { return };
    let Some(t) = w.target_info(target).filter(|t| !t.alive) else {
        return;
    };
    let hoglin =
        matches!(target, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Hoglin));
    // PiglinAi.wantsToDance: a fresh random seeded with the game time, not
    // the piglin's own.
    if hoglin && LegacyRandom::new(w.game_time).next_float() < 0.1 {
        m.extra.scratch[0] = 1;
    }
    m.extra.scratch[1] = CELEBRATION;
    m.extra.home = Some([
        t.position.x.floor() as i32,
        t.position.y.floor() as i32,
        t.position.z.floor() as i32,
    ]);
}
/// `Hoglin.isConverting` and `AbstractPiglin.isConverting` on a client: in a
/// dimension that is not piglin safe (`nether` is false), a mob that is not
/// immune (`flag2`) and has AI shakes. `counter2 > 0` draws the shake, as it
/// does on the server while the zombification counts up.
pub fn client_converting(m: &mut Mob, nether: bool) {
    m.extra.counter2 = i32::from(!nether && !m.extra.flag2 && !m.no_ai);
}
/// Per-tick piglin, piglin brute and hoglin state: the brute's activity and
/// growl, a piglin's admiring, anger and zombification, and the hoglin attack
/// animation.
pub fn ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.species() == Species::Hoglin && m.alive() {
        hoglin_repellent(m, w);
    }
    if m.species() == Species::Piglin {
        if m.extra.ints[0] > 0 {
            m.extra.ints[0] -= 1;
        }
        // `StopAttackingIfTargetInvalid`: a hoglin handed over by a hunt
        // broadcast has no target goal of its own to drop it when it dies.
        if let Some(who @ Who::Mob(_)) = m.ai.target
            && w.target_info(who)
                .is_none_or(|t| !t.alive || !t.can_be_seen_as_enemy)
        {
            m.ai.target = None;
        }
    }
    // `PiglinBrute.customServerAiStep`: `updateActivity`, then
    // `maybePlayActivitySound`, after the goals that pick the brute's target.
    if m.alive() && m.species() == Species::PiglinBrute {
        update_activity(m, w, events);
        maybe_play_brute_sound(m, level_random, events);
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) && !hostile_to_player(m, w) {
            m.ai.target = None;
        }
    }
    // `Brain.forgetOutdatedMemories`: `ADMIRING_DISABLED`, `DISABLE_WALK_TO_ADMIRE_ITEM`
    // and an `ADMIRING_ITEM` nothing is held for run out.
    let b = &mut m.extra.piglin;
    b.admire_disabled = (b.admire_disabled - 1).max(0);
    b.walk_off = (b.walk_off - 1).max(0);
    b.seen_admire = (b.seen_admire - 1).max(0);
    // The admired item is in the off hand; a spear in use counts up instead.
    if m.extra.using_item > 0 && m.extra.off_hand.is_some() {
        m.extra.using_item -= 1;
        if m.extra.using_item == 0 {
            events.push(MobEvent::FinishAdmiring { mob: m.id });
        }
    }
    // Zombification outside the Nether (piglins without immunity, hoglins).
    if !w.nether && !m.extra.flag2 && m.alive() {
        m.extra.counter2 += 1;
        if m.extra.counter2 > CONVERSION {
            // `Hoglin.finishConversion` is `ConversionParams.single(this, true,
            // false)`, `AbstractPiglin.finishConversion` `(this, true, true)`;
            // both leave the new mob nauseous (the game adds it).
            let (kind, sound, params) = match m.species() {
                Species::Hoglin => (
                    "minecraft:zoglin",
                    "entity.hoglin.converted_to_zombified",
                    super::conversion::Params {
                        keep_equipment: true,
                        preserve_can_pick_up_loot: false,
                    },
                ),
                Species::PiglinBrute => (
                    "minecraft:zombified_piglin",
                    "entity.piglin_brute.converted_to_zombified",
                    super::conversion::Params::KEEP_ALL,
                ),
                _ => (
                    "minecraft:zombified_piglin",
                    "entity.piglin.converted_to_zombified",
                    super::conversion::Params::KEEP_ALL,
                ),
            };
            events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
            events.push(MobEvent::Convert {
                from: Box::new(m.clone()),
                kind,
                params,
            });
            m.removed = true;
        }
    } else {
        m.extra.counter2 = 0;
    }
    // PiglinSpecificSensor (every 20 ticks): the nearest repelling block.
    if m.species() == Species::Piglin && m.alive() && m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_piglin_repellent(m, w);
        piglin_sense(m, w);
    }
}
/// `PiglinSpecificSensor.findNearestRepellent`: the nearest `#piglin_repellents`
/// block within 8 blocks sideways and 4 up or down; a soul campfire only
/// counts while lit.
fn find_piglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:piglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |p, name| {
        tag.contains(name)
            && (name != "minecraft:soul_campfire"
                || w.prop(p[0], p[1], p[2], "lit") == Some("true"))
    })
}
/// `HoglinBase.throwTarget`: a shove away and up of the attack knockback less
/// the target's knockback resistance (none at all when that is used up).
fn throw_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    let resistance = match who {
        Who::Mob(id) => cx.w.seen(id).map_or(0.0, |s| s.knockback_resistance),
        Who::Player => 0.0,
    };
    let power = m.kind.attack_knockback - resistance;
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    if power <= 0.0 {
        return;
    }
    let away = Vec3::new(
        t.position.x - m.position.x,
        0.0,
        t.position.z - m.position.z,
    );
    // `yRot` takes the random whole number of degrees as radians, as vanilla does.
    let angle = (m.random.next_int_bound(21).unwrap_or(10) - 10) as f32;
    let scale = power * f64::from(m.random.next_float() * 0.5 + 0.2);
    let push = away.normalize().scale(scale).rotate_y(angle);
    let up = power * f64::from(m.random.next_float()) * 0.5;
    cx.events.push(MobEvent::Push {
        target: who,
        velocity: [push.x, up, push.z],
    });
}
/// `HoglinAi.onHitTarget`: an adult that hits a piglin while piglins
/// outnumber the hoglins around it retreats from that piglin and tells the
/// hoglins it sees to. (The broadcast of the attack target otherwise is not
/// ported.)
fn on_hit_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    if m.is_baby() {
        return;
    }
    let Some(at) = cx.w.target_info(who).map(|t| t.position) else {
        return;
    };
    if species_of(cx.w, who) == Some(Species::Piglin) && piglins_outnumber_hoglins(m) {
        set_avoid_target(m, cx.lr, who, at);
        broadcast_retreat(m, cx.w, who, at, cx.events);
    }
}
/// `HoglinBase.hurtAndThrowTarget`.
pub fn hoglin_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
    // Entity event 4: clients start the headbutt.
    cx.events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: [m.position.x, m.position.y, m.position.z],
    });
    if m.species() == Species::Hoglin {
        on_hit_target(m, who, cx);
    }
    let attack = m.attrs.attack_damage;
    let amount = if !m.is_baby() && attack as i32 > 0 {
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
    if !m.is_baby() {
        throw_target(m, who, cx);
    }
    let sound = if m.species() == Species::Zoglin {
        "entity.zoglin.attack"
    } else {
        "entity.hoglin.attack"
    };
    cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
}
/// `AbstractPiglinModel.setupAnim`: the ears flap around their default angle
/// (30 degrees for adults, 5 for babies).
pub fn ears(p: &mut Poses, s: &State) {
    use horizon_core::geometry::trig::DEG_TO_RAD;
    let default = if s.baby { 5.0 } else { 30.0 } * DEG_TO_RAD;
    let freq = s.age * 0.1 + s.pos * 0.5;
    let amp = 0.08 + s.speed * 0.4;
    p.zrot("left_ear", -default - (freq * 1.2).mcos() * amp);
    p.zrot("right_ear", default + freq.mcos() * amp);
}
/// Piglins' and brutes' ears flap (`AbstractPiglinModel`), arms follow the
/// arm pose (`PiglinModel`).
pub fn piglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    // HumanoidMobRenderer.getArmPose: the main arm holding a spear (at rest
    // too), which charges while a spear is in use (an admired item is not).
    let spear = m
        .extra
        .main_hand
        .as_deref()
        .and_then(crate::spear::Kinetic::of)
        .map(|kinetic| {
            let using = m.extra.using_item > 0 && m.extra.off_hand.is_none();
            (
                kinetic,
                if using {
                    m.extra.using_item as f32 + s.partial
                } else {
                    0.0
                },
            )
        });
    // Piglin.getArmPose: dancing comes first.
    let pose = if m.extra.scratch[0] > 0 {
        PiglinPose::Dancing
    } else if m.extra.using_item > 0 && m.extra.off_hand.is_some() {
        PiglinPose::AdmiringItem
    } else if m.extra.flag {
        PiglinPose::CrossbowCharge
    } else if m.extra.main_hand.as_deref() == Some("minecraft:crossbow") && m.ai.aggressive {
        PiglinPose::CrossbowHold
    } else if m.ai.aggressive && m.extra.main_hand.is_some() && spear.is_none() {
        PiglinPose::MeleeWeapon
    } else {
        PiglinPose::Default
    };
    piglin_pose_holding(
        p,
        s,
        pose,
        (25.0, m.extra.using_item as f32),
        m.main_arm_left(),
        m.extra.passenger,
        spear,
    );
}
/// `PiglinArmPose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PiglinPose {
    Default,
    Dancing,
    MeleeWeapon,
    CrossbowHold,
    CrossbowCharge,
    AdmiringItem,
}
/// `PiglinModel.setupAnim`; `left` when the piglin's main arm is the left one
/// and `passenger` when it rides something (`HumanoidModel`'s riding pose).
pub fn piglin_pose(
    p: &mut Poses,
    s: &State,
    pose: PiglinPose,
    max_charge: f32,
    ticks: f32,
    left: bool,
    passenger: bool,
) {
    piglin_pose_holding(p, s, pose, (max_charge, ticks), left, passenger, None);
}
/// `piglin_pose` for a piglin whose main arm holds a spear (`spear`: the
/// spear and the ticks the arm has used it, 0 at rest).
pub fn piglin_pose_holding(
    p: &mut Poses,
    s: &State,
    pose: PiglinPose,
    (max_charge, ticks): (f32, f32),
    left: bool,
    passenger: bool,
    spear: Option<(crate::spear::Kinetic, f32)>,
) {
    use super::anim::{ArmPose, Hum, crossbow_charge, crossbow_hold, humanoid};
    let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
    if let Some((kinetic, ticks_using)) = spear {
        let side = usize::from(left);
        if left {
            h.left = ArmPose::Spear;
        } else {
            h.right = ArmPose::Spear;
        }
        h.spear[side] = Some(kinetic);
        h.spear_ticks[side] = ticks_using;
    }
    h.weapon_swing = pose == PiglinPose::MeleeWeapon;
    h.left_main = left;
    h.passenger = passenger;
    h.charge = (max_charge, ticks);
    humanoid(p, s, &h);
    ears(p, s);
    let (hy, hx) = (s.y_rot.to_radians(), s.x_rot.to_radians());
    let rad = |v: f64| v as f32;
    let deg = horizon_core::geometry::trig::DEG_TO_RAD;
    match pose {
        PiglinPose::Dancing => {
            let dance = s.age / 60.0;
            p.zrot(
                "right_ear",
                rad(std::f64::consts::PI / 6.0) + deg * (dance * 30.0).msin() * 10.0,
            );
            p.zrot(
                "left_ear",
                rad(-std::f64::consts::PI / 6.0) - deg * (dance * 30.0).mcos() * 10.0,
            );
            p.get("head")[0] += (dance * 10.0).msin();
            p.get("head")[1] += (dance * 40.0).msin() + 0.4;
            let z = deg * (70.0 + (dance * 40.0).mcos() * 10.0);
            p.zrot("right_arm", z);
            p.zrot("left_arm", -z);
            p.get("right_arm")[1] += (dance * 40.0).msin() * 0.5 - 0.5;
            p.get("left_arm")[1] += (dance * 40.0).msin() * 0.5 + 0.5;
            p.get("body")[1] += (dance * 40.0).msin() * 0.35;
        }
        // holdWeaponHigh
        PiglinPose::MeleeWeapon if s.swing == 0.0 => {
            p.xrot(if left { "left_arm" } else { "right_arm" }, -1.8)
        }
        PiglinPose::CrossbowHold => crossbow_hold(p, hx, hy, !left),
        PiglinPose::CrossbowCharge => crossbow_charge(p, max_charge, ticks, !left),
        PiglinPose::AdmiringItem => {
            p.xrot("head", 0.5);
            p.yrot("head", 0.0);
            if left {
                p.yrot("right_arm", -0.5);
                p.xrot("right_arm", -0.9);
            } else {
                p.yrot("left_arm", 0.5);
                p.xrot("left_arm", -0.9);
            }
        }
        _ => {}
    }
}
/// `Hoglin.handleEntityEvent` and `Zoglin`'s event 4 on a client: the
/// headbutt, which `aiStep` then counts down.
pub fn client_event(m: &mut Mob, event: u8) {
    if event == 4 {
        m.extra.anim_ticks = 10;
    }
}
/// The headbutt's countdown (`Hoglin.aiStep`, `Zoglin.aiStep`) on a client.
pub fn client_tick(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
}
/// `HoglinModel.setupAnim`.
pub fn hoglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    let (pos, sp) = (s.pos, s.speed);
    p.zrot("right_ear", -PI * 2.0 / 9.0 - sp * pos.msin());
    p.zrot("left_ear", PI * 2.0 / 9.0 + sp * pos.msin());
    p.yrot("head", s.y_rot.to_radians());
    let remaining = m.extra.anim_ticks as f32;
    let f = 1.0 - (10.0 - 2.0 * remaining).abs() / 10.0;
    p.xrot("head", 0.872_664_63 + (-PI / 9.0 - 0.872_664_63) * f);
    if s.baby {
        p.get("head")[1] += f * 2.5;
    }
    p.xrot("right_front_leg", pos.mcos() * 1.2 * sp);
    p.xrot("left_front_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("right_hind_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("left_hind_leg", pos.mcos() * 1.2 * sp);
}

#[cfg(test)]
mod brute_sound_tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    fn mob(species: Species) -> Mob {
        let mut kind = kind();
        Arc::make_mut(&mut kind).species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }
    fn sounds(events: &[MobEvent]) -> Vec<&'static str> {
        events
            .iter()
            .filter_map(|e| match e {
                MobEvent::Sound(sound, ..) => Some(*sound),
                _ => None,
            })
            .collect()
    }

    /// `PiglinBruteAi.updateActivity`: a brute with an attack target enters
    /// the fight activity and growls `entity.piglin_brute.angry`, losing the
    /// target is silent, and a piglin keeps `entity.piglin.angry`.
    #[test]
    fn a_brute_names_the_brute_growl() {
        let mut brute = mob(Species::PiglinBrute);
        brute.ai.target = Some(Who::Player);
        assert_eq!(activity(&brute), Activity::Fight);
        // The switch itself takes a level; it runs where the vanilla data is.
        with_test_world(&[], |w, _| {
            let mut events = Vec::new();
            update_activity(&mut brute, w, &mut events);
            assert_eq!(sounds(&events), ["entity.piglin_brute.angry"]);
            brute.ai.target = None;
            let mut events = Vec::new();
            update_activity(&mut brute, w, &mut events);
            assert!(sounds(&events).is_empty());
            let mut piglin = mob(Species::Piglin);
            piglin.ai.target = Some(Who::Player);
            let mut events = Vec::new();
            update_activity(&mut piglin, w, &mut events);
            assert_eq!(sounds(&events), ["entity.piglin.angry"]);
        });
    }

    /// `PiglinBruteAi.maybePlayActivitySound`: the level random is drawn on
    /// every tick, fighting or not, and a hit only sounds in a fight.
    #[test]
    fn the_growl_roll_draws_every_tick_and_only_sounds_in_a_fight() {
        let mut brute = mob(Species::PiglinBrute);
        let mut lr = LegacyRandom::new(7);
        let mut control = LegacyRandom::new(7);
        let mut events = Vec::new();
        let mut growls = 0;
        for tick in 0..2000 {
            let fight = tick % 2 == 0;
            brute.extra.piglin.activity = if fight {
                Activity::Fight
            } else {
                Activity::Idle
            };
            // The same draw on a stream of its own says when the roll hit.
            let hit = control.next_float() < 0.0125;
            maybe_play_brute_sound(&mut brute, &mut lr, &mut events);
            if hit && fight {
                growls += 1;
            }
            assert_eq!(sounds(&events).len(), growls, "tick {tick}");
        }
        assert!(growls > 0, "the roll never hit");
    }
}

#[cfg(test)]
mod crossbow_tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn piglin() -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:piglin".into();
        k.species = Species::Piglin;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// The spawn weapon is equipped after the goals are registered, so the
    /// crossbow goal must exist on a piglin that holds nothing yet, and the
    /// melee goal must be off once a crossbow is held.
    #[test]
    fn a_piglin_with_a_crossbow_shoots_instead_of_meleeing() {
        let mut m = piglin();
        assert!(format!("{:?}", m.ai.goals).contains("CrossbowAttack"));
        assert!(!super::super::spear_use::holds_non_melee_weapon(&m));
        m.extra.set_main_hand(Some("minecraft:crossbow".into()));
        assert!(super::super::spear_use::holds_non_melee_weapon(&m));
        m.extra.set_main_hand(Some("minecraft:golden_sword".into()));
        assert!(!super::super::spear_use::holds_non_melee_weapon(&m));
    }

    #[test]
    fn a_loved_item_in_sight_is_admired_until_too_far_or_too_long() {
        let mut b = Memories::default();
        // Seen: the memory is set at once, and not again while it is there.
        b.admire_seen(false, Some(25.0), true);
        assert_eq!(b.seen_admire, ADMIRE);
        b.seen_admire = 50;
        b.admire_seen(false, Some(25.0), true);
        assert_eq!(b.seen_admire, 50);
        // An item that is not loved starts nothing.
        let mut c = Memories::default();
        c.admire_seen(false, Some(25.0), false);
        assert_eq!(c.seen_admire, 0);
        // 9 blocks away or gone (`closerThan(9)`) ends it.
        b.admire_seen(false, Some(81.0), true);
        assert_eq!(b.seen_admire, 0);
        b.seen_admire = 50;
        b.admire_seen(false, None, true);
        assert_eq!(b.seen_admire, 0);
        // Admiring disabled by a player's blow: never starts.
        let mut d = Memories {
            admire_disabled: 3,
            ..Memories::default()
        };
        d.admire_seen(false, Some(1.0), true);
        assert_eq!(d.seen_admire, 0);
        // Tired of trying: 201 ticks near the item stop it and disable the walk
        // for 200 ticks, in which it does not start again.
        let mut e = Memories::default();
        e.admire_seen(false, Some(4.0), true);
        for _ in 0..203 {
            e.seen_admire = 50;
            e.admire_seen(false, Some(4.0), true);
        }
        assert_eq!((e.seen_admire, e.walk_off, e.reach_time), (0, 200, None));
        e.admire_seen(false, Some(4.0), true);
        assert_eq!(e.seen_admire, 0);
        // An item in the off hand leaves the memory alone.
        let mut f = Memories {
            seen_admire: 40,
            ..Memories::default()
        };
        f.admire_seen(true, None, false);
        assert_eq!(f.seen_admire, 40);
    }
}
