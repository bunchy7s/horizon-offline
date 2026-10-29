//! The camera (`Camera.tick`, `tickFov`, `calculateFov`, `alignWithEntity`,
//! `getMaxZoom`, `setRotation`, `move`), view and hurt bobbing
//! (`GameRenderer.bobView`, `bobHurt`), the walk and bob history they read
//! (`ClientAvatarState`), and `AbstractClientPlayer.getFieldOfViewModifier`.
//!
//! The float arithmetic follows vanilla and JOML's default (non-FMA,
//! non-fast-math) code paths operation by operation.
use horizon_core::geometry::trig;

/// `(float) Math.PI`.
const PI: f32 = std::f32::consts::PI;
/// `(float) (Math.PI / 180.0)`.
const DEG_TO_RAD: f32 = trig::DEG_TO_RAD;

/// `Math.min(float, float)`: NaN wins and -0.0 is below 0.0.
pub fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else if a <= b {
        a
    } else {
        b
    }
}
/// `Math.max(float, float)`.
pub fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a >= b {
        a
    } else {
        b
    }
}
/// `Mth.lerp(float, float, float)`.
pub fn lerp(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}
/// `Mth.lerp(double, double, double)`.
pub fn lerp_d(alpha: f64, p0: f64, p1: f64) -> f64 {
    p0 + alpha * (p1 - p0)
}
/// `Mth.wrapDegrees(float)`.
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut normalized = angle % 360.0;
    if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized < -180.0 {
        normalized += 360.0;
    }
    normalized
}
/// `Mth.clamp(float, float, float)`.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else {
        java_min(value, max)
    }
}

/// JOML's float trigonometry (`org.joml.Math` without fast math).
pub mod joml {
    use super::PI;
    const TAU: f32 = std::f32::consts::TAU;
    const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

    /// `Math.sin(float)`: `java.lang.Math.sin` narrowed.
    pub fn sin(rad: f32) -> f32 {
        f64::from(rad).sin() as f32
    }
    /// `Math.cosFromSin(float, float)`.
    pub fn cos_from_sin(sin: f32, angle: f32) -> f32 {
        let cos = (1.0 - sin * sin).sqrt();
        let a = angle + HALF_PI;
        let mut b = a - (a / TAU) as i32 as f32 * TAU;
        if f64::from(b) < 0.0 {
            b += TAU;
        }
        if b >= PI { -cos } else { cos }
    }

    /// A JOML `Quaternionf` (x, y, z, w).
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Quat {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }
    impl Quat {
        pub const IDENTITY: Self = Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        /// `Quaternionf.rotationYXZ(angleY, angleX, angleZ)`.
        pub fn rotation_yxz(angle_y: f32, angle_x: f32, angle_z: f32) -> Self {
            let sx = sin(angle_x * 0.5);
            let cx = cos_from_sin(sx, angle_x * 0.5);
            let sy = sin(angle_y * 0.5);
            let cy = cos_from_sin(sy, angle_y * 0.5);
            let sz = sin(angle_z * 0.5);
            let cz = cos_from_sin(sz, angle_z * 0.5);
            let x = cy * sx;
            let y = sy * cx;
            let z = sy * sx;
            let w = cy * cx;
            Self {
                x: x * cz + y * sz,
                y: y * cz - x * sz,
                z: w * sz - z * cz,
                w: w * cz + z * sz,
            }
        }
        /// `Quaternionf.transform(x, y, z, dest)` (`Vector3f.rotate`).
        pub fn transform(&self, v: [f32; 3]) -> [f32; 3] {
            let [x, y, z] = v;
            let xx = self.x * self.x;
            let yy = self.y * self.y;
            let zz = self.z * self.z;
            let ww = self.w * self.w;
            let xy = self.x * self.y;
            let xz = self.x * self.z;
            let yz = self.y * self.z;
            let xw = self.x * self.w;
            let zw = self.z * self.w;
            let yw = self.y * self.w;
            let k = 1.0 / (xx + yy + zz + ww);
            [
                (xx - yy - zz + ww) * k * x + (2.0 * (xy - zw) * k * y + 2.0 * (xz + yw) * k * z),
                2.0 * (xy + zw) * k * x + ((yy - xx - zz + ww) * k * y + 2.0 * (yz - xw) * k * z),
                2.0 * (xz - yw) * k * x + (2.0 * (yz + xw) * k * y + (zz - xx - yy + ww) * k * z),
            ]
        }
    }

