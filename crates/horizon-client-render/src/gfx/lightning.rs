//! Seeded, bolt-local lightning meshes retained until their last active bolt disappears.
use crate::{gfx::upload::Uploads, mesher::Vertex};
use glam::DVec3;
use horizon_core::random::{LegacyRandom, RandomSource};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// A bolt's render state. Simulation lifetime and flashes stay in the game.
#[derive(Clone, Copy)]
pub struct Bolt {
    pub at: [f64; 3],
    pub seed: i64,
}

#[derive(Default)]
pub struct Input<'a> {
    pub bolts: &'a [Bolt],
    /// Current atlas white texel ([`white_uv`]); a changed rect invalidates
    /// retained vertices.
    pub uv: [f32; 2],
}

/// The atlas point an untextured quad (`RenderTypes.lightning` has no
/// texture) samples: the middle of the white sprite. Its corner (`u0`, `v0`)
/// is on the sprite's border, where the atlas's nearest sampling picks the
/// neighbouring, transparent texels for most fragments.
pub fn white_uv(white: &crate::hud::Rect) -> [f32; 2] {
    [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0]
}

// Four layers, fourteen segments (eight trunk, two branches of three), four quads.
pub(super) const QUADS: u32 = 4 * 14 * 4;

/// `LightningBoltRenderer.submit`: keep random draws, layers and quad order unchanged.
fn geometry(seed: i64, uv: [f32; 2]) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(QUADS as usize * 4);
    // ARGB 0.45, 0.45, 0.5 at 0.3 alpha, packed RGBA.
    let color = (76u32 << 24) | (127 << 16) | (114 << 8) | 114;
    let mut random = LegacyRandom::new(seed);
    let (mut xs, mut zs) = ([0f32; 8], [0f32; 8]);
    let (mut x_off, mut z_off) = (0f32, 0f32);
    for i in (0..8).rev() {
        xs[i] = x_off;
        zs[i] = z_off;
        x_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
        z_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
    }
    for layer in 0..4 {
        let mut random = LegacyRandom::new(seed);
        for branch in 0..3 {
            let trunk = branch == 0;
            let start = 7 - branch;
            let end = if trunk { 0 } else { start - 3 + 1 };
            let mut sx = xs[start] - x_off;
            let mut sz = zs[start] - z_off;
            for segment in (end..=start).rev() {
                let (ex, ez) = (sx, sz);
                let spread = if trunk { 11 } else { 31 };
                sx += (random.next_int_bound(spread).unwrap_or(0) - spread / 2) as f32;
                sz += (random.next_int_bound(spread).unwrap_or(0) - spread / 2) as f32;
                let mut top = 0.1 + layer as f32 * 0.2;
                let mut bottom = top;
                if trunk {
                    top *= segment as f32 * 0.1 + 1.0;
                    bottom *= (segment as f32 - 1.0) * 0.1 + 1.0;
                }
                for (rx, rz, lx, lz) in [
                    (false, false, true, false),
                    (true, false, true, true),
                    (true, true, false, true),
                    (false, true, false, false),
                ] {
                    let s = |p: bool, r: f32| if p { r } else { -r };
                    let y0 = segment as f32 * 16.0;
                    let corners = [
                        [sx + s(rx, bottom), y0, sz + s(rz, bottom)],
                        [ex + s(rx, top), y0 + 16.0, ez + s(rz, top)],
                        [ex + s(lx, top), y0 + 16.0, ez + s(lz, top)],
                        [sx + s(lx, bottom), y0, sz + s(lz, bottom)],
                    ];
                    for c in corners {
                        out.push(Vertex {
                            normal: [0; 4],
                            pos: c,
                            uv,
                            // HDR output: the bolt shines like the sun (entity.wgsl fs_hdr_mark).
                            layer: 2 | u32::from(crate::mesher::VERY_BRIGHT) << 16,
                            color,
                            light: 240 | 240 << 8,

                            glint_uv: [0.0, 0.0],
                        });
                    }
                }
            }
        }
    }
    out
}

struct Cached<T> {
    mesh: T,
    used: bool,
}

/// Shared seeds use the same mesh; storage is bounded by active unique seeds,
/// including after seed changes, removals, dimension switches and disconnects.
struct Cache<T> {
    uv: Option<[f32; 2]>,
    meshes: HashMap<i64, Cached<T>>,
}
impl<T> Cache<T> {
    fn new() -> Self {
        Self {
            uv: None,
            meshes: HashMap::new(),
        }
    }

    fn prepare(&mut self, input: &Input<'_>, mut build: impl FnMut(i64, [f32; 2]) -> T) {
        if self.uv != Some(input.uv) {
            self.meshes.clear();
            self.uv = Some(input.uv);
        }
        for cached in self.meshes.values_mut() {
            cached.used = false;
        }
        for bolt in input.bolts {
            self.meshes
                .entry(bolt.seed)
                .or_insert_with(|| Cached {
                    mesh: build(bolt.seed, input.uv),
                    used: true,
                })
                .used = true;
        }
        self.meshes.retain(|_, cached| cached.used);
        // HashMap::retain keeps its allocation. Release historical peak capacity
        // as well as meshes once the storm (or connection) is gone.
        if self.meshes.is_empty() {
            self.meshes = HashMap::new();
        } else if self.meshes.capacity() > self.meshes.len() * 4 {
            self.meshes.shrink_to_fit();
        }
    }
}

fn offset(bolt: &Bolt, camera: DVec3) -> [f32; 3] {
    (DVec3::from_array(bolt.at) - camera).as_vec3().to_array()
}

pub(super) struct Renderer {
    cache: Cache<wgpu::Buffer>,
    instances: Option<wgpu::Buffer>,
    capacity: usize,
}
impl Renderer {
    pub(super) fn new() -> Self {
        Self {
            cache: Cache::new(),
            instances: None,
            capacity: 0,
        }
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        input: &Input<'_>,
        camera: DVec3,
    ) {
        self.cache.prepare(input, |seed, uv| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Lightning vertices"),
                contents: bytemuck::cast_slice(&geometry(seed, uv)),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        if input.bolts.is_empty() {
            self.instances = None;
            self.capacity = 0;
            return;
        }
        if input.bolts.len() > self.capacity || input.bolts.len() < self.capacity / 4 {
            self.capacity = input.bolts.len().next_power_of_two();
            self.instances = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Lightning offsets"),
                size: (self.capacity * std::mem::size_of::<[f32; 3]>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let offsets: Vec<_> = input
            .bolts
            .iter()
            .map(|bolt| offset(bolt, camera))
            .collect();
        uploads.buffer(
            encoder,
            self.instances.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&offsets),
        );
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, input: &Input<'_>) {
        let Some(instances) = &self.instances else {
            return;
        };
        pass.set_vertex_buffer(1, instances.slice(..));
        // Follow simulation order, including repeated seeds; HashMap order must
        // never change additive accumulation or the order before dragon rays.
        for (i, bolt) in input.bolts.iter().enumerate() {
            pass.set_vertex_buffer(0, self.cache.meshes[&bolt.seed].mesh.slice(..));
            pass.draw_indexed(0..QUADS * 6, 0, i as u32..i as u32 + 1);
        }
    }
}

#[cfg(test)]
mod tests;
