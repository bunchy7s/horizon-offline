//! Happy ghasts and ghastlings (`HappyGhast`, `HappyGhastAi`): the adult
//! floats about following torchlight of snowballs and harnesses, the
//! ghastling follows players and adults, a harnessed adult can be ridden and
//! flown, healing is slow (fast in the rain) and `HappyGhastModel`.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State},
    control::{LookKind, MoveKind, Operation},
    goals::{Cx, Float, Goal, LOOK, MOVE, goal_clone},
    nav::NavKind,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

// Happy ghast state: `head` the harness, `scratch[0]` 1 once set up as a
// baby and 2 as an adult, `ai.home` the restriction, `counter`
// the end tick of the follow-an-adult range pick.
// `scratch[1]` is `leashHolderTime` on the server and `IS_LEASH_HOLDER` (1 or 0) on a client.
/// `HappyGhast.notifyLeashHolder` for a leashed mob that supports a quad lead.
pub fn notify_leash_holder(m: &mut Mob) {
    if m.species() == super::Species::HappyGhast {
        m.extra.scratch[1] = 5;
    }
}
/// `HappyGhast.isLeashHolder`: `IS_LEASH_HOLDER`, true while a quad-leashed
/// mob hangs on it (and four ticks after).
pub fn is_leash_holder(m: &Mob) -> bool {
    m.extra.scratch[1] > 0
}
/// `Attributes.FLYING_SPEED` and the speed `travel` uses (`* 5 / 3`).
pub const FLYING_SPEED: f64 = 0.05;
pub const AIR_SPEED: f32 = 0.05 * 5.0 / 3.0;

