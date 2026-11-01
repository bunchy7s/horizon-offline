//! Display/TextDisplay synced data (26.3), checked against vanilla accessors
//! and ClientboundSetEntityDataPacket's own stream codec.
use crate::{
    save::{float, number},
    text::Text,
};
use horizon_protocol::entity_data::{Value, serializer};
use horizon_world::nbt::Tag;
use serde::Serialize;
use std::sync::Arc;

/// `TextDisplay.Align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Center,
    Left,
    Right,
}

/// `Display` and `TextDisplay`'s synced data (ids 8 to 27); the default is
/// what their `defineSynchedData` starts with.
#[derive(Clone, Debug, Serialize)]
pub struct TextDisplayData {
    pub interpolation_delay: i32,
    pub interpolation_duration: i32,
    pub teleport_duration: i32,
    pub translation: [f32; 3],
    pub scale: [f32; 3],
    pub left_rotation: [f32; 4],
    pub right_rotation: [f32; 4],
    pub billboard: u8,
    pub brightness: i32,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color: i32,
    /// Shared: a frame's interpolated copy of the data must not copy it.
    #[serde(skip)]
    pub text: Arc<Text>,
    pub line_width: i32,
    pub background: i32,
    pub opacity: i8,
    pub flags: u8,
}

impl Default for TextDisplayData {
    fn default() -> Self {
        Self {
            interpolation_delay: 0,
            interpolation_duration: 0,
            teleport_duration: 0,
            translation: [0.0; 3],
            scale: [1.0; 3],
            left_rotation: [0.0, 0.0, 0.0, 1.0],
            right_rotation: [0.0, 0.0, 0.0, 1.0],
            billboard: 0,
            brightness: -1,
            view_range: 1.0,
            shadow_radius: 0.0,
            shadow_strength: 1.0,
            width: 0.0,
            height: 0.0,
            glow_color: -1,
            text: Arc::new(Text::empty()),
            line_width: 200,
            background: 0x4000_0000,
            opacity: -1,
            flags: 0,
        }
    }
}

impl TextDisplayData {
    /// DisplayRenderer's billboard orientation followed by Transformation's T*L*S*R.
    pub fn matrix(&self, entity: [f32; 2], camera: [f32; 2]) -> glam::Mat4 {
        let [yaw, pitch] = match self.billboard {
            1 => [camera[0] - 180.0, entity[1]],
            2 => [entity[0], -camera[1]],
            3 => [camera[0] - 180.0, -camera[1]],
            _ => entity,
        };
        let orientation = glam::Quat::from_euler(
            glam::EulerRot::YXZ,
            -yaw * 0.017_453_292,
            pitch * 0.017_453_292,
            0.0,
        );
        glam::Mat4::from_quat(orientation)
            * glam::Mat4::from_translation(self.translation.into())
            * glam::Mat4::from_quat(glam::Quat::from_array(self.left_rotation))
            * glam::Mat4::from_scale(self.scale.into())
            * glam::Mat4::from_quat(glam::Quat::from_array(self.right_rotation))
    }

    pub fn blend(&self, from: &Self, t: f32) -> Self {
        let mut out = self.clone();
        for i in 0..3 {
            out.translation[i] =
                from.translation[i] + t * (self.translation[i] - from.translation[i]);
            out.scale[i] = from.scale[i] + t * (self.scale[i] - from.scale[i]);
        }
        out.left_rotation = glam::Quat::from_array(from.left_rotation)
            .slerp(glam::Quat::from_array(self.left_rotation), t)
            .to_array();
        out.right_rotation = glam::Quat::from_array(from.right_rotation)
            .slerp(glam::Quat::from_array(self.right_rotation), t)
            .to_array();
        // Display$LinearFloatInterpolator: Mth.lerp(t, previous, current).
        out.shadow_radius = from.shadow_radius + t * (self.shadow_radius - from.shadow_radius);
        out.shadow_strength =
            from.shadow_strength + t * (self.shadow_strength - from.shadow_strength);
        let int = |a: i32, b: i32| a + (t * (b as f32 - a as f32)).floor() as i32;
        out.opacity = int(i32::from(from.opacity), i32::from(self.opacity)) as i8;
        out.background = (0..4).fold(0, |color, channel| {
            color
                | (int(
                    (from.background as u32 >> (channel * 8) & 255) as i32,
                    (self.background as u32 >> (channel * 8) & 255) as i32,
                ) << (channel * 8))
        });
        out
    }
}

