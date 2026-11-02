use super::*;
use std::cell::Cell;

// Frozen pre-cache generator: compare every Vertex byte, including primitive
// order and the original camera-relative rounding, without game resources.
fn reference(seed: i64, at: [f64; 3], camera: DVec3, uv: [f32; 2]) -> Vec<Vertex> {
    let mut out = Vec::new();
    // ARGB 0.45, 0.45, 0.5 at 0.3 alpha, packed RGBA.
    let color = (76u32 << 24) | (127 << 16) | (114 << 8) | 114;
    let b = Bolt { seed, at };
    {
        let o = (glam::DVec3::from_array(b.at) - camera).as_vec3();
        let mut random = LegacyRandom::new(b.seed);
        let (mut xs, mut zs) = ([0f32; 8], [0f32; 8]);
        let (mut x_off, mut z_off) = (0f32, 0f32);
        for i in (0..8).rev() {
            xs[i] = x_off;
            zs[i] = z_off;
            x_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
            z_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
        }
        for layer in 0..4 {
            let mut random = LegacyRandom::new(b.seed);
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
                                pos: (o + glam::Vec3::from_array(c)).to_array(),
                                uv,
                                // The HDR tier lightning.rs adds; the geometry is vanilla's.
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
    }
    out
}

fn seeds() -> Vec<i64> {
    let mut seeds = vec![0, 1, -1, i64::MIN, i64::MAX, 0x123456789abcdef];
    let mut random = LegacyRandom::new(42);
    seeds.extend((0..256).map(|_| random.next_long()));
    seeds
}

/// Pinned 26.3 LightningBoltRenderer.lambda$submit$0, captured with an
/// identity PoseStack and a VertexConsumer recording big-endian xyz floats.
#[test]
fn geometry_matches_pinned_vanilla_vertex_digest() {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for seed in seeds() {
        for vertex in geometry(seed, [0.0; 2]) {
            for value in vertex.pos {
                digest.update(value.to_be_bytes());
            }
        }
    }
    assert_eq!(
        format!("{:x}", digest.finalize()),
        "a9ed25caa50b9aa340faa4bb417b3d533b061f0dce66514e4bb212931444dee6"
    );
}

#[test]
fn retained_geometry_matches_original_generator() {
    let uv = [0.8125, 0.03125];
    for seed in seeds() {
        let local = geometry(seed, uv);
        assert_eq!(local.len(), QUADS as usize * 4);
        for (at, camera) in [
            ([0.0; 3], DVec3::ZERO),
            ([12.5, 64.0, -12.5], DVec3::new(-4.25, 71.125, 32.5)),
            (
                [29_999_999.5, -64.0, -29_999_999.5],
                DVec3::new(29_999_987.125, 67.75, -29_999_983.875),
            ),
        ] {
            let offset = offset(&Bolt { seed, at }, camera);
            let translated: Vec<_> = local
                .iter()
                .map(|v| Vertex {
                    pos: (glam::Vec3::from_array(offset) + glam::Vec3::from_array(v.pos))
                        .to_array(),
                    ..*v
                })
                .collect();
            let expected = reference(seed, at, camera, uv);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&translated),
                bytemuck::cast_slice::<_, u8>(&expected),
                "seed {seed}"
            );
        }
    }
}

#[test]
fn cache_tracks_seed_lifecycle_movement_and_atlas_reload() {
    let mut cache = Cache::new();
    let builds = Cell::new(0);
    let build = |seed, uv| {
        builds.set(builds.get() + 1);
        geometry(seed, uv)
    };
    let mut bolts = [
        Bolt {
            seed: 123,
            at: [0.0; 3],
        },
        Bolt {
            seed: 456,
            at: [1.0; 3],
        },
    ];
    fn input(bolts: &[Bolt]) -> Input<'_> {
        Input {
            bolts,
            uv: [0.25, 0.5],
        }
    }
    cache.prepare(&input(&bolts), build);
    assert_eq!(builds.get(), 2);
    assert_eq!(cache.meshes.len(), 2);
    // Any number of rendered frames or multiplayer position updates reuse meshes.
    for i in 0..100 {
        bolts[0].at = [i as f64; 3];
        cache.prepare(&input(&bolts), build);
    }
    assert_eq!(builds.get(), 2);
    // A flash changes the seed; retire its previous mesh immediately.
    bolts[0].seed = 789;
    cache.prepare(&input(&bolts), build);
    assert_eq!(builds.get(), 3);
    assert!(!cache.meshes.contains_key(&123));
    // Removal retains other bolts, and repeated seeds still share geometry.
    bolts[0].seed = 456;
    cache.prepare(&input(&bolts), build);
    assert_eq!(builds.get(), 3);
    assert_eq!(cache.meshes.len(), 1);
    // Atlas replacement must update white texel UVs without changing the seed.
    cache.prepare(
        &Input {
            bolts: &bolts,
            uv: [0.75, 0.25],
        },
        build,
    );
    assert_eq!(builds.get(), 4);
    assert_eq!(cache.meshes[&456].mesh[0].uv, [0.75, 0.25]);
    // Disconnect/dimension switch/expiry drops every retained resource.
    cache.prepare(&Input::default(), build);
    assert!(cache.meshes.is_empty());
    assert_eq!(cache.meshes.capacity(), 0);
    cache.prepare(&input(&bolts), build);
    assert_eq!(builds.get(), 5);
}

