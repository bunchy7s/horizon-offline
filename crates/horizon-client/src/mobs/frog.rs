//! Frogs (`Frog`, `FrogAi`): the `Croak`, `ShootTongue` and long jump
//! behaviours as goals, the poses they set and the client's animation states.
//! AnimState states: jump, croak, tongue, swim idle.
//!
//! The brain's idle `RunOne` picks a croak three times as often as a stroll
//! (weights 3 and 1; it also holds a do-nothing option that only repeats
//! the pick), so a frog that is not walking croaks in three of four idle
//! picks: `Croak` takes those, and the frog's stroll then follows at once.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    goals::{
        Breed, Cx, Float, Goal, LOOK, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, Tempt,
        goal_clone,
    },
    sync,
};
use crate::blocks::Fluid;
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use std::fmt::Debug;

const JUMP: usize = 0;
const CROAK: usize = 1;
const TONGUE: usize = 2;
const SWIM_IDLE: usize = 3;

/// `Croak.CROAK_TICKS`.
const CROAK_TICKS: i32 = 60;
/// `ShootTongue`'s duration and eating distance.
const TONGUE_TIME_OUT: i32 = 100;
const EATING_DISTANCE: f64 = 1.75;
/// `FrogAttackablesSensor.TARGET_DETECTION_DISTANCE` and the sensor's scan
/// rate.
const DETECTION_DISTANCE: f64 = 10.0;
const SCAN_RATE: i32 = 20;

/// `FrogAi.TIME_BETWEEN_LONG_JUMPS` and the brain memory
/// `LONG_JUMP_COOLDOWN_TICKS` (saved with the others, as `finalize` sets it).
const TIME_BETWEEN_LONG_JUMPS: (i32, i32) = (100, 140);
/// `GoatAi.TIME_BETWEEN_LONG_JUMPS`.
const GOAT_TIME_BETWEEN_LONG_JUMPS: (i32, i32) = (600, 1200);
const LONG_JUMP_COOLDOWN: &str = "long_jump_cooling_down";
/// `FrogAi`'s `LongJumpToPreferredBlock`: `MAX_LONG_JUMP_WIDTH`,
/// `MAX_LONG_JUMP_HEIGHT`, `MAX_JUMP_VELOCITY_MULTIPLIER` and the chance to
/// look for a lily pad or big dripleaf first.
const JUMP_WIDTH: i32 = 4;
const JUMP_HEIGHT: i32 = 2;
/// The goat's `MAX_JUMP_VELOCITY_MULTIPLIER` is the same number.
const JUMP_VELOCITY_MULTIPLIER: f32 = 3.571_428_8;
const PREFERRED_BLOCKS_CHANCE: f32 = 0.5;
/// `GoatAi`'s `LongJumpToRandomPos`: `MAX_LONG_JUMP_WIDTH` and
/// `MAX_LONG_JUMP_HEIGHT`.
const GOAT_JUMP_WIDTH: i32 = 5;
const GOAT_JUMP_HEIGHT: i32 = 5;
/// `LongJumpToRandomPos.ALLOWED_ANGLES`, `FIND_JUMP_TRIES` and
/// `PREPARE_JUMP_DURATION`.
const ALLOWED_ANGLES: [i32; 4] = [65, 70, 75, 80];
const FIND_JUMP_TRIES: i32 = 20;
const PREPARE_JUMP_DURATION: i32 = 40;
/// `LongJumpMidJump`'s duration (`TIME_OUT_DURATION`).
const MID_JUMP_DURATION: i32 = 100;

/// `ShootTongue.State`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    MoveToTarget,
    CatchAnimation,
    EatAnimation,
    #[default]
    Done,
}

/// The frog's brain memories and `DATA_TONGUE_TARGET_ID`.
#[derive(Clone, Debug, Default)]
pub struct State {
    croak: i32,
    /// The land beside water `LaySpawn` walks to (its `WALK_TARGET`).
    lay_target: Option<[i32; 3]>,
    /// Ticks to the attackables sensor's next scan, and `NEAREST_ATTACKABLE`.
    scan: i32,
    nearest: Option<u32>,
    /// `UNREACHABLE_TONGUE_TARGETS`: the mob and the ticks it stays in mind.
    unreachable: Vec<(u32, i32)>,
    /// `ATTACK_TARGET` while `ShootTongue` runs, and the synched tongue target.
    target: Option<u32>,
    pub tongue_target: Option<u32>,
    phase: Phase,
    eat_timer: i32,
    path_counter: i32,
    time: i32,
    /// `LivingEntity.discardFriction`, set through the long jump.
    pub discard_friction: bool,
}

