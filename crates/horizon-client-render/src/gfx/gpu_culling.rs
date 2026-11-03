//! GPU culling: the frame's near solid section draws listed by a compute pass
//! and drawn with one `multi_draw_indexed_indirect_count` per arena page,
//! where the device can draw a count the GPU wrote (`MULTI_DRAW_INDIRECT_COUNT`:
//! DX12, Vulkan 1.2 or `VK_KHR_draw_indirect_count`). Listing those draws on
//! the CPU, a few per section by the sides it shows the eye, and grouping them
//! by page costs the render thread time in proportion to the sections in view:
//! about a fifth of its frame at render distance 64.
//!
//! Every resident section with solid quads has a record here, under an id it
//! keeps for as long as it stays (`SectionGpu::gpu_id`; resident slots move on
//! every removal). The CPU's walk still decides what is drawn: it marks the ids
//! of the whole sections it keeps within the render distance
//! (`GpuCulling::choose`), and the pass draws exactly those, all the solid
//! quads of each in one draw (the pipelines cull the faces turned away), with
//! the origin the walk gives it: `resident::origin`'s f32 bits, copied, never
//! computed on the GPU. Sections fading in or out, sections past the distance
//! and translucent quads stay on the CPU's lists. The pass `apply`s the records
//! that changed, then `cull` keeps the chosen ones and counts them per page and
//! block of ids, `scan` turns the counts into where each block's draws start,
//! and `scatter` writes every block's draws there in id order. Appended with
//! atomics alone, a page's draws would come in the order the GPU happens to
//! schedule them, and where faces of two sections meet at exactly the same
//! depth the later draw keeps the pixel, so such pixels would flicker; in id
//! order they differ from the CPU's front-to-back order only there, and stay
//! put.
//!
//! This is not `cull` (occlusion culling, `HORIZON_GPU_CULL`), which tests the
//! CPU's lists against phase 1's depth: frames drawn from here leave it off.
//!
//! wgpu checks a count draw only against the size of its buffers, never draw by
//! draw, so every draw here is in bounds by construction: its index count is
//! its section's own solid quads times 6, which the quad index buffer covers
//! (it grows before a larger mesh uploads); its base vertex starts its
//! section's own range of the page it is drawn from; its first instance is an
//! id below the table's end, which `origins` holds; and each page's count is
//! clamped to its region, which `scatter` never writes past. A region has room
//! for every record of its page, so in practice nothing is clamped.
use super::{
    SectionKey,
    arena::{Arena, try_create},
    pack_timing::PassTiming,
    resident,
    submit::Queue,
    upload::Uploads,
};
use glam::DVec3;

/// One section (`gpu_culling.wgsl`'s `Section`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Record {
    /// Section x, y (from the level's bottom), z and arena page; -1: a free id.
    key: [i32; 4],
    /// The first vertex of its solid quads (a draw's base vertex, an i32: a
    /// page holds fewer vertices), its solid quads, then two words kept at 0.
    a: [u32; 4],
}

impl Record {
    const FREE: Self = Self {
        key: [0, 0, 0, -1],
        a: [0; 4],
    };
    const BYTES: u64 = std::mem::size_of::<Self>() as u64;
    /// `quads` solid quads from vertex `start` of arena page `page`.
    pub fn new(key: SectionKey, page: u32, start: u64, quads: u32) -> Self {
        Self {
            key: [key.0, key.1, key.2, page as i32],
            a: [start as u32, quads, 0, 0],
        }
    }
    fn page(&self) -> Option<u32> {
        u32::try_from(self.key[3]).ok()
    }
}

/// A changed record, for `apply` (`gpu_culling.wgsl`'s `Update`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Update {
    /// Its id, and padding to the record's alignment.
    id: [u32; 4],
    record: Record,
}

impl Update {
    const BYTES: u64 = std::mem::size_of::<Self>() as u64;
}

/// `gpu_culling.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// Ids the pass tests, and the `Update`s `apply` writes.
    ids: u32,
    updates: u32,
    /// Arena pages the counts and regions cover (at least one), and where the
    /// regions and the chosen ids' bits start in `tables`, in words.
    pages: u32,
    regions: u32,
    mask: u32,
    pad: [u32; 3],
    /// The tables' first column: x, 0, z, 0.
    first: [i32; 4],
    /// Their entries along x, y and z, and 0.
    cells: [u32; 4],
}

/// Counts before the pages' (the shader's `STATS`): the sections `cull` kept,
/// and the draws, fewer only where a region was short.
const STATS: u64 = 2;
/// Ids per block (the shader's `BLOCK`), and the workgroup of the passes over ids.
const WORKGROUP: u32 = 64;
/// Workgroups along x when there are more (the shader's `ROW`).
const ROW: u32 = 65535;
/// Changed records past which the whole table is written at once: their
/// updates would pass a megabyte, the staging belt's chunk (`upload::Uploads`).
const DIRTY_LIMIT: usize = (1 << 20) / Update::BYTES as usize;

/// Tests read the counts, draws and origins back.
const TEST_READ: wgpu::BufferUsages = if cfg!(test) {
    wgpu::BufferUsages::COPY_SRC
} else {
    wgpu::BufferUsages::empty()
};

/// What a frame's pass works from.
pub(super) struct FrameInputs {
    pub camera: DVec3,
    /// The level's lowest block y and its height in sections.
    pub min_y: i32,
    pub sections: i32,
    /// The render distance in columns, as the walk takes it.
    pub radius: i32,
}

