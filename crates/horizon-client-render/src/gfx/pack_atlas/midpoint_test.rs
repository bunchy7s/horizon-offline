//! Original GPU probes of the production terrain inputs, without a resource or shader pack.

use super::PackAtlas;
use crate::mesher::{TerrainVertex, UV_STEPS};
use wgpu::util::DeviceExt;

#[test]
fn mid_tex_coord_follows_each_quads_mapped_corners() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    eprintln!("midpoint probe adapter: {:?}", adapter.get_info());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 5,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 5, 5, 16).unwrap();
    queue.submit([encoder.finish()]);
    // Each row uses a different atlas cell. Expected atlas coordinates are independent
    // numbers, including the existing clamp of a model UV beyond the sprite's edge.
    let cases = [
        (
            "whole",
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            [1. / 6., 0.25],
            [1. / 6., 0.25],
            [[0., 0.], [1. / 3., 0.], [1. / 3., 0.5], [0., 0.5]],
        ),
        (
            "half-height",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 0.5]],
            [0.5, 0.125],
            [0.5, 0.25],
            [
                [1. / 3., 0.],
                [2. / 3., 0.],
                [2. / 3., 0.25],
                [1. / 3., 0.25],
            ],
        ),
        (
            "rotated partial",
            [[0.25, 0.75], [0.25, 0.25], [0.5, 0.25], [0.5, 0.75]],
            [19. / 24., 0.25],
            [5. / 6., 0.25],
            [
                [0.75, 0.375],
                [0.75, 0.125],
                [5. / 6., 0.125],
                [5. / 6., 0.375],
            ],
        ),
        (
            "clamped",
            [[0., 0.], [1.5, 0.], [1.5, 0.5], [0., 0.5]],
            [1. / 6., 0.625],
            [1. / 6., 0.75],
            [[0., 0.5], [1. / 3., 0.5], [1. / 3., 0.75], [0., 0.75]],
        ),
        (
            "fourth corner",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 1.]],
            [0.5, 0.6875],
            [0.5, 0.75],
            [
                [1. / 3., 0.5],
                [2. / 3., 0.5],
                [2. / 3., 0.75],
                [1. / 3., 1.],
            ],
        ),
    ];
    let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let vertices: Vec<_> = cases
        .iter()
        .enumerate()
        .flat_map(|(layer, (_, uv, ..))| {
            (0..4).map(move |k| {
                TerrainVertex::new(positions[k], uv[k], layer as u16 | 0x8000, u32::MAX, 0)
            })
        })
        .collect();
    let mut failures = Vec::new();
    // Positive origins encode fade bits beside the power-of-two scale. Negative origins
    // use the older fractional scale encoding. Both must match the mapped vertex UVs.
    for origin in [
        f32::from_bits((1. / UV_STEPS).to_bits() | 0x12345),
        -3. - 1. / UV_STEPS,
    ] {
        for flat in [true, false] {
            for shadow in [false, true] {
                let pixels = sample(&device, &queue, &atlas, &vertices, origin, [flat, shadow]);
                for (quad, (name, _, midpoint, fallback, mapped)) in cases.iter().enumerate() {
                    let centre = if flat { midpoint } else { fallback };
                    for (corner, uv) in mapped.iter().enumerate() {
                        let actual = pixels[4 * quad + corner];
                        let expected = [centre[0], centre[1], uv[0], uv[1]];
                        if actual
                            .iter()
                            .zip(expected)
                            .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-6)
                        {
                            failures.push(format!("{name} corner {corner}, flat={flat} shadow={shadow} origin={origin}: {actual:?} != {expected:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn sample(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &PackAtlas,
    vertices: &[TerrainVertex],
    scale: f32,
    [flat, shadow]: [bool; 2],
) -> Vec<[f32; 4]> {
    let pp = |code| horizon_shaders::Preprocessed {
        code,
        ..Default::default()
    };
    let vertex = pp(format!(
        "attribute vec2 mc_midTexCoord;\nvarying vec4 probe;\nvoid main() {{\n\
         probe = vec4(mc_midTexCoord, gl_MultiTexCoord0.xy);\n\
         gl_Position = vec4((float(gl_VertexID) + 0.5) * {} - 1.0, 0.0, 0.0, 1.0);\n}}",
        2. / vertices.len() as f32,
    ));
    // Integer attachments retain the float bits without requiring float-render-target
    // capabilities on the adapter that the test runner picks.
    let fragment = pp(
        "varying vec4 probe;\nlayout(location = 0) out uvec4 midpoint_bits;\n\
        void main() { midpoint_bits = floatBitsToUint(probe); }"
            .into(),
    );
    let inputs = atlas.terrain_inputs(flat);
    let program = if shadow {
        horizon_shaders::translate_shadow(&vertex, &fragment, &inputs)
    } else {
        horizon_shaders::translate_gbuffers(
            &vertex,
            &fragment,
            horizon_shaders::Target::Buffers,
            &inputs,
        )
    }
    .unwrap();
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let entries = [
        vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
        vec![storage(0), storage(1)],
        vec![],
        vec![storage(0), storage(1)],
    ];
    let layouts: Vec<_> = entries
        .iter()
        .map(|entries| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        })
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
        ..Default::default()
    });
    let module = |module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        })
    };
    let vs = module(program.vertex);
    let fs = module(program.fragment);
    let vertex_layouts = [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &crate::gfx::TERRAIN_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![5 => Float32x4],
        }),
    ];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &vertex_layouts[..if flat { 1 } else { 2 }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::PointList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Uint,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    let uniform = buffer(
        &vec![0; program.interface.uniform_size.max(16) as usize],
        wgpu::BufferUsages::UNIFORM,
    );
    let slots = buffer(bytemuck::bytes_of(&0u32), wgpu::BufferUsages::STORAGE);
    let origin = buffer(
        bytemuck::bytes_of(&[0., 0., 0., scale]),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let ids = buffer(bytemuck::bytes_of(&[0i32; 15]), wgpu::BufferUsages::STORAGE);
    let data = buffer(
        bytemuck::cast_slice(vertices),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let bindings = [
        vec![(0, &uniform)],
        vec![(0, &slots), (1, &origin)],
        vec![],
        vec![(0, &ids), (1, &data)],
    ];
    let groups: Vec<_> = bindings
        .iter()
        .zip(&layouts)
        .map(|(buffers, layout)| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &buffers
                    .iter()
                    .map(|&(binding, buffer)| wgpu::BindGroupEntry {
                        binding,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: vertices.len() as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: vertices.len() as u64 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        for (k, group) in groups.iter().enumerate() {
            pass.set_bind_group(k as u32, group, &[]);
        }
        pass.set_vertex_buffer(0, data.slice(..));
        if !flat {
            pass.set_vertex_buffer(1, origin.slice(..));
        }
        pass.draw(0..vertices.len() as u32, 0..1);
    }
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = read.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
}
