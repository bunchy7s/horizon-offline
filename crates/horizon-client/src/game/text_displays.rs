//! Server text displays: metadata, interpolation, billboard transforms and world text.
use super::Game;
use horizon_client::{
    gfx::TEXT_POLYGON_OFFSET_OVERLAY,
    hud::HudBatch,
    mesher::Vertex,
    text_display::{self, Lines, TextDisplayData},
};
use horizon_protocol::entity_data::Value;

pub(super) struct RemoteTextDisplay {
    pub data: TextDisplayData,
    from: TextDisplayData,
    shown: TextDisplayData,
    position: glam::DVec3,
    previous: glam::DVec3,
    target: glam::DVec3,
    rotation: [f32; 2],
    old_rotation: [f32; 2],
    target_rotation: [f32; 2],
    steps: i32,
    age: i64,
    start: i64,
    duration: i32,
    dirty: bool,
    start_dirty: bool,
    duration_dirty: bool,
    initialized: bool,
    /// `Display.lastProgress`: the progress the last frame drew, where a new
    /// interpolation starts from.
    last_progress: f32,
    /// `TextDisplay.clientDisplayCache`: the split lines, until the render
    /// state changes.
    lines: Option<Lines>,
}

/// `Display.RENDER_STATE_IDS` and `TextDisplay.TEXT_RENDER_STATE_IDS`: the
/// data a render state is built from (not the view range, size or glow).
fn renders(id: u8) -> bool {
    matches!(id, 11..=16 | 18 | 19 | 23..=27)
}

impl RemoteTextDisplay {
    pub fn new(position: [f64; 3], yaw: f32) -> Self {
        Self {
            data: Default::default(),
            from: Default::default(),
            shown: Default::default(),
            position: position.into(),
            previous: position.into(),
            target: position.into(),
            rotation: [yaw, 0.0],
            old_rotation: [yaw, 0.0],
            target_rotation: [yaw, 0.0],
            steps: 0,
            age: 0,
            start: 0,
            duration: 0,
            dirty: true,
            start_dirty: false,
            duration_dirty: false,
            initialized: false,
            last_progress: 0.0,
            lines: None,
        }
    }
    fn progress(&self, partial: f32) -> f32 {
        if self.duration <= 0 {
            1.0
        } else {
            (((self.age - self.start) as f32 + partial) / self.duration as f32).clamp(0.0, 1.0)
        }
    }
    pub fn metadata(&mut self, values: &[(u8, i32, Value)]) {
        self.start_dirty |= values.iter().any(|v| v.0 == 8);
        self.duration_dirty |= values.iter().any(|v| v.0 == 9);
        self.dirty |= values.iter().any(|v| renders(v.0));
        self.data.apply(values);
    }
    fn tick(&mut self) {
        self.age += 1;
        self.previous = self.position;
        self.old_rotation = self.rotation;
        if self.steps > 0 {
            self.position += (self.target - self.position) / f64::from(self.steps);
            for i in 0..2 {
                self.rotation[i] +=
                    angle(self.target_rotation[i] - self.rotation[i]) / self.steps as f32;
            }
            self.steps -= 1;
        }
        if self.start_dirty {
            self.start = self.age + i64::from(self.data.interpolation_delay);
        }
        if self.duration_dirty {
            self.duration = self.data.interpolation_duration;
        }
        if self.dirty {
            self.from = if self.initialized && self.duration > 0 {
                self.shown.blend(&self.from, self.last_progress)
            } else {
                self.data.clone()
            };
            self.shown = self.data.clone();
            self.initialized = true;
            self.lines = None;
        }
        self.dirty = false;
        self.start_dirty = false;
        self.duration_dirty = false;
    }
    fn move_to(&mut self, position: [f64; 3], rotation: Option<(f32, f32)>) {
        self.target = position.into();
        if let Some((yaw, pitch)) = rotation {
            self.target_rotation = [yaw, pitch];
        }
        self.steps = self.data.teleport_duration.clamp(0, 59);
        if self.steps == 0 {
            self.position = self.target;
            self.previous = self.target;
            self.rotation = self.target_rotation;
            self.old_rotation = self.rotation;
        }
    }
}
fn angle(d: f32) -> f32 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

