//! The uniforms a shader pack's post-processing programs read once per frame (spec 5.2, class
//! F), from the renderer's `Frame` and the few values only the game knows (`PackInputs`).
//! Matrices follow GL's conventions (5.5): `gbufferProjection` maps view depth to clip z in
//! [-w, w], and the depth buffers packs read hold GL window depth.

use glam::{DMat4, DVec3, Mat4, Vec3, Vec4};
use horizon_shaders::Shadow;

use super::Frame;

/// What the game passes for the pack's uniforms, beyond what `Frame` carries.
#[derive(Clone, Copy, Debug, Default)]
pub struct PackInputs {
    /// `Level.getDayTime`.
    pub day_time: i64,
    /// The interpolated rain and thunder levels, 0..1.
    pub rain: f32,
    pub thunder: f32,
    /// The fluid at the camera: 0 none, 1 water, 2 lava, 3 powder snow.
    pub eye_in: i32,
    pub blindness: f32,
    /// Block and sky light at the camera, 0..15.
    pub eye_light: [i32; 2],
    pub hide_gui: bool,
    /// The brightness option, 0..1.
    pub brightness: f32,
    /// The biome at the camera (3.4): registry id, `CAT_*`, `PPT_*`, temperature, downfall.
    pub biome: i32,
    pub biome_category: i32,
    pub biome_precipitation: i32,
    pub temperature: f32,
    pub rainfall: f32,
    /// Whether the biome at the camera is known (its chunk is loaded).
    pub biome_known: bool,
    /// `heldItemId`, `heldItemId2` (item.properties, 5.3) and `heldBlockLightValue(2)`.
    pub held_items: [i32; 2],
    pub held_light: [i32; 2],
    /// `isRightHanded` (the Main Hand option) and `is_on_ground`.
    pub right_handed: bool,
    pub on_ground: bool,
    /// The dimension type's lowest block (`bedrockLevel`), build height (`heightLimit`),
    /// logical height, sky light and ceiling.
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    /// `isElytraFlying`: the player is fall flying.
    pub elytra_flying: bool,
    /// `cloudHeight`, see [`cloud_height`].
    pub cloud_height: f32,
    /// The values native mods hand their packs (`frame.mods`, `native_shader_params`).
    pub mod_params: [[f32; 4]; 8],
    /// The player's state, for Iris's status uniforms.
    pub status: PlayerStatus,
    /// The End flash: its intensity, 0..1 (0 outside the End), and its direction from the
    /// camera in world space, of length 100 (`endFlashIntensity`, `endFlashPosition`).
    pub end_flash: f32,
    pub end_flash_direction: Vec3,
}

/// The `cloudHeight` uniform. Vanilla reads `visual/cloud_height` only while the cloud color
/// has alpha (`LevelExtractor`); a dimension without clouds has no height, which the uniform
/// reports as NaN. NaN is not vanilla's own value: packs map it to "no clouds".
pub fn cloud_height(cloud_color_alpha: f32, height: f32) -> f32 {
    if crate::clouds::has_color(cloud_color_alpha) {
        height
    } else {
        f32::NAN
    }
}

/// The local player's state behind Iris's status uniforms (`currentPlayerHealth`,
/// `isSpectator`, `is_burning`...).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerStatus {
    /// Survival or adventure: the vitals are -1 in creative and spectator.
    pub survival: bool,
    pub health: f32,
    pub max_health: f32,
    pub food: f32,
    /// Air supply ticks, of 300.
    pub air: f32,
    /// Armor points (the `armor` attribute).
    pub armor: f32,
    pub spectator: bool,
    pub riding: bool,
    pub feet_in_water: bool,
    pub swimming: bool,
    pub burning: bool,
    pub hurt: bool,
    pub invisible: bool,
    pub sneaking: bool,
    pub sprinting: bool,
}

/// `maxPlayerHunger`, `maxPlayerAir` and `maxPlayerArmor`: Minecraft's own maximums.
const MAX_HUNGER: f32 = 20.0;
const MAX_AIR: f32 = 300.0;
const MAX_ARMOR: f32 = 50.0;

/// One uniform's value: up to 16 floats or 3 integers.
#[derive(Clone, Copy, Debug)]
pub enum Value {
    F([f32; 16]),
    I([i32; 3]),
}

impl Value {
    fn f(v: &[f32]) -> Self {
        let mut a = [0.0; 16];
        a[..v.len()].copy_from_slice(v);
        Value::F(a)
    }
    fn i(v: &[i32]) -> Self {
        let mut a = [0; 3];
        a[..v.len()].copy_from_slice(v);
        Value::I(a)
    }

