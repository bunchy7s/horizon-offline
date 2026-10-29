//! Display/TextDisplay synced data (26.3), checked against vanilla accessors
//! and ClientboundSetEntityDataPacket's own stream codec.
use crate::text::Text;
use horizon_protocol::entity_data::Value;
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
    #[test]
    fn partial_updates_retain_values_and_bad_vectors_do_not_panic() {
        let mut d = TextDisplayData::default();
        d.apply(&[(24, 1, Value::Int(80)), (27, 0, Value::Byte(24))]);
        d.apply(&[(11, 0, Value::Floats(vec![1.0]))]);
        assert_eq!(d.line_width, 80);
        assert_eq!(d.align(), Align::Left);
        assert_eq!(d.translation, [0.0; 3]);
    }
}
