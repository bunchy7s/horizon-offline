//! Armadillos (`Armadillo`, `ArmadilloAi`): a player who sprints or rides
//! nearby, an undead mob or the last attacker scares one into a ball (IDLE,
//! ROLLING, SCARED, UNROLLING); it peeks out now and then, and a ball takes
//! less damage. The brain's `ARMADILLO_SCARE_DETECTED` sensor, its
//! `DANGER_DETECTED_RECENTLY` memory and the `ArmadilloBallUp` behaviour are
//! kept in `State`; the walking behaviours are the goals of `register`.
//! AnimState states: roll out, roll up, peek.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    goals::{
        Breed, Cx, Float, Goal, LOOK, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, Tempt,
        goal_clone,
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub const IDLE: u8 = 0;
pub const ROLLING: u8 = 1;
pub const SCARED: u8 = 2;
pub const UNROLLING: u8 = 3;

const ROLL_OUT: usize = 0;
const ROLL_UP: usize = 1;
const PEEK: usize = 2;

/// `ArmadilloState.animationDuration`.
fn animation_duration(state: u8) -> i32 {
    match state {
        ROLLING => 10,
        SCARED => 50,
        UNROLLING => 30,
        _ => 0,
    }
}

/// `Armadillo.ARMADILLO_STATE` and the brain's memory and behaviour.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub state: u8,
    /// `inStateTicks`.
    pub in_state_ticks: i32,
    /// `DANGER_DETECTED_RECENTLY`: ticks to live, 0 when absent.
    danger: i32,
    /// Ticks to the sensor's next scan.
    sensor: i32,
    /// `ArmadilloBallUp`: running, its peek timer and whether it saw the
    /// danger around last tick.
    ball_up: bool,
    next_peek: i32,
    danger_was_around: bool,
    /// Whether the PANIC activity was the active one after the last tick.
    panic_activity: bool,
    /// `peekReceivedClient`.
    peek_received: bool,
}

pub fn register(m: &mut Mob) {
    let mut panic = Panic::new(2.0);
    panic.environmental = true;
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, ArmadilloPanic(panic));
    g.add(2, Rolled);
    // ArmadilloAi.initIdleActivity: AnimalMakeLove(1.0), then RunOne of
    // FollowTemptation(1.25, armadillo_food) and BabyFollowAdult(5..16, 1.25).
    g.add(3, Idle(Breed::new(1.0)));
    let tempt = Tempt::new(1.25, m.kind.food.clone(), false);
    g.add(4, Idle(ArmadilloTempt(tempt)));
    g.add(4, Idle(BabyFollowAdult::default()));
    g.add(5, Stroll::avoiding_water(1.0));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
}

