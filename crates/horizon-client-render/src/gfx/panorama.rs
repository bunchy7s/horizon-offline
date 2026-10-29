//! Vanilla 26.3's CubeMapTexture and CubeMap, separate from the pixel-filtered GUI atlas.
use crate::{Result, hud::decode};
use glam::Mat4;
use std::path::Path;
use wgpu::util::DeviceExt;

/// Native cube order: +X, -X, +Y, -Y, +Z, -Z (CubeMapTexture.SUFFIXES).
const FACE_ORDER: [u8; 6] = [1, 3, 5, 4, 0, 2];

#[derive(Debug)]
pub struct Faces {
    pub size: u32,
    pub pixels: Vec<u8>,
}

impl Faces {
    pub fn load(root: &Path) -> Self {
        Self::try_load(root).unwrap_or_else(|error| {
            log::warn!("Horizon: title panorama unavailable ({error})");
            Self {
                size: 1,
                pixels: [96, 110, 111, 255].repeat(6),
            }
        })
    }

    fn try_load(root: &Path) -> Result<Self> {
        let background = root.join("assets/minecraft/textures/gui/title/background");
        let mut size = 0;
        let mut pixels = Vec::new();
        for face in FACE_ORDER {
            let path = background.join(format!("panorama_{face}.png"));
            let (w, h, data) = decode(&path)?;
            if w != h || (size != 0 && w != size) {
                return Err(format!(
                    "{}: panorama faces must be equally sized squares",
                    path.display()
                )
                .into());
            }
            size = w;
            // CubeMapTexture copies each original image with flipY=true. Keep every texel.
            for row in data.chunks_exact((w * 4) as usize).rev() {
                pixels.extend_from_slice(row);
            }
        }
        Ok(Self { size, pixels })
    }
}

fn projection(width: u32, height: u32, yaw: f32, pitch: f32) -> Mat4 {
    // CubeMap.render: 85-degree FOV, near/far 0.05/10, rotationX(PI), rotateX, rotateY.
    Mat4::perspective_rh(
        85.0f32.to_radians(),
        width as f32 / height.max(1) as f32,
        0.05,
        10.0,
    ) * Mat4::from_rotation_x(std::f32::consts::PI)
        * Mat4::from_rotation_x(pitch.to_radians())
        * Mat4::from_rotation_y(yaw.to_radians())
}

pub(super) struct Panorama {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl Panorama {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        faces: &Faces,
    ) -> Self {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("Title panorama cube"),
                size: wgpu::Extent3d {
                    width: faces.size,
                    height: faces.size,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &faces.pixels,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        // CubeMapTexture's blur=true metadata: linear minification and magnification, no mips.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Title panorama linear filtering"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Title panorama projection"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Title panorama bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Title panorama resources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Title panorama shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("panorama.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Title panorama layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Title panorama"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_panorama"),
                compilation_options: Default::default(),
                buffers: &[],
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
                entry_point: Some("fs_panorama"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
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
        }
    }

    pub(super) fn prepare(
        &self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        yaw: f32,
        pitch: f32,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(&projection(width, height, yaw, pitch).to_cols_array()),
        );
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..36, 0..1);
    }
}

/// `PanoramaRenderer`: accumulate rotation so changing speed does not jump
/// back to a different point on the cube. `now` is monotonic seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rotation {
    last: f32,
    yaw: f32,
}
impl Rotation {
    pub fn update(&mut self, now: f32, speed: f32) -> f32 {
        self.yaw = (self.yaw - (now - self.last).max(0.0) * 2.0 * speed) % 360.0;
        self.last = now;
        self.yaw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU32, Ordering},
    };

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(size: u32) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "horizon-panorama-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            let fixture = Self(root);
            fs::create_dir_all(fixture.background()).unwrap();
            for face in 0..6 {
                fixture.write(face, size, size);
            }
            fixture
        }
        fn background(&self) -> std::path::PathBuf {
            self.0
                .join("assets/minecraft/textures/gui/title/background")
        }
        fn write(&self, face: u8, w: u32, h: u32) {
            let file =
                fs::File::create(self.background().join(format!("panorama_{face}.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..h)
                .flat_map(|y| {
                    (0..w).flat_map(move |x| [face, (x % 251) as u8, (y % 253) as u8, 255])
                })
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rotation_speed_changes_preserve_position() {
        let mut rotation = Rotation::default();
        assert_eq!(rotation.update(5.0, 1.0), -10.0);
        assert_eq!(rotation.update(10.0, 0.0), -10.0);
        assert_eq!(rotation.update(12.0, 0.5), -12.0);
        assert_eq!(rotation.update(13.0, 1.0), -14.0);
    }

    #[test]
    fn original_resolution_texels_and_vanilla_cube_orientation_are_preserved() {
        let f = Fixture::new(1024);
        let faces = Faces::try_load(&f.0).unwrap();
        assert_eq!(faces.size, 1024);
        assert_eq!(faces.pixels.len(), 6 * 1024 * 1024 * 4);
        // CubeMapTexture's actual layer suffixes, with each source row flipped vertically.
        for (layer, source_face) in [1, 3, 5, 4, 0, 2].into_iter().enumerate() {
            for y in 0..1024 {
                for x in 0..1024 {
                    let at = ((layer * 1024 + y) * 1024 + x) * 4;
                    assert_eq!(
                        faces.pixels[at..at + 4],
                        [source_face, (x % 251) as u8, ((1023 - y) % 253) as u8, 255]
                    );
                }
            }
        }
    }

    #[test]
    fn placeholders_load_and_incomplete_or_mismatched_cubes_fall_back() {
        let f = Fixture::new(1);
        assert_eq!(Faces::try_load(&f.0).unwrap().size, 1);
        f.write(5, 2, 1);
        assert!(
            Faces::try_load(&f.0)
                .unwrap_err()
                .to_string()
                .contains("equally sized squares")
        );
        let fallback = Faces::load(&f.0);
        assert_eq!(fallback.pixels.len(), 24);
        f.write(5, 2, 2);
        assert!(Faces::try_load(&f.0).is_err());
        fs::remove_file(f.background().join("panorama_5.png")).unwrap();
        assert!(Faces::try_load(&f.0).is_err());
        assert_eq!(Faces::load(&f.0).size, 1);
    }

    #[test]
    fn cube_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("panorama.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn vanilla_camera_centres_the_positive_z_face_and_pitches_ten_degrees() {
        let straight = projection(1920, 1080, 0.0, 0.0) * glam::Vec4::new(0.0, 0.0, 1.0, 1.0);
        assert!((straight.x / straight.w).abs() < 1e-6);
        assert!((straight.y / straight.w).abs() < 1e-6);
        assert!(straight.w > 0.0);
        let pitched = projection(1920, 1080, 0.0, 10.0) * glam::Vec4::new(0.0, 0.0, 1.0, 1.0);
        let expected_y = 10.0f32.to_radians().tan() / 42.5f32.to_radians().tan();
        assert!((pitched.y / pitched.w - expected_y).abs() < 1e-6);
    }
}
