//! The night sky's stars, built once into a static vertex buffer as
//! `SkyRenderer.buildStars` does, and drawn each frame with one small uniform
//! (the sky's rotation and the star brightness) instead of being regenerated
//! and uploaded every frame.
use glam::{Mat4, Vec3};
use horizon_core::random::RandomSource;
use wgpu::util::DeviceExt;

/// `SkyRenderer.buildStars` with its fixed seed: up to 1500 quads on a
/// sphere of radius 100, each turned to face the centre and spun about its
/// own axis, as triangles (two per quad).
pub fn star_triangles() -> Vec<[f32; 3]> {
    let mut random = horizon_core::random::LegacyRandom::new(10842);
    let mut out = Vec::with_capacity(1500 * 6);
    for _ in 0..1500 {
        let x = random.next_float() * 2.0 - 1.0;
        let y = random.next_float() * 2.0 - 1.0;
        let z = random.next_float() * 2.0 - 1.0;
        let size = 0.15 + random.next_float() * 0.1;
        let l = x * x + y * y + z * z;
        if l <= 0.010_000_001 || l >= 1.0 {
            continue;
        }
        let center = Vec3::new(x, y, z).normalize() * 100.0;
        let z_rot = (random.next_double() * std::f64::consts::TAU) as f32;
        // Matrix3f.rotateTowards(-center, up).rotateZ(-zRot).
        let forward = -center.normalize();
        let right = Vec3::Y.cross(forward).normalize_or(Vec3::X);
        let up = forward.cross(right);
        let (s, c) = (-z_rot).sin_cos();
        let (r2, u2) = (right * c + up * s, up * c - right * s);
        let corner = |a: f32, b: f32| (center + r2 * a + u2 * b).to_array();
        let quad = [
            corner(size, -size),
            corner(size, size),
            corner(-size, size),
            corner(-size, -size),
        ];
        out.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    clip_from_sky: [[f32; 4]; 4],
    color: [f32; 4],
}

pub(super) struct Stars {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    count: u32,
}

impl Stars {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let triangles = star_triangles();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Stars vertex buffer"),
            contents: bytemuck::cast_slice(&triangles),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Stars rotation and brightness"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Stars bindings"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Stars resources"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Stars shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("stars.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Stars layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Stars"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_stars"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_stars"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // RenderPipelines.STARS: the celestial blend (src alpha, one).
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            group,
            uniform,
            vertices,
            count: triangles.len() as u32,
        }
    }

    /// This frame's sky rotation (camera-relative world from star space,
    /// times the view projection) and star brightness.
    pub(super) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        clip_from_sky: Mat4,
        brightness: f32,
    ) {
        let b = brightness.clamp(0.0, 1.0);
        let uniform = Uniform {
            clip_from_sky: clip_from_sky.to_cols_array_2d(),
            color: [b; 4],
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_lie_on_the_sky_sphere_as_whole_quads() {
        let triangles = star_triangles();
        // About 52% of the cube's random points fall inside the unit ball.
        assert!((700 * 6..1500 * 6).contains(&triangles.len()));
        assert_eq!(triangles.len() % 6, 0);
        for p in &triangles {
            let r = Vec3::from_array(*p).length();
            assert!((99.9..100.1).contains(&r), "{r}");
        }
        assert_eq!(triangles, star_triangles());
    }
}