    const PERSPECTIVE: u8 = 1;
    const IDENTITY: u8 = 4;
    const TRANSLATION: u8 = 8;

    /// A JOML `Matrix4f` with its property flags, which select the code
    /// paths (and so the rounding) of later operations. `m[column * 4 + row]`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Mat4 {
        pub m: [f32; 16],
        pub properties: u8,
    }
    impl Default for Mat4 {
        fn default() -> Self {
            Self::IDENTITY
        }
    }
    impl Mat4 {
        /// `new Matrix4f()`.
        pub const IDENTITY: Self = Self {
            m: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            properties: 30,
        };
        fn get(&self, column: usize, row: usize) -> f32 {
            self.m[column * 4 + row]
        }
        fn set(&mut self, column: usize, row: usize, v: f32) {
            self.m[column * 4 + row] = v;
        }
        fn reset_unless_identity(&mut self) {
            if self.properties & IDENTITY == 0 {
                self.m = Self::IDENTITY.m;
            }
        }
        /// `Matrix4f.translation(x, y, z)`.
        pub fn translation(&mut self, x: f32, y: f32, z: f32) {
            self.reset_unless_identity();
            self.set(3, 0, x);
            self.set(3, 1, y);
            self.set(3, 2, z);
            self.properties = 26;
        }
        /// `Matrix4f.setTranslation(x, y, z)`.
        pub fn set_translation(&mut self, x: f32, y: f32, z: f32) {
            self.set(3, 0, x);
            self.set(3, 1, y);
            self.set(3, 2, z);
            self.properties &= !(PERSPECTIVE | IDENTITY);
        }
        /// `Matrix4f.translate(x, y, z)`.
        pub fn translate(&mut self, x: f32, y: f32, z: f32) {
            if self.properties & IDENTITY != 0 {
                self.translation(x, y, z);
                return;
            }
            for row in 0..4 {
                let v = self.get(0, row) * x
                    + (self.get(1, row) * y + (self.get(2, row) * z + self.get(3, row)));
                self.set(3, row, v);
            }
            self.properties &= !(PERSPECTIVE | IDENTITY);
        }
        fn rotation_axis(&mut self, axis: usize, ang: f32) {
            let sin = sin(ang);
            let cos = cos_from_sin(sin, ang);
            self.reset_unless_identity();
            match axis {
                0 => {
                    self.set(1, 1, cos);
                    self.set(1, 2, sin);
                    self.set(2, 1, -sin);
                    self.set(2, 2, cos);
                }
                1 => {
                    self.set(0, 0, cos);
                    self.set(0, 2, -sin);
                    self.set(2, 0, sin);
                    self.set(2, 2, cos);
                }
                _ => {
                    self.set(0, 0, cos);
                    self.set(0, 1, sin);
                    self.set(1, 0, -sin);
                    self.set(1, 1, cos);
                }
            }
            self.properties = 18;
        }
        /// `Matrix4f.rotateX/rotateY/rotateZ(ang)`, with the identity and
        /// pure-translation shortcuts.
        fn rotate_axis(&mut self, axis: usize, ang: f32) {
            if self.properties & IDENTITY != 0 {
                self.rotation_axis(axis, ang);
                return;
            }
            if self.properties & TRANSLATION != 0 {
                let (x, y, z) = (self.get(3, 0), self.get(3, 1), self.get(3, 2));
                self.rotation_axis(axis, ang);
                self.set_translation(x, y, z);
                return;
            }
            let sin = sin(ang);
            let cos = cos_from_sin(sin, ang);
            let s = *self;
            match axis {
                0 => {
                    for row in 0..4 {
                        let (l1, l2) = (s.get(1, row), s.get(2, row));
                        self.set(2, row, l1 * -sin + l2 * cos);
                        self.set(1, row, l1 * cos + l2 * sin);
                    }
                }
                1 => {
                    for row in 0..4 {
                        let (l0, l2) = (s.get(0, row), s.get(2, row));
                        self.set(0, row, l0 * cos + l2 * -sin);
                        self.set(2, row, l0 * sin + l2 * cos);
                    }
                }
                _ => {
                    // rotateTowardsXY(sin, cos).
                    let (dx, dy) = (sin, cos);
                    for row in 0..4 {
                        let (l0, l1) = (s.get(0, row), s.get(1, row));
                        self.set(0, row, l0 * dy + l1 * dx);
                        self.set(1, row, l0 * -dx + l1 * dy);
                    }
                }
            }
            self.properties &= !(PERSPECTIVE | IDENTITY | TRANSLATION);
        }
        pub fn rotate_x(&mut self, ang: f32) {
            self.rotate_axis(0, ang);
        }
        pub fn rotate_y(&mut self, ang: f32) {
            self.rotate_axis(1, ang);
        }
        pub fn rotate_z(&mut self, ang: f32) {
            // rotateTowardsXY also short-cuts on identity, but rotateZ has
            // already taken that branch.
            self.rotate_axis(2, ang);
        }
    }
}
use joml::{Mat4, Quat};

