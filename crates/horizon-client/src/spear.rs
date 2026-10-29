//! Spears: `KineticWeapon` (what holding the use button does: the speed
//! conditions of a stab, `Item.Properties.spear`'s numbers) and
//! `SpearAnimations` (how the spear is held in first and third person while
//! it charges).
use crate::first_person::SwingAnimation;
use glam::{Mat4, Vec3};
use horizon_core::{geometry::trig::Mth, jmath::pow};
use std::f32::consts::PI;

/// `KineticWeapon.Condition`.
#[derive(Clone, Copy, Debug)]
pub struct Condition {
    pub max_duration_ticks: i32,
    pub min_speed: f32,
    pub min_relative_speed: f32,
}
impl Condition {
    pub fn test(
        self,
        ticks_used: i32,
        attacker_speed: f64,
        relative_speed: f64,
        entity_factor: f64,
    ) -> bool {
        ticks_used <= self.max_duration_ticks
            && attacker_speed >= f64::from(self.min_speed) * entity_factor
            && relative_speed >= f64::from(self.min_relative_speed) * entity_factor
    }
}

/// `KineticWeapon`: a spear's charge.
#[derive(Clone, Copy, Debug)]
pub struct Kinetic {
    pub contact_cooldown_ticks: i32,
    pub delay_ticks: i32,
    pub dismount: Condition,
    pub knockback: Condition,
    pub damage: Condition,
    pub forward_movement: f32,
    pub damage_multiplier: f32,
    pub sound: &'static str,
    pub hit_sound: &'static str,
    /// The `attack_animation` duration, and the `piercing_weapon` sound of a
    /// jab.
    pub attack_ticks: i32,
    pub attack_sound: &'static str,
}

/// `AttackRange` of a spear: the reach of its stabs.
pub struct Reach {
    pub min: f32,
    pub max: f32,
    pub min_creative: f32,
    pub max_creative: f32,
    pub hitbox_margin: f32,
    pub mob_factor: f32,
}
pub const REACH: Reach = Reach {
    min: 2.0,
    max: 4.5,
    min_creative: 2.0,
    max_creative: 6.5,
    hitbox_margin: 0.125,
    mob_factor: 0.5,
};

impl Kinetic {
    /// The `kinetic_weapon` of a spear item (`Item.Properties.spear`).
    pub fn of(item: &str) -> Option<Self> {
        // (attack duration, damage multiplier, delay, dismount time and
        // speed, knockback time and speed, damage time and speed): seconds
        // and blocks per second.
        let (attack, multiplier, delay, dismount, knockback, damage) = match item {
            "minecraft:wooden_spear" => (0.65, 0.7, 0.75, (5.0, 14.0), (10.0, 5.1), (15.0, 4.6)),
            "minecraft:stone_spear" => (0.75, 0.82, 0.7, (4.5, 13.0), (9.0, 5.1), (13.75, 4.6)),
            "minecraft:copper_spear" => (0.85, 0.82, 0.65, (4.0, 12.0), (8.25, 5.1), (12.5, 4.6)),
            "minecraft:iron_spear" => (0.95, 0.95, 0.6, (2.5, 11.0), (6.75, 5.1), (11.25, 4.6)),
            "minecraft:golden_spear" => (0.95, 0.7, 0.7, (3.5, 13.0), (8.5, 5.1), (13.75, 4.6)),
            "minecraft:diamond_spear" => (1.05, 1.075, 0.5, (3.0, 10.0), (6.5, 5.1), (10.0, 4.6)),
            "minecraft:netherite_spear" => (1.15, 1.2, 0.4, (2.5, 9.0), (5.5, 5.1), (8.75, 4.6)),
            _ => return None,
        };
        let wood = item == "minecraft:wooden_spear";
        let ticks = |seconds: f32| (seconds * 20.0) as i32;
        Some(Self {
            contact_cooldown_ticks: 10,
            delay_ticks: ticks(delay),
            // `Condition.ofAttackerSpeed`, and `ofRelativeSpeed` for the damage.
            dismount: Condition {
                max_duration_ticks: ticks(dismount.0),
                min_speed: dismount.1,
                min_relative_speed: 0.0,
            },
            knockback: Condition {
                max_duration_ticks: ticks(knockback.0),
                min_speed: knockback.1,
                min_relative_speed: 0.0,
            },
            damage: Condition {
                max_duration_ticks: ticks(damage.0),
                min_speed: 0.0,
                min_relative_speed: damage.1,
            },
            forward_movement: 0.38,
            damage_multiplier: multiplier,
            sound: if wood {
                "minecraft:item.spear_wood.use"
            } else {
                "minecraft:item.spear.use"
            },
            hit_sound: if wood {
                "minecraft:item.spear_wood.hit"
            } else {
                "minecraft:item.spear.hit"
            },
            attack_ticks: ticks(attack),
            attack_sound: if wood {
                "minecraft:item.spear_wood.attack"
            } else {
                "minecraft:item.spear.attack"
            },
        })
    }

