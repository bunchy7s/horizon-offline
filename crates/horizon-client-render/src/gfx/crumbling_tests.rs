use wgpu::util::DeviceExt;

/// Sample the production WGSL with the production nearest/no-mipmap atlas sampler.
#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn crack_samples_stay_aligned_between_stages() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    eprintln!("crack sampling adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct Case {
        uv: [f32; 2],
        stage: u32,
        extent: u32,
    }
    let mut cases = Vec::new();
    let mut expected = Vec::<[u32; 2]>::new();
    // Include exact texel boundaries, their neighbours, negative projected UVs,
    // wrapping and the far sprite edge. Origins straddle the packing error's sign change.
    for extent in [16, 32] {
        for x in [32, 2080, 4064] {
            for y in [0, 32] {
                for i in -32..=32 {
                    for delta in [-0.0001, 0.0, 0.0001] {
                        let uv = [i as f32 / 16.0 + delta, 0.5 - delta];
                        cases.push(Case {
                            uv,
                            stage: crate::breaking::stage_light([
                                x as f32 / 4096.0,
                                y as f32 / 64.0,
                            ]),
                            extent,
                        });
                        expected.push(uv.map(|v| {
                            (((v - v.floor()) * extent as f32).floor() as u32).min(extent - 1)
                        }));
                    }
                }
            }
        }
    }
    let math = include_str!("../entity.wgsl")
        .split("// CRUMBLING_UV_BEGIN")
        .nth(1)
        .unwrap()
        .split("// CRUMBLING_UV_END")
        .next()
        .unwrap();
    let source = format!(
        "{math}\n{}",
        r#"
struct Case { uv: vec2<f32>, stage: u32, extent: u32 }
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<vec4<u32>>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    let origin = vec2(f32(c.stage & 65535u), f32(c.stage >> 16u)) / 65535.0;
    let dimensions = textureDimensions(atlas);
    let uv = crumbling_uv(c.uv, origin, vec2(f32(c.extent)) / vec2<f32>(dimensions), dimensions);
    // Keep the old addressing as a control: the fixture must reproduce its stage jitter.
    let old_uv = origin + fract(c.uv) * vec2(f32(c.extent)) / vec2<f32>(dimensions);
    results[id.x] = vec4<u32>(
        vec2<u32>(round(textureSampleLevel(atlas, atlas_sampler, uv, 0.0).rg * 255.0)),
        vec2<u32>(round(textureSampleLevel(atlas, atlas_sampler, old_uv, 0.0).rg * 255.0)));
}"#
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Crack sampling regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    // Every sprite repeats the same coordinate pattern, so moving a stage's
    // atlas origin must leave every sampled local texel unchanged.
    let pixels: Vec<u8> = (0..64)
        .flat_map(|y| (0..4096).flat_map(move |x| [(x % 32) as u8, (y % 32) as u8, 0, 255]))
        .collect();
    let atlas = device.create_texture_with_data(
        &queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 4096,
                height: 64,
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
        &pixels,
    );
    let view = atlas.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (expected.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    let actual: &[[u32; 4]] = bytemuck::cast_slice(&mapped);
    for (i, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            &actual[..2],
            expected,
            "case {i}, uv={:?}, stage={}, extent={}",
            cases[i].uv,
            cases[i].stage,
            cases[i].extent
        );
    }
    let old_mismatches = actual
        .iter()
        .zip(&expected)
        .filter(|(a, e)| &a[2..] != e.as_slice())
        .count();
    assert!(
        old_mismatches > 0,
        "fixture must detect the original sampling bug"
    );
    eprintln!(
        "{} crack texture samples matched; old addressing mismatched {old_mismatches}",
        cases.len()
    );
}