    /// Writes the value as the type the pack declared, at `offset` of a std140 block.
    pub fn write(&self, ty: &str, out: &mut [u8], offset: usize) {
        let ints = matches!(ty, "int" | "bool" | "uint") || ty.starts_with(['i', 'u']);
        let (floats, integers): ([f32; 16], [i32; 16]) = match *self {
            Value::F(f) => (f, f.map(|v| v as i32)),
            Value::I(i) => {
                let mut f = [0.0; 16];
                let mut v = [0; 16];
                for k in 0..3 {
                    f[k] = i[k] as f32;
                    v[k] = i[k];
                }
                (f, v)
            }
        };
        let count = match ty {
            "mat4" => 16,
            "mat3" => {
                // std140: three columns, each padded to a vec4, from the mat4's upper left.
                for c in 0..3 {
                    let column: [f32; 3] = std::array::from_fn(|r| floats[c * 4 + r]);
                    put(out, offset + c * 16, bytemuck::cast_slice(&column));
                }
                return;
            }
            t => match t.as_bytes().last() {
                Some(n @ b'2'..=b'4') if t.contains("vec") => (n - b'0') as usize,
                _ => 1,
            },
        };
        if ints {
            put(out, offset, bytemuck::cast_slice(&integers[..count]));
        } else {
            put(out, offset, bytemuck::cast_slice(&floats[..count]));
        }
    }
}

fn put(out: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(dst) = out.get_mut(offset..offset + bytes.len()) {
        dst.copy_from_slice(bytes);
    }
}

/// The uniforms provided, in the order of [`FrameUniforms::values`].
pub const NAMES: &[&str] = &[
    "cameraPosition",
    "previousCameraPosition",
    "eyeAltitude",
    "upPosition",
    "sunPosition",
    "moonPosition",
    "shadowLightPosition",
    "sunAngle",
    "shadowAngle",
    "worldTime",
    "worldDay",
    "moonPhase",
    "frameCounter",
    "frameTime",
    "frameTimeCounter",
    "viewWidth",
    "viewHeight",
    "aspectRatio",
    "near",
    "far",
    "rainStrength",
    "wetness",
    "thunderStrength",
    "fogColor",
    "skyColor",
    "fogMode",
    "fogStart",
    "fogEnd",
    "fogDensity",
    "fogShape",
    "isEyeInWater",
    "blindness",
    "nightVision",
    "darknessFactor",
    "darknessLightFactor",
    "screenBrightness",
    "hideGUI",
    "eyeBrightness",
    "eyeBrightnessSmooth",
    "heldItemId",
    "heldItemId2",
    "heldBlockLightValue",
    "heldBlockLightValue2",
    "renderStage",
    "biome",
    "biome_category",
    "biome_precipitation",
    "temperature",
    "rainfall",
    "gbufferModelView",
    "gbufferModelViewInverse",
    "gbufferProjection",
    "gbufferProjectionInverse",
    "gbufferPreviousModelView",
    "gbufferPreviousProjection",
    "atlasSize",
    "shadowModelView",
    "shadowModelViewInverse",
    "shadowProjection",
    "shadowProjectionInverse",
    "cameraPositionInt",
    "cameraPositionFract",
    "previousCameraPositionInt",
    "previousCameraPositionFract",
    "eyePosition",
    "relativeEyePosition",
    "playerLookVector",
    "playerBodyVector",
    "firstPersonCamera",
    "currentPlayerHealth",
    "maxPlayerHealth",
    "currentPlayerHunger",
    "maxPlayerHunger",
    "currentPlayerAir",
    "maxPlayerAir",
    "currentPlayerArmor",
    "maxPlayerArmor",
    "isSpectator",
    "isRiding",
    "feetInWater",
    "inSwimmingAnimation",
    "is_burning",
    "is_hurt",
    "is_invisible",
    "is_sneaking",
    "is_sprinting",
    "endFlashIntensity",
    "endFlashPosition",
    "centerDepthSmooth",
    "isRightHanded",
    "is_on_ground",
    "bedrockLevel",
    "heightLimit",
    "logicalHeightLimit",
    "hasSkylight",
    "hasCeiling",
    "isElytraFlying",
    "cloudHeight",
];

/// Values carried from frame to frame.
pub struct FrameUniforms {
    pub values: Vec<Value>,
    /// The block atlas's size (`atlasSize`), 0 without one.
    pub atlas: [u32; 2],
    start: std::time::Instant,
    last: Option<std::time::Instant>,
    frame: u32,
    previous: Option<(glam::DVec3, Mat4, Mat4)>,
    wetness: f32,
    eye_smooth: [f32; 2],
    /// `depthtex0` at the screen's centre as last read back, and `centerDepthSmooth`.
    pub center_depth_raw: Option<f32>,
    center_depth: Option<f32>,
    /// The pack's shadow settings (7.1).
    pub shadow: Shadow,
}

impl Default for FrameUniforms {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            atlas: [0; 2],
            start: std::time::Instant::now(),
            last: None,
            frame: 0,
            previous: None,
            wetness: 0.0,
            eye_smooth: [0.0; 2],
            center_depth_raw: None,
            center_depth: None,
            shadow: Shadow::default(),
        }
    }
}

/// `(0, 100, 0)` on the frame the sky draws a body on: Y -90°, then X by its angle in degrees;
/// `sunPathRotation` (`tilt`, degrees) then turns the path about the world's X axis (7.1, OQ-6).
fn body(angle: f32, tilt: f32) -> Vec3 {
    (tilt_matrix(tilt)
        * Mat4::from_rotation_y((-90.0f32).to_radians())
        * Mat4::from_rotation_x(angle.to_radians()))
    .transform_vector3(Vec3::new(0.0, 100.0, 0.0))
}

/// `sunPathRotation`'s turn of the sky's bodies: `tilt` degrees about the world's X axis, so a
/// positive angle moves the noon sun toward -Z (north), as Iris does.
pub fn tilt_matrix(tilt: f32) -> Mat4 {
    Mat4::from_rotation_x(-tilt.to_radians())
}

