//! Mob controls: `MoveControl` (with the flying and smooth-swimming
//! variants), `LookControl`, `JumpControl` and `BodyRotationControl`.
use super::{Mob, MobWorld, Species, living::wrap};
use horizon_core::geometry::trig;

/// `2.5000003E-7F` widened: the squared distance controls treat as arrived.
const ARRIVED: f64 = 2.500_000_3e-7_f32 as f64;
/// `1.0E-5F` widened.
pub(super) const EPSILON: f64 = 1.0e-5_f32 as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Operation {
    #[default]
    Wait,
    MoveTo,
    Strafe,
    Jumping,
}
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MoveKind {
    #[default]
    Ground,
    /// FlyingMoveControl(maxTurn, hoversInPlace)
    Flying { max_turn: f32, hovers: bool },
    /// SmoothSwimmingMoveControl(maxTurnX, maxTurnY, inWater, outsideWater, applyGravity)
    SmoothSwimming {
        max_turn_x: f32,
        max_turn_y: f32,
        in_water: f32,
        outside: f32,
        gravity: bool,
    },
    /// Goals drive the mob directly (slimes, ghasts, squid, fish, bats, phantoms).
    Custom,
}
#[derive(Clone, Debug, Default)]
pub struct MoveControl {
    pub kind: MoveKind,
    pub wanted: [f64; 3],
    pub speed_modifier: f64,
    pub strafe_forwards: f32,
    pub strafe_right: f32,
    pub operation: Operation,
}
impl MoveControl {
    pub fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }
    pub fn set_wanted(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.wanted = [x, y, z];
        self.speed_modifier = speed;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }
    pub fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }
}
/// MoveControl.rotlerp: turn at most `max` degrees, result in [0, 360].
pub fn rotlerp(a: f32, b: f32, max: f32) -> f32 {
    let diff = wrap(b - a).clamp(-max, max);
    let mut r = a + diff;
    if r < 0.0 {
        r += 360.0;
    } else if r > 360.0 {
        r -= 360.0;
    }
    r
}
/// Control.rotateTowards.
pub fn rotate_towards(from: f32, to: f32, max: f32) -> f32 {
    from + wrap(to - from).clamp(-max, max)
}
/// Mth.rotateIfNecessary.
pub fn rotate_if_necessary(base: f32, target: f32, max: f32) -> f32 {
    let d = wrap(target - base).clamp(-max, max);
    target - d
}
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LookKind {
    #[default]
    Default,
    /// SmoothSwimmingLookControl(maxYRotFromCenter)
    SmoothSwimming(f32),
    /// Look controls that never reset pitch (e.g. ghasts keep their own).
    Fixed,
    /// `HappyGhast.HappyGhastLookControl`.
    HappyGhast,
    /// `Shulker.ShulkerLookControl`: the yaw turns in the attach face's
    /// plane and the head is never clamped to the body.
    Shulker,
}
#[derive(Clone, Debug, Default)]
pub struct LookControl {
    pub kind: LookKind,
    pub wanted: [f64; 3],
    pub y_max_rot_speed: f32,
    pub x_max_rot_angle: f32,
    pub cooldown: i32,
}
impl LookControl {
    pub fn set_look_at(&mut self, at: [f64; 3], y_speed: f32, x_max: f32) {
        self.wanted = at;
        self.y_max_rot_speed = y_speed;
        self.x_max_rot_angle = x_max;
        self.cooldown = 2;
    }
}
#[derive(Clone, Debug, Default)]
pub struct BodyRotation {
    head_stable_time: i32,
    last_stable_head: f32,
}
impl Mob {
    /// `Mob.getMaxHeadXRot`, with `Wolf`'s 20 degrees while sitting.
    pub fn max_head_x_rot(&self) -> f32 {
        if self.species() == Species::Wolf && self.extra.sitting {
            20.0
        } else {
            self.kind.max_head_x_rot
        }
    }
    pub fn look_at(&mut self, at: [f64; 3]) {
        let (y, x) = (self.kind.head_rot_speed, self.max_head_x_rot());
        self.ai.look.set_look_at(at, y, x);
    }
    pub fn look_at_speed(&mut self, at: [f64; 3], y_speed: f32, x_max: f32) {
        self.ai.look.set_look_at(at, y_speed, x_max);
    }
    /// Mob.lookAt(entity, yMax, xMax): rotate the body yaw/pitch directly.
    pub fn turn_to(&mut self, at: [f64; 3], y_max: f32, x_max: f32) {
        let (dx, dz) = (at[0] - self.position.x, at[2] - self.position.z);
        let dy = at[1] - self.eye_y();
        let sd = (dx * dx + dz * dz).sqrt();
        let y_rot = trig::atan2_degrees(dz, dx) - 90.0;
        let x_rot = -trig::atan2_degrees(dy, sd);
        self.head_pitch = rotate_toward_max(self.head_pitch, x_rot, x_max);
        self.yaw = rotate_toward_max(self.yaw, y_rot, y_max);
    }
    /// MoveControl.tick (and the flying / smooth-swimming variants).
    pub fn move_control_tick(&mut self, w: &MobWorld, events: &mut Vec<super::MobEvent>) {
        if self.species() == super::Species::Rabbit {
            return super::rabbit::move_control(self, w);
        }
        // Fox.FoxMoveControl.tick: still while asleep, sitting or faceplanted.
        if self.species() == super::Species::Fox && !super::fox::can_move(self) {
            return;
        }
        // Panda.PandaMoveControl: only while it can act.
        if self.species() == super::Species::Panda
            && !super::panda::can_perform_action(self, w.thundering)
        {
            return;
        }
        if self.species() == super::Species::Turtle {
            return super::turtle::move_control(self);
        }
        let attr_speed = self.movement_speed();
        match self.ai.mov.kind {
            MoveKind::Custom => super::ai::custom_move_control(self, w, events),
            MoveKind::Ground => self.ground_move_tick(w, attr_speed),
            MoveKind::Flying { max_turn, hovers } => {
                let m = &mut self.ai.mov;
                if m.operation == Operation::MoveTo {
                    m.operation = Operation::Wait;
                    self.no_gravity = true;
                    let [wx, wy, wz] = self.ai.mov.wanted;
                    let (xd, yd, zd) = (
                        wx - self.position.x,
                        wy - self.position.y,
                        wz - self.position.z,
                    );
                    if xd * xd + yd * yd + zd * zd < ARRIVED {
                        self.ai.yya = 0.0;
                        self.ai.zza = 0.0;
                        return;
                    }
                    let y_rot = trig::atan2_degrees(zd, xd) - 90.0;
                    self.yaw = rotlerp(self.yaw, y_rot, 90.0);
                    let speed = (self.ai.mov.speed_modifier
                        * if self.on_ground {
                            attr_speed
                        } else {
                            self.kind.flying_speed
                        }) as f32;
                    self.set_speed(speed);
                    let sd = (xd * xd + zd * zd).sqrt();
                    if yd.abs() > EPSILON || sd.abs() > EPSILON {
                        let x_rot = -trig::atan2_degrees(yd, sd);
                        // Entity.setXRot clamps: a pitch rotlerp wraps to 359 becomes 90.
                        self.head_pitch =
                            (rotlerp(self.head_pitch, x_rot, max_turn) % 360.0).clamp(-90.0, 90.0);
                        self.ai.yya = if yd > 0.0 { speed } else { -speed };
                    }
                } else {
                    if !hovers {
                        self.no_gravity = false;
                    }
                    self.ai.yya = 0.0;
                    self.ai.zza = 0.0;
                }
            }
            MoveKind::SmoothSwimming {
                max_turn_x,
                max_turn_y,
                in_water,
                outside,
                gravity,
            } => {
                if gravity && self.fluids.in_water {
                    self.velocity.y += 0.005;
                }
                if self.ai.mov.operation == Operation::MoveTo && !self.ai.nav.is_done() {
                    let [wx, wy, wz] = self.ai.mov.wanted;
                    let (xd, yd, zd) = (
                        wx - self.position.x,
                        wy - self.position.y,
                        wz - self.position.z,
                    );
                    if xd * xd + yd * yd + zd * zd < ARRIVED {
                        self.ai.zza = 0.0;
                    } else {
                        let y_rot = trig::atan2_degrees(zd, xd) - 90.0;
                        self.yaw = rotlerp(self.yaw, y_rot, max_turn_y);
                        self.body_yaw = self.yaw;
                        self.head_yaw = self.yaw;
                        let speed = (self.ai.mov.speed_modifier * attr_speed) as f32;
                        if self.fluids.in_water {
                            self.ai.speed = speed * in_water;
                            let sd = (xd * xd + zd * zd).sqrt();
                            if yd.abs() > EPSILON || sd.abs() > EPSILON {
                                let x_rot = -trig::atan2_degrees(yd, sd);
                                let x_rot = wrap(x_rot).clamp(-max_turn_x, max_turn_x);
                                self.head_pitch = rotate_towards(self.head_pitch, x_rot, 5.0);
                            }
                            let angle = f64::from(self.head_pitch * trig::DEG_TO_RAD);
                            let (c, s) = (trig::cos(angle), trig::sin(angle));
                            self.ai.zza = c * speed;
                            self.ai.yya = -s * speed;
                        } else {
                            let left = wrap(self.yaw - y_rot).abs();
                            let factor = 1.0 - ((left - 10.0) / 50.0).clamp(0.0, 1.0);
                            self.set_speed(speed * outside * factor);
                        }
                    }
                } else {
                    self.ai.speed = 0.0;
                    self.ai.xxa = 0.0;
                    self.ai.yya = 0.0;
                    self.ai.zza = 0.0;
                }
            }
        }
    }
    /// Mob.setSpeed also sets the forward input.
    pub fn set_speed(&mut self, speed: f32) {
        self.ai.speed = speed;
        self.ai.zza = speed;
    }
    pub(crate) fn ground_move_tick(&mut self, w: &MobWorld, attr_speed: f64) {
        match self.ai.mov.operation {
            Operation::Strafe => {
                let speed = self.ai.mov.speed_modifier as f32 * attr_speed as f32;
                let (mut xa, mut za) = (self.ai.mov.strafe_forwards, self.ai.mov.strafe_right);
                let mut dist = (xa * xa + za * za).sqrt();
                if dist < 1.0 {
                    dist = 1.0;
                }
                dist = speed / dist;
                xa *= dist;
                za *= dist;
                let angle = f64::from(self.yaw * trig::DEG_TO_RAD);
                let (s, c) = (trig::sin(angle), trig::cos(angle));
                let dx = xa * c - za * s;
                let dz = za * c + xa * s;
                if !self.walkable(w, dx, dz) {
                    self.ai.mov.strafe_forwards = 1.0;
                    self.ai.mov.strafe_right = 0.0;
                }
                self.set_speed(speed);
                self.ai.zza = self.ai.mov.strafe_forwards;
                self.ai.xxa = self.ai.mov.strafe_right;
                self.ai.mov.operation = Operation::Wait;
            }
            Operation::MoveTo => {
                self.ai.mov.operation = Operation::Wait;
                let [wx, wy, wz] = self.ai.mov.wanted;
                let (xd, yd, zd) = (
                    wx - self.position.x,
                    wy - self.position.y,
                    wz - self.position.z,
                );
                if xd * xd + yd * yd + zd * zd < ARRIVED {
                    self.ai.zza = 0.0;
                    return;
                }
                let y_rot = trig::atan2_degrees(zd, xd) - 90.0;
                self.yaw = rotlerp(self.yaw, y_rot, 90.0);
                self.set_speed((self.ai.mov.speed_modifier * attr_speed) as f32);
                let (bx, by, bz) = (
                    self.position.x.floor() as i32,
                    self.position.y.floor() as i32,
                    self.position.z.floor() as i32,
                );
                let state = w.world.block(bx, by, bz);
                let top = w.data.state(state).top;
                let name = &w.facts.get(state).name;
                let door_or_fence = name.ends_with("_door") || name.ends_with("fence");
                if yd > self.kind.step_height
                    && xd * xd + zd * zd < f64::from(self.width().max(1.0))
                    || top > 0.0
                        && self.position.y < f64::from(top) + f64::from(by)
                        && !door_or_fence
                {
                    self.ai.jump = true;
                    self.ai.mov.operation = Operation::Jumping;
                }
            }
            Operation::Jumping => {
                self.set_speed((self.ai.mov.speed_modifier * attr_speed) as f32);
                if self.on_ground || self.is_in_liquid() {
                    self.ai.mov.operation = Operation::Wait;
                }
            }
            Operation::Wait => self.ai.zza = 0.0,
        }
    }
    fn walkable(&self, w: &MobWorld, dx: f32, dz: f32) -> bool {
        let x = (self.position.x + f64::from(dx)).floor() as i32;
        let z = (self.position.z + f64::from(dz)).floor() as i32;
        let y = self.position.y.floor() as i32;
        w.path_type_static(x, y, z) == super::path::PathType::Walkable
    }
    /// LookControl.tick (and SmoothSwimmingLookControl).
    pub fn look_control_tick(&mut self) {
        let look = self.ai.look.clone();
        let fox = self.species() == super::Species::Fox;
        // Fox.FoxLookControl: no turning in sleep.
        if fox && super::fox::flag(self, super::fox::SLEEPING) {
            return;
        }
        if look.kind == LookKind::HappyGhast {
            return super::happy_ghast::look_control(self);
        }
        let (x_d, y_d) = self.look_angles(look.wanted);
        match look.kind {
            LookKind::HappyGhast => {}
            LookKind::SmoothSwimming(max) => {
                if look.cooldown > 0 {
                    self.ai.look.cooldown -= 1;
                    if let Some(y) = y_d {
                        self.head_yaw =
                            rotate_towards(self.head_yaw, y + 20.0, look.y_max_rot_speed);
                    }
                    if let Some(x) = x_d {
                        self.head_pitch =
                            rotate_towards(self.head_pitch, x + 10.0, look.x_max_rot_angle);
                    }
                } else {
                    if self.ai.nav.is_done() {
                        self.head_pitch = rotate_towards(self.head_pitch, 0.0, 5.0);
                    }
                    self.head_yaw =
                        rotate_towards(self.head_yaw, self.body_yaw, look.y_max_rot_speed);
                }
                let diff = wrap(self.head_yaw - self.body_yaw);
                if diff < -max {
                    self.body_yaw -= 4.0;
                } else if diff > max {
                    self.body_yaw += 4.0;
                }
            }
            LookKind::Shulker => {
                // ShulkerLookControl: resetXRotOnTick, getXRotD 0, and the
                // yaw worked out in the attach face's plane (getYRotD).
                self.head_pitch = 0.0;
                if look.cooldown > 0 {
                    self.ai.look.cooldown -= 1;
                    if let Some(y) = super::shulker::look_yaw(self, look.wanted) {
                        self.head_yaw = rotate_towards(self.head_yaw, y, look.y_max_rot_speed);
                    }
                } else {
                    self.head_yaw = rotate_towards(self.head_yaw, self.body_yaw, 10.0);
                }
                // clampHeadRotationToBody does nothing for a shulker.
            }
            LookKind::Default | LookKind::Fixed => {
                // FoxLookControl.resetXRotOnTick keeps a stalking or pouncing pitch.
                if look.kind == LookKind::Default && (!fox || super::fox::resets_x_rot(self)) {
                    self.head_pitch = 0.0;
                }
                if look.cooldown > 0 {
                    self.ai.look.cooldown -= 1;
                    if let Some(y) = y_d {
                        self.head_yaw = rotate_towards(self.head_yaw, y, look.y_max_rot_speed);
                    }
                    if let Some(x) = x_d {
                        self.head_pitch = rotate_towards(self.head_pitch, x, look.x_max_rot_angle);
                    }
                } else {
                    self.head_yaw = rotate_towards(self.head_yaw, self.body_yaw, 10.0);
                }
                if !self.ai.nav.is_done() {
                    self.head_yaw =
                        rotate_if_necessary(self.head_yaw, self.body_yaw, self.kind.max_head_y_rot);
                }
            }
        }
        // Goat.setYHeadRot: the head is clamped to `getMaxHeadYRot` (15
        // degrees) from the body on every set, so a standing goat cannot
        // swivel it further than a walking one.
        if self.species() == Species::Goat {
            self.head_yaw =
                rotate_if_necessary(self.head_yaw, self.body_yaw, self.kind.max_head_y_rot);
        }
    }
    fn look_angles(&self, at: [f64; 3]) -> (Option<f32>, Option<f32>) {
        let (xd, yd, zd) = (
            at[0] - self.position.x,
            at[1] - self.eye_y(),
            at[2] - self.position.z,
        );
        let sd = (xd * xd + zd * zd).sqrt();
        let x = (yd.abs() > EPSILON || sd.abs() > EPSILON).then(|| -trig::atan2_degrees(yd, sd));
        let y =
            (zd.abs() > EPSILON || xd.abs() > EPSILON).then(|| trig::atan2_degrees(zd, xd) - 90.0);
        (x, y)
    }
    /// JumpControl.tick.
    pub fn jump_control_tick(&mut self) {
        if self.species() == super::Species::Rabbit {
            return super::rabbit::jump_control(self);
        }
        self.ai.jumping = self.ai.jump;
        self.ai.jump = false;
    }
    /// BodyRotationControl.clientTick (Mob.tickHeadTurn).
    pub fn body_rotation_tick(&mut self) {
        // Shulker.ShulkerBodyRotationControl.clientTick is empty: the body
        // stays at 0 (recreateFromPacket), only the head turns.
        if self.species() == super::Species::Shulker {
            self.body_yaw = 0.0;
            self.previous_body_yaw = 0.0;
            return;
        }
        let dx = self.position.x - self.previous.x;
        let dz = self.position.z - self.previous.z;
        let max = self.kind.max_head_y_rot;
        if dx * dx + dz * dz > ARRIVED {
            self.body_yaw = self.yaw;
            self.head_yaw = rotate_if_necessary(self.head_yaw, self.body_yaw, max);
            self.ai.body.last_stable_head = self.head_yaw;
            self.ai.body.head_stable_time = 0;
        } else if (self.head_yaw - self.ai.body.last_stable_head).abs() > 15.0 {
            self.ai.body.head_stable_time = 0;
            self.ai.body.last_stable_head = self.head_yaw;
            self.body_yaw = rotate_if_necessary(self.body_yaw, self.head_yaw, max);
        } else {
            self.ai.body.head_stable_time += 1;
            if self.ai.body.head_stable_time > 10 {
                let t = self.ai.body.head_stable_time - 10;
                let fraction = (t as f32 / 10.0).clamp(0.0, 1.0);
                self.body_yaw =
                    rotate_if_necessary(self.body_yaw, self.head_yaw, max * (1.0 - fraction));
            }
        }
    }
}
/// Entity.rotlerp used by Mob.lookAt (clamped turn, no wrap of the result).
fn rotate_toward_max(from: f32, to: f32, max: f32) -> f32 {
    from + wrap(to - from).clamp(-max, max)
}

