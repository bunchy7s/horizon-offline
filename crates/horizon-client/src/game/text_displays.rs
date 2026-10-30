//! Server text displays: metadata, interpolation, billboard transforms and world text.
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    gfx::TEXT_POLYGON_OFFSET_OVERLAY,
    hud::HudBatch,
    items::ItemStack,
    mesher::Vertex,
    text_display::{self, Lines, TextDisplayData},
};
use horizon_protocol::entity_data::Value;
use horizon_world::registry::BlockStateId;

/// What a display shows besides its transformation: its text (in `data`),
/// a block state (`BlockDisplay`) or an item and how it is displayed
/// (`ItemDisplay`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Shows {
    Text,
    Block,
    Item,
}

pub(super) struct RemoteTextDisplay {
    pub data: TextDisplayData,
    shows: Shows,
    /// `BlockDisplay.DATA_BLOCK_STATE_ID`.
    block: Option<BlockStateId>,
    /// `ItemDisplay.DATA_ITEM_STACK` and `DATA_ITEM_DISPLAY_ID`.
    item: Option<ItemStack>,
    item_display: u8,
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
    /// `Entity.vehicle`: the mob it rides (`SetPassengers`), which places it
    /// every tick (`positionRider`), as NPC plugins hang a name over a head.
    pub(super) vehicle: Option<u32>,
}

/// `Display.RENDER_STATE_IDS` and `TextDisplay.TEXT_RENDER_STATE_IDS`: the
/// data a render state is built from (not the view range, size or glow).
fn renders(id: u8) -> bool {
    matches!(id, 11..=16 | 18 | 19 | 23..=27)
}