/// The world direction of the shadow light (7.1): the sun by day, the moon by night.
pub fn shadow_light(frame: &Frame, shadow: &Shadow) -> Vec3 {
    let sun_angle = (frame.sky.sun_angle / 360.0 + 0.25).rem_euclid(1.0);
    let angle = if sun_angle < 0.5 {
        frame.sky.sun_angle
    } else {
        frame.sky.moon_angle
    };
    body(angle, shadow.sun_path_rotation).normalize()
}

/// `shadow_camera`'s view and projection in wgpu's clip space (depth 0..1), for culling.
pub fn shadow_culling(frame: &Frame, shadow: &Shadow) -> Mat4 {
    let (view, gl) = shadow_camera(shadow_light(frame, shadow), frame.camera, shadow);
    // GL's depth -w..w to wgpu's 0..w.
    let depth = Mat4::from_cols(
        Vec4::X,
        Vec4::Y,
        Vec4::new(0.0, 0.0, 0.5, 0.0),
        Vec4::new(0.0, 0.0, 0.5, 1.0),
    );
    depth * gl * view
}

/// How far toward the light the shadow projection starts, in blocks: part of the projection,
/// not of `shadowModelView` (Iris's matrices, measured).
pub const SHADOW_EYE: f32 = 100.0;

/// The shadow camera as Iris places it (7.1; measured from the outside): its view (player
/// space to shadow view space) and GL projection. The view undoes `sunPathRotation`, turns the
/// light's path about Z by the light's angle from the zenith, then X by 90 degrees, so the
/// light lies on the view's +Z. Its origin is `shadowIntervalSize / 2 - camera % interval`
/// (the remainder takes the camera's sign) from the player on each world axis, so the map moves
/// in whole steps. The orthographic projection covers `shadowDistance` each side and depths
/// from `-near` to `far`, counted from `SHADOW_EYE` blocks toward the light; or perspective
/// with `shadowMapFov`.
pub fn shadow_camera(light: Vec3, camera: DVec3, shadow: &Shadow) -> (Mat4, Mat4) {
    let tilt = f64::from(shadow.sun_path_rotation).to_radians();
    let untilted = DMat4::from_rotation_x(tilt).transform_vector3(light.as_dvec3());
    let zenith = untilted.x.atan2(untilted.y);
    let rotation = DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2)
        * DMat4::from_rotation_z(zenith)
        * DMat4::from_rotation_x(tilt);
    let step = f64::from(shadow.interval);
    let origin = if step > 0.0 {
        let remainder = if shadow.euclidean_snap {
            camera.rem_euclid(DVec3::splat(step))
        } else {
            camera % step
        };
        DVec3::splat(step / 2.0) - remainder
    } else {
        DVec3::ZERO
    };
    let view = (rotation * DMat4::from_translation(-origin)).as_mat4();
    let (near, far) = (shadow.near, shadow.far);
    let eye = Mat4::from_translation(Vec3::new(0.0, 0.0, -SHADOW_EYE));
    let projection = match shadow.fov {
        Some(fov) => Mat4::perspective_rh_gl(fov.to_radians(), 1.0, near, far) * eye,
        None => {
            let d = shadow.distance;
            Mat4::orthographic_rh_gl(-d, d, -d, d, -near, far) * eye
        }
    };
    (view, projection)
}

/// `value` moved toward `target` with a half-life of `half_life` seconds over `dt` (5.6).
fn smooth(value: f32, target: f32, half_life: f32, dt: f32) -> f32 {
    target + (value - target) * 0.5f32.powf(dt / half_life)
}