    /// The spear's `attack_animation`: a STAB (`SwingAnimationType` 2) as long
    /// as its attack duration.
    pub fn attack_animation(&self) -> SwingAnimation {
        SwingAnimation {
            kind: 2,
            duration: self.attack_ticks,
        }
    }
}

/// `KineticWeapon.HIT_FEEDBACK_TICKS`.
pub const HIT_FEEDBACK_TICKS: i64 = 10;

/// `Mth.clamp` on floats (a NaN stays one).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { v.min(hi) }
}
/// `SpearAnimations.progress`.
fn progress(time: f32, start: f32, end: f32) -> f32 {
    clamp((time - start) / (end - start), 0.0, 1.0)
}
fn square(x: f32) -> f32 {
    x * x
}
fn cube(x: f32) -> f32 {
    x * x * x
}
fn p(x: f32, y: f64) -> f64 {
    pow(f64::from(x), y)
}

// `net.minecraft.util.Ease`.
fn in_quad(x: f32) -> f32 {
    x * x
}
fn out_quart(x: f32) -> f32 {
    1.0 - square(square(1.0 - x))
}
fn out_cubic(x: f32) -> f32 {
    1.0 - cube(1.0 - x)
}
fn out_back(x: f32) -> f32 {
    1.0 + 2.70158 * cube(x - 1.0) + 1.70158 * square(x - 1.0)
}
fn in_out_sine(x: f32) -> f32 {
    -((PI * x).mcos() - 1.0) / 2.0
}
fn in_out_expo(x: f32) -> f32 {
    if x < 0.5 {
        if x == 0.0 {
            0.0
        } else {
            (p(2.0, f64::from(20.0 * x - 10.0)) / 2.0) as f32
        }
    } else if x == 1.0 {
        1.0
    } else {
        ((2.0 - p(2.0, f64::from(-20.0 * x + 10.0))) / 2.0) as f32
    }
}
fn in_out_back(x: f32) -> f32 {
    if x < 0.5 {
        return 4.0 * x * x * (7.189_819 * x - 2.594_909_4) / 2.0;
    }
    let dt = 2.0 * x - 2.0;
    (dt * dt * (3.594_909_4 * dt + 2.594_909_4) + 2.0) / 2.0
}
fn in_out_elastic(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    let sin = (f64::from(20.0 * x - 11.125) * f64::from(PI * 4.0 / 9.0)).sin();
    if x < 0.5 {
        (-(p(2.0, f64::from(20.0 * x - 10.0)) * sin) / 2.0) as f32
    } else {
        (p(2.0, f64::from(-20.0 * x + 10.0)) * sin / 2.0 + 1.0) as f32
    }
}
fn out_circ(x: f32) -> f32 {
    f64::from(1.0 - square(x - 1.0)).sqrt() as f32
}
fn in_circ(x: f32) -> f32 {
    (-f64::from(1.0 - x * x).sqrt()) as f32 + 1.0
}

/// `SpearAnimations.UseParams`.
struct UseParams {
    raise: f32,
    raise_start: f32,
    raise_middle: f32,
    raise_end: f32,
    sway: f32,
    lower: f32,
    raise_back: f32,
    sway_intensity: f32,
    sway_slow: f32,
    sway_fast: f32,
}
impl UseParams {
    fn of(k: &Kinetic, time: f32) -> Self {
        let finish_raising = k.delay_ticks;
        let finish_swaying = k.dismount.max_duration_ticks + finish_raising;
        let start_swaying = finish_swaying - 20;
        let finish_lowering = k.knockback.max_duration_ticks + finish_raising;
        let start_lowering = finish_lowering - 40;
        let finish_raising_back = k.damage.max_duration_ticks + finish_raising;
        let raise = progress(time, 0.0, finish_raising as f32);
        let sway = progress(time, start_swaying as f32, finish_swaying as f32);
        let lower = out_cubic(in_out_elastic(progress(
            time - 20.0,
            start_lowering as f32,
            finish_lowering as f32,
        )));
        let raise_back = progress(
            time,
            (finish_raising_back - 5) as f32,
            finish_raising_back as f32,
        );
        let sway_intensity = 2.0 * out_circ(sway) - 2.0 * in_circ(raise_back);
        let degrees = PI / 180.0;
        Self {
            raise,
            raise_start: progress(raise, 0.0, 0.5),
            raise_middle: progress(raise, 0.5, 0.8),
            raise_end: progress(raise, 0.8, 1.0),
            sway,
            lower,
            raise_back,
            sway_intensity,
            sway_slow: (time * 19.0 * degrees).msin() * sway_intensity,
            sway_fast: (time * 30.0 * degrees).msin() * sway_intensity,
        }
    }
}