/// `AdultSensor` and `BabyFollowAdult.create(5..16, 1.25)`: a baby with no walk
/// target walks towards the nearest adult of its kind in sight that is 5 to 17
/// blocks away, until it is within 4.
#[derive(Clone, Debug, Default)]
struct BabyFollowAdult {
    adult: Option<u32>,
}
/// `UniformInt.of(5, 16)`'s bounds and `speedModifier`.
const FOLLOW_MIN: f64 = 5.0;
const FOLLOW_MAX: f64 = 16.0;
const FOLLOW_SPEED: f64 = 1.25;
impl BabyFollowAdult {
    /// `NearestVisibleLivingEntities.findClosest(same type and not a baby)`.
    fn nearest_adult(m: &mut Mob, cx: &mut Cx) -> Option<u32> {
        let species = m.species();
        let p = m.position;
        let area = Aabb::new(
            Vec3::new(p.x - FOLLOW_MAX, p.y - FOLLOW_MAX, p.z - FOLLOW_MAX),
            Vec3::new(p.x + FOLLOW_MAX, p.y + FOLLOW_MAX, p.z + FOLLOW_MAX),
        );
        let ids: Vec<(u32, f64)> =
            cx.w.others_near(area)
                .filter(|s| s.alive && !s.baby && s.species == species)
                .map(|s| (s.id, m.distance_sqr(s.position)))
                .collect();
        let mut best: Option<(u32, f64)> = None;
        for (id, d) in ids {
            if best.is_none_or(|b| d < b.1) && m.sees(cx.w, Who::Mob(id)) {
                best = Some((id, d));
            }
        }
        best.map(|b| b.0)
    }
    fn target(&self, m: &Mob, cx: &Cx) -> Option<([f64; 3], f64)> {
        let s = cx.w.seen(self.adult?)?;
        Some((
            [s.position.x, s.position.y, s.position.z],
            m.distance_sqr(s.position),
        ))
    }
}
impl Goal for BabyFollowAdult {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.is_baby() || !m.ai.nav.is_done() {
            return false;
        }
        self.adult = Self::nearest_adult(m, cx);
        self.target(m, cx).is_some_and(|(_, d)| {
            (FOLLOW_MIN * FOLLOW_MIN..(FOLLOW_MAX + 1.0) * (FOLLOW_MAX + 1.0)).contains(&d)
        })
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.is_baby()
            && !m.ai.nav.is_done()
            && self
                .target(m, cx)
                .is_some_and(|(_, d)| d > (FOLLOW_MIN - 1.0) * (FOLLOW_MIN - 1.0))
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((at, _)) = self.target(m, cx) {
            m.nav_move_to_entity(cx.w, at, FOLLOW_MIN as i32 - 1, FOLLOW_SPEED);
        }
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.nav_stop();
        self.adult = None;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(s) = self.adult.and_then(|a| cx.w.seen(a)) {
            m.look_at([s.position.x, s.eye_y, s.position.z]);
        }
    }
}

/// `Armadillo.isScared`.
pub fn scared(m: &Mob) -> bool {
    m.extra.armadillo.state != IDLE
}
/// `ArmadilloState.shouldHideInShell`: the model shows its shell.
pub fn hiding_in_shell(m: &Mob) -> bool {
    let a = &m.extra.armadillo;
    match a.state {
        ROLLING => a.in_state_ticks > 5,
        SCARED => true,
        UNROLLING => a.in_state_ticks < 26,
        _ => false,
    }
}
/// `Armadillo.getState`, as it is synched.
pub fn state(m: &Mob) -> u8 {
    m.extra.armadillo.state
}
/// `Armadillo.switchToState` and `onSyncedDataUpdated(ARMADILLO_STATE)`.
pub fn set_state(m: &mut Mob, state: u8) {
    let a = &mut m.extra.armadillo;
    if a.state != state {
        a.state = state;
        a.in_state_ticks = 0;
    }
}
/// `Armadillo.canStayRolledUp`.
fn can_stay_rolled_up(m: &Mob) -> bool {
    !m.ai.panicking
        && !m.fluids.in_water
        && !m.fluids.in_lava
        && m.extra.leash.is_none()
        && m.extra.mount.is_none()
        && m.extra.rider.is_none()
}
/// `Entity.playSound(sound)`.
fn play(m: &Mob, events: &mut Vec<MobEvent>, sound: &'static str) {
    if !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::Sound(sound, at, 1.0, 1.0));
    }
}
/// `Armadillo.rollUp`.
fn roll_up(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !scared(m) {
        super::creaking::stop_in_place(m);
        m.extra.in_love = 0;
        if !m.silent {
            m.voice("entity.armadillo.roll", events);
        }
        set_state(m, ROLLING);
    }
}
/// `Armadillo.rollOut`.
fn roll_out(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if scared(m) {
        if !m.silent {
            m.voice("entity.armadillo.unroll_finish", events);
        }
        set_state(m, IDLE);
    }
}

