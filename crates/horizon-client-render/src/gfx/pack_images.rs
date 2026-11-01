//! A shader pack's own images (`image.<name>`) and storage buffers (`bufferObject.<n>`) on the
//! GPU: created at the render size they depend on, kept across frames, images cleared before a
//! frame when the pack asks.

use std::collections::BTreeMap;

use horizon_shaders::{BufferObject, CustomImage, Format, Kind};

pub(super) struct PackImage {
    pub(super) def: CustomImage,
    pub(super) format: wgpu::TextureFormat,
    pub(super) dimension: wgpu::TextureViewDimension,
    pub(super) view: Option<wgpu::TextureView>,
    /// Last frame's contents, for a pass that samples and stores the image (`copied`).
    pub(super) copy: Option<wgpu::TextureView>,
    pub(super) copied: bool,
    /// The contents before a compute pass that reads them so (`snapshot`), apart from `copy`,
    /// which a later pass may still sample.
    pub(super) before: Option<wgpu::TextureView>,
    pub(super) snapshotted: bool,
    /// The render size the image was made for (relative images follow it).
    made_for: [u32; 2],
}

impl PackImage {
    pub(super) fn sample_type(&self, features: wgpu::Features) -> wgpu::TextureSampleType {
        self.format
            .sample_type(None, Some(features))
            .unwrap_or(wgpu::TextureSampleType::Float { filterable: false })
    }
}

/// The images `image.*` declares.
pub(super) fn images(defs: Vec<CustomImage>) -> Vec<PackImage> {
    defs.into_iter()
        .map(|def| PackImage {
            format: format(def.format),
            dimension: if def.is_3d() {
                wgpu::TextureViewDimension::D3
            } else {
                wgpu::TextureViewDimension::D2
            },
            def,
            view: None,
            copy: None,
            copied: false,
            before: None,
            snapshotted: false,
            made_for: [0; 2],
        })
        .collect()
}

/// The wgpu format of an image's internal format: a storage format of the same channels.
pub(super) fn format(f: Format) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as T;
    let pick = |r, rg, rgba| match f.channels {
        1 => r,
        2 => rg,
        _ => rgba,
    };
    match (f.kind, f.bits) {
        (Kind::Int, 8) => pick(T::R8Sint, T::Rg8Sint, T::Rgba8Sint),
        (Kind::Int, 16) => pick(T::R16Sint, T::Rg16Sint, T::Rgba16Sint),
        (Kind::Int, _) => pick(T::R32Sint, T::Rg32Sint, T::Rgba32Sint),
        (Kind::Uint, 8) => pick(T::R8Uint, T::Rg8Uint, T::Rgba8Uint),
        (Kind::Uint, 16) => pick(T::R16Uint, T::Rg16Uint, T::Rgba16Uint),
        (Kind::Uint, _) => pick(T::R32Uint, T::Rg32Uint, T::Rgba32Uint),
        (Kind::Float, 32) => pick(T::R32Float, T::Rg32Float, T::Rgba32Float),
        (Kind::Float, _) => pick(T::R16Float, T::Rg16Float, T::Rgba16Float),
        (Kind::Snorm, _) => pick(T::R8Snorm, T::Rg8Snorm, T::Rgba8Snorm),
        (Kind::Unorm, 16) => pick(T::R16Unorm, T::Rg16Unorm, T::Rgba16Unorm),
        (Kind::Unorm, _) => pick(T::R8Unorm, T::Rg8Unorm, T::Rgba8Unorm),
    }
}

/// Whether this device can make `def`: its format's features, and a size in pixels within the
/// device's limits.
pub(super) fn makeable(device: &wgpu::Device, def: &CustomImage) -> Result<(), String> {
    let format = format(def.format);
    let missing = format.required_features() - device.features();
    if !missing.is_empty() {
        return Err(format!(
            "{format:?} needs {missing:?}, which this GPU lacks"
        ));
    }
    let limits = device.limits();
    let largest = if def.is_3d() {
        limits.max_texture_dimension_3d
    } else {
        limits.max_texture_dimension_2d
    };
    let [width, height, depth] = def.extent([1, 1]);
    let relative = matches!(def.size, horizon_shaders::ImageSize::Relative(_));
    if !relative && width.max(height).max(depth) > largest {
        return Err(format!(
            "{width}x{height}x{depth} is larger than this GPU's {largest}"
        ));
    }
    Ok(())
}

