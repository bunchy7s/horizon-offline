//! Animated block sprites, drawn on the GPU as vanilla 26.3 draws them:
//! `SpriteContents.AnimatedTexture` keeps every frame in video memory, and
//! each tick `animate_sprite_blit` or `animate_sprite_interpolate` draws the
//! frame shown (mixed with the next one for `interpolate`) into the atlas.
//!
//! Every frame's mip chain is uploaded once, at the sprite's own size. A tick
//! that changes sprites writes one small job per changed sprite and runs one
//! compute dispatch per mip level (`animate.wgsl`), which repeats the chain
//! into the array's larger layers by whole pixels and mixes interpolated
//! frames. Blending on the CPU at the array's size and copying every changed
//! sprite level by level (a copy command each, a blit each in Mesa) was about
//! a fifth of the render thread on an integrated GPU.
//!
//! Without compute shaders (OpenGL before 4.3, the last fallback) the same
//! jobs run on the CPU, the same integer mix as `animate.wgsl`, and each
//! changed level is copied into its layer.
use super::upload::Uploads;
use crate::mipmap::Settings;
use crate::sprites::{FrameBlend, SpriteAnimation};
use wgpu::util::DeviceExt;

/// Bytes of `animate.wgsl`'s `Job`.
const JOB_BYTES: usize = 20;
/// `animate.wgsl`'s workgroup edge.
const WORKGROUP: u32 = 8;

struct Sprite {
    animation: SpriteAnimation,
    /// What its layer shows (`None`: the frame the array was created with,
    /// rewritten on the first tick).
    shown: Option<FrameBlend>,
    /// Its layer in its sprite array.
    layer: u32,
    /// Each frame's mip chain: its offset in words in the group's frames.
    frames: Vec<u32>,
    /// Edge of the chains' first level.
    edge: u32,
    group: usize,
}

/// The animated sprites of one sprite array whose frames share one buffer.
struct Group {
    /// Where its jobs start in `SpriteAnimator::jobs`, in bytes.
    offset: u64,
    /// One bind group per mip level of the array (none on the CPU).
    levels: Vec<wgpu::BindGroup>,
    /// This tick's jobs.
    jobs: Vec<u8>,
    /// Its sprite array and its frames, for the CPU.
    array: usize,
    buffer: usize,
}

/// Where the jobs run.
enum Writer {
    Gpu {
        jobs: wgpu::Buffer,
        pipeline: wgpu::ComputePipeline,
    },
    /// Every frames buffer's words, and the sprite arrays to copy into.
    Cpu {
        frames: Vec<Vec<u8>>,
        textures: Vec<wgpu::Texture>,
    },
}

pub(super) struct SpriteAnimator {
    sprites: Vec<Sprite>,
    groups: Vec<Group>,
    writer: Writer,
    /// Edge of the arrays' layers.
    size: u32,
    /// The layers the last `animate` rewrote, counted over all arrays.
    changed: Vec<u32>,
}

