use super::*;
use crate::gfx::{Globals, Renderer};
use bytemuck::Zeroable;
use glam::Mat4;

#[test]
fn geometry_matches_pre_residency_vertex_snapshots() {
    use sha2::{Digest, Sha256};
    // Captured from game/end.rs at 34c3c1cf for full, tiled and non-binary UVs.
    // Quantize positions only, avoiding platform sin/cos rounding differences;
    // quad order, UVs, tint, material and light retain their exact byte values.
    for (uv, expected) in [
        (
            [0.0, 0.0, 1.0, 1.0],
            "82a9be3f3ed1f733017df2c81dc6dff163f67bf739c768e3fd0f90f00c7802f6",
        ),
        (
            [0.125, 0.25, 0.375, 0.5],
            "05a058bd8e70cff900c8e1c81061892ed27866d86498ebe1e8ae5af4c3b770ef",
        ),
        (
            [0.13, 0.27, 0.41, 0.59],
            "93064b9fd2e745da46b927b2625edc8a381bb8c6e20b0f0880e816c4c1745ec1",
        ),
    ] {
        let mut mesh = vertices(uv);
        assert_eq!(mesh.len(), VERTICES as usize);
        for vertex in &mut mesh {
            vertex.pos = vertex.pos.map(|x| {
                let n = (x * 1024.0).round() / 1024.0;
                if n == 0.0 { 0.0 } else { n }
            });
        }
        let hash = format!("{:x}", Sha256::digest(bytemuck::cast_slice(&mesh)));
        assert_eq!(hash, expected);
    }
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    eprintln!("End sky test adapter: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&Default::default())).ok()
}

#[test]
fn resident_buffer_survives_frames_and_refreshes_with_the_atlas() {
    let Some((device, _)) = device() else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    let mut sky = EndSky::default();
    let uv = [0.125, 0.25, 0.375, 0.5];
    sky.prepare(&device, Some(uv));
    let first = sky.mesh.as_ref().unwrap().1.clone();
    assert_eq!(
        first.size(),
        u64::from(VERTICES) * size_of::<Vertex>() as u64
    );
    // Immutable: steady-state frames cannot upload into this buffer.
    assert_eq!(first.usage(), wgpu::BufferUsages::VERTEX);
    for _ in 0..600 {
        sky.prepare(&device, Some(uv));
        assert_eq!(sky.mesh.as_ref().unwrap().1, first);
    }
    let moved = [0.5, 0.5, 0.75, 0.75];
    sky.prepare(&device, Some(moved));
    assert_ne!(sky.mesh.as_ref().unwrap().1, first);
    assert_eq!(sky.mesh.as_ref().unwrap().0, moved);
    sky.prepare(&device, None);
    assert!(sky.mesh.is_none());
    sky.prepare(&device, Some(uv));
    assert_ne!(sky.mesh.as_ref().unwrap().1, first);
    // A resource reload/device recreation starts a new renderer-owned cache,
    // even if the replacement atlas happens to use exactly the same UVs.
    let mut replacement = EndSky::default();
    replacement.prepare(&device, Some(uv));
    assert_ne!(
        replacement.mesh.as_ref().unwrap().1,
        sky.mesh.as_ref().unwrap().1
    );
}

/// The actual entity shader and celestial blend, comparing the old combined
/// vertex stream with a resident sky draw followed by a separate flash draw.
#[test]
fn separate_draws_match_combined_celestial_pixels() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("End sky differential shader"),
        source: wgpu::ShaderSource::Wgsl(crate::gfx::entity_shader().into()),
    });
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("End sky differential pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_entity"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_celestial"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
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
    let texture = |pixels: &[u8], width, height| {
        device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            pixels,
        )
    };
    // Asymmetric, translucent texels expose UV/winding and blend-order errors.
    let pixels: Vec<u8> = (0..16 * 16)
        .flat_map(|i| {
            [
                (i * 17) as u8,
                (i * 31) as u8,
                (i * 7) as u8,
                (80 + i % 176) as u8,
            ]
        })
        .collect();
    let atlas = texture(&pixels, 16, 16).create_view(&Default::default());
    let lightmap = texture(&[255; 4], 1, 1).create_view(&Default::default());
    let sampler = device.create_sampler(&Default::default());
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: size_of::<Globals>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&atlas),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&lightmap),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let indices = Renderer::quad_indices(&device, VERTICES / 4 + 1);
    let mut resident = EndSky::default();
    let uv = [0.125, 0.25, 0.375, 0.5];
    resident.prepare(&device, Some(uv));
    let first = resident.mesh.as_ref().unwrap().1.clone();
    let render = |sky: &EndSky, flash: &[Vertex], combined: bool| {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut vertices = if combined { vertices(uv) } else { Vec::new() };
        vertices.extend_from_slice(flash);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            if !combined {
                sky.draw(&mut pass);
            }
            if !vertices.is_empty() {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..vertices.len() as u32 / 4 * 6, 0, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        readback.slice(..).get_mapped_range().unwrap().to_vec()
    };
    let mut cases = 0;
    let mut visible_flash_cases = 0;
    for yaw in [0.0_f32, 45.0, 90.0, 180.0, 270.0] {
        for pitch in [-85.0_f32, -30.0, 0.0, 45.0, 85.0] {
            let mut globals = Globals::zeroed();
            globals.view_proj = (Mat4::perspective_rh(90_f32.to_radians(), 1.0, 0.05, 1024.0)
                * Mat4::from_rotation_x(pitch.to_radians())
                * Mat4::from_rotation_y(yaw.to_radians()))
            .to_cols_array_2d();
            queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&globals));
            let no_flash = render(&resident, &[], true);
            for level in [0_u32, 1, 127, 255] {
                let flash: Vec<_> = if level == 0 {
                    Vec::new()
                } else {
                    // Same flash size/height, with an arbitrary EndFlashState pose.
                    let pose = Mat4::from_rotation_y(1.3) * Mat4::from_rotation_x(-1.6);
                    [(-60.0, -60.0), (60.0, -60.0), (60.0, 60.0), (-60.0, 60.0)]
                        .map(|(x, z)| Vertex {
                            pos: pose.transform_point3(glam::vec3(x, 100.0, z)).to_array(),
                            uv: [0.75 + x / 480.0, 0.75 + z / 480.0],
                            layer: 2,
                            color: level * 0x01010101,
                            light: 0,
                            normal: [0; 4],

                            glint_uv: [0.0, 0.0],
                        })
                        .to_vec()
                };
                resident.prepare(&device, Some(uv));
                let expected = render(&resident, &flash, true);
                let actual = render(&resident, &flash, false);
                visible_flash_cases += usize::from(expected != no_flash);
                assert_eq!(actual, expected, "yaw={yaw} pitch={pitch} flash={level}");
                assert!(
                    actual.as_chunks::<4>().0.iter().any(|p| p[..3] != [0; 3]),
                    "empty frame"
                );
                assert_eq!(resident.mesh.as_ref().unwrap().1, first);
                cases += 1;
            }
        }
    }
    assert!(
        visible_flash_cases > 0,
        "the flash must affect the comparison"
    );
    eprintln!("{cases} combined/resident framebuffer cases matched exactly");
}