#[cfg(test)]
mod goat_head_tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn mob(species: Species) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        // Mob.getMaxHeadYRot: 15 for a goat (Goat.getMaxHeadYRot), 75 otherwise.
        kind.max_head_y_rot = if species == Species::Goat { 15.0 } else { 75.0 };
        Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// `Goat.setYHeadRot`: whatever the look control asks for, a goat's head
    /// stays within `getMaxHeadYRot` (15 degrees) of its body, standing still
    /// or walking; another mob turns as far as it likes.
    #[test]
    fn a_goats_head_stays_within_fifteen_degrees_of_its_body() {
        let turned = |species: Species| {
            let mut m = mob(species);
            m.body_yaw = 0.0;
            // A target 40 blocks to the side: a quarter turn away.
            m.ai.look.kind = LookKind::Default;
            m.ai.look.wanted = [40.0, 1.0, 0.0];
            m.ai.look.y_max_rot_speed = 1000.0;
            m.ai.look.x_max_rot_angle = 40.0;
            m.ai.look.cooldown = 20;
            m.look_control_tick();
            wrap(m.head_yaw - m.body_yaw)
        };
        let goat = turned(Species::Goat);
        assert!(
            goat.abs() <= 15.0 + 1.0e-3,
            "the goat's head turned {goat} degrees from its body"
        );
        assert!(
            turned(Species::Cow).abs() > 15.0,
            "a cow is not held to the goat's 15 degrees"
        );
    }
}