/// `SpearAnimations.hitFeedbackAmount`.
fn hit_feedback(ticks_since: f32) -> f32 {
    0.4 * (out_quart(progress(ticks_since, 1.0, 3.0))
        - in_out_sine(progress(ticks_since, 3.0, 10.0)))
}

/// `PoseStack.rotateAround`.
fn rotate_around(rotation: Mat4, about: Vec3) -> Mat4 {
    Mat4::from_translation(about) * rotation * Mat4::from_translation(-about)
}

/// `SpearAnimations.firstPersonUse`, after the hand's own
/// `translate(invert * 0.56, -0.52, -0.72)`.
pub fn first_person_use(k: &Kinetic, ticks_since_hit: f32, time_held: f32, right: bool) -> Mat4 {
    let u = UseParams::of(k, time_held);
    let invert = if right { 1.0 } else { -1.0 };
    let degrees = |d: f32| d.to_radians();
    let mut m = Mat4::from_translation(Vec3::new(
        invert * (u.raise * 0.15 + u.raise_end * -0.05 + u.sway * -0.1 + u.sway_slow * 0.005),
        u.raise * -0.075 + u.raise_middle * 0.075 + u.sway_fast * 0.01,
        u.raise_start * 0.05 + u.raise_end * -0.05 + u.sway_slow * 0.005,
    ));
    m *= rotate_around(
        Mat4::from_rotation_x(degrees(
            -65.0 * in_out_back(u.raise) - 35.0 * u.lower
                + 100.0 * u.raise_back
                + -0.5 * u.sway_fast,
        )),
        Vec3::new(0.0, 0.1, 0.0),
    );
    // Axis.YN: a turn the other way round.
    m *= rotate_around(
        Mat4::from_rotation_y(-degrees(
            invert * (-90.0 * progress(u.raise, 0.5, 0.55) + 90.0 * u.sway + 2.0 * u.sway_slow),
        )),
        Vec3::new(invert * 0.15, 0.0, 0.0),
    );
    m * Mat4::from_translation(Vec3::new(0.0, -hit_feedback(ticks_since_hit), 0.0))
}

/// `SpearAnimations.thirdPersonHandUse`: the angles (x, y and the z added to
/// the arm's own) of the arm holding a spear. `ticks_using` is the time the
/// item has been in use by this arm (0 when it is not).
pub fn third_person_arm(
    k: &Kinetic,
    ticks_using: f32,
    right: bool,
    head: (f32, f32),
    gliding_or_swimming: bool,
) -> (f32, f32, f32) {
    let invert = if right { 1.0 } else { -1.0 };
    let degrees = PI / 180.0;
    let mut y = -0.1 * invert + head.1;
    let mut x = -PI / 2.0 + head.0 + 0.8;
    if gliding_or_swimming {
        x -= 0.959_931_1;
    }
    y = degrees * clamp(180.0 / PI * y, -60.0, 60.0);
    x = degrees * clamp(180.0 / PI * x, -120.0, 30.0);
    let mut z = 0.0;
    if ticks_using > 0.0 {
        let u = UseParams::of(k, ticks_using);
        y += -invert * u.sway_fast * degrees * u.sway_intensity;
        z = -invert * u.sway_slow * degrees * u.sway_intensity * 0.5;
        x += degrees
            * (-40.0 * u.raise_start
                + 30.0 * u.raise_middle
                + -20.0 * u.raise_end
                + 20.0 * u.lower
                + 10.0 * u.raise_back
                + 0.6 * u.sway_slow * u.sway_intensity);
    }
    (x, y, z)
}

