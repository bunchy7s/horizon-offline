//! Mod block entities drawn on the client: `native_block_entity_submit`
//! (vanilla's `BlockEntityRenderer.submit`) gets a [`BlockEntityDraw`], whose
//! pose stack (`PoseStack`) places what the helpers here add to the entity
//! batch, as vanilla's block entity renderers draw.

use super::{Dim, Game};
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_assets::{
    bake::{bake_flat_face, default_uv},
    item_model::DisplayContext,
    model::{Direction, Material},
};
use horizon_client::{
    block_entity_render::{diffuse, diffuse_lights, vertex_light},
    containers::{BlockEntity, ModBlockEntity},
    fast_hash::FxHashMap,
    gfx::Cull,
    hud::Rect,
    items::ItemStack,
    mesher::Vertex,
};
use horizon_world::registry::BlockStateId;

/// `BlockEntityRenderer.getViewDistance`'s default, in blocks.
const VIEW_DISTANCE: f64 = 64.0;
/// A mod block entity's bounds until its mod says otherwise: the block with
/// the block of margin Horizon culls vanilla's block entities with.
const DEFAULT_BOUNDS: [f32; 6] = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];

/// Whether `bounds` (min then max, in blocks from `origin`, the block's corner
/// relative to the camera) meet the frustum; an infinite bound always does
/// (NeoForge's `INFINITE_EXTENT_AABB`), as does a frame without a frustum.
fn in_frustum(frustum: Option<&impl Cull>, origin: Vec3, bounds: [f32; 6]) -> bool {
    if bounds.iter().any(|b| !b.is_finite()) {
        return true;
    }
    let min = origin + Vec3::new(bounds[0], bounds[1], bounds[2]);
    let max = origin + Vec3::new(bounds[3], bounds[4], bounds[5]);
    frustum.is_none_or(|f| f.intersects(min, max))
}

/// One mod block entity's frame, as `submit` gets it: the render state's
/// partial tick, game time and light, and the pose stack the helpers draw
/// under (`native_draw_quad`, `native_draw_cube`, `native_draw_block`).
// Native mods read and drive it; nothing in Horizon does.
#[allow(dead_code)]
pub struct BlockEntityDraw {
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    pub partial: f32,
    /// The level's game time, for animations.
    pub game_time: i64,
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16; set it to light a part otherwise.
    pub light: u32,
    /// The block drawn, for `native_draw_item`'s owner.
    block: [i32; 3],
    poses: Vec<Mat4>,
    lights: (Vec3, Vec3),
    vertices: Vec<Vertex>,
    kept: bool,
}