pub fn register(m: &mut Mob) {
    let food = m.kind.food.clone();
    // `Frog()`: SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true). The
    // frog's movement speed is 1.0, so the controller's 0.1 outside-water
    // speed is what keeps a walking frog at vanilla's pace; the plain ground
    // controller walks ten times too fast (#2213).
    m.ai.mov.kind = super::control::MoveKind::SmoothSwimming {
        max_turn_x: 85.0,
        max_turn_y: 10.0,
        in_water: 0.02,
        outside: 0.1,
        gravity: true,
    };
    // `Frog.FrogPathNavigation` (`AmphibiousPathNavigation`, and
    // `prefersShallowSwimming` is the frog's own override): water deeper than
    // ten blocks under sea level costs more, so a frog paths the shallows.
    m.ai.nav.kind = super::nav::NavKind::Amphibious {
        prefers_shallow: true,
    };
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, Panic::new(1.25));
    // `FrogAi.initIdleActivity`: `AnimalMakeLove` first, then `FollowTemptation(1.25)`.
    g.add(2, Breed::new(1.0));
    g.add(3, Unless(is_pregnant, Tempt::new(1.25, food, false)));
    g.add(2, ShootTongue);
    g.add(2, LaySpawn::default());
    g.add(3, LongJump::default());
    g.add(4, Croak);
    g.add(
        5,
        Unless(laying_trip, Stroll::avoiding_water(1.0).interval(1)),
    );
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
}

/// `Entity.setPose` and `Frog.onSyncedDataUpdated(DATA_POSE)`.
pub fn set_pose(m: &mut Mob, id: u8) {
    let old = std::mem::replace(&mut m.extra.pose, id);
    if old != id {
        pose_changed(m);
    }
}
/// The animation states of the pose the frog has taken.
pub fn pose_changed(m: &mut Mob) {
    let age = m.age as f32;
    let pose = m.extra.pose;
    let states = &mut m.anim.states;
    for (state, on) in [
        (JUMP, pose == sync::LONG_JUMPING),
        (CROAK, pose == sync::CROAKING),
        (TONGUE, pose == sync::USING_TONGUE),
    ] {
        if on {
            states[state].start(age);
        } else {
            states[state].stop();
        }
    }
}
/// `Frog.tick` on a client: the idle swim while it floats still. The server
/// runs it too, for a window that shows its own mobs.
pub fn tick(m: &mut Mob) {
    let age = m.age as f32;
    let still = m.fluids.in_water && m.limb_speed <= 1.0e-5;
    m.anim.states[SWIM_IDLE].animate_when(still, age);
}

/// `Frog.canEat`: a slime or magma cube of size 1.
/// `Frog.updateWalkAnimation`: the frog's short steps walk the legs 25 times
/// the distance (the base is 4), and not at all while the jump plays.
pub fn walk_target(m: &Mob, distance: f32) -> f32 {
    if m.anim.states[JUMP].started.is_some() {
        0.0
    } else {
        (distance * 25.0).min(1.0)
    }
}

fn can_eat(species: Species, size: i32) -> bool {
    matches!(species, Species::Slime | Species::MagmaCube) && size == 1
}

/// `FrogAttackablesSensor` (every 20 ticks) and the memory timers.
pub fn server_ai_step(m: &mut Mob, w: &MobWorld) {
    count_down_cooldown(m);
    let f = &mut m.extra.frog;
    f.unreachable.retain_mut(|(_, ttl)| {
        *ttl -= 1;
        *ttl > 0
    });
    f.scan -= 1;
    if f.scan > 0 {
        return;
    }
    f.scan = SCAN_RATE;
    let unreachable: Vec<u32> = f.unreachable.iter().map(|u| u.0).collect();
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    let nearest = w
        .nearby(m.position, DETECTION_DISTANCE, |s| {
            can_eat(s.species, s.size)
                && !unreachable.contains(&s.id)
                && w.line_of_sight(eye, Vec3::new(s.position.x, s.eye_y, s.position.z))
        })
        .find(|s| s.position.distance_squared(m.position) < DETECTION_DISTANCE * DETECTION_DISTANCE)
        .map(|s| s.id);
    m.extra.frog.nearest = nearest;
}

/// `Croak`: the pose sync::CROAKING for 60 ticks (the croak is the frog's ambient
/// sound).
#[derive(Clone, Debug)]
struct Croak;
impl Goal for Croak {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        // checkExtraStartConditions (standing), no walk target, and not in
        // a liquid (start does nothing there).
        m.extra.pose == sync::STANDING
            && m.on_ground
            && !m.fluids.in_water
            && !m.fluids.in_lava
            && m.ai.nav.is_done()
            && m.random.next_int_bound(4).unwrap_or(0) != 0
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.frog.croak < CROAK_TICKS
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.frog.croak = 0;
        set_pose(m, sync::CROAKING);
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_pose(m, sync::STANDING);
    }
    fn tick(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.frog.croak += 1;
    }
    fn every_tick(&self) -> bool {
        true
    }
}

/// `IS_PREGNANT`, set by `Frog.spawnChildFromBreeding` and erased once the
/// frog has laid its frogspawn.
const PREGNANT: &str = "is_pregnant";
/// `LaySpawn`'s walk target is set.
fn laying_trip(m: &Mob) -> bool {
    m.extra.frog.lay_target.is_some()
}
pub fn is_pregnant(m: &Mob) -> bool {
    m.extra.memories.iter().any(|x| x.0 == PREGNANT)
}
pub fn set_pregnant(m: &mut Mob, on: bool) {
    let at = m.extra.memories.iter().position(|x| x.0 == PREGNANT);
    match (at, on) {
        (None, true) => m.extra.memories.push((PREGNANT, 1, i64::MAX)),
        (Some(i), false) => {
            m.extra.memories.remove(i);
        }
        _ => {}
    }
}

