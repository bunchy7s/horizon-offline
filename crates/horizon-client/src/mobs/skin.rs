//! Mob models posed on the GPU (`gfx::entity_skin`): the store of models
//! uploaded once, and the walk that turns one `render::draw` into the parts'
//! matrices and jobs the shader expands into the very vertices `draw` would
//! push, in its order.
//!
//! A model takes this path only when every polygon faces an axis: the
//! normal matrix times an axis is one of its columns, exactly, so the shader's
//! packed normals are the CPU's to the bit. Positions and UVs are the same
//! sums in the same order (`Mat4::transform_point3`).

use super::{ModelNormals, Part, anim::Poses};
use crate::fast_hash::FxHashMap;
use crate::gfx::entity_skin::{SkinFrame, SkinJob, SkinPart, SkinVertex};
use std::sync::Arc;

/// A stored model: where its vertices start, and per part (pre-order) its
/// own vertices and the end of its subtree, for the walk.
struct Stored {
    first: u32,
    parts: Vec<StoredPart>,
    /// The model's bounds in its parts' own space, per part: min and max of
    /// its own vertices (none: no polygons).
    bounds: Vec<Option<([f32; 3], [f32; 3])>>,
}

#[derive(Clone, Copy)]
struct StoredPart {
    /// The part's first vertex, from the model's.
    first: u32,
    count: u32,
    /// One past the last part of its subtree.
    end: u32,
}

/// The models on the GPU, by the `Arc` they are drawn from. An entry holds
/// its `Arc`, so the address is never reused while stored; models no one
/// else holds any more are dropped by `begin`.
#[derive(Default)]
pub struct SkinModels {
    stored: FxHashMap<usize, (Arc<Part>, Option<Stored>)>,
    vertices: u32,
}

/// Models whose store would grow past this restart it (a pack reload's
/// models pile up otherwise).
const STORE_LIMIT: u32 = 4 << 20;

impl SkinModels {
    /// Starts a frame: drops the models nothing else holds (their vertices
    /// stay in the store until it restarts), restarts the store when it grew
    /// too large or the renderer's (`stored`, its size) is out of step.
    pub fn begin(&mut self, frame: &mut SkinFrame, stored: u32) {
        if self.vertices > STORE_LIMIT || stored + frame.new_model.len() as u32 != self.vertices {
            self.restart(frame);
        }
        self.stored
            .retain(|_, (model, _)| Arc::strong_count(model) > 1);
        frame.clear_draws();
    }

    fn restart(&mut self, frame: &mut SkinFrame) {
        self.stored.clear();
        self.vertices = 0;
        frame.new_model.clear();
        frame.base = 0;
        frame.reset = true;
    }

    fn entry(&mut self, model: &Arc<Part>, frame: &mut SkinFrame) -> Option<&Stored> {
        let key = Arc::as_ptr(model) as usize;
        let vertices = &mut self.vertices;
        let (_, stored) = self.stored.entry(key).or_insert_with(|| {
            // The renderer appends `new_model` to its store in order, so a
            // model's first vertex is the store's size when it is added.
            let before = frame.new_model.len();
            let stored = flatten(model, *vertices, &mut frame.new_model);
            *vertices += (frame.new_model.len() - before) as u32;
            (model.clone(), stored)
        });
        stored.as_ref()
    }
}

/// The model's vertices in pre-order (a part's polygons, then its children),
/// appended to `out`; `None` when a polygon does not face an axis.
fn flatten(model: &Part, first: u32, out: &mut Vec<SkinVertex>) -> Option<Stored> {
    let start = out.len();
    let mut parts = Vec::new();
    let mut bounds = Vec::new();
    fn walk(
        part: &Part,
        start: usize,
        out: &mut Vec<SkinVertex>,
        parts: &mut Vec<StoredPart>,
        bounds: &mut Vec<Option<([f32; 3], [f32; 3])>>,
    ) -> bool {
        let index = parts.len();
        let own = (out.len() - start) as u32;
        parts.push(StoredPart {
            first: own,
            count: 0,
            end: 0,
        });
        bounds.push(None);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for poly in &part.polygons {
            let Some(axis) = axis(poly.normal) else {
                return false;
            };
            for v in poly.vertices {
                let pos = (glam::Vec3::new(v[0], v[1], v[2]) / 16.0).to_array();
                for a in 0..3 {
                    lo[a] = lo[a].min(pos[a]);
                    hi[a] = hi[a].max(pos[a]);
                }
                out.push(SkinVertex {
                    pos,
                    uv: [v[3], v[4]],
                    axis,
                    part: index as u32,
                });
            }
        }
        parts[index].count = (out.len() - start) as u32 - own;
        if !part.polygons.is_empty() {
            bounds[index] = Some((lo, hi));
        }
        for child in &part.children {
            if !walk(child, start, out, parts, bounds) {
                return false;
            }
        }
        parts[index].end = parts.len() as u32;
        true
    }
    if !walk(model, start, out, &mut parts, &mut bounds) {
        out.truncate(start);
        return None;
    }
    Some(Stored {
        first,
        parts,
        bounds,
    })
}