#[allow(dead_code)]
impl BlockEntityDraw {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    pub fn push(&mut self) {
        let pose = self.pose();
        self.poses.push(pose);
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    pub fn pop(&mut self) {
        if self.poses.len() > 1 {
            self.poses.pop();
        }
    }
    /// Moves what is drawn next by `by`, in blocks.
    pub fn translate(&mut self, by: [f32; 3]) {
        self.apply(Mat4::from_translation(Vec3::from_array(by)));
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    pub fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        let axis = Vec3::from_array(axis).normalize_or(Vec3::Y);
        let pivot = Vec3::from_array(pivot);
        self.apply(
            Mat4::from_translation(pivot)
                * Mat4::from_quat(Quat::from_axis_angle(axis, degrees.to_radians()))
                * Mat4::from_translation(-pivot),
        );
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    pub fn scale(&mut self, by: [f32; 3]) {
        self.apply(Mat4::from_scale(Vec3::from_array(by)));
    }
    /// Keeps what this call drew: the block is drawn the same again, with
    /// neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called, until its state,
    /// its block entity (what the server sent), its light or the resource
    /// packs change. Call it only when what you drew reads none of
    /// `partial`, `game_time`, the blocks around it or anything else of
    /// yours, and moves nothing by itself: an idle machine, not a turning
    /// shaft. Kept geometry stands still: an enchanted item's glint, and an
    /// item drawn with `native_draw_item` whose model reads the time (a clock,
    /// a compass), stop where they were. API @API:block_entity_keep@.
    pub fn keep(&mut self) {
        self.kept = true;
    }
    fn apply(&mut self, m: Mat4) {
        if let Some(pose) = self.poses.last_mut() {
            *pose *= m;
        }
    }
    fn pose(&self) -> Mat4 {
        self.poses.last().copied().unwrap_or(Mat4::IDENTITY)
    }
    /// A quad of `corners` (blocks, under the pose) with unit UVs, shaded as
    /// vanilla's block entity models are (`Lighting`'s two lights).
    fn quad(&mut self, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], sprite: Rect, tint: u32) {
        let pose = self.pose();
        let at = corners.map(|c| pose.transform_point3(Vec3::from_array(c)));
        let normal = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            (at[2] - at[1]).cross(at[3] - at[1]).normalize_or(Vec3::Y)
        } else {
            normal
        };
        let color = shaded(tint, diffuse(normal, self.lights));
        for (p, uv) in at.into_iter().zip(uv) {
            self.vertices.push(Vertex {
                pos: p.to_array(),
                uv: [
                    sprite.u0 + (sprite.u1 - sprite.u0) * uv[0],
                    sprite.v0 + (sprite.v1 - sprite.v0) * uv[1],
                ],
                layer: 0,
                color,
                light: self.light,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

/// One face of a `native_draw_cube` box: its sprite (`mymod:entity/cog`,
/// `mymod:block/casing`) and its UVs in texels of 16 per side, as a model
/// face's `uv` (none takes the box's own, as a model's default).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct CubeFace<'a> {
    pub sprite: &'a str,
    pub uv: Option<[f32; 4]>,
}

/// 0xRRGGBB `tint` times `brightness`, as an opaque vertex colour.
fn shaded(tint: u32, brightness: f32) -> u32 {
    let c = |shift: u32| (((tint >> shift) & 255) as f32 * brightness) as u32;
    0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16)
}

/// Game ticks kept geometry stays after it was last drawn, as signs' text.
const KEPT_FOR: i64 = 100;

/// What kept geometry was drawn with besides its block entity: drawn again
/// when any of it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeptKey {
    dim: Dim,
    state: u32,
    light: u32,
}

struct KeptEntry {
    key: KeptKey,
    entity: ModBlockEntity,
    bounds: [f32; 6],
    /// Game time it was last drawn.
    used: i64,
    /// Relative to the block's corner.
    vertices: Vec<Vertex>,
}

/// Geometry mods kept (`BlockEntityDraw::keep`), by position. As
/// `SignTexts` keeps a sign's text, it is kept relative to the block and
/// moved to the camera each frame.
#[derive(Default)]
pub(super) struct KeptGeometry {
    entries: FxHashMap<[i32; 3], KeptEntry>,
    /// Game time of the last sweep of what is out of view.
    swept: i64,
}

impl KeptGeometry {
    /// Drops what was not drawn for `KEPT_FOR` ticks, once every `KEPT_FOR`.
    fn sweep(&mut self, now: i64) {
        if (now - self.swept).abs() >= KEPT_FOR {
            self.swept = now;
            self.entries
                .retain(|_, e| (now - KEPT_FOR..=now).contains(&e.used));
        }
    }
    /// What is kept at `pos` when it was drawn with `key` and `entity`,
    /// marked drawn at `now`.
    fn get(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        entity: &ModBlockEntity,
        now: i64,
    ) -> Option<&KeptEntry> {
        let entry = self
            .entries
            .get_mut(&pos)
            .filter(|e| e.key == key && e.entity == *entity)?;
        entry.used = now;
        Some(entry)
    }
    fn keep(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        entity: &ModBlockEntity,
        bounds: [f32; 6],
        vertices: &[Vertex],
        now: i64,
    ) {
        let mut kept = self
            .entries
            .remove(&pos)
            .map_or_else(Vec::new, |e| e.vertices);
        kept.clear();
        kept.extend_from_slice(vertices);
        self.entries.insert(
            pos,
            KeptEntry {
                key,
                entity: entity.clone(),
                bounds,
                used: now,
                vertices: kept,
            },
        );
    }
    fn forget(&mut self, pos: [i32; 3]) {
        self.entries.remove(&pos);
    }
    /// Forgets everything: the atlas's sprites moved.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Game<'_> {
    /// `native_block_entity_submit` for each of `blocks` (in drawn sections,
    /// drawn by no vanilla renderer) whose block entity is a mod's, closer than
    /// `BlockEntityRenderer.getViewDistance` to the camera (`shouldRender`)
    /// and with its bounds (`native_block_entity_render_bounding_box`) in the
    /// frustum.
    pub(super) fn mod_block_entity_geometry<'a>(
        &self,
        blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
        camera: DVec3,
        partial: f32,
        out: &mut Vec<Vertex>,
    ) {
        let mut draw = BlockEntityDraw {
            partial,
            game_time: self.world.game_time,
            light: 0,
            block: [0; 3],
            poses: Vec::new(),
            lights: diffuse_lights(self.dim == Dim::Nether),
            vertices: std::mem::take(out),
            kept: false,
        };
        let now = self.world.game_time;
        let mut kept = self.kept_block_entities.borrow_mut();
        kept.sweep(now);
        for &(pos, state) in blocks {
            let corner = DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            // Vec3.closerThan: strictly closer.
            if (corner + 0.5).distance_squared(camera) >= VIEW_DISTANCE * VIEW_DISTANCE {
                continue;
            }
            let Some(BlockEntity::Mod(entity)) =
                self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
            else {
                continue;
            };
            let origin = (corner - camera).as_vec3();
            let light = vertex_light(self.world.light(pos[0], pos[1], pos[2]));
            let key = KeptKey {
                dim: self.dim,
                state,
                light,
            };
            if let Some(entry) = kept.get(pos, key, entity, now) {
                if in_frustum(self.view_frustum.as_ref(), origin, entry.bounds) {
                    let start = draw.vertices.len();
                    self.mark_block_entity(start, state);
                    draw.vertices.extend(entry.vertices.iter().map(|v| Vertex {
                        pos: (Vec3::from_array(v.pos) + origin).to_array(),
                        ..*v
                    }));
                    self.cull_entity_vertices(&mut draw.vertices, start);
                }
                continue;
            }
            let id = BlockStateId(state);
            let block = &*self.res.facts().get(id).name;
            let mut bounds = DEFAULT_BOUNDS;
            self.native_block_entity_render_bounding_box(pos, block, id, entity, &mut bounds);
            if !in_frustum(self.view_frustum.as_ref(), origin, bounds) {
                continue;
            }
            let start = draw.vertices.len();
            self.mark_block_entity(start, state);
            // Drawn relative to the block, then moved to the camera, so what
            // is kept can be moved again.
            draw.block = pos;
            draw.poses.clear();
            draw.poses.push(Mat4::IDENTITY);
            draw.light = light;
            draw.kept = false;
            self.native_block_entity_submit(pos, block, id, entity, &mut draw);
            if draw.kept {
                kept.keep(pos, key, entity, bounds, &draw.vertices[start..], now);
            } else {
                kept.forget(pos);
            }
            for v in &mut draw.vertices[start..] {
                v.pos = (Vec3::from_array(v.pos) + origin).to_array();
            }
            self.cull_entity_vertices(&mut draw.vertices, start);
        }
        *out = draw.vertices;
    }
}

/// Helpers for drawing mod block entities, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite. API 71.
    pub(super) fn native_draw_quad(
        &self,
        draw: &mut BlockEntityDraw,
        sprite: &str,
        corners: [[f32; 3]; 4],
        uv: [f32; 4],
        tint: u32,
    ) -> bool {
        let Some(rect) = self.gui.atlas.sprites.get(sprite).copied() else {
            return false;
        };
        let [u0, v0, u1, v1] = uv.map(|t| t / 16.0);
        draw.quad(
            corners,
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            rect,
            tint,
        );
        true
    }

    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out. API 71.
    pub(super) fn native_draw_cube(
        &self,
        draw: &mut BlockEntityDraw,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        let mut all = true;
        for (direction, face) in Direction::ALL.into_iter().zip(faces) {
            let Some(face) = face else { continue };
            let Some(rect) = self.gui.atlas.sprites.get(face.sprite).copied() else {
                all = false;
                continue;
            };
            let uv = face.uv.unwrap_or_else(|| default_uv(from, to, direction));
            let material = Material {
                sprite: String::new(),
                force_translucent: false,
            };
            if let Ok(quad) = bake_flat_face(from, to, uv, direction, material, -1) {
                draw.quad(quad.positions, quad.uv, rect, tint);
            }
        }
        all
    }

