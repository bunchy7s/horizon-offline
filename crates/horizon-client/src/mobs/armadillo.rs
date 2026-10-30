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
        Cx, Float, Goal, LOOK, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, goal_clone,
    },
};
use horizon_core::{geometry::Vec3, random::RandomSource};

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
    g.add(5, Stroll::avoiding_water(1.0));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
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
/// point.
pub fn reduce_damage(m: &Mob, amount: f32) -> f32 {
    if scared(m) {
        ((amount - 1.0) / 2.0).max(0.0)
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
        DamageKind::Cactus | DamageKind::OnFire | DamageKind::InFire | DamageKind::Lava
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
