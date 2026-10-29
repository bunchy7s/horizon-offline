//! Flat section draws: where indirect multi-draw is emulated, as on Metal
//! (wgpu replays it as one draw call per indirect entry), the frame's section
//! draws are copied by a compute pass into one index list, in draw order, and
//! drawn with one draw call per run of sections in the same arena page.
//!
//! The picture is the one the per-section draws give: the same triangles
//! from the same vertices in the same order, through the same vertex and
//! fragment code. Only where a vertex finds its section's origin changes:
//! from the instance attribute to a table. A section's vertices fill whole
//! blocks of `GRAIN` vertices of its page (the arena rounds every range up),
//! so each block names one section: its id here, which indexes the frame's
//! camera-relative origins (computed on the CPU exactly as for the instance
//! attribute).
//!
//! With occlusion culling (`cull`), phase 1's draws and the translucent ones
//! are copied before the world pass, as without it. Phase 2's draws are
//! copied after the test, which tells which ones it saw: each run of a page
//! reserves room for all of its draws, the copy packs the seen ones at its
//! start, and the run is drawn indirectly with the count the copy reached.
use super::arena::GRAIN;

/// One section draw for `flatten.wgsl`: index count, first sorted index (or
/// `QUADS` for a solid draw; phase 2: its entry in phase 2's draws), base
/// vertex, first index in the list (phase 2: its draw call in phase 2).
type Range = [u32; 4];

/// Where a group of ranges takes its indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Solid draws: quads.
    Quads,
    /// Occlusion culling's phase 2: quads, of the sections its test saw.
    Seen,
    /// Translucent draws: this sorted index page.
    Sorted(u32),
}

/// The frame's draw calls, as `plan` lays them out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    phase1_calls: usize,
    solid_calls: usize,
    /// Where the shadow camera's solid and translucent calls begin.
    shadow_calls: [usize; 2],
    total: u64,
}
const QUADS: u32 = u32::MAX;
const GROUP_LIMIT: u32 = 65535;

struct PageTable {
    /// Section id of every block of the page.
    blocks: wgpu::Buffer,
    group: Option<wgpu::BindGroup>,
}

/// A run of the frame's index list drawn from one arena page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Call {
    pub page: u32,
    pub first: u32,
    pub count: u32,
}

pub struct Flat {
    pub solid: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    flatten: wgpu::ComputePipeline,
    compute_layout: wgpu::BindGroupLayout,
    flatten_seen: wgpu::ComputePipeline,
    seen_layout: wgpu::BindGroupLayout,
    /// Phase 2's draw calls (`DrawIndexedIndirectArgs`), whose index counts
    /// `flatten_seen` makes.
    phase2_calls: wgpu::Buffer,
    phase2_call_capacity: u64,
    phase2_call_data: Vec<[u32; 5]>,
    /// The copy of phase 2's draws, and its ranges.
    phase2_group: Option<(wgpu::BindGroup, u32)>,
    /// The layout of a page's bind group 1 (shader packs' terrain pipelines share it).
    pub(super) draw_layout: wgpu::BindGroupLayout,
    blocks_per_page: u64,
    pages: Vec<Option<PageTable>>,
    free_ids: Vec<u32>,
    next_id: u32,
    origins: wgpu::Buffer,
    origin_capacity: u64,
    origin_data: Vec<[f32; 4]>,
    /// The frame's index list.
    pub indices: wgpu::Buffer,
    index_capacity: u64,
    ranges: wgpu::Buffer,
    range_capacity: u64,
    range_data: Vec<Range>,
    /// Segments of `range_data` by index source: (source, first range,
    /// ranges).
    segments: Vec<(Source, u64, u64)>,
    empty: wgpu::Buffer,
    /// This frame's draw calls: solid ones (phase 1's, then phase 2's when
    /// culling), then translucent ones.
    pub calls: Vec<Call>,
    pub phase1_calls: usize,
    pub solid_calls: usize,
    /// Where a shader pack's shadow camera's solid and translucent calls
    /// begin (`calls.len()` without them): the camera's translucent calls
    /// end at the first.
    pub shadow_calls: [usize; 2],
    /// Frames with fewer sections in view keep the per-section draws.
    pub min_sections: usize,
    range_align: u64,
}