/// 0..2 for +x, +y, +z, 3..5 for -x, -y, -z; `None` for any other normal.
fn axis(n: [f32; 3]) -> Option<u32> {
    let mut found = None;
    for (a, &c) in n.iter().enumerate() {
        if c == 1.0 || c == -1.0 {
            if found.is_some() {
                return None;
            }
            found = Some(a as u32 + if c < 0.0 { 3 } else { 0 });
        } else if c != 0.0 {
            return None;
        }
    }
    found
}

/// One draw's parameters (`render::draw_scrolled`'s).
#[derive(Clone, Copy)]
pub struct Paint {
    pub uv: [f32; 4],
    pub scroll: f32,
    pub color: u32,
    pub light: u32,
    pub overlay: u32,
}

/// Where a frame's posed draws go, and the bounds of the current mob's.
pub struct Skin<'a> {
    pub models: &'a mut SkinModels,
    pub frame: &'a mut SkinFrame,
    /// Camera-relative bounds of what this mob drew here (for culling).
    pub bounds: Option<(glam::Vec3, glam::Vec3)>,
}

impl Skin<'_> {
    /// Whether this model can be posed here (stored on first sight).
    pub fn supports(&mut self, model: &Arc<Part>) -> bool {
        self.models.entry(model, self.frame).is_some()
    }

    /// `render::draw_scrolled` on the GPU: false (nothing done) when the
    /// model cannot take this path.
    pub fn draw(
        &mut self,
        model: &Arc<Part>,
        base: glam::Mat4,
        poses: &Poses,
        paint: Paint,
    ) -> bool {
        let Some(stored) = self.models.entry(model, self.frame) else {
            return false;
        };
        let first_part = self.frame.parts.len() as u32;
        self.frame.parts.resize(
            self.frame.parts.len() + stored.parts.len(),
            bytemuck::Zeroable::zeroed(),
        );
        let mut walk = Walk {
            stored,
            poses,
            parts: &mut self.frame.parts[first_part as usize..],
            jobs: &mut self.frame.jobs,
            vertices: &mut self.frame.vertices,
            bounds: &mut self.bounds,
            first_part,
            paint,
            index: 0,
        };
        walk.part(model, base, ModelNormals::new(base));
        true
    }
}

struct Walk<'a> {
    stored: &'a Stored,
    poses: &'a Poses,
    parts: &'a mut [SkinPart],
    jobs: &'a mut Vec<SkinJob>,
    vertices: &'a mut u32,
    bounds: &'a mut Option<(glam::Vec3, glam::Vec3)>,
    first_part: u32,
    paint: Paint,
    /// The pre-order index of the part being walked.
    index: u32,
}