    /// Draws block `block`'s model (`mymod:cog`, any block with a model) in
    /// the state with `properties` (`[("axis", "x")]`, the rest default)
    /// under `draw`'s pose, as vanilla's `submitBlockModel` draws a block in
    /// an entity's place: a block's baked model, with its default tints. A
    /// moving part is often a block of the mod's own, never placed. False for
    /// an unknown block or property. API 71.
    pub(super) fn native_draw_block(
        &self,
        draw: &mut BlockEntityDraw,
        block: &str,
        properties: &[(&str, &str)],
    ) -> bool {
        let registry = &self.pack.assets.registry;
        let Some(mut state) = registry
            .default_state(block)
            .and_then(|id| registry.state(id))
            .cloned()
        else {
            return false;
        };
        for (key, value) in properties {
            match state.properties.get_mut(*key) {
                Some(v) => *v = (*value).to_owned(),
                None => return false,
            }
        }
        let Some(id) = registry.id(&state) else {
            return false;
        };
        let pose = draw.pose();
        self.block_model_vertices(id, 42, pose, draw.light, true, &mut draw.vertices);
        true
    }

    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `native_block_entity_update_tag` and read them from `entity.nbt`.
    /// API @API:block_entity_draw_item@.
    pub(super) fn native_draw_item(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        display: DisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        // ShelfBlockEntity.position(): Vec3.atCenterOf(getBlockPos()).
        let owner = owner_yaw
            .map(|yaw| super::item_draw::owner_at(draw.block.map(|c| f64::from(c) + 0.5), yaw, ""));
        let item = self.resolve_item(stack, display, owner.as_ref(), seed);
        if item.is_empty() {
            return false;
        }
        let pose = draw.pose();
        self.submit_item_world(&item, pose, draw.light, &mut draw.vertices);
        true
    }

