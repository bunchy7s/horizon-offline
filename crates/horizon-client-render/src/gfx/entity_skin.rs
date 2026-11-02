//! Entity models posed on the GPU. The CPU path (`mobs::render::draw`) walks a
//! model's parts and pushes every vertex of every polygon, transformed by its
//! part's matrix: ~280 vertices of 44 bytes per mob, then copied by material
//! and uploaded each frame. Here each model's vertices are uploaded once, in
//! model space, and a frame only sends each drawn part's matrices and one job
//! per run of drawn parts; `entity_skin.wgsl` writes the same vertices, in the
//! same order, into the entity vertex buffer, after the CPU's ordinary entities
//! (`EntityBatch` leaves the gap). Every pass that draws ordinary entities
//! then draws them as before.
//!
//! The game decides which draws come here (`SkinFrame`): models whose
//! polygons all face an axis, drawn as ordinary opaque or cutout entities.
//! The rest stays on the CPU path. Item models come too ([`SkinJob::item`]):
//! their quads all face an axis, and their shading is baked into the colour.

use super::upload::Uploads;
use crate::mesher::Vertex;

/// A model vertex: position / 16, model UV, the face's normal as an axis (0..2
/// = +x, +y, +z; 3..5 = -x, -y, -z), the part's index in its model.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub axis: u32,
    pub part: u32,
}

/// A drawn part: its geometry matrix and its normal matrix, by columns, the
/// normal matrix's columns normalized when the part's scale is not uniform.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinPart {
    pub matrix: [f32; 16],
    pub normal: [f32; 9],
}

impl SkinPart {
    /// An item layer's part: its pose, and its quads' colours by axis.
    pub fn item(matrix: glam::Mat4, colors: [u32; 6]) -> Self {
        let mut normal = [0.0; 9];
        for (word, color) in normal.iter_mut().zip(colors) {
            *word = f32::from_bits(color);
        }
        Self {
            matrix: matrix.to_cols_array(),
            normal,
        }
    }
}

/// One run of a model's vertices drawn with one texture rect, colour, light
/// and overlay. `dst` counts from the frame's first posed vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinJob {
    pub first: u32,
    pub count: u32,
    pub part: u32,
    pub dst: u32,
    pub rect: [f32; 4],
    pub scroll: f32,
    pub color: u32,
    pub light: u32,
    pub overlay: u32,
    /// 1 when the overlay is the armour glint: the model UV rides in `glint_uv`.
    pub glint: u32,
    /// How the vertices are finished: [`SkinJob::MODEL`], [`SkinJob::ITEM`] or
    /// [`SkinJob::FLAT`].
    pub mode: u32,
    /// 1 for a model whose render type culls back faces (`mobs::Polygon::cull_back`):
    /// the normal's w is `mesher::SHADED_CULL_W` instead of -127.
    pub cull: u32,
    /// Always 0: the shader reads it as a zero it cannot fold away.
    pub spare: u32,
}

impl SkinJob {
    /// A model's polygons: the face normal posed, the job's colour.
    pub const MODEL: u32 = 0;
    /// An item model's quads (`item_render::WorldSink`): the colour by the face's
    /// axis rides in the part's first 6 normal words ([`SkinPart::item`]), shaded on
    /// the CPU, and the normal is `mesher::CULL_BACK`.
    pub const ITEM: u32 = 1;
    /// A model's polygons with no normal at all (all four bytes 0): the eyes render
    /// type's layers, which entity.wgsl leaves unshaded.
    pub const FLAT: u32 = 2;
}

const _: () = assert!(std::mem::size_of::<Vertex>() == 44);
const _: () = assert!(std::mem::size_of::<SkinVertex>() == 7 * 4);
const _: () = assert!(std::mem::size_of::<SkinPart>() == 25 * 4);
const _: () = assert!(std::mem::size_of::<SkinJob>() == 16 * 4);

/// What the game hands the renderer for one frame.
#[derive(Default)]
pub struct SkinFrame {
    /// The model store restarted (models are registered again from 0).
    pub reset: bool,
    /// The store's size before `new_model`: the renderer takes the frame only
    /// when its own store is that large (it missed none of the earlier ones).
    pub base: u32,
    /// Model vertices registered since the last frame, at the store's end.
    pub new_model: Vec<SkinVertex>,
    pub parts: Vec<SkinPart>,
    pub jobs: Vec<SkinJob>,
    /// Vertices the jobs write: a whole number of quads.
    pub vertices: u32,
    /// Glowing entities' silhouettes posed here (`OutlineBufferSource`'s copy of
    /// their draws, [`SkinFrame::outline_since`]): `dst` counts from the first.
    pub outline_jobs: Vec<SkinJob>,
    /// Vertices the outline jobs write.
    pub outline_vertices: u32,
}