/// Sections in view (and distant tiles) from which a frame is drawn flat.
/// Below it the index list costs the GPU more than the draw calls cost the
/// CPU: on an M4 Pro at render distance 16 (2,900 draws) the copy and the
/// list add about 0.4 ms of GPU time for 0.5–0.8 ms less CPU time; at 32
/// (9,300 draws) the CPU saves 1.3 ms. The bar was 4,096 draws when they
/// were 2.2 to 2.5 per section; a section's draw count now depends on the
/// sides it shows the eye (`mesher::Sides`), so it counts sections.
const MIN_SECTIONS: usize = 1600;

/// Whether flat draws apply: the backend emulates multi-draw, and vertex
/// shaders can read storage buffers large enough for an index page.
/// `HORIZON_FLAT_DRAWS=1` draws every frame flat, `0` none.
pub fn wanted(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
    match std::env::var("HORIZON_FLAT_DRAWS").as_deref() {
        Ok("0") => return false,
        Ok("1") => return supported(adapter, device),
        _ => {}
    }
    adapter.get_info().backend == wgpu::Backend::Metal && supported(adapter, device)
}

fn supported(adapter: &wgpu::Adapter, device: &wgpu::Device) -> bool {
    let flags = adapter.get_downlevel_capabilities().flags;
    let limits = device.limits();
    flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE | wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && limits.max_storage_buffers_per_shader_stage >= 3
        && limits.max_storage_buffer_binding_size >= super::arena::PAGE_BYTES
}

