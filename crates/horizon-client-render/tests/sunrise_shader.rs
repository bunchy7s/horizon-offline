#[path = "../src/gfx/sunrise_fan.rs"]
mod sunrise_fan;

#[path = "../src/gfx/wgsl_prefix.rs"]
mod wgsl_prefix;

fn sunrise_shader() -> String {
    let terrain = include_str!("../src/terrain.wgsl");
    let start = terrain.find("fn sunrise_weight_full").unwrap();
    let end = terrain[start..].find("// The sky's colour").unwrap() + start;
    let mut source = String::from(
        "struct Globals { sunrise: vec4<f32>, sunrise_fan: array<vec4<f32>,32> }\n\
         @group(0) @binding(0) var<uniform> globals: Globals;\n\
         @group(0) @binding(1) var<storage,read> rays: array<vec4<f32>>;\n\
         @group(0) @binding(2) var<storage,read_write> results: array<vec2<f32>>;\n",
    );
    source.push_str(&terrain[start..end]);
    source.push_str(
        "@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {\n\
             if id.x < arrayLength(&rays) {\n\
                 let d = rays[id.x].xyz;\n\
                 results[id.x] = vec2(sunrise_weight_full(d), sunrise_weight(d));\n\
             }\n\
         }\n",
    );
    source
}

#[test]
fn extracted_sunrise_shader_validates() {
    let module = naga::front::wgsl::parse_str(&sunrise_shader()).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

// Execute the actual optimized and reference WGSL on a host GPU.
// This opt-in test compares shipped WGSL with the full fan search, not FPS.
#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn sunrise_sectors_match_reference_on_real_gpu() {
    use wgpu::util::DeviceExt;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("Sunrise GPU correctness: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(sunrise_shader().into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut rays = Vec::<[f32; 4]>::new();
    for yaw in 0..360 {
        for pitch in -89..90 {
            let (sy, cy) = (yaw as f32).to_radians().sin_cos();
            let (sp, cp) = (pitch as f32).to_radians().sin_cos();
            rays.push([sy * cp, sp, cy * cp, 0.]);
        }
    }
    rays.extend([
        [1., 0., 0., 0.],
        [-1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., -1., 0., 0.],
    ]);
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&rays),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (rays.len() * 8) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for alpha in [0.01, 0.2, 0.65, 1.] {
        for hemisphere in [-1., 1.] {
            let mut globals = vec![[1., 1., 1., alpha]];
            globals.extend(sunrise_fan::normals(
                alpha,
                glam::Vec3::new(0., 0., hemisphere),
            ));
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&globals),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = [&uniform, &input, &output];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &bindings
                    .iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: b.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups((rays.len() as u32).div_ceil(64), 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
            queue.submit([encoder.finish()]);
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            {
                let mapped = read.slice(..).get_mapped_range().unwrap();
                let pairs: &[[f32; 2]] = bytemuck::cast_slice(&mapped);
                for (i, pair) in pairs.iter().enumerate() {
                    assert!(
                        (pair[0] - pair[1]).abs() < 1e-4,
                        "alpha={alpha} hemisphere={hemisphere} ray={:?} values={pair:?}",
                        rays[i]
                    );
                }
            }
            read.unmap();
        }
    }
}

#[test]
fn native_pack_sky_keeps_sunrise_helpers_private() {
    use wgsl_prefix::{TERRAIN, prefixed, section};
    // Use the same complete terrain sections and prefixer as native_sky.rs.
    let sky = prefixed(
        &[
            section(TERRAIN, "struct Globals {", "@group(0) @binding(1)"),
            section(TERRAIN, "fn linear_fog(", "fn apply_fog("),
            section(TERRAIN, "struct SkyOut {", "// rendertype_lines"),
        ]
        .concat(),
        "horizon_sky_",
    );
    for name in [
        "sunrise_weight",
        "sunrise_weight_full",
        "sunrise_triangle_weight",
    ] {
        assert!(sky.contains(&format!("fn horizon_sky_{name}(")));
    }
    // A native pack may declare the same helper names as the renderer.
    let own = "fn sunrise_weight(d: vec3<f32>) -> f32 { return d.x; }\n\
               fn sunrise_weight_full(d: vec3<f32>) -> f32 { return d.y; }\n\
               fn sunrise_triangle_weight(d: vec3<f32>, i: u32) -> f32 { return d.z + f32(i); }\n";
    let module = naga::front::wgsl::parse_str(&format!("{own}\n{sky}")).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn complete_terrain_shader_still_validates_for_fallback_sky() {
    let module = naga::front::wgsl::parse_str(wgsl_prefix::TERRAIN).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}