impl Game<'_> {
    pub(super) fn move_remote_text_display(
        &mut self,
        id: i32,
        position: [f64; 3],
        rotation: Option<(f32, f32)>,
    ) {
        if let Some(d) = self.text_displays.get_mut(&id) {
            d.move_to(position, rotation);
        }
    }
    pub(super) fn tick_text_displays(&mut self) {
        for display in self.text_displays.values_mut() {
            display.tick();
        }
    }
    pub(super) fn text_display_geometry(
        &mut self,
        camera: glam::DVec3,
        partial: f64,
        normal: &mut Vec<Vertex>,
        see_through: &mut Vec<Vertex>,
    ) {
        self.text_display_vertices = 0;
        self.mark_translucent_entity(normal.len(), "minecraft:text_display");
        let atlas = &self.gui.atlas;
        let front = self.camera_type == super::perspective::CameraType::ThirdPersonFront;
        let angles = if front {
            [self.yaw + 180.0, -self.pitch]
        } else {
            [self.yaw, self.pitch]
        };
        let scale = self.view_scale();
        for display in self.text_displays.values_mut() {
            display.last_progress = display.progress(partial as f32);
            let d = display.shown.blend(&display.from, display.last_progress);
            let position = display.previous.lerp(display.position, partial);
            let range = f64::from(d.view_range) * 64.0 * scale;
            if position.distance_squared(camera) >= range * range || d.text.is_empty() {
                continue;
            }
            let rotation = std::array::from_fn(|i| {
                display.old_rotation[i]
                    + angle(display.rotation[i] - display.old_rotation[i]) * partial as f32
            });
            let lines = display
                .lines
                .get_or_insert_with(|| Lines::split(&atlas.font, &d.text, d.line_width));
            let matrix = glam::Mat4::from_translation((position - camera).as_vec3())
                * d.matrix(rotation, angles)
                * lines.pose();
            if !matrix.is_finite() {
                continue;
            }
            let mut batch = HudBatch::new([1, 1]);
            text_display::draw(
                &mut batch,
                atlas,
                &d,
                lines,
                self.settings.text_background(0.25),
            );
            let world_light = self.world.light(
                position.x.floor() as i32,
                position.y.floor() as i32,
                position.z.floor() as i32,
            );
            let light = if d.brightness == -1 {
                horizon_client::block_entity_render::vertex_light(world_light)
            } else {
                ((d.brightness as u32 >> 20 & 15) * 16)
                    | (((d.brightness as u32 >> 4 & 15) * 16) << 8)
            };
            // Font.DisplayMode: SEE_THROUGH over everything, else POLYGON_OFFSET.
            let through = d.flags & 2 != 0;
            let (out, layer) = if through {
                (&mut *see_through, 2)
            } else {
                (&mut *normal, TEXT_POLYGON_OFFSET_OVERLAY)
            };
            for quad in batch.vertices.as_chunks::<6>().0 {
                for i in [0, 1, 2, 5] {
                    let v = quad[i];
                    let point =
                        matrix.transform_point3(glam::Vec3::new(v.pos[0], v.pos[1], v.depth));
                    out.push(Vertex {
                        pos: point.to_array(),
                        uv: v.uv,
                        color: v.color,
                        light,
                        layer,
                        normal: [0; 4],

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
            self.text_display_vertices += batch.vertices.len() / 6 * 4;
            self.map_patches.extend(batch.patches);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_and_position_interpolation_keep_their_separate_clocks() {
        let mut d = RemoteTextDisplay::new([0.0; 3], 0.0);
        d.tick();
        d.metadata(&[
            (8, 1, Value::Int(0)),
            (9, 1, Value::Int(4)),
            (10, 1, Value::Int(2)),
            (11, 0, Value::Floats(vec![4.0, 0.0, 0.0])),
        ]);
        d.tick();
        assert_eq!(d.shown.blend(&d.from, d.progress(0.5)).translation[0], 0.5);
        d.move_to([2.0, 0.0, 0.0], Some((90.0, 0.0)));
        d.tick();
        assert_eq!(d.position.x, 1.0);
        assert_eq!(d.rotation[0], 45.0);
        d.tick();
        assert_eq!(d.position.x, 2.0);
        assert_eq!(d.shown.blend(&d.from, d.progress(0.0)).translation[0], 2.0);
    }
}