impl FrameUniforms {
    /// This frame's values for a render size of `size` pixels.
    pub fn update(&mut self, frame: &Frame, inputs: &PackInputs, size: [u32; 2]) {
        let now = std::time::Instant::now();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f32());
        self.last = Some(now);
        let aspect = size[0] as f32 / size[1].max(1) as f32;
        // Packs take gbufferProjection for a plain perspective (they invert only its perspective
        // terms): the view bob and the nausea spin belong to the model-view, as GL drew them.
        let view = frame.bob * frame.spin * crate::camera::view_matrix(frame.yaw, frame.pitch);
        let projection = Mat4::perspective_rh_gl(
            frame.fov_degrees.to_radians(),
            aspect,
            0.05,
            frame.depth_far,
        );
        let camera = frame.camera.as_vec3();
        let first_frame = self.previous.is_none();
        let (previous_exact, previous_view, previous_projection) =
            self.previous.unwrap_or((frame.camera, view, projection));
        let previous_camera = previous_exact.as_vec3();
        self.previous = Some((frame.camera, view, projection));
        // The camera's whole blocks and the rest, exact however far from the origin.
        let whole = |p: glam::DVec3| p.floor();
        let int = |p: glam::DVec3| Value::i(&whole(p).as_ivec3().to_array());
        let fract = |p: glam::DVec3| Value::f(&(p - whole(p)).as_vec3().to_array());
        // The player's, not the camera's: in a third-person view the camera is behind (or in
        // front of, turned around) the eye the pack lights from.
        let [yaw, pitch] = frame.eye_rotation.map(f32::to_radians);
        let look = Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );

        let to_view = |v: Vec3| view.transform_vector3(v);
        let sun_angle = (frame.sky.sun_angle / 360.0 + 0.25).rem_euclid(1.0);
        let day = sun_angle < 0.5;
        let tilt = self.shadow.sun_path_rotation;
        let sun = to_view(body(frame.sky.sun_angle, tilt));
        let moon = to_view(body(frame.sky.moon_angle, tilt));
        // Half-lives are given in ticks (7.2); smoothing runs per frame in seconds (OQ-26).
        let half_life = if inputs.rain > self.wetness {
            600.0
        } else {
            200.0
        };
        self.wetness = smooth(self.wetness, inputs.rain, half_life / 20.0, dt);
        let eye = inputs.eye_light.map(|l| (l * 16) as f32);
        if first_frame {
            self.wetness = inputs.rain;
            self.eye_smooth = eye;
        }
        for (smoothed, target) in self.eye_smooth.iter_mut().zip(eye) {
            *smoothed = smooth(*smoothed, target, 10.0 / 20.0, dt);
        }
        // `centerDepthHalflife`'s default, 1 tick (OptiFine's shaders.txt); the first value
        // read back is taken as it is.
        if let Some(raw) = self.center_depth_raw {
            self.center_depth = Some(match self.center_depth {
                Some(v) => smooth(v, raw, 1.0 / 20.0, dt),
                None => raw,
            });
        }
        let center_depth = self.center_depth.unwrap_or(1.0);
        // Vanilla fades by the stronger of two linear fogs; a pack gets one: the pair (start
        // and end together) of the one that is complete nearer (OQ-36). Mixing the nearer start
        // of one with the nearer end of the other made fog begin at the camera on clear days.
        let (fog_color, fog_start, fog_end) = match &frame.fog {
            Some(f) => {
                let (start, end) = if f.environmental_end < f.render_distance_end {
                    (f.environmental_start, f.environmental_end)
                } else {
                    (f.render_distance_start, f.render_distance_end)
                };
                ([f.color[0], f.color[1], f.color[2]], start, end)
            }
            None => (frame.sky.fog_color, frame.distance * 0.9, frame.distance),
        };
        let light = shadow_light(frame, &self.shadow);
        let (shadow_view, shadow_projection) = shadow_camera(light, frame.camera, &self.shadow);
        let day_time = inputs.day_time.rem_euclid(24000) as i32;
        let world_day = inputs.day_time.div_euclid(24000);
        let seconds = (now - self.start).as_secs_f32() % 3600.0;

        let value = |name: &str| -> Value {
            let v3 = |v: Vec3| Value::f(&v.to_array());
            let m4 = |m: Mat4| Value::f(&m.to_cols_array());
            match name {
                "cameraPosition" => v3(camera),
                "previousCameraPosition" => v3(previous_camera),
                "cameraPositionInt" => int(frame.camera),
                "cameraPositionFract" => fract(frame.camera),
                "previousCameraPositionInt" => int(previous_exact),
                "previousCameraPositionFract" => fract(previous_exact),
                // The camera is the eye in first person.
                "eyePosition" => v3(frame.eye.as_vec3()),
                "relativeEyePosition" => v3((frame.camera - frame.eye).as_vec3()),
                "playerLookVector" | "playerBodyVector" => v3(look),
                "firstPersonCamera" => Value::i(&[i32::from(frame.first_person)]),
                "isRightHanded" => Value::i(&[i32::from(inputs.right_handed)]),
                "is_on_ground" => Value::i(&[i32::from(inputs.on_ground)]),
                "bedrockLevel" => Value::i(&[inputs.min_y]),
                "heightLimit" => Value::i(&[inputs.height]),
                "logicalHeightLimit" => Value::i(&[inputs.logical_height]),
                "hasSkylight" => Value::i(&[i32::from(inputs.has_skylight)]),
                "hasCeiling" => Value::i(&[i32::from(inputs.has_ceiling)]),
                "eyeAltitude" => Value::f(&[frame.camera.y as f32]),
                "upPosition" => v3(to_view(Vec3::new(0.0, 100.0, 0.0))),
                "sunPosition" => v3(sun),
                "endFlashPosition" => v3(to_view(inputs.end_flash_direction)),
                "centerDepthSmooth" => Value::f(&[center_depth]),
                "moonPosition" => v3(moon),
                "shadowLightPosition" => v3(if day { sun } else { moon }),
                "sunAngle" => Value::f(&[sun_angle]),
                "shadowAngle" => Value::f(&[if day { sun_angle } else { sun_angle - 0.5 }]),
                "worldTime" => Value::i(&[day_time]),
                "worldDay" => Value::i(&[world_day as i32]),
                "moonPhase" => Value::i(&[world_day.rem_euclid(8) as i32]),
                "frameCounter" => Value::i(&[self.frame as i32]),
                "shadowModelView" => m4(shadow_view),
                "shadowModelViewInverse" => m4(shadow_view.inverse()),
                "shadowProjection" => m4(shadow_projection),
                "shadowProjectionInverse" => m4(shadow_projection.inverse()),
                "atlasSize" => Value::i(&[self.atlas[0] as i32, self.atlas[1] as i32]),
                "frameTime" => Value::f(&[dt]),
                "frameTimeCounter" => Value::f(&[seconds]),
                "viewWidth" => Value::f(&[size[0] as f32]),
                "viewHeight" => Value::f(&[size[1] as f32]),
                "aspectRatio" => Value::f(&[aspect]),
                "near" => Value::f(&[0.05]),
                // The chunk render distance, not the distant terrain's reach.
                "far" => Value::f(&[frame.detail_distance]),
                "rainStrength" => Value::f(&[inputs.rain]),
                "wetness" => Value::f(&[self.wetness]),
                "thunderStrength" => Value::f(&[inputs.thunder]),
                "fogColor" => Value::f(&fog_color),
                "skyColor" => Value::f(&frame.sky.sky_color),
                // GL_LINEAR: vanilla's fog is linear.
                "fogMode" => Value::i(&[9729]),
                "fogStart" => Value::f(&[fog_start]),
                "fogEnd" => Value::f(&[fog_end]),
                "fogDensity" => Value::f(&[1.0]),
                "fogShape" => Value::i(&[0]),
                "isEyeInWater" => Value::i(&[inputs.eye_in]),
                "blindness" => Value::f(&[inputs.blindness]),
                "nightVision" => Value::f(&[frame.night_vision]),
                "darknessFactor" => Value::f(&[frame.darkness.0]),
                "darknessLightFactor" => Value::f(&[frame.darkness.1]),
                "screenBrightness" => Value::f(&[inputs.brightness]),
                "hideGUI" => Value::i(&[inputs.hide_gui as i32]),
                "eyeBrightness" => Value::i(&eye.map(|v| v as i32)),
                "biome" => Value::i(&[inputs.biome]),
                "biome_category" => Value::i(&[inputs.biome_category]),
                "biome_precipitation" => Value::i(&[inputs.biome_precipitation]),
                "temperature" => Value::f(&[inputs.temperature]),
                "rainfall" => Value::f(&[inputs.rainfall]),
                "eyeBrightnessSmooth" => Value::i(&self.eye_smooth.map(|v| v.round() as i32)),
                "gbufferModelView" => m4(view),
                "gbufferModelViewInverse" => m4(view.inverse()),
                "gbufferProjection" => m4(projection),
                "gbufferProjectionInverse" => m4(projection.inverse()),
                "gbufferPreviousModelView" => m4(previous_view),
                "gbufferPreviousProjection" => m4(previous_projection),
                "heldItemId" => Value::i(&[inputs.held_items[0]]),
                "heldItemId2" => Value::i(&[inputs.held_items[1]]),
                "heldBlockLightValue" => Value::i(&[inputs.held_light[0]]),
                "heldBlockLightValue2" => Value::i(&[inputs.held_light[1]]),
                "isElytraFlying" => Value::i(&[inputs.elytra_flying as i32]),
                "cloudHeight" => Value::f(&[inputs.cloud_height]),
                // renderStage is NONE outside gbuffers draws.
                _ => status_value(name, inputs).unwrap_or(Value::i(&[0])),
            }
        };
        self.values.clear();
        self.values.extend(NAMES.iter().map(|n| value(n)));
        self.frame = (self.frame + 1) % 720_720;
    }

    /// `hz_fog_color` and `hz_fog` of the translator's block (the compatibility `gl_Fog`).
    pub fn fog(&self) -> [Vec4; 2] {
        let get = |name| match index(name).and_then(|i| self.values.get(i)) {
            Some(Value::F(f)) => *f,
            _ => [0.0; 16],
        };
        let (color, start, end) = (get("fogColor"), get("fogStart")[0], get("fogEnd")[0]);
        [
            Vec4::new(color[0], color[1], color[2], 1.0),
            Vec4::new(1.0, start, end, 1.0 / (end - start).max(1e-3)),
        ]
    }
}

