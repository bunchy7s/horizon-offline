//! Turtles (`Turtle`): the egg a bred mother carries (`TurtleBreedGoal`), her
//! walk home (`TurtleGoHomeGoal`) and the egg she digs into the sand
//! (`TurtleLayEggGoal`). `extra.flag` is `hasEgg`, `extra.flag2` `isLayingEgg`,
//! `extra.counter` `layEggCounter`, `extra.home` `homePos`, `extra.counter2`
//! `goingHome` and `extra.point` `travelPos`. Its own `TurtleMoveControl`
//! sets the speed it walks and swims at, and `travel` swims with its own
//! `travelInWater`.
use super::{
    Mob, MobEvent, MobWorld,
    control::{EPSILON, Operation, rotlerp},
    goals::{
        Breed, Cx, Goal, JUMP, LookAtPlayer, MOVE, MoveToBlock, Panic, Stroll, Tempt, goal_clone,
        reduced,
    },
    nav::NavKind,
    path::PathType,
};
use crate::blocks::Fluid;
use horizon_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};

/// `Turtle.registerGoals`, and the path maluses and navigation `Turtle` and
/// `TurtlePathNavigation` give it.
pub fn register(m: &mut Mob) {
    m.set_malus(PathType::Water, 0.0);
    for t in [
        PathType::DoorIronClosed,
        PathType::DoorWoodClosed,
        PathType::DoorOpen,
    ] {
        m.set_malus(t, -1.0);
    }
    m.ai.nav.kind = NavKind::Amphibious {
        prefers_shallow: false,
    };
    let food = m.kind.food.clone();
    let g = &mut m.ai.goals;
    g.add(0, Panic::new(1.2).looking_for_water(7));
    g.add(1, Breed::new(1.0));
    g.add(1, LayEgg(MoveToBlock::new(1.0, 16, 1)));
    g.add(2, Tempt::new(1.1, food, false));
    g.add(
        3,
        GoToWater(MoveToBlock {
            vertical_start: -1,
            ..MoveToBlock::new(1.0, 24, 1)
        }),
    );
    g.add(4, GoHome::default());
    g.add(7, Travel::default());
    g.add(8, LookAtPlayer::new(8.0));
    g.add(9, RandomStroll(Stroll::new(1.0).interval(100)));
}
/// `Turtle.goingHome`.
fn going_home(m: &Mob) -> bool {
    m.extra.counter2 != 0
}
fn set_going_home(m: &mut Mob, on: bool) {
    m.extra.counter2 = i32::from(on);
}
/// `Turtle.getWalkTargetValue`; `light` is `getPathfindingCostFromLightLevels`.
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3], light: f32) -> f32 {
    if !going_home(m) && w.fluid(p[0], p[1], p[2]) == Fluid::Water
        || is_sand(w, [p[0], p[1] - 1, p[2]])
    {
        10.0
    } else {
        light
    }
}
/// `TurtlePathNavigation.isStableDestination` while travelling: water only.
/// `None` without a `travelPos`, the amphibious rule then.
pub fn stable_destination(m: &Mob, w: &MobWorld, p: [i32; 3]) -> Option<bool> {
    m.extra
        .point
        .map(|_| w.block_name(p[0], p[1], p[2]) == "minecraft:water")
}
/// `Turtle.hasEgg`.
pub fn has_egg(m: &Mob) -> bool {
    m.extra.flag
}
/// `Turtle.setLayingEgg`.
fn set_laying_egg(m: &mut Mob, on: bool) {
    m.extra.counter = i32::from(on);
    m.extra.flag2 = on;
}
/// `homePos.closerToCenterThan(position, distance)`. Spawning, hatching and
/// loading all set a home; one made any other way is at home where it stands.
fn close_to_home(m: &Mob, distance: f64) -> bool {
    let home = m.extra.home.unwrap_or([
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]);
    let d = [
        f64::from(home[0]) + 0.5 - m.position.x,
        f64::from(home[1]) + 0.5 - m.position.y,
        f64::from(home[2]) + 0.5 - m.position.z,
    ];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < distance * distance
}
fn block_of(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `TurtleEggBlock.isSand`.
fn is_sand(w: &MobWorld, p: [i32; 3]) -> bool {
    w.has_tag("minecraft:sand", w.block_name(p[0], p[1], p[2]))
}
/// `Turtle.TurtleMoveControl.updateSpeed`: slow in water away from home
/// (slower still as a baby), and halved every tick on land, never under 0.06.
fn update_speed(m: &mut Mob) {
    if m.fluids.in_water {
        m.velocity.y += 0.005;
        if !close_to_home(m, 16.0) {
            m.set_speed((m.ai.speed / 2.0).max(0.08));
        }
        if m.is_baby() {
            m.set_speed((m.ai.speed / 3.0).max(0.06));
        }
    } else if m.on_ground {
        m.set_speed((m.ai.speed / 2.0).max(0.06));
    }
}
/// `Turtle.TurtleMoveControl.tick`: ease the speed an eighth of the way to
/// the wanted one each tick (after `updateSpeed` cut it), face the wanted
/// point and lift towards it; no jumping, and the operation is never reset.
pub fn move_control(m: &mut Mob) {
    update_speed(m);
    if m.ai.mov.operation != Operation::MoveTo || m.ai.nav.is_done() {
        m.set_speed(0.0);
        return;
    }
    let [wx, wy, wz] = m.ai.mov.wanted;
    let (xd, yd, zd) = (wx - m.position.x, wy - m.position.y, wz - m.position.z);
    let dist = (xd * xd + yd * yd + zd * zd).sqrt();
    if dist < EPSILON {
        m.set_speed(0.0);
        return;
    }
    let y_rot = trig::atan2_degrees(zd, xd) - 90.0;
    m.yaw = rotlerp(m.yaw, y_rot, 90.0);
    m.body_yaw = m.yaw;
    let wanted = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
    // Mth.lerp(0.125F, getSpeed(), wanted).
    m.set_speed(m.ai.speed + 0.125 * (wanted - m.ai.speed));
    m.velocity.y += f64::from(m.ai.speed) * (yd / dist) * 0.1;
}
/// `Turtle.travelInWater`: swims at 0.1 move speed with 0.9 drag, and sinks
/// slowly unless it has a target or is going home and near its home. On
/// land the usual travel.
pub fn travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    if !m.fluids.in_water {
        return false;
    }
    m.move_relative(0.1, input);
    m.do_move(w, events);
    m.velocity = m.velocity.scale(0.9);
    if m.ai.target.is_none() && (!going_home(m) || !close_to_home(m, 20.0)) {
        m.velocity.y -= 0.005;
    }
    true
}
/// `Turtle.aiStep`: while digging, sand crumbs fly every fifth tick.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.alive() && m.extra.flag2 && m.extra.counter >= 1 && m.extra.counter % 5 == 0 {
        let at = block_of(m);
        let below = [at[0], at[1] - 1, at[2]];
        if is_sand(w, below) {
            events.push(MobEvent::BlockBreakEffect {
                pos: at,
                block: below,
            });
        }
    }
}