fn storage(binding: u32, stage: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Lays out the frame's index list: each draw's range (destinations in draw
/// order) grouped by index source into `ranges`, every group padded to
/// `align` entries for binding, with `segments` naming each group; and the
/// draw calls, one per run of draws in the same vertex page, phase 1's solid
/// ones, phase 2's (draws `phase1_draws..solid_draws`) and translucent ones
/// apart. A phase 2 range names its draw in phase 2 and its call in phase 2
/// instead of a destination: the call's room holds all of its draws. Draws
/// from `shadow[0]` (solid) and `shadow[1]` (translucent) on are a shader
/// pack's shadow camera's, with calls of their own.
#[allow(clippy::too_many_arguments)]
fn plan(
    draws: &[[u32; 5]],
    pages: &[(u32, Option<u32>)],
    phase1_draws: usize,
    solid_draws: usize,
    shadow: [usize; 2],
    align: u64,
    calls: &mut Vec<Call>,
    ranges: &mut Vec<Range>,
    segments: &mut Vec<(Source, u64, u64)>,
) -> Plan {
    calls.clear();
    ranges.clear();
    segments.clear();
    let (mut phase1_calls, mut solid_calls) = (None, None);
    let mut shadow_calls = [None; 2];
    let mut total = 0u64;
    let mut by_source: Vec<(Source, Vec<Range>)> = Vec::new();
    for (i, (d, &(page, sorted))) in draws.iter().zip(pages).enumerate() {
        if i == phase1_draws {
            phase1_calls.get_or_insert(calls.len());
        }
        if i == solid_draws {
            solid_calls = Some(calls.len());
        }
        for (k, &from) in shadow.iter().enumerate() {
            if i == from {
                shadow_calls[k].get_or_insert(calls.len());
            }
        }
        let starts = i == phase1_draws || i == solid_draws || shadow.contains(&i);
        match calls.last_mut() {
            Some(last) if last.page == page && !starts => last.count += d[0],
            _ => calls.push(Call {
                page,
                first: total as u32,
                count: d[0],
            }),
        }
        let (source, range) = match sorted {
            Some(p) => (Source::Sorted(p), [d[0], d[2], d[3], total as u32]),
            None if (phase1_draws..solid_draws).contains(&i) => (
                Source::Seen,
                [
                    d[0],
                    (i - phase1_draws) as u32,
                    d[3],
                    (calls.len() - 1 - phase1_calls.unwrap_or(0)) as u32,
                ],
            ),
            None => (Source::Quads, [d[0], QUADS, d[3], total as u32]),
        };
        match by_source.iter_mut().find(|(s, _)| *s == source) {
            Some((_, list)) => list.push(range),
            None => by_source.push((source, vec![range])),
        }
        total += u64::from(d[0]);
    }
    for (source, list) in by_source {
        segments.push((source, ranges.len() as u64, list.len() as u64));
        ranges.extend(list);
        let padded = (ranges.len() as u64).next_multiple_of(align);
        ranges.resize(padded as usize, [0; 4]);
    }
    let solid_calls = solid_calls.unwrap_or(calls.len());
    Plan {
        phase1_calls: phase1_calls.unwrap_or(calls.len()).min(solid_calls),
        solid_calls,
        shadow_calls: shadow_calls.map(|c| c.unwrap_or(calls.len())),
        total,
    }
}

impl Flat {
    /// `pipeline(layout, label, vertex entry, fragment entry, blend)` builds a
    /// terrain pipeline as the per-section ones, without the origin buffer.
    pub fn new(
        device: &wgpu::Device,
        terrain_layout: &wgpu::BindGroupLayout,
        page_vertices: u64,
        pipeline: impl Fn(
            &wgpu::PipelineLayout,
            &str,
            &str,
            Option<wgpu::BlendState>,
        ) -> wgpu::RenderPipeline,
    ) -> Self {
        let vertex = wgpu::ShaderStages::VERTEX;
        let compute = wgpu::ShaderStages::COMPUTE;
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Flat section draws"),
            entries: &[storage(0, vertex, true), storage(1, vertex, true)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Flat terrain layout"),
            bind_group_layouts: &[Some(terrain_layout), Some(&draw_layout)],
            ..Default::default()
        });
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Flatten section draws"),
            entries: &[
                storage(0, compute, true),
                storage(1, compute, true),
                storage(2, compute, false),
            ],
        });
        let seen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Flatten phase 2 section draws"),
            entries: &[
                storage(0, compute, true),
                storage(2, compute, false),
                storage(3, compute, true),
                storage(4, compute, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Flatten section draws"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flatten.wgsl").into()),
        });
        let compute_pipeline = |layout: &wgpu::BindGroupLayout, entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Flatten section draws"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Flatten section draws"),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    }),
                ),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let flatten = compute_pipeline(&compute_layout, "flatten");
        let flatten_seen = compute_pipeline(&seen_layout, "flatten_seen");
        let storage_usage = wgpu::BufferUsages::STORAGE;
        Self {
            solid: pipeline(&layout, "Terrain solid (flat)", "fs_solid", None),
            translucent: pipeline(
                &layout,
                "Terrain translucent (flat)",
                "fs_translucent",
                Some(wgpu::BlendState::ALPHA_BLENDING),
            ),
            flatten,
            compute_layout,
            flatten_seen,
            seen_layout,
            phase2_calls: buffer(
                device,
                "Flat section draw calls, phase 2",
                20,
                wgpu::BufferUsages::INDIRECT | storage_usage,
            ),
            phase2_call_capacity: 1,
            phase2_call_data: Vec::new(),
            phase2_group: None,
            draw_layout,
            blocks_per_page: page_vertices.div_ceil(GRAIN),
            pages: Vec::new(),
            free_ids: Vec::new(),
            next_id: 0,
            origins: buffer(device, "Section origins by id", 16, storage_usage),
            origin_capacity: 1,
            origin_data: Vec::new(),
            indices: buffer(
                device,
                "Flat section indices",
                16,
                wgpu::BufferUsages::INDEX | storage_usage,
            ),
            index_capacity: 4,
            ranges: buffer(device, "Flat section ranges", 16, storage_usage),
            range_capacity: 1,
            range_data: Vec::new(),
            segments: Vec::new(),
            empty: buffer(device, "No sorted indices", 16, storage_usage),
            calls: Vec::new(),
            phase1_calls: 0,
            solid_calls: 0,
            shadow_calls: [0; 2],
            min_sections: if std::env::var("HORIZON_FLAT_DRAWS").as_deref() == Ok("1") {
                0
            } else {
                MIN_SECTIONS
            },
            range_align: (u64::from(device.limits().min_storage_buffer_offset_alignment) / 16)
                .max(1),
        }
    }
    /// A section uploaded to `vertices` vertices from `start` of `page`: its
    /// id, which its blocks now name (written through `uploads` in `encoder`).
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        page: u32,
        start: u64,
        vertices: u64,
    ) -> u32 {
        let id = self.free_ids.pop().unwrap_or_else(|| {
            self.next_id += 1;
            self.next_id - 1
        });
        let page = page as usize;
        if self.pages.len() <= page {
            self.pages.resize_with(page + 1, || None);
        }
        let table = self.pages[page].get_or_insert_with(|| PageTable {
            blocks: buffer(
                device,
                "Section ids by block",
                self.blocks_per_page * 4,
                wgpu::BufferUsages::STORAGE,
            ),
            group: None,
        });
        let first = start / GRAIN;
        let ids = vec![id; vertices.div_ceil(GRAIN) as usize];
        uploads.buffer(
            encoder,
            &table.blocks,
            first * 4,
            bytemuck::cast_slice(&ids),
        );
        id
    }
    pub fn remove(&mut self, id: u32) {
        self.free_ids.push(id);
    }
    pub fn clear(&mut self) {
        self.pages.clear();
        self.free_ids.clear();
        self.next_id = 0;
    }
    /// Forgets the tables of pages the arena released.
    pub fn trim(&mut self, live: impl Fn(u32) -> bool) {
        for (i, page) in self.pages.iter_mut().enumerate() {
            if page.is_some() && !live(i as u32) {
                *page = None;
            }
        }
    }
    /// Builds the frame's index list from `draws` (`DrawIndexedIndirectArgs`:
    /// index count, instances, first index, base vertex, first instance),
    /// solid ones first, with their (vertex page, sorted index page) and
    /// the origins of the sections they draw by id. Records the copy into
    /// `encoder`; `calls` then lists the frame's draw calls. False when the
    /// frame cannot be drawn flat (the per-section draws then apply).
    ///
    /// When culling, the solid draws from `phase1_draws` are phase 2's, which
    /// `phase2` holds as the test leaves them: `flatten_phase2` copies them.
    /// Draws from `shadow` on are a shader pack's shadow camera's (`plan`).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        draws: &[[u32; 5]],
        pages: &[(u32, Option<u32>)],
        phase1_draws: usize,
        solid_draws: usize,
        shadow: [usize; 2],
        phase2: Option<&wgpu::Buffer>,
        origins: impl Iterator<Item = (u32, [f32; 4])>,
        sorted_page: impl Fn(u32) -> wgpu::Buffer,
    ) -> bool {
        self.phase2_group = None;
        let phase1_draws = if phase2.is_some() {
            phase1_draws
        } else {
            solid_draws
        };
        // Origins by section id.
        self.origin_data.clear();
        for (id, o) in origins {
            let id = id as usize;
            if self.origin_data.len() <= id {
                self.origin_data.resize(id + 1, [0.0; 4]);
            }
            self.origin_data[id] = o;
        }
        if self.origin_data.len() as u64 > self.origin_capacity {
            self.origin_capacity = (self.origin_data.len() as u64).next_power_of_two();
            self.origins = buffer(
                device,
                "Section origins by id",
                self.origin_capacity * 16,
                wgpu::BufferUsages::STORAGE,
            );
            for page in self.pages.iter_mut().flatten() {
                page.group = None;
            }
        }
        if !self.origin_data.is_empty() {
            uploads.buffer(
                encoder,
                &self.origins,
                0,
                bytemuck::cast_slice(&self.origin_data),
            );
        }
        let Plan {
            phase1_calls,
            solid_calls,
            shadow_calls,
            total,
        } = plan(
            draws,
            pages,
            phase1_draws,
            solid_draws,
            shadow,
            self.range_align,
            &mut self.calls,
            &mut self.range_data,
            &mut self.segments,
        );
        self.phase1_calls = phase1_calls;
        self.solid_calls = solid_calls;
        self.shadow_calls = shadow_calls;
        if total == 0 || total * 4 > device.limits().max_storage_buffer_binding_size {
            self.calls.clear();
            self.phase1_calls = 0;
            self.solid_calls = 0;
            self.shadow_calls = [0; 2];
            return false;
        }
        if self.range_data.len() as u64 > self.range_capacity {
            self.range_capacity = (self.range_data.len() as u64).next_power_of_two();
            self.ranges = buffer(
                device,
                "Flat section ranges",
                self.range_capacity * 16,
                wgpu::BufferUsages::STORAGE,
            );
        }
        if total > self.index_capacity {
            self.index_capacity = total.next_power_of_two();
            self.indices = buffer(
                device,
                "Flat section indices",
                self.index_capacity * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
            );
        }
        uploads.buffer(
            encoder,
            &self.ranges,
            0,
            bytemuck::cast_slice(&self.range_data),
        );
        // Phase 2's calls start empty, each at the start of its room.
        self.phase2_call_data.clear();
        self.phase2_call_data.extend(
            self.calls[phase1_calls..solid_calls]
                .iter()
                .map(|c| [0, 1, c.first, 0, 0]),
        );
        let phase2_calls = self.phase2_call_data.len() as u64;
        if phase2_calls > self.phase2_call_capacity {
            self.phase2_call_capacity = phase2_calls.next_power_of_two();
            self.phase2_calls = buffer(
                device,
                "Flat section draw calls, phase 2",
                self.phase2_call_capacity * 20,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
            );
        }
        if phase2_calls > 0 {
            uploads.buffer(
                encoder,
                &self.phase2_calls,
                0,
                bytemuck::cast_slice(&self.phase2_call_data),
            );
        }
        let ranges_at = |first: u64, count: u64| {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.ranges,
                offset: first * 16,
                size: wgpu::BufferSize::new(count * 16),
            })
        };
        if let (Some(phase2), Some(&(_, first, count))) = (
            phase2,
            self.segments.iter().find(|(s, ..)| *s == Source::Seen),
        ) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Flatten phase 2 section draws"),
                layout: &self.seen_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ranges_at(first, count),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.indices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: phase2.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.phase2_calls.as_entire_binding(),
                    },
                ],
            });
            self.phase2_group = Some((group, count as u32));
        }
        let groups: Vec<(wgpu::BindGroup, u32)> = self
            .segments
            .iter()
            .filter_map(|&(source, first, count)| {
                let sorted = match source {
                    Source::Quads => None,
                    Source::Seen => return None,
                    Source::Sorted(page) => Some(sorted_page(page)),
                };
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Flatten section draws"),
                    layout: &self.compute_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ranges_at(first, count),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: sorted.as_ref().unwrap_or(&self.empty).as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.indices.as_entire_binding(),
                        },
                    ],
                });
                Some((group, count as u32))
            })
            .collect();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Flatten section draws"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.flatten);
        for (group, count) in &groups {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups((*count).min(GROUP_LIMIT), count.div_ceil(GROUP_LIMIT), 1);
        }
        drop(pass);
        for page in self.calls.iter().map(|c| c.page) {
            let table = self.pages[page as usize]
                .as_mut()
                .expect("a drawn page has its block table");
            if table.group.is_none() {
                table.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Flat section draws"),
                    layout: &self.draw_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: table.blocks.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.origins.as_entire_binding(),
                        },
                    ],
                }));
            }
        }
        true
    }
    /// After occlusion culling's test: copies the phase 2 draws it saw.
    pub fn flatten_phase2(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some((group, count)) = &self.phase2_group else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Flatten phase 2 section draws"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.flatten_seen);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups((*count).min(GROUP_LIMIT), count.div_ceil(GROUP_LIMIT), 1);
    }

    /// The camera's translucent calls.
    pub fn translucent_calls(&self) -> &[Call] {
        &self.calls[self.solid_calls..self.shadow_calls[0]]
    }

    /// Draws `calls` with a flat pipeline set and `vertices` giving each
    /// page's vertex buffer.
    pub fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        calls: &[Call],
        vertices: impl Fn(u32) -> &'a wgpu::Buffer,
    ) {
        self.draw_with(pass, calls, vertices, |_, _| {});
    }
    /// `draw`, calling `page` before each run of calls from one arena page (a shader pack binds
    /// that page's vertices for reading).
    pub fn draw_with<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        calls: &[Call],
        vertices: impl Fn(u32) -> &'a wgpu::Buffer,
        page: impl Fn(&mut wgpu::RenderPass<'_>, u32),
    ) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for call in calls {
            self.bind_page(pass, call.page, &vertices);
            page(pass, call.page);
            pass.draw_indexed(call.first..call.first + call.count, 0, 0..1);
        }
    }
    /// Draws phase 2's calls, as `flatten_phase2` left them.
    pub fn draw_phase2<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        vertices: impl Fn(u32) -> &'a wgpu::Buffer,
    ) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for (k, call) in self.calls[self.phase1_calls..self.solid_calls]
            .iter()
            .enumerate()
        {
            self.bind_page(pass, call.page, &vertices);
            pass.draw_indexed_indirect(&self.phase2_calls, k as u64 * 20);
        }
    }
    fn bind_page<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        page: u32,
        vertices: &impl Fn(u32) -> &'a wgpu::Buffer,
    ) {
        let table = self.pages[page as usize]
            .as_ref()
            .and_then(|t| t.group.as_ref())
            .expect("prepared page");
        pass.set_vertex_buffer(0, vertices(page).slice(..));
        pass.set_bind_group(1, table, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws as `render` writes them: solid (page, quads at a base vertex)
    /// first, then translucent (page, sorted page, count, first sorted index,
    /// base vertex).
    fn sample() -> (Vec<[u32; 5]>, Vec<(u32, Option<u32>)>, usize) {
        let draws = vec![
            [6 * 3, 1, 0, 0, 0],
            [6 * 2, 1, 0, 512, 1],
            [6, 1, 0, 256, 2],
            [6 * 4, 1, 0, 1024, 3],
            [6 * 2, 1, 6, 768, 1],
            [6, 1, 0, 40, 0],
        ];
        let pages = vec![
            (0, None),
            (0, None),
            (1, None),
            (0, None),
            (0, Some(0)),
            (0, Some(1)),
        ];
        (draws, pages, 4)
    }

    #[test]
    fn plan_draws_runs_of_a_page_in_order() {
        let (draws, pages, solid) = sample();
        let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
        let Plan {
            phase1_calls,
            solid_calls,
            total,
            ..
        } = plan(
            &draws,
            &pages,
            solid,
            solid,
            [draws.len(); 2],
            4,
            &mut calls,
            &mut ranges,
            &mut segments,
        );
        assert_eq!(total, 6 * 13);
        assert_eq!(phase1_calls, solid_calls);
        let call = |page, first, count| Call { page, first, count };
        // Pages 0, 0 | 1 | 0, then translucent 0, 0 (never merged with solid).
        assert_eq!(
            calls,
            [
                call(0, 0, 30),
                call(1, 30, 6),
                call(0, 36, 24),
                call(0, 60, 18)
            ]
        );
        assert_eq!(solid_calls, 3);
        // Solid ranges, then each sorted page's, every group padded to 4.
        assert_eq!(
            segments,
            [
                (Source::Quads, 0, 4),
                (Source::Sorted(0), 4, 1),
                (Source::Sorted(1), 8, 1)
            ]
        );
        assert_eq!(ranges[3], [24, QUADS, 1024, 36]);
        assert_eq!(ranges[4], [12, 6, 768, 60]);
        assert_eq!(ranges[8], [6, 0, 40, 72]);
        assert_eq!(ranges.len(), 12);
    }

    /// A shader pack's shadow draws follow the camera's, in calls of their
    /// own even from the same page: solid ones, then translucent ones.
    #[test]
    fn plan_gives_the_shadow_lists_calls_of_their_own() {
        let (mut draws, mut pages, solid) = sample();
        let camera = draws.len();
        draws.extend([[6, 1, 0, 40, 6], [6 * 2, 1, 0, 64, 7], [6, 1, 6, 80, 7]]);
        pages.extend([(0, None), (0, None), (0, Some(0))]);
        let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
        let plan = plan(
            &draws,
            &pages,
            solid,
            solid,
            [camera, camera + 2],
            4,
            &mut calls,
            &mut ranges,
            &mut segments,
        );
        assert_eq!((plan.solid_calls, plan.shadow_calls), (3, [4, 5]));
        let call = |page, first, count| Call { page, first, count };
        assert_eq!(
            &calls[3..],
            [call(0, 60, 18), call(0, 78, 18), call(0, 96, 6)]
        );
        assert_eq!(plan.total, 6 * 17);
    }

    /// Phase 2's draws (here the third and fourth) get calls of their own,
    /// each with room for all of its draws, and ranges naming their draw
    /// and call in phase 2.
    #[test]
    fn plan_gives_phase_2_calls_of_its_own() {
        let (draws, pages, solid) = sample();
        let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
        let plan = plan(
            &draws,
            &pages,
            2,
            solid,
            [draws.len(); 2],
            4,
            &mut calls,
            &mut ranges,
            &mut segments,
        );
        assert_eq!(
            plan,
            Plan {
                phase1_calls: 1,
                solid_calls: 3,
                shadow_calls: [4, 4],
                total: 6 * 13
            }
        );
        let call = |page, first, count| Call { page, first, count };
        // Phase 1: page 0 | phase 2: page 1, page 0 | translucent: page 0.
        assert_eq!(
            calls,
            [
                call(0, 0, 30),
                call(1, 30, 6),
                call(0, 36, 24),
                call(0, 60, 18)
            ]
        );
        assert_eq!(segments[1], (Source::Seen, 4, 2));
        assert_eq!(ranges[4], [6, 0, 256, 0]);
        assert_eq!(ranges[5], [24, 1, 1024, 1]);
    }

    /// The compute pass writes exactly the indices the per-section draws
    /// read: quads 0 1 2 0 2 3 for solid draws, sorted indices otherwise,
    /// plus the base vertex. With culling, phase 2's runs hold the indices
    /// of the draws the test saw, in order, and their calls count them.
    #[test]
    fn flatten_writes_the_indices_of_the_per_section_draws() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            eprintln!("no GPU device: skipped");
            return;
        };
        use wgpu::util::DeviceExt;
        let (draws, pages, solid) = sample();
        let align = (u64::from(device.limits().min_storage_buffer_offset_alignment) / 16).max(1);
        let sorted: [Vec<u32>; 2] = [
            (0..32).map(|i| (i * 7) % 32).collect(),
            (0..8).rev().collect(),
        ];
        let indices = |d: &[u32; 5], sorted_page: Option<u32>| -> Vec<u32> {
            (0..d[0])
                .map(|k| {
                    d[3] + match sorted_page {
                        None => (k / 6) * 4 + [0, 1, 2, 0, 2, 3][(k % 6) as usize],
                        Some(p) => sorted[p as usize][(d[2] + k) as usize],
                    }
                })
                .collect()
        };
        let storage = |entries: &[u32], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(entries),
                usage: wgpu::BufferUsages::STORAGE | usage,
            })
        };
        let none = wgpu::BufferUsages::empty();
        let sorted_buffers = [
            storage(&sorted[0], none),
            storage(&sorted[1], none),
            storage(&[0; 4], none),
        ];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("flatten.wgsl").into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let (flatten, flatten_seen) = (pipeline("flatten"), pipeline("flatten_seen"));
        let read_back = |buffer: &wgpu::Buffer, words: u64| -> Vec<u32> {
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: words * 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(buffer, 0, &read, 0, words * 4);
            queue.submit([encoder.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec()
        };
        // Without culling, then with the third and fourth draws in phase 2,
        // of which the test saw only the fourth.
        for phase1 in [solid, 2] {
            let (mut calls, mut ranges, mut segments) = (Vec::new(), Vec::new(), Vec::new());
            let plan = plan(
                &draws,
                &pages,
                phase1,
                solid,
                [draws.len(); 2],
                align,
                &mut calls,
                &mut ranges,
                &mut segments,
            );
            // Phase 2's draws as the test leaves them: instance counts of 0,
            // 1 where it saw the section.
            let mut phase2: Vec<[u32; 5]> = draws[phase1..solid]
                .iter()
                .map(|&[count, _, first, base, instance]| [count, 0, first, base, instance])
                .collect();
            if let Some(last) = phase2.last_mut() {
                last[1] = 1;
            }
            let phase2_calls: Vec<[u32; 5]> = calls[plan.phase1_calls..plan.solid_calls]
                .iter()
                .map(|c| [0, 1, c.first, 0, 0])
                .collect();
            let range_buffer = storage(bytemuck::cast_slice(&ranges), none);
            let phase2_buffer = storage(
                bytemuck::cast_slice(&[phase2.as_slice(), &[[0; 5]]].concat()),
                none,
            );
            let calls_buffer = storage(
                bytemuck::cast_slice(&[phase2_calls.as_slice(), &[[0; 5]]].concat()),
                wgpu::BufferUsages::COPY_SRC,
            );
            let out = storage(&vec![0; plan.total as usize], wgpu::BufferUsages::COPY_SRC);
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                for &(source, first, count) in &segments {
                    let ranges = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &range_buffer,
                        offset: first * 16,
                        size: wgpu::BufferSize::new(count * 16),
                    });
                    let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
                    let (pipeline, entries) = match source {
                        Source::Seen => (
                            &flatten_seen,
                            vec![
                                entry(0, ranges),
                                entry(2, out.as_entire_binding()),
                                entry(3, phase2_buffer.as_entire_binding()),
                                entry(4, calls_buffer.as_entire_binding()),
                            ],
                        ),
                        Source::Quads | Source::Sorted(_) => {
                            let page = match source {
                                Source::Sorted(p) => p as usize,
                                _ => 2,
                            };
                            (
                                &flatten,
                                vec![
                                    entry(0, ranges),
                                    entry(1, sorted_buffers[page].as_entire_binding()),
                                    entry(2, out.as_entire_binding()),
                                ],
                            )
                        }
                    };
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &entries,
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(count as u32, 1, 1);
                }
            }
            queue.submit([encoder.finish()]);
            let got = read_back(&out, plan.total);
            // Phase 1's and the translucent calls: each of their draws, in order.
            let expected: Vec<u32> = (0..draws.len())
                .filter(|i| !(phase1..solid).contains(i))
                .flat_map(|i| indices(&draws[i], pages[i].1))
                .collect();
            let drawn: Vec<u32> = calls
                .iter()
                .enumerate()
                .filter(|(c, _)| !(plan.phase1_calls..plan.solid_calls).contains(c))
                .flat_map(|(_, c)| got[c.first as usize..(c.first + c.count) as usize].to_vec())
                .collect();
            assert_eq!(drawn, expected);
            // Phase 2: page 1's run drew nothing; page 0's holds the fourth
            // draw's indices.
            if phase1 < solid {
                let counts = read_back(&calls_buffer, phase2_calls.len() as u64 * 5);
                assert_eq!(counts[0], 0);
                assert_eq!(counts[5], draws[3][0]);
                let run = &calls[plan.phase1_calls + 1];
                assert_eq!(
                    got[run.first as usize..(run.first + draws[3][0]) as usize],
                    indices(&draws[3], None)
                );
            }
        }
    }
}
