//! Breezes (`Breeze`): the poses and their animations, the long jump's
//! arithmetic, the sounds of `Breeze.tick` and `BreezeModel`'s layers. The
//! brain (`BreezeAi` and its behaviours) is in `monster_brain`.
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
    keyframes::Animation,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    jmath,
    random::RandomSource,
};
use std::collections::HashMap;

// `variant3` is the pose, `variant` the pose the animations last saw,
// `vector[2]` `soundTick`. AnimState states: idle, shoot, slide, slide back,
// inhale, long jump.
pub const STANDING: u8 = 0;
pub const SHOOTING: u8 = 1;
pub const INHALING: u8 = 2;
pub const LONG_JUMPING: u8 = 3;
pub const SLIDING: u8 = 4;

const ALLOWED_ANGLES: [i32; 5] = [40, 55, 60, 75, 80];

/// `Breeze`'s constructor: no goals, the brain thinks (`monster_brain`).
pub fn register(m: &mut Mob) {
    m.set_malus(super::path::PathType::OnTopOfTrapdoor, -1.0);
    m.set_malus(super::path::PathType::Fire, -1.0);
}
pub fn pose(m: &Mob) -> u8 {
    m.extra.variant3
}
pub(super) fn set_pose(m: &mut Mob, pose: u8) {
    m.extra.variant3 = pose;
}
/// `Pose.id` of the pose (`sync::pose_id`), and its inverse.
pub fn pose_id(m: &Mob) -> u8 {
    match pose(m) {
        SHOOTING => 16,
        INHALING => 17,
        LONG_JUMPING => 6,
        SLIDING => 15,
        _ => 0,
    }
}
pub(super) fn set_pose_id(m: &mut Mob, id: u8) {
    set_pose(
        m,
        match id {
            16 => SHOOTING,
            17 => INHALING,
            6 => LONG_JUMPING,
            15 => SLIDING,
            _ => STANDING,
        },
    );
}
/// `Breeze.getFiringYPosition`.
pub fn firing_y(m: &Mob) -> f64 {
    m.position.y + f64::from(m.height()) / 2.0 + f64::from(0.3_f32)
}
/// `LongJump.canJumpFromCurrentPosition`: not on honey, and four blocks of
/// air or water above.
pub(super) fn can_jump_from_here(m: &Mob, w: &MobWorld) -> bool {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    if w.block_name(x, y, z) == "minecraft:honey_block" {
        return false;
    }
    (1..=4).all(|i| {
        w.facts.is_air(w.world.block(x, y + i, z))
            || w.fluid(x, y + i, z) == crate::blocks::Fluid::Water
    })
}
/// `LongJump.snapToSurface`: the block above where a clip ten blocks down
/// hits, else ten blocks up.
pub(super) fn snap_to_surface(w: &MobWorld, p: Vec3) -> Option<[i32; 3]> {
    let above = |hit: Vec3| {
        [
            hit.x.floor() as i32,
            hit.y.floor() as i32 + 1,
            hit.z.floor() as i32,
        ]
    };
    if let Some(hit) = w.clip_block(p, Vec3::new(p.x, p.y - 10.0, p.z)) {
        return Some(above(hit));
    }
    w.clip_block(p, Vec3::new(p.x, p.y + 10.0, p.z)).map(above)
}
/// `EntityType.isBlockDangerous` for the breeze (not fire immune).
pub(super) fn dangerous_block(name: &str) -> bool {
    matches!(
        name,
        "minecraft:fire"
            | "minecraft:soul_fire"
            | "minecraft:lava"
            | "minecraft:magma_block"
            | "minecraft:lava_cauldron"
            | "minecraft:wither_rose"
            | "minecraft:sweet_berry_bush"
            | "minecraft:cactus"
            | "minecraft:powder_snow"
    )
}
/// `LongJumpUtil.calculateJumpVectorForAngle`; with a world, the arc is
/// checked for collisions (`checkCollision`).
pub(super) fn jump_vector(
    m: &Mob,
    target: Vec3,
    max_velocity: f32,
    angle: i32,
    collide: Option<&MobWorld>,
) -> Option<Vec3> {
    let pos = m.position;
    let plane = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z)
        .normalize()
        .scale(0.5);
    let aim = target.subtract(plane);
    let d = aim.subtract(pos);
    let angrad = angle as f32 * std::f32::consts::PI / 180.0;
    // Math.atan2 is StrictMath.atan2 (fdlibm).
    let xz = horizon_core::geometry::trig::fdlibm_atan2(d.z, d.x);
    let r2 = d.x * d.x + 0.0 + d.z * d.z;
    let r = r2.sqrt();
    let y = d.y;
    let g = m.gravity();
    let sin2 = jmath::sin(f64::from(2.0 * angrad));
    let cos_sq = jmath::pow(jmath::cos(f64::from(angrad)), 2.0);
    let (sin, cos) = (jmath::sin(f64::from(angrad)), jmath::cos(f64::from(angrad)));
    let v0sq = r2 * g / (r * sin2 - 2.0 * y * cos_sq);
    if v0sq < 0.0 {
        return None;
    }
    let v0 = v0sq.sqrt();
    if v0 > f64::from(max_velocity) {
        return None;
    }
    let (v0r, v0y) = (v0 * cos, v0 * sin);
    if let Some(w) = collide {
        // `ceil(r / v0r) * 2` points along the arc, each step clear of blocks.
        let samples = (r / v0r).ceil() as i32 * 2;
        let (sin_xz, cos_xz) = (jmath::sin(xz), jmath::cos(xz));
        let mut ri = 0.0;
        let mut previous: Option<Vec3> = None;
        for _ in 0..samples - 1 {
            ri += r / f64::from(samples);
            let yi = sin / cos * ri - jmath::pow(ri, 2.0) * g / (2.0 * v0sq * jmath::pow(cos, 2.0));
            let p = Vec3::new(pos.x + ri * cos_xz, pos.y + yi, pos.z + ri * sin_xz);
            if previous.is_some_and(|q| !clear_transition(m, w, q, p)) {
                return None;
            }
            previous = Some(p);
        }
    }
    Some(Vec3::new(v0r * jmath::cos(xz), v0y, v0r * jmath::sin(xz)).scale(f64::from(0.95_f32)))
}
/// `LongJumpUtil.isClearTransition`: the box of `Pose.LONG_JUMPING` fits at
/// points no further apart than nine tenths of its smaller side.
fn clear_transition(m: &Mob, w: &MobWorld, from: Vec3, to: Vec3) -> bool {
    let (width, height, _) = m.awake_dims_in(true);
    let (width, height) = (width as f32, height as f32);
    let d = to.subtract(from);
    let min = f64::from(width.min(height));
    let checks = (d.length() / min).ceil() as i32;
    let step = d.normalize().scale(min * f64::from(0.9_f32));
    let half = f64::from(width) / 2.0;
    let mut p = from;
    (0..checks).all(|i| {
        p = if i == checks - 1 {
            to
        } else {
            p.add_vector(step)
        };
        // `EntityDimensions.makeBoundingBox`.
        w.no_collision(Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + f64::from(height), p.z + half),
        ))
    })
}
/// `LongJump.calculateOptimalJumpVector`: the allowed angles shuffled, the
/// first that reaches the target.
pub(super) fn optimal_jump_vector(m: &mut Mob, target: Vec3) -> Option<Vec3> {
    let mut angles = ALLOWED_ANGLES;
    for i in (2..=angles.len()).rev() {
        let j = m.random.next_int_bound(i as i32).unwrap_or(0) as usize;
        angles.swap(i - 1, j);
    }
    let max = 0.058_333_334_f32 * m.attrs.follow_range as f32;
    let v = angles
        .iter()
        .find_map(|&a| jump_vector(m, target, max, a, None))?;
    if m.has_effect("minecraft:jump_boost") {
        let amp = m
            .extra
            .effects
            .iter()
            .find(|e| e.0 == "minecraft:jump_boost")
            .map_or(0, |e| e.2);
        // LivingEntity.getJumpBoostPower
        let boost = f64::from(0.1_f32 * (amp + 1) as f32);
        return Some(v.add_vector(Vec3::new(0.0, v.normalize().y * boost, 0.0)));
    }
    Some(v)
}
/// Whether `Breeze.shouldDiscardFriction` applies (a long jump in the air).
pub fn discards_friction(m: &Mob) -> bool {
    m.species() == Species::Breeze && pose(m) == LONG_JUMPING
}
/// The random draws of `Breeze.tick` before `super.tick()`: the ground
/// particle count (standing, shooting or inhaling) and the whirl sound clock
/// (`playWhirlSound` is a local sound, heard on the client only).
pub fn tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if matches!(pose(m), SHOOTING | INHALING | STANDING) {
        // emitGroundParticles(1 + random.nextInt(1))
        let _ = m.random.next_int_bound(1);
    }
    if m.extra.vector[2] == 0.0 {
        m.extra.vector[2] = f64::from(m.random.next_int_between_inclusive(1, 80).unwrap_or(1));
    } else {
        m.extra.vector[2] -= 1.0;
    }
    if m.extra.vector[2] == 0.0 {
        let pitch = 0.7 + 0.4 * m.random.next_float();
        let volume = 0.8 + 0.2 * m.random.next_float();
        events.push(MobEvent::LocalSound(
            "entity.breeze.whirl",
            [m.position.x, m.position.y, m.position.z],
            volume,
            pitch,
        ));
    }
}
/// The client's pose-driven animation states (`onSyncedDataUpdated` and
/// `Breeze.tick`).
pub fn ai_step(m: &mut Mob) {
    let age = m.age as f32;
    let current = pose(m);
    if current != m.extra.variant {
        // resetAnimations, then the pose's own animation.
        let st = &mut m.anim.states;
        st[1].stop();
        st[0].stop();
        st[4].stop();
        st[5].stop();
        match current {
            SHOOTING => st[1].start_if_stopped(age),
            INHALING => st[4].start_if_stopped(age),
            SLIDING => st[2].start_if_stopped(age),
            _ => {}
        }
        m.extra.variant = current;
    }
    if current == LONG_JUMPING {
        m.anim.states[5].start_if_stopped(age);
    }
    m.anim.states[0].start_if_stopped(age);
    if current != SLIDING && m.anim.states[2].started.is_some() {
        m.anim.states[3].start(age);
        m.anim.states[2].stop();
    }
}
/// `Breeze.playAmbientSound` (a local sound at volume and pitch 1) and its
/// sound selection.
pub fn ambient(m: &Mob) -> Option<&'static str> {
    if m.ai.target.is_some() && m.on_ground {
        return None;
    }
    Some(if m.on_ground {
        "entity.breeze.idle_ground"
    } else {
        "entity.breeze.idle_air"
    })
}
/// `BreezeModel.setupAnim`: the six keyframe animations.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    let age = s.age;
    for (state, name) in [
        (&m.anim.states[0], "IDLE"),
        (&m.anim.states[1], "SHOOT"),
        (&m.anim.states[2], "SLIDE"),
        (&m.anim.states[3], "SLIDE_BACK"),
        (&m.anim.states[4], "INHALE"),
        (&m.anim.states[5], "JUMP"),
    ] {
        if let (Some(ms), Some(a)) = (
            state.millis(age),
            animations.get(&format!("BreezeAnimation.{name}")),
        ) {
            a.apply(ms, 1.0, p);
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn the_highest_arc_reaches_a_target_within_range() {
        // Angle 80 degrees: v0^2 = r^2 g / (r sin 2a - 2 y cos^2 a).
        let g = 0.08;
        let (r, y): (f64, f64) = (10.0, 0.0);
        let a = 60.0_f64.to_radians();
        let v0sq = r * r * g / (r * (2.0 * a).sin() - 2.0 * y * a.cos().powi(2));
        assert!(v0sq.sqrt() < 1.4);
    }
}