impl RemoteTextDisplay {
    pub fn new(shows: Shows, position: [f64; 3], yaw: f32) -> Self {
        Self {
            data: Default::default(),
            shows,
            block: None,
            item: None,
            item_display: 0,
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
            vehicle: None,
        }
    }
    /// `Entity.positionRider` through `setPos`: where its vehicle seats it this
    /// tick. The previous position stays the last tick's, so it moves smoothly
    /// with the vehicle; a teleport's interpolation no longer applies.
    pub(super) fn ride(&mut self, at: glam::DVec3) {
        self.position = at;
        self.target = at;
        self.steps = 0;
    }
    fn progress(&self, partial: f32) -> f32 {
        if self.duration <= 0 {
            1.0
        } else {
            (((self.age - self.start) as f32 + partial) / self.duration as f32).clamp(0.0, 1.0)
        }
    }
    /// The entries after `Display`'s: a block state, or an item with its
    /// display context (the stack comes converted by the caller).
    pub fn show(&mut self, values: &[(u8, i32, Value)], item: Option<Option<ItemStack>>) {
        for (id, _, value) in values {
            match (self.shows, id, value) {
                (Shows::Block, 23, Value::Id(state)) => {
                    self.block = Some(BlockStateId(u32::try_from(*state).unwrap_or(0)));
                }
                (Shows::Item, 24, Value::Byte(context)) => self.item_display = *context as u8,
                _ => {}
            }
        }
        if let Some(item) = item
            && self.shows == Shows::Item
        {
            self.item = item;
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
    /// The camera's yaw and pitch as `DisplayRenderer` reads them (the
    /// front view turns around).
    fn display_camera_angles(&self) -> [f32; 2] {
        if self.camera_type == super::perspective::CameraType::ThirdPersonFront {
            [self.yaw + 180.0, -self.pitch]
        } else {
            [self.yaw, self.pitch]
        }
    }
    /// `DisplayRenderer.getBlockLightLevel` and `getSkyLightLevel`: the
    /// brightness override, else the light at the entity.
    fn display_light(
        world: &horizon_client::world::World,
        d: &TextDisplayData,
        position: glam::DVec3,
    ) -> u32 {
        if d.brightness == -1 {
            let world_light = world.light(
                position.x.floor() as i32,
                position.y.floor() as i32,
                position.z.floor() as i32,
            );
            horizon_client::block_entity_render::vertex_light(world_light)
        } else {
            ((d.brightness as u32 >> 20 & 15) * 16) | (((d.brightness as u32 >> 4 & 15) * 16) << 8)
        }
    }
    /// `DisplayRenderer.BlockDisplayRenderer` and `ItemDisplayRenderer`: the
    /// block model or the item under the display's transformation, lit like
    /// any entity and never tinted by a hurt overlay.
    pub(super) fn block_item_display_geometry(
        &mut self,
        camera: glam::DVec3,
        partial: f64,
        out: &mut Vec<Vertex>,
    ) {
        let angles = self.display_camera_angles();
        let scale = self.view_scale();
        let mut displays = std::mem::take(&mut self.text_displays);
        for shows in [Shows::Block, Shows::Item] {
            let kind = if shows == Shows::Block {
                "minecraft:block_display"
            } else {
                "minecraft:item_display"
            };
            self.mark_entity(out.len(), kind);
            for (id, display) in displays.iter_mut().filter(|(_, d)| d.shows == shows) {
                display.last_progress = display.progress(partial as f32);
                let d = display.shown.blend(&display.from, display.last_progress);
                let position = display.previous.lerp(display.position, partial);
                let range = f64::from(d.view_range) * 64.0 * scale;
                if position.distance_squared(camera) >= range * range {
                    continue;
                }
                let rotation = std::array::from_fn(|i| {
                    display.old_rotation[i]
                        + angle(display.rotation[i] - display.old_rotation[i]) * partial as f32
                });
                let matrix = glam::Mat4::from_translation((position - camera).as_vec3())
                    * d.matrix(rotation, angles);
                if !matrix.is_finite() {
                    continue;
                }
                let light = Self::display_light(&self.world, &d, position);
                if let Some(state) = display.block {
                    // BlockModelResolver.update: model seed 42.
                    self.block_model_vertices(state, 42, matrix, light, true, out);
                } else if let Some(stack) = &display.item {
                    let context = match display.item_display {
                        1 => DisplayContext::ThirdPersonLeftHand,
                        2 => DisplayContext::ThirdPersonRightHand,
                        3 => DisplayContext::FirstPersonLeftHand,
                        4 => DisplayContext::FirstPersonRightHand,
                        5 => DisplayContext::Head,
                        6 => DisplayContext::Gui,
                        7 => DisplayContext::Ground,
                        8 => DisplayContext::Fixed,
                        9 => DisplayContext::OnShelf,
                        _ => DisplayContext::None,
                    };
                    // updateForNonLiving: seeded by the entity id.
                    let state = self.resolve_item(stack, context, None, *id);
                    let pose = matrix * glam::Mat4::from_rotation_y(std::f32::consts::PI);
                    self.submit_item_world(&state, pose, light, out);
                }
            }
        }
        self.mark_entity(out.len(), "");
        self.text_displays = displays;
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
        let angles = self.display_camera_angles();
        let scale = self.view_scale();
        for display in self.text_displays.values_mut() {
            if display.shows != Shows::Text {
                continue;
            }
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
            let light = Self::display_light(&self.world, &d, position);
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
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
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

    /// A display riding a vehicle sits where the vehicle seats it each tick,
    /// moving from the last tick's seat, and a teleport sent meanwhile does
    /// not drag it off the seat.
    #[test]
    fn a_ridden_display_moves_from_seat_to_seat() {
        let mut d = RemoteTextDisplay::new(Shows::Text, [0.0, 64.0, 0.0], 0.0);
        d.metadata(&[(10, 1, Value::Int(3))]);
        d.move_to([9.0, 64.0, 9.0], None);
        let seat = glam::DVec3::new(0.0, 65.8, 0.0);
        d.ride(seat);
        assert_eq!(d.position, seat);
        d.tick();
        assert_eq!((d.previous, d.position), (seat, seat));
        let next = glam::DVec3::new(1.0, 65.8, 0.0);
        d.ride(next);
        assert_eq!((d.previous, d.position), (seat, next));
    }
}
