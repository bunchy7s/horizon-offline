//! Polar bears (`PolarBear`): a neutral animal that defends its cubs,
//! fights back when hurt, and rears up before each swing. A cub flees what
//! hurts it and calls the adults.
//!
//! `extra.flag` is `DATA_STANDING_ID`, `extra.counter` the warning sound's
//! delay, `extra.anger` the `NeutralMob` anger left and `extra.flag2` whether
//! the player is who it is angry at. `anim.v[STAND]` is
//! `clientSideStandAnimation`.
use horizon_core::{geometry::Vec3, random::RandomSource};

use super::{
    DamageKind, Mob, MobWorld, Species, Who,
    goals::{
        Cx, Float, FollowParent, Goal, HurtByTarget, LookAtPlayer, MOVE, MeleeAttack,
        NearestTarget, Panic, RandomLookAround, Stroll, goal_clone,
    },
    world::TargetInfo,
};

/// `anim.v` slot of `clientSideStandAnimation`.
pub const STAND: usize = 0;
/// `STAND_ANIMATION_TICKS`.
const STAND_TICKS: f32 = 6.0;

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, MeleeAttack::new(1.25, true).polar_bear());
    g.add(1, PolarBearPanic(Panic::new(2.0)));
    g.add(4, FollowParent::new(1.25));
    g.add(5, Stroll::new(1.0));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
    let t = &mut m.ai.targets;
    t.add(1, HurtByTarget::new().polar_bear());
    // PolarBearAttackPlayersGoal: players near a cub, within half the
    // follow range, never for a cub itself.
    t.add(
        2,
        NearestTarget::player(true)
            .interval(20)
            .follow_scale(0.5)
            .reach()
            .gate(|m, _| !m.is_baby())
            .when(cub_nearby)
            .named("PolarBearAttackPlayersGoal"),
    );
    t.add(
        3,
        NearestTarget::player(true).when(|m, _| m.extra.anger > 0 && m.extra.flag2),
    );
    t.add(
        4,
        NearestTarget::species(&[Species::Fox], true)
            .interval(10)
            .reach()
            .when(|m, _| !m.is_baby()),
    );
}

/// `PolarBearAttackPlayersGoal.canUse`: a polar bear cub within 8 blocks
/// (4 up and down).
fn cub_nearby(m: &Mob, w: &MobWorld) -> bool {
    let area = m.bounds().inflate(Vec3::new(8.0, 4.0, 8.0));
    w.others_near(area)
        .any(|s| s.species == Species::PolarBear && s.baby && s.bounds.intersects(area))
}

/// `PanicGoal` with a function of the bear's age for the damage tag: a cub
/// panics at `PANIC_CAUSES`, an adult only at `PANIC_ENVIRONMENTAL_CAUSES`.
#[derive(Clone, Debug)]
struct PolarBearPanic(Panic);
impl Goal for PolarBearPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        (m.is_baby() || environmental_panic(m)) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
}
/// `DamageTypeTags.PANIC_ENVIRONMENTAL_CAUSES` (fire, lava, cactus), or
/// burning.
fn environmental_panic(m: &Mob) -> bool {
    m.fire_ticks > 0 && !m.kind.flags.fire_immune
        || m.last_damage.is_some_and(|(d, t)| {
            matches!(
                d.kind,
                DamageKind::OnFire
                    | DamageKind::InFire
                    | DamageKind::Lava
                    | DamageKind::Cactus
                    | DamageKind::LightningBolt
                    | DamageKind::HotFloor
            ) && i64::from(m.age) - t <= 40
        })
}

/// `PolarBear.setStanding`.
pub fn set_standing(m: &mut Mob, standing: bool) {
    m.extra.flag = standing;
}

/// `PolarBear.playWarningSound`.
fn play_warning_sound(m: &mut Mob, cx: &mut Cx) {
    if m.extra.counter <= 0 {
        m.voice("entity.polar_bear.warning", cx.events);
        m.extra.counter = 40;
    }
}

/// `PolarBearMeleeAttackGoal.checkAndPerformAttack`: strike when ready and
/// in reach, rear up and growl in the last 10 ticks while close.
pub fn check_and_perform_attack(
    until_attack: &mut i32,
    cooldown: i32,
    m: &mut Mob,
    who: Who,
    t: &TargetInfo,
    cx: &mut Cx,
) {
    if *until_attack <= 0 && m.within_melee_range(t) && m.sees(cx.w, who) {
        *until_attack = cooldown;
        super::ai::do_hurt_target(m, who, cx);
        set_standing(m, false);
        return;
    }
    let reach = t.bounds.size().x as f32 + 3.0;
    if m.distance_sqr(t.position) < f64::from(reach * reach) {
        if *until_attack <= 0 {
            set_standing(m, false);
            *until_attack = cooldown;
        }
        if *until_attack <= 10 {
            set_standing(m, true);
            play_warning_sound(m, cx);
        }
    } else {
        *until_attack = cooldown;
        set_standing(m, false);
    }
}

/// `PolarBear.tick`'s client half: `clientSideStandAnimation` follows the
/// synched standing flag by one tick a tick. A client of a server runs it
/// too.
pub fn client_tick(m: &mut Mob) {
    let v = m.anim.v[STAND];
    m.anim.vo[STAND] = v;
    let step = if m.extra.flag { 1.0 } else { -1.0 };
    m.anim.v[STAND] = (v + step).clamp(0.0, STAND_TICKS);
}
/// `PolarBear.getStandingAnimationScale`.
pub fn standing_scale(m: &Mob, partial: f32) -> f32 {
    m.anim.lerp(STAND, partial) / STAND_TICKS
}

/// `PolarBear.tick`: the stand animation, the warning sound's delay and
/// `NeutralMob.updatePersistentAnger`.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    client_tick(m);
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.ai.target.is_some() {
        // Anger follows the target and starts over every tick it is held:
        // 20 to 39 seconds (`PERSISTENT_ANGER_TIME`).
        m.extra.flag2 = m.ai.target == Some(Who::Player);
        m.extra.anger = 400 + m.random.next_int_bound(381).unwrap_or(0);
    } else if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 {
            stop_being_angry(m);
        }
    }
    // A player in creative or spectator mode is forgiven at once.
    if m.extra.flag2 && w.player.as_ref().is_some_and(|p| p.creative || p.spectator) {
        stop_being_angry(m);
    }
}
/// `NeutralMob.playerDied`: a bear angry at the player (its target, kept in
/// `flag2` until the anger ends) stops.
pub fn forgive_dead_player(m: &mut Mob) {
    if m.extra.flag2 {
        stop_being_angry(m);
    }
}
/// `NeutralMob.stopBeingAngry`.
fn stop_being_angry(m: &mut Mob) {
    m.ai.last_hurt_by = None;
    m.ai.target = None;
    m.extra.anger = 0;
    m.extra.flag2 = false;
}