/// `Armadillo.hurtServer`: a ball takes half of the damage past the first
/// point (below it a negative amount, which `LivingEntity.hurtServer` makes 0).
pub fn reduce_damage(m: &Mob, amount: f32) -> f32 {
    if scared(m) {
        (amount - 1.0) / 2.0
    } else {
        amount
    }
}
/// `Armadillo.actuallyHurt`: a hit by a living thing curls it up (and keeps
/// the danger in mind for 80 ticks), a hit by fire and the like unrolls it.
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    if m.no_ai || m.health <= 0.0 {
        return;
    }
    let mut events = Vec::new();
    if source.attacker.is_some() {
        m.extra.armadillo.danger = 80;
        if can_stay_rolled_up(m) {
            roll_up(m, &mut events);
        }
    } else if environmental(source.kind) {
        roll_out(m, &mut events);
    }
    m.outbox.append(&mut events);
}
/// `DamageTypeTags.PANIC_ENVIRONMENTAL_CAUSES`.
pub fn environmental(kind: DamageKind) -> bool {
    matches!(
        kind,
        DamageKind::Cactus
            | DamageKind::OnFire
            | DamageKind::InFire
            | DamageKind::Lava
            | DamageKind::LightningBolt
            | DamageKind::HotFloor
    )
}
/// The hurt sound: a ball's is muffled, and it has no ambient sound
/// (`getAmbientSound`, `getHurtSound`); empty is none.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    let [ambient, hurt, death] = m.kind.sounds;
    if scared(m) {
        ["", "entity.armadillo.hurt_reduced", death]
    } else {
        [ambient, hurt, death]
    }
}

/// `EntityTypeTags.UNDEAD`.
fn undead(s: Species) -> bool {
    matches!(
        s,
        Species::Skeleton
            | Species::Stray
            | Species::WitherSkeleton
            | Species::SkeletonHorse
            | Species::Bogged
            | Species::Parched
            | Species::ZombieHorse
            | Species::CamelHusk
            | Species::Zombie
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
            | Species::Zoglin
            | Species::Drowned
            | Species::Husk
            | Species::ZombieNautilus
            | Species::Wither
            | Species::Phantom
    )
}
/// `Armadillo.isScaredBy` for the nearby living entities: within 7 blocks
/// sideways and 2 up and down, an undead mob, the last attacker, or a
/// player who sprints or rides.
fn scared_by_someone(m: &Mob, w: &MobWorld) -> bool {
    let area = m.bounds().inflate(Vec3::new(7.0, 2.0, 7.0));
    let attacker =
        m.ai.last_hurt_by
            .filter(|_| m.age as i32 - m.ai.last_hurt_by_time <= 100);
    if let Some(p) = &w.player
        && p.alive
        && !p.spectator
        && p.bounds.intersects(area)
        && (attacker == Some(Who::Player) || p.sprinting || p.riding)
    {
        return true;
    }
    w.others_near(area).any(|s| {
        s.id != m.id
            && s.alive
            && s.bounds.intersects(area)
            && (undead(s.species) || attacker == Some(Who::Mob(s.id)))
    })
}

