//! Weather's unsplit quads, using the entity atlas/lightmap resources.
use crate::weather::{Time, Vertex};

pub(super) struct Weather {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    capacity: usize,
    count: u32,
}
impl Weather {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        entities: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Weather time"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Weather layout"),
            bind_group_layouts: &[Some(entities), Some(&bindings)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4,
            3=>Float32x4,4=>Uint32x4,5=>Unorm8x4,6=>Uint8x4,7=>Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Weather"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_weather"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_weather"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Weather time"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Weather time"),
            layout: &bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            group,
            uniform,
            vertices: Self::buffer(device, 4),
            capacity: 4,
            count: 0,
        }
    }
    fn buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Weather vertices"),
            size: (capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        vertices: &[Vertex],
        time: Time,
    ) {
        self.count = vertices.len() as u32;
        if vertices.is_empty() {
            return;
        }
        if vertices.len() > self.capacity {
            self.capacity = vertices.len().next_power_of_two();
            self.vertices = Self::buffer(device, self.capacity);
        }
        uploads.buffer(encoder, &self.vertices, 0, bytemuck::cast_slice(vertices));
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&time));
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, entities: &wgpu::BindGroup) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, entities, &[]);
        pass.set_bind_group(1, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        // The renderer binds the shared triangle-list quad indices.
        pass.draw_indexed(0..self.count / 4 * 6, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use crate::weather::{Parameters, Time};
    use wgpu::util::DeviceExt;

    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct Case {
        animation: [f32; 4],
        bases: [u32; 4],
        time: [f32; 4],
        uv: [f32; 2],
        snow: u32,
        _pad: u32,
    }

    /// Execute the production WGSL math, rather than a Rust translation of it.
    /// Requires a GPU; run explicitly on renderer changes, not on headless CI.
    #[test]
    #[ignore = "requires a Vulkan/Metal/DX12 adapter"]
    fn weather_shader_uvs_match_legacy_bits() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        eprintln!("weather parity adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut cases = Vec::new();
        let mut expected = Vec::new();
        for i in 0..4096i32 {
            let x = i.wrapping_mul(45_238_971);
            let z = i.wrapping_mul(-418_711).wrapping_add(i32::MIN);
            let p = Parameters::new(x, z);
            let bases = p.snow_base.map(f64::to_bits);
            for ticks in [-1, 0, 511, 512, 131_071, 131_072, i64::MAX] {
                let partial = [0.0, 0.5, 0.999, f32::from_bits(0x3f7fffff)][i as usize % 4];
                for snow in [false, true] {
                    let column = p.column(snow, ticks, x, -64, 320, z, 0, partial);
                    let uv = [1.0, if i & 1 == 0 { -16.0 } else { 80.0 }];
                    cases.push(Case {
                        animation: [
                            p.rain_speed,
                            p.rain_tick_offset,
                            p.snow_drift[0],
                            p.snow_drift[1],
                        ],
                        bases: [
                            bases[0] as u32,
                            (bases[0] >> 32) as u32,
                            bases[1] as u32,
                            (bases[1] >> 32) as u32,
                        ],
                        time: Time::new(ticks, partial).values,
                        uv,
                        snow: u32::from(snow),
                        _pad: 0,
                    });
                    expected.push([
                        (uv[0] + column.u_offset).to_bits(),
                        (uv[1] + column.v_offset).to_bits(),
                    ]);
                }
            }
        }
        // Force halfway rounding, cancellation, zero, tiny bases and either
        // sign of drift. Gaussian seeds alone rarely land on exact midpoints.
        for base in [
            0.0,
            2.0f64.powi(-53),
            0.5,
            0.5 + 2.0f64.powi(-25),
            0.5 + 2.0f64.powi(-25) + 2.0f64.powi(-53),
            0.5 + 2.0f64.powi(-14) + 2.0f64.powi(-53),
            1.0 - 2.0f64.powi(-53),
        ] {
            for delta in [0.0f32, -0.0, -0.5, 0.5, 1024.0, -1024.0, 2.0f32.powi(-40)] {
                // time=100, u scale=.01 -> delta exactly, v is unused.
                let bits = base.to_bits();
                cases.push(Case {
                    animation: [3.0, 0.0, delta, 0.0],
                    bases: [bits as u32, (bits >> 32) as u32, 0, 0],
                    time: [100.0, 0.0, 0.0, 0.0],
                    uv: [0.0, 0.0],
                    snow: 1,
                    _pad: 0,
                });
                expected.push([((base + f64::from(delta)) as f32).to_bits(), 0]);
            }
        }
        let expected: Vec<[u32; 4]> = expected
            .into_iter()
            .map(|bits| {
                let uv = bits.map(f32::from_bits);
                let texel = [
                    256 + ((uv[0] - uv[0].floor()) * 16.0).floor() as u32,
                    512 + ((uv[1] - uv[1].floor()) * 32.0).floor() as u32,
                ];
                [bits[0], bits[1], texel[0], texel[1]]
            })
            .collect();
        let math = include_str!("../entity.wgsl")
            .split("// WEATHER_MATH_BEGIN")
            .nth(1)
            .unwrap()
            .split("// WEATHER_MATH_END")
            .next()
            .unwrap();
        let source = format!(
            "{math}\n{}",
            r#"
struct Case { animation: vec4<f32>, bases: vec4<u32>, time: vec4<f32>, uv: vec2<f32>, snow: u32, pad: u32 }
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<vec4<u32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    let uv = c.uv + weather_phase(c.animation, c.bases, c.snow, c.time);
    let texel = weather_texel(uv, vec4(0.25, 0.5, 0.015625, 0.03125), vec2(1024u));
    results[id.x] = vec4(bitcast<vec2<u32>>(uv), vec2<u32>(texel));
}"#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Weather parity"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Weather parity"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
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
                actual, expected,
                "case {i}, snow={}, time={:?}, animation={:?}",
                cases[i].snow, cases[i].time, cases[i].animation
            );
        }
        eprintln!("{} GPU weather UV cases matched bit for bit", cases.len());
    }
}