    /// Property `key` of block state `state` (a machine's `facing`, from the
    /// `state` `native_block_entity_submit` gets), if it has one: no lookup in
    /// the world, so cheap each frame. API @API:block_entity_draw_item@.
    pub(super) fn native_block_state_property(
        &self,
        state: BlockStateId,
        key: &str,
    ) -> Option<&str> {
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw() -> BlockEntityDraw {
        BlockEntityDraw {
            partial: 0.0,
            game_time: 0,
            light: 0xF0F0,
            block: [0; 3],
            poses: vec![Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
            lights: diffuse_lights(false),
            vertices: Vec::new(),
            kept: false,
        }
    }

    fn sprite() -> Rect {
        Rect {
            u0: 0.5,
            v0: 0.25,
            u1: 0.75,
            v1: 0.5,
            w: 16,
            h: 16,
        }
    }

    #[test]
    fn the_pose_stack_turns_about_its_pivot_and_pops_back() {
        let mut d = draw();
        d.push();
        d.rotate_about([0.0, 1.0, 0.0], 90.0, [0.5, 0.5, 0.5]);
        // (1, 0, 0.5) turns a quarter about the block's centre to (0.5, 0, 0).
        let p = d.pose().transform_point3(Vec3::new(1.0, 0.0, 0.5));
        assert!(p.abs_diff_eq(Vec3::new(10.5, 0.0, 0.0), 1e-5), "{p}");
        d.pop();
        d.pop();
        // Popping past the block's own pose keeps it.
        let p = d.pose().transform_point3(Vec3::ZERO);
        assert_eq!(p, Vec3::new(10.0, 0.0, 0.0));
        d.translate([0.0, 2.0, 0.0]);
        d.scale([2.0, 2.0, 2.0]);
        let p = d.pose().transform_point3(Vec3::ONE);
        assert_eq!(p, Vec3::new(12.0, 4.0, 2.0));
    }

    #[test]
    fn a_quad_lands_in_its_sprite_lit_and_shaded() {
        let mut d = draw();
        // An upward face: full brightness under vanilla's level lights is
        // below 1 (two lights), so the colour is shaded but opaque.
        d.quad(
            [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            sprite(),
            0xFFFFFF,
        );
        assert_eq!(d.vertices.len(), 4);
        let v = d.vertices[2];
        assert_eq!(v.pos, [11.0, 1.0, 1.0]);
        assert_eq!(v.uv, [0.75, 0.5]);
        assert_eq!(v.light, 0xF0F0);
        let up = diffuse(Vec3::Y, diffuse_lights(false));
        assert_eq!(v.color, shaded(0xFFFFFF, up));
        assert_eq!(v.color >> 24, 0xFF);
    }

    /// A frustum that sees nothing.
    struct Blind;
    impl Cull for Blind {
        fn intersects(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
        fn contains(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
    }

    #[test]
    fn infinite_bounds_and_frames_without_a_frustum_are_never_culled() {
        let origin = Vec3::new(1.0e6, 0.0, 0.0);
        let infinite = [f32::NEG_INFINITY, 0.0, 0.0, f32::INFINITY, 1.0, 1.0];
        assert!(!in_frustum(Some(&Blind), origin, DEFAULT_BOUNDS));
        assert!(in_frustum(Some(&Blind), origin, infinite));
        assert!(in_frustum(None::<&Blind>, origin, DEFAULT_BOUNDS));
    }

    #[test]
    fn a_tint_keeps_its_channels_in_vertex_order() {
        // The vertex colour is RGBA8 read little-endian: red in the low byte.
        assert_eq!(shaded(0x10_20_30, 1.0), 0xFF_30_20_10);
    }

    fn machine(progress: i32) -> ModBlockEntity {
        ModBlockEntity {
            id: "mymod:machine".into(),
            data: vec![progress],
            ..Default::default()
        }
    }

    fn vertex(x: f32) -> Vertex {
        Vertex {
            pos: [x, 0.0, 0.0],
            ..bytemuck::Zeroable::zeroed()
        }
    }

    const KEY: KeptKey = KeptKey {
        dim: Dim::Overworld,
        state: 7,
        light: 0xF0F0,
    };

    #[test]
    fn kept_geometry_is_drawn_again_until_what_it_was_drawn_with_changes() {
        let mut kept = KeptGeometry::default();
        let pos = [1, 2, 3];
        kept.keep(pos, KEY, &machine(0), DEFAULT_BOUNDS, &[vertex(0.5)], 0);
        let entry = kept.get(pos, KEY, &machine(0), 1).expect("kept");
        assert_eq!(entry.vertices.len(), 1);
        assert_eq!(entry.vertices[0].pos, [0.5, 0.0, 0.0]);
        assert_eq!(entry.bounds, DEFAULT_BOUNDS);
        // New data from the server, another state, light or dimension, or
        // another position: none of them is the kept geometry.
        assert!(kept.get(pos, KEY, &machine(1), 1).is_none());
        for changed in [
            KeptKey { state: 8, ..KEY },
            KeptKey {
                light: 0x00F0,
                ..KEY
            },
            KeptKey {
                dim: Dim::Nether,
                ..KEY
            },
        ] {
            assert!(kept.get(pos, changed, &machine(0), 1).is_none());
        }
        assert!(kept.get([1, 2, 4], KEY, &machine(0), 1).is_none());
        // Drawn again and kept with the new data, it replaces the old.
        kept.keep(pos, KEY, &machine(1), DEFAULT_BOUNDS, &[vertex(0.25)], 2);
        assert!(kept.get(pos, KEY, &machine(0), 2).is_none());
        let entry = kept.get(pos, KEY, &machine(1), 2).expect("kept again");
        assert_eq!(entry.vertices[0].pos, [0.25, 0.0, 0.0]);
        // Drawn without keeping: forgotten.
        kept.forget(pos);
        assert!(kept.get(pos, KEY, &machine(1), 3).is_none());
    }

    #[test]
    fn kept_geometry_out_of_view_for_its_ticks_is_swept() {
        let mut kept = KeptGeometry::default();
        let (seen, gone) = ([0, 0, 0], [5, 0, 0]);
        kept.keep(seen, KEY, &machine(0), DEFAULT_BOUNDS, &[vertex(0.0)], 0);
        kept.keep(gone, KEY, &machine(0), DEFAULT_BOUNDS, &[vertex(0.0)], 0);
        kept.get(gone, KEY, &machine(0), 50);
        // A sweep at 100 keeps both: each was drawn in the last 100 ticks.
        kept.sweep(100);
        assert!(kept.entries.contains_key(&gone));
        assert!(kept.entries.contains_key(&seen));
        kept.get(seen, KEY, &machine(0), 150);
        // At 199 no sweep is due yet; at 200 the one last drawn at 50 goes.
        kept.sweep(199);
        assert!(kept.entries.contains_key(&gone));
        kept.sweep(200);
        assert!(!kept.entries.contains_key(&gone));
        assert!(kept.get(seen, KEY, &machine(0), 200).is_some());
    }
}