/// `Display.updateCulling`: either edge at 0 sets `noCulling`, so the renderer
/// skips the frustum box (`DisplayRenderer.affectedByCulling`).
pub fn no_culling(width: f32, height: f32) -> bool {
    width == 0.0 || height == 0.0
}

/// `Display.updateCulling` / `getInterpolatedBoundingBox` at an interpolated
/// origin: the AABB from `(x − width/2, y, z − width/2)` to
/// `(x + width/2, y + height, z + width/2)`.
pub fn culling_box(origin: glam::DVec3, width: f32, height: f32) -> (glam::DVec3, glam::DVec3) {
    let half = f64::from(width) * 0.5;
    let tall = f64::from(height);
    (
        glam::DVec3::new(origin.x - half, origin.y, origin.z - half),
        glam::DVec3::new(origin.x + half, origin.y + tall, origin.z + half),
    )
}

impl TextDisplayData {
    /// Omitted fields retain their previous values.
    pub fn apply(&mut self, values: &[(u8, i32, Value)]) {
        for (id, _, value) in values {
            match (id, value) {
                (8, Value::Int(v)) => self.interpolation_delay = *v,
                (9, Value::Int(v)) => self.interpolation_duration = *v,
                (10, Value::Int(v)) => self.teleport_duration = *v,
                (11, Value::Floats(v)) if v.len() == 3 => self.translation.copy_from_slice(v),
                (12, Value::Floats(v)) if v.len() == 3 => self.scale.copy_from_slice(v),
                (13, Value::Floats(v)) if v.len() == 4 => self.left_rotation.copy_from_slice(v),
                (14, Value::Floats(v)) if v.len() == 4 => self.right_rotation.copy_from_slice(v),
                (15, Value::Byte(v)) => self.billboard = *v as u8,
                (16, Value::Int(v)) => self.brightness = *v,
                (17, Value::Float(v)) => self.view_range = *v,
                (18, Value::Float(v)) => self.shadow_radius = *v,
                (19, Value::Float(v)) => self.shadow_strength = *v,
                (20, Value::Float(v)) => self.width = *v,
                (21, Value::Float(v)) => self.height = *v,
                (22, Value::Int(v)) => self.glow_color = *v,
                (23, Value::Text(v)) => self.text = Arc::new(crate::entity_nbt::name_text(v)),
                (24, Value::Int(v)) => self.line_width = *v,
                (25, Value::Int(v)) => self.background = *v,
                (26, Value::Byte(v)) => self.opacity = *v,
                (27, Value::Byte(v)) => self.flags = *v as u8,
                _ => {}
            }
        }
    }
    /// `TextDisplay.getAlign`: the left flag wins over the right one.
    pub fn align(&self) -> Align {
        if self.flags & 8 != 0 {
            Align::Left
        } else if self.flags & 16 != 0 {
            Align::Right
        } else {
            Align::Center
        }
    }
}

/// One synched data entry: kept when it differs from its default, or always
/// with `all` (the defaults a client is sent when a value goes back to one).
fn entry(
    out: &mut Vec<(u8, i32, Value)>,
    all: bool,
    id: u8,
    kind: i32,
    value: Value,
    default: Value,
) {
    if all || value != default {
        out.push((id, kind, if all { default } else { value }));
    }
}