impl SpriteAnimator {
    /// `textures` are the sprite arrays, `split` layers each, `size` pixels
    /// wide; `animations` hold their frames at that size. `compute`: whether
    /// the device says it runs compute shaders writing storage textures.
    pub fn new(
        device: &wgpu::Device,
        animations: &[SpriteAnimation],
        size: u32,
        settings: &[Settings],
        textures: &[wgpu::Texture],
        split: u32,
        compute: bool,
    ) -> Self {
        let pipeline = if compute { pipeline(device) } else { None };
        let limits = device.limits();
        let limit = if pipeline.is_some() {
            limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size)
        } else {
            // Word offsets.
            u64::from(u32::MAX) * 4
        };
        let mut frames: Vec<Vec<u8>> = Vec::new();
        // Each group's sprite array and frames buffer.
        let mut keys: Vec<(u32, usize)> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        let mut sprites = Vec::with_capacity(animations.len());
        for animation in animations {
            let pixels: Vec<_> = (0..animation.frame_count())
                .map(|frame| animation.pixels(frame))
                .collect();
            let (chains, edge) = source_chains(&pixels, size, settings, animation.layer);
            let bytes: u64 = chains.iter().map(|chain| chain.len() as u64).sum();
            if bytes > limit {
                log::warn!(
                    "Horizon: layer {}'s animation ({bytes} bytes) does not fit in a storage buffer; it stays still",
                    animation.layer
                );
                continue;
            }
            if frames
                .last()
                .is_none_or(|data| data.len() as u64 + bytes > limit)
            {
                frames.push(Vec::new());
            }
            let buffer = frames.len() - 1;
            let data = &mut frames[buffer];
            let offsets = chains
                .iter()
                .map(|chain| {
                    let offset = (data.len() / 4) as u32;
                    data.extend_from_slice(chain);
                    offset
                })
                .collect();
            let key = (animation.layer / split, buffer);
            let group = keys.iter().position(|&k| k == key).unwrap_or_else(|| {
                keys.push(key);
                counts.push(0);
                keys.len() - 1
            });
            counts[group] += 1;
            sprites.push(Sprite {
                animation: animation.clone(),
                shown: None,
                layer: animation.layer % split,
                frames: offsets,
                edge,
                group,
            });
        }
        let Some(pipeline) = pipeline else {
            let groups = keys
                .iter()
                .zip(&counts)
                .map(|(&(array, buffer), &count)| Group {
                    offset: 0,
                    levels: Vec::new(),
                    jobs: Vec::with_capacity(count * JOB_BYTES),
                    array: array as usize,
                    buffer,
                })
                .collect();
            return Self {
                sprites,
                groups,
                writer: Writer::Cpu {
                    frames,
                    textures: textures.to_vec(),
                },
                size,
                changed: Vec::new(),
            };
        };
        let frames: Vec<_> = frames
            .iter()
            .map(|data| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Animated sprite frames"),
                    contents: data,
                    usage: wgpu::BufferUsages::STORAGE,
                })
            })
            .collect();
        let align = u64::from(limits.min_storage_buffer_offset_alignment);
        let mut offsets = Vec::with_capacity(counts.len());
        let mut total = 0;
        for &count in &counts {
            offsets.push(total);
            total = (total + (count * JOB_BYTES) as u64).next_multiple_of(align);
        }
        let jobs = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Animated sprite jobs"),
            size: total.max(align),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let groups = keys
            .iter()
            .zip(&counts)
            .zip(offsets)
            .map(|((&(array, buffer), &count), offset)| {
                let texture = &textures[array as usize];
                let levels = (0..texture.mip_level_count())
                    .map(|level| {
                        let view = texture.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("Animated sprites"),
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            base_mip_level: level,
                            mip_level_count: Some(1),
                            ..Default::default()
                        });
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("Animated sprites"),
                            layout: &layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: frames[buffer].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &jobs,
                                        offset,
                                        size: wgpu::BufferSize::new((count * JOB_BYTES) as u64),
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                            ],
                        })
                    })
                    .collect();
                Group {
                    offset,
                    levels,
                    jobs: Vec::with_capacity(count * JOB_BYTES),
                    array: array as usize,
                    buffer,
                }
            })
            .collect();
        Self {
            sprites,
            groups,
            writer: Writer::Gpu { jobs, pipeline },
            size,
            changed: Vec::new(),
        }
    }

    /// Whether the jobs can run on the GPU: compute shaders writing a
    /// storage texture and reading two storage buffers (not OpenGL before
    /// 4.3).
    pub fn compute(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
        let limits = device.limits();
        adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            && limits.max_storage_textures_per_shader_stage >= 1
            && limits.max_storage_buffers_per_shader_stage >= 2
    }

    /// The layers the last `animate` rewrote (for the shader pack atlas).
    pub fn changed(&self) -> &[u32] {
        &self.changed
    }

    /// Show every sprite's frame at `tick`, rewriting only the sprites whose
    /// frame or mix changed (`SpriteContents.AnimationState.tick`).
    pub fn animate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut Uploads,
        tick: u64,
    ) {
        if self.collect(tick) == 0 {
            return;
        }
        let (jobs, pipeline) = match &self.writer {
            Writer::Gpu { jobs, pipeline } => (jobs, pipeline),
            Writer::Cpu { frames, textures } => {
                for group in &mut self.groups {
                    let (frames, texture) = (&frames[group.buffer], &textures[group.array]);
                    for job in group.jobs.as_chunks::<JOB_BYTES>().0 {
                        let (words, _) = job.as_chunks::<4>();
                        let job: [u32; 5] = std::array::from_fn(|i| u32::from_le_bytes(words[i]));
                        for level in 0..texture.mip_level_count() {
                            let width = self.size >> level;
                            uploads.texture(
                                encoder,
                                wgpu::TexelCopyTextureInfo {
                                    texture,
                                    mip_level: level,
                                    origin: wgpu::Origin3d {
                                        x: 0,
                                        y: 0,
                                        z: job[0],
                                    },
                                    aspect: wgpu::TextureAspect::All,
                                },
                                width,
                                width,
                                &mix_level(frames, job, width),
                            );
                        }
                    }
                    group.jobs.clear();
                }
                return;
            }
        };
        for group in &self.groups {
            uploads.buffer(encoder, jobs, group.offset, &group.jobs);
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Animated sprites"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        for group in &mut self.groups {
            let count = (group.jobs.len() / JOB_BYTES) as u32;
            if count == 0 {
                continue;
            }
            for (level, bind) in group.levels.iter().enumerate() {
                let groups = (self.size >> level).div_ceil(WORKGROUP);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(groups, groups, count);
            }
            group.jobs.clear();
        }
    }

    /// Queue a job for every sprite whose blend at `tick` differs from what
    /// its layer shows; the number of jobs.
    fn collect(&mut self, tick: u64) -> usize {
        let mut count = 0;
        self.changed.clear();
        for sprite in &mut self.sprites {
            let blend = sprite.animation.blend(tick);
            if sprite.shown == Some(blend) {
                continue;
            }
            sprite.shown = Some(blend);
            self.changed.push(sprite.animation.layer);
            let job = [
                sprite.layer,
                sprite.frames[blend.frame],
                sprite.frames[blend.next],
                blend.millis,
                sprite.edge,
            ];
            let jobs = &mut self.groups[sprite.group].jobs;
            for word in job {
                jobs.extend_from_slice(&word.to_le_bytes());
            }
            count += 1;
        }
        count
    }
}