/// Whether GPU culling is offered: where the device draws a count the GPU wrote
/// and runs compute shaders, where its multi-draws offset instance-rate origins
/// by each draw's first instance (`multi_draw`; Intel Gen11/Gen12's Vulkan
/// driver gets that wrong, `flat::is_broken_intel_vulkan`, tested here too for
/// when flat draws are off), where Translucent Batching is
/// (`translucent_batchable`), and where vanilla frames never draw flat (flat
/// draws off, or for a shader pack's terrain alone): flat draws copy the CPU's
/// lists.
pub(super) fn supported(
    info: &wgpu::AdapterInfo,
    features: wgpu::Features,
    downlevel: wgpu::DownlevelFlags,
    limits: &wgpu::Limits,
    multi_draw: bool,
    flat_min_sections: Option<usize>,
) -> bool {
    super::translucent_batchable(info, multi_draw)
        && flat_min_sections.is_none_or(|min| min == usize::MAX)
        && features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
        && downlevel.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        // The pass binds 7; the eighth keeps room for cave culling's graph.
        && limits.max_storage_buffers_per_shader_stage >= 8
        && !super::flat::is_broken_intel_vulkan(info)
}

/// A buffer that is recreated larger when it must hold more.
struct Grown {
    buffer: wgpu::Buffer,
    bytes: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl Grown {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        bytes: u64,
        usage: wgpu::BufferUsages,
    ) -> Option<Self> {
        Some(Self {
            buffer: try_create(device, label, bytes, usage)?,
            bytes,
            label,
            usage,
        })
    }
    /// Room for `bytes`, in a power of two within the device's limits:
    /// `Some(true)` when the buffer was made anew (its contents lost), `None`
    /// past the limits or when the memory for it is missing.
    fn reserve(&mut self, device: &wgpu::Device, bytes: u64) -> Option<bool> {
        if bytes <= self.bytes {
            return Some(false);
        }
        let limits = device.limits();
        let mut limit = limits.max_buffer_size;
        if self.usage.contains(wgpu::BufferUsages::STORAGE) {
            limit = limit.min(limits.max_storage_buffer_binding_size);
        }
        if bytes > limit {
            return None;
        }
        let size = bytes.next_power_of_two().min(limit & !3);
        self.buffer = try_create(device, self.label, size, self.usage)?;
        self.bytes = size;
        Some(true)
    }
}

/// The section table as the CPU keeps it, by id, and what changed since
/// `GpuCulling::prepare` last wrote it.
#[derive(Default)]
pub(super) struct Records {
    records: Vec<Record>,
    /// Free ids below the table's end, a bit each (64 a word, lowest bit first).
    free: Vec<u64>,
    /// No free id is below `64 *` this.
    first_free: usize,
    /// Ids changed since, unless there were too many to list (`all_dirty`).
    dirty: Vec<u32>,
    all_dirty: bool,
    /// Per arena page, the records on it.
    page_sections: Vec<u32>,
}

impl Records {
    /// A resident section with solid quads, `quads` of them from vertex `start`
    /// of arena page `page`: its id, the lowest free one or a new one.
    pub fn insert(&mut self, key: SectionKey, page: u32, start: u64, quads: u32) -> u32 {
        let id = match self.take_free() {
            Some(id) => id,
            None => {
                self.records.push(Record::FREE);
                self.records.len() as u32 - 1
            }
        };
        self.records[id as usize] = Record::new(key, page, start, quads);
        self.add_to(page);
        self.mark(id);
        id
    }
    /// The section with `id` went.
    pub fn remove(&mut self, id: u32) {
        self.remove_all([id]);
    }
    /// The sections with `ids` went, a bit set for each (a teleport frees tens of
    /// thousands at once, `Renderer::remove_columns_at_once`). Their records stay
    /// on the GPU until their ids are taken again: the walk never chooses a free
    /// id (`choose`), so nothing reads them there.
    pub fn remove_all(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            let page = self.records.get(id as usize).and_then(Record::page);
            debug_assert!(page.is_some(), "no record {id} to remove");
            let Some(page) = page else {
                continue;
            };
            self.page_sections[page as usize] -= 1;
            self.records[id as usize] = Record::FREE;
            let word = id as usize / 64;
            if self.free.len() <= word {
                self.free.resize(word + 1, 0);
            }
            self.free[word] |= 1 << (id % 64);
            self.first_free = self.first_free.min(word);
        }
        // Free ids at the end leave the table, so the pass tests fewer.
        while self.records.last().is_some_and(|r| r.page().is_none()) {
            self.records.pop();
            let id = self.records.len();
            self.free[id / 64] &= !(1 << (id % 64));
        }
        self.free.truncate(self.records.len().div_ceil(64));
        self.first_free = self.first_free.min(self.free.len());
    }
    /// The lowest free id, taken.
    fn take_free(&mut self) -> Option<u32> {
        while let Some(&word) = self.free.get(self.first_free) {
            if word != 0 {
                self.free[self.first_free] = word & (word - 1);
                return Some((self.first_free * 64) as u32 + word.trailing_zeros());
            }
            self.first_free += 1;
        }
        None
    }
    /// The section with `id` moved to vertex `start` of page `page`, its quads
    /// with it (`move_off_page`).
    pub fn relocate(&mut self, id: u32, page: u32, start: u64) {
        let from = self.records.get(id as usize).and_then(Record::page);
        debug_assert!(from.is_some(), "no record {id} to move");
        let Some(from) = from else {
            return;
        };
        let record = &mut self.records[id as usize];
        record.key[3] = page as i32;
        record.a[0] = start as u32;
        self.page_sections[from as usize] -= 1;
        self.add_to(page);
        self.mark(id);
    }
    /// Every section went.
    pub fn clear(&mut self) {
        self.records.clear();
        self.free.clear();
        self.first_free = 0;
        self.dirty = Vec::new();
        self.all_dirty = false;
        self.page_sections.clear();
    }
    /// The sections with a record.
    #[cfg(any(test, debug_assertions))]
    pub fn live(&self) -> usize {
        let free: u32 = self.free.iter().map(|word| word.count_ones()).sum();
        self.records.len() - free as usize
    }
    /// The record of the section with `id`; `None` for a free id.
    #[cfg(any(test, debug_assertions))]
    pub fn at(&self, id: u32) -> Option<&Record> {
        self.records.get(id as usize).filter(|r| r.page().is_some())
    }
    #[cfg(any(test, debug_assertions))]
    pub fn page_sections(&self) -> &[u32] {
        &self.page_sections
    }
    /// The table's end: the ids the pass tests.
    fn ids(&self) -> u32 {
        self.records.len() as u32
    }
    fn add_to(&mut self, page: u32) {
        let page = page as usize;
        if self.page_sections.len() <= page {
            self.page_sections.resize(page + 1, 0);
        }
        self.page_sections[page] += 1;
    }
    fn mark(&mut self, id: u32) {
        if self.all_dirty {
            return;
        }
        if self.dirty.len() >= DIRTY_LIMIT {
            self.all_dirty = true;
            self.dirty = Vec::new();
        } else {
            self.dirty.push(id);
        }
    }
}