/// `Display.readAdditionalSaveData`: the synched data entries (ids 8 to 22)
/// of a saved display, or with `all` their defaults.
pub fn display_data(tag: &Tag, all: bool) -> Vec<(u8, i32, Value)> {
    let int = |key: &str| number(tag.get(key)).map(|v| v as i32);
    let real = |key: &str| float(tag.get(key)).map(|v| v as f32);
    let mut out = Vec::new();
    let mut set = |id: u8, kind: i32, value: Value, default: Value| {
        entry(&mut out, all, id, kind, value, default);
    };
    let defaults = TextDisplayData::default();
    // The start delay, the transformation duration, the teleport duration (0 to 59).
    let ints = [
        (8, "start_interpolation", 0, i32::MIN, i32::MAX),
        (9, "interpolation_duration", 0, i32::MIN, i32::MAX),
        (10, "teleport_duration", 0, 0, 59),
        (22, "glow_color_override", -1, i32::MIN, i32::MAX),
    ];
    for (id, key, default, low, high) in ints {
        let value = int(key).unwrap_or(default).clamp(low, high);
        set(id, serializer::INT, Value::Int(value), Value::Int(default));
    }
    // `Transformation.EXTENDED_CODEC` (four parts or MATRIX4F), else `Transformation.IDENTITY`.
    let t = tag.get("transformation").and_then(transformation);
    let (translation, left, scale, right) = t.unwrap_or((
        defaults.translation,
        defaults.left_rotation,
        defaults.scale,
        defaults.right_rotation,
    ));
    let parts = [
        (
            11,
            serializer::VECTOR3,
            translation.to_vec(),
            defaults.translation.to_vec(),
        ),
        (
            12,
            serializer::VECTOR3,
            scale.to_vec(),
            defaults.scale.to_vec(),
        ),
        (
            13,
            serializer::QUATERNION,
            left.to_vec(),
            defaults.left_rotation.to_vec(),
        ),
        (
            14,
            serializer::QUATERNION,
            right.to_vec(),
            defaults.right_rotation.to_vec(),
        ),
    ];
    for (id, kind, value, default) in parts {
        set(id, kind, Value::Floats(value), Value::Floats(default));
    }
    let billboard = match tag.get("billboard") {
        Some(Tag::String(s)) => match s.to_string_lossy().as_str() {
            "vertical" => 1,
            "horizontal" => 2,
            "center" => 3,
            _ => 0,
        },
        _ => 0,
    };
    set(15, serializer::BYTE, Value::Byte(billboard), Value::Byte(0));
    // `Brightness.pack`: block light in bits 4 to 7, sky light in bits 20 to 23.
    let brightness = tag.get("brightness").and_then(|b| {
        let light = |key: &str| number(b.get(key)).filter(|v| (0..=15).contains(v));
        Some((light("block")? as i32) << 4 | (light("sky")? as i32) << 20)
    });
    set(
        16,
        serializer::INT,
        Value::Int(brightness.unwrap_or(-1)),
        Value::Int(-1),
    );
    for (id, key, default) in [
        (17, "view_range", 1.0),
        (18, "shadow_radius", 0.0),
        (19, "shadow_strength", 1.0),
        (20, "width", 0.0),
        (21, "height", 0.0),
    ] {
        let value = real(key).unwrap_or(default);
        set(
            id,
            serializer::FLOAT,
            Value::Float(value),
            Value::Float(default),
        );
    }
    out
}

/// `TextDisplay.readAdditionalSaveData`: the text, line width, background,
/// opacity and style flags (ids 23 to 27), or with `all` their defaults.
pub fn text_display_data(tag: &Tag, all: bool) -> Vec<(u8, i32, Value)> {
    let int = |key: &str| number(tag.get(key)).map(|v| v as i32);
    let mut out = Vec::new();
    let mut set = |id: u8, kind: i32, value: Value, default: Value| {
        entry(&mut out, all, id, kind, value, default);
    };
    let empty = Tag::String("".into());
    set(
        23,
        serializer::COMPONENT,
        Value::Text(tag.get("text").unwrap_or(&empty).clone()),
        Value::Text(empty.clone()),
    );
    set(
        24,
        serializer::INT,
        Value::Int(int("line_width").unwrap_or(200)),
        Value::Int(200),
    );
    let background = Value::Int(int("background").unwrap_or(0x4000_0000));
    set(25, serializer::INT, background, Value::Int(0x4000_0000));
    let opacity = Value::Byte(int("text_opacity").unwrap_or(-1) as i8);
    set(26, serializer::BYTE, opacity, Value::Byte(-1));
    // Shadow 1, see-through 2, default background 4, then the alignment: left 8, right 16.
    let flag = |key: &str, bit: i8| {
        if int(key).is_some_and(|v| v != 0) {
            bit
        } else {
            0
        }
    };
    let align = match tag.get("alignment") {
        Some(Tag::String(s)) => match s.to_string_lossy().as_str() {
            "left" => 8,
            "right" => 16,
            _ => 0,
        },
        _ => 0,
    };
    let flags = flag("shadow", 1) | flag("see_through", 2) | flag("default_background", 4) | align;
    set(27, serializer::BYTE, Value::Byte(flags), Value::Byte(0));
    out
}

