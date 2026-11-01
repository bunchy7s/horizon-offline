//! Bats (`Bat`): hanging upside down under a solid block until a player comes
//! near, then fluttering about towards random nearby air (`customServerAiStep`),
//! and `BatModel`'s flying and resting keyframe animations. `extra.flag` is
//! `isResting`, `extra.positions[0]` the `targetPosition`; animation states:
//! flying, resting.
use super::{Mob, MobEvent, MobWorld, data::REDSTONE_CONDUCTOR, living::wrap};
use horizon_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};

/// `Bat(EntityType, Level)`: a new bat hangs.
pub fn register(m: &mut Mob) {
    m.extra.flag = true;
}
pub fn resting(m: &Mob) -> bool {
    m.extra.flag
}
/// `Bat.setResting`.
pub fn set_resting(m: &mut Mob, value: bool) {
    m.extra.flag = value;
}
/// `Bat.hurtServer`: a hit wakes it.
pub fn hurt(m: &mut Mob) {
    set_resting(m, false);
}
/// `levelEvent(1025)`: the take-off flutter, unless silent.
fn take_off(m: &Mob, p: [i32; 3], events: &mut Vec<MobEvent>) {
    if !m.silent {
        events.push(MobEvent::LevelEventSound {
            sound: "entity.bat.takeoff",
            at: [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ],
            volume: 0.05,
            spread: 0.2,
        });
    }
}
/// `Bat.customServerAiStep`.
pub fn server_ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let pos = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    let above = [pos[0], pos[1] + 1, pos[2]];
    let conductor =
        |p: [i32; 3]| w.data.state(w.world.block(p[0], p[1], p[2])).flags & REDSTONE_CONDUCTOR != 0;
    if resting(m) {
        if conductor(above) {
            if m.random.next_int_bound(200).unwrap_or(1) == 0 {
                m.head_yaw = m.random.next_int_bound(360).unwrap_or(0) as f32;
            }
            // getNearestPlayer(forNonCombat().range(4.0)).
            if m.can_see_player(w, 4.0) {
                set_resting(m, false);
                take_off(m, pos, events);
            }
        } else {
            set_resting(m, false);
            take_off(m, pos, events);
        }
        return;
    }
    if let Some(&t) = m.extra.positions.first()
        && (!w.facts.is_air(w.world.block(t[0], t[1], t[2])) || t[1] <= w.world.bottom_y())
    {
        m.extra.positions.clear();
    }
    let near = |t: [i32; 3]| {
        let d = |a: i32, b: f64| f64::from(a) + 0.5 - b;
        let (dx, dy, dz) = (
            d(t[0], m.position.x),
            d(t[1], m.position.y),
            d(t[2], m.position.z),
        );
        dx * dx + dy * dy + dz * dz < 4.0
    };
    let current = m.extra.positions.first().copied();
    if current.is_none()
        || m.random.next_int_bound(30).unwrap_or(1) == 0
        || current.is_some_and(near)
    {
        let r = &mut m.random;
        let mut offset = |bound: i32| r.next_int_bound(bound).unwrap_or(0);
        let x = m.position.x + f64::from(offset(7)) - f64::from(offset(7));
        let y = m.position.y + f64::from(offset(6)) - 2.0;
        let z = m.position.z + f64::from(offset(7)) - f64::from(offset(7));
        m.extra.positions.clear();
        m.extra
            .positions
            .push([x.floor() as i32, y.floor() as i32, z.floor() as i32]);
    }
    let t = m.extra.positions[0];
    let dx = f64::from(t[0]) + 0.5 - m.position.x;
    let dy = f64::from(t[1]) + 0.1 - m.position.y;
    let dz = f64::from(t[2]) + 0.5 - m.position.z;
    let v = m.velocity;
    let tenth = f64::from(0.1_f32);
    let v = Vec3::new(
        v.x + (signum(dx) * 0.5 - v.x) * tenth,
        v.y + (signum(dy) * f64::from(0.7_f32) - v.y) * tenth,
        v.z + (signum(dz) * 0.5 - v.z) * tenth,
    );
    m.velocity = v;
    let y_rot = trig::atan2_degrees(v.z, v.x) - 90.0;
    let diff = wrap(y_rot - m.yaw);
    m.ai.zza = 0.5;
    m.yaw += diff;
    if m.random.next_int_bound(100).unwrap_or(1) == 0 && conductor(above) {
        set_resting(m, true);
    }
}
/// `Math.signum`: zero stays zero (Rust's `signum` gives 1).
fn signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() {
        v
    } else {
        v.signum()
    }
}
/// `Bat.tick` after `super.tick()`: a hanging bat stays put, hooked to the
/// block above; a flying one loses 40% of its vertical speed. Then
/// `setupAnimationStates`.
pub fn tick(m: &mut Mob) {
    if resting(m) {
        m.velocity = Vec3::ZERO;
        m.position.y = m.position.y.floor() + 1.0 - f64::from(m.height());
    } else {
        m.velocity.y *= 0.6;
    }
    setup_animation_states(m);
}
/// `Bat.setupAnimationStates`: client visuals, without changing server motion.
pub fn setup_animation_states(m: &mut Mob) {
    let age = m.age as f32;
    if resting(m) {
        m.anim.states[0].stop();
        m.anim.states[1].start_if_stopped(age);
    } else {
        m.anim.states[1].stop();
        m.anim.states[0].start_if_stopped(age);
    }
}