/// Camera-relative world view used by terrain, sky and the first-person hand.
pub fn view_matrix(yaw: f32, pitch: f32) -> glam::Mat4 {
    // Vanilla uses the conjugate of Camera.rotation, not a world-up look-at.
    let q = Quat::rotation_yxz(PI - yaw * DEG_TO_RAD, -pitch * DEG_TO_RAD, 0.0);
    glam::Mat4::from_quat(glam::Quat::from_xyzw(-q.x, -q.y, -q.z, q.w))
}

/// `CameraType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}
impl CameraType {
    pub fn is_first_person(self) -> bool {
        self == Self::FirstPerson
    }
    pub fn is_mirrored(self) -> bool {
        self == Self::ThirdPersonFront
    }
    /// `CameraType.cycle` (F5).
    pub fn cycle(self) -> Self {
        match self {
            Self::FirstPerson => Self::ThirdPersonBack,
            Self::ThirdPersonBack => Self::ThirdPersonFront,
            Self::ThirdPersonFront => Self::FirstPerson,
        }
    }
}

/// What `AbstractClientPlayer.getFieldOfViewModifier` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FovPlayer {
    /// `Abilities.flying`.
    pub flying: bool,
    /// `Abilities.getWalkingSpeed()`.
    pub walking_speed: f32,
    /// `getAttributeValue(MOVEMENT_SPEED)`.
    pub movement_speed: f64,
    /// The item in use and its use ticks, when using one.
    pub using: Option<(UseItem, i32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseItem {
    Bow,
    Spyglass,
    Other,
}
/// `AbstractClientPlayer.getFieldOfViewModifier(firstPerson, effectScale)`.
pub fn field_of_view_modifier(p: &FovPlayer, first_person: bool, effect_scale: f32) -> f32 {
    let mut modifier = 1.0f32;
    if p.flying {
        modifier *= 1.1;
    }
    let walking_speed = p.walking_speed;
    if walking_speed != 0.0 {
        let speed_factor = p.movement_speed as f32 / walking_speed;
        modifier *= (speed_factor + 1.0) / 2.0;
    }
    if let Some((item, ticks)) = p.using {
        match item {
            UseItem::Bow => {
                let scale = java_min(ticks as f32 / 20.0, 1.0);
                modifier *= 1.0 - scale * scale * 0.15;
            }
            // isScoping: using a spyglass.
            UseItem::Spyglass if first_person => return 0.1,
            _ => {}
        }
    }
    lerp(effect_scale, 1.0, modifier)
}

/// The camera entity as `Camera.alignWithEntity` and the field-of-view
/// death and hurt effects see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraEntity {
    pub position: [f64; 3],
    /// `xo`, `yo`, `zo`.
    pub previous: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub x_rot_o: f32,
    pub y_rot_o: f32,
    /// `Entity.getEyeHeight()`.
    pub eye_height: f32,
    /// A living entity's `getScale()` and camera distance attribute.
    pub living: Option<LivingCamera>,
    /// The vehicle's scale and camera distance when it is living.
    pub mount: Option<(f32, f64)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LivingCamera {
    pub scale: f32,
    pub camera_distance: f64,
    /// `isSleeping()` with `getBedOrientation()` as its `toYRot()`.
    pub sleeping: Option<Option<f32>>,
    /// `isDeadOrDying()` and `deathTime`.
    pub dead: Option<i32>,
}
impl CameraEntity {
    /// `Entity.getXRot(partialTicks)`.
    pub fn view_x_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.x_rot
        } else {
            lerp(partial, self.x_rot_o, self.x_rot)
        }
    }
    /// `LocalPlayer.getViewYRot` (`getYRot(partialTicks)`, a `Mth.rotLerp`).
    pub fn view_y_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.y_rot
        } else {
            self.y_rot_o + partial * wrap_degrees(self.y_rot - self.y_rot_o)
        }
    }
}