/// `Transformation.EXTENDED_CODEC`'s translation, left rotation, scale and
/// right rotation: all four parts, or a `MATRIX4F` (sixteen floats, row-major)
/// that `new Transformation(matrix)` decomposes (`MatrixUtil.svdDecompose`).
fn transformation(tag: &Tag) -> Option<([f32; 3], [f32; 4], [f32; 3], [f32; 4])> {
    use horizon_assets::math::{
        Mat4, normalize_quaternion, quaternion_from_axis_angle, svd_decompose,
    };
    let list = |tag: Option<&Tag>| -> Option<Vec<f32>> {
        match tag? {
            Tag::List(l) => l.iter().map(|v| float(Some(v)).map(|v| v as f32)).collect(),
            _ => None,
        }
    };
    if let Some(values) = list(Some(tag)) {
        let values: [f32; 16] = values.try_into().ok()?;
        return Some(svd_decompose(&Mat4::from_row_major(values)));
    }
    let vector = |key: &str| -> Option<[f32; 3]> { list(tag.get(key))?.try_into().ok() };
    // `ExtraCodecs.QUATERNIONF`: four components (normalized) or an axis and an
    // angle (`new Quaternionf(AxisAngle4f)`: the angle wrapped, the axis as given).
    let rotation = |key: &str| -> Option<[f32; 4]> {
        let value = tag.get(key)?;
        if let Some(q) = list(Some(value)) {
            return Some(normalize_quaternion(q.try_into().ok()?));
        }
        let angle = float(value.get("angle"))? as f32;
        let axis: [f32; 3] = list(value.get("axis"))?.try_into().ok()?;
        Some(quaternion_from_axis_angle(angle, axis))
    };
    Some((
        vector("translation")?,
        rotation("left_rotation")?,
        vector("scale")?,
        rotation("right_rotation")?,
    ))
}

/// `TextDisplay.CachedInfo`: each line and its width, and the widest.
pub struct Lines {
    pub lines: Vec<(Text, f32)>,
    pub width: f32,
}

impl Lines {
    /// `TextDisplayRenderer.splitLines`: Font.split, no indent on wrapped lines.
    pub fn split(font: &crate::font::Font, text: &Text, line_width: i32) -> Self {
        let lines: Vec<_> = font
            .split_formatted(text, line_width as f32)
            .into_iter()
            .map(|line| {
                let width = font.width_text(&line).ceil();
                (line, width)
            })
            .collect();
        let width = lines.iter().map(|l| l.1).fold(0.0, f32::max);
        Self { lines, width }
    }
    /// Ten units a line, less the spacing under the last.
    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * 10.0 - 1.0
    }
    /// `submitInner`'s pose for the text, after the display's own: half a
    /// turn about y, the mirrored 0.025 scale and `translate(1 - width / 2,
    /// -height, 0)`.
    pub fn pose(&self) -> glam::Mat4 {
        glam::Mat4::from_rotation_y(std::f32::consts::PI)
            * glam::Mat4::from_scale(glam::Vec3::splat(-0.025))
            * glam::Mat4::from_translation(glam::Vec3::new(
                1.0 - self.width / 2.0,
                -self.height(),
                0.0,
            ))
    }
}

/// The corners of each six-vertex `HudBatch` quad that `draw` fills, in the
/// order the world draws them: `Font`'s glyphs go top left, bottom left,
/// bottom right, top right, so under `Lines::pose` they wind
/// counter-clockwise on the side the text reads from, the face the culled
/// `TEXT_POLYGON_OFFSET` and `TEXT_SEE_THROUGH` keep.
pub const CORNERS: [usize; 4] = [5, 2, 1, 0];