impl SkinFrame {
    /// Starts a frame's draws; models not yet handed to the renderer stay.
    pub fn clear_draws(&mut self) {
        self.parts.clear();
        self.jobs.clear();
        self.vertices = 0;
        self.outline_jobs.clear();
        self.outline_vertices = 0;
    }

    /// Copies the draws since job `first` into the glowing entities' outline
    /// (`Frame::glowing`'s, on the GPU), as `glowing.rs`'s `mark_outline` copies
    /// vertices: the same ones, in `color` (`Vertex::color`) with no normal.
    pub fn outline_since(&mut self, first: usize, color: u32) {
        for i in first..self.jobs.len() {
            let job = self.jobs[i];
            self.outline_jobs.push(SkinJob {
                dst: self.outline_vertices,
                color,
                mode: SkinJob::FLAT,
                ..job
            });
            self.outline_vertices += job.count;
        }
    }

    /// The renderer took the new models.
    pub fn handed(&mut self) {
        self.base += self.new_model.len() as u32;
        self.new_model.clear();
        self.reset = false;
    }

    /// Where the draws stand, to take back a mob's (`truncate`).
    pub fn mark(&self) -> (usize, usize, u32) {
        (self.jobs.len(), self.parts.len(), self.vertices)
    }

    /// Takes back the draws since `mark` (a mob culled after drawing).
    pub fn truncate(&mut self, (jobs, parts, vertices): (usize, usize, u32)) {
        self.jobs.truncate(jobs);
        self.parts.truncate(parts);
        self.vertices = vertices;
    }

    /// Whether the renderer has anything to take.
    pub fn pending(&self) -> bool {
        self.reset
            || !self.new_model.is_empty()
            || !self.jobs.is_empty()
            || !self.outline_jobs.is_empty()
    }
}

/// Jobs per row of workgroups (the dispatch limit is 65535 per dimension).
const ROW: u32 = 32_768;

pub(super) struct EntitySkin {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    model: Vec<SkinVertex>,
    model_buffer: Option<wgpu::Buffer>,
    parts: Grow,
    jobs: Grow,
    /// The jobs with `dst` moved to the frame's gap, as uploaded.
    staged: Vec<SkinJob>,
    /// The bind group and the buffers it binds (buffers compare by identity).
    group: Option<(wgpu::BindGroup, [wgpu::Buffer; 4])>,
}

/// A storage buffer that grows to the next power of two.
struct Grow {
    label: &'static str,
    buffer: Option<wgpu::Buffer>,
}

impl Grow {
    fn ensure(&mut self, device: &wgpu::Device, bytes: u64) -> bool {
        let bytes = bytes.max(256).next_power_of_two();
        if self.buffer.as_ref().is_some_and(|b| b.size() >= bytes) {
            return false;
        }
        self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        true
    }
}

impl EntitySkin {
    /// The entity vertex buffer's usage: the jobs write into it.
    pub(super) fn usage(skinning: bool) -> wgpu::BufferUsages {
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        if skinning {
            usage | wgpu::BufferUsages::STORAGE
        } else {
            usage
        }
    }