/// `animate.wgsl`'s pipeline, unless the device fails to make it: an OpenGL
/// driver can list compute shaders among its extensions while wgpu writes
/// the GLSL version of its core version, which has none (Mesa's
/// `MESA_GL_VERSION_OVERRIDE`, older drivers).
fn pipeline(device: &wgpu::Device) -> Option<wgpu::ComputePipeline> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Animated sprites"),
        source: wgpu::ShaderSource::Wgsl(include_str!("animate.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Animated sprites"),
        layout: None,
        module: &module,
        entry_point: Some("animate"),
        compilation_options: Default::default(),
        cache: None,
    });
    let internal = pollster::block_on(internal.pop());
    match internal.or(pollster::block_on(validation.pop())) {
        None => Some(pipeline),
        Some(e) => {
            log::warn!("Horizon renderer: animated sprites drawn on the CPU ({e})");
            None
        }
    }
}

/// `animate.wgsl` on the CPU: one `width` wide level of a job
/// (`[layer, current, next, millis, edge]`) from `frames`' chains.
fn mix_level(frames: &[u8], [_, current, next, millis, edge]: [u32; 5], width: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * width * 4) as usize);
    for y in 0..width {
        for x in 0..width {
            let texel = if width >= edge {
                let scale = width / edge;
                y / scale * edge + x / scale
            } else {
                4 * (edge * edge - width * width) / 3 + y * width + x
            };
            let at = |chain: u32| ((chain + texel) * 4) as usize;
            let a = &frames[at(current)..at(current) + 4];
            if millis == 0 {
                out.extend_from_slice(a);
                continue;
            }
            let b = &frames[at(next)..at(next) + 4];
            out.extend(a.iter().zip(b).map(|(&a, &b)| {
                ((u32::from(a) * (1000 - millis) + u32::from(b) * millis + 500) / 1000) as u8
            }));
        }
    }
    out
}

