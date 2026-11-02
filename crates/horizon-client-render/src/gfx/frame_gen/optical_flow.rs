// This file is part of the FidelityFX SDK.
//
// Copyright (C) 2024 Advanced Micro Devices, Inc.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files(the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and /or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions :
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! FSR 3.1's optical flow (AMD FidelityFX SDK 1.1.4, `ffx_opticalflow*`, MIT), ported to
//! WGSL (`optical_flow.wgsl`): block motion between the previous frame's colour and this
//! one's, and scene change detection, for frame interpolation (`interpolation.rs`).
//!
//! As in `ffx_opticalflow.cpp`: luma at the display size and 6 halvings, ping-ponged
//! between frames; a histogram of the luma for scene change detection; then from the
//! coarsest of 7 levels down, a block search seeded by the level above, a 3x3 filter and
//! a scale to the next level. The ported passes differ only where WGSL needs it: the luma
//! levels are `R32Float` holding AMD's 8-bit integers, wave operations are workgroup
//! atomics, and the luma pyramid is one dispatch per level instead of one for all (SPD).
//! The luma textures aren't cleared on a reset (AMD's are): its first 6 frames report a
//! scene change and no motion, and rewrite them.
//!
//! The vectors point from a block of this frame to where it was in the previous one, in
//! display pixels: content moving right by 4 pixels gives (-4, 0).

/// The size in pixels of the square blocks the flow has one vector for.
pub const BLOCK_SIZE: u32 = 8;
/// `OpticalFlowMaxPyramidLevels`: the search's levels.
const LEVELS: usize = 7;
/// `HistogramBins * HistogramsPerDim²`: the scene change histograms.
const HISTOGRAM_WIDTH: u64 = 256 * 9;

/// `optical_flow.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    display: [i32; 2],
    level: u32,
    level_count: u32,
    frame_index: u32,
    transfer_function: u32,
    min_max_luminance: [f32; 2],
}

struct Pipelines {
    prepare_luma: wgpu::ComputePipeline,
    luma_pyramid: wgpu::ComputePipeline,
    scd_histogram: wgpu::ComputePipeline,
    scd_divergence: wgpu::ComputePipeline,
    search: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    scale: wgpu::ComputePipeline,
}

/// One frame parity's bindings: which luma pyramid is this frame's.
struct Frame {
    /// This frame's luma at the display size, where `prepare_luma` writes.
    luma0: wgpu::TextureView,
    luma_pyramid: [wgpu::BindGroup; LEVELS - 1],
    scd_histogram: wgpu::BindGroup,
    search: [wgpu::BindGroup; LEVELS],
    scale: [wgpu::BindGroup; LEVELS - 1],
}

pub struct OpticalFlow {
    display: [u32; 2],
    pipelines: Pipelines,
    /// One per level, the level's constants (the passes outside the search use level 0's).
    params: Vec<wgpu::Buffer>,
    /// The bindings of the frames whose luma lands in pyramid 0, and in pyramid 1.
    frames: [Frame; 2],
    filter: [wgpu::BindGroup; LEVELS],
    scd_divergence: wgpu::BindGroup,
    scd_histogram: wgpu::Buffer,
    scd_previous_histogram: wgpu::Buffer,
    scd_temp: wgpu::Buffer,
    scd_output: wgpu::Buffer,
    vectors: wgpu::TextureView,
    flow_sizes: [[u32; 2]; LEVELS],
    /// `resourceFrameIndex` and `frameIndex`.
    resource_frame: u32,
    frame_index: u32,
    first: bool,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Bind `entries` (binding number, resource) for `pipeline`'s own layout.
fn bind(
    device: &wgpu::Device,
    pipeline: &wgpu::ComputePipeline,
    entries: &[(u32, wgpu::BindingResource)],
) -> wgpu::BindGroup {
    let entries: Vec<_> = entries
        .iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Optical flow"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    })
}

fn view(v: &wgpu::TextureView) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(v)
}

fn whole(b: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
    b.as_entire_binding()
}