impl FrameUniforms {
    /// Evaluates the pack's custom uniforms with this frame's values and puts them after
    /// `NAMES`' values, in `custom.uniforms()` order.
    pub fn eval_custom(&mut self, custom: &mut horizon_shaders::Custom) {
        self.values.truncate(NAMES.len());
        let values = &self.values;
        let builtin = |name: &str| {
            let v = values.get(index(name)?)?;
            let n = components(name);
            Some(match *v {
                Value::F(f) => horizon_shaders::Val::vector(&f.map(f64::from)[..n]),
                Value::I(i) => horizon_shaders::Val::vector(&i.map(f64::from)[..n.min(3)]),
            })
        };
        let dt = match index("frameTime").and_then(|i| values.get(i)) {
            Some(Value::F(f)) => f64::from(f[0]),
            _ => 0.0,
        };
        let out = custom.eval(dt, &builtin);
        self.values
            .extend(out.iter().map(|v| Value::f(&v.v.map(|x| x as f32)[..v.n])));
    }
}

/// Iris's player status and dimension uniforms, from `inputs`: the vitals are fractions of
/// their maximum, and -1 (with the maximums of health and air) in creative and spectator.
fn status_value(name: &str, inputs: &PackInputs) -> Option<Value> {
    let status = inputs.status;
    let vital = |v: f32| {
        Value::f(&[if status.survival {
            v.clamp(0.0, 1.0)
        } else {
            -1.0
        }])
    };
    let flag = |b: bool| Value::i(&[i32::from(b)]);
    Some(match name {
        "currentPlayerHealth" => vital(status.health / status.max_health.max(1.0)),
        "maxPlayerHealth" => Value::f(&[if status.survival {
            status.max_health
        } else {
            -1.0
        }]),
        "currentPlayerHunger" => vital(status.food / MAX_HUNGER),
        "maxPlayerHunger" => Value::f(&[MAX_HUNGER]),
        "currentPlayerAir" => vital(status.air / MAX_AIR),
        "maxPlayerAir" => Value::f(&[if status.survival { MAX_AIR } else { -1.0 }]),
        "currentPlayerArmor" => vital(status.armor / MAX_ARMOR),
        "maxPlayerArmor" => Value::f(&[MAX_ARMOR]),
        "isSpectator" => flag(status.spectator),
        "isRiding" => flag(status.riding),
        "feetInWater" => flag(status.feet_in_water),
        "inSwimmingAnimation" => flag(status.swimming),
        "is_burning" => flag(status.burning),
        "is_hurt" => flag(status.hurt),
        // Spectators count as invisible.
        "is_invisible" => flag(status.invisible || status.spectator),
        "is_sneaking" => flag(status.sneaking),
        "is_sprinting" => flag(status.sprinting),
        "endFlashIntensity" => Value::f(&[inputs.end_flash]),
        _ => return None,
    })
}