/// `Armadillo.customServerAiStep`'s brain: memories expire, the scare
/// sensor scans every fifth tick, the core's rolling out, then the
/// `ArmadilloBallUp` behaviour of the PANIC activity.
pub fn server_ai_step(m: &mut Mob, cx: &mut Cx) {
    let events = &mut *cx.events;
    let a = &mut m.extra.armadillo;
    if a.danger > 0 {
        a.danger -= 1;
    }
    a.sensor -= 1;
    if a.sensor <= 0 {
        a.sensor = 5;
        if !can_stay_rolled_up(m) {
            m.extra.armadillo.danger = 0;
        } else if scared_by_someone(m, cx.w) {
            m.extra.armadillo.danger = 80;
        }
    }
    // ARMADILLO_ROLLING_OUT: no danger left in mind.
    if m.extra.armadillo.danger == 0 && scared(m) {
        roll_out(m, events);
    }
    if m.extra.armadillo.panic_activity && !m.extra.armadillo.ball_up && m.on_ground && !liquid(m) {
        // ArmadilloBallUp.start.
        m.extra.armadillo.ball_up = true;
        roll_up(m, events);
    }
    if m.extra.armadillo.ball_up {
        ball_up_tick(m, events);
    }
    let a = &mut m.extra.armadillo;
    a.panic_activity = a.danger > 0 && !m.ai.panicking;
}
fn liquid(m: &Mob) -> bool {
    m.fluids.in_water || m.fluids.in_lava
}
/// `ArmadilloBallUp.tickOrStop`.
fn ball_up_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !scared(m) {
        // canStillUse is false: stop.
        m.extra.armadillo.ball_up = false;
        if !can_stay_rolled_up(m) {
            roll_out(m, events);
        }
        return;
    }
    let a = &mut m.extra.armadillo;
    if a.next_peek > 0 {
        a.next_peek -= 1;
    }
    if a.state == ROLLING && a.in_state_ticks > animation_duration(ROLLING) {
        set_state(m, SCARED);
        if m.on_ground {
            play(m, events, "entity.armadillo.land");
        }
        return;
    }
    let danger = m.extra.armadillo.danger;
    let around = danger > 75;
    if around != m.extra.armadillo.danger_was_around {
        m.extra.armadillo.next_peek = next_peek_timer(m);
    }
    m.extra.armadillo.danger_was_around = around;
    let state = m.extra.armadillo.state;
    if state == SCARED {
        if m.extra.armadillo.next_peek == 0 && m.on_ground && around {
            peek(m, events);
            m.extra.armadillo.next_peek = next_peek_timer(m);
        }
        if danger < animation_duration(UNROLLING) {
            play(m, events, "entity.armadillo.unroll_start");
            set_state(m, UNROLLING);
        }
    } else if state == UNROLLING && danger > animation_duration(UNROLLING) {
        set_state(m, SCARED);
    }
}
/// `ArmadilloBallUp.pickNextPeekTimer`.
fn next_peek_timer(m: &mut Mob) -> i32 {
    animation_duration(SCARED) + m.random.next_int_between_inclusive(100, 400).unwrap_or(100)
}
/// `level.broadcastEntityEvent(body, 64)`; the sound is the client's
/// `playLocalSound`, which the server plays for everyone here.
fn peek(m: &mut Mob, events: &mut Vec<MobEvent>) {
    entity_event(m, 64);
    let at = [m.position.x, m.position.y, m.position.z];
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 64,
        at,
    });
    events.push(MobEvent::Sound("entity.armadillo.peek", at, 1.0, 1.0));
}
/// `Armadillo.handleEntityEvent`: 64 is a peek.
pub fn entity_event(m: &mut Mob, event: u8) {
    if event == 64 {
        m.extra.armadillo.peek_received = true;
    }
}

/// `Armadillo.tick`: the animation states on a client, the ball's head held
/// to its body, and `inStateTicks`. Server and client both run it.
pub fn tick(m: &mut Mob) {
    animate(m);
    if scared(m) {
        // clampHeadRotationToBody with getMaxHeadYRot 0.
        m.head_yaw += super::living::wrap(m.body_yaw - m.head_yaw);
    }
    m.extra.armadillo.in_state_ticks += 1;
}
/// `Armadillo.setupAnimationStates`.
fn animate(m: &mut Mob) {
    let age = m.age as f32;
    let a = &mut m.extra.armadillo;
    let s = &mut m.anim.states;
    match a.state {
        ROLLING => {
            s[ROLL_OUT].stop();
            s[ROLL_UP].start_if_stopped(age);
            s[PEEK].stop();
        }
        SCARED => {
            s[ROLL_OUT].stop();
            s[ROLL_UP].stop();
            if a.peek_received {
                s[PEEK].stop();
                a.peek_received = false;
            }
            if a.in_state_ticks == 0 {
                s[PEEK].start(age);
                s[PEEK].fast_forward(animation_duration(SCARED));
            } else {
                s[PEEK].start_if_stopped(age);
            }
        }
        UNROLLING => {
            s[ROLL_OUT].start_if_stopped(age);
            s[ROLL_UP].stop();
            s[PEEK].stop();
        }
        _ => {
            s[ROLL_OUT].stop();
            s[ROLL_UP].stop();
            s[PEEK].stop();
        }
    }
}

