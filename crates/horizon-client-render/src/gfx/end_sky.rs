//! The End sky's six tiled faces, retained for the lifetime of the renderer's atlas.
use crate::mesher::Vertex;
use wgpu::util::DeviceExt;

const VERTICES: u32 = 6 * 16 * 16 * 4;

#[derive(Default)]
pub(super) struct EndSky {
    mesh: Option<([f32; 4], wgpu::Buffer)>,
}

impl EndSky {
    /// Called only when the End sky is enabled. Rebuild on atlas UV changes;
    /// resource reload and device recreation also replace the owning renderer.
    pub(super) fn prepare(&mut self, device: &wgpu::Device, uv: Option<[f32; 4]>) {
        let Some(uv) = uv else {
            self.mesh = None;
            return;
        };
        if self.mesh.as_ref().is_some_and(|(old, _)| *old == uv) {
            return;
        }
        let vertices = vertices(uv);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("End sky vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.mesh = Some((uv, buffer));
    }

    /// The celestial pipeline, atlas bind group and quad indices are already bound.
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((_, buffer)) = &self.mesh {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw_indexed(0..VERTICES / 4 * 6, 0, 0..1);
        }
    }
}

/// SkyRenderer.buildEndSky: six faces of end_sky.png tiled 16 times,
/// tinted 0x282828. The atlas does not wrap, so each face is 16×16 quads.
fn vertices(uv: [f32; 4]) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(VERTICES as usize);
    use std::f32::consts::FRAC_PI_2;
    let faces = [
        glam::Mat4::IDENTITY,
        glam::Mat4::from_rotation_x(FRAC_PI_2),
        glam::Mat4::from_rotation_x(-FRAC_PI_2),
        glam::Mat4::from_rotation_x(std::f32::consts::PI),
        glam::Mat4::from_rotation_z(FRAC_PI_2),
        glam::Mat4::from_rotation_z(-FRAC_PI_2),
    ];
    let step = 200.0 / 16.0;
    for pose in faces {
        for a in 0..16 {
            for b in 0..16 {
                let (x0, z0) = (-100.0 + step * a as f32, -100.0 + step * b as f32);
                let (x1, z1) = (x0 + step, z0 + step);
                let corners = [
                    (x0, z0, [uv[0], uv[1]]),
                    (x0, z1, [uv[0], uv[3]]),
                    (x1, z1, [uv[2], uv[3]]),
                    (x1, z0, [uv[2], uv[1]]),
                ];
                for (x, z, uv) in corners {
                    let p = pose.transform_point3(glam::Vec3::new(x, -100.0, z));
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.to_array(),
                        uv,
                        layer: 2,
                        color: 0xFF28_2828,
                        light: 0,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