/// Makes the images that are missing or sized for another render size; whether it made any.
pub(super) fn resize_images(
    device: &wgpu::Device,
    images: &mut [PackImage],
    size: [u32; 2],
) -> bool {
    let largest = device.limits().max_texture_dimension_2d;
    let mut made = false;
    for image in images {
        let relative = matches!(image.def.size, horizon_shaders::ImageSize::Relative(_));
        if image.view.is_some() && (!relative || image.made_for == size) {
            continue;
        }
        made = true;
        // A relative image larger than the render size stops at the device's limit.
        let [width, height, depth] = image.def.extent(size);
        let [width, height] = [width.min(largest), height.min(largest)];
        let make = |usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(&image.def.name),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: depth,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: if image.dimension == wgpu::TextureViewDimension::D3 {
                        wgpu::TextureDimension::D3
                    } else {
                        wgpu::TextureDimension::D2
                    },
                    format: image.format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        // Atomics on a texture need it made for them (Metal ignores them otherwise).
        let atomic = device.features().contains(wgpu::Features::TEXTURE_ATOMIC)
            && matches!(
                image.format,
                wgpu::TextureFormat::R32Sint | wgpu::TextureFormat::R32Uint
            );
        let atomic = if atomic {
            wgpu::TextureUsages::STORAGE_ATOMIC
        } else {
            wgpu::TextureUsages::empty()
        };
        image.view = Some(make(
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | atomic,
        ));
        image.copy = image
            .copied
            .then(|| make(wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::STORAGE_BINDING));
        image.before = image
            .snapshotted
            .then(|| make(wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::STORAGE_BINDING));
        image.made_for = size;
    }
    made
}

/// The most images a pack may have.
pub(super) const IMAGES: usize = 64;

/// Copies `image` as it is before a compute pass whose loads read it so.
pub(super) fn snapshot(encoder: &mut wgpu::CommandEncoder, image: &PackImage) {
    if let (Some(view), Some(before)) = (&image.view, &image.before) {
        encoder.copy_texture_to_texture(
            view.texture().as_image_copy(),
            before.texture().as_image_copy(),
            view.texture().size(),
        );
    }
}

/// Copies the images of `mask` (bit n: image n), which a pass samples and stores, for that pass
/// to sample.
pub(super) fn copy(encoder: &mut wgpu::CommandEncoder, images: &[PackImage], mask: u64) {
    for (_, image) in images
        .iter()
        .enumerate()
        .filter(|(k, _)| mask & 1 << k != 0)
    {
        if let (Some(view), Some(copy)) = (&image.view, &image.copy) {
            encoder.copy_texture_to_texture(
                view.texture().as_image_copy(),
                copy.texture().as_image_copy(),
                view.texture().size(),
            );
        }
    }
}