/// The fluid `Camera.getFluidInCamera` reports, as the field of view sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraFluid {
    #[default]
    None,
    Water,
    Lava,
    PowderSnow,
}

/// `Camera`: the eye-height and field-of-view easing and the view placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye_height: f32,
    pub eye_height_old: f32,
    pub fov_modifier: f32,
    pub old_fov_modifier: f32,
    pub position: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub rotation: Quat,
    pub forwards: [f32; 3],
    pub up: [f32; 3],
    pub left: [f32; 3],
    pub detached: bool,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            eye_height: 0.0,
            eye_height_old: 0.0,
            fov_modifier: 0.0,
            old_fov_modifier: 0.0,
            position: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            rotation: Quat::IDENTITY,
            forwards: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            left: [-1.0, 0.0, 0.0],
            detached: false,
        }
    }
}
impl Camera {
    /// `Camera.tick`: the eye height eases halfway toward the entity's
    /// (with a level and an entity), then `tickFov` with the camera entity's
    /// field-of-view modifier target (1 when it is not a player).
    pub fn tick(&mut self, entity_eye_height: Option<f32>, target_fov_modifier: f32) {
        if let Some(eye) = entity_eye_height {
            self.eye_height_old = self.eye_height;
            self.eye_height += (eye - self.eye_height) * 0.5;
        }
        self.old_fov_modifier = self.fov_modifier;
        self.fov_modifier += (target_fov_modifier - self.fov_modifier) * 0.5;
        self.fov_modifier = clamp(self.fov_modifier, 0.1, 1.5);
    }
    /// `Camera.setRotation`.
    pub fn set_rotation(&mut self, y_rot: f32, x_rot: f32) {
        self.x_rot = x_rot;
        self.y_rot = y_rot;
        self.rotation = Quat::rotation_yxz(PI - y_rot * DEG_TO_RAD, -x_rot * DEG_TO_RAD, 0.0);
        self.forwards = self.rotation.transform([0.0, 0.0, -1.0]);
        self.up = self.rotation.transform([0.0, 1.0, 0.0]);
        self.left = self.rotation.transform([-1.0, 0.0, 0.0]);
    }
    /// `Camera.move(forwards, up, right)`.
    pub fn move_by(&mut self, forwards: f32, up: f32, right: f32) {
        let offset = self.rotation.transform([right, up, -forwards]);
        self.position = [
            self.position[0] + f64::from(offset[0]),
            self.position[1] + f64::from(offset[1]),
            self.position[2] + f64::from(offset[2]),
        ];
    }
    /// `Camera.alignWithEntity` (not riding a minecart with the new
    /// movement): the entity's view rotation and interpolated eye, then a
    /// detached camera backed off along the view (`getMaxZoom`) or a sleeping
    /// one turned to the bed. `clip` is `Level.clip` with
    /// `ClipContext.Block.VISUAL` and no fluids; it returns the hit location
    /// unless the ray misses.
    pub fn align_with_entity(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        mut clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.set_rotation(entity.view_y_rot(partial), entity.view_x_rot(partial));
        let p = f64::from(partial);
        self.position = [
            lerp_d(p, entity.previous[0], entity.position[0]),
            lerp_d(p, entity.previous[1], entity.position[1])
                + f64::from(lerp(partial, self.eye_height_old, self.eye_height)),
            lerp_d(p, entity.previous[2], entity.position[2]),
        ];
        self.detached = !camera_type.is_first_person();
        if self.detached {
            if camera_type.is_mirrored() {
                self.set_rotation(self.y_rot + 180.0, -self.x_rot);
            }
            let (mut distance, mut scale) = (4.0f32, 1.0f32);
            if let Some(living) = &entity.living {
                scale = living.scale;
                distance = living.camera_distance as f32;
            }
            let (mut mount_scale, mut mount_distance) = (scale, distance);
            if let Some((s, d)) = entity.mount {
                mount_scale = s;
                mount_distance = d as f32;
            }
            let zoom = self.max_zoom(
                java_max(scale * distance, mount_scale * mount_distance),
                &mut clip,
            );
            self.move_by(-zoom, 0.0, 0.0);
        } else if let Some(LivingCamera {
            sleeping: Some(bed),
            ..
        }) = entity.living
        {
            self.set_rotation(bed.map_or(0.0, |yaw| yaw - 180.0), 0.0);
            self.move_by(0.0, 0.15, 0.0);
        }
    }
    /// `Camera.getMaxZoom`: eight rays from the corners of a 0.2 cube about
    /// the eye, shortening the distance to the nearest visual-shape hit.
    pub fn max_zoom(
        &self,
        mut distance: f32,
        clip: &mut impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) -> f32 {
        let at = self.position;
        for i in 0..8 {
            let offset = [
                ((i & 1) * 2 - 1) as f32,
                ((i >> 1 & 1) * 2 - 1) as f32,
                ((i >> 2 & 1) * 2 - 1) as f32,
            ];
            let from: [f64; 3] = std::array::from_fn(|a| at[a] + f64::from(offset[a] * 0.1));
            let back = f64::from(-distance);
            let to: [f64; 3] =
                std::array::from_fn(|a| from[a] + f64::from(self.forwards[a]) * back);
            if let Some(hit) = clip(from, to) {
                let d = [hit[0] - at[0], hit[1] - at[1], hit[2] - at[2]];
                let dist_sq = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f32;
                if dist_sq < distance * distance {
                    distance = dist_sq.sqrt();
                }
            }
        }
        distance
    }
    /// `Camera.calculateFov`: the option scaled by the eased modifier, then
    /// the death and fluid effects.
    pub fn calculate_fov(
        &self,
        partial: f32,
        fov_option: i32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
        panoramic: bool,
    ) -> f32 {
        if panoramic {
            return 90.0;
        }
        let fov = fov_option as f32 * lerp(partial, self.old_fov_modifier, self.fov_modifier);
        modify_fov_based_on_death_or_fluid(partial, fov, effect_scale, dead, fluid)
    }
    /// `Camera.calculateHudFov`.
    pub fn calculate_hud_fov(
        &self,
        partial: f32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
    ) -> f32 {
        modify_fov_based_on_death_or_fluid(partial, 70.0, effect_scale, dead, fluid)
    }
}
/// `Camera.update`: the far plane covers four times the render distance and
/// the whole cloud range (`depthFar`).
pub fn depth_far(render_distance_chunks: i32, cloud_range_chunks: i32) -> f32 {
    (render_distance_chunks as f32 * 16.0 * 4.0).max((cloud_range_chunks * 16) as f32)
}