/// `Turtle.TurtleLayEggGoal`: a mother with an egg within 9 blocks of home
/// walks to sand with air above, digs for 200 ticks and lays it.
#[derive(Clone, Debug)]
struct LayEgg(MoveToBlock);
impl LayEgg {
    /// `isValidTarget`: sand with air above it.
    fn valid(w: &MobWorld, p: [i32; 3]) -> bool {
        w.facts.is_air(w.world.block(p[0], p[1] + 1, p[2])) && is_sand(w, p)
    }
}
impl Goal for LayEgg {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn name(&self) -> &'static str {
        "TurtleLayEggGoal"
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !(has_egg(m) && close_to_home(m, 9.0)) {
            return false;
        }
        // MoveToBlockGoal.canUse.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        self.0.next_start = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.find_nearest(m, cx.w, Self::valid)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(cx.w, Self::valid) && has_egg(m) && close_to_home(m, 9.0)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx.w);
        if m.fluids.in_water || !self.0.reached {
            return;
        }
        if m.extra.counter < 1 {
            set_laying_egg(m, true);
        } else if m.extra.counter > 200 {
            let at = [m.position.x, m.position.y, m.position.z];
            let pitch = 0.9 + m.random.next_float() * 0.2;
            cx.events
                .push(MobEvent::Sound("entity.turtle.lay_egg", at, 0.3, pitch));
            // Blocks.TURTLE_EGG with EGGS = nextInt(4) + 1 above the sand.
            let eggs = m.random.next_int_bound(4).unwrap_or(0) + 1;
            let registry = cx.w.registry;
            let state = registry
                .default_state("minecraft:turtle_egg")
                .and_then(|d| registry.state(d).cloned())
                .and_then(|mut s| {
                    s.properties.insert("eggs".into(), eggs.to_string());
                    registry.resolve(&s).ok()
                });
            let b = self.0.block;
            if let Some(state) = state {
                cx.events.push(MobEvent::SetBlock {
                    pos: [b[0], b[1] + 1, b[2]],
                    state,
                });
            }
            m.extra.flag = false;
            set_laying_egg(m, false);
            m.extra.in_love = 600;
        }
        if m.extra.flag2 {
            m.extra.counter += 1;
        }
    }
}