/// `SpearAnimations.thirdPersonUseItem`: the spear's own pose in the hand
/// held for `time_held` ticks, `swing` into the current swing.
pub fn third_person_use_item(
    k: &Kinetic,
    time_held: f32,
    swing: f32,
    ticks_since_hit: f32,
    right: bool,
) -> Mat4 {
    if time_held == 0.0 {
        return Mat4::IDENTITY;
    }
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    let u = UseParams::of(k, time_held);
    let invert = if right { 1.0 } else { -1.0 };
    let raise_modified = 1.0 - out_back(1.0 - u.raise);
    let hit = hit_feedback(ticks_since_hit);
    let degrees = |d: f32| d.to_radians();
    Mat4::from_translation(Vec3::new(
        0.0,
        -hit * 0.4,
        -k.forward_movement * (raise_modified - u.raise_back) + hit,
    )) * rotate_around(
        Mat4::from_rotation_x(-degrees(
            70.0 * (u.raise - u.raise_back) - 40.0 * (attack - retract),
        )),
        Vec3::new(0.0, -0.031_25, 0.125),
    ) * rotate_around(
        Mat4::from_rotation_y(degrees(
            invert * 90.0 * (u.raise - u.sway + 3.0 * retract + attack),
        )),
        Vec3::new(0.0, 0.0, 0.125),
    )
}

/// `SpearAnimations.firstPersonAttack`, after `applyItemArmTransform`: the
/// jab (a STAB swing), `attack` into the swing.
pub fn first_person_attack(attack: f32, right: bool) -> Mat4 {
    let invert = if right { 1.0 } else { -1.0 };
    let start = in_out_sine(progress(attack, 0.0, 0.05));
    let middle = out_back(progress(attack, 0.05, 0.2));
    let end = in_out_expo(progress(attack, 0.4, 1.0));
    Mat4::from_translation(Vec3::new(
        invert * 0.1 * (start - middle),
        -0.075 * (start - end),
        0.65 * (start - middle),
    )) * Mat4::from_rotation_x((-70.0 * (start - end)).to_radians())
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.25 * (end - middle)))
}

/// `SpearAnimations.thirdPersonAttackHand`: what a STAB swing adds to the
/// swinging arm's `xRot`, `swing` into the swing.
pub fn third_person_attack_arm(swing: f32) -> f32 {
    let prepare = in_out_sine(progress(swing, 0.0, 0.05));
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    (90.0 * prepare - 120.0 * attack + 30.0 * retract).to_radians()
}

/// `SpearAnimations.thirdPersonAttackItem`: the item in the swinging hand
/// thrust out by a STAB swing, `forward` the main hand's `forward_movement`
/// (0 without a kinetic weapon there).
pub fn third_person_attack_item(swing: f32, forward: f32) -> Mat4 {
    if swing <= 0.0 {
        return Mat4::IDENTITY;
    }
    let attack = in_quad(progress(swing, 0.05, 0.2));
    let retract = in_out_expo(progress(swing, 0.4, 1.0));
    rotate_around(
        Mat4::from_rotation_x((-70.0 * (attack - retract)).to_radians()),
        Vec3::new(0.0, -0.125, 0.125),
    ) * Mat4::from_translation(Vec3::new(0.0, forward * (attack - retract), 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jab_starts_and_ends_at_rest() {
        assert_eq!(first_person_attack(0.0, true), Mat4::IDENTITY);
        assert!(first_person_attack(1.0, true).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        // Thrust out and turned mid-jab, mirrored in the left hand.
        let right = first_person_attack(0.2, true);
        assert!(!right.abs_diff_eq(Mat4::IDENTITY, 1e-3));
        let left = first_person_attack(0.2, false);
        assert!((left.w_axis.x + right.w_axis.x).abs() < 1e-6);
        assert_eq!(third_person_attack_item(0.0, 0.38), Mat4::IDENTITY);
        assert!(third_person_attack_item(1.0, 0.38).abs_diff_eq(Mat4::IDENTITY, 1e-5));
        // The arm draws back, thrusts forward, then returns.
        assert_eq!(third_person_attack_arm(0.0), 0.0);
        assert!((third_person_attack_arm(0.05) - 90f32.to_radians()).abs() < 1e-5);
        assert!(third_person_attack_arm(0.2) < 0.0);
        assert!(third_person_attack_arm(1.0).abs() < 1e-5);
    }
}