/// A goal that waits while `blocked` holds: a pregnant frog is in the lay
/// spawn activity, which has no `FollowTemptation`, and a frog with a walk
/// target (`MoveToTargetSink`) does not run the stroll `RunOne`.
#[derive(Clone)]
struct Unless<G>(fn(&Mob) -> bool, G);
impl<G: Debug> Debug for Unless<G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.1.fmt(f)
    }
}
impl<G: Goal + Clone + 'static> Goal for Unless<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.1.flags()
    }
    fn name(&self) -> &'static str {
        self.1.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !(self.0)(m) && self.1.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !(self.0)(m) && self.1.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.1.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.1.every_tick()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.1.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.1.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.1.tick(m, cx)
    }
}

/// `FrogAi.initLaySpawnActivity` for a pregnant frog: `TryFindLandNearLiquid(8, 1.0)`
/// looks for land beside water and walks there, and `TryLaySpawnOnFluidNearLand`
/// puts frogspawn on the water next to it. Both run every tick (the brain's
/// walk target is `lay_target` here); the wandering `RunOne` is the goals below it.
#[derive(Clone, Debug, Default)]
struct LaySpawn {
    /// `nextOkStartTime`: the search runs again once it has passed; every tick the
    /// behaviour is tried before that pushes it 40 ticks on.
    next_ok: i64,
}
impl LaySpawn {
    fn is_air(w: &MobWorld, p: [i32; 3]) -> bool {
        w.facts.is_air(w.world.block(p[0], p[1], p[2]))
    }
    fn empty(w: &MobWorld, p: [i32; 3]) -> bool {
        w.collision_bounds(p).is_none()
    }
    /// `TryFindLandNearLiquid`'s `findFirst` test.
    fn land_near_water(w: &MobWorld, p: [i32; 3]) -> bool {
        if !Self::empty(w, p) || Self::empty(w, [p[0], p[1] - 1, p[2]]) {
            return false;
        }
        HORIZONTAL.iter().any(|d| {
            let n = [p[0] + d[0], p[1], p[2] + d[1]];
            Self::is_air(w, n) && w.fluid(n[0], n[1] - 1, n[2]) == Fluid::Water
        })
    }
    fn find(&mut self, m: &mut Mob, cx: &mut Cx) {
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        if cx.w.fluid(at[0], at[1], at[2]) == Fluid::Water {
            return;
        }
        let now = cx.w.game_time;
        let search = now >= self.next_ok;
        self.next_ok = now + 40;
        if !search {
            return;
        }
        let found = manhattan_box(at, 8)
            .into_iter()
            .find(|p| (p[0] != at[0] || p[2] != at[2]) && Self::land_near_water(cx.w, *p));
        if let Some(p) = found {
            let to = [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            ];
            m.look_at([to[0], to[1] + 0.5, to[2]]);
            // `MoveToTargetSink`: `createPath(target block, 0)`.
            let path = m.create_path_to(cx.w, p, 0);
            m.nav_move_to_path(cx.w, path, 1.0);
            m.extra.frog.lay_target = Some(p);
        }
    }
    fn lay(m: &mut Mob, cx: &mut Cx) {
        if m.fluids.in_water || !m.on_ground {
            return;
        }
        let below = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32 - 1,
            m.position.z.floor() as i32,
        ];
        for d in HORIZONTAL {
            let rel = [below[0] + d[0], below[1], below[2] + d[1]];
            // `getFaceShape(UP).isEmpty()`: nothing reaches the top of the block.
            let open_top =
                cx.w.collision_bounds(rel)
                    .is_none_or(|b| b.max_position().y < 1.0);
            let supports = cx.w.fluid(rel[0], rel[1], rel[2]) == Fluid::Water
                || cx.w.has_tag(
                    "minecraft:supports_frogspawn",
                    cx.w.block_name(rel[0], rel[1], rel[2]),
                );
            let spawn = [rel[0], rel[1] + 1, rel[2]];
            if open_top && supports && Self::is_air(cx.w, spawn) {
                if let Some(state) = cx.w.registry.default_state("minecraft:frogspawn") {
                    cx.events.push(MobEvent::SetBlock { pos: spawn, state });
                }
                cx.events.push(MobEvent::Sound(
                    "entity.frog.lay_spawn",
                    [m.position.x, m.position.y, m.position.z],
                    1.0,
                    1.0,
                ));
                set_pregnant(m, false);
                return;
            }
        }
    }
}
impl LaySpawn {
    /// The brain's turn: `TryFindLandNearLiquid`, then `TryLaySpawnOnFluidNearLand`.
    /// They run ahead of the wandering `RunOne`, so the selector runs them where it
    /// asks first (`can_use` and `can_continue`), before the stroll can start.
    fn step(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !is_pregnant(m) {
            return false;
        }
        // `MoveToTargetSink` erases the walk target once the trip is over.
        if m.extra.frog.lay_target.is_some() && m.ai.nav.is_done() {
            m.extra.frog.lay_target = None;
        }
        if m.extra.frog.lay_target.is_none() && m.ai.nav.is_done() {
            self.find(m, cx);
        }
        if m.extra.frog.lay_target.is_some() {
            Self::lay(m, cx);
        }
        is_pregnant(m)
    }
}
impl Goal for LaySpawn {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.step(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.step(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.frog.lay_target = None;
    }
}
/// `Direction.Plane.HORIZONTAL` as x and z steps: north, east, south, west.
const HORIZONTAL: [[i32; 2]; 4] = [[0, -1], [1, 0], [0, 1], [-1, 0]];
/// `BlockPos.withinManhattan`: nearest first, then x, y, and +z before -z.
fn manhattan_box(o: [i32; 3], range: i32) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for depth in 0..=range * 3 {
        let max_x = range.min(depth);
        for x in -max_x..=max_x {
            let max_y = range.min(depth - x.abs());
            for y in -max_y..=max_y {
                let z = depth - x.abs() - y.abs();
                if z <= range {
                    out.push([o[0] + x, o[1] + y, o[2] + z]);
                    if z != 0 {
                        out.push([o[0] + x, o[1] + y, o[2] - z]);
                    }
                }
            }
        }
    }
    out
}