/// `Turtle.TurtleGoHomeGoal`: a mother with an egg heads home; any adult
/// that strayed 64 blocks away does now and then.
#[derive(Clone, Debug, Default)]
struct GoHome {
    stuck: bool,
    close_to_home_try_ticks: i32,
}
impl Goal for GoHome {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn name(&self) -> &'static str {
        "TurtleGoHomeGoal"
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        if m.is_baby() {
            false
        } else if has_egg(m) {
            true
        } else {
            m.random.next_int_bound(reduced(700)).unwrap_or(1) == 0 && !close_to_home(m, 64.0)
        }
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_going_home(m, true);
        self.stuck = false;
        self.close_to_home_try_ticks = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        set_going_home(m, false);
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !close_to_home(m, 7.0) && !self.stuck && self.close_to_home_try_ticks <= reduced(600)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let home = m.extra.home.unwrap_or(block_of(m));
        let close = close_to_home(m, 16.0);
        if close {
            self.close_to_home_try_ticks += 1;
        }
        if !m.ai.nav.is_done() {
            return;
        }
        let to = [
            f64::from(home[0]) + 0.5,
            f64::from(home[1]),
            f64::from(home[2]) + 0.5,
        ];
        let mut next = m
            .default_random_pos_towards(cx.w, 16, 3, to, std::f64::consts::PI / 10.0)
            .or_else(|| m.default_random_pos_towards(cx.w, 8, 7, to, std::f64::consts::FRAC_PI_2));
        if let Some(at) = next
            && !close
            && cx.w.block_name(
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ) != "minecraft:water"
        {
            next = m.default_random_pos_towards(cx.w, 16, 5, to, std::f64::consts::FRAC_PI_2);
        }
        match next {
            Some(at) => {
                m.nav_move_to(cx.w, at, 1.0);
            }
            None => self.stuck = true,
        }
    }
}