/// `ArmadilloAi.ArmadilloPanic`: it unrolls, then runs.
#[derive(Clone, Debug)]
struct ArmadilloPanic(Panic);
impl Goal for ArmadilloPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        "Panic"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        roll_out(m, cx.events);
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
/// A behaviour of the IDLE activity: `ArmadilloAi.updateActivity` picks
/// PANIC over it while danger is in mind and it is not panicking, and a
/// ball does not walk (`ArmadilloAi$1`, its `MoveToTargetSink`).
#[derive(Clone, Debug)]
struct Idle<G>(G);
fn idle(m: &Mob) -> bool {
    !scared(m) && !m.extra.armadillo.panic_activity
}
impl<G: Goal + Clone + 'static> Goal for Idle<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        idle(m) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        idle(m) && self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx);
    }
}
/// `FollowTemptation` with `ArmadilloAi.initIdleActivity`'s close enough
/// distance: 1 block for a baby, 2 for an adult.
#[derive(Clone, Debug)]
struct ArmadilloTempt(Tempt);
impl Goal for ArmadilloTempt {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
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
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop_distance = if m.is_baby() { 1.0 } else { 2.0 };
        self.0.tick(m, cx);
    }
}
/// The behaviours that do not run while the armadillo is a ball: its
/// `MoveToTargetSink` and the idle activity's walking and looking.
#[derive(Clone, Debug)]
struct Rolled;
impl Goal for Rolled {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        scared(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use std::sync::Arc;

    /// An armadillo on the ground.
    fn armadillo() -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:armadillo".into();
        k.species = Species::Armadillo;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.on_ground = true;
        m
    }