/// Clears the images the pack clears before each frame: with `clear_texture` where the device
/// has it (`clear`), else by copying from `zeros`, a buffer of zeros made large enough.
pub(super) fn clear(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    images: &[PackImage],
    clear: bool,
    zeros: &mut Option<wgpu::Buffer>,
) {
    for view in images
        .iter()
        .filter(|i| i.def.clear)
        .filter_map(|i| i.view.as_ref())
    {
        let texture = view.texture();
        if clear {
            encoder.clear_texture(texture, &Default::default());
            continue;
        }
        let size = texture.size();
        let texel = u64::from(texture.format().block_copy_size(None).unwrap_or(4));
        let row = (u64::from(size.width) * texel).next_multiple_of(256);
        // Zeros for a slice of rows at a time, the whole image taken in slices, so a large
        // volume doesn't need a buffer of its size.
        let rows = (ZEROS / row).clamp(1, u64::from(size.height)) as u32;
        let bytes = row * u64::from(rows);
        if zeros.as_ref().is_none_or(|z| z.size() < bytes) {
            // New buffers start zeroed.
            *zeros = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Shader pack image zeros"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
        }
        let Some(buffer) = zeros.as_ref() else {
            continue;
        };
        for z in 0..size.depth_or_array_layers {
            for y in (0..size.height).step_by(rows as usize) {
                let height = rows.min(size.height - y);
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row as u32),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y, z },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: size.width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

/// The most bytes of zeros kept to clear images without `CLEAR_TEXTURE`.
const ZEROS: u64 = 4 << 20;

/// The pack's storage buffers, bound in set 4.
pub(super) struct Buffers {
    pub(super) layout: wgpu::BindGroupLayout,
    objects: BTreeMap<u32, BufferObject>,
    pub(super) group: Option<wgpu::BindGroup>,
    buffers: Vec<wgpu::Buffer>,
    made_for: [u32; 2],
}

impl Buffers {
    /// The layout of `objects`, bound in `stages`.
    pub(super) fn new(
        device: &wgpu::Device,
        objects: BTreeMap<u32, BufferObject>,
        stages: wgpu::ShaderStages,
    ) -> Self {
        let entries: Vec<_> = objects
            .keys()
            .map(|&binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: stages,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack buffers"),
            entries: &entries,
        });
        Self {
            layout,
            objects,
            group: None,
            buffers: Vec::new(),
            made_for: [0; 2],
        }
    }

    /// Makes the buffers (again, when one depends on the render size and it changed): their
    /// contents start zeroed and persist across frames.
    pub(super) fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let relative = self
            .objects
            .values()
            .any(|o| matches!(o, BufferObject::PerPixel { .. }));
        if self.group.is_some() && (!relative || self.made_for == size) {
            return;
        }
        let limits = device.limits();
        let largest = limits
            .max_buffer_size
            .min(limits.max_storage_buffer_binding_size)
            & !3;
        self.buffers = self
            .objects
            .iter()
            .map(|(binding, o)| {
                let wanted = o.size(size).next_multiple_of(4);
                if wanted > largest {
                    log::warn!(
                        "shader pack: bufferObject.{binding} wants {wanted} bytes, this device \
                         binds at most {largest}"
                    );
                }
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Shader pack buffer"),
                    size: wanted.min(largest),
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let entries: Vec<_> = self
            .objects
            .keys()
            .zip(&self.buffers)
            .map(|(&binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            })
            .collect();
        self.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack buffers"),
            layout: &self.layout,
            entries: &entries,
        }));
        self.made_for = size;
    }

    /// Whether the pack declares buffer `binding`.
    pub(super) fn has(&self, binding: u32) -> bool {
        self.objects.contains_key(&binding)
    }
}

/// A debug readback of one of the pack's images (`HORIZON_SHADER_PACK_DUMP=<name>`): every 60
/// frames, how many of its texels are not zero, logged.
pub(super) struct Dump {
    name: String,
    frame: u32,
    pending: Option<(wgpu::Buffer, usize, [u32; 3], u32)>,
}

impl Dump {
    pub(super) fn new() -> Option<Self> {
        let name = std::env::var("HORIZON_SHADER_PACK_DUMP").ok()?;
        Some(Self {
            name,
            frame: 0,
            pending: None,
        })
    }

    /// Reads last frame's copy, once its commands ran.
    pub(super) fn read(&mut self, device: &wgpu::Device) {
        let Some((buffer, texel, [w, h, d], row)) = self.pending.take() else {
            return;
        };
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let Ok(data) = buffer.slice(..).get_mapped_range() else {
            return;
        };
        let (mut set, mut total) = (0usize, 0usize);
        for z in 0..d as usize {
            for y in 0..h as usize {
                let start = (z * h as usize + y) * row as usize;
                for x in 0..w as usize {
                    let at = start + x * texel;
                    total += 1;
                    if data[at..at + texel].iter().any(|&b| b != 0) {
                        set += 1;
                    }
                }
            }
        }
        log::info!(
            "shader pack image {}: {set} of {total} texels set",
            self.name
        );
    }

    /// Copies the image for `read` every 60 frames.
    pub(super) fn copy(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        images: &[PackImage],
    ) {
        self.frame += 1;
        if !self.frame.is_multiple_of(60) {
            return;
        }
        let Some(image) = images.iter().find(|i| i.def.name == self.name) else {
            return;
        };
        let Some(view) = &image.view else { return };
        let size = view.texture().size();
        let texel = image.format.block_copy_size(None).unwrap_or(4) as usize;
        let row = (size.width as usize * texel).next_multiple_of(256) as u32;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack image dump"),
            size: u64::from(row) * u64::from(size.height) * u64::from(size.depth_or_array_layers),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            view.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        let dims = [size.width, size.height, size.depth_or_array_layers];
        self.pending = Some((buffer, texel, dims, row));
    }
}