/// `Turtle.TurtleGoToWaterGoal`: a turtle on land walks to water within 24
/// blocks, unless it is going home or carries an egg.
#[derive(Clone, Debug)]
struct GoToWater(MoveToBlock);
impl GoToWater {
    fn valid(w: &MobWorld, p: [i32; 3]) -> bool {
        w.block_name(p[0], p[1], p[2]) == "minecraft:water"
    }
}
impl Goal for GoToWater {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn name(&self) -> &'static str {
        "TurtleGoToWaterGoal"
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let wanted = if m.is_baby() && !m.fluids.in_water {
            true
        } else {
            !going_home(m) && !m.fluids.in_water && !has_egg(m)
        };
        if !wanted {
            return false;
        }
        // MoveToBlockGoal.canUse.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        self.0.next_start = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.find_nearest(m, cx.w, Self::valid)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.fluids.in_water && self.0.try_ticks <= 1200 && Self::valid(cx.w, self.0.block)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        // MoveToBlockGoal.tick, with shouldRecalculatePath every 160 ticks.
        let b = self.0.block;
        let d = [
            f64::from(b[0]) + 0.5 - m.position.x,
            f64::from(b[1] + 1) + 0.5 - m.position.y,
            f64::from(b[2]) + 0.5 - m.position.z,
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= 1.0 {
            self.0.reached = false;
            self.0.try_ticks += 1;
            if self.0.try_ticks % 160 == 0 {
                let at = [
                    f64::from(b[0]) + 0.5,
                    f64::from(b[1] + 1),
                    f64::from(b[2]) + 0.5,
                ];
                m.nav_move_to(cx.w, at, self.0.speed);
            }
        } else {
            self.0.reached = true;
            self.0.try_ticks -= 1;
        }
    }
}

/// `Turtle.TurtleTravelGoal`: a turtle in water swims towards a spot up to
/// 512 blocks away, a leg at a time.
#[derive(Clone, Debug, Default)]
struct Travel {
    stuck: bool,
}
impl Goal for Travel {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn name(&self) -> &'static str {
        "TurtleTravelGoal"
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !going_home(m) && !has_egg(m) && m.fluids.in_water
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let xt = m.random.next_int_bound(1025).unwrap_or(0) - 512;
        let mut yt = m.random.next_int_bound(9).unwrap_or(0) - 4;
        let zt = m.random.next_int_bound(1025).unwrap_or(0) - 512;
        if f64::from(yt) + m.position.y > f64::from(cx.w.sea_level - 1) {
            yt = 0;
        }
        m.extra.point = Some([
            (f64::from(xt) + m.position.x).floor(),
            (f64::from(yt) + m.position.y).floor(),
            (f64::from(zt) + m.position.z).floor(),
        ]);
        self.stuck = false;
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !m.ai.nav.is_done() && !self.stuck && !going_home(m) && m.extra.in_love <= 0 && !has_egg(m)
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.point = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(travel) = m.extra.point else {
            self.stuck = true;
            return;
        };
        if !m.ai.nav.is_done() {
            return;
        }
        // Vec3.atBottomCenterOf(travelPos).
        let to = [travel[0] + 0.5, travel[1], travel[2] + 0.5];
        let tenth = f64::from((std::f64::consts::PI / 10.0) as f32);
        let half = f64::from(std::f32::consts::FRAC_PI_2);
        let mut next = m
            .default_random_pos_towards(cx.w, 16, 3, to, tenth)
            .or_else(|| m.default_random_pos_towards(cx.w, 8, 7, to, half));
        if let Some(n) = next {
            let (xc, zc) = (n[0].floor() as i32, n[2].floor() as i32);
            // Level.hasChunksAt(xc - 34, zc - 34, xc + 34, zc + 34).
            let loaded = ((xc - 34) >> 4..=(xc + 34) >> 4).all(|chunk_x| {
                ((zc - 34) >> 4..=(zc + 34) >> 4)
                    .all(|chunk_z| cx.w.world.chunks.contains_key(&(chunk_x, chunk_z)))
            });
            if !loaded {
                next = None;
            }
        }
        match next {
            Some(at) => {
                m.nav_move_to(cx.w, at, 1.0);
            }
            None => self.stuck = true,
        }
    }
}

