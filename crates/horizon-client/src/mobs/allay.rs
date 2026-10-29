//! Allays (`Allay`): given an item, they fetch matching drops within 32
//! blocks and bring them to their player (or a liked note block), dance to
//! jukeboxes and duplicate when given an amethyst shard mid-dance.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State},
    control::MoveKind,
    goals::{Cx, Goal, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, goal_clone},
    nav::NavKind,
};
use horizon_core::geometry::Vec3;
use horizon_core::geometry::trig::Mth;

// Allay state: `main_hand` the held item, `ints[1]` how many it carries,
// `tame` a liked player, `point` the wanted item (set by the game),
// `home` the liked note block with `ints[2]` its cooldown, `ints[3]` the
// pickup cooldown, `flag` dancing with `ints[4]` the dance clock,
// `egg_time` the duplication cooldown. AnimState slots: holding (0),
// spinning (1).
pub const HOLD: usize = 0;
pub const SPIN: usize = 1;

pub fn register(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Flying {
        max_turn: 20.0,
        hovers: true,
    };
    m.ai.nav.kind = NavKind::Flying;
    // FlyingPathNavigation.setRequiredPathLength(48).
    m.set_required_path_length(48.0);
    let g = &mut m.ai.goals;
    g.add(0, Panic::new(2.5));
    g.add(1, Fetch);
    g.add(2, Deliver);
    g.add(3, StayClose);
    g.add(4, Stroll::flying(1.0));
    g.add(5, LookAtPlayer::new(6.0));
    g.add(6, RandomLookAround::default());
}
/// Where carried items go: the liked note block, else the player.
fn deposit(m: &Mob, cx: &Cx) -> Option<[f64; 3]> {
    if let Some(h) = m.extra.home
        && m.extra.ints[2] > 0
    {
        return Some([
            f64::from(h[0]) + 0.5,
            f64::from(h[1]) + 1.0,
            f64::from(h[2]) + 0.5,
        ]);
    }
    if !m.extra.tame {
        return None;
    }
    let p = cx.w.player.as_ref().filter(|p| p.alive && !p.spectator)?;
    (p.position.distance_squared(m.position) <= 64.0 * 64.0).then_some([
        p.position.x,
        p.position.y,
        p.position.z,
    ])
}
/// `GoToWantedItem`: fly to the wanted drop.
#[derive(Clone, Debug)]
struct Fetch;
impl Goal for Fetch {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.point.is_some() && m.extra.ints[3] <= 0
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = m.extra.point
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 1.75);
        }
    }
}
/// `GoAndGiveItemsToTarget`: carry the load to the deposit and throw it.
#[derive(Clone, Debug)]
struct Deliver;
impl Goal for Deliver {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.ints[1] > 0 && m.extra.point.is_none() && deposit(m, cx).is_some()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = deposit(m, cx)
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 2.25);
        }
    }
}
/// `StayCloseToTarget.create(.., 4, 16, 2.25)`: acts only once the deposit is
/// 16 blocks away or more, then walks until within 4.
#[derive(Clone, Debug)]
struct StayClose;
impl Goal for StayClose {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.point.is_none()
            && deposit(m, cx).is_some_and(|p| {
                let d = (p[0] - m.position.x).powi(2)
                    + (p[1] - m.position.y).powi(2)
                    + (p[2] - m.position.z).powi(2);
                d >= 256.0
            })
    }
    /// `MoveToTargetSink.reachedTarget`: the walk target it set is reached
    /// within a Manhattan block distance of 4 (`closeEnough`).
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.point.is_none()
            && deposit(m, cx).is_some_and(|p| {
                let at = [m.position.x, m.position.y, m.position.z];
                (0..3)
                    .map(|i| (p[i].floor() as i32 - at[i].floor() as i32).abs())
                    .sum::<i32>()
                    > 4
            })
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = deposit(m, cx)
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 2.25);
        }
    }
}
/// `Allay.travel`: `travelFlying(input, getSpeed())` — the airborne speed is
/// the one the move control set this tick, and no gravity applies, so an
/// allay never falls under its own travel.
pub fn travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    let speed = m.ai.speed;
    m.travel_flying_speeds(w, input, events, 0.02, 0.02, speed);
    true
}
/// `Allay.aiStep` / `tick`: healing, cooldowns, the hold and spin clocks.
pub fn ai_step(m: &mut Mob) {
    if m.alive() && m.age.is_multiple_of(10) {
        m.health = (m.health + 1.0).min(m.attrs.max_health);
    }
    for i in [2, 3] {
        if m.extra.ints[i] > 0 {
            m.extra.ints[i] -= 1;
        }
    }
    if m.extra.egg_time > 0 {
        m.extra.egg_time -= 1;
    }
    animate(m);
}
/// `Allay.tick`'s client half: the hold and spin clocks (the server's and a
/// client's).
pub fn animate(m: &mut Mob) {
    let holding = m.extra.main_hand.is_some();
    let v = &mut m.anim.v;
    v[HOLD] = (v[HOLD] + if holding { 1.0 } else { -1.0 }).clamp(0.0, 5.0);
    if m.extra.flag {
        m.extra.ints[4] += 1;
        let spinning = (m.extra.ints[4] as f32 % 55.0) < 15.0;
        v[SPIN] = (v[SPIN] + if spinning { 1.0 } else { -1.0 }).clamp(0.0, 15.0);
    } else {
        m.extra.ints[4] = 0;
        v[SPIN] = 0.0;
        m.anim.vo[SPIN] = 0.0;
    }
}
/// `AllayModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    use std::f32::consts::PI;
    let (speed, pos) = (s.speed, s.pos);
    let flap_speed = (s.age * 20.0).to_radians() + pos;
    let flap = flap_speed.mcos() * PI * 0.15 + speed;
    let idle_bob = (s.age * 9.0).to_radians();
    let flying = (speed / 0.3).min(1.0);
    let idle = 1.0 - flying;
    let hold = m.anim.lerp(HOLD, s.partial) / 5.0;
    if m.extra.flag {
        let dance = (s.age * 8.0).to_radians() + speed;
        let freq = dance.mcos() * 16f32.to_radians();
        let spin = m.anim.lerp(SPIN, s.partial) / 15.0;
        let spinning = (m.extra.ints[4] as f32 % 55.0) < 15.0;
        if spinning {
            p.yrot("root", PI * 4.0 * spin);
        }
        p.zrot("root", freq * (1.0 - spin));
        p.yrot("head", dance.mcos() * 30f32.to_radians() * (1.0 - spin));
        p.zrot("head", dance.mcos() * 14f32.to_radians() * (1.0 - spin));
    } else {
        p.xrot("head", s.x_rot.to_radians());
        p.yrot("head", s.y_rot.to_radians());
    }
    p.xrot("right_wing", 0.436_332_32 * (1.0 - flying));
    p.yrot("right_wing", -PI / 4.0 + flap);
    p.xrot("left_wing", 0.436_332_32 * (1.0 - flying));
    p.yrot("left_wing", PI / 4.0 - flap);
    p.xrot("body", flying * PI / 4.0);
    let arm_x = hold * (-PI / 3.0 + (-1.134_464 + PI / 3.0) * flying);
    // `(float) Math.cos`: the exact function, not `Mth.cos`.
    p.get("root")[1] += (f64::from(idle_bob).cos() as f32) * 0.25 * idle;
    p.xrot("right_arm", arm_x);
    p.xrot("left_arm", arm_x);
    let arm_bob = idle * (1.0 - hold);
    let amount = 0.436_332_32 - (idle_bob + PI * 1.5).mcos() * PI * 0.075 * arm_bob;
    p.zrot("left_arm", -amount);
    p.zrot("right_arm", amount);
    p.yrot("right_arm", 0.279_252_68 * hold);
    p.yrot("left_arm", -0.279_252_68 * hold);
}