/// `Projection.getMatrix` for a perspective set up by `setupPerspective`:
/// JOML's `Matrix4f.setPerspective(fov * DEG_TO_RAD, width / height, near,
/// far, zZeroToOne)`, column-major. Vanilla's `Projection` passes its planes
/// reversed (`near` is `zFar`) for reversed depth.
pub fn perspective(
    fov_degrees: f32,
    width: f32,
    height: f32,
    near: f32,
    far: f32,
    zero_to_one: bool,
) -> [f32; 16] {
    let h = horizon_core::jmath::tan(f64::from(fov_degrees * DEG_TO_RAD * 0.5)) as f32;
    let aspect = width / height;
    let mut m = [0.0f32; 16];
    m[0] = 1.0 / (h * aspect);
    m[5] = 1.0 / h;
    let e = 1.0e-6f32;
    if far > 0.0 && far.is_infinite() {
        m[10] = e - 1.0;
        m[14] = (e - if zero_to_one { 1.0 } else { 2.0 }) * near;
    } else if near > 0.0 && near.is_infinite() {
        m[10] = (if zero_to_one { 0.0 } else { 1.0 }) - e;
        m[14] = ((if zero_to_one { 1.0 } else { 2.0 }) - e) * far;
    } else {
        m[10] = (if zero_to_one { far } else { far + near }) / (near - far);
        m[14] = (if zero_to_one { far } else { far + far }) * near / (near - far);
    }
    m[11] = -1.0;
    m
}
/// `GameRenderer.render3dHud`: the first-person hand and held items are
/// drawn with their own projection at `hudFov` (70 degrees, changed only by
/// death and fluids), never the FOV option or the sprint, speed and bow
/// modifier.
pub fn hand_projection(hud_fov: f32, width: f32, height: f32, far: f32) -> [f32; 16] {
    perspective(hud_fov, width, height, far, 0.05, true)
}
/// `Camera.modifyFovBasedOnDeathOrFluid`. `dead` is the camera entity's
/// `deathTime` while it is a living entity that is dead or dying.
pub fn modify_fov_based_on_death_or_fluid(
    partial: f32,
    mut fov: f32,
    effect_scale: f32,
    dead: Option<i32>,
    fluid: CameraFluid,
) -> f32 {
    if let Some(death_time) = dead {
        let duration = java_min(death_time as f32 + partial, 20.0);
        fov /= (1.0 - 500.0 / (duration + 500.0)) * 2.0 + 1.0;
    }
    if matches!(fluid, CameraFluid::Lava | CameraFluid::Water) {
        fov *= lerp(effect_scale, 1.0, 0.857_142_87);
    }
    fov
}