/// `ShootTongue` with the tongue activity's `StartAttacking` and
/// `StopAttackingIfTargetInvalid`: walk up to a small slime or magma cube,
/// shoot the tongue, pull it in and eat it.
#[derive(Clone, Debug)]
struct ShootTongue;
impl Goal for ShootTongue {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(id) = m.extra.frog.nearest else {
            return false;
        };
        // The start conditions: no walk target, no panic, not croaking.
        if m.ai.panicking || m.extra.pose == sync::CROAKING || !m.ai.nav.is_done() {
            return false;
        }
        let Some(t) = cx.w.seen(id).filter(|t| t.alive) else {
            m.extra.frog.nearest = None;
            return false;
        };
        // canPathfindToTarget: a path that ends at the target.
        let at = [t.position.x, t.position.y, t.position.z];
        let ends = m
            .create_path_to_entity(cx.w, at, 0)
            .is_some_and(|p| f64::from(p.dist_to_target) < EATING_DISTANCE);
        if !ends {
            // The target is forgotten and not taken for 100 ticks.
            let f = &mut m.extra.frog;
            f.nearest = None;
            if !f.unreachable.iter().any(|u| u.0 == id) {
                if f.unreachable.len() == 5 {
                    f.unreachable.remove(0);
                }
                f.unreachable.push((id, 100));
            }
            return false;
        }
        m.extra.frog.target = Some(id);
        true
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let f = &m.extra.frog;
        f.phase != Phase::Done
            && f.time <= TONGUE_TIME_OUT
            && !m.ai.panicking
            && f.target
                .is_some_and(|id| cx.w.seen(id).is_some_and(|t| t.alive))
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(t) = m.extra.frog.target.and_then(|id| cx.w.seen(id)) else {
            return;
        };
        let at = [t.position.x, t.position.y, t.position.z];
        let eye = [t.position.x, t.eye_y, t.position.z];
        m.look_at(eye);
        m.extra.frog.tongue_target = m.extra.frog.target;
        m.nav_move_to_entity(cx.w, at, 0, 2.0);
        let f = &mut m.extra.frog;
        f.path_counter = 10;
        f.time = 0;
        f.phase = Phase::MoveToTarget;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        let f = &mut m.extra.frog;
        f.target = None;
        f.tongue_target = None;
        f.phase = Phase::Done;
        set_pose(m, sync::STANDING);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(id) = m.extra.frog.target else {
            return;
        };
        let Some(t) = cx.w.seen(id) else {
            return;
        };
        let (at, pos) = ([t.position.x, t.position.y, t.position.z], t.position);
        m.look_at([t.position.x, t.eye_y, t.position.z]);
        m.extra.frog.tongue_target = Some(id);
        m.extra.frog.time += 1;
        let here = [m.position.x, m.position.y, m.position.z];
        match m.extra.frog.phase {
            Phase::MoveToTarget => {
                if pos.distance_squared(m.position) < EATING_DISTANCE * EATING_DISTANCE {
                    cx.events
                        .push(MobEvent::Sound("entity.frog.tongue", here, 2.0, 1.0));
                    set_pose(m, sync::USING_TONGUE);
                    // `target.setDeltaMovement(vectorTo(frog).normalize().scale(0.75))`: the
                    // target's own motion is replaced, and it is drawn in.
                    let pull = m.position.subtract(pos).normalize().scale(0.75);
                    cx.events.push(MobEvent::SetVelocity {
                        target: Who::Mob(id),
                        velocity: [pull.x, pull.y, pull.z],
                    });
                    let f = &mut m.extra.frog;
                    f.eat_timer = 0;
                    f.phase = Phase::CatchAnimation;
                } else if m.extra.frog.path_counter <= 0 {
                    m.nav_move_to_entity(cx.w, at, 0, 2.0);
                    m.extra.frog.path_counter = 10;
                } else {
                    m.extra.frog.path_counter -= 1;
                }
            }
            Phase::CatchAnimation => {
                let f = &mut m.extra.frog;
                f.eat_timer += 1;
                if f.eat_timer > 6 {
                    f.phase = Phase::EatAnimation;
                    // eatEntity.
                    cx.events
                        .push(MobEvent::Sound("entity.frog.eat", here, 2.0, 1.0));
                    super::ai::do_hurt_target(m, Who::Mob(id), cx);
                    // `if (!target.isAlive()) target.remove(KILLED)`: no death animation.
                    cx.events.push(MobEvent::RemoveKilled { target: id });
                }
            }
            Phase::EatAnimation => {
                let f = &mut m.extra.frog;
                if f.eat_timer >= 10 {
                    f.phase = Phase::Done;
                } else {
                    f.eat_timer += 1;
                }
            }
            Phase::Done => {}
        }
    }
}