#[test]
fn long_flash_sequence_keeps_only_active_seeds() {
    let mut cache = Cache::new();
    for seed in 0..10_000 {
        cache.prepare(
            &Input {
                bolts: &[Bolt { seed, at: [0.0; 3] }],
                uv: [0.0; 2],
            },
            |seed, _| seed,
        );
        assert_eq!(cache.meshes.len(), 1);
        assert_eq!(cache.meshes[&seed].mesh, seed);
        assert!(cache.meshes.capacity() < 16);
    }
}

/// Exercise the actual entity shader and retained draw path offscreen. Run
/// explicitly, separately from other headless device tests (some GPU drivers
/// fail during concurrent device teardown). Machines without an adapter skip it.
#[test]
#[ignore = "requires GPU; run separately with --ignored --exact"]
fn retained_gpu_draw_matches_transient_additive_draw() {
    use bytemuck::Zeroable;
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    eprintln!("lightning differential GPU: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Lightning differential shader"),
        source: wgpu::ShaderSource::Wgsl(crate::gfx::entity_shader().into()),
    });
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
    // Attribute 7: 6 is the entity vertex buffer's glint UV.
    let offsets = wgpu::vertex_attr_array![7=>Float32x3];
    let vertices = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    let retained_vertices = [
        vertices[0].clone(),
        Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &offsets,
        }),
    ];
    let pipelines: Vec<_> = [false, true]
        .into_iter()
        .map(|retained| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Lightning differential pipeline"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if retained {
                        "vs_lightning"
                    } else {
                        "vs_entity"
                    }),
                    compilation_options: Default::default(),
                    buffers: if retained {
                        &retained_vertices
                    } else {
                        &vertices
                    },
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
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
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::SrcAlpha,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
        .collect();
    let globals = crate::gfx::Globals {
        view_proj: (glam::Mat4::orthographic_rh(-100.0, 100.0, -10.0, 150.0, 0.1, 400.0)
            * glam::Mat4::look_at_rh(
                glam::Vec3::new(80.0, 30.0, 160.0),
                glam::Vec3::new(0.0, 60.0, 0.0),
                glam::Vec3::Y,
            ))
        .to_cols_array_2d(),
        ..crate::gfx::Globals::zeroed()
    };
    let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let size = wgpu::Extent3d {
        width: 256,
        height: 256,
        depth_or_array_layers: 1,
    };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let white = device
        .create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
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
            &[255; 4],
        )
        .create_view(&Default::default());
    let sampler = device.create_sampler(&Default::default());
    let groups: Vec<_> = pipelines
        .iter()
        .map(|pipeline| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&white),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&white),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        })
        .collect();
    let mut renderer = Renderer::new();
    let mut uploads = Uploads::new(&device);
    let mut bolts = [Bolt {
        seed: 123,
        at: [29_999_999.5, 64.0, -29_999_999.5],
    }; 3];
    bolts[1].seed = -456;
    bolts[1].at[0] -= 32.0;
    bolts[2].at[2] += 20.0;
    let indices = crate::gfx::Renderer::quad_indices(&device, QUADS * 3);
    for frame in 0..4 {
        let camera = DVec3::new(29_999_997.125 + frame as f64, 62.75, -29_999_998.875);
        if frame == 2 {
            bolts[0].seed = 789;
        }
        let input = Input {
            bolts: &bolts[..if frame == 3 { 1 } else { 3 }],
            uv: if frame == 3 { [0.75; 2] } else { [0.5; 2] },
        };
        let expected: Vec<_> = input
            .bolts
            .iter()
            .flat_map(|bolt| reference(bolt.seed, bolt.at, camera, input.uv))
            .collect();
        let transient = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&expected),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.prepare(&device, &mut uploads, &mut encoder, &input, camera);
        let mut readbacks = Vec::new();
        for mode in 0..2 {
            let color = texture(
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let view = color.create_view(&Default::default());
            let depth = texture(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&pipelines[mode]);
                pass.set_bind_group(0, &groups[mode], &[]);
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                if mode == 0 {
                    pass.set_vertex_buffer(0, transient.slice(..));
                    pass.draw_indexed(0..expected.len() as u32 / 4 * 6, 0, 0..1);
                } else {
                    renderer.draw(&mut pass, &input);
                }
            }
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256 * 256 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(1024),
                        rows_per_image: None,
                    },
                },
                size,
            );
            readbacks.push(read);
        }
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        for read in &readbacks {
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels: Vec<Vec<u8>> = readbacks
            .iter()
            .map(|read| read.slice(..).get_mapped_range().unwrap().to_vec())
            .collect();
        assert!(pixels[0].iter().any(|&b| b != 0), "empty reference frame");
        assert_eq!(pixels[0], pixels[1], "frame {frame}: GPU output differs");
    }
}