/// `ClientAvatarState`: the walked distance, view bob and cloak of a player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AvatarBob {
    pub walk_dist: f32,
    pub walk_dist_o: f32,
    pub bob: f32,
    pub bob_o: f32,
    /// `xCloak`, `yCloak`, `zCloak`: a point trailing the player, which the cape leans from.
    pub cloak: [f64; 3],
    pub cloak_o: [f64; 3],
}
impl AvatarBob {
    /// `ClientAvatarState.tick` (at the start of `AbstractClientPlayer.tick`)
    /// with the player's position then: `moveCloak` eases the cloak a quarter
    /// of the way there, or puts it there from 10 blocks away.
    pub fn tick(&mut self, position: [f64; 3]) {
        self.walk_dist_o = self.walk_dist;
        self.cloak_o = self.cloak;
        for i in 0..3 {
            let d = position[i] - self.cloak[i];
            if d > 10.0 || d < -10.0 {
                self.cloak[i] = position[i];
                self.cloak_o[i] = position[i];
            } else {
                self.cloak[i] += d * 0.25;
            }
        }
    }
    /// `AvatarRenderer.extractCapeState`: the cape's `capeFlap`, `capeLean`
    /// and `capeLean2` (degrees) for the player at `position` (interpolated)
    /// with its body turned `body_yaw` and its `fallFlyingScale`.
    pub fn cape(
        &self,
        partial: f32,
        position: [f64; 3],
        body_yaw: f32,
        fall_flying_scale: f32,
    ) -> [f32; 3] {
        let d: [f64; 3] = std::array::from_fn(|i| {
            lerp_d(f64::from(partial), self.cloak_o[i], self.cloak[i]) - position[i]
        });
        let rad = f64::from(body_yaw * DEG_TO_RAD);
        let (sin, cos) = (f64::from(trig::sin(rad)), f64::from(-trig::cos(rad)));
        let mut flap = (d[1] as f32 * 10.0).clamp(-6.0, 32.0);
        let lean = ((d[0] * sin + d[2] * cos) as f32 * 100.0 * (1.0 - fall_flying_scale))
            .clamp(0.0, 150.0);
        let lean2 = ((d[0] * cos - d[2] * sin) as f32 * 100.0).clamp(-20.0, 20.0);
        let bob = self.interpolated_bob(partial);
        let walk = lerp(partial, self.walk_dist_o, self.walk_dist);
        flap += trig::sin(f64::from(walk * 6.0)) * 32.0 * bob;
        [flap, lean, lean2]
    }
    /// `LocalPlayer.move`: `addWalkedDistance(Mth.length(dx, dz) * 0.6F)`
    /// with the float horizontal displacement.
    pub fn add_walked(&mut self, dx: f32, dz: f32) {
        self.walk_dist += (dx * dx + dz * dz).sqrt() * 0.6;
    }
    /// `AbstractClientPlayer.updateBob`: toward the horizontal speed (at most
    /// 0.1) on the ground, toward 0 otherwise.
    pub fn update_bob(&mut self, on_ground_alive_not_swimming: bool, velocity: [f64; 3]) {
        let target = if on_ground_alive_not_swimming {
            java_min(
                0.1,
                (velocity[0] * velocity[0] + velocity[2] * velocity[2]).sqrt() as f32,
            )
        } else {
            0.0
        };
        self.bob_o = self.bob;
        self.bob += (target - self.bob) * 0.4;
    }
    /// `ClientAvatarState.resetBob` (`AbstractClientPlayer.rideTick`).
    pub fn reset_bob(&mut self) {
        self.bob_o = self.bob;
        self.bob = 0.0;
    }
    pub fn interpolated_bob(&self, partial: f32) -> f32 {
        lerp(partial, self.bob_o, self.bob)
    }
    pub fn backwards_interpolated_walk_distance(&self, partial: f32) -> f32 {
        let wda = self.walk_dist - self.walk_dist_o;
        -(self.walk_dist + wda * partial)
    }
}