/// The mip chain a frame's layer is made of, and its first level's edge.
/// `mipmap::generate` makes a sprite scaled up by whole pixels to the
/// array's size from its own size's chain, its first level repeated into the
/// larger levels: that chain, which the shader repeats. Any other sprite: its
/// chain at the array's size.
#[cfg(test)]
fn source_chain(pixels: &[u8], size: u32, settings: &[Settings], layer: u32) -> (Vec<u8>, u32) {
    let (mut chains, edge) = source_chains(&[pixels], size, settings, layer);
    (chains.swap_remove(0), edge)
}

/// Every frame's chain, laid out as one frame's `source_chain`, but mipmapped as
/// vanilla's `SpriteContents.increaseMipLevel` does: over the whole animation
/// sheet (the frames stacked), so `solidify` and the alpha-to-coverage scale
/// of each level are the sheet's, not each frame's. A frame alone has a
/// single texel square at its 2x2 level, and its coverage scale could round
/// a whole far frame transparent (kelp blinking out at distance).
fn source_chains(
    frames: &[&[u8]],
    size: u32,
    settings: &[Settings],
    layer: u32,
) -> (Vec<Vec<u8>>, u32) {
    let s = settings.get(layer as usize).copied().unwrap_or_default();
    let edge = if s.native_size.is_power_of_two() && s.native_size < size {
        s.native_size
    } else {
        size
    };
    let mut sheet = crate::mipmap::Image {
        width: edge,
        height: edge * frames.len() as u32,
        pixels: Vec::with_capacity((edge * edge) as usize * frames.len()),
    };
    for frame in frames {
        let frame = crate::sprites::resample(frame, size, edge);
        sheet.pixels.extend_from_slice(frame.as_chunks::<4>().0);
    }
    let has_transparent = s.has_transparent || sheet.pixels.iter().any(|p| p[3] == 0);
    let levels = crate::mipmap::generate(
        sheet,
        edge.trailing_zeros(),
        Settings {
            has_transparent,
            ..s
        },
    );
    let chains = (0..frames.len())
        .map(|frame| {
            let mut chain = Vec::new();
            for level in &levels {
                let texels = (level.width * level.width) as usize;
                chain.extend(
                    level.pixels[frame * texels..(frame + 1) * texels]
                        .iter()
                        .flatten(),
                );
            }
            chain
        })
        .collect();
    (chains, edge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mipmap::Strategy;

    /// `animate.wgsl` on the CPU: `chain` (first level `edge` wide) as the
    /// full chain of a `size` layer, mixed `millis` thousandths of the way
    /// towards `next`.
    fn reference(chain: &[u8], next: &[u8], millis: u32, edge: u32, size: u32) -> Vec<u8> {
        let levels = size.trailing_zeros() + 1;
        let mut out = Vec::new();
        for level in 0..levels {
            let width = size >> level;
            for y in 0..width {
                for x in 0..width {
                    let texel = if width >= edge {
                        let scale = width / edge;
                        y / scale * edge + x / scale
                    } else {
                        4 * (edge * edge - width * width) / 3 + y * width + x
                    } as usize;
                    for c in 0..4 {
                        let (a, b) = (
                            u32::from(chain[texel * 4 + c]),
                            u32::from(next[texel * 4 + c]),
                        );
                        out.push(((a * (1000 - millis) + b * millis + 500) / 1000) as u8);
                    }
                }
            }
        }
        out
    }

    fn frame(size: u32, seed: u32) -> Vec<u8> {
        (0..size * size)
            .flat_map(|i| {
                let v = i
                    .wrapping_mul(2654435761)
                    .wrapping_add(seed.wrapping_mul(40503));
                let alpha = match v >> 29 {
                    0 => 0,
                    1 => 77,
                    _ => 255,
                };
                [(v >> 3) as u8, (v >> 11) as u8, (v >> 19) as u8, alpha]
            })
            .collect()
    }

    fn settings(native: u32, strategy: Strategy) -> Settings {
        Settings {
            native_size: native,
            strategy,
            alpha_cutoff_bias: -0.1,
            ..Default::default()
        }
    }

    #[test]
    fn source_chains_repeat_into_the_array_chain() {
        // Sprites scaled up by whole pixels, at the array's size, and of a
        // size that is not a power of two (scaled up to the next one).
        for (native, size) in [(16, 16), (16, 64), (8, 32), (32, 64), (1, 16), (24, 32)] {
            for strategy in [
                Strategy::Auto,
                Strategy::Mean,
                Strategy::Cutout,
                Strategy::StrictCutout,
                Strategy::DarkCutout,
            ] {
                for item in [false, true] {
                    let settings = [
                        Settings::default(),
                        Settings {
                            item,
                            ..settings(native, strategy)
                        },
                    ];
                    let pixels = crate::sprites::resample(&frame(native, 3), native, size);
                    let (expected, _) = super::super::mip_chain(&pixels, size, &settings, 1, 1);
                    let (chain, edge) = source_chain(&pixels, size, &settings, 1);
                    let fits = native.is_power_of_two() && native < size;
                    assert_eq!(edge, if fits { native } else { size });
                    assert_eq!(
                        reference(&chain, &chain, 0, edge, size),
                        expected,
                        "{native} in {size}, {strategy:?}, item {item}"
                    );
                }
            }
        }
    }

    #[test]
    fn real_animation_chains_repeat_into_the_array_chain() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/water_still.png")
            .exists()
        {
            return;
        }
        let mut sprites = crate::sprites::Sprites::default();
        for name in [
            "water_still",
            "water_flow",
            "lava_still",
            "fire_0",
            "nether_portal",
            "sea_lantern",
            "prismarine",
            "magma",
            "kelp",
            "straw_bed",
        ] {
            sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap();
        }
        sprites.finish();
        assert_eq!(sprites.size, 64);
        assert_eq!(sprites.animations.len(), 9);
        for animation in &sprites.animations {
            for frame in 0..animation.frame_count() {
                let pixels = animation.pixels(frame);
                let (expected, _) =
                    super::super::mip_chain(pixels, 64, &sprites.mips, animation.layer, 1);
                let (chain, edge) = source_chain(pixels, 64, &sprites.mips, animation.layer);
                assert!(edge < 64, "layer {}", animation.layer);
                assert_eq!(reference(&chain, &chain, 0, edge, 64), expected);
            }
        }
    }

    #[test]
    fn kelp_frames_share_the_sheet_mips() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/kelp.png")
            .exists()
        {
            return;
        }
        let mut sprites = crate::sprites::Sprites::default();
        for name in ["kelp", "kelp_plant", "seagrass", "tall_seagrass_top"] {
            sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap();
        }
        sprites.finish();
        // A level drawn nowhere: no texel passes the cutout's alpha test.
        let blank = |chain: &[u8], edge: u32| {
            let mut start = 0;
            let mut level = edge;
            let mut blank = Vec::new();
            while level > 0 {
                let bytes = (level * level * 4) as usize;
                let texels = &chain[start..start + bytes];
                blank.push(texels.as_chunks::<4>().0.iter().all(|p| p[3] < 128));
                start += bytes;
                level >>= 1;
            }
            blank
        };
        let mut blinked = 0;
        for animation in &sprites.animations {
            let size = sprites.size;
            let pixels: Vec<_> = (0..animation.frame_count())
                .map(|f| animation.pixels(f))
                .collect();
            let (chains, edge) = source_chains(&pixels, size, &sprites.mips, animation.layer);
            for (frame, chain) in chains.iter().enumerate() {
                // The 1x1 level has no texel square to measure coverage on
                // (vanilla leaves it unscaled); every larger level shows.
                let levels = blank(chain, edge);
                assert!(
                    !levels[..levels.len() - 1].contains(&true),
                    "layer {} frame {frame}: {levels:?}",
                    animation.layer
                );
                let (alone, _) = source_chain(pixels[frame], size, &sprites.mips, animation.layer);
                let alone = blank(&alone, edge);
                blinked += usize::from(alone[..alone.len() - 1].contains(&true));
            }
        }
        // Mipmapped frame by frame, some kelp frames vanished at the 2x2 level.
        assert!(blinked > 0);
    }

    #[test]
    fn rounding_matches_f32_mix_but_for_exact_halves() {
        // The CPU blend this replaced: f32 mix, then rounded. They differ
        // only where the exact mix is a half, which f32 rounded either way.
        for millis in 1..1000u32 {
            let t = millis as f32 / 1000.0;
            for a in 0..=255u32 {
                for b in (0..=255u32).step_by(3) {
                    let f = ((a as f32 / 255.0) * (1.0 - t) + (b as f32 / 255.0) * t) * 255.0;
                    let exact = a * (1000 - millis) + b * millis;
                    let ours = (exact + 500) / 1000;
                    if exact % 1000 != 500 {
                        assert_eq!(ours, f.round() as u32, "{a} {b} {millis}");
                    }
                }
            }
        }
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// Every layer's full mip chain, layer major.
    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Vec<Vec<u8>> {
        let layers = texture.depth_or_array_layers();
        let mut out = vec![Vec::new(); layers as usize];
        for level in 0..texture.mip_level_count() {
            let edge = texture.width() >> level;
            let padded = (edge * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(padded * edge * layers),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(edge),
                    },
                },
                wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: layers,
                },
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap();
            for (layer, image) in data.chunks((padded * edge) as usize).enumerate() {
                for row in image.chunks(padded as usize) {
                    out[layer].extend_from_slice(&row[..edge as usize * 4]);
                }
            }
        }
        out
    }

    #[test]
    fn gpu_layers_match_the_cpu_chains_every_tick() {
        layers_match_the_cpu_chains_every_tick(true);
    }

    /// Without compute shaders (OpenGL before 4.3): the same layers.
    #[test]
    fn copied_layers_match_the_cpu_chains_every_tick() {
        layers_match_the_cpu_chains_every_tick(false);
    }

    fn layers_match_the_cpu_chains_every_tick(compute: bool) {
        let Some((device, queue)) = device() else {
            return;
        };
        let size = 32;
        // Layers 1 and 3: 8-pixel sprites with and without interpolation;
        // layer 4: one at the array's size. Two layers per array.
        let clips = [
            (
                1,
                8,
                r#"{"frametime":3,"interpolate":true,"frames":[0,2,1]}"#,
            ),
            (3, 8, r#"{"frametime":2}"#),
            (4, 32, r#"{"frametime":5,"interpolate":true}"#),
        ];
        let mut mips = vec![Settings::default(); 6];
        let animations: Vec<_> = clips
            .iter()
            .map(|&(layer, native, metadata)| {
                mips[layer as usize] = settings(native, Strategy::Cutout);
                let frames = (0..3)
                    .map(|f| crate::sprites::resample(&frame(native, layer * 7 + f), native, size))
                    .collect();
                crate::sprites::test_frames(layer, frames, metadata)
            })
            .collect();
        let split = 2;
        let textures: Vec<_> = (0..3)
            .map(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: split,
                    },
                    mip_level_count: size.trailing_zeros() + 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
            .collect();
        let mut animator =
            SpriteAnimator::new(&device, &animations, size, &mips, &textures, split, compute);
        let mut uploads = Uploads::new(&device);
        let mut writes = Vec::new();
        for tick in [0, 0, 1, 2, 3, 4, 7, 7, 12, 30] {
            let mut encoder = device.create_command_encoder(&Default::default());
            let shown: Vec<_> = animator.sprites.iter().map(|s| s.shown).collect();
            animator.animate(&mut encoder, &mut uploads, tick);
            writes.push(
                animator
                    .sprites
                    .iter()
                    .zip(shown)
                    .filter(|(s, shown)| s.shown != *shown)
                    .count(),
            );
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            for animation in &animations {
                let blend = animation.blend(tick);
                let pixels: Vec<_> = (0..animation.frame_count())
                    .map(|f| animation.pixels(f))
                    .collect();
                let (chains, edge) = source_chains(&pixels, size, &mips, animation.layer);
                let (current, next) = (&chains[blend.frame], &chains[blend.next]);
                let layers = read_back(
                    &device,
                    &queue,
                    &textures[(animation.layer / split) as usize],
                );
                assert_eq!(
                    layers[(animation.layer % split) as usize],
                    reference(current, next, blend.millis, edge, size),
                    "layer {} at tick {tick}",
                    animation.layer
                );
            }
        }
        // Every sprite on the first tick, then only those whose frame or mix
        // changed since they were written: the interpolated ones every tick
        // but layer 1 at tick 30 (two 9-tick loops after 12), layer 3 when
        // its frame turns over (every 2 ticks, but 12 shows 7's frame).
        assert_eq!(writes, [3, 0, 2, 3, 2, 3, 3, 0, 2, 1]);
    }
}