/// `horizon/white` (4x4 texels) at (40, 12) of a 64x32 atlas.
const ATLAS: (u32, u32) = (64, 32);
fn white_sprite() -> crate::hud::Rect {
    let (w, h) = (ATLAS.0 as f32, ATLAS.1 as f32);
    crate::hud::Rect {
        u0: 40.0 / w,
        v0: 12.0 / h,
        u1: 44.0 / w,
        v1: 16.0 / h,
        w: 4,
        h: 4,
    }
}

/// The white sprite's corner is on its border with the next atlas texels,
/// which nearest sampling picks for most of a bolt's fragments (#3171): the
/// point sampled is at least a texel inside the sprite.
#[test]
fn white_uv_is_inside_the_white_sprite() {
    let [u, v] = white_uv(&white_sprite());
    let (x, y) = (u * ATLAS.0 as f32, v * ATLAS.1 as f32);
    assert!((41.0..=43.0).contains(&x), "u at texel {x}");
    assert!((13.0..=15.0).contains(&y), "v at texel {y}");
}

/// A bolt seen from 14 blocks through the atlas as the game binds it (nearest
/// sampling, only the white sprite opaque) draws the same pixels as through a
/// texture that is white everywhere (#3171: the sprite's corner left a few
/// scattered pixels). Run explicitly, like the test above.
#[test]
#[ignore = "requires GPU; run separately with --ignored --exact"]
fn bolt_through_the_atlas_white_sprite_is_fully_drawn() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(crate::gfx::entity_shader().into()),
    });
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
    let offsets = wgpu::vertex_attr_array![7=>Float32x3];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &offsets,
        }),
    ];
    // The lightning pipeline (gfx.rs): additive, into RGBA8.
    let additive = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_lightning"),
            compilation_options: Default::default(),
            buffers: &buffers,
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
                    color: additive,
                    alpha: additive,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // The game's camera: 70 degrees, reversed depth, the bolt 14 blocks ahead.
    let globals = crate::gfx::Globals {
        view_proj: (glam::Mat4::perspective_rh(70f32.to_radians(), 1.0, 400.0, 0.05)
            * glam::Mat4::look_at_rh(glam::Vec3::ZERO, glam::Vec3::Z, glam::Vec3::Y))
        .to_cols_array_2d(),
        ..bytemuck::Zeroable::zeroed()
    };
    let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let texture = |(width, height): (u32, u32), data: &[u8]| {
        device
            .create_texture_with_data(
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
                data,
            )
            .create_view(&Default::default())
    };
    let white = texture((1, 1), &[255; 4]);
    // Transparent but for the white sprite.
    let mut texels = vec![0u8; (ATLAS.0 * ATLAS.1 * 4) as usize];
    for y in 12..16 {
        for x in 40..44 {
            let i = ((y * ATLAS.0 + x) * 4) as usize;
            texels[i..i + 4].copy_from_slice(&[255; 4]);
        }
    }
    let atlas = texture(ATLAS, &texels);
    // The atlas sampler (gfx.rs): wgpu's default, nearest.
    let sampler = device.create_sampler(&Default::default());
    let indices = crate::gfx::Renderer::quad_indices(&device, QUADS);
    let size = 256;
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    };
    let draw = |view: &wgpu::TextureView, uv: [f32; 2]| -> Vec<u8> {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&white),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bolts = [Bolt {
            seed: 123,
            at: [0.0, -1.62, 14.0],
        }];
        let input = Input { bolts: &bolts, uv };
        let mut renderer = Renderer::new();
        let mut uploads = Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.prepare(&device, &mut uploads, &mut encoder, &input, DVec3::ZERO);
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        {
            let target = color.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            renderer.draw(&mut pass, &input);
        }
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(size * size * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: None,
                },
            },
            extent,
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        read.slice(..).get_mapped_range().unwrap().to_vec()
    };
    let lit = |pixels: &[u8]| pixels.chunks(4).filter(|p| p[3] != 0).count();
    let expected = draw(&white, [0.5; 2]);
    let drawn = draw(&atlas, white_uv(&white_sprite()));
    assert!(lit(&expected) > 1000, "{} pixels", lit(&expected));
    assert_eq!(
        lit(&drawn),
        lit(&expected),
        "pixels drawn through the atlas"
    );
    assert!(drawn == expected, "the bolt's colours through the atlas");
}