/// The parts of `CameraRenderState.entityRenderState` the bobbing reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BobState {
    /// A living camera entity: `hurtTime - partial`, `hurtDuration`,
    /// `getHurtDir()`, and `deathTime + partial` when dead or dying.
    pub living: Option<HurtBob>,
    /// A player camera entity: the backwards-interpolated walk distance and
    /// the interpolated bob.
    pub player: Option<(f32, f32)>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HurtBob {
    pub hurt_time: f32,
    pub hurt_duration: i32,
    pub hurt_dir: f32,
    pub death_time: Option<f32>,
}
/// `PoseStack.rotateDegrees` on the pose with an `Axis`.
fn rotate_degrees(m: &mut Mat4, axis: usize, degrees: f32) {
    let ang = degrees * DEG_TO_RAD;
    match axis {
        0 => m.rotate_x(ang),
        1 => m.rotate_y(ang),
        _ => m.rotate_z(ang),
    }
}
/// `GameRenderer.bobHurt`.
pub fn bob_hurt(state: &BobState, damage_tilt_strength: f64, pose: &mut Mat4) {
    let Some(living) = state.living else { return };
    let mut hurt = living.hurt_time;
    if let Some(death_time) = living.death_time {
        let duration = java_min(death_time, 20.0);
        rotate_degrees(pose, 2, 40.0 - 8000.0 / (duration + 200.0));
    }
    if hurt < 0.0 {
        return;
    }
    hurt /= living.hurt_duration as f32;
    hurt = trig::sin(f64::from(hurt * hurt * hurt * hurt * PI));
    let rr = living.hurt_dir;
    rotate_degrees(pose, 1, -rr);
    let tilt = (f64::from(-hurt) * 14.0 * damage_tilt_strength) as f32;
    rotate_degrees(pose, 2, tilt);
    rotate_degrees(pose, 1, rr);
}
/// `GameRenderer.bobView`.
pub fn bob_view(state: &BobState, pose: &mut Mat4) {
    let Some((walk, bob)) = state.player else {
        return;
    };
    pose.translate(
        trig::sin(f64::from(walk * PI)) * bob * 0.5,
        -(trig::cos(f64::from(walk * PI)) * bob).abs(),
        0.0,
    );
    rotate_degrees(pose, 2, trig::sin(f64::from(walk * PI)) * bob * 3.0);
    rotate_degrees(
        pose,
        0,
        (trig::cos(f64::from(walk * PI - 0.2)) * bob).abs() * 5.0,
    );
}
/// The bob matrix `GameRenderer.renderLevel` multiplies into the projection
/// (and `renderItemInHand` into the hand's pose).
pub fn bob_matrix(state: &BobState, damage_tilt_strength: f64, bob_view_option: bool) -> Mat4 {
    let mut pose = Mat4::IDENTITY;
    bob_hurt(state, damage_tilt_strength, &mut pose);
    if bob_view_option {
        bob_view(state, &mut pose);
    }
    pose
}

/// The spinning effect `GameRenderer.renderLevel` multiplies into the level's
/// projection after the bob: a skew about a tilted axis, from a portal's or
/// Nausea's intensity (already scaled by Distortion Effects) and the angle in
/// degrees. The first-person hand keeps its own projection without it.
pub fn spinning_effect_matrix(intensity: f32, angle_degrees: f32) -> glam::Mat4 {
    if intensity <= 0.0 {
        return glam::Mat4::IDENTITY;
    }
    let mut skew = 5.0 / (intensity * intensity + 5.0) - intensity * 0.04;
    skew *= skew;
    let axis = glam::Vec3::new(
        0.0,
        std::f32::consts::SQRT_2 / 2.0,
        std::f32::consts::SQRT_2 / 2.0,
    );
    let angle = angle_degrees * DEG_TO_RAD;
    glam::Mat4::from_axis_angle(axis, angle)
        * glam::Mat4::from_scale(glam::Vec3::new(1.0 / skew, 1.0, 1.0))
        * glam::Mat4::from_axis_angle(axis, -angle)
}