pub fn register(m: &mut Mob) {
    setup(m, m.is_baby());
}
/// `adultGhastSetup` and `babyGhastSetup`: the ghastling flies with a
/// `FlyingMoveControl` and has no goals (its brain drives it); the adult
/// floats with `GhastMoveControl` and three goals, its brain stopped.
fn setup(m: &mut Mob, baby: bool) {
    let was = m.extra.scratch[0];
    m.extra.scratch[0] = if baby { 1 } else { 2 };
    m.ai.goals.clear();
    m.ai.nav.stop();
    // A new move and look control (nothing wanted) and navigation.
    m.ai.mov.operation = Operation::Wait;
    m.ai.mov.wanted = [0.0; 3];
    m.ai.mov.speed_modifier = 0.0;
    m.ai.look.wanted = [0.0; 3];
    m.ai.look.cooldown = 0;
    m.extra.counter2 = 0;
    // The new navigation's PathNavigation constructor.
    m.ai.nav.required_path_length = 16.0;
    m.ai.nav.max_visited_nodes =
        (super::finalize::base_value(m, super::finalize::Attr::FollowRange) * 16.0).floor() as i32;
    if baby {
        m.ai.look.kind = LookKind::Default;
        m.ai.mov.kind = MoveKind::Flying {
            max_turn: 180.0,
            hovers: true,
        };
        // BabyFlyingPathNavigation.
        m.ai.nav.kind = NavKind::Flying;
        m.ai.nav.can_float = true;
        m.set_required_path_length(48.0);
        return;
    }
    if was == 1 {
        super::animal_brain::grown_up(m);
    }
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.kind = NavKind::Ground;
    m.ai.look.kind = LookKind::HappyGhast;
    let g = &mut m.ai.goals;
    g.add(3, HappyGhastFloat);
    g.add(4, GhastTempt::default());
    g.add(5, FloatAround { blocks: 16 });
}
fn wearing_harness(m: &Mob) -> bool {
    m.extra.head.is_some()
}
/// `getControllingPassenger`: a player on a harnessed adult.
pub fn controlled(m: &Mob) -> bool {
    m.rider.is_some() && harnessed_adult(m)
}
/// A harnessed adult, whether or not a rider is on it yet.
pub fn harnessed_adult(m: &Mob) -> bool {
    wearing_harness(m) && !m.is_baby()
}
/// `getHappyGhastRestrictionRadius`.
fn restriction_radius(m: &Mob) -> i32 {
    if !m.is_baby() && m.extra.head.is_none() {
        64
    } else {
        32
    }
}
/// `Mob.isWithinHome` for the ghast's restriction.
fn within_home(m: &Mob, p: Vec3) -> bool {
    let Some((h, r)) = m.ai.home else {
        return true;
    };
    let r = f64::from(r);
    let (dx, dy, dz) = (
        f64::from(h[0]) + 0.5 - p.x,
        f64::from(h[1]) + 0.5 - p.y,
        f64::from(h[2]) + 0.5 - p.z,
    );
    dx * dx + dy * dy + dz * dz < r * r
}
/// `isOnStillTimeout`: a player standing on it (or just off it) holds it
/// still (`serverStillTimeout`, kept in `counter`).
pub fn on_still_timeout(m: &Mob) -> bool {
    !m.is_baby() && m.extra.counter > 0
}
/// Client-side `HappyGhast.canBeCollidedWith(Player)`.
pub fn collides_with_player(m: &Mob, feet_y: f64) -> bool {
    player_collision(
        m.alive(),
        m.is_baby(),
        on_still_timeout(m),
        feet_y,
        m.bounds().max_position().y,
    )
}
fn player_collision(alive: bool, baby: bool, still: bool, feet_y: f64, top: f64) -> bool {
    alive && !baby && (feet_y >= top || still)
}
/// `Mob.stopInPlace`.
fn stop_in_place(m: &mut Mob) {
    m.ai.nav.stop();
    m.ai.xxa = 0.0;
    m.ai.yya = 0.0;
    m.ai.zza = 0.0;
    m.ai.speed = 0.0;
    m.velocity = Vec3::ZERO;
}
/// `Ghast.GhastMoveControl(careful)`: held still on a still timeout, else
/// a push toward the wanted point every 2 to 6 ticks while the way there
/// is clear.
pub fn move_control(m: &mut Mob, w: &MobWorld) {
    if on_still_timeout(m) {
        m.ai.mov.operation = Operation::Wait;
        stop_in_place(m);
    }
    if m.ai.mov.operation != Operation::MoveTo {
        return;
    }
    // floatDuration-- <= 0
    let duration = m.extra.counter2;
    m.extra.counter2 -= 1;
    if duration > 0 {
        return;
    }
    m.extra.counter2 += m.random.next_int_bound(5).unwrap_or(0) + 2;
    let [wx, wy, wz] = m.ai.mov.wanted;
    let travel = Vec3::new(wx - m.position.x, wy - m.position.y, wz - m.position.z);
    if can_reach(m, w, travel) {
        // travel.normalize().scale(FLYING_SPEED * 5 / 3)
        let len = (travel.x * travel.x + travel.y * travel.y + travel.z * travel.z).sqrt();
        let unit = if len < f64::from(1.0e-5_f32) {
            Vec3::ZERO
        } else {
            Vec3::new(travel.x / len, travel.y / len, travel.z / len)
        };
        let speed = FLYING_SPEED * 5.0 / 3.0;
        m.velocity =
            m.velocity
                .add_vector(Vec3::new(unit.x * speed, unit.y * speed, unit.z * speed));
    } else {
        m.ai.mov.operation = Operation::Wait;
    }
}
/// `EntityDimensions.makeBoundingBox` at another position.
fn bounds_at(m: &Mob, p: Vec3) -> Aabb {
    let half = f64::from(m.width()) / 2.0;
    let h = f64::from(m.height());
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + h, p.z + half),
    )
}
/// `GhastMoveControl.canReach`: every block around the destination box can
/// be crossed, and so can every block the box sweeps through on the way.
fn can_reach(m: &Mob, w: &MobWorld, travel: Vec3) -> bool {
    let aabb = m.bounds();
    let dest = aabb.move_by(travel);
    let around = dest.inflate(Vec3::new(1.0, 1.0, 1.0));
    let (lo, hi) = (around.min_position(), around.max_position());
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if !traversal_possible(m, w, None, [x, y, z], false, false) {
                    return false;
                }
            }
        }
    }
    let (water, lava) = (m.fluids.in_water, m.fluids.in_lava);
    let start = m.position;
    let end = start.add_vector(travel);
    horizon_core::geometry::sweep_intersected_blocks(start, end, dest, 1 << 22, |p, _| {
        let at = [p.x, p.y, p.z];
        let unit = Aabb::new(
            Vec3::new(f64::from(at[0]), f64::from(at[1]), f64::from(at[2])),
            Vec3::new(
                f64::from(at[0]) + 1.0,
                f64::from(at[1]) + 1.0,
                f64::from(at[2]) + 1.0,
            ),
        );
        aabb.intersects(unit) || traversal_possible(m, w, Some((start, end)), at, water, lava)
    })
    .unwrap_or(false)
}
/// `GhastMoveControl.blockTraversalPossible` for a careful ghast: air, or no
/// collision (along the way when `along` is given), never a
/// `#happy_ghast_avoids` block, and a fluid only where the ghast already is.
fn traversal_possible(
    m: &Mob,
    w: &MobWorld,
    along: Option<(Vec3, Vec3)>,
    p: [i32; 3],
    through_water: bool,
    through_lava: bool,
) -> bool {
    let state = w.world.block(p[0], p[1], p[2]);
    if w.facts.is_air(state) {
        return true;
    }
    let boxes = w.block_collision_boxes(p[0], p[1], p[2]);
    let no_collisions = match along {
        Some((from, to)) => !bounds_at(m, from).collided_along_vector(to.subtract(from), &boxes),
        None => boxes.is_empty(),
    };
    if w.has_tag(
        "minecraft:happy_ghast_avoids",
        w.block_name(p[0], p[1], p[2]),
    ) {
        return false;
    }
    let (fluid, height) = w.fluid_height(p[0], p[1], p[2]);
    if fluid != crate::blocks::Fluid::None {
        let collided = match along {
            None => true,
            // collidedWithFluid: the fluid's own height.
            Some((from, to)) => {
                let fluid_box = Aabb::new(
                    Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])),
                    Vec3::new(
                        f64::from(p[0]) + 1.0,
                        f64::from(p[1]) + height,
                        f64::from(p[2]) + 1.0,
                    ),
                );
                bounds_at(m, from).collided_along_vector(to.subtract(from), &[fluid_box])
            }
        };
        if collided {
            return match fluid {
                crate::blocks::Fluid::Water => through_water,
                _ => through_lava,
            };
        }
    }
    no_collisions
}
/// `HappyGhast.HappyGhastLookControl`: square to the world on a still
/// timeout, face what it looks at, else face its motion
/// (`Ghast.faceMovementDirection`).
pub fn look_control(m: &mut Mob) {
    use horizon_core::geometry::trig;
    let to_deg = 180.0_f32 / std::f32::consts::PI;
    if on_still_timeout(m) {
        // Mth.wrapDegrees90
        let mut close = m.yaw % 90.0;
        if close >= 45.0 {
            close -= 90.0;
        }
        if close < -45.0 {
            close += 90.0;
        }
        m.yaw -= close;
        m.head_yaw = m.yaw;
    } else if m.ai.look.cooldown > 0 {
        m.ai.look.cooldown -= 1;
        let [wx, _, wz] = m.ai.look.wanted;
        m.yaw = -(trig::atan2(wx - m.position.x, wz - m.position.z) as f32) * to_deg;
        m.body_yaw = m.yaw;
        m.head_yaw = m.body_yaw;
    } else if m.ai.target.is_none() {
        let v = m.velocity;
        m.yaw = -(trig::atan2(v.x, v.z) as f32) * to_deg;
        m.body_yaw = m.yaw;
    }
}
/// `HappyGhast.tick`'s still timeout: it runs out (after the first three
/// seconds) unless a player stands on top (`scanPlayerAboveGhast`).
pub fn still_tick(m: &mut Mob, w: &MobWorld) {
    if m.is_baby() {
        m.extra.counter = 0;
        return;
    }
    if m.extra.counter > 0 && m.age > 60 {
        m.extra.counter -= 1;
    }
    if player_above(m, w) {
        m.extra.counter = 10;
    }
}
/// `scanPlayerAboveGhast`: a player (not one riding a happy ghast) within
/// a block around the top, up to half the ghast's height above it.
pub fn player_above(m: &Mob, w: &MobWorld) -> bool {
    let Some(p) = w.player.as_ref().filter(|p| !p.spectator) else {
        return false;
    };
    if m.rider.is_some() {
        return false;
    }
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    let area = Aabb::new(
        Vec3::new(lo.x - 1.0, hi.y - f64::from(1.0e-5_f32), lo.z - 1.0),
        Vec3::new(hi.x + 1.0, hi.y + (hi.y - lo.y) / 2.0, hi.z + 1.0),
    );
    area.contains(p.position)
}
/// `HappyGhastFloatGoal`: a `FloatGoal` off a still timeout.
#[derive(Clone, Debug)]
struct HappyGhastFloat;
impl Goal for HappyGhastFloat {
    goal_clone!();
    fn name(&self) -> &'static str {
        "HappyGhastFloatGoal"
    }
    fn flags(&self) -> u8 {
        Float.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !on_still_timeout(m) && Float.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        Float.can_use(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        Float.tick(m, cx)
    }
}
/// `TemptGoal.ForNonPathfinders`: drift toward a player holding food, or
/// food and harnesses for a bare adult.
#[derive(Clone, Debug, Default)]
struct GhastTempt {
    calm_down: i32,
}
impl GhastTempt {
    fn wants(m: &Mob, item: &Option<std::sync::Arc<str>>) -> bool {
        let Some(i) = item else { return false };
        let harness = !wearing_harness(m) && !m.is_baby() && i.ends_with("_harness");
        harness || m.kind.food.iter().any(|f| f.as_str() == &**i)
    }
}
impl Goal for GhastTempt {
    goal_clone!();
    fn name(&self) -> &'static str {
        "ForNonPathfinders"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let Some(p) = &cx.w.player else { return false };
        // getNearestPlayer(TargetingConditions.forNonCombat()
        // .ignoreLineOfSight().range(TEMPT_RANGE 16)): creative players too.
        let range = (16.0_f64 * p.visibility_percent()).max(2.0);
        p.alive
            && !p.spectator
            && (Self::wants(m, &p.main_hand) || Self::wants(m, &p.off_hand))
            && m.distance_sqr(p.position) <= range * range
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        // stopNavigation: setWait; calmDown = reducedTickDelay(100).
        m.ai.mov.operation = Operation::Wait;
        self.calm_down = super::goals::reduced(100);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.player.clone() else { return };
        m.look_at(p.eye);
        // stopDistance 7.
        if m.distance_sqr(p.position) < 49.0 {
            m.ai.mov.operation = Operation::Wait;
        } else {
            let f = m.random.next_double();
            let at = [
                (p.eye[0] - m.position.x) * f + m.position.x,
                (p.eye[1] - m.position.y) * f + m.position.y,
                (p.eye[2] - m.position.z) * f + m.position.z,
            ];
            m.ai.mov.set_wanted(at[0], at[1], at[2], 1.0);
        }
    }
}
/// `Ghast.RandomFloatAroundGoal` with a restriction and a wish for open
/// air next to something solid.
#[derive(Clone, Debug)]
struct FloatAround {
    blocks: i32,
}
impl FloatAround {
    fn good(&self, w: &MobWorld, p: Vec3) -> bool {
        if self.blocks <= 0 {
            return true;
        }
        let at = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        if !w.facts.is_air(w.world.block(at[0], at[1], at[2])) {
            return false;
        }
        for d in [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ] {
            for i in 1..self.blocks {
                if !w.facts.is_air(w.world.block(
                    at[0] + d[0] * i,
                    at[1] + d[1] * i,
                    at[2] + d[2] * i,
                )) {
                    return true;
                }
            }
        }
        false
    }
}
impl Goal for FloatAround {
    goal_clone!();
    fn name(&self) -> &'static str {
        "RandomFloatAroundGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        if !m.ai.mov.has_wanted() {
            return true;
        }
        let [x, y, z] = m.ai.mov.wanted;
        let d =
            (x - m.position.x).powi(2) + (y - m.position.y).powi(2) + (z - m.position.z).powi(2);
        !(1.0..=3600.0).contains(&d)
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        false
    }
    /// `getSuitableFlyToPosition`: the first of 64 random points within 16
    /// blocks (and home) that is good; failing that, the last point, kept
    /// below the motion-blocking surface.
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let c = m.position;
        let choose = |m: &mut Mob| {
            let r = &mut m.random;
            Vec3::new(
                c.x + f64::from((r.next_float() * 2.0 - 1.0) * 16.0),
                c.y + f64::from((r.next_float() * 2.0 - 1.0) * 16.0),
                c.z + f64::from((r.next_float() * 2.0 - 1.0) * 16.0),
            )
        };
        let mut result: Option<Vec3> = None;
        for _ in 0..64 {
            let t = choose(m);
            result = within_home(m, t).then_some(t);
            if let Some(t) = result
                && self.good(cx.w, t)
            {
                m.ai.mov.set_wanted(t.x, t.y, t.z, 1.0);
                return;
            }
        }
        let mut t = result.unwrap_or_else(|| choose(m));
        let top =
            cx.w.motion_blocking_height(t.x.floor() as i32, t.z.floor() as i32);
        if top < t.y.floor() as i32 && top > cx.w.world.bottom_y() {
            t.y = m.position.y - (m.position.y - t.y).abs();
        }
        m.ai.mov.set_wanted(t.x, t.y, t.z, 1.0);
    }
}
/// What a rider asks (`getRiddenInput`, `tickRidden`).
pub fn ridden(m: &mut Mob) {
    let Some(r) = m.rider else { return };
    // tickRidden: turn 8% of the way toward the rider's yaw; pitch half of theirs.
    let diff = super::living::wrap(r.yaw - m.yaw);
    m.yaw += diff * 0.08;
    m.head_pitch = r.pitch * 0.5;
    m.body_yaw = m.yaw;
    m.head_yaw = m.yaw;
    m.ai.nav.stop();
    let (mut forward, mut up) = (0.0, 0.0);
    if r.forward != 0.0 {
        let mut f = r.pitch.to_radians().cos();
        let mut u = -r.pitch.to_radians().sin();
        if r.forward < 0.0 {
            f *= -0.5;
            u *= -0.5;
        }
        forward = f;
        up = u;
    }
    if r.jumping {
        up += 0.5;
    }
    let scale = 3.9 * FLYING_SPEED as f32;
    m.ai.xxa = r.strafe * scale;
    m.ai.yya = up * scale;
    m.ai.zza = forward * scale;
    m.ai.mov.operation = Operation::Wait;
}
/// The adult or ghastling setup for its age (`ageBoundaryReached`).
pub fn ensure_setup(m: &mut Mob) {
    let baby = m.is_baby();
    if m.extra.scratch[0] != if baby { 1 } else { 2 } {
        setup(m, baby);
    }
}
/// `HappyGhast.aiStep`, `tick` and `checkRestriction`: the adult/baby
/// setup at the age boundary, the home, face-the-motion and slow healing.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    ensure_setup(m);
    // HappyGhast.tick: `leashHolderTime` runs down (the synced flag follows it).
    if m.extra.scratch[1] > 0 {
        m.extra.scratch[1] -= 1;
    }
    // checkRestriction.
    if m.extra.leash.is_none() && m.rider.is_none() {
        let radius = restriction_radius(m);
        let here = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        // !hasHome || !home.closerThan(blockPosition, radius + 16) || radius changed
        let stale = match m.ai.home {
            Some((h, r)) => {
                (h[0] - here[0]).pow(2) + (h[1] - here[1]).pow(2) + (h[2] - here[2]).pow(2)
                    >= (radius + 16).pow(2)
                    || radius != r
            }
            None => true,
        };
        if stale {
            m.ai.home = Some((here, radius));
        }
    }
    // continuousHeal: a point every 30 seconds, every second in rain or snow
    // (Level.precipitationAt).
    if m.alive() && m.death_time == 0 && m.health < m.attrs.max_health {
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let wet = w.precipitation_at(x, y, z).is_some();
        let interval = if wet { 20 } else { 600 };
        if m.age.is_multiple_of(interval) {
            m.health = (m.health + 1.0).min(m.attrs.max_health);
        }
    }
    still_tick(m, w);
}
/// Sounds: the ghastling has its own.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    if m.is_baby() {
        [
            "entity.ghastling.ambient",
            "entity.ghastling.hurt",
            "entity.ghastling.death",
        ]
    } else {
        [
            "entity.happy_ghast.ambient",
            "entity.happy_ghast.hurt",
            "entity.happy_ghast.death",
        ]
    }
}
/// `getSoundVolume`: 4 for adults, 1 for ghastlings.
pub fn volume(m: &Mob) -> f32 {
    if m.is_baby() { 1.0 } else { 4.0 }
}
/// `HappyGhastModel.setupAnim`: the tentacles, and the body squeezed under a harness.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    // HappyGhastHarnessModel.setupAnim: the goggles down while ridden
    // (vanilla's literal -0.7854, not -PI/4).
    #[allow(clippy::approx_constant)]
    if p.has("goggles") {
        let g = p.get("goggles");
        if m.rider.is_some() {
            g[3] = 0.0;
            g[1] = 14.0;
        } else {
            g[3] = -0.7854;
            g[1] = 9.0;
        }
    }
    if wearing_harness(m) {
        let b = p.get("body");
        b[6] = 0.9375;
        b[7] = 0.9375;
        b[8] = 0.9375;
    }
    super::nether::ghast_anim(s, p);
}
/// Texture of the harness layer for the worn item.
pub fn harness_texture(item: &str) -> Option<String> {
    let name = item.strip_prefix("minecraft:")?;
    name.ends_with("_harness")
        .then(|| format!("entity/equipment/happy_ghast_body/{name}"))
}
/// Events for the player getting on or off.
pub fn goggles(down: bool, m: &Mob) -> MobEvent {
    MobEvent::Sound(
        if down {
            "entity.happy_ghast.harness_goggles_down"
        } else {
            "entity.happy_ghast.harness_goggles_up"
        },
        m.center(),
        1.0,
        1.0,
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_collision_matches_vanilla_top_and_still_timeout_rules() {
        assert!(player_collision(true, false, false, 84.0, 84.0));
        assert!(player_collision(true, false, false, 85.0, 84.0));
        assert!(!player_collision(true, false, false, 83.99, 84.0));
        assert!(player_collision(true, false, true, 83.99, 84.0));
        assert!(!player_collision(true, true, true, 84.0, 84.0));
        assert!(!player_collision(false, false, true, 84.0, 84.0));
    }

    #[test]
    fn a_harness_names_its_texture() {
        assert_eq!(
            harness_texture("minecraft:red_harness").as_deref(),
            Some("entity/equipment/happy_ghast_body/red_harness")
        );
        assert_eq!(harness_texture("minecraft:saddle"), None);
    }
    #[test]
    fn travel_speed_is_five_thirds_of_the_flying_speed() {
        assert!((f64::from(AIR_SPEED) - FLYING_SPEED * 5.0 / 3.0).abs() < 1e-7);
    }
}