/// `TextDisplayRenderer.submitInner` after its `translate(1 - width / 2,
/// -height, 0)`: the background and each line, in text units, each quad's
/// `depth` its z (the background 0.01 behind, `Font.prepareBackground`).
/// `default_background` is `Options.getBackgroundOpacity(0.25)` as a colour.
pub fn draw(
    batch: &mut crate::hud::HudBatch,
    atlas: &crate::hud::HudAtlas,
    d: &TextDisplayData,
    lines: &Lines,
    default_background: u32,
) {
    use crate::hud::rgba;
    let (width, height) = (lines.width, lines.height());
    let bg = if d.flags & 4 != 0 {
        default_background
    } else {
        d.background as u32
    };
    if bg != 0 {
        let start = batch.vertices.len();
        batch.fill(
            atlas,
            // submitTextBackground(-1, -1, width, height): corners, not a size.
            -1.0,
            -1.0,
            width + 1.0,
            height + 1.0,
            rgba(
                (bg >> 16) as u8,
                (bg >> 8) as u8,
                bg as u8,
                (bg >> 24) as u8,
            ),
        );
        for v in &mut batch.vertices[start..] {
            v.depth = -0.01;
        }
    }
    let align = d.align();
    for (i, (line, line_width)) in lines.lines.iter().enumerate() {
        let x = match align {
            Align::Left => 0.0,
            Align::Right => width - line_width,
            Align::Center => width / 2.0 - line_width / 2.0,
        };
        batch.text_styled_world(
            atlas,
            line,
            x,
            i as f32 * 10.0,
            rgba(255, 255, 255, d.opacity as u8),
            d.flags & 1 != 0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::Compound;

    /// Whether a text quad seen from `camera` (yaw, pitch, at `eye` from the
    /// display) faces it (counter-clockwise on screen, the face back-face
    /// culling keeps) and whether its text reads left to right.
    fn seen(d: &TextDisplayData, entity: [f32; 2], camera: [f32; 2], eye: glam::Vec3) -> [bool; 2] {
        let lines = Lines {
            lines: vec![],
            width: 10.0,
        };
        let mut batch = crate::hud::HudBatch::new([1, 1]);
        batch.quad(
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
        let view_proj = glam::Mat4::perspective_rh(1.2, 1.0, 1000.0, 0.05)
            * crate::camera::view_matrix(camera[0], camera[1])
            * glam::Mat4::from_translation(-eye);
        let matrix = view_proj * d.matrix(entity, camera) * lines.pose();
        let [a, b, c, _] = CORNERS.map(|i| {
            let v = batch.vertices[i];
            matrix.project_point3(glam::Vec3::new(v.pos[0], v.pos[1], v.depth))
        });
        let ccw = (b - a).truncate().perp_dot((c - a).truncate()) > 0.0;
        // The text's x axis, from the left edge to the right, on screen.
        let right =
            matrix.project_point3(glam::Vec3::X).x > matrix.project_point3(glam::Vec3::ZERO).x;
        [ccw, right]
    }

    #[test]
    fn text_quads_face_the_side_they_read_from() {
        let mut d = TextDisplayData::default();
        // A fixed display at yaw 0 reads from the south: in front it faces
        // the camera, from the north it shows its back, which the text
        // pipelines cull.
        assert_eq!(
            seen(&d, [0.0, 0.0], [180.0, 0.0], glam::Vec3::Z * 3.0),
            [true, true]
        );
        assert_eq!(
            seen(&d, [0.0, 0.0], [0.0, 0.0], glam::Vec3::Z * -3.0),
            [false, false]
        );
        assert_eq!(
            seen(&d, [90.0, 0.0], [-90.0, 0.0], glam::Vec3::X * -3.0),
            [true, true]
        );
        // A mirrored scale turns the readable side round with the front.
        d.scale = [-1.0, 1.0, 1.0];
        assert_eq!(
            seen(&d, [0.0, 0.0], [0.0, 0.0], glam::Vec3::Z * -3.0),
            [true, true]
        );
        assert_eq!(
            seen(&d, [0.0, 0.0], [180.0, 0.0], glam::Vec3::Z * 3.0),
            [false, false]
        );
        // Billboards face the camera from anywhere it looks at them.
        d.scale = [1.0; 3];
        for billboard in 1..=3 {
            d.billboard = billboard;
            for yaw in [0.0, 37.0, 90.0, 180.0, -135.0] {
                let pitch = if billboard == 1 { 0.0 } else { 30.0 };
                let forward = glam::Vec3::new(
                    -f32::to_radians(yaw).sin() * f32::to_radians(pitch).cos(),
                    -f32::to_radians(pitch).sin(),
                    f32::to_radians(yaw).cos() * f32::to_radians(pitch).cos(),
                );
                let entity = if billboard == 2 {
                    [180.0 + yaw, 0.0]
                } else {
                    [0.0, 0.0]
                };
                assert_eq!(
                    seen(&d, entity, [yaw, pitch], forward * -3.0),
                    [true, true],
                    "billboard {billboard} at yaw {yaw}"
                );
            }
        }
    }
    #[test]
    fn partial_updates_retain_values_and_bad_vectors_do_not_panic() {
        let mut d = TextDisplayData::default();
        d.apply(&[(24, 1, Value::Int(80)), (27, 0, Value::Byte(24))]);
        d.apply(&[(11, 0, Value::Floats(vec![1.0]))]);
        assert_eq!(d.line_width, 80);
        assert_eq!(d.align(), Align::Left);
        assert_eq!(d.translation, [0.0; 3]);
    }
    #[test]
    fn a_matrix_transformation_is_decomposed() {
        let matrix = [
            2.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0, -2.0, 0.0, 0.0, 0.5, 0.25, 0.0, 0.0, 0.0, 1.0,
        ];
        let tag = Tag::Compound(
            [("transformation".into(), crate::save::floats(&matrix))]
                .into_iter()
                .collect(),
        );
        let data = display_data(&tag, false);
        let part = |id: u8| data.iter().find(|e| e.0 == id).map(|e| e.2.clone());
        assert_eq!(part(11), Some(Value::Floats(vec![1.0, -2.0, 0.25])));
        assert_eq!(part(12), Some(Value::Floats(vec![2.0, 3.0, 0.5])));
        assert_eq!(part(13), None);
    }
    #[test]
    fn axis_angle_rotations_keep_the_axis_and_wrap_the_angle() {
        let mut rotation = Compound::new();
        rotation.insert(
            "angle".into(),
            Tag::Float((-std::f32::consts::PI).to_bits()),
        );
        rotation.insert("axis".into(), crate::save::floats(&[0.0, 0.0, 2.0]));
        let mut t = Compound::new();
        t.insert("translation".into(), crate::save::floats(&[0.0; 3]));
        t.insert("scale".into(), crate::save::floats(&[1.0; 3]));
        t.insert("left_rotation".into(), Tag::Compound(rotation));
        t.insert(
            "right_rotation".into(),
            crate::save::floats(&[0.0, 0.0, 0.0, 2.0]),
        );
        let (_, left, _, right) = transformation(&Tag::Compound(t)).unwrap();
        assert_eq!(
            left,
            horizon_assets::math::quaternion_from_axis_angle(
                -std::f32::consts::PI,
                [0.0, 0.0, 2.0]
            )
        );
        assert!(left[2] > 1.9);
        assert_eq!(right, [0.0, 0.0, 0.0, 1.0]);
    }

    /// `Display.updateCulling`: width or height 0 disables the frustum box.
    #[test]
    fn zero_width_or_height_disables_display_culling() {
        assert!(no_culling(0.0, 1.0));
        assert!(no_culling(2.0, 0.0));
        assert!(no_culling(0.0, 0.0));
        assert!(!no_culling(2.0, 1.0));
    }

    /// `Display.updateCulling`: the box is width-centred on XZ and rises by height.
    #[test]
    fn display_culling_box_is_centred_on_xz_and_stands_on_y() {
        let (lo, hi) = culling_box(glam::DVec3::new(10.0, 64.0, -3.0), 2.0, 1.0);
        assert_eq!(lo, glam::DVec3::new(9.0, 64.0, -4.0));
        assert_eq!(hi, glam::DVec3::new(11.0, 65.0, -2.0));
    }

    /// `Display$LinearFloatInterpolator`: the shadow radius and strength blend
    /// across the interpolation duration with the transformation's progress.
    #[test]
    fn the_blend_interpolates_shadow_radius_and_strength() {
        let from = TextDisplayData::default();
        let shown = TextDisplayData {
            shadow_radius: 2.0,
            shadow_strength: 0.25,
            ..TextDisplayData::default()
        };
        let half = shown.blend(&from, 0.5);
        assert_eq!((half.shadow_radius, half.shadow_strength), (1.0, 0.625));
        assert_eq!(shown.blend(&from, 0.0).shadow_radius, 0.0);
        assert_eq!(shown.blend(&from, 1.0).shadow_strength, 0.25);
    }
}