#[cfg(test)]
mod tests {
    /// `moveCloak` trails a quarter of the way, jumps from 10 blocks away; the
    /// cape leans back as the player walks forward (south, body yaw 0).
    #[test]
    fn the_cloak_trails_the_player() {
        let mut a = super::AvatarBob::default();
        a.tick([0.0, 64.0, 0.0]);
        assert_eq!((a.cloak, a.cloak_o), ([0.0, 64.0, 0.0], [0.0, 64.0, 0.0]));
        a.tick([0.0, 64.0, 1.0]);
        assert_eq!(a.cloak, [0.0, 64.0, 0.25]);
        let [flap, lean, lean2] = a.cape(1.0, [0.0, 64.0, 1.0], 0.0, 0.0);
        assert_eq!((flap, lean, lean2), (0.0, 75.0, 0.0));
        // Falling lifts the flap, at most 32 degrees.
        assert_eq!(a.cape(1.0, [0.0, 60.0, 0.25], 0.0, 0.0)[0], 32.0);
        // Gliding takes the lean away.
        assert_eq!(a.cape(1.0, [0.0, 64.0, 1.0], 0.0, 1.0)[1], 0.0);
    }

    #[test]
    fn depth_far_covers_the_cloud_range() {
        // renderDistance 2 with the default Cloud Distance (128 chunks).
        assert_eq!(super::depth_far(2, 128), 2048.0);
        assert_eq!(super::depth_far(32, 2), 2048.0);
        assert_eq!(super::depth_far(12, 12), 768.0);
    }
    use super::*;
    #[test]
    fn view_keeps_yaw_at_vertical_pitch_limits() {
        for yaw in [0.0, 45.0, 90.0, 180.0, -90.0, 360.0] {
            for pitch in [-90.0, -89.999, 0.0, 89.999, 90.0] {
                let view = view_matrix(yaw, pitch);
                let mut camera = Camera::default();
                camera.set_rotation(yaw, pitch);
                for (world, expected) in [
                    (camera.forwards, -glam::Vec3::Z),
                    (camera.up, glam::Vec3::Y),
                    (camera.left, -glam::Vec3::X),
                ] {
                    let got = view.transform_vector3(glam::Vec3::from(world));
                    assert!(
                        (got - expected).length() < 1e-5,
                        "yaw={yaw}, pitch={pitch}: {got:?} != {expected:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn camera_eases_eye_height_and_fov() {
        let mut camera = Camera::default();
        camera.tick(Some(1.62), 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.5);
        camera.tick(None, 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.75);
    }
    #[test]
    fn rotation_faces_south_at_zero_yaw() {
        let mut camera = Camera::default();
        camera.set_rotation(0.0, 0.0);
        assert!((camera.forwards[2] - 1.0).abs() < 1e-6);
        camera.set_rotation(90.0, 0.0);
        assert!((camera.forwards[0] + 1.0).abs() < 1e-6);
    }
    #[test]
    fn detached_camera_backs_off_until_blocked() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        let mut camera = Camera::default();
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| None);
        assert!((camera.position[2] + 4.0).abs() < 1e-6);
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| {
            Some([0.0, 0.0, -1.0])
        });
        assert!(camera.position[2] > -1.1 && camera.position[2] < -0.9);
    }
    #[test]
    fn fov_modifier_combines_speed_flight_and_bow() {
        let mut p = FovPlayer {
            flying: false,
            walking_speed: 0.1,
            movement_speed: 0.13,
            using: None,
        };
        assert_eq!(
            field_of_view_modifier(&p, true, 1.0),
            (0.13f32 / 0.1 + 1.0) / 2.0
        );
        assert_eq!(field_of_view_modifier(&p, true, 0.0), 1.0);
        p.using = Some((UseItem::Spyglass, 3));
        assert_eq!(field_of_view_modifier(&p, true, 1.0), 0.1);
        assert_ne!(field_of_view_modifier(&p, false, 1.0), 0.1);
    }
}
