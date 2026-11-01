//! Item frames and glow item frames: `ItemFrameItem` placement on any face,
//! `ItemFrame.survives`, `interact` (put an item in, then rotate it),
//! `hurtServer` (take the item out, then break the frame) and
//! `ItemFrameRenderer` (the frame model and the item at a 45 degree step).
use super::{Game, Hit, InteractTarget};
use horizon_assets::{bake::bake_model, model::CuboidModel, resolve::ModelLibrary};
use horizon_client::{entities::ItemFrame, items::ItemStack, mesher::Vertex};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use std::sync::Arc;

/// A baked frame model: (corners in block units, unit UVs, sprite) per quad.
pub(super) type FrameModel = Vec<([[f32; 3]; 4], [[f32; 2]; 4], String)>;

/// Bake `block/<name>` (item_frame / glow_item_frame) from the resources.
pub(super) fn bake_frame(root: &std::path::Path, name: &str) -> FrameModel {
    let mut models = std::collections::BTreeMap::new();
    let mut pending = vec![format!("minecraft:block/{name}")];
    while let Some(model) = pending.pop() {
        let Some((ns, path)) = model.split_once(':') else {
            continue;
        };
        let Ok(text) =
            std::fs::read_to_string(root.join(format!("assets/{ns}/models/{path}.json")))
        else {
            continue;
        };
        let Ok(m) = CuboidModel::from_json(&text) else {
            continue;
        };
        if let Some(parent) = &m.parent {
            pending.push(parent.clone());
        }
        models.insert(model, m);
    }
    let Ok(library) = ModelLibrary::new(models) else {
        return Vec::new();
    };
    let Ok(resolved) = library.resolve(&format!("minecraft:block/{name}")) else {
        return Vec::new();
    };
    let Ok(quads) = bake_model(&resolved) else {
        return Vec::new();
    };
    let unit = if quads
        .iter()
        .flat_map(|q| q.positions)
        .any(|p| p.iter().any(|v| *v > 1.5))
    {
        16.0
    } else {
        1.0
    };
    quads
        .into_iter()
        .map(|q| {
            (
                q.positions.map(|p| p.map(|v| v / unit)),
                q.uv,
                q.material
                    .sprite
                    .trim_start_matches("minecraft:")
                    .to_owned(),
            )
        })
        .collect()
}
fn facing_of(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    }
}
fn sound(f: &ItemFrame, what: &str) -> String {
    let prefix = if f.glow {
        "glow_item_frame"
    } else {
        "item_frame"
    };
    format!("minecraft:entity.{prefix}.{what}")
}
fn center(f: &ItemFrame) -> [f64; 3] {
    let (lo, hi) = f.bounds();
    std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0)
}
impl Game<'_> {
    /// `ItemFrame.survives` (not fixed): no collision, a solid block behind
    /// (or a diode for wall frames), no overlapping hanging entity.
    fn frame_survives(&self, f: &ItemFrame) -> bool {
        if f.fixed {
            return true;
        }
        let (lo, hi) = f.bounds();
        let bounds = Aabb::new(
            Vec3::new(lo[0], lo[1], lo[2]),
            Vec3::new(hi[0], hi[1], hi[2]),
        );
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            return false;
        }
        let s = f.step();
        let behind = self.res.facts().get(self.world.block(
            f.pos[0] - s[0],
            f.pos[1] - s[1],
            f.pos[2] - s[2],
        ));
        let supported = behind.legacy_solid || (s[1] == 0 && behind.has_class("DiodeBlock"));
        // canCoexist(true): only hanging entities facing the same way block it.
        let overlaps = |o: ([f64; 3], [f64; 3])| (0..3).all(|i| o.0[i] < hi[i] && lo[i] < o.1[i]);
        supported
            && !self
                .item_frames
                .iter()
                .any(|o| o.id != f.id && o.facing == f.facing && overlaps(o.bounds()))
    }
    /// `ItemFrameItem.useOn`.
    pub(super) fn place_item_frame(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let glow = match &*stack.item {
            "minecraft:item_frame" => false,
            "minecraft:glow_item_frame" => true,
            _ => return false,
        };
        let invisible = stack.extra.as_ref().is_some_and(|extra| {
            extra.other.iter().any(|(k, v)| {
                (k == "minecraft:entity_data" || k == "entity_data")
                    && horizon_client::save::number(v.get("Invisible")).unwrap_or(0) != 0
            })
        });
        let pos: [i32; 3] = std::array::from_fn(|i| hit.pos[i] + hit.face[i]);
        // `ItemFrameItem.mayPlace`: `Level.isInsideBuildHeight(blockPos)`, so a frame is not
        // placed on the top face of the highest block or under the lowest one.
        if !(horizon_client::chunk::MIN_Y
            ..horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT)
            .contains(&pos[1])
        {
            return false;
        }
        let frame = ItemFrame {
            id: self.next_entity + 1,
            pos,
            facing: facing_of(hit.face),
            glow,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible,
            check_interval: 0,
        };
        if !self.frame_survives(&frame) {
            return false;
        }
        self.next_entity += 1;
        self.audio
            .play(&sound(&frame, "place"), Some(center(&frame)), 1.0, 1.0);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.item_frames.push(frame);
        if !self.creative {
            self.consume_held();
        }
        true
    }
    /// The frame under the crosshair within reach and its distance.
    pub(super) fn frame_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, f) in self.item_frames.iter().enumerate() {
            let (lo, hi) = f.bounds();
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// Nearer than the targeted block?
    pub(super) fn before_block(&self, distance: f64) -> bool {
        self.pick_unfiltered().is_none_or(|h| {
            let eye = self.aim().map(|(eye, _)| eye).unwrap_or_default();
            (0..3)
                .map(|a| (h.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt()
                > distance
        })
    }
    /// `ItemFrame.interact` for the frame under the crosshair, or the one a
    /// guest's interact packet named.
    pub(super) fn interact_frame(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.item_frames, t.id, |f| f.id).map(|i| (i, 0.0)),
            None => self.frame_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if (target.is_none() && !self.before_block(d)) || self.item_frames[i].fixed {
            return false;
        }
        let at = center(&self.item_frames[i]);
        if self.item_frames[i].item.is_none() {
            let Some(held) = self.held().cloned() else {
                return false;
            };
            self.item_frames[i].item = Some(ItemStack { count: 1, ..held });
            self.audio
                .play(&sound(&self.item_frames[i], "add_item"), Some(at), 1.0, 1.0);
            if !self.creative {
                self.consume_held();
            }
        } else {
            let f = &mut self.item_frames[i];
            f.rotation = (f.rotation + 1) % 8;
            let s = sound(f, "rotate_item");
            self.audio.play(&s, Some(at), 1.0, 1.0);
        }
        let p = self.item_frames[i].pos;
        self.world.modified.insert((p[0] >> 4, p[2] >> 4));
        self.frame_output_changed(p);
        self.swing_arm();
        true
    }
    /// `ItemFrame.hurtServer` from the player: the item comes out first.
    pub(super) fn attack_frame(&mut self) -> bool {
        let Some((i, d)) = self.frame_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        if self.item_frames[i].fixed {
            return true;
        }
        let with_frame = self.item_frames[i].item.is_none();
        let at = center(&self.item_frames[i]);
        let f = if with_frame {
            self.item_frames.remove(i)
        } else {
            self.item_frames[i].clone()
        };
        self.audio.play(
            &sound(&f, if with_frame { "break" } else { "remove_item" }),
            Some(at),
            1.0,
            1.0,
        );
        if !with_frame {
            self.item_frames[i].item = None;
        }
        self.drop_frame_items(&f, with_frame, true);
        self.world.modified.insert((f.pos[0] >> 4, f.pos[2] >> 4));
        if f.item.is_some() {
            self.frame_output_changed(f.pos);
        }
        true
    }
    /// `ItemFrame.setItem` / `setRotation`: `updateNeighbourForOutputSignal`, so
    /// the comparators reading the frame take its new signal.
    fn frame_output_changed(&mut self, pos: [i32; 3]) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.update_neighbour_for_output_signal(level, ticks, at));
    }
    /// `ItemFrame.dropItem`: the frame (when breaking) and the held item.
    fn drop_frame_items(&mut self, f: &ItemFrame, with_frame: bool, by_player: bool) {
        // The framed map forgets the frame, except when the drops are off and
        // something caused the break (`removeFramedMap` runs only for a null
        // `causedBy` then).
        if self.ui.rules.bool("entity_drops") || !by_player {
            self.remove_framed_map(f);
        }
        if !self.ui.rules.bool("entity_drops") || (by_player && self.creative) {
            return;
        }
        let s = f.step().map(f64::from);
        let c = center(f);
        let at = [c[0] + s[0] * 0.15, c[1], c[2] + s[2] * 0.15];
        let mut drops = Vec::new();
        if with_frame {
            drops.push(ItemStack {
                item: Arc::from(f.kind()),
                count: 1,
                damage: 0,
                extra: None,
            });
        }
        if let Some(item) = f.item.clone()
            && self.world.random.next_float() < f.drop_chance
        {
            drops.push(item);
        }
        if by_player && !drops.is_empty() {
            let tool = self.held().cloned();
            self.native_use_drops(
                "",
                c,
                None,
                Some((f.id, f.kind())),
                tool.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            self.spawn_item(stack, at, [0.0; 3], 10);
        }
    }
    /// `ServerExplosion.hurtEntities` for item frames: a frame within the blast
    /// is killed and dropped with its item (`BlockAttachedEntity.hurtServer`).
    /// A fixed frame is not hurt by an explosion, and `blast_spares_decorations`
    /// keeps every frame without `mobGriefing`.
    pub(super) fn explosion_breaks_frames(
        &mut self,
        blast: [f64; 3],
        diameter: f64,
        interaction: super::explosion::BlockInteraction,
    ) {
        if self.blast_spares_decorations(interaction) {
            return;
        }
        let mut i = 0;
        while i < self.item_frames.len() {
            let c = center(&self.item_frames[i]);
            let dist =
                ((c[0] - blast[0]).powi(2) + (c[1] - blast[1]).powi(2) + (c[2] - blast[2]).powi(2))
                    .sqrt()
                    / diameter;
            if dist > 1.0 || self.item_frames[i].fixed {
                i += 1;
                continue;
            }
            let f = self.item_frames.remove(i);
            self.audio
                .play(&sound(&f, "break"), Some(center(&f)), 1.0, 1.0);
            self.drop_frame_items(&f, true, false);
            self.world.modified.insert((f.pos[0] >> 4, f.pos[2] >> 4));
            if f.item.is_some() {
                self.frame_output_changed(f.pos);
            }
        }
    }
    /// BlockAttachedEntity.tick for frames.
    pub(super) fn tick_item_frames(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.item_frames.len() {
            let at = center(&self.item_frames[i]);
            if !ticking(at[0].floor() as i32, at[2].floor() as i32) {
                i += 1;
                continue;
            }
            self.item_frames[i].check_interval += 1;
            if self.item_frames[i].check_interval >= 100 {
                self.item_frames[i].check_interval = 0;
                let f = self.item_frames[i].clone();
                if !self.frame_survives(&f) {
                    self.item_frames.remove(i);
                    self.audio
                        .play(&sound(&f, "break"), Some(center(&f)), 1.0, 1.0);
                    self.drop_frame_items(&f, true, false);
                    if f.item.is_some() {
                        self.frame_output_changed(f.pos);
                    }
                    continue;
                }
            }
            i += 1;
        }
    }
    /// ItemFrameRenderer: the frame model, then the item half size on its face.
    pub(super) fn frame_geometry(&mut self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let sprites = &self.gui.atlas.sprites;
        let mut maps: Vec<(i32, glam::Mat4, u32)> = Vec::new();
        let mut items: Vec<(ItemStack, u32, glam::Mat4, u8)> = Vec::new();
        // ItemFrame.shouldRenderAtSqrDistance: 1024 blocks at a view scale
        // of 1.
        let range = 16.0 * 64.0 * self.view_scale();
        for f in &self.item_frames {
            let entity_start = out.len();
            let map = f.item.as_ref().and_then(super::maps::map_id);
            // EntityRenderer.shouldRender: its box (a block wide with a map),
            // measured from the box's centre.
            let (lo, hi) = f.bounds();
            let (mut lo, mut hi) = (glam::DVec3::from(lo), glam::DVec3::from(hi));
            if map.is_some() {
                let s = f.step();
                for i in 0..3 {
                    if s[i] == 0 {
                        let c = (lo[i] + hi[i]) / 2.0;
                        (lo[i], hi[i]) = (c - 0.5, c + 0.5);
                    }
                }
            }
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let s = f.step();
            // Pose: to the block centre, then X/Y rotation by facing.
            let (x_rot, y_rot) = if s[1] == 0 {
                let to_y = match f.facing {
                    3 => 0.0,
                    4 => 90.0,
                    2 => 180.0,
                    _ => 270.0,
                };
                (0.0f64, 180.0f64 - to_y)
            } else {
                (-90.0 * f64::from(s[1]), 180.0)
            };
            let pose = glam::DMat4::from_translation(
                glam::DVec3::new(
                    f64::from(f.pos[0]) + 0.5,
                    f64::from(f.pos[1]) + 0.5,
                    f64::from(f.pos[2]) + 0.5,
                ) - camera,
            ) * glam::DMat4::from_rotation_x(x_rot.to_radians())
                * glam::DMat4::from_rotation_y(y_rot.to_radians());
            let light = self.world.light(f.pos[0], f.pos[1], f.pos[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // ItemFrameRenderer.getBlockLightLevel: a glow frame's model is at least 5 bright.
            let frame_light = if f.glow {
                (light & 0xF0) | (light & 15).max(5)
            } else {
                light
            };
            let frame_packed =
                (u32::from(frame_light >> 4) * 16) | ((u32::from(frame_light & 15) * 16) << 8);
            let frame_start = out.len();
            let mut quad =
                |m: &glam::DMat4, corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], light: u32| {
                    // The frame model is drawn with the entity shader, lit by its face normal.
                    let at = corners.map(|c| m.transform_point3(glam::DVec3::from_array(c)));
                    let n = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
                    let normal = [0, 1, 2].map(|k| (n[k].clamp(-1.0, 1.0) * 127.0) as i8);
                    for (p, uv) in at.into_iter().zip(uv) {
                        out.push(Vertex {
                            normal: [normal[0], normal[1], normal[2], -127],
                            pos: p.as_vec3().to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light,

                            glint_uv: [0.0, 0.0],
                        });
                    }
                };
            let map_uv = |r: &horizon_client::hud::Rect, uv: [[f32; 2]; 4]| {
                uv.map(|[u, v]| [r.u0 + u * (r.u1 - r.u0), r.v0 + v * (r.v1 - r.v0)])
            };
            if !f.invisible {
                let model = match (f.glow, map.is_some()) {
                    (true, true) => &self.glow_map_frame_model,
                    (false, true) => &self.map_frame_model,
                    (true, false) => &self.glow_frame_model,
                    (false, false) => &self.frame_model,
                };
                for (corners, uv, sprite) in model {
                    let Some(r) = sprites.get(sprite) else {
                        continue;
                    };
                    quad(
                        &pose,
                        corners.map(|c| {
                            [
                                f64::from(c[0]) - 0.5,
                                f64::from(c[1]) - 0.5,
                                f64::from(c[2]) - 0.5,
                            ]
                        }),
                        map_uv(r, *uv),
                        frame_packed,
                    );
                }
            }
            // HDR output: a glow frame shines softly (entity.wgsl fs_hdr_mark); the item in it
            // is only lit, as vanilla draws it.
            if f.glow {
                for v in &mut out[frame_start..] {
                    v.layer |= u32::from(horizon_client::mesher::INCANDESCENT) << 16;
                }
            }
            self.cull_entity_vertices(out, entity_start);
            if let Some(id) = map {
                // ItemFrameRenderer: the map fills the face, in 90-degree steps.
                let depth = if f.invisible { 0.5 } else { 0.4375 };
                // A glow frame's map is 15728850 = 0xF000D2 (sky coordinate 240,
                // block coordinate 210), not full bright.
                let item_light = if f.glow { 240 | 210 << 8 } else { packed };
                let m = pose
                    * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                    * glam::DMat4::from_rotation_z(
                        (f64::from(f.rotation % 4 * 2) * 45.0).to_radians(),
                    )
                    * glam::DMat4::from_rotation_z(std::f64::consts::PI)
                    * glam::DMat4::from_scale(glam::DVec3::splat(0.0078125))
                    * glam::DMat4::from_translation(glam::DVec3::new(-64.0, -64.0, -1.0));
                maps.push((id, m.as_mat4(), item_light));
                continue;
            }
            let Some(stack) = f.item.clone() else {
                continue;
            };
            // `GlowItemFrame` items render full bright (15728880).
            let item_light = if f.glow { 0xFF } else { light };
            let depth = if f.invisible { 0.5 } else { 0.4375 };
            let m = pose
                * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                * glam::DMat4::from_rotation_z((f64::from(f.rotation) * 45.0).to_radians())
                * glam::DMat4::from_scale(glam::DVec3::splat(0.5));
            items.push((stack, f.id, m.as_mat4(), item_light));
        }
        // `extractRenderState`: FIXED, seeded by the frame entity.
        for (stack, id, m, light) in items {
            let entity_start = out.len();
            let owner = super::item_draw::owner_at([0.0; 3], 0.0, "minecraft:item_frame");
            let state = self.resolve_item(
                &stack,
                horizon_assets::item_model::DisplayContext::Fixed,
                Some(&owner),
                id as i32,
            );
            self.submit_item_world(
                &state,
                m,
                horizon_client::block_entity_render::vertex_light(light),
                out,
            );
            self.cull_entity_vertices(out, entity_start);
        }
        for (id, m, light) in maps {
            let entity_start = out.len();
            self.map_vertices(id, true, m, light, out);
            self.cull_entity_vertices(out, entity_start);
        }
    }
}
