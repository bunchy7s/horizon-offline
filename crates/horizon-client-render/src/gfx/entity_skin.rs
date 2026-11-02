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
//! The rest stays on the CPU path.

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
    pub spare: [u32; 3],
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
}

impl SkinFrame {
    /// Starts a frame's draws; models not yet handed to the renderer stay.
    pub fn clear_draws(&mut self) {
        self.parts.clear();
        self.jobs.clear();
        self.vertices = 0;
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
        self.reset || !self.new_model.is_empty() || !self.jobs.is_empty()
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
    /// `out` from vertex `base` on. Call after the CPU vertices' upload.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SkinFrame,
        out: &wgpu::Buffer,
        base: u32,
    ) {
        if frame.jobs.is_empty() {
            return;
        }
        let Some(model) = &self.model_buffer else {
            return;
        };
        // Whole rows of workgroups past `ROW` jobs: the padding jobs write nothing.
        let count = frame.jobs.len() as u32;
        let rows = count.div_ceil(ROW);
        let padded = if rows > 1 {
            (rows * ROW) as usize
        } else {
            frame.jobs.len()
        };
        self.staged.clear();
        self.staged.extend(frame.jobs.iter().map(|job| SkinJob {
            dst: job.dst + base,
            ..*job
        }));
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
    let mut out = vec![bytemuck::Zeroable::zeroed(); frame.vertices as usize];
    for job in &frame.jobs {
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
            out[(job.dst + k) as usize] = Vertex {
                pos: pos.to_array(),
                uv: [r[0] + (r[2] - r[0]) * u, r[1] + (r[3] - r[1]) * v.uv[1]],
                layer: job.overlay,
                color: job.color,
                light: job.light,
                normal: [packed[0], packed[1], packed[2], -127],
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
    /// texture and the armour glint: the shader writes what `expand` does,
    /// to the bit, after the vertices before them.
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
            ],
            vertices: 72,
        };
        let expected = expand(&model, &frame);
        // The posed vertices land after 5 others, which they leave alone.
        let before = 5u32;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(before + frame.vertices) * 44,
            usage: EntitySkin::usage(true) | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let sentinel = vec![0xabu8; before as usize * 44];
        queue.write_buffer(&out, 0, &sentinel);
        let mut uploads = Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(skin.register(&device, &mut uploads, &mut encoder, &frame));
        skin.dispatch(&device, &mut uploads, &mut encoder, &frame, &out, before);
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
    }
}