impl OpticalFlow {
    /// For frames of `display` size.
    pub fn new(device: &wgpu::Device, display: [u32; 2]) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Optical flow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("optical_flow.wgsl").into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipelines = Pipelines {
            prepare_luma: pipeline("prepare_luma"),
            luma_pyramid: pipeline("luma_pyramid"),
            scd_histogram: pipeline("scd_histogram_pass"),
            scd_divergence: pipeline("scd_divergence_pass"),
            search: pipeline("search_pass"),
            filter: pipeline("filter_flow"),
            scale: pipeline("scale_pass"),
        };
        let params: Vec<_> = (0..LEVELS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Optical flow constants"),
                    size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let luma: [Vec<wgpu::TextureView>; 2] = [0, 1].map(|_| {
            (0..LEVELS)
                .map(|k| {
                    let size = display.map(|v| (v >> k).max(1));
                    texture(
                        device,
                        "Optical flow luma",
                        size,
                        wgpu::TextureFormat::R32Float,
                    )
                })
                .collect()
        });
        let mut flow_sizes = [[0; 2]; LEVELS];
        flow_sizes[0] = display.map(|v| v.div_ceil(BLOCK_SIZE));
        for l in 1..LEVELS {
            flow_sizes[l] = flow_sizes[l - 1].map(|v| v.div_ceil(2));
        }
        let flow = |label| -> Vec<wgpu::TextureView> {
            flow_sizes
                .iter()
                .map(|&size| texture(device, label, size, wgpu::TextureFormat::Rg32Sint))
                .collect()
        };
        // Per level: the scaled prediction from the level above, the search's vectors and
        // the filtered ones (level 0's are the output).
        let (predicted, searched, filtered) = (
            flow("Optical flow prediction"),
            flow("Optical flow search"),
            flow("Optical flow"),
        );
        let scd_histogram = buffer(device, "Scene change histogram", HISTOGRAM_WIDTH * 4);
        let scd_previous_histogram = buffer(
            device,
            "Scene change previous histogram",
            HISTOGRAM_WIDTH * 4,
        );
        let scd_temp = buffer(device, "Scene change temp", 12);
        let scd_output = buffer(device, "Scene change detection", 12);
        let frame = |current: usize| {
            let (cur, prev) = (&luma[current], &luma[1 - current]);
            Frame {
                luma0: cur[0].clone(),
                luma_pyramid: std::array::from_fn(|k| {
                    bind(
                        device,
                        &pipelines.luma_pyramid,
                        &[(2, view(&cur[k + 1])), (3, view(&cur[k]))],
                    )
                }),
                scd_histogram: bind(
                    device,
                    &pipelines.scd_histogram,
                    &[
                        (0, whole(&params[0])),
                        (3, view(&cur[0])),
                        (7, whole(&scd_histogram)),
                    ],
                ),
                search: std::array::from_fn(|l| {
                    bind(
                        device,
                        &pipelines.search,
                        &[
                            (0, whole(&params[l])),
                            (3, view(&cur[l])),
                            (4, view(&prev[l])),
                            (5, view(&predicted[l])),
                            (6, view(&searched[l])),
                            (10, whole(&scd_output)),
                        ],
                    )
                }),
                scale: std::array::from_fn(|i| {
                    let l = i + 1;
                    bind(
                        device,
                        &pipelines.scale,
                        &[
                            (0, whole(&params[l])),
                            (3, view(&cur[l])),
                            (4, view(&prev[l])),
                            (5, view(&filtered[l])),
                            (6, view(&predicted[l - 1])),
                            (10, whole(&scd_output)),
                        ],
                    )
                }),
            }
        };
        let frames = [frame(0), frame(1)];
        let filter = std::array::from_fn(|l| {
            bind(
                device,
                &pipelines.filter,
                &[(5, view(&searched[l])), (6, view(&filtered[l]))],
            )
        });
        let scd_divergence = bind(
            device,
            &pipelines.scd_divergence,
            &[
                (7, whole(&scd_histogram)),
                (8, whole(&scd_previous_histogram)),
                (9, whole(&scd_temp)),
                (10, whole(&scd_output)),
            ],
        );
        Self {
            display,
            pipelines,
            params,
            frames,
            filter,
            scd_divergence,
            scd_histogram,
            scd_previous_histogram,
            scd_temp,
            scd_output,
            vectors: filtered[0].clone(),
            flow_sizes,
            resource_frame: 0,
            frame_index: 0,
            first: true,
        }
    }

