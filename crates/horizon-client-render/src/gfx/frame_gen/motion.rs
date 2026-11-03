//! Motion vectors for what moves on its own, for frame generation: the camera's motion
//! vectors (from the depth) see mobs and dropped items as still. The game tags each
//! entity's vertices with its id (`Frame::entity_motion`); an entity whose vertices have
//! the same shape as last frame's (the same count, UVs and layers) gets each vertex's
//! own motion, limbs and spin included, drawn into a texture at the render size
//! (`motion.wgsl`) that the interpolation reads before the camera's.

use crate::mesher::Vertex;
use glam::{DVec3, Mat4, Vec3};
use std::collections::HashMap;

/// `motion.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    jittered: [[f32; 4]; 4],
    current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
}

/// A vertex this frame and where it was last frame, both relative to this frame's camera.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionVertex {
    position: [f32; 3],
    previous: [f32; 3],
    uv: [f32; 2],
    /// 1 for a translucent item, whose texture says where it shows.
    translucent: u32,
}

pub struct ObjectMotion {
    pipeline: wgpu::RenderPipeline,
    atlas: wgpu::TextureView,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    vertices: wgpu::Buffer,
    /// Triangles of the entities matched this frame.
    staged: Vec<MotionVertex>,
    /// Last frame's vertices by entity, and its camera.
    tracked: HashMap<u64, Vec<Vertex>>,
    camera: DVec3,
    target: Option<([u32; 2], wgpu::TextureView)>,
}

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl ObjectMotion {
    /// `atlas`: the entities' texture atlas.
    pub fn new(device: &wgpu::Device, atlas: wgpu::TextureView) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Frame generation motion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("motion.wgsl").into()),
        });
        let attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Frame generation motion"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_motion"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MotionVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_motion"),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            atlas,
            sampler: device.create_sampler(&Default::default()),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Frame generation motion"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertices: vertex_buffer(device, 1 << 12),
            staged: Vec::new(),
            tracked: HashMap::new(),
            camera: DVec3::ZERO,
            target: None,
        }
    }

    /// Forget last frame's entities: the next frame has no motion of its own.
    pub fn reset(&mut self) {
        self.tracked.clear();
    }

    /// Match this frame's `entities` (relative to `camera`) to last frame's by the ids
    /// `spans` gives their vertices (start, id; 0 for none, each up to the next start), and
    /// keep them for the next frame.
    pub fn track(&mut self, entities: &[Vertex], spans: &[(u32, u64)], camera: DVec3) {
        self.staged.clear();
        let shift = (self.camera - camera).as_vec3();
        let mut tracked = HashMap::with_capacity(self.tracked.len());
        for (i, &(start, id)) in spans.iter().enumerate() {
            let end = spans.get(i + 1).map_or(entities.len(), |s| s.0 as usize);
            let Some(now) = entities
                .get(start as usize..end)
                .filter(|v| id != 0 && !v.is_empty())
            else {
                continue;
            };
            if let Some(then) = self.tracked.remove(&id)
                && then.len() == now.len()
                && then
                    .iter()
                    .zip(now)
                    .all(|(a, b)| a.uv == b.uv && a.layer == b.layer)
            {
                let pair = |i: usize| MotionVertex {
                    position: now[i].pos,
                    previous: (Vec3::from(then[i].pos) + shift).into(),
                    uv: now[i].uv,
                    translucent: u32::from(matches!(
                        now[i].layer,
                        super::super::entity_batch::TRANSLUCENT_ITEM_OVERLAY
                            | super::super::entity_batch::TRANSLUCENT_GLINT_ITEM_OVERLAY
                    )),
                };
                for q in (0..now.len() / 4).map(|q| q * 4) {
                    self.staged
                        .extend([q, q + 1, q + 2, q, q + 2, q + 3].map(pair));
                }
            }
            tracked.insert(id, now.to_vec());
        }
        self.tracked = tracked;
        self.camera = camera;
    }

    /// Draw the matched entities' motion vectors at `size` (the world's render size),
    /// where `depth` (the world's) shows them; the rest of the texture is 0.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        size: [u32; 2],
        jittered: Mat4,
        current: Mat4,
        previous: Mat4,
    ) -> wgpu::TextureView {
        if self.target.as_ref().is_none_or(|t| t.0 != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Frame generation motion"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.target = Some((size, texture.create_view(&Default::default())));
        }
        let target = self.target.as_ref().expect("made above").1.clone();
        let bytes: &[u8] = bytemuck::cast_slice(&self.staged);
        if bytes.len() as u64 > self.vertices.size() {
            self.vertices = vertex_buffer(device, (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytes);
        }
        let params = Params {
            jittered: jittered.to_cols_array_2d(),
            current: current.to_cols_array_2d(),
            previous: previous.to_cols_array_2d(),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frame generation motion"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Frame generation motion"),
            color_attachments: &attachments,
            ..Default::default()
        });
        if !self.staged.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..bytes.len() as u64));
            pass.draw(0..self.staged.len() as u32, 0..1);
        }
        drop(pass);
        target
    }
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame generation motion vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("motion.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    fn quad(x: f32, uv: f32) -> Vec<Vertex> {
        (0..4)
            .map(|i| Vertex {
                pos: [x + i as f32, 0.0, -5.0],
                uv: [uv, 0.0],
                ..bytemuck::Zeroable::zeroed()
            })
            .collect()
    }

    /// An entity seen last frame gets its vertices' own motion, in this frame's camera
    /// space; a new one, one whose shape changed and untagged geometry get none.
    #[test]
    fn tracks_each_entity_by_id() {
        let Some((device, queue)) = super::super::optical_flow::tests::device() else {
            return;
        };
        let atlas = super::super::optical_flow::tests::upload(&device, &queue, [1, 1], &[0; 4]);
        let mut motion = ObjectMotion::new(&device, atlas);
        let mut entities = quad(0.0, 0.0);
        entities.extend(quad(10.0, 0.5));
        motion.track(&entities, &[(0, 7), (4, 8)], DVec3::ZERO);
        assert!(motion.staged.is_empty());
        // Entity 7 moved 1 block right, the camera 2 blocks right; entity 8 became
        // another item; 9 is new; then untagged geometry.
        let mut entities = quad(-1.0, 0.0);
        entities.extend(quad(10.0, 0.25));
        entities.extend(quad(20.0, 0.0));
        entities.extend(quad(30.0, 0.0));
        motion.track(
            &entities,
            &[(0, 7), (4, 8), (8, 9), (12, 0)],
            DVec3::new(2.0, 0.0, 0.0),
        );
        assert_eq!(motion.staged.len(), 6);
        let v = motion.staged[0];
        assert_eq!(v.position, [-1.0, 0.0, -5.0]);
        assert_eq!(v.previous, [-2.0, 0.0, -5.0]);
        assert_eq!(motion.tracked.len(), 3);
    }
}