/// The pass, its buffers and the records it draws from: made when GPU culling
/// is switched on, dropped when it is switched off.
pub(super) struct GpuCulling {
    /// `gpu_culling.wgsl`'s entry points: apply, cull, scan, scatter.
    pipelines: [wgpu::ComputePipeline; 4],
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sections: Grown,
    tables: Grown,
    updates: Grown,
    counts: Grown,
    draws: Grown,
    origins: Grown,
    runs: Grown,
    /// Dropped as soon as one of its buffers is made anew.
    group: Option<wgpu::BindGroup>,
    /// What `sections` holds once `prepare` writes it; a field of its own, so
    /// that the arena's compaction moves records while it borrows the
    /// renderer's other fields.
    pub records: Records,
    /// This frame's pages with records: (page, first draw of its region, the
    /// region's size).
    pages: Vec<(u32, u32, u32)>,
    /// This frame's ids to test, records to `apply`, and pages the counts
    /// cover (at least one).
    ids: u32,
    n_updates: u32,
    pass_pages: u32,
    /// The ids the walk chose this frame, a bit each, where they go in
    /// `tables` (in words), and how many.
    mask: Vec<u32>,
    mask_at: u64,
    chosen: u32,
    /// This frame's updates and table words, kept for their room.
    scratch: Vec<Update>,
    words: Vec<u32>,
}