    /// The compute pipeline, or `None` without compute shaders (the CPU path
    /// then draws every model).
    pub(super) fn new(device: &wgpu::Device, compute: bool) -> Option<Self> {
        // HORIZON_NO_ENTITY_SKIN: every model on the CPU path (A/B runs, a driver's bug).
        if !compute
            || device.limits().max_storage_buffers_per_shader_stage < 4
            || std::env::var_os("HORIZON_NO_ENTITY_SKIN").is_some()
        {
            return None;
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Entity skinning"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity_skin.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Entity skinning"),
            layout: None,
            module: &module,
            entry_point: Some("skin"),
            compilation_options: Default::default(),
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("Horizon renderer: entity models posed on the CPU ({e})");
            return None;
        }
        Some(Self {
            pipeline,
            layout,
            model: Vec::new(),
            model_buffer: None,
            parts: Grow {
                label: "Entity skinning parts",
                buffer: None,
            },
            jobs: Grow {
                label: "Entity skinning jobs",
                buffer: None,
            },
            staged: Vec::new(),
            group: None,
        })
    }

    /// The model vertices stored (`SkinFrame::base` of the next frame).
    pub(super) fn stored(&self) -> u32 {
        self.model.len() as u32
    }

    /// Takes the frame's new models; whether its jobs can run (the store
    /// holds every model they read).
    pub(super) fn register(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SkinFrame,
    ) -> bool {
        if frame.reset {
            self.model.clear();
            self.model_buffer = None;
        }
        if self.model.len() as u32 != frame.base {
            // Out of step with the game's store: it restarts it.
            return false;
        }
        if frame.new_model.is_empty() {
            return self.model_buffer.is_some() || frame.jobs.is_empty();
        }
        let old = self.model.len();
        self.model.extend_from_slice(&frame.new_model);
        let bytes = (self.model.len() * std::mem::size_of::<SkinVertex>()) as u64;
        if self
            .model_buffer
            .as_ref()
            .is_some_and(|b| b.size() >= bytes)
        {
            let buffer = self.model_buffer.as_ref().expect("checked");
            uploads.buffer(
                encoder,
                buffer,
                (old * std::mem::size_of::<SkinVertex>()) as u64,
                bytemuck::cast_slice(&frame.new_model),
            );
        } else {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Entity skinning models"),
                size: bytes.max(256).next_power_of_two(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            uploads.buffer(encoder, &buffer, 0, bytemuck::cast_slice(&self.model));
            self.model_buffer = Some(buffer);
        }
        true
    }

    /// Records the frame's jobs: they write `frame.vertices` vertices into
    /// `out` from vertex `base` on, and the outline jobs `frame.outline_vertices`
    /// from vertex `outline_base` on. Call after the CPU vertices' upload.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SkinFrame,
        out: &wgpu::Buffer,
        base: u32,
        outline_base: u32,
    ) {
        if frame.jobs.is_empty() && frame.outline_jobs.is_empty() {
            return;
        }
        let Some(model) = &self.model_buffer else {
            return;
        };
        // Whole rows of workgroups past `ROW` jobs: the padding jobs write nothing.
        let count = (frame.jobs.len() + frame.outline_jobs.len()) as u32;
        let rows = count.div_ceil(ROW);
        let padded = if rows > 1 {
            (rows * ROW) as usize
        } else {
            count as usize
        };
        self.staged.clear();
        let moved = |at: u32| {
            move |job: &SkinJob| SkinJob {
                dst: job.dst + at,
                ..*job
            }
        };
        self.staged.extend(frame.jobs.iter().map(moved(base)));
        self.staged
            .extend(frame.outline_jobs.iter().map(moved(outline_base)));
        self.staged.resize(padded, SkinJob::default());
        self.parts.ensure(
            device,
            (frame.parts.len() * std::mem::size_of::<SkinPart>()) as u64,
        );
        self.jobs
            .ensure(device, (padded * std::mem::size_of::<SkinJob>()) as u64);
        let (Some(parts), Some(jobs)) = (&self.parts.buffer, &self.jobs.buffer) else {
            return;
        };
        uploads.buffer(encoder, parts, 0, bytemuck::cast_slice(&frame.parts));
        uploads.buffer(encoder, jobs, 0, bytemuck::cast_slice(&self.staged));
        let key = [model.clone(), parts.clone(), jobs.clone(), out.clone()];
        if self.group.as_ref().is_none_or(|(_, k)| *k != key) {
            let entries: Vec<_> = key
                .iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Entity skinning"),
                layout: &self.layout,
                entries: &entries,
            });
            self.group = Some((group, key));
        }
        let Some((group, _)) = &self.group else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Entity skinning"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(count.min(ROW), rows, 1);
    }
}

/// What `entity_skin.wgsl` writes for `frame`'s jobs, computed on the CPU
/// the same way: the reference its tests hold the shader and the game's walk
/// (`mobs::skin`) to. `model` is the whole store.
pub fn expand(model: &[SkinVertex], frame: &SkinFrame) -> Vec<Vertex> {
    expand_jobs(model, frame, &frame.jobs, frame.vertices)
}

/// [`expand`] of the frame's outline jobs: the silhouettes they write.
pub fn expand_outline(model: &[SkinVertex], frame: &SkinFrame) -> Vec<Vertex> {
    expand_jobs(model, frame, &frame.outline_jobs, frame.outline_vertices)
}