impl Walk<'_> {
    /// `render::draw_normals` without the vertices: the part's matrices, and
    /// its polygons as (part of) a job.
    fn part(&mut self, part: &Part, base: glam::Mat4, normals: ModelNormals) {
        let index = self.index;
        let stored = self.stored.parts[index as usize];
        let p = self.poses.pose(part);
        let m = super::render::posed(base, p);
        if self.poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
            // The whole subtree is left out.
            self.index = stored.end;
            return;
        }
        let normals = normals.part(p);
        let n = if normals.trusted {
            normals.matrix
        } else {
            glam::Mat3::from_cols(
                normals.matrix.x_axis.normalize_or_zero(),
                normals.matrix.y_axis.normalize_or_zero(),
                normals.matrix.z_axis.normalize_or_zero(),
            )
        };
        self.parts[index as usize] = SkinPart {
            matrix: m.to_cols_array(),
            normal: n.to_cols_array(),
        };
        if !self.poses.skips(part) && stored.count > 0 {
            self.emit(stored);
            if let Some((lo, hi)) = self.stored.bounds[index as usize] {
                let (lo, hi) = (glam::Vec3::from_array(lo), glam::Vec3::from_array(hi));
                let center = m.transform_point3((lo + hi) * 0.5);
                let half = (hi - lo) * 0.5;
                let r = glam::Mat3::from_mat4(m);
                let extent =
                    r.x_axis.abs() * half.x + r.y_axis.abs() * half.y + r.z_axis.abs() * half.z;
                let (a, b) = (center - extent, center + extent);
                *self.bounds = Some(match *self.bounds {
                    Some((l, h)) => (l.min(a), h.max(b)),
                    None => (a, b),
                });
            }
        }
        self.index += 1;
        for child in &part.children {
            self.part(child, m, normals);
        }
    }

    /// The part's vertices: extends the last job when they follow its own.
    fn emit(&mut self, stored: StoredPart) {
        let first = self.stored.first + stored.first;
        let paint = self.paint;
        if let Some(job) = self.jobs.last_mut()
            && job.first + job.count == first
            && job.part == self.first_part
            && job.dst + job.count == *self.vertices
        {
            job.count += stored.count;
        } else {
            self.jobs.push(SkinJob {
                first,
                count: stored.count,
                part: self.first_part,
                dst: *self.vertices,
                rect: paint.uv,
                scroll: paint.scroll,
                color: paint.color,
                light: paint.light,
                overlay: paint.overlay,
                glint: u32::from(paint.overlay == super::render::ARMOR_GLINT),
                spare: [0; 3],
            });
        }
        *self.vertices += stored.count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::entity_skin::expand;
    use crate::mobs::{Mob, MobData, MobTypes, render};

    /// Every vanilla mob, at a few ages and turns: the GPU path's vertices
    /// (its jobs expanded as `entity_skin.wgsl` does) are the CPU path's, to
    /// the bit and in the same order, for each mob it takes; the mobs it
    /// leaves to the CPU draw exactly as before. Needs the prepared resources.
    #[test]
    fn posed_on_the_gpu_every_mob_draws_what_the_cpu_draws() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped");
            return;
        }
        let types = MobTypes::load(&root).unwrap();
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: MobData::load_animations(&root),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        };
        // Every texture its own rect, so a swapped texture shows.
        let atlas = |name: &str| {
            let h = name
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
            let u = (h % 61) as f32 / 64.0;
            let v = (h / 61 % 61) as f32 / 64.0;
            Some([u, v, u + 1.0 / 64.0, v + 1.0 / 32.0])
        };
        let camera = glam::DVec3::new(0.3, 62.0, -4.0);
        let mut models = SkinModels::default();
        let mut frame = SkinFrame::default();
        let mut store: Vec<crate::gfx::entity_skin::SkinVertex> = Vec::new();
        let (mut posed, mut kept) = (0, 0);
        let mut ids: Vec<_> = types.types.keys().cloned().collect();
        ids.sort();
        for (n, id) in ids.iter().enumerate() {
            let kind = Arc::new(types.types[id].clone());
            for (age, yaw) in [(0u32, 0.0f32), (37, 123.0), (412, -71.5)] {
                let at = horizon_core::geometry::Vec3::new(1.5 + n as f64 % 5.0, 64.0, 3.25);
                let mut m = Mob::new(n as u32 + 1, kind.clone(), at, yaw);
                m.previous = horizon_core::geometry::Vec3::new(at.x - 0.1, at.y, at.z + 0.05);
                m.age = age;
                m.limb_speed = 0.6;
                m.limb_position = age as f32 * 0.37;
                let mut cpu = Vec::new();
                render::render_with(&m, 0.4, camera, &atlas, 0x50a0, &data, &[], &mut cpu);
                models.begin(&mut frame, store.len() as u32);
                let mut gpu = Vec::new();
                let mut skin = Skin {
                    models: &mut models,
                    frame: &mut frame,
                    bounds: None,
                };
                render::render_skinned(&m, 0.4, camera, &atlas, 0x50a0, &data, &mut gpu, &mut skin);
                let bounds = skin.bounds;
                if frame.reset {
                    store.clear();
                }
                assert_eq!(store.len() as u32, frame.base);
                store.extend_from_slice(&frame.new_model);
                frame.handed();
                if frame.jobs.is_empty() {
                    kept += 1;
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&gpu),
                        bytemuck::cast_slice::<_, u8>(&cpu),
                        "{id} age {age}: the CPU path changed"
                    );
                    continue;
                }
                posed += 1;
                assert!(gpu.is_empty(), "{id}: a posed mob left vertices on the CPU");
                let expanded = expand(&store, &frame);
                assert_eq!(expanded.len(), cpu.len(), "{id} age {age}: vertex count");
                for (i, (a, b)) in expanded.iter().zip(&cpu).enumerate() {
                    assert_eq!(
                        bytemuck::bytes_of(a),
                        bytemuck::bytes_of(b),
                        "{id} age {age} vertex {i}: {a:?} != {b:?}"
                    );
                }
                // The culling bounds hold every vertex.
                let (lo, hi) = bounds.expect("a posed mob has bounds");
                for v in &cpu {
                    let p = glam::Vec3::from_array(v.pos);
                    assert!(
                        p.cmpge(lo - 1e-4).all() && p.cmple(hi + 1e-4).all(),
                        "{id}: {p} outside {lo}..{hi}"
                    );
                }
            }
        }
        eprintln!("posed on the GPU: {posed} mob draws; kept on the CPU: {kept}");
        assert!(posed > 3 * 40, "only {posed} draws took the GPU path");
    }
}