impl GpuCulling {
    /// `None` when the device fails to make the pipelines or the memory for the
    /// buffers is missing (caught, not raised: raised errors are fatal).
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage =
            |binding, read_only| entry(binding, wgpu::BufferBindingType::Storage { read_only });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GPU culling"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                storage(1, false),
                storage(2, true),
                storage(3, true),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU culling"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_culling.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GPU culling"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipelines = ["apply", "cull", "scan", "scatter"].map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("GPU culling"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("Horizon renderer: GPU culling unavailable ({e})");
            return None;
        }
        use wgpu::BufferUsages as U;
        Some(Self {
            pipelines,
            layout,
            params: try_create(
                device,
                "GPU culling params",
                std::mem::size_of::<Params>() as u64,
                U::UNIFORM | U::COPY_DST,
            )?,
            sections: Grown::new(
                device,
                "GPU culling sections",
                1024 * Record::BYTES,
                U::STORAGE | U::COPY_DST,
            )?,
            tables: Grown::new(
                device,
                "GPU culling tables",
                1 << 14,
                U::STORAGE | U::COPY_DST,
            )?,
            updates: Grown::new(
                device,
                "GPU culling updates",
                1024 * Update::BYTES,
                U::STORAGE | U::COPY_DST,
            )?,
            counts: Grown::new(
                device,
                "GPU culling counts",
                (STATS + 64) * 4,
                U::STORAGE | U::INDIRECT | U::COPY_DST | TEST_READ,
            )?,
            draws: Grown::new(
                device,
                "GPU culling draws",
                4096 * 20,
                U::STORAGE | U::INDIRECT | TEST_READ,
            )?,
            origins: Grown::new(
                device,
                "GPU culling origins",
                1024 * 16,
                U::VERTEX | U::STORAGE | TEST_READ,
            )?,
            runs: Grown::new(device, "GPU culling runs", 1024 * 4, U::STORAGE)?,
            group: None,
            records: Records::default(),
            pages: Vec::new(),
            ids: 0,
            n_updates: 0,
            pass_pages: 1,
            mask: Vec::new(),
            mask_at: 0,
            chosen: 0,
            scratch: Vec::new(),
            words: Vec::new(),
        })
    }

    /// Records the frame's uploads into `encoder` and clears its counts, before
    /// the walk: the records that changed (through the queue when the whole
    /// table goes), the origins along each axis, the pages' regions and the
    /// parameters. Then the walk `choose`s, and `upload_choices` and `dispatch`
    /// come before the world pass `draw`s. False when a buffer fails to grow:
    /// nothing is written, and nothing of the frame is dispatched or drawn.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &Queue,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        input: &FrameInputs,
    ) -> bool {
        self.pages.clear();
        self.ids = 0;
        self.n_updates = 0;
        self.prepare_buffers(device, queue, uploads, encoder, input)
            .is_some()
    }

    fn prepare_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &Queue,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        input: &FrameInputs,
    ) -> Option<()> {
        // Every buffer grows before anything is written. One made anew drops
        // the bind group at once, and a new section table needs every record:
        // a failure later keeps both for the next frame.
        let ids = self.records.ids();
        if self
            .sections
            .reserve(device, u64::from(ids) * Record::BYTES)?
        {
            self.group = None;
            self.records.all_dirty = true;
        }
        // The changed records, each once; ids past the end left with the
        // records they held.
        self.scratch.clear();
        if !self.records.all_dirty {
            let Records { records, dirty, .. } = &mut self.records;
            dirty.sort_unstable();
            dirty.dedup();
            let changed = dirty.iter().filter(|&&id| id < ids);
            self.scratch.extend(changed.map(|&id| Update {
                id: [id, 0, 0, 0],
                record: records[id as usize],
            }));
        }
        // `tables`: the origins of the columns within the distance along x,
        // of the level's sections along y and of the columns along z, then
        // each page's region, then the chosen ids' bits.
        let radius = input.radius.max(0);
        let side = 2 * radius + 1;
        let height = input.sections.max(0);
        let pass_pages = self.records.page_sections.len().max(1) as u32;
        let regions = (2 * side + height) as u32;
        let mask_at = regions + 2 * pass_pages;
        let mask_words = ids.div_ceil(32);
        // A region has room for a draw of every record on its page.
        let total: u32 = self.records.page_sections.iter().sum();
        // The statistics, each page's count, then each block's per page.
        let count_words = STATS + u64::from(pass_pages) * (1 + u64::from(ids.div_ceil(WORKGROUP)));
        for (buffer, bytes) in [
            (&mut self.updates, self.scratch.len() as u64 * Update::BYTES),
            (&mut self.origins, u64::from(ids) * 16),
            (&mut self.runs, u64::from(ids) * 4),
            (&mut self.tables, u64::from(mask_at + mask_words) * 4),
            (&mut self.counts, count_words * 4),
            (&mut self.draws, u64::from(total.max(1)) * 20),
        ] {
            if buffer.reserve(device, bytes)? {
                self.group = None;
            }
        }
        if self.records.all_dirty {
            // The whole table, after it grew or more records changed than
            // updates take (GPU culling switched on in a loaded world, a
            // teleport). Through the queue, which waits for the frame on the
            // submit thread once, rather than the belt, which would keep a
            // staging buffer this large for good.
            if ids > 0 {
                queue.write_buffer(
                    &self.sections.buffer,
                    0,
                    bytemuck::cast_slice(&self.records.records),
                );
            }
        } else {
            uploads.buffer(
                encoder,
                &self.updates.buffer,
                0,
                bytemuck::cast_slice(&self.scratch),
            );
        }
        self.records.dirty.clear();
        self.records.all_dirty = false;
        // `resident::origin(..).as_vec3()` takes each component from its axis
        // alone: the walk's own expression, so the same bits.
        let (cx, cz) = resident::camera_column(input.camera);
        let (x0, z0) = (cx - radius, cz - radius);
        let origin = |key| resident::origin(key, input.min_y, input.camera).as_vec3();
        self.words.clear();
        let words = &mut self.words;
        words.extend((x0..x0 + side).map(|x| origin((x, 0, 0)).x.to_bits()));
        words.extend((0..height).map(|y| origin((0, y, 0)).y.to_bits()));
        words.extend((z0..z0 + side).map(|z| origin((0, 0, z)).z.to_bits()));
        let (sizes, mut first) = (&self.records.page_sections, 0);
        for page in 0..pass_pages {
            let size = sizes.get(page as usize).copied().unwrap_or(0);
            words.extend([first, size]);
            if size > 0 {
                self.pages.push((page, first, size));
            }
            first += size;
        }
        uploads.buffer(
            encoder,
            &self.tables.buffer,
            0,
            bytemuck::cast_slice(&self.words),
        );
        let params = Params {
            ids,
            updates: self.scratch.len() as u32,
            pages: pass_pages,
            regions,
            mask: mask_at,
            first: [x0, 0, z0, 0],
            cells: [side as u32, height as u32, side as u32, 0],
            ..Default::default()
        };
        uploads.buffer(encoder, &self.params, 0, bytemuck::bytes_of(&params));
        encoder.clear_buffer(&self.counts.buffer, 0, Some(count_words * 4));
        if self.group.is_none() {
            self.group = Some(self.bind(device));
        }
        self.ids = ids;
        self.n_updates = params.updates;
        self.pass_pages = pass_pages;
        self.mask.clear();
        self.mask.resize(mask_words as usize, 0);
        self.mask_at = u64::from(mask_at);
        self.chosen = 0;
        Some(())
    }

    fn bind(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU culling"),
            layout: &self.layout,
            entries: &[
                entry(0, &self.params),
                entry(1, &self.sections.buffer),
                entry(2, &self.tables.buffer),
                entry(3, &self.updates.buffer),
                entry(4, &self.counts.buffer),
                entry(5, &self.draws.buffer),
                entry(6, &self.origins.buffer),
                entry(7, &self.runs.buffer),
            ],
        })
    }

    /// The walk keeps the section with `id`, whole and within the render
    /// distance: the pass draws it this frame, and the CPU does not.
    pub fn choose(&mut self, id: u32) {
        let (word, bit) = (id as usize / 32, 1 << (id % 32));
        debug_assert!(
            id < self.ids && self.mask[word] & bit == 0,
            "id {id} chosen twice or past the table's {}",
            self.ids
        );
        self.mask[word] |= bit;
        self.chosen += 1;
    }

    /// The walk's choices, after it and before `dispatch`.
    pub fn upload_choices(&self, uploads: &mut Uploads, encoder: &mut wgpu::CommandEncoder) {
        uploads.buffer(
            encoder,
            &self.tables.buffer,
            self.mask_at * 4,
            bytemuck::cast_slice(&self.mask),
        );
    }

    /// The sections chosen this frame.
    pub fn chosen(&self) -> u32 {
        self.chosen
    }

    /// The pass, after `upload_choices`: the changed records into the table,
    /// then the chosen sections' draws.
    pub fn dispatch(&self, encoder: &mut wgpu::CommandEncoder, timing: Option<&mut PassTiming>) {
        let Some(group) = self.group.as_ref().filter(|_| self.ids > 0) else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GPU culling"),
            timestamp_writes: timing.and_then(|t| t.compute_writes("GPU culling")),
        });
        pass.set_bind_group(0, group, &[]);
        let [apply, cull, scan, scatter] = &self.pipelines;
        if self.n_updates > 0 {
            pass.set_pipeline(apply);
            pass.dispatch_workgroups(self.n_updates.div_ceil(WORKGROUP), 1, 1);
        }
        let blocks = self.ids.div_ceil(WORKGROUP);
        pass.set_pipeline(cull);
        pass.dispatch_workgroups(blocks.min(ROW), blocks.div_ceil(ROW), 1);
        pass.set_pipeline(scan);
        pass.dispatch_workgroups(self.pass_pages, 1, 1);
        pass.set_pipeline(scatter);
        pass.dispatch_workgroups(blocks.min(ROW), blocks.div_ceil(ROW), 1);
    }

    /// The chosen sections' solid draws, a count draw per page, with the quad
    /// indices bound. Leaves the origins at vertex slot 1.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, arena: &Arena) {
        if self.pages.is_empty() {
            return;
        }
        pass.set_vertex_buffer(1, self.origins.buffer.slice(..));
        for &(page, first, size) in &self.pages {
            pass.set_vertex_buffer(0, arena.buffer(page).slice(..));
            pass.multi_draw_indexed_indirect_count(
                &self.draws.buffer,
                u64::from(first) * 20,
                &self.counts.buffer,
                (STATS + u64::from(page)) * 4,
                size,
            );
        }
    }

    /// Draw calls `draw` makes.
    pub fn draw_calls(&self) -> usize {
        self.pages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::in_view_distance;
    use std::collections::BTreeMap;

    /// xorshift64*.
    struct Rng(u64);
    impl Rng {
        fn bits(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u32) -> u32 {
            (self.bits() >> 33) as u32 % n
        }
        fn range(&mut self, low: i32, high: i32) -> i32 {
            low + self.below((high - low + 1) as u32) as i32
        }
        fn unit(&mut self) -> f32 {
            (self.bits() >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    /// Where a section's solid quads are: key, arena page, first vertex, quads.
    type Placed = (SectionKey, u32, u64, u32);

    /// Ids, page counts and changes against a plain list of what each id holds:
    /// the lowest free id comes back, free ids at the end leave, counts follow
    /// moves (to pages past the last too), and more changes than updates take
    /// mark the whole table.
    #[test]
    fn records_keep_ids_page_counts_and_moves() {
        let mut records = Records::default();
        let a = records.insert((0, 0, 0), 0, 0, 1);
        let b = records.insert((1, 0, 0), 0, 256, 2);
        records.relocate(b, 5, 512);
        assert_eq!(records.page_sections(), [1, 0, 0, 0, 0, 1]);
        assert_eq!(records.at(b), Some(&Record::new((1, 0, 0), 5, 512, 2)));
        records.remove(a);
        assert_eq!((records.ids(), records.live(), records.at(a)), (2, 1, None));
        assert_eq!(records.insert((2, 0, 0), 1, 0, 3), a);
        records.remove(b);
        assert_eq!(
            (records.ids(), records.page_sections()),
            (1, &[0, 1, 0, 0, 0, 0][..])
        );

        // The free ids by their bits, which must be exactly the free records below the
        // table's end, none of them below `first_free`.
        let free_ids = |r: &Records| -> Vec<usize> {
            let set = |id: usize| {
                r.free
                    .get(id / 64)
                    .is_some_and(|&w| (w >> (id % 64)) & 1 == 1)
            };
            let span = r.records.len().max(r.free.len() * 64);
            let free = |id: usize| r.records.get(id).is_some_and(|x| x.page().is_none());
            assert!((0..span).all(|id| set(id) == free(id)), "free bits");
            assert!(
                r.free[..r.first_free.min(r.free.len())]
                    .iter()
                    .all(|&w| w == 0)
            );
            (0..span).filter(|&id| set(id)).collect()
        };
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        // What each id holds; `None`: a free id.
        let mut model: Vec<Option<Placed>> = vec![Some(((2, 0, 0), 1, 0, 3))];
        for step in 0..4_000 {
            let live: Vec<u32> = (0..model.len() as u32)
                .filter(|&id| model[id as usize].is_some())
                .collect();
            let (page, start) = (
                rng.below(4 + step / 500),
                u64::from(rng.below(1 << 20)) * 256,
            );
            match rng.below(5) {
                op if op < 2 || live.is_empty() => {
                    let key = (rng.range(-9, 9), rng.range(0, 23), rng.range(-9, 9));
                    let quads = 1 + rng.below(500);
                    let lowest = model
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(model.len());
                    assert_eq!(records.insert(key, page, start, quads) as usize, lowest);
                    if lowest == model.len() {
                        model.push(None);
                    }
                    model[lowest] = Some((key, page, start, quads));
                }
                op => {
                    let id = live[rng.below(live.len() as u32) as usize];
                    let held = model[id as usize].as_mut().expect("a live id");
                    if op < 4 {
                        records.relocate(id, page, start);
                        (held.1, held.2) = (page, start);
                    } else {
                        records.remove(id);
                        model[id as usize] = None;
                        while model.last() == Some(&None) {
                            model.pop();
                        }
                    }
                }
            }
            assert_eq!(records.ids() as usize, model.len(), "step {step}");
            assert_eq!(records.live(), model.iter().flatten().count());
            let mut pages = vec![0; records.page_sections().len()];
            for (id, held) in model.iter().enumerate() {
                let record = held.map(|(key, page, start, quads)| {
                    pages[page as usize] += 1;
                    Record::new(key, page, start, quads)
                });
                assert_eq!(
                    records.at(id as u32),
                    record.as_ref(),
                    "step {step}, id {id}"
                );
            }
            assert_eq!(records.page_sections(), pages, "step {step}");
            free_ids(&records);
        }

        // A few, then many at once, as a teleport drops them (onto free ids, past one
        // at the table's new end): the table removing each leaves.
        let (mut each, mut all) = (Records::default(), Records::default());
        for n in 0..1_000 {
            let (key, page, start) = ((n % 31, n % 24, n / 31), n as u32 % 3, n as u64 * 256);
            assert_eq!(
                each.insert(key, page, start, 1),
                all.insert(key, page, start, 1)
            );
        }
        let gone: Vec<u32> = (0..1_000).filter(|id| id % 3 != 1 || *id >= 900).collect();
        for ids in [&[1u32, 4, 898][..], &gone[..]] {
            for &id in ids {
                each.remove(id);
            }
            all.remove_all(ids.iter().copied());
            assert_eq!((all.ids(), all.live()), (each.ids(), each.live()));
            assert_eq!(all.page_sections(), each.page_sections());
            assert_eq!(free_ids(&all), free_ids(&each));
            assert!((0..each.ids()).all(|id| all.at(id) == each.at(id)));
        }
        // The lowest free id comes back first, then the next (0 left with `gone`, 1 before).
        assert_eq!((all.take_free(), all.take_free()), (Some(0), Some(1)));
        assert_eq!(all.ids(), 896);
        // All of them: an empty table.
        let live: Vec<u32> = (0..all.ids()).filter(|&id| all.at(id).is_some()).collect();
        all.remove_all(live);
        assert_eq!((all.ids(), all.live(), all.free.len()), (0, 0, 0));
        assert!(all.page_sections().iter().all(|&n| n == 0));

        records.clear();
        assert_eq!((records.ids(), records.live()), (0, 0));
        assert!(records.page_sections().is_empty() && records.dirty.is_empty());
        for x in 0..DIRTY_LIMIT as i32 {
            records.insert((x, 0, 0), 0, 0, 1);
        }
        assert!(!records.all_dirty && records.dirty.len() == DIRTY_LIMIT);
        records.relocate(0, 1, 256);
        assert!(records.all_dirty && records.dirty.is_empty());
        records.clear();
        assert!(!records.all_dirty);
        assert_eq!(records.insert((0, 0, 0), 0, 0, 1), 0);
    }

    /// `Params`, `Section` and `Update` as the shader lays them out, and the
    /// constants both sides use.
    #[test]
    fn params_and_records_match_the_shader() {
        let module = naga::front::wgsl::parse_str(include_str!("gpu_culling.wgsl")).unwrap();
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();
        // A struct's size and its members' offsets.
        let layout = |name: &str| {
            let (handle, ty) = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name}"));
            let naga::TypeInner::Struct { members, .. } = &ty.inner else {
                panic!("{name} is a struct");
            };
            let members = members
                .iter()
                .map(|m| (m.name.clone().unwrap(), m.offset as usize));
            (layouter[handle].size as usize, members.collect::<Vec<_>>())
        };
        use std::mem::{offset_of, size_of};
        let (size, members) = layout("Params");
        assert_eq!(size, size_of::<Params>());
        for (name, offset) in [
            ("ids", offset_of!(Params, ids)),
            ("updates", offset_of!(Params, updates)),
            ("pages", offset_of!(Params, pages)),
            ("regions", offset_of!(Params, regions)),
            ("mask", offset_of!(Params, mask)),
            ("pad0", offset_of!(Params, pad)),
            ("first", offset_of!(Params, first)),
            ("cells", offset_of!(Params, cells)),
        ] {
            assert!(members.contains(&(name.into(), offset)), "Params.{name}");
        }
        let named = |pairs: [(&str, usize); 2]| pairs.map(|(n, o)| (n.to_owned(), o)).to_vec();
        let key = [
            ("key", offset_of!(Record, key)),
            ("a", offset_of!(Record, a)),
        ];
        assert_eq!(layout("Section"), (size_of::<Record>(), named(key)));
        let id = [
            ("id", offset_of!(Update, id)),
            ("section", offset_of!(Update, record)),
        ];
        assert_eq!(layout("Update"), (size_of::<Update>(), named(id)));
        assert_eq!((size_of::<Record>(), size_of::<Update>()), (32, 48));
        let constant = |name: &str| {
            let (_, c) = module
                .constants
                .iter()
                .find(|(_, c)| c.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name}"));
            match module.global_expressions[c.init] {
                naga::Expression::Literal(naga::Literal::U32(value)) => u64::from(value),
                ref e => panic!("{name}: {e:?}"),
            }
        };
        // A whole section's instance w: `section_instance`'s bare UV step.
        let step = super::super::section_instance([0.0; 3], 0, 1.0)[3];
        assert_eq!(step, 1.0 / crate::mesher::UV_STEPS);
        for (name, value) in [
            ("STEP_BITS", u64::from(step.to_bits())),
            ("ROW", u64::from(ROW)),
            ("BLOCK", u64::from(WORKGROUP)),
            ("STATS", STATS),
        ] {
            assert_eq!(constant(name), value, "{name}");
        }
    }

    /// A device of `backends` that draws counts with first instances, if this
    /// machine has one; on DirectX 12, `compiler` compiles its shaders. Not
    /// Intel's Gen11 and Gen12 Vulkan driver, which gets first instances wrong
    /// and is never offered GPU culling (`supported`).
    fn device(
        backends: wgpu::Backends,
        compiler: wgpu::Dx12Compiler,
        name: &str,
    ) -> Option<(wgpu::Device, wgpu::Queue)> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        desc.backend_options.dx12.shader_compiler = compiler;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let needed =
            wgpu::Features::MULTI_DRAW_INDIRECT_COUNT | wgpu::Features::INDIRECT_FIRST_INSTANCE;
        if !adapter.features().contains(needed)
            || super::super::flat::is_broken_intel_vulkan(&adapter.get_info())
        {
            return None;
        }
        eprintln!("GPU culling on {} ({name})", adapter.get_info().name);
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: needed,
            ..Default::default()
        }))
        .ok()
    }

    /// `test` on each device here that draws counts with first instances:
    /// Vulkan, and DirectX 12 with each of its compilers. Players' DirectX 12
    /// compiles with FXC, part of Windows: wgpu takes DXC only where
    /// `dxcompiler.dll` loads, and the game ships none.
    fn on_each_device(test: impl Fn(&wgpu::Device, &Queue)) {
        let mut tested = false;
        for (backends, compiler, name) in [
            (wgpu::Backends::VULKAN, Default::default(), "Vulkan"),
            (
                wgpu::Backends::DX12,
                wgpu::Dx12Compiler::Fxc,
                "DirectX 12, FXC",
            ),
            (
                wgpu::Backends::DX12,
                wgpu::Dx12Compiler::default_dynamic_dxc(),
                "DirectX 12, DXC",
            ),
        ] {
            if let Some((device, queue)) = device(backends, compiler, name) {
                test(&device, &Queue::new(queue, false));
                tested = true;
            }
        }
        if !tested {
            eprintln!("no GPU with indirect count draws: skipped");
        }
    }

    fn read(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        bytes: u64,
    ) -> Vec<u32> {
        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(
            device,
            queue,
            &buffer.slice(..bytes),
            move |result| tx.send(result.unwrap().to_vec()).unwrap(),
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        bytemuck::pod_collect_to_vec(&rx.recv().unwrap())
    }

    /// The pass lists exactly the draws of the sections it is given: one draw
    /// of all the solid quads of each, from its page and first vertex, with the
    /// instance the CPU path gives a whole section, every page's draws in id
    /// order. One table serves every case, changed between them as sections
    /// come, go and move (ids reused, records written as updates and whole),
    /// and cleared once as for another world; cameras near the world's origin
    /// and 20 million blocks out.
    #[test]
    fn the_pass_lists_the_draws_of_the_sections_it_is_given() {
        on_each_device(draws_match);
    }

    fn draws_match(device: &wgpu::Device, queue: &Queue) {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let (min_y, height) = (-64, 24);
        let mut gpu = GpuCulling::new(device).expect("the pass");
        let mut uploads = Uploads::new(device);
        let mut held: BTreeMap<u32, Placed> = BTreeMap::new();
        let (mut camera, mut compared) = (DVec3::ZERO, 0);
        for case in 0..24 {
            let radius = [2, 5, 9][case % 3];
            // Case 4 looks from case 3's camera, 20 million blocks out, while
            // every section moves and none comes: the whole table is written
            // though it keeps its room.
            if case != 4 {
                let far = |axis| if case % 4 == axis { 10_000.0 } else { 1.0 };
                camera = DVec3::new(
                    f64::from(rng.range(-2_000, 2_000)) * far(3) + f64::from(rng.unit()),
                    f64::from(rng.range(-60, 300)) + f64::from(rng.unit()),
                    f64::from(rng.range(-2_000, 2_000)) * far(1) + f64::from(rng.unit()),
                );
            }
            let (cx, cz) = resident::camera_column(camera);
            if case == 12 {
                // Another world.
                gpu.records.clear();
                held.clear();
            }
            // A third of the sections go and a sixth move, some to pages
            // past the last.
            let mut ids: Vec<u32> = held.keys().copied().collect();
            for _ in 0..ids.len() / 3 {
                let id = ids.swap_remove(rng.below(ids.len() as u32) as usize);
                gpu.records.remove(id);
                held.remove(&id);
            }
            let mut moves = ids.len() / 6;
            while !ids.is_empty() && (moves > 0 || (case == 4 && !gpu.records.all_dirty)) {
                moves = moves.saturating_sub(1);
                let id = ids[rng.below(ids.len() as u32) as usize];
                let (page, start) = (rng.below(6), u64::from(rng.below(40_000)) * 256);
                gpu.records.relocate(id, page, start);
                let placed = held.get_mut(&id).expect("a live id");
                (placed.1, placed.2) = (page, start);
            }
            // New sections around the camera, within the distance and past it.
            let place = |gpu: &mut GpuCulling, held: &mut BTreeMap<u32, Placed>, rng: &mut Rng| {
                let near = |rng: &mut Rng| rng.range(-radius - 3, radius + 3);
                let key = (cx + near(rng), rng.range(0, height - 1), cz + near(rng));
                let (page, start) = (rng.below(3), u64::from(rng.below(40_000)) * 256);
                let quads = 1 + rng.below(2_000);
                let id = gpu.records.insert(key, page, start, quads);
                assert!(
                    held.insert(id, (key, page, start, quads)).is_none(),
                    "id {id} twice"
                );
            };
            let new = if case == 4 { 0 } else { 3_000 };
            for _ in 0..new {
                place(&mut gpu, &mut held, &mut rng);
            }
            let room = gpu.sections.bytes;
            if case == 18 {
                // Past the table's room: it grows, and is written whole.
                while u64::from(gpu.records.ids()) * Record::BYTES <= room {
                    place(&mut gpu, &mut held, &mut rng);
                }
            }
            let whole = gpu.records.all_dirty;
            let mut encoder = device.create_command_encoder(&Default::default());
            let input = FrameInputs {
                camera,
                min_y,
                sections: height,
                radius,
            };
            assert!(gpu.prepare(device, queue, &mut uploads, &mut encoder, &input));
            match case {
                4 => assert!(
                    whole && gpu.sections.bytes == room,
                    "case 4: in place, whole"
                ),
                18 => assert!(gpu.sections.bytes > room, "case 18: the table grows"),
                _ => {}
            }
            // The walk's choice: some of the sections within the distance, and
            // a few past the tables, which the pass leaves out.
            let mut expected = Vec::new();
            for (&id, &(key, page, start, quads)) in &held {
                let outside = (key.0 - cx).abs() > radius || (key.2 - cz).abs() > radius;
                if in_view_distance((cx, cz), radius, key.0, key.2) && rng.below(3) > 0 {
                    gpu.choose(id);
                    let origin = resident::origin(key, min_y, camera).as_vec3();
                    let instance = super::super::section_instance(origin.to_array(), 0, 1.0);
                    expected.push((page, start as u32, quads * 6, instance.map(f32::to_bits)));
                } else if outside && rng.below(16) == 0 {
                    gpu.choose(id);
                }
            }
            gpu.upload_choices(&mut uploads, &mut encoder);
            gpu.dispatch(&mut encoder, None);
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            let pages = u64::from(gpu.pass_pages);
            let counts = read(device, queue, &gpu.counts.buffer, (STATS + pages) * 4);
            let draws = read(device, queue, &gpu.draws.buffer, gpu.draws.bytes);
            let origins = read(device, queue, &gpu.origins.buffer, u64::from(gpu.ids) * 16);
            let mut got = Vec::new();
            for &(page, first, size) in &gpu.pages {
                let count = counts[(STATS + u64::from(page)) as usize];
                assert!(
                    count <= size,
                    "case {case}, page {page}: {count} draws, room {size}"
                );
                let mut last = None;
                for d in draws[first as usize * 5..][..count as usize * 5].chunks(5) {
                    // In id order, the same every frame.
                    assert!(
                        last < Some(d[4]),
                        "case {case}, page {page}: out of id order"
                    );
                    last = Some(d[4]);
                    assert_eq!([d[1], d[2]], [1, 0], "case {case}: one instance, index 0");
                    let instance = &origins[d[4] as usize * 4..][..4];
                    let instance: [u32; 4] = instance.try_into().unwrap();
                    got.push((page, d[3], d[0], instance));
                }
            }
            expected.sort_unstable();
            got.sort_unstable();
            assert_eq!(got.len(), expected.len(), "case {case}: draws");
            assert_eq!(got, expected, "case {case}");
            let kept = expected.len() as u32;
            assert_eq!(
                counts[..2],
                [kept, kept],
                "case {case}: sections kept, draws"
            );
            compared += expected.len();
        }
        // Not a comparison of empty lists.
        assert!(compared > 2_000, "{compared} draws");
    }

    /// `draw`'s count draws draw what the pass listed, page by page: each
    /// chosen section's quads from its own range, with its own origin, which
    /// the draw's first instance picks out of the instance-rate origins (on
    /// DirectX 12 too, where no count draw's first instance reaches
    /// `instance_index`), and nothing past a page's count, though its region
    /// still holds the frame before's longer list. Drawn through the terrain's
    /// vertex layouts, each quad writes its origin's bits to a pixel of its own.
    #[test]
    fn the_count_draws_draw_each_chosen_section_with_its_origin() {
        on_each_device(draws_land);
    }

    /// `draws_land`'s shaders: a quad's corners put it on its pixel, which
    /// gets the bits of the origin it is drawn with.
    const ORIGINS: &str = "
struct Quad {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) origin: vec4<u32>,
}
@vertex fn vs(@location(0) position: vec3<f32>, @location(5) origin: vec4<f32>) -> Quad {
    return Quad(vec4<f32>(position.xy, 0.0, 1.0), bitcast<vec4<u32>>(origin));
}
@fragment fn fs(quad: Quad) -> @location(0) vec4<u32> {
    return quad.origin;
}
";

    fn draws_land(device: &wgpu::Device, queue: &Queue) {
        use crate::mesher::TerrainVertex;
        // Section i's quad q covers pixel (4 * (i % 16) + q, i / 16). The
        // sections take turns on pages 0, 1 and 3, a grain each: page 2 has none.
        const SECTIONS: u32 = 192;
        let (width, height) = (64, SECTIONS / 16);
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let grain = super::super::arena::GRAIN;
        let mut arena = Arena::new(device, stride, grain, false);
        for _ in 0..4 {
            assert!(arena.open_page(device, u64::from(SECTIONS / 3) * grain));
        }
        let mut gpu = GpuCulling::new(device).expect("the pass");
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (radius, min_y, levels) = (4, -64, 24);
        let corner = |x: u32, y: u32| TerrainVertex {
            pos: [
                x as f32 / width as f32 * 2.0 - 1.0,
                1.0 - y as f32 / height as f32 * 2.0,
                0.0,
            ],
            ..bytemuck::Zeroable::zeroed()
        };
        let mut sections = Vec::new();
        for i in 0..SECTIONS {
            let (page, start) = ([0, 1, 3][i as usize % 3], u64::from(i / 3) * grain);
            let quads = 1 + rng.below(4);
            let (x, y) = (4 * (i % 16), i / 16);
            let vertices: Vec<_> = (x..x + quads)
                .flat_map(|x| [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)])
                .map(|(x, y)| corner(x, y))
                .collect();
            let bytes = bytemuck::cast_slice(&vertices);
            queue.write_buffer(arena.buffer(page), start * stride, bytes);
            let near = |rng: &mut Rng| rng.range(-radius, radius);
            let key = (near(&mut rng), rng.range(0, levels - 1), near(&mut rng));
            sections.push((gpu.records.insert(key, page, start, quads), key, quads));
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(ORIGINS.into()),
        });
        let origins = wgpu::vertex_attr_array![5 => Float32x4];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &super::super::TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &origins,
            }),
        ];
        let format = wgpu::TextureFormat::Rgba32Uint;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: None,
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: None,
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(width * height) * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let indices = super::super::Renderer::quad_indices(device, 4);
        let mut uploads = Uploads::new(device);
        // The second frame chooses about half as many, from another camera in
        // the same column.
        let cameras = [
            DVec3::new(8.375, 70.625, 3.125),
            DVec3::new(2.75, -3.5, 13.875),
        ];
        for (frame, camera) in cameras.into_iter().enumerate() {
            let mut encoder = device.create_command_encoder(&Default::default());
            let input = FrameInputs {
                camera,
                min_y,
                sections: levels,
                radius,
            };
            assert!(gpu.prepare(device, queue, &mut uploads, &mut encoder, &input));
            let mut expected = vec![[0; 4]; (width * height) as usize];
            for (i, &(id, key, quads)) in sections.iter().enumerate() {
                if rng.below(3) < 2 - frame as u32 {
                    gpu.choose(id);
                    let origin = resident::origin(key, min_y, camera).as_vec3();
                    let instance = super::super::section_instance(origin.to_array(), 0, 1.0);
                    let at = i / 16 * width as usize + 4 * (i % 16);
                    expected[at..at + quads as usize].fill(instance.map(f32::to_bits));
                }
            }
            gpu.upload_choices(&mut uploads, &mut encoder);
            gpu.dispatch(&mut encoder, None);
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
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            gpu.draw(&mut pass, &arena);
            drop(pass);
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &pixels,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 16),
                        rows_per_image: None,
                    },
                },
                target.size(),
            );
            uploads.finish(&encoder);
            queue.submit([encoder.finish()]);
            let got = read(device, queue, &pixels, pixels.size());
            let row = width as usize;
            for (p, (got, want)) in got.chunks(4).zip(&expected).enumerate() {
                let (i, q) = (p / row * 16 + p % row / 4, p % 4);
                assert_eq!(got, want, "frame {frame}, section {i}, quad {q}");
            }
        }
    }
}
