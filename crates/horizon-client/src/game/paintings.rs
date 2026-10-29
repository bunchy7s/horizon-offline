//! Paintings: `HangingEntityItem.useOn` placement with `Painting.create`'s
//! variant choice, `HangingEntity.survives` checks every 100 ticks,
//! breaking by attack (`dropItem`), saving, and `PaintingRenderer` quads.
use super::{Game, Hit};
use horizon_client::{entities::Painting, items::ItemStack, mesher::Vertex};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::sync::Arc;

/// A `painting_variant` and whether `#placeable` lists it (in tag order).
#[derive(Clone, Debug)]
pub(super) struct Variant {
    pub id: Arc<str>,
    pub asset: String,
    pub width: i32,
    pub height: i32,
}
/// Variants by id plus the `#minecraft:placeable` list.
#[derive(Default)]
pub(super) struct PaintingVariants {
    pub all: Vec<Variant>,
    pub placeable: Vec<Arc<str>>,
}
impl PaintingVariants {
    pub fn load(root: &std::path::Path) -> Self {
        let data = root.join("data/minecraft");
        let mut all = Vec::new();
        if let Ok(entries) = std::fs::read_dir(data.join("painting_variant")) {
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                all.push(Variant {
                    id: Arc::from(format!("minecraft:{name}")),
                    asset: v["asset_id"]
                        .as_str()
                        .unwrap_or(name)
                        .trim_start_matches("minecraft:")
                        .to_owned(),
                    width: v["width"].as_i64().unwrap_or(1) as i32,
                    height: v["height"].as_i64().unwrap_or(1) as i32,
                });
            }
        }
        let placeable = std::fs::read(data.join("tags/painting_variant/placeable.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["values"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(Arc::from))
            .collect();
        Self { all, placeable }
    }
    pub fn get(&self, id: &str) -> Option<&Variant> {
        self.all.iter().find(|v| &*v.id == id)
    }
}
fn overlaps(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|i| a.0[i] < b.1[i] && b.0[i] < a.1[i])
}
impl Game<'_> {
    /// `HangingEntity.survives`: no block collision in the box, solid blocks
    /// (or diodes) behind it and no other painting overlapping it.
    pub(super) fn painting_survives(&self, p: &Painting) -> bool {
        let (lo, hi) = p.bounds();
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
        // calculateSupportBox: moved half a block into the wall, deflated.
        let s = p.step().map(f64::from);
        let (slo, shi) = (
            std::array::from_fn::<f64, 3, _>(|i| lo[i] - s[i] * 0.5 + 1.0e-7),
            std::array::from_fn::<f64, 3, _>(|i| hi[i] - s[i] * 0.5 - 1.0e-7),
        );
        for x in slo[0].floor() as i32..=shi[0].floor() as i32 {
            for y in slo[1].floor() as i32..=shi[1].floor() as i32 {
                for z in slo[2].floor() as i32..=shi[2].floor() as i32 {
                    let f = self.res.facts().get(self.world.block(x, y, z));
                    if !(f.legacy_solid || f.has_class("DiodeBlock")) {
                        return false;
                    }
                }
            }
        }
        !self
            .paintings
            .iter()
            .any(|o| o.id != p.id && overlaps(o.bounds(), (lo, hi)))
    }
    /// `HangingEntityItem.useOn` for a painting against `hit`'s face.
    pub(super) fn place_painting(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:painting" || hit.face[1] != 0 {
            return false;
        }
        let facing = match hit.face {
            [0, 0, 1] => 0,
            [-1, 0, 0] => 1,
            [0, 0, -1] => 2,
            _ => 3,
        };
        let pos = [
            hit.pos[0] + hit.face[0],
            hit.pos[1],
            hit.pos[2] + hit.face[2],
        ];
        self.next_entity += 1;
        let mut candidate = Painting {
            id: self.next_entity,
            pos,
            facing,
            variant: Arc::from(""),
            width: 1,
            height: 1,
            check_interval: 0,
        };
        // Painting.create: the largest placeable variants that survive, one at random.
        let mut fitting: Vec<Variant> = Vec::new();
        for id in self.painting_variants.placeable.clone() {
            let Some(v) = self.painting_variants.get(&id).cloned() else {
                continue;
            };
            (candidate.width, candidate.height) = (v.width, v.height);
            if self.painting_survives(&candidate) {
                fitting.push(v);
            }
        }
        let Some(largest) = fitting.iter().map(|v| v.width * v.height).max() else {
            return false;
        };
        fitting.retain(|v| v.width * v.height >= largest);
        let pick = self
            .world
            .random
            .next_int_bound(fitting.len() as i32)
            .unwrap_or(0) as usize;
        let v = &fitting[pick];
        (candidate.variant, candidate.width, candidate.height) = (v.id.clone(), v.width, v.height);
        let (lo, hi) = candidate.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.audio
            .play("minecraft:entity.painting.place", Some(at), 1.0, 1.0);
        self.paintings.push(candidate);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        if !self.creative {
            self.consume_held();
        }
        true
    }
    /// The painting under the crosshair within reach and its distance.
    pub(super) fn painting_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        let mut best: Option<(usize, f64)> = None;
        for (i, p) in self.paintings.iter().enumerate() {
            let (lo, hi) = p.bounds();
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
    /// `Painting.hurtServer` -> kill and `dropItem`.
    pub(super) fn break_painting(&mut self, index: usize, by_player: bool) {
        let p = self.paintings.remove(index);
        let (lo, hi) = p.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.world.modified.insert((p.pos[0] >> 4, p.pos[2] >> 4));
        if !self.ui.rules.bool("entity_drops") {
            return;
        }
        self.audio
            .play("minecraft:entity.painting.break", Some(at), 1.0, 1.0);
        if by_player && self.creative {
            return;
        }
        // HangingEntity.spawnAtLocation: 0.15 out from the wall.
        let s = p.step().map(f64::from);
        let stack = ItemStack {
            item: Arc::from("minecraft:painting"),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(
            stack,
            [at[0] + s[0] * 0.15, at[1], at[2] + s[2] * 0.15],
            [0.0; 3],
            10,
        );
    }
    /// BlockAttachedEntity.tick: check survival every 100 ticks.
    pub(super) fn tick_paintings(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.paintings.len() {
            let (lo, hi) = self.paintings[i].bounds();
            let centre = [0, 2].map(|k| ((lo[k] + hi[k]) / 2.0).floor() as i32);
            if !ticking(centre[0], centre[1]) {
                i += 1;
                continue;
            }
            self.paintings[i].check_interval += 1;
            if self.paintings[i].check_interval >= 100 {
                self.paintings[i].check_interval = 0;
                let p = self.paintings[i].clone();
                if !self.painting_survives(&p) {
                    self.break_painting(i, false);
                    continue;
                }
            }
            i += 1;
        }
    }
    /// PaintingRenderer: the variant on the front, the back texture behind and
    /// on the edges.
    pub(super) fn painting_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let sprites = &self.gui.atlas.sprites;
        let Some(back) = sprites.get("painting/back").copied() else {
            return;
        };
        for p in &self.paintings {
            let entity_start = out.len();
            // EntityRenderer.shouldRender: the default range and its box,
            // measured from the box's centre (HangingEntity's position).
            let (lo, hi) = p.bounds();
            let (lo, hi) = (glam::DVec3::from(lo), glam::DVec3::from(hi));
            let range = super::entity_culling::default_range(hi - lo, self.view_scale());
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let Some(v) = self.painting_variants.get(&p.variant) else {
                continue;
            };
            let Some(front) = sprites.get(&format!("painting/{}", v.asset)).copied() else {
                continue;
            };
            let (lo, hi) = p.bounds();
            let c = glam::DVec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ) - camera;
            let s = p.step().map(f64::from);
            let l = p.left().map(f64::from);
            let out_dir = glam::DVec3::new(s[0], 0.0, s[2]);
            // The viewer's right is the painting's left.
            let right = glam::DVec3::new(l[0], 0.0, l[2]);
            let up = glam::DVec3::Y;
            let (hw, hh, d) = (
                f64::from(p.width) / 2.0,
                f64::from(p.height) / 2.0,
                1.0 / 32.0,
            );
            let light = self.world.light(p.pos[0], p.pos[1], p.pos[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let mut quad = |corners: [glam::DVec3; 4], uv: [[f32; 2]; 4], shade: f32| {
                let k = (255.0 * shade) as u32;
                let color = 0xFF00_0000 | k << 16 | k << 8 | k;
                for (p, uv) in corners.iter().zip(uv) {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.as_vec3().to_array(),
                        uv,
                        layer: 0,
                        color,
                        light: packed,

                        glint_uv: [0.0, 0.0],
                    });
                }
            };
            let full = |r: horizon_client::hud::Rect| {
                [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]]
            };
            let f = c + out_dir * d;
            quad(
                [
                    f - right * hw - up * hh,
                    f + right * hw - up * hh,
                    f + right * hw + up * hh,
                    f - right * hw + up * hh,
                ],
                full(front),
                1.0,
            );
            let b = c - out_dir * d;
            quad(
                [
                    b + right * hw - up * hh,
                    b - right * hw - up * hh,
                    b - right * hw + up * hh,
                    b + right * hw + up * hh,
                ],
                full(back),
                0.8,
            );
            // Edges: a one-pixel strip of the back texture.
            let strip = [
                [back.u0, back.v0 + (back.v1 - back.v0) / 16.0],
                [back.u1, back.v0 + (back.v1 - back.v0) / 16.0],
                [back.u1, back.v0],
                [back.u0, back.v0],
            ];
            let (t, bt) = (up * hh, -up * hh);
            quad(
                [
                    b + t + right * hw,
                    b + t - right * hw,
                    f + t - right * hw,
                    f + t + right * hw,
                ],
                strip,
                1.0,
            );
            quad(
                [
                    f + bt + right * hw,
                    f + bt - right * hw,
                    b + bt - right * hw,
                    b + bt + right * hw,
                ],
                strip,
                0.5,
            );
            quad(
                [
                    b - right * hw - up * hh,
                    f - right * hw - up * hh,
                    f - right * hw + up * hh,
                    b - right * hw + up * hh,
                ],
                strip,
                0.6,
            );
            quad(
                [
                    f + right * hw - up * hh,
                    b + right * hw - up * hh,
                    b + right * hw + up * hh,
                    f + right * hw + up * hh,
                ],
                strip,
                0.6,
            );
            self.cull_entity_vertices(out, entity_start);
        }
    }
}