/// `CountDownCooldownTicks.tick` of `LONG_JUMP_COOLDOWN_TICKS`: one less each
/// tick, erased the tick after it reaches 0 (the core activity of a frog and
/// a goat).
pub(super) fn count_down_cooldown(m: &mut Mob) {
    match jump_cooldown(m) {
        Some(t) if t > 0 => set_jump_cooldown(m, Some(t - 1)),
        Some(_) => set_jump_cooldown(m, None),
        None => {}
    }
}
/// `LONG_JUMP_COOLDOWN_TICKS`.
fn jump_cooldown(m: &Mob) -> Option<i64> {
    m.extra
        .memories
        .iter()
        .find(|x| x.0 == LONG_JUMP_COOLDOWN)
        .map(|x| x.1)
}
fn set_jump_cooldown(m: &mut Mob, ticks: Option<i64>) {
    let memories = &mut m.extra.memories;
    let at = memories.iter().position(|x| x.0 == LONG_JUMP_COOLDOWN);
    match (at, ticks) {
        (Some(i), Some(t)) => memories[i].1 = t,
        (Some(i), None) => {
            memories.remove(i);
        }
        (None, Some(t)) => memories.push((LONG_JUMP_COOLDOWN, t, i64::MAX)),
        (None, None) => {}
    }
}
/// `TIME_BETWEEN_LONG_JUMPS.sample` from the level's random, divided as the
/// caller asks.
fn sample_cooldown(m: &mut Mob, lr: &mut LegacyRandom, div: i32) {
    let (lo, hi) = if m.species() == Species::Goat {
        GOAT_TIME_BETWEEN_LONG_JUMPS
    } else {
        TIME_BETWEEN_LONG_JUMPS
    };
    let t = lr.next_int_between_inclusive(lo, hi).unwrap_or(lo) / div;
    set_jump_cooldown(m, Some(i64::from(t)));
}

/// The jump sound each activity hands to `LongJumpToRandomPos`: the frog's, or
/// the goat's, which has a screaming one.
fn jump_sound(m: &Mob) -> &'static str {
    match (m.species(), m.extra.screaming) {
        (Species::Goat, true) => "entity.goat.screaming.long_jump",
        (Species::Goat, false) => "entity.goat.long_jump",
        _ => "entity.frog.long_jump",
    }
}

/// `LivingEntity.setDiscardFriction`: the flag lives in the frog's or the
/// goat's state.
fn set_discard_friction(m: &mut Mob, on: bool) {
    if m.species() == Species::Goat {
        m.extra.goat.discard_friction = on;
    } else {
        m.extra.frog.discard_friction = on;
    }
}
/// `Entity.setPose` to or from a long jump: a goat's box changes with it.
fn set_jump_pose(m: &mut Mob, w: &MobWorld, id: u8) {
    if m.species() == Species::Goat {
        super::goat::set_pose(m, w, id);
    } else {
        set_pose(m, id);
    }
}

/// `FrogAi.isAcceptableLandingSpot`: no fluid in the target, below or above
/// it; a lily pad or big dripleaf (`frog_prefer_jump_to`) or a trapdoor; else
/// `LongJumpToRandomPos.defaultAcceptableLandingSpot`: a solid block below
/// and a path type the frog walks without a malus.
fn acceptable_landing(m: &Mob, w: &MobWorld, [x, y, z]: [i32; 3]) -> bool {
    use super::path::PathType;
    if (y - 1..=y + 1).any(|y| w.fluid(x, y, z) != crate::blocks::Fluid::None) {
        return false;
    }
    let preferred = |y| w.has_tag("minecraft:frog_prefer_jump_to", w.block_name(x, y, z));
    if preferred(y) || preferred(y - 1) {
        return true;
    }
    let here = w.path_type_static(x, y, z);
    if here == PathType::Trapdoor
        || w.facts.is_air(w.world.block(x, y, z))
            && w.path_type_static(x, y - 1, z) == PathType::Trapdoor
    {
        return true;
    }
    default_acceptable_landing(m, w, [x, y, z])
}
/// `LongJumpToRandomPos.defaultAcceptableLandingSpot`: a solid block below and
/// a path type the mob walks without a malus.
fn default_acceptable_landing(m: &Mob, w: &MobWorld, [x, y, z]: [i32; 3]) -> bool {
    w.facts.get(w.world.block(x, y - 1, z)).solid_render
        && m.malus(w.path_type_static(x, y, z)) == 0.0
}