fn expand_jobs(
    model: &[SkinVertex],
    frame: &SkinFrame,
    jobs: &[SkinJob],
    vertices: u32,
) -> Vec<Vertex> {
    let mut out = vec![bytemuck::Zeroable::zeroed(); vertices as usize];
    for job in jobs {
        for k in 0..job.count {
            let v = model[(job.first + k) as usize];
            let part = &frame.parts[(job.part + v.part) as usize];
            let m = glam::Mat4::from_cols_array(&part.matrix);
            let pos = m.transform_point3(glam::Vec3::from_array(v.pos));
            let c = (v.axis % 3) as usize * 3;
            let column = glam::Vec3::from_slice(&part.normal[c..c + 3]);
            let n = if v.axis >= 3 { -column } else { column };
            let packed = n.to_array().map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
            let u = if job.scroll == 0.0 {
                v.uv[0]
            } else {
                (v.uv[0] + job.scroll) % 1.0
            };
            let r = job.rect;
            let (color, normal) = if job.mode == SkinJob::ITEM {
                let color = part.normal[v.axis as usize].to_bits();
                (color, crate::mesher::CULL_BACK)
            } else if job.mode == SkinJob::FLAT {
                (job.color, [0; 4])
            } else {
                let w = if job.cull != 0 {
                    crate::mesher::SHADED_CULL_W
                } else {
                    -127
                };
                (job.color, [packed[0], packed[1], packed[2], w])
            };
            out[(job.dst + k) as usize] = Vertex {
                pos: pos.to_array(),
                uv: [r[0] + (r[2] - r[0]) * u, r[1] + (r[3] - r[1]) * v.uv[1]],
                layer: job.overlay,
                color,
                light: job.light,
                normal,
                glint_uv: if job.glint != 0 { v.uv } else { [0.0, 0.0] },
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// A cube's 24 vertices for part `part`, its faces on every axis.
    fn cube(part: u32) -> Vec<SkinVertex> {
        let mut out = Vec::new();
        for axis in 0..6u32 {
            for corner in 0..4u32 {
                let a = (axis % 3) as usize;
                let sign = if axis >= 3 { -0.25 } else { 0.25 };
                let mut pos = [0.0f32; 3];
                pos[a] = sign;
                pos[(a + 1) % 3] = if corner & 1 == 0 { -0.25 } else { 0.25 } + 0.1 * part as f32;
                pos[(a + 2) % 3] = if corner & 2 == 0 { -0.3 } else { 0.3 };
                out.push(SkinVertex {
                    pos,
                    uv: [corner as f32 / 3.0, 0.125 * axis as f32 + 0.0625],
                    axis,
                    part,
                });
            }
        }
        out
    }

    fn part(m: glam::Mat4, normal: glam::Mat3) -> SkinPart {
        SkinPart {
            matrix: m.to_cols_array(),
            normal: normal.to_cols_array(),
        }
    }

    /// Two draws of a two-part model with turned, scaled parts, a scrolled
    /// texture and the armour glint, and an item's: the shader writes what
    /// `expand` does, to the bit, after the vertices before them.
    #[test]
    fn the_shader_writes_what_the_cpu_reference_computes() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let Some(mut skin) = EntitySkin::new(&device, true) else {
            eprintln!("no compute shaders: skipped");
            return;
        };
        let model: Vec<SkinVertex> = cube(0).into_iter().chain(cube(1)).collect();
        let turn =
            glam::Mat4::from_quat(glam::Quat::from_euler(glam::EulerRot::ZYX, 0.3, -1.1, 2.0));
        let base = glam::Mat4::from_translation(glam::Vec3::new(-3.7, 1.25, 9.1)) * turn;
        let child = base
            * glam::Mat4::from_translation(glam::Vec3::new(0.2, -0.4, 0.05))
            * glam::Mat4::from_scale(glam::Vec3::new(1.0, 2.5, 0.5));
        let normals = glam::Mat3::from_mat4(child).inverse().transpose();
        let normals = glam::Mat3::from_cols(
            normals.x_axis.normalize_or_zero(),
            normals.y_axis.normalize_or_zero(),
            normals.z_axis.normalize_or_zero(),
        );
        let frame = SkinFrame {
            reset: false,
            base: 0,
            new_model: model.clone(),
            parts: vec![
                part(base, glam::Mat3::from_mat4(turn)),
                part(child, normals),
                part(base * turn, glam::Mat3::from_mat4(turn * turn)),
                part(child * turn, glam::Mat3::IDENTITY),
                // Colours whose bits as floats are NaNs and denormals: copied, never computed.
                SkinPart::item(
                    child * turn,
                    [
                        0xff80_4020,
                        0x7fc0_0001,
                        0x0000_0001,
                        0xffff_ffff,
                        0x7f80_0001,
                        0x1234_5678,
                    ],
                ),
            ],
            jobs: vec![
                SkinJob {
                    first: 0,
                    count: 48,
                    part: 0,
                    dst: 0,
                    rect: [0.25, 0.5, 0.375, 0.5625],
                    scroll: 0.0,
                    color: 0xff80_40ff,
                    light: 0xf0_70,
                    overlay: 1,
                    // The bat's encoding: the shader must write -126 like `expand`.
                    cull: 1,
                    ..Default::default()
                },
                SkinJob {
                    first: 24,
                    count: 24,
                    part: 2,
                    dst: 48,
                    rect: [0.0, 0.0, 0.0625, 0.0625],
                    scroll: 0.73,
                    color: u32::MAX,
                    light: 0xf0f0,
                    overlay: 36,
                    glint: 1,
                    ..Default::default()
                },
                // An item's quads: colour by axis from its part, `CULL_BACK` normals.
                SkinJob {
                    first: 0,
                    count: 24,
                    part: 4,
                    dst: 72,
                    rect: [0.5, 0.25, 0.5625, 0.3125],
                    light: 0xa0_30,
                    overlay: 2 << 16,
                    mode: SkinJob::ITEM,
                    ..Default::default()
                },
                // An eyes layer: no normal, even over a culled model's encoding; the
                // tier and OIT tag ride in the overlay word.
                SkinJob {
                    first: 24,
                    count: 24,
                    part: 1,
                    dst: 96,
                    rect: [0.125, 0.75, 0.25, 0.875],
                    color: 0x80ff_ffff,
                    light: 0xf0f0,
                    overlay: crate::gfx::OIT_ENTITY | 3 << 16 | 7,
                    mode: SkinJob::FLAT,
                    cull: 1,
                    ..Default::default()
                },
            ],
            vertices: 120,
            outline_jobs: Vec::new(),
            outline_vertices: 0,
        };
        // A glowing entity's silhouette: the item job and the first job again,
        // in its team colour with no normal.
        let mut frame = frame;
        frame.jobs.push(frame.jobs[2]);
        frame.jobs.last_mut().expect("pushed").dst = 120;
        frame.jobs.push(SkinJob {
            dst: 144,
            ..frame.jobs[0]
        });
        frame.vertices = 192;
        frame.outline_since(4, 0xff20_c0e0);
        assert_eq!(frame.outline_vertices, 72);
        let expected = expand(&model, &frame);
        let outline = expand_outline(&model, &frame);
        // The posed vertices land after 5 others, which they leave alone, and the
        // silhouettes 3 past them.
        let before = 5u32;
        let outline_base = before + frame.vertices + 3;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(outline_base + frame.outline_vertices) * 44,
            usage: EntitySkin::usage(true) | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let sentinel = vec![0xabu8; before as usize * 44];
        queue.write_buffer(&out, 0, &sentinel);
        let mut uploads = Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(skin.register(&device, &mut uploads, &mut encoder, &frame));
        skin.dispatch(
            &device,
            &mut uploads,
            &mut encoder,
            &frame,
            &out,
            before,
            outline_base,
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: out.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&out, 0, &read, 0, out.size());
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range().unwrap();
        assert_eq!(&data[..sentinel.len()], &sentinel[..]);
        let got: &[Vertex] = bytemuck::cast_slice(&data[sentinel.len()..]);
        for (i, (got, want)) in got.iter().zip(&expected).enumerate() {
            assert_eq!(
                bytemuck::bytes_of(got),
                bytemuck::bytes_of(want),
                "vertex {i}: {got:?} != {want:?}"
            );
        }
        let silhouettes = &got[(outline_base - before) as usize..];
        assert_eq!(silhouettes.len(), outline.len());
        for (i, (got, want)) in silhouettes.iter().zip(&outline).enumerate() {
            assert_eq!(
                bytemuck::bytes_of(got),
                bytemuck::bytes_of(want),
                "silhouette {i}: {got:?} != {want:?}"
            );
        }
        // A silhouette is its draw's vertex with the colour and no normal
        // (`mark_outline`).
        for (copy, of) in outline.iter().zip(&expected[120..]) {
            let marked = Vertex {
                color: 0xff20_c0e0,
                normal: [0; 4],
                ..*of
            };
            assert_eq!(bytemuck::bytes_of(copy), bytemuck::bytes_of(&marked));
        }
    }
}