/// How many components the uniform `name` has (16 for a matrix).
fn components(name: &str) -> usize {
    match name {
        "cameraPosition"
        | "previousCameraPosition"
        | "cameraPositionInt"
        | "cameraPositionFract"
        | "previousCameraPositionInt"
        | "previousCameraPositionFract"
        | "eyePosition"
        | "relativeEyePosition"
        | "playerLookVector"
        | "playerBodyVector"
        | "upPosition"
        | "sunPosition"
        | "moonPosition"
        | "shadowLightPosition"
        | "fogColor"
        | "skyColor"
        | "endFlashPosition" => 3,
        "eyeBrightness" | "eyeBrightnessSmooth" | "atlasSize" => 2,
        n if n.starts_with("gbuffer") || n.starts_with("shadow") && n != "shadowAngle" => 16,
        _ => 1,
    }
}

/// The position of `name` in [`NAMES`].
pub fn index(name: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_the_declared_type() {
        let mut out = vec![0u8; 64];
        Value::i(&[7]).write("float", &mut out, 0);
        assert_eq!(f32::from_ne_bytes(out[0..4].try_into().unwrap()), 7.0);
        Value::f(&[1.5, 2.5]).write("ivec2", &mut out, 4);
        assert_eq!(i32::from_ne_bytes(out[8..12].try_into().unwrap()), 2);
        Value::i(&[1]).write("bool", &mut out, 16);
        assert_eq!(i32::from_ne_bytes(out[16..20].try_into().unwrap()), 1);
        Value::f(&[1.0; 16]).write("mat4", &mut out, 60);
        assert_eq!(out[60..64], [0; 4], "a value past the block is dropped");
    }

    /// Iris's status uniforms as its documentation defines them (shaders.properties, "Status").
    #[test]
    fn status_uniforms_follow_iris() {
        let float = |name: &str, inputs: &PackInputs| match status_value(name, inputs) {
            Some(Value::F(f)) => f[0],
            other => panic!("{name}: {other:?}"),
        };
        let int = |name: &str, inputs: &PackInputs| match status_value(name, inputs) {
            Some(Value::I(i)) => i[0],
            other => panic!("{name}: {other:?}"),
        };
        let mut inputs = PackInputs {
            status: PlayerStatus {
                survival: true,
                health: 15.0,
                max_health: 20.0,
                food: 10.0,
                air: 150.0,
                armor: 20.0,
                sneaking: true,
                ..Default::default()
            },
            ..Default::default()
        };
        // The vitals are fractions of their maximums.
        assert_eq!(float("currentPlayerHealth", &inputs), 0.75);
        assert_eq!(float("maxPlayerHealth", &inputs), 20.0);
        assert_eq!(float("currentPlayerHunger", &inputs), 0.5);
        assert_eq!(float("maxPlayerHunger", &inputs), 20.0);
        assert_eq!(float("currentPlayerAir", &inputs), 0.5);
        assert_eq!(float("maxPlayerAir", &inputs), 300.0);
        assert_eq!(float("currentPlayerArmor", &inputs), 0.4);
        assert_eq!(float("maxPlayerArmor", &inputs), 50.0);
        assert_eq!(int("is_sneaking", &inputs), 1);
        assert_eq!(int("is_invisible", &inputs), 0);
        // Creative and spectator: -1 for the vitals and the maximums of health and air;
        // spectators are invisible.
        inputs.status.survival = false;
        inputs.status.spectator = true;
        for name in [
            "currentPlayerHealth",
            "maxPlayerHealth",
            "currentPlayerHunger",
            "currentPlayerAir",
            "maxPlayerAir",
            "currentPlayerArmor",
        ] {
            assert_eq!(float(name, &inputs), -1.0, "{name}");
        }
        assert_eq!(float("maxPlayerHunger", &inputs), 20.0);
        assert_eq!(float("maxPlayerArmor", &inputs), 50.0);
        assert_eq!(int("isSpectator", &inputs), 1);
        assert_eq!(int("is_invisible", &inputs), 1);
        // Each is provided, not left at zero for the pack.
        for name in [
            "currentPlayerHealth",
            "isSpectator",
            "is_sprinting",
            "bedrockLevel",
        ] {
            assert!(index(name).is_some(), "{name} is not provided");
        }
        assert!(
            NAMES
                .iter()
                .filter(|n| status_value(n, &inputs).is_some())
                .count()
                == 18
        );
    }

    #[test]
    fn cloud_height_needs_a_cloud_color() {
        assert_eq!(cloud_height(0.8, 192.33), 192.33);
        assert_eq!(cloud_height(0.8, 120.0), 120.0);
        assert!(cloud_height(0.0, 192.33).is_nan());
    }

    #[test]
    fn elytra_and_cloud_height_uniforms_are_written() {
        assert!(NAMES.ends_with(&["isElytraFlying", "cloudHeight"]));
        let mut out = vec![0u8; 16];
        Value::i(&[1]).write("bool", &mut out, 0);
        assert_eq!(i32::from_ne_bytes(out[0..4].try_into().unwrap()), 1);
        Value::f(&[192.33]).write("float", &mut out, 4);
        assert_eq!(f32::from_ne_bytes(out[4..8].try_into().unwrap()), 192.33);
        Value::f(&[f32::NAN]).write("float", &mut out, 8);
        assert!(f32::from_ne_bytes(out[8..12].try_into().unwrap()).is_nan());
    }

    #[test]
    fn noon_is_straight_up_and_the_sun_sets_in_the_west() {
        assert!(body(0.0, 0.0).abs_diff_eq(Vec3::new(0.0, 100.0, 0.0), 1e-3));
        assert!(body(90.0, 0.0).abs_diff_eq(Vec3::new(-100.0, 0.0, 0.0), 1e-3));
        // sunPathRotation tilts noon toward -Z and leaves sunset on the horizon.
        assert!(body(0.0, 30.0).abs_diff_eq(Vec3::new(0.0, 86.60254, -50.0), 1e-3));
        assert!(body(90.0, 30.0).abs_diff_eq(Vec3::new(-100.0, 0.0, 0.0), 1e-3));
    }

    #[test]
    fn the_shadow_camera_looks_down_from_the_light_in_steps() {
        let shadow = Shadow::default();
        let mut camera = DVec3::new(1000.3, 70.0, -20.7);
        // Noon: the light is straight up.
        let (view, projection) = shadow_camera(Vec3::Y, camera, &shadow);
        let at = |p: Vec3| view.transform_point3(p);
        // Straight down: a block below the player is farther from the light.
        assert!(at(Vec3::new(0.0, -1.0, 0.0)).z < at(Vec3::ZERO).z);
        // The view's origin is interval / 2 - camera % interval from the player on each axis,
        // the remainder with the camera's sign: up to a step and a half on negative axes.
        let origin = view.inverse().w_axis.truncate();
        assert!(
            origin.abs_diff_eq(Vec3::new(0.7, 1.0, 1.7), 1e-3),
            "{origin}"
        );
        // Moving within a step keeps the world where it was in the map.
        camera.x = 1000.6;
        let (moved, _) = shadow_camera(Vec3::Y, camera, &shadow);
        let world = Vec3::new(5.0, -3.0, 9.0);
        let shift = Vec3::new(0.3, 0.0, 0.0);
        assert!(
            moved
                .transform_point3(world - shift)
                .abs_diff_eq(at(world), 1e-3)
        );
        // shadowDistance blocks reach the map's edge.
        let edge = projection.project_point3(Vec3::new(160.0, 0.0, 0.0));
        assert!((edge.x.abs() - 1.0).abs() < 1e-4, "{edge}");
    }

    /// Iris's matrices at 3000 ticks with `sunPathRotation = 30` and the camera at
    /// (0.5, -58.38, 0.5), read from its uniforms by a probe pack.
    #[test]
    fn the_shadow_camera_matches_iris() {
        let shadow = Shadow {
            sun_path_rotation: 30.0,
            ..Shadow::default()
        };
        let light = Vec3::new(0.56782, 0.71287, -0.41158).normalize();
        let (view, projection) = shadow_camera(light, DVec3::new(0.5, -58.38, 0.5), &shadow);
        let iris_view = Mat4::from_cols_array_2d(&[
            [0.82315, 0.0, 0.56782, 0.0],
            [-0.49175, -0.5, 0.71287, 0.0],
            [0.28391, -0.86603, -0.41158, 0.0],
            [0.12508, 1.12301, -1.06188, 1.0],
        ]);
        assert!(view.abs_diff_eq(iris_view, 2e-4), "{view}");
        assert!(
            (projection.z_axis.z + 0.0078110).abs() < 1e-6,
            "{projection}"
        );
        assert!((projection.w_axis.z + 0.21851).abs() < 1e-4, "{projection}");
        // Elsewhere the origin keeps to the steps: Iris put it at (-0.5, 1.38, 2.5) here.
        let (view, _) = shadow_camera(light, DVec3::new(3.5, -58.38, -1.5), &shadow);
        let origin = view.inverse().w_axis.truncate();
        assert!(
            origin.abs_diff_eq(Vec3::new(-0.5, 1.38, 2.5), 1e-3),
            "{origin}"
        );
    }
}