    /// The flow from the previous call's `color` to this one's. `color`: `display` size,
    /// a float texture holding Horizon's frame (gamma-encoded RGB, the swapchain's
    /// values: AMD's SDR back buffer). `reset`: the previous frame is unrelated (first
    /// frame, resize, teleport).
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        reset: bool,
    ) {
        let reset = reset || self.first;
        self.first = false;
        self.frame_index = if reset { 0 } else { self.frame_index + 1 };
        if reset {
            for b in [
                &self.scd_temp,
                &self.scd_output,
                &self.scd_histogram,
                &self.scd_previous_histogram,
            ] {
                encoder.clear_buffer(b, 0, None);
            }
        }
        for (level, buffer) in self.params.iter().enumerate() {
            let params = Params {
                display: self.display.map(|v| v as i32),
                level: level as u32,
                level_count: LEVELS as u32,
                frame_index: self.frame_index,
                transfer_function: 0,
                min_max_luminance: [0.0, 1.0],
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
        }
        let frame = &self.frames[(self.resource_frame & 1) as usize];
        let prepare = bind(
            device,
            &self.pipelines.prepare_luma,
            &[
                (0, whole(&self.params[0])),
                (1, view(color)),
                (2, view(&frame.luma0)),
            ],
        );
        let p = &self.pipelines;
        let [w, h] = self.display;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Optical flow"),
            timestamp_writes: None,
        });
        let mut run = |pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, x, y, z| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(x, y, z);
        };
        run(
            &p.prepare_luma,
            &prepare,
            (w.div_ceil(2)).div_ceil(16),
            (h.div_ceil(2)).div_ceil(16),
            1,
        );
        for (k, group) in frame.luma_pyramid.iter().enumerate() {
            let size = self.display.map(|v| (v >> (k + 1)).max(1));
            run(
                &p.luma_pyramid,
                group,
                size[0].div_ceil(8),
                size[1].div_ceil(8),
                1,
            );
        }
        run(
            &p.scd_histogram,
            &frame.scd_histogram,
            ((w / 4) / 3).div_ceil(32),
            16,
            9,
        );
        run(&p.scd_divergence, &self.scd_divergence, 9, 3, 1);
        for level in (0..LEVELS).rev() {
            let luma = self.display.map(|v| (v >> level).max(1));
            run(
                &p.search,
                &frame.search[level],
                (luma[0].div_ceil(4) * 16).div_ceil(64),
                luma[1].div_ceil(16),
                1,
            );
            let size = self.flow_sizes[level];
            run(
                &p.filter,
                &self.filter[level],
                size[0].div_ceil(16),
                size[1].div_ceil(4),
                1,
            );
            if level > 0 {
                let next = self.flow_sizes[level - 1];
                run(
                    &p.scale,
                    &frame.scale[level - 1],
                    next[0].div_ceil(4),
                    next[1].div_ceil(4),
                    1,
                );
            }
        }
        drop(pass);
        self.resource_frame = (self.resource_frame + 1) % 16;
    }

    /// This frame's vectors, one per `BLOCK_SIZE` block: `Rg32Sint`, the display size
    /// over `BLOCK_SIZE` rounded up,
    /// holding the values FSR's `R16G16_SINT` optical flow vector texture holds.
    pub fn vectors(&self) -> &wgpu::TextureView {
        &self.vectors
    }

    /// FSR's scene change detection texture (`R32_UINT`, 3 texels) as a storage
    /// buffer of 3 `u32`: element `i` holds texel `i`'s value.
    pub fn scene_change(&self) -> &wgpu::Buffer {
        &self.scd_output
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("optical_flow.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    /// A device with WebGPU's default limits (what every GPU wgpu runs on has), or `None`
    /// without a GPU.
    pub fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// Noise in 4x4 pixel cells, moved by `shift` pixels: what was at `p` is at `p + shift`.
    /// A `seed` of 100 or more gives cells of one primary each.
    pub fn pattern(size: [u32; 2], seed: u32, shift: [i32; 2]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for y in 0..size[1] as i32 {
            for x in 0..size[0] as i32 {
                let (cx, cy) = ((x - shift[0]).div_euclid(4), (y - shift[1]).div_euclid(4));
                let mut h = (cx as u32).wrapping_mul(0x9e37_79b9)
                    ^ (cy as u32).wrapping_mul(0x85eb_ca6b)
                    ^ seed.wrapping_mul(0xc2b2_ae35);
                h ^= h >> 15;
                h = h.wrapping_mul(0x2c1b_3c6d);
                h ^= h >> 12;
                let v = (h & 0xff) as u8;
                if seed >= 100 {
                    // Primaries: no two neighbours' colours point the same way.
                    let mut c = [0, 0, 0, 255];
                    c[(h >> 8) as usize % 3] = 128 + v / 2;
                    pixels.extend_from_slice(&c);
                } else {
                    pixels.extend_from_slice(&[v, v.wrapping_add(40), v / 2, 255]);
                }
            }
        }
        pixels
    }

    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        pixels: &[u8],
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
        texture.create_view(&Default::default())
    }

    /// `texture`'s texels (`texel` bytes each), row by row.
    pub fn read(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        texel: u32,
    ) -> Vec<u8> {
        let size = texture.size();
        let row = (size.width * texel).next_multiple_of(256);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        mapped
            .chunks(row as usize)
            .flat_map(|r| r[..(size.width * texel) as usize].to_vec())
            .collect()
    }

    fn run(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        flow: &mut OpticalFlow,
        color: &wgpu::TextureView,
        reset: bool,
    ) {
        let mut encoder = device.create_command_encoder(&Default::default());
        flow.dispatch(device, queue, &mut encoder, color, reset);
        queue.submit([encoder.finish()]);
    }

    fn vectors(device: &wgpu::Device, queue: &wgpu::Queue, flow: &OpticalFlow) -> Vec<[i32; 2]> {
        read(device, queue, flow.vectors.texture(), 8)
            .chunks(8)
            .map(|c| {
                [
                    i32::from_le_bytes(c[0..4].try_into().unwrap()),
                    i32::from_le_bytes(c[4..8].try_into().unwrap()),
                ]
            })
            .collect()
    }

    /// A frame moved right by 16 and down by 8: the blocks inside report where they were
    /// (-16, -8), and nothing reads as a scene change. A much darker frame then does.
    #[test]
    fn finds_a_shift_and_a_scene_change() {
        let Some((device, queue)) = device() else {
            return;
        };
        let size = [512, 288];
        let still = upload(&device, &queue, size, &pattern(size, 1, [0, 0]));
        let moved = upload(&device, &queue, size, &pattern(size, 1, [16, 8]));
        // Scene changes are told by the luma's histogram: a much darker frame.
        let dark: Vec<u8> = pattern(size, 2, [0, 0]).iter().map(|v| v / 5).collect();
        let other = upload(&device, &queue, size, &dark);
        let mut flow = OpticalFlow::new(&device, size);
        // The first 6 frames after a reset are scene changes.
        run(&device, &queue, &mut flow, &still, true);
        for _ in 0..6 {
            run(&device, &queue, &mut flow, &still, false);
        }
        run(&device, &queue, &mut flow, &moved, false);
        let [w, h] = flow.flow_sizes[0];
        let found = vectors(&device, &queue, &flow);
        let inside: Vec<_> = (4..h - 4)
            .flat_map(|y| (4..w - 4).map(move |x| (x, y)))
            .map(|(x, y)| found[(y * w + x) as usize])
            .collect();
        let right = inside.iter().filter(|v| **v == [-16, -8]).count();
        assert!(
            right * 10 >= inside.len() * 9,
            "{right} of {} blocks: {:?}",
            inside.len(),
            &inside[..8]
        );
        let scd = |device: &wgpu::Device, queue: &wgpu::Queue, flow: &OpticalFlow| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 12,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(flow.scene_change(), 0, &buffer, 0, 12);
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
            u32::from_le_bytes(bytes[4..8].try_into().unwrap())
        };
        assert_eq!(scd(&device, &queue, &flow) & 0xf, 0);
        run(&device, &queue, &mut flow, &other, false);
        assert_eq!(scd(&device, &queue, &flow) & 1, 1);
        assert!(vectors(&device, &queue, &flow).iter().all(|v| *v == [0, 0]));
    }
}
