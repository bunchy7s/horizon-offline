//! The first-person hand and held items drawn by a shader pack's `gbuffers_hand` (spec 2.3):
//! `Frame::hand`'s entity vertices, read as entities are (`pack_entities::inputs`), with the
//! hand's own projection as `gl_ProjectionMatrix` (5.2: `gbufferProjection` stays the world's).

use glam::Mat4;

use super::Vertex;

/// `MC_HAND_DEPTH`'s value (1.5): the hand's depth is scaled by it through its projection, so
/// that it sits in front of the world in the shared depth buffer.
const HAND_DEPTH: f32 = 0.125;

/// `HzHand`: the hand's projection and its inverse.
const UNIFORM: u64 = 2 * 64;

/// What `gbuffers_hand` reads: the entity inputs, and the hand's projection from set 1.
pub fn inputs() -> horizon_shaders::GbuffersInputs {
    let mut inputs = super::pack_entities::inputs();
    inputs.declarations.push_str(
        "layout(set = 1, binding = 1) uniform HzHand { mat4 hz_hand_projection; mat4 hz_hand_projection_inverse; };\n",
    );
    inputs.body.push_str(
        "    hz_ProjectionMatrix = hz_hand_projection;\n    \
         hz_ProjectionMatrixInverse = hz_hand_projection_inverse;\n    \
         hz_ModelViewProjectionMatrix = hz_hand_projection * hz_ModelViewMatrix;\n",
    );
    inputs
}

/// The hand's projection in GL's conventions: `hud_fov` degrees, depth scaled by
/// `MC_HAND_DEPTH` (the view bob is in `gbufferModelView`).
pub fn projection(hud_fov: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::from_scale(glam::Vec3::new(1.0, 1.0, HAND_DEPTH))
        * Mat4::perspective_rh_gl(hud_fov.to_radians(), aspect, 0.05, far)
}

/// The hand's vertices (also read at set 1 for normals and tangents), its uniform and quad
/// indices.
pub struct PackHand {
    pub layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    uniform: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub indices: wgpu::Buffer,
    capacity: u32,
    /// Quads uploaded this frame.
    pub quads: u32,
}

impl PackHand {
    pub fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack hand"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(1, wgpu::BufferBindingType::Uniform),
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack hand"),
            size: UNIFORM,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let capacity = 256;
        let (vertices, group) = Self::vertex_buffer(device, &layout, &uniform, capacity);
        Self {
            indices: super::pack_entities::quad_indices(device, capacity),
            layout,
            vertices,
            uniform,
            group,
            capacity,
            quads: 0,
        }
    }

    fn vertex_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        quads: u32,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack hand vertices"),
            size: u64::from(quads) * 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack hand"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        (buffer, group)
    }

    /// Uploads this frame's hand and its projection.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hand: &[Vertex],
        projection: Mat4,
    ) {
        self.quads = (hand.len() / 4) as u32;
        if self.quads == 0 {
            return;
        }
        if self.quads > self.capacity {
            self.capacity = self.quads.next_power_of_two();
            (self.vertices, self.group) =
                Self::vertex_buffer(device, &self.layout, &self.uniform, self.capacity);
            self.indices = super::pack_entities::quad_indices(device, self.capacity);
        }
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(hand));
        let matrices = [projection, projection.inverse()].map(|m| m.to_cols_array());
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&matrices));
    }

    pub fn vertices(&self) -> wgpu::BufferSlice<'_> {
        self.vertices.slice(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand's window depth lies within `MC_HAND_DEPTH` of the middle (packs tell the hand
    /// apart by it), and x, y match the plain projection.
    #[test]
    fn hand_depth_is_scaled_around_the_middle() {
        let p = projection(70.0, 16.0 / 9.0, 512.0);
        let plain = Mat4::perspective_rh_gl(70f32.to_radians(), 16.0 / 9.0, 0.05, 512.0);
        for z in [-0.06f32, -0.5, -3.0, -400.0] {
            let v = glam::Vec4::new(0.1, -0.2, z, 1.0);
            let (a, b) = (p * v, plain * v);
            let window = (a.z / a.w) * 0.5 + 0.5;
            assert!(
                (window - 0.5).abs() <= HAND_DEPTH * 0.5 + 1e-6,
                "{z}: {window}"
            );
            assert!((a.x / a.w - b.x / b.w).abs() < 1e-6);
        }
    }
}