    #[test]
    fn a_hit_rolls_it_up_into_a_ball() {
        // Armadillo.actuallyHurt: a hit by a living thing curls it up and
        // keeps the danger in mind for 80 ticks, so the model shows the
        // shell instead of the armadillo (#2759).
        let mut m = armadillo();
        after_hurt(&mut m, &Damage::player([0.0, 0.0, 0.0]));
        assert_eq!(state(&m), ROLLING);
        // Armadillo.rollUp: the roll sound.
        assert!(
            m.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::Sound(s, ..) if *s == "entity.armadillo.roll")),
            "the roll plays its sound"
        );
        // ArmadilloState.shouldHideInShell: the shell shows after five ticks.
        for _ in 0..6 {
            tick(&mut m);
        }
        assert!(hiding_in_shell(&m), "the model shows the shell");
    }

    /// Runs the goals of `m` for `ticks` ticks; the events.
    fn run_goals(m: &mut Mob, w: &MobWorld, ticks: u32) -> Vec<MobEvent> {
        let mut events = Vec::new();
        let mut goals = std::mem::take(&mut m.ai.goals);
        let mut lr = horizon_core::random::LegacyRandom::new(0);
        let mut cx = Cx {
            w,
            lr: &mut lr,
            events: &mut events,
        };
        for _ in 0..ticks {
            goals.tick(m, &mut cx);
        }
        m.ai.goals = goals;
        events
    }
    /// Real armadillos from the vanilla data, with their goals, at these
    /// spots; `None` without the data.
    fn real<const N: usize>(at: [[f64; 3]; N]) -> Option<[Mob; N]> {
        let mut out = None;
        crate::mobs::world::with_test_world(&[], |_, types| {
            let kind = Arc::new(types.types["minecraft:armadillo"].clone());
            let mut id = 0;
            out = Some(at.map(|[x, y, z]| {
                id += 1;
                let mut m = Mob::new(id, kind.clone(), Vec3::new(x, y, z), 0.0);
                m.on_ground = true;
                m
            }));
        });
        out
    }

    #[test]
    fn two_armadillos_in_love_make_a_baby() {
        // ArmadilloAi.initIdleActivity: AnimalMakeLove(ARMADILLO, 1.0), so two
        // armadillos fed spider eyes breed (#3512).
        let Some([mut a, mut b, mut c]) =
            real([[0.5, 64.0, 0.5], [2.5, 64.0, 0.5], [0.5, 64.0, 1.5]])
        else {
            return;
        };
        for m in [&mut a, &mut b, &mut c] {
            m.extra.in_love = 600;
        }
        let partner = [b.seen(None)];
        crate::mobs::world::with_test_world(&partner, |w, _| {
            let events = run_goals(&mut a, w, 80);
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    MobEvent::Breed {
                        parent: 1,
                        partner: 2,
                        ..
                    }
                )),
                "the pair breeds"
            );
            assert_eq!(a.extra.in_love, 0);
            assert_eq!(a.extra.age, 6000, "the parent rests");

            // ArmadilloAi.updateActivity: with danger in mind and no panic,
            // PANIC replaces IDLE and its behaviours do not run, nor do a
            // ball's.
            c.extra.armadillo.panic_activity = true;
            let events = run_goals(&mut c, w, 80);
            assert!(!events.iter().any(|e| matches!(e, MobEvent::Breed { .. })));
            assert!(!c.ai.goals.is_running("Breed"));
            c.extra.armadillo.panic_activity = false;
            set_state(&mut c, SCARED);
            run_goals(&mut c, w, 1);
            assert!(!c.ai.goals.is_running("Breed"), "a ball does not breed");
            set_state(&mut c, IDLE);
            run_goals(&mut c, w, 1);
            assert!(c.ai.goals.is_running("Breed"), "it looks for its mate");
        });
    }

    #[test]
    fn a_baby_armadillo_follows_an_adult() {
        // BabyFollowAdult(5..16, 1.25) in the idle activity.
        let Some([mut baby, adult]) = real([[0.5, 64.0, 0.5], [8.5, 64.0, 0.5]]) else {
            return;
        };
        baby.extra.age = -24000;
        crate::mobs::world::with_test_world(&[adult.seen(None)], |w, _| {
            run_goals(&mut baby, w, 1);
            assert!(baby.ai.goals.is_running("BabyFollowAdult"));
            set_state(&mut baby, SCARED);
            run_goals(&mut baby, w, 1);
            assert!(
                !baby.ai.goals.is_running("BabyFollowAdult"),
                "not as a ball"
            );
        });
    }

    #[test]
    fn a_baby_stays_when_the_adult_is_within_five_blocks() {
        // BabyFollowAdult: `!closerThan(adult, 5)`.
        let Some([mut baby, adult]) = real([[0.5, 64.0, 0.5], [4.5, 64.0, 0.5]]) else {
            return;
        };
        baby.extra.age = -24000;
        crate::mobs::world::with_test_world(&[adult.seen(None)], |w, _| {
            run_goals(&mut baby, w, 1);
            assert!(!baby.ai.goals.is_running("BabyFollowAdult"));
        });
    }

    #[test]
    fn a_tempted_baby_comes_closer_than_an_adult() {
        // ArmadilloAi.initIdleActivity: FollowTemptation is close enough at
        // 1 block for a baby and 2 for an adult.
        let Some([mut baby, mut adult]) = real([[0.5, 64.0, 0.5], [4.5, 64.0, 0.5]]) else {
            return;
        };
        baby.extra.age = -24000;
        crate::mobs::world::with_test_world(&[], |w, _| {
            let mut events = Vec::new();
            let mut lr = horizon_core::random::LegacyRandom::new(0);
            let mut cx = Cx {
                w,
                lr: &mut lr,
                events: &mut events,
            };
            let mut tempt = ArmadilloTempt(Tempt::new(1.25, baby.kind.food.clone(), false));
            tempt.tick(&mut baby, &mut cx);
            assert_eq!(tempt.0.stop_distance, 1.0);
            tempt.tick(&mut adult, &mut cx);
            assert_eq!(tempt.0.stop_distance, 2.0);
        });
    }

    #[test]
    fn campfire_damage_is_fire_and_has_no_knockback_but_does_not_cause_panic() {
        // `campfire` is in #is_fire and #no_knockback, but in neither
        // #panic_causes nor #panic_environmental_causes (#4037).
        let source = Damage::of(DamageKind::Campfire);
        assert!(source.is_fire());
        assert!(source.no_knockback());
        assert!(!source.causes_panic());
        assert!(!environmental(DamageKind::Campfire));
    }
}