/// `Turtle.TurtleRandomStrollGoal`: no stroll in water, going home or with an
/// egg.
#[derive(Clone, Debug)]
struct RandomStroll(Stroll);
impl Goal for RandomStroll {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        "TurtleRandomStrollGoal"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.fluids.in_water && !going_home(m) && !has_egg(m) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{
        Species,
        path::{Path, PathNode, PathType},
    };
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// A turtle (movement speed 0.25) strolling (`speedModifier` 1.0) to a
    /// point ten blocks ahead along a path it has not finished.
    fn strolling_turtle() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:turtle".into();
        k.species = Species::Turtle;
        k.speed = 0.25;
        k.step_height = 1.0;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.on_ground = true;
        let node = PathNode {
            x: 0,
            y: 0,
            z: 10,
            kind: PathType::Walkable,
        };
        m.ai.nav.path = Some(Path {
            nodes: vec![node],
            next: 0,
            target: [0, 0, 10],
            dist_to_target: 0.0,
            reached: true,
        });
        m.ai.mov.set_wanted(0.0, 0.0, 10.0, 1.0);
        m
    }
    fn speeds(m: &mut Mob, ticks: usize) -> Vec<f32> {
        (0..ticks)
            .map(|_| {
                move_control(m);
                assert_eq!(m.ai.zza, m.ai.speed, "setSpeed sets the forward input");
                m.ai.speed
            })
            .collect()
    }

    #[test]
    fn walks_on_land_at_a_third_of_its_movement_speed() {
        // TurtleMoveControl.tick on the ground: updateSpeed makes it
        // max(speed / 2, 0.06) = 0.06, then Mth.lerp(0.125F, 0.06, 0.25),
        // every tick. The default MoveControl walked it at the full 0.25, so
        // the walk animation (and TurtleModel's 8 * walkSpeed leg swing) ran
        // about nine times too fast (#1776).
        let mut m = strolling_turtle();
        let land = 0.06_f32 + 0.125 * (0.25 - 0.06_f32);
        assert_eq!(speeds(&mut m, 40), vec![land; 40]);
        // Facing the wanted point, the body with it; it never jumps.
        assert_eq!(m.body_yaw, m.yaw);
        assert!(crate::mobs::living::wrap(m.yaw).abs() < 1.0e-3);
        assert!(!m.ai.jump);
        assert_eq!(m.ai.mov.operation, Operation::MoveTo);
        assert_eq!(m.velocity.y, 0.0);
    }

    #[test]
    fn the_leg_animation_runs_at_vanilla_s_rate() {
        // LivingEntity.updateWalkAnimation turns the distance a tick into
        // min(distance * 4, 1), and TurtleModel.setupAnim swings the legs
        // by 8 * that speed, so the rate of the walk animation is the rate
        // of the walk. The turtle walks at its own pace, so the animation
        // runs at vanilla's rate instead of at the maximum (#1776).
        let mut m = strolling_turtle();
        let mut last = m.position;
        let mut distance = 0.0_f64;
        for _ in 0..40 {
            // Entity.tick keeps the position the tick started at.
            m.previous = last;
            // TurtleMoveControl.tick: the forward input.
            move_control(&mut m);
            // LivingEntity.travelInAir on grass: the friction influenced
            // speed is the input speed (grass' friction is 0.6, so it is
            // not above it), the move uses the motion as it is, and the
            // motion is then scaled by the friction (0.6) times the air
            // drag (0.91). The move itself needs a world, so the flat
            // ground is stepped here.
            m.move_relative(m.ai.speed, Vec3::new(0.0, 0.0, 1.0));
            m.position.x += m.velocity.x;
            m.position.z += m.velocity.z;
            let drag = f64::from(0.6_f32 * 0.91_f32);
            m.velocity.x *= drag;
            m.velocity.z *= drag;
            // LivingEntity.updateWalkAnimation.
            m.update_walk_animation();
            distance = (m.position.x - m.previous.x).hypot(m.position.z - m.previous.z);
            last = m.position;
        }
        // The ground speed: the input (0.06 + 0.125 * (0.25 - 0.06) =
        // 0.08375) over one minus the drag (0.546) = 0.18447 blocks a tick.
        assert!((distance - 0.083_75 / (1.0 - 0.546)).abs() < 1.0e-6);
        // min(distance * 4, 1) for that pace: 0.73789. The generic
        // MoveControl walked the turtle at the full 0.25, which put it at
        // 0.55066 blocks a tick and saturated the walk animation at 1.0.
        assert!((m.limb_speed - 0.737_885_5).abs() < 1.0e-6);
    }

    #[test]
    fn water_speed_is_cut_away_from_home_and_for_babies() {
        // updateSpeed in water near home: no cut, the lerp climbs towards
        // 0.25, and it lifts the turtle 0.005 a tick. (Only the control's
        // speed: Turtle.travelInWater is not ported.)
        let mut m = strolling_turtle();
        m.on_ground = false;
        m.fluids.in_water = true;
        let s = speeds(&mut m, 60);
        assert_eq!(s[0], 0.125 * 0.25);
        assert!(s.windows(2).all(|w| w[1] > w[0]) && s[59] > 0.24 && s[59] < 0.25);
        assert!((m.velocity.y - 60.0 * 0.005).abs() < 1.0e-9);
        // More than 16 blocks from home: max(speed / 2, 0.08) first.
        let mut m = strolling_turtle();
        m.on_ground = false;
        m.fluids.in_water = true;
        m.extra.home = Some([0, 0, 40]);
        let far = 0.08_f32 + 0.125 * (0.25 - 0.08_f32);
        assert_eq!(speeds(&mut m, 20), vec![far; 20]);
        // A baby: max(speed / 3, 0.06).
        let mut m = strolling_turtle();
        m.on_ground = false;
        m.fluids.in_water = true;
        m.extra.age = -24000;
        let young = 0.06_f32 + 0.125 * (0.25 - 0.06_f32);
        assert_eq!(speeds(&mut m, 20), vec![young; 20]);
    }

    #[test]
    fn stops_when_the_path_is_done_or_the_point_reached() {
        let mut m = strolling_turtle();
        m.ai.nav.stop();
        assert_eq!(speeds(&mut m, 1), vec![0.0]);
        let mut m = strolling_turtle();
        m.ai.mov.set_wanted(0.0, 0.0, 0.0, 1.0);
        assert_eq!(speeds(&mut m, 1), vec![0.0]);
        // updateSpeed runs before the early return: a stopped turtle in
        // water is still lifted 0.005, path done or not moving at all.
        let mut m = strolling_turtle();
        m.fluids.in_water = true;
        m.ai.nav.stop();
        assert_eq!(speeds(&mut m, 1), vec![0.0]);
        assert_eq!(m.velocity.y, 0.005);
        let mut m = strolling_turtle();
        m.fluids.in_water = true;
        m.ai.mov.operation = Operation::Wait;
        assert_eq!(speeds(&mut m, 1), vec![0.0]);
        assert_eq!(m.velocity.y, 0.005);
    }

    #[test]
    fn lifts_towards_a_point_above_by_the_slope_of_the_way_there() {
        // setDeltaMovement(... + getSpeed() * (dy / dist) * 0.1): two up,
        // three across and ten ahead.
        let mut m = strolling_turtle();
        m.ai.mov.set_wanted(3.0, 2.0, 10.0, 1.0);
        let speed = speeds(&mut m, 1)[0];
        let dist = (3.0_f64 * 3.0 + 2.0 * 2.0 + 10.0 * 10.0).sqrt();
        assert_eq!(m.velocity.y, f64::from(speed) * (2.0 / dist) * 0.1);
        assert!(m.velocity.y > 0.0);
    }

    #[test]
    fn turns_at_most_ninety_degrees_a_tick_and_wraps_into_a_turn() {
        // rotlerp(yRot, target, 90): from 150 towards about -16.7 it turns
        // the full 90 to 60, then reaches the target, kept in [0, 360] as
        // about 343.3; the body follows the yaw each tick.
        let mut m = strolling_turtle();
        m.ai.mov.set_wanted(3.0, 0.0, 10.0, 1.0);
        m.yaw = 150.0;
        m.body_yaw = 150.0;
        let target = trig::atan2_degrees(10.0, 3.0) - 90.0;
        assert!((-17.0..-16.0).contains(&target));
        move_control(&mut m);
        assert_eq!((m.yaw, m.body_yaw), (60.0, 60.0));
        move_control(&mut m);
        assert_eq!(m.yaw, 60.0 + (target - 60.0) + 360.0);
        assert_eq!(m.body_yaw, m.yaw);
    }
}