/// `LongJumpToRandomPos.PossibleJump`: a block and its weight.
type Candidate = ([i32; 3], i32);

/// `FrogAi`'s long jump activity: `LongJumpToPreferredBlock` looks for a spot
/// it cannot walk to, faces it for 40 ticks and leaps; `LongJumpMidJump`
/// holds the pose until it lands. One goal here, like the brain activity
/// that keeps the idle croak and stroll from starting meanwhile.
#[derive(Clone, Debug, Default)]
pub(super) struct LongJump {
    candidates: Vec<Candidate>,
    not_preferred: Vec<Candidate>,
    wanting_preferred: bool,
    initial: Option<Vec3>,
    chosen: Option<Vec3>,
    /// The chosen spot, looked at while preparing.
    look: [f64; 3],
    tries: i32,
    /// Ticks since the jump was chosen.
    prepare: i32,
    /// `LONG_JUMP_MID_JUMP`, and the ticks `LongJumpMidJump` has run.
    mid_jump: bool,
    in_air: Option<i32>,
}
impl LongJump {
    /// `WeightedRandom.getRandomItem` with the level's random over the
    /// candidates, removed once drawn.
    fn draw(&mut self, lr: &mut LegacyRandom) -> Option<Candidate> {
        let total: i32 = self.candidates.iter().map(|c| c.1).sum();
        if total == 0 {
            return None;
        }
        let mut pick = lr.next_int_bound(total).unwrap_or(0);
        let i = self.candidates.iter().position(|c| {
            pick -= c.1;
            pick < 0
        })?;
        Some(self.candidates.remove(i))
    }
    /// `LongJumpToPreferredBlock.getJumpCandidate`: while it wants them, the
    /// first drawn spot over a preferred block, else the first one passed over.
    fn candidate(&mut self, w: &MobWorld, lr: &mut LegacyRandom) -> Option<Candidate> {
        if !self.wanting_preferred {
            return self.draw(lr);
        }
        while !self.candidates.is_empty() {
            if let Some(c) = self.draw(lr) {
                let [x, y, z] = c.0;
                if w.has_tag("minecraft:frog_prefer_jump_to", w.block_name(x, y - 1, z)) {
                    return Some(c);
                }
                self.not_preferred.push(c);
            }
        }
        (!self.not_preferred.is_empty()).then(|| self.not_preferred.remove(0))
    }
    /// `LongJumpToRandomPos.pickCandidate`: the first acceptable spot it can
    /// reach in an arc but not by an 8 block path.
    fn pick(&mut self, m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
        while !self.candidates.is_empty() {
            let Some((target, _)) = self.candidate(w, lr) else {
                continue;
            };
            let here = [m.position.x.floor() as i32, m.position.z.floor() as i32];
            // GoatAi's landing spot is LongJumpToRandomPos's default; the
            // frog's is FrogAi.isAcceptableLandingSpot.
            let acceptable = if m.species() == Species::Goat {
                default_acceptable_landing(m, w, target)
            } else {
                acceptable_landing(m, w, target)
            };
            if here == [target[0], target[2]] || !acceptable {
                continue;
            }
            let centre = Vec3::new(
                f64::from(target[0]) + 0.5,
                f64::from(target[1]) + 0.5,
                f64::from(target[2]) + 0.5,
            );
            let Some(v) = optimal_jump_vector(m, w, centre) else {
                continue;
            };
            self.look = [centre.x, centre.y, centre.z];
            let walkable = m
                .create_path_raw(w, &[target], 0, 8.0)
                .is_some_and(|p| p.reached);
            if !walkable {
                self.chosen = Some(v);
                self.prepare = 0;
                return;
            }
        }
    }
    /// `LongJumpMidJump.stop` (a goat lands with `entity.goat.step`, a frog
    /// with `entity.frog.step`), or the end of a search.
    fn end(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.mid_jump {
            if m.on_ground {
                // multiply(0.1F, 1.0, 0.1F): the float widened.
                let slow = f64::from(0.1_f32);
                m.velocity = m.velocity.multiply(Vec3::new(slow, 1.0, slow));
                let at = [m.position.x, m.position.y, m.position.z];
                let landing = if m.species() == Species::Goat {
                    "entity.goat.step"
                } else {
                    "entity.frog.step"
                };
                cx.events.push(MobEvent::Sound(landing, at, 2.0, 1.0));
            }
            set_jump_pose(m, cx.w, sync::STANDING);
            sample_cooldown(m, cx.lr, 1);
        }
        set_discard_friction(m, false);
        *self = Self::default();
    }
}
/// `LongJumpToRandomPos.start`'s candidates: every block of the box but the
/// mob's own, weighted by the squared distance, in `betweenClosed` order.
fn candidates([x, y, z]: [i32; 3], width: i32, height: i32) -> Vec<Candidate> {
    let mut out = Vec::new();
    for dz in -width..=width {
        for dy in -height..=height {
            for dx in -width..=width {
                if (dx, dy, dz) != (0, 0, 0) {
                    out.push(([x + dx, y + dy, z + dz], dx * dx + dy * dy + dz * dz));
                }
            }
        }
    }
    out
}
/// `LongJumpToRandomPos.calculateOptimalJumpVector`: the angles shuffled, the
/// first arc that reaches the target clear of blocks. `Collections.shuffle`
/// without a random draws from an unseeded one, not the mob's or the level's.
fn optimal_jump_vector(m: &Mob, w: &MobWorld, target: Vec3) -> Option<Vec3> {
    let mut random = crate::sound_engine::SoundEngine::unseeded();
    let mut angles = ALLOWED_ANGLES;
    for i in (2..=angles.len()).rev() {
        let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        angles.swap(i - 1, j);
    }
    let max = (m.attrs.jump_strength * f64::from(JUMP_VELOCITY_MULTIPLIER)) as f32;
    angles
        .iter()
        .find_map(|&a| super::breeze::jump_vector(m, target, max, a, Some(w)))
}
impl Goal for LongJump {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn interruptable(&self) -> bool {
        !self.mid_jump
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // The activity: no cooldown, and for a frog out of water (a frog in
        // water is in its swim activity; a goat has none, so its check below
        // sets the cooldown instead). FrogAi.initJumpActivity also wants
        // TEMPTING_PLAYER and BREED_TARGET absent, and the LAY_SPAWN activity
        // (IS_PREGNANT) goes before LONG_JUMP. The tempting player and the
        // breed target are not gated: Tempt and Breed hold MOVE at a better priority.
        if jump_cooldown(m).is_some()
            || m.species() == Species::Frog && is_pregnant(m)
            || m.species() != Species::Goat && m.fluids.in_water
        {
            return false;
        }
        // GoatAi.initLongJumpActivity's other memories, and the activity
        // order of GoatAi.updateActivity.
        if m.species() == Species::Goat && !super::goat::long_jump_activity(m, cx.w) {
            return false;
        }
        // checkExtraStartConditions: on the ground, not in water or lava or
        // on honey.
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let can = m.on_ground
            && !m.fluids.in_water
            && !m.fluids.in_lava
            && cx.w.block_name(x, y, z) != "minecraft:honey_block";
        if !can {
            sample_cooldown(m, cx.lr, 2);
        }
        can
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        // LongJumpToRandomPos.start.
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let (width, height) = if m.species() == Species::Goat {
            (GOAT_JUMP_WIDTH, GOAT_JUMP_HEIGHT)
        } else {
            (JUMP_WIDTH, JUMP_HEIGHT)
        };
        *self = Self {
            candidates: candidates([x, y, z], width, height),
            tries: FIND_JUMP_TRIES,
            initial: Some(m.position),
            ..Self::default()
        };
        // LongJumpToPreferredBlock.start: the frog's; a goat's activity is a
        // plain LongJumpToRandomPos, which draws nothing from the mob's random.
        if m.species() != Species::Goat {
            self.wanting_preferred = m.random.next_float() < PREFERRED_BLOCKS_CHANCE;
        }
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.mid_jump {
            // LongJumpMidJump starts the tick after the leap and runs until
            // the mob is on the ground again, for 100 ticks at most; water
            // switches a frog's brain to the swim activity, which stops it
            // too (a goat has none).
            match self.in_air {
                None => {
                    self.in_air = Some(0);
                    set_discard_friction(m, true);
                    set_jump_pose(m, cx.w, sync::LONG_JUMPING);
                }
                Some(t) => self.in_air = Some(t + 1),
            }
            return !m.on_ground
                && (m.species() == Species::Goat || !m.fluids.in_water)
                && self.in_air.is_some_and(|t| t <= MID_JUMP_DURATION);
        }
        let valid = self.initial == Some(m.position)
            && self.tries > 0
            && !m.fluids.in_water
            && !(self.chosen.is_none() && self.candidates.is_empty());
        if !valid {
            sample_cooldown(m, cx.lr, 2);
        }
        valid
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.end(m, cx);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if self.mid_jump {
            return;
        }
        let Some(jump) = self.chosen else {
            self.tries -= 1;
            self.pick(m, cx.w, cx.lr);
            return;
        };
        m.look_at(self.look);
        self.prepare += 1;
        if self.prepare < PREPARE_JUMP_DURATION {
            return;
        }
        // The leap: the body's facing, no friction, the arc's velocity
        // lengthened by jump boost.
        m.yaw = m.body_yaw;
        set_discard_friction(m, true);
        let len = jump.length();
        let boost = m
            .extra
            .effects
            .iter()
            .find(|e| e.0 == "minecraft:jump_boost")
            .map_or(0.0, |e| f64::from(0.1_f32 * (e.2 + 1) as f32));
        m.velocity = jump.scale((len + boost) / len);
        self.mid_jump = true;
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events.push(MobEvent::Sound(jump_sound(m), at, 1.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    fn frog() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = std::sync::Arc::make_mut(&mut kind);
        k.id = "minecraft:frog".into();
        k.species = Species::Frog;
        Mob::new(1, kind, Vec3::new(0.5, 0.0, 0.5), 0.0)
    }

    /// `Frog.updateWalkAnimation`: target `min(distance * 25, 1)`, 0 while
    /// the jump animation is started, eased by 0.4 (`WalkAnimationState.update`).
    #[test]
    fn a_frog_walks_its_legs_at_the_vanilla_rate() {
        let mut m = frog();
        m.previous = m.position;
        m.position.x += 0.02;
        m.update_walk_animation();
        assert!((m.limb_speed - 0.5 * 0.4).abs() < 1e-6, "{}", m.limb_speed);
        assert!((m.limb_position - m.limb_speed).abs() < 1e-6);

        let mut m = frog();
        m.previous = m.position;
        m.position.x += 0.1;
        m.update_walk_animation();
        assert!((m.limb_speed - 0.4).abs() < 1e-6, "{}", m.limb_speed);

        let mut m = frog();
        m.anim.states[JUMP].start(0.0);
        m.previous = m.position;
        m.position.x += 0.02;
        m.update_walk_animation();
        assert_eq!(m.limb_speed, 0.0);
    }

    /// `LongJumpToRandomPos.start`: the 9 x 5 x 9 box without the frog's own
    /// block, x fastest, then y, then z, each weighted by its squared distance.
    #[test]
    fn the_candidates_cover_the_jump_box_in_vanilla_order() {
        let c = candidates([10, 64, -3], JUMP_WIDTH, JUMP_HEIGHT);
        assert_eq!(c.len(), 9 * 5 * 9 - 1);
        assert_eq!(c[0], ([6, 62, -7], 16 + 4 + 16));
        assert_eq!(c[1], ([7, 62, -7], 9 + 4 + 16));
        assert_eq!(c[9], ([6, 63, -7], 16 + 1 + 16));
        assert_eq!(c[45], ([6, 62, -6], 16 + 4 + 9));
        assert!(!c.iter().any(|(p, _)| *p == [10, 64, -3]));
        assert_eq!(c.last().unwrap().0, [14, 66, 1]);
    }

    /// `WeightedRandom.getRandomItem`: `nextInt(total)` walked down the
    /// list, and the drawn candidate is removed.
    #[test]
    fn a_draw_takes_the_weighted_candidate_out() {
        let mut j = LongJump {
            candidates: vec![([0, 0, 0], 1), ([1, 0, 0], 4), ([2, 0, 0], 9)],
            ..LongJump::default()
        };
        let mut r = LegacyRandom::new(7);
        let mut lr = r.clone();
        let pick = r.next_int_bound(14).unwrap();
        let want = if pick < 1 {
            0
        } else if pick < 5 {
            1
        } else {
            2
        };
        assert_eq!(j.draw(&mut lr).unwrap().0, [want, 0, 0]);
        assert_eq!(j.candidates.len(), 2);
        assert!(!j.candidates.iter().any(|c| c.0 == [want, 0, 0]));
    }

    /// `CountDownCooldownTicks`: the memory falls by one a tick and is gone
    /// the tick after it reached 0; the sample is 100..=140, halved after a
    /// failed search, drawn from the level's random and not the frog's.
    #[test]
    fn pregnancy_is_a_memory_that_the_laying_erases() {
        let mut m = frog();
        assert!(!is_pregnant(&m));
        set_pregnant(&mut m, true);
        set_pregnant(&mut m, true);
        assert!(is_pregnant(&m));
        assert_eq!(
            m.extra.memories.iter().filter(|x| x.0 == PREGNANT).count(),
            1
        );
        set_pregnant(&mut m, false);
        assert!(!is_pregnant(&m));
    }

    #[test]
    fn the_land_search_goes_out_by_manhattan_distance() {
        let all = manhattan_box([0, 0, 0], 8);
        assert_eq!(all[0], [0, 0, 0]);
        assert_eq!(all.len(), 17 * 17 * 17);
        // x, then y, and +z before -z, at distance 1.
        assert_eq!(
            all[1..7],
            [
                [-1, 0, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
                [0, 1, 0],
                [1, 0, 0]
            ]
        );
    }

    #[test]
    fn the_long_jump_cooldown_counts_down_and_is_erased() {
        let mut m = frog();
        let own = m.random.state();
        sample_cooldown(&mut m, &mut LegacyRandom::new(3), 2);
        let t = jump_cooldown(&m).unwrap();
        assert!((50..=70).contains(&t));
        assert_eq!(m.random.state(), own);
        set_jump_cooldown(&mut m, Some(1));
        set_jump_cooldown(&mut m, Some(0));
        assert_eq!(jump_cooldown(&m), Some(0));
        set_jump_cooldown(&mut m, None);
        assert_eq!(jump_cooldown(&m), None);
        assert!(!m.extra.memories.iter().any(|x| x.0 == LONG_JUMP_COOLDOWN));
    }
}