#[cfg(test)]
mod initial_history_tests {
    use super::*;
    use crate::hud::HudBatch;
    fn float(u: &FrameUniforms, name: &str) -> [f32; 16] {
        match u.values[index(name).unwrap()] {
            Value::F(v) => v,
            _ => panic!("float uniform expected"),
        }
    }
    #[test]
    fn first_history_is_current_then_tracks_the_actual_previous_frame() {
        let hud = HudBatch::new([800, 450]);
        let mut f = frame(&hud);
        let mut u = FrameUniforms::default();
        f.camera = DVec3::new(1024.25, -3.5, 11.75);
        f.yaw = 21.0;
        f.pitch = -16.0;
        u.update(&f, &PackInputs::default(), [800, 450]);
        assert_eq!(
            float(&u, "previousCameraPosition"),
            float(&u, "cameraPosition")
        );
        for (prior, current) in [
            ("gbufferPreviousProjection", "gbufferProjection"),
            ("gbufferPreviousModelView", "gbufferModelView"),
        ] {
            assert_eq!(float(&u, prior), float(&u, current));
            assert!(float(&u, prior).iter().any(|v| *v != 0.0));
        }
        let camera = float(&u, "cameraPosition");
        let view = float(&u, "gbufferModelView");
        let projection = float(&u, "gbufferProjection");
        f.camera = DVec3::new(-100.0, 20.0, 40.0);
        f.yaw = 90.0;
        f.pitch = 50.0;
        f.fov_degrees = 100.0;
        u.update(&f, &PackInputs::default(), [1000, 500]);
        assert_eq!(float(&u, "previousCameraPosition"), camera);
        assert_eq!(float(&u, "gbufferPreviousModelView"), view);
        assert_eq!(float(&u, "gbufferPreviousProjection"), projection);
        assert_ne!(
            float(&u, "previousCameraPosition"),
            float(&u, "cameraPosition")
        );
    }
    #[test]
    fn environmental_smoothing_starts_at_the_first_inputs_without_resetting_each_frame() {
        let hud = HudBatch::new([800, 450]);
        let f = frame(&hud);
        let mut u = FrameUniforms::default();
        let mut input = PackInputs {
            rain: 0.75,
            eye_light: [3, 14],
            ..Default::default()
        };
        u.update(&f, &input, [800, 450]);
        assert_eq!(float(&u, "wetness")[0], 0.75);
        match u.values[index("eyeBrightnessSmooth").unwrap()] {
            Value::I(v) => assert_eq!(&v[..2], &[48, 224]),
            _ => panic!("integer uniform expected"),
        }
        input.rain = 0.0;
        input.eye_light = [0, 0];
        u.update(&f, &input, [800, 450]);
        assert!(float(&u, "wetness")[0] > 0.0);
        assert!(u.eye_smooth.iter().all(|v| *v > 0.0));
    }
    fn frame(hud: &HudBatch) -> Frame<'_> {
        Frame {
            eye: glam::DVec3::ZERO,
            eye_rotation: [0.0; 2],
            first_person: true,
            shadow_scene_revision: (0, 0),
            shadow_cache: false,
            water_plane: None,
            weather: &[],
            weather_time: crate::weather::Time::new(0, 0.0),
            camera: glam::DVec3::new(0.5, 100.0, 0.5),
            yaw: 0.0,
            pitch: 10.0,
            fov_degrees: 85.0,
            bob: glam::Mat4::IDENTITY,
            spin: glam::Mat4::IDENTITY,
            sky: crate::environment::Sky::default(),
            distance: 12.0,
            detail_distance: 12.0,
            depth_far: 1000.0,
            underwater: false,
            cave_culling: false,
            title: true,
            sky_blocked: false,
            skybox: false,
            end_sky: false,
            end_sky_sprite: None,
            environment_fog: None,
            water_fog: [0.0; 3],
            fog: None,
            dark_disc: false,
            outline: Vec::new(),
            outline_after_terrain: false,
            outline_high_contrast: false,
            debug_lines: &[],
            debug_lines_on_top: &[],
            debug_crosshair: &[],
            hud,
            glint_alpha: 0.75,
            glint_ticks: 0.0,
            nether_lighting: false,
            entities: &[],
            entity_motion: &[],
            colored_lights: &[],
            emissive_layers: &[],
            shadow_only: &[],
            entity_spans: &[],
            glowing: &[],
            dynamic_lights: &[],
            clouds: None,
            lightning: Default::default(),
            additive: &[],
            see_through: &[],
            celestial: &[],
            stars: None,
            hand: &[],
            hand_fov_degrees: 70.0,
            screen: &[],
            pack: Default::default(),
            border: None,
            game_time: 0.0,
            shader_game_time: 0.0,
            end_portal_sprites: [[0.0; 4]; 2],
            crumbling_sprite: [0.0; 4],
            ticks: 0,
            darkness: (0.0, 0.0),
            night_vision: 0.0,
            block_flicker: 0.0,
            boss_darken: 0.0,
            menu_blur: None,
            chunk_fade: 0.0,
            post_effects: crate::post_chain::Request::none(),
        }
    }
}
