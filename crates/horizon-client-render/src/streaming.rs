//! Chunk streaming: a worker pool generates, lights and meshes chunk columns
//! around the player, closest first. The game thread owns the world and only
//! hands immutable `Arc` snapshots to workers; results carry a sequence number
//! so stale work is dropped.
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::section_mask::SectionMask;
use crate::{
    chunk::Chunk,
    graphics::LeafCullingMode,
    light::{LightChanges, LightTables, apply_lighting, apply_relighting, light_chunk},
    mesher::{MeshScratch, MeshTables, Mesher, Region, SectionMesh, borrow},
    world::World,
};
use horizon_core::biome::BiomeZoom;
use horizon_world::block_render::FaceVisibility;
use horizon_world::worldgen::decoration_order::DecorationOrder;
use std::{
    collections::{BinaryHeap, HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, RwLock, TryLockError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

/// How long a worker waits with nothing to do before it gives back its
/// generator's scratch buffers ([`ChunkGenerator::idle`]).
pub const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(5);

/// A chunk under generation, opaque to the streamer.
pub type Proto = Box<dyn std::any::Any + Send>;
/// Chunk generation in vanilla's stages: a neighbour-independent proto chunk
/// (biomes, noise, surface, carvers), decoration of a 3x3 neighbourhood
/// (FEATURES, which may write into neighbours), then conversion of a chunk
/// whose eight neighbours are decorated into a final column. Implementations
/// must be thread-safe; each worker thread owns one `Worker`.
pub trait ChunkGenerator: Send + Sync + 'static {
    type Worker: Send + 'static;
    fn worker(&self) -> Self::Worker;
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler>
    where
        Self: Sized,
    {
        let worker = self.worker();
        Box::new(GeneratorSampler {
            generator: self,
            worker,
        })
    }
    /// An undecorated far-terrain column, without allocating gameplay chunks.
    fn lod_surface(
        &self,
        _worker: &mut Self::Worker,
        _x: i32,
        _z: i32,
    ) -> Option<crate::lod::Surface> {
        None
    }
    /// A previously saved final chunk, bypassing generation.
    fn load(&self, _x: i32, _z: i32) -> Option<Chunk> {
        None
    }
    /// What a loaded chunk needs from the generator before the game uses it,
    /// on the worker that loaded it.
    fn prepare_loaded(&self, _worker: &mut Self::Worker, _chunk: &Chunk) {}
    /// The worker has had nothing to do for [`IDLE_RELEASE`]: give back what
    /// it keeps between jobs only to go faster.
    fn idle(&self, _worker: &mut Self::Worker) {}
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Proto;
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
    /// Whether a chunk next to final ones (loaded from the save, as the
    /// chunks of a world an older version made) is decorated, its features
    /// spilling into them (`proto_beside_final`, `final_writes`), as vanilla's
    /// are; otherwise it is left undecorated.
    fn decorates_beside_final(&self) -> bool {
        false
    }
    /// The proto chunk of a final chunk, for a decoration region to hold: its
    /// blocks are `snapshot`'s.
    fn proto_beside_final(&self, _worker: &mut Self::Worker, _snapshot: &Chunk) -> Option<Proto> {
        None
    }
    /// What decoration wrote into the proto chunk of a final chunk: the blocks
    /// that differ from `snapshot`, as `([x, y, z], state)`.
    fn final_writes(&self, _snapshot: &Chunk, _proto: &Proto) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
    /// The block entities decoration made at the blocks `writes` put into a
    /// final chunk (a chest of a structure piece, a spawner), with their
    /// contents.
    fn final_block_entities(
        &self,
        _proto: &Proto,
        _writes: &[([i32; 3], u32)],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        Vec::new()
    }
    fn finish(&self, proto: Proto) -> Chunk;
    /// Whether `proto` has been decorated (a saved one can come back decorated).
    fn is_decorated(&self, _proto: &Proto) -> bool {
        false
    }
    /// What to save of a proto chunk dropped before it became final.
    /// A decorated proto chunk's blocks as a column, to light a finished
    /// neighbour against (vanilla lights with neighbours at FEATURES).
    fn light_view(&self, _proto: &Proto) -> Option<Chunk> {
        None
    }
    /// A decorated proto chunk's column sections (`Chunk::sections`
    /// indexes) holding a block other than air: what its neighbours' light
    /// layers count once it is at INITIALIZE_LIGHT.
    fn non_air_sections(&self, _proto: &Proto) -> Option<SectionMask> {
        None
    }
    fn saved_proto(&self, _proto: &Proto) -> Option<crate::chunk::SavedProto> {
        None
    }
    /// Put a saved proto chunk's state back onto `proto`, generated again.
    fn restore_proto(&self, _proto: &mut Proto, _saved: crate::chunk::SavedProto) {}
    /// Save proto chunks decoration wrote to, so that they come back as they
    /// were: generating them again would lose those writes.
    fn save_protos(&self, _protos: &[&Proto]) {}
    /// Save a column that finished after the player moved away: its
    /// neighbours' decoration is done, so it can't be generated again.
    fn save_finished(&self, _chunk: Chunk) {}
}
pub type LodDetail = ((i32, i32), Arc<[crate::lod::Column; 256]>);

/// Independent, thread-owned far-terrain sampling state.
pub trait LodSampler: Send {
    /// Independent sampling state; native detail generation stays with the owner.
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        None
    }
    fn request_detail(&mut self, _chunks: &[(i32, i32)], _center: [i32; 2]) {}
    fn drain_details(&mut self) -> Vec<LodDetail> {
        Vec::new()
    }
    /// Reduced far detail (`graphics::FarDetail::Reduced`): `grid` may then
    /// approximate wide cells.
    fn set_approximate(&mut self, _approximate: bool) {}
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface>;
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        crate::lod::Column::from_surface(self.sample(x, z))
    }
    fn reduced_column(&mut self, x: i32, z: i32, _width: i32) -> crate::lod::Column {
        self.column(x, z)
    }
    /// `side` x `side` columns from (`x`, `z`), `stride` blocks apart; none
    /// when sampling failed and should be tried again later.
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        let mut columns = Vec::with_capacity(side * side);
        for dz in 0..side {
            for dx in 0..side {
                columns.push(self.reduced_column(
                    x + dx as i32 * stride,
                    z + dz as i32 * stride,
                    stride,
                ));
            }
        }
        columns
    }
}
pub trait LodSource: Send + Sync {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler>;
}
struct GeneratorSampler<G: ChunkGenerator> {
    generator: Arc<G>,
    worker: G::Worker,
}
impl<G: ChunkGenerator> LodSampler for GeneratorSampler<G> {
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
        self.generator.lod_surface(&mut self.worker, x, z)
    }
}
impl<G: ChunkGenerator> LodSource for G {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        ChunkGenerator::lod_sampler(self)
    }
}
/// The generator's saving, for the streamer (which doesn't know its type).
trait Saves: LodSource {
    fn save_protos(&self, protos: &[&Proto]);
    fn save_finished(&self, chunk: Chunk);
}
impl<G: ChunkGenerator> Saves for G {
    fn save_protos(&self, protos: &[&Proto]) {
        ChunkGenerator::save_protos(self, protos);
    }
    fn save_finished(&self, chunk: Chunk) {
        ChunkGenerator::save_finished(self, chunk);
    }
}
/// Per-job mesher inputs that change with the dimension or the options.
#[derive(Clone)]
struct MeshSettings {
    nether: bool,
    shader_pack_lighting: bool,
    zoom: BiomeZoom,
    blend_radius: i32,
    smooth_lighting: bool,
    cutout_leaves: bool,
    translucent_layers: Arc<[bool]>,
    leaf_culling: LeafCullingMode,
}
enum Job {
    Generate(i32, i32),
    /// A region to decorate, with the final chunks in it (region index and
    /// the chunk as it is): their slots hold nothing until the worker makes
    /// their proto chunks.
    Decorate(i32, i32, Box<[Proto; 9]>, Vec<(usize, Chunk)>),
    Finish(Proto),
    Light(i32, i32, Box<Region>, u64, bool),
    Mesh(i32, i32, i32, Box<Region>, u64, MeshSettings),
    /// Proto chunks checked out to be saved (`ChunkGenerator::save_protos`),
    /// off the main thread like vanilla's IOWorker.
    Save(Vec<((i32, i32), Proto)>),
    /// A proto chunk checked out for its light view (`light_region`).
    LightView(i32, i32, Proto),
    /// Meshes the renderer is done with (`Streaming::free`): their memory
    /// is given back here, not in the frame.
    Free(Vec<((i32, i32, i32), SectionMesh)>),
}
/// What decoration wrote into the final chunks of its region.
#[derive(Default)]
pub struct FinalWrites {
    blocks: Vec<([i32; 3], u32)>,
    block_entities: Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)>,
}
pub enum Done {
    Generated(Chunk),
    /// A proto chunk, and whether it was decorated (loaded so from the save).
    Proto(i32, i32, Proto, bool),
    /// A decorated region, the region indices of its final chunks and what
    /// decoration wrote into them.
    Decorated(i32, i32, Box<[Proto; 9]>, Vec<usize>, FinalWrites),
    Lit(i32, i32, Vec<crate::chunk::Light>, u64),
    Meshed(i32, i32, i32, SectionMesh, u64),
    Saved(Vec<((i32, i32), Proto)>),
    /// A proto chunk back from `Job::LightView`, with its view.
    LightView(i32, i32, Proto, Option<Chunk>),
}
struct Queued {
    priority: i64,
    /// The column the job is for and what it adds to the column's distance
    /// (`Streaming::submit`): the priority is computed again from them when
    /// the centre moves (`Streaming::requeue`). None for saves, which go first.
    at: Option<(i32, i32)>,
    bias: i64,
    order: u64,
    job: Job,
}
impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.priority == o.priority && self.order == o.order
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap: lower priority value = more urgent.
        o.priority
            .cmp(&self.priority)
            .then(o.order.cmp(&self.order))
    }
}
struct Shared {
    queue: Mutex<BinaryHeap<Queued>>,
    ready: Condvar,
    stop: AtomicBool,
    /// What meshing reads (block models and sprites), swapped when resource
    /// packs reload; each mesh job takes the current one.
    context: RwLock<Arc<WorkerContext>>,
    done: Mailbox,
    /// Only workers use this pool; frames never wait for its lock. Retaining
    /// cleared buffers avoids racing the renderer for the process heap.
    meshes: Mutex<MeshPool>,
    /// Workers that have freed their buffers ([`IDLE_RELEASE`]) and taken no
    /// job since: the last of them gives the freed memory back to the OS.
    released: AtomicUsize,
    /// The workers that started.
    workers: AtomicUsize,
}
#[derive(Default)]
struct MeshPool {
    meshes: Vec<SectionMesh>,
    bytes: usize,
}
impl MeshPool {
    const BYTES: usize = 16 << 20;
    const COUNT: usize = 128;
    fn take(&mut self) -> SectionMesh {
        let mesh = self.meshes.pop().unwrap_or_default();
        self.bytes -= mesh.capacity_bytes();
        mesh
    }
    /// A rejected mesh is dropped by the caller, after releasing the lock.
    fn put(&mut self, mut mesh: SectionMesh) -> Option<SectionMesh> {
        let bytes = mesh.capacity_bytes();
        if bytes == 0 || self.meshes.len() == Self::COUNT || bytes > Self::BYTES - self.bytes {
            return Some(mesh);
        }
        mesh.clear();
        self.bytes += bytes;
        self.meshes.push(mesh);
        None
    }
}

/// Finished jobs, from the workers to the game thread, in the order they
/// finished. Not an `mpsc` channel: its receiver waits for a sender it finds
/// half way through a send, and a worker descheduled there on a busy machine
/// held the frame for a scheduler quantum (10 to 30 ms, the RD 32 ocean
/// flight's 1% lows). The frame takes what is there without waiting; while a
/// worker holds the lock, it takes it the next frame.
#[derive(Default)]
struct Mailbox {
    done: Mutex<VecDeque<Done>>,
    arrived: Condvar,
}
impl Mailbox {
    fn send(&self, done: Done) {
        self.done.lock().unwrap().push_back(done);
        self.arrived.notify_one();
    }
    fn try_lock(&self) -> Option<MutexGuard<'_, VecDeque<Done>>> {
        match self.done.try_lock() {
            Ok(done) => Some(done),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(e)) => panic!("{e}"),
        }
    }
    /// Every finished job, or none while a worker is sending.
    fn take(&self) -> VecDeque<Done> {
        self.try_lock()
            .map(|mut done| std::mem::take(&mut *done))
            .unwrap_or_default()
    }
    fn try_recv(&self) -> Option<Done> {
        self.try_lock()?.pop_front()
    }
    fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Done> {
        let done = self.done.lock().unwrap();
        let (mut done, _) = self
            .arrived
            .wait_timeout_while(done, timeout, |done| done.is_empty())
            .unwrap();
        done.pop_front()
    }
}
pub struct Streaming {
    shared: Arc<Shared>,
    /// Columns generated and saved as they were left behind: new when they
    /// next arrive, though read from the save (`World::loaded_chunks`).
    dropped_new: FxHashSet<(i32, i32)>,
    non_air_sections: Arc<dyn Fn(&Proto) -> Option<SectionMask> + Send + Sync>,
    threads: Vec<thread::JoinHandle<()>>,
    order: u64,
    /// Ticks since the columns and protos that fell out of range were swept.
    scan_age: u8,
    /// The centre and radius the sweep was last reset for.
    scan_area: ((i32, i32), i32),
    generating: FxHashSet<(i32, i32)>,
    generation_scan: GenerationScan,
    /// Proto chunks: `None` while checked out to a decoration or save job.
    protos: FxHashMap<(i32, i32), Option<Proto>>,
    /// The light jobs' views of proto chunks past the area (`light_region`),
    /// built once per proto: each is a neighbour of up to three columns and
    /// waits in failed regions until the last one loads. Dropped whenever
    /// the proto is checked out, replaced or removed.
    light_views: FxHashMap<(i32, i32), Arc<Chunk>>,
    decorated: FxHashSet<(i32, i32)>,
    decorating: usize,
    /// The final chunks a running decoration writes into: another one that
    /// reaches them waits, as vanilla's features run one after the other and
    /// each sees what the one before placed.
    decorating_finals: HashSet<(i32, i32)>,
    /// Save jobs in flight.
    saving: usize,
    /// Proto chunks checked out to a worker for their light view.
    viewing: FxHashSet<(i32, i32)>,
    /// Vanilla's decoration order around the player, and the centre and
    /// radius it was last ranked for.
    decoration_order: DecorationOrder,
    order_area: Option<((i32, i32), i32)>,
    /// What the queued jobs' priorities were computed from (`requeue`): the
    /// centre, the loading screen, the band, and the extra areas.
    queue_basis: (((i32, i32), bool, Option<Band>), Vec<((i32, i32), i32)>),
    finishing: FxHashSet<(i32, i32)>,
    decorates: bool,
    /// `ChunkGenerator::decorates_beside_final`.
    saves_beside_final: bool,
    lighting: FxHashMap<(i32, i32), u64>,
    meshing: FxHashMap<(i32, i32, i32), u64>,
    /// The sequence each section's latest accepted mesh was dispatched at
    /// (`CompiledSectionMesh.compileTaskStartTimeNs`).
    compile_started: FxHashMap<(i32, i32, i32), u64>,
    /// Chunks whose light is current.
    lit: FxHashSet<(i32, i32)>,
    /// Columns whose sections have been meshed at least once.
    meshed: FxHashSet<(i32, i32)>,
    /// Dirty sections of loaded columns past the render distance, meshed once
    /// the player comes closer, and the area they were set aside for.
    waiting: FxHashSet<(i32, i32, i32)>,
    waiting_area: ((i32, i32), i32, usize),
    /// Proto chunks decoration wrote to since they were generated or saved.
    written: FxHashSet<(i32, i32)>,
    saves: Arc<dyn Saves>,
    /// Results received while `save` waited, for the next `update`.
    deferred: Vec<Done>,
    sequence: u64,
    pub center: (i32, i32),
    /// The view distance around `center`; below 0, no area there (a dedicated
    /// server nobody is on).
    pub radius: i32,
    /// `/forceload`ed chunks: kept loaded whatever the distance.
    pub forced: HashSet<(i32, i32)>,
    /// No work is scheduled while paused (title screen).
    pub paused: bool,
    /// The level loading screen is up: work goes ring by ring (`Ranges::rings`).
    pub loading: bool,
    /// Dimension flags: sky light, Nether face shading.
    pub sky_light: bool,
    pub nether_shading: bool,
    /// A shader pack lights the faces: sections mesh without face shading
    /// (`Mesher::shader_pack_lighting`).
    pub shader_pack_lighting: bool,
    /// `BiomeManager`'s zoom seed (the hashed world seed) and `biomeBlendRadius`.
    pub biome_zoom: BiomeZoom,
    pub biome_blend: i32,
    /// The `ao` and `cutoutLeaves` options.
    pub smooth_lighting: bool,
    pub cutout_leaves: bool,
    /// Sprite layers a shader pack draws in the translucent pass (`[[materials]]
    /// translucent`), by layer; empty without one.
    pub translucent_layers: Arc<[bool]>,
    /// What the mesher does with leaf faces inside a canopy (`LeafCullingMode`).
    pub leaf_culling: LeafCullingMode,
    /// Incremental relighting of block edits on lit chunks (see `light::relight`).
    light_tables: Arc<LightTables>,
    /// Applied edits by sequence, replayed onto light jobs snapshotted before them.
    light_log: Vec<(u64, (i32, i32, i32))>,
    light_edit_sequence: u64,
    /// Edit sequence at each in-flight light job's snapshot.
    light_snapshots: FxHashMap<(i32, i32), u64>,
    pub generated_total: u64,
    pub meshed_total: u64,
    /// Chunks come from a server (`insert_remote`/`forget`): nothing is
    /// generated and columns are only unloaded when the server says so.
    pub remote: bool,
    /// More areas to keep loaded (centre chunk, radius), e.g. around the
    /// players of a server. Each is a square, like vanilla's ticket levels:
    /// chunks finish out to `radius + 1` (Chebyshev) and generation reaches
    /// past that as around the player. They are generated and kept but not meshed.
    pub extra: Vec<((i32, i32), i32)>,
    /// `extra`'s squares as a grid, made again when `extra` changes.
    cover: Option<ExtraCover>,
    /// Mesh the sections for drawing (false on a server thread).
    pub mesh: bool,
    /// The pregeneration band (`set_band`).
    band: Option<Band>,
    /// The columns and the next row the band's decoration ranks were given
    /// for (`set_band`).
    band_ranked: Option<(i32, i32, i32)>,
}
/// Snapshot only after a scan found no missing candidates. Public range fields
/// and `World::chunks` can change without going through Streaming, so compare
/// their values and structural membership token rather than a frame timeout.
#[derive(Default)]
struct GenerationScan {
    exhausted: bool,
    center: (i32, i32),
    radius: i32,
    extra: Vec<((i32, i32), i32)>,
    forced: HashSet<(i32, i32)>,
    band: Option<Band>,
    chunk_revision: u64,
}
impl GenerationScan {
    fn unchanged(&self, ranges: &Ranges<'_>, world: &World) -> bool {
        self.exhausted
            && self.center == ranges.center
            && self.radius == ranges.radius
            && self.extra == ranges.extra
            && self.forced == *ranges.forced
            && self.band.as_ref() == ranges.band
            && self.chunk_revision == world.chunks.membership_revision()
    }
    fn remember(&mut self, ranges: Ranges<'_>, world: &World) {
        self.exhausted = true;
        self.center = ranges.center;
        self.radius = ranges.radius;
        self.extra.clear();
        self.extra.extend_from_slice(ranges.extra);
        self.forced.clone_from(ranges.forced);
        self.band = ranges.band.copied();
        self.chunk_revision = world.chunks.membership_revision();
    }
}
struct GenerationInputs<'a> {
    ranges: Ranges<'a>,
    world: &'a World,
    generating: &'a FxHashSet<(i32, i32)>,
    protos: &'a FxHashMap<(i32, i32), Option<Proto>>,
    finishing: &'a FxHashSet<(i32, i32)>,
    decorates: bool,
    remote: bool,
    loading: bool,
}
impl GenerationScan {
    /// None means the scan was skipped; Some preserves the original candidate
    /// ordering and bounded batch, including an empty exhausted scan.
    fn candidates(
        &mut self,
        input: GenerationInputs<'_>,
        capacity: usize,
    ) -> Option<Vec<(i64, i32, i32)>> {
        // Generation (load or proto), closest first, bounded in flight.
        let (margin, reach) = Ranges::margins(input.decorates);
        if input.remote
            || input.generating.len() >= capacity
            || self.unchanged(&input.ranges, input.world)
        {
            return None;
        }
        {
            let mut wanted = Vec::new();
            // Each area's square of columns, as one run per row. Players'
            // areas overlap (a server's players often stand together), so
            // the runs of a row are merged and each column is looked at once.
            let mut runs: Vec<(i32, i32, i32)> = Vec::new();
            for (center, radius) in std::iter::once((input.ranges.center, input.ranges.radius))
                .chain(input.ranges.extra.iter().copied())
            {
                let r = radius + margin.max(reach + 1);
                runs.extend((center.1 - r..=center.1 + r).map(|z| (z, center.0 - r, center.0 + r)));
            }
            if let Some(band) = input.ranges.band {
                let r = margin.max(reach + 1);
                runs.extend((band.z0 - r..=band.z1 + r).map(|z| (z, band.x0 - r, band.x1 + r)));
            }
            runs.sort_unstable();
            let mut runs = runs.into_iter().peekable();
            while let Some((z, x0, mut x1)) = runs.next() {
                while let Some((_, _, next_x1)) =
                    runs.next_if(|&(nz, nx0, _)| nz == z && nx0 <= x1.saturating_add(1))
                {
                    x1 = x1.max(next_x1);
                }
                for x in x0..=x1 {
                    if !input.ranges.requested(x, z, input.decorates)
                        || input.world.chunks.contains_key(&(x, z))
                        || input.generating.contains(&(x, z))
                        || input.protos.contains_key(&(x, z))
                        || input.finishing.contains(&(x, z))
                    {
                        continue;
                    }
                    wanted.push((input.ranges.priority(x, z, input.loading), x, z));
                }
            }
            // Forced chunks and their generation neighbourhood: the ring that
            // is decorated (or finished, without decoration) and the one its
            // 3x3 needs.
            let reach = if input.decorates {
                FORCED_TERRAIN_RING
            } else {
                FORCED_FEATURES_RING
            };
            for &(fx, fz) in input.ranges.forced {
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, z) = (fx + dx, fz + dz);
                        if input.world.chunks.contains_key(&(x, z))
                            || input.generating.contains(&(x, z))
                            || input.protos.contains_key(&(x, z))
                            || input.finishing.contains(&(x, z))
                        {
                            continue;
                        }
                        wanted.push((input.ranges.priority(x, z, input.loading), x, z));
                    }
                }
            }
            if wanted.is_empty() {
                let ranges = Ranges {
                    center: input.ranges.center,
                    radius: input.ranges.radius,
                    extra: input.ranges.extra,
                    forced: input.ranges.forced,
                    cover: input.ranges.cover,
                    band: input.ranges.band,
                };
                self.remember(ranges, input.world);
            } else {
                // A bounded batch may leave candidates behind. Keep scanning
                // as capacity opens, even if no range or membership changed.
                self.exhausted = false;
            }
            wanted.sort_unstable();
            wanted.dedup();
            wanted.truncate(capacity - input.generating.len());
            Some(wanted)
        }
    }
}
pub struct WorkerContext {
    pub assets: Arc<crate::assets::Assets>,
    pub render: Arc<crate::assets::RenderAssets>,
    pub tables: Arc<MeshTables>,
    pub biomes: Arc<crate::biome::Biomes>,
    pub light: Arc<LightTables>,
}
impl Streaming {
    pub fn new<G: ChunkGenerator>(
        generator: Arc<G>,
        context: Arc<WorkerContext>,
        threads: usize,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(BinaryHeap::new()),
            ready: Condvar::new(),
            stop: AtomicBool::new(false),
            context: RwLock::new(context.clone()),
            done: Mailbox::default(),
            meshes: Mutex::new(MeshPool::default()),
            released: AtomicUsize::new(0),
            workers: AtomicUsize::new(0),
        });
        let light_tables = context.light.clone();
        let saves: Arc<dyn Saves> = generator.clone();
        let threads = (0..threads.max(1))
            .map(|i| {
                let shared = shared.clone();
                let generator = generator.clone();
                let light = context.light.clone();
                thread::Builder::new()
                    .name(format!("horizon-worker-{i}"))
                    .spawn(move || {
                        // Ticks and frames come first (horizon_threads).
                        horizon_threads::behind_server();
                        let mut worker = generator.worker();
                        // The face visibility table and its occlusion cache stay with
                        // the worker across mesh jobs, like vanilla's per-thread cache:
                        // cloning it per job copied every state and face shape.
                        let mut faces: Option<(Arc<MeshTables>, FaceVisibility)> = None;
                        let mut mesh_scratch = MeshScratch::default();
                        loop {
                            let job = {
                                let mut queue = shared.queue.lock().unwrap();
                                let mut released = false;
                                loop {
                                    if shared.stop.load(Ordering::Relaxed) {
                                        return;
                                    }
                                    if let Some(job) = queue.pop() {
                                        if released {
                                            shared.released.fetch_sub(1, Ordering::Relaxed);
                                        }
                                        break job.job;
                                    }
                                    if released {
                                        queue = shared.ready.wait(queue).unwrap();
                                        continue;
                                    }
                                    let (woken, wait) =
                                        shared.ready.wait_timeout(queue, IDLE_RELEASE).unwrap();
                                    queue = woken;
                                    if wait.timed_out() && queue.is_empty() {
                                        // A server nobody is on, a client standing still:
                                        // free the generator's buffers (and the mesher's
                                        // face cache and scratch) until the next job
                                        // needs them.
                                        drop(queue);
                                        generator.idle(&mut worker);
                                        faces = None;
                                        mesh_scratch = MeshScratch::default();
                                        released = true;
                                        // The allocator keeps freed pages until memory
                                        // runs short: once every worker has freed its own,
                                        // hand them back.
                                        if shared.released.fetch_add(1, Ordering::Relaxed) + 1
                                            == shared.workers.load(Ordering::Relaxed)
                                        {
                                            // No worker needs retained mesh capacity now.
                                            // Drop it outside the pool lock before asking
                                            // the allocator to return freed pages.
                                            let meshes =
                                                std::mem::take(&mut *shared.meshes.lock().unwrap());
                                            drop(meshes);
                                            horizon_threads::release_free_memory();
                                        }
                                        queue = shared.queue.lock().unwrap();
                                    }
                                }
                            };
                            let done = match job {
                                Job::Generate(x, z) => match generator.load(x, z) {
                                    Some(chunk) => {
                                        generator.prepare_loaded(&mut worker, &chunk);
                                        Done::Generated(chunk)
                                    }
                                    None => {
                                        let proto = generator.proto(&mut worker, x, z);
                                        let decorated = generator.is_decorated(&proto);
                                        Done::Proto(x, z, proto, decorated)
                                    }
                                },
                                Job::Decorate(x, z, mut region, finals) => {
                                    // The final chunks' proto chunks, made here: a
                                    // generated proto chunk holding their blocks.
                                    for (i, snapshot) in &finals {
                                        if let Some(proto) =
                                            generator.proto_beside_final(&mut worker, snapshot)
                                        {
                                            region[*i] = proto;
                                        }
                                    }
                                    generator.decorate(&mut worker, &mut region);
                                    let mut writes = FinalWrites::default();
                                    for (i, snapshot) in &finals {
                                        let blocks = generator.final_writes(snapshot, &region[*i]);
                                        writes.block_entities.extend(
                                            generator.final_block_entities(&region[*i], &blocks),
                                        );
                                        writes.blocks.extend(blocks);
                                        region[*i] = Box::new(());
                                    }
                                    let finals = finals.into_iter().map(|(i, _)| i).collect();
                                    Done::Decorated(x, z, region, finals, writes)
                                }
                                Job::Finish(proto) => {
                                    let mut chunk = generator.finish(proto);
                                    chunk.generated = true;
                                    Done::Generated(chunk)
                                }
                                Job::Save(protos) => {
                                    let refs: Vec<&Proto> = protos.iter().map(|(_, p)| p).collect();
                                    generator.save_protos(&refs);
                                    Done::Saved(protos)
                                }
                                Job::LightView(x, z, proto) => {
                                    let view = generator.light_view(&proto);
                                    Done::LightView(x, z, proto, view)
                                }
                                Job::Free(meshes) => {
                                    for (_, mesh) in meshes {
                                        let rejected = shared.meshes.lock().unwrap().put(mesh);
                                        drop(rejected);
                                    }
                                    continue;
                                }
                                Job::Light(x, z, region, seq, sky) => {
                                    Done::Lit(x, z, light_chunk(&borrow(&region), &light, sky), seq)
                                }
                                Job::Mesh(x, sy, z, region, seq, settings) => {
                                    let context = shared.context.read().unwrap().clone();
                                    let visibility = match faces.take() {
                                        Some((tables, visibility))
                                            if Arc::ptr_eq(&tables, &context.tables) =>
                                        {
                                            visibility
                                        }
                                        _ => context.tables.visibility.clone(),
                                    };
                                    let mut mesher = Mesher {
                                        nether_shading: settings.nether,
                                        shader_pack_lighting: settings.shader_pack_lighting,
                                        zoom: settings.zoom,
                                        blend_radius: settings.blend_radius,
                                        smooth_lighting: settings.smooth_lighting,
                                        cutout_leaves: settings.cutout_leaves,
                                        translucent_layers: &settings.translucent_layers,
                                        leaf_culling: settings.leaf_culling,
                                        assets: &context.assets,
                                        render: &context.render,
                                        tables: &context.tables,
                                        biomes: &context.biomes,
                                        visibility,
                                    };
                                    let recycled = shared.meshes.lock().unwrap().take();
                                    let mut mesh = mesher.mesh_reusing(
                                        &borrow(&region),
                                        sy,
                                        recycled,
                                        &mut mesh_scratch,
                                    );
                                    mesh.sort_solid();
                                    faces = Some((context.tables.clone(), mesher.visibility));
                                    Done::Meshed(x, sy, z, mesh, seq)
                                }
                            };
                            shared.done.send(done);
                        }
                    })
            })
            // A thread the system refuses (a container's process limit reached) leaves the pool a
            // worker short: a server that panicked here went down with its players' moves unsaved.
            .filter_map(|spawned| {
                spawned
                    .inspect_err(|e| log::error!("Horizon: chunk worker not started: {e}"))
                    .ok()
            })
            .collect::<Vec<_>>();
        shared.workers.store(threads.len(), Ordering::Relaxed);
        let non_air_sections = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.non_air_sections(p))
        };
        Self {
            shared,
            non_air_sections,
            threads,
            order: 0,
            scan_age: 0,
            scan_area: ((i32::MIN, i32::MIN), i32::MIN),
            generating: Default::default(),
            generation_scan: GenerationScan::default(),
            protos: Default::default(),
            light_views: Default::default(),
            decorated: Default::default(),
            dropped_new: Default::default(),
            decorating: 0,
            decorating_finals: HashSet::new(),
            saving: 0,
            viewing: Default::default(),
            decoration_order: DecorationOrder::default(),
            order_area: None,
            queue_basis: Default::default(),
            finishing: Default::default(),
            decorates: generator.decorates(),
            saves_beside_final: generator.decorates_beside_final(),
            lighting: Default::default(),
            meshing: Default::default(),
            compile_started: Default::default(),
            lit: Default::default(),
            meshed: Default::default(),
            waiting: Default::default(),
            waiting_area: ((0, 0), 0, 0),
            written: Default::default(),
            saves,
            deferred: Vec::new(),
            sequence: 0,
            center: (0, 0),
            radius: 8,
            forced: HashSet::new(),
            paused: false,
            loading: false,
            sky_light: true,
            nether_shading: false,
            shader_pack_lighting: false,
            biome_zoom: BiomeZoom::new(0),
            biome_blend: 2,
            smooth_lighting: true,
            cutout_leaves: true,
            translucent_layers: Arc::from([]),
            leaf_culling: LeafCullingMode::Off,
            light_tables,
            light_log: Vec::new(),
            light_edit_sequence: 0,
            light_snapshots: Default::default(),
            generated_total: 0,
            meshed_total: 0,
            remote: false,
            extra: Vec::new(),
            cover: None,
            mesh: true,
            band: None,
            band_ranked: None,
        }
    }
    /// Shares generation resources, but gives distant work its own worker state.
    pub fn lod_source(&self) -> Arc<dyn LodSource> {
        self.saves.clone()
    }
    /// Resource packs reloaded (`LevelRenderer.allChanged`): mesh with the new
    /// models and sprites, and rebuild every loaded section. Meshes in flight
    /// were made with the old resources; their results are dropped.
    pub fn reload_resources(&mut self, world: &mut World, context: Arc<WorkerContext>) {
        *self.shared.context.write().unwrap() = context;
        self.meshing.clear();
        self.compile_started.clear();
        for &(x, z) in world.chunks.keys() {
            if self.lit.contains(&(x, z)) {
                for sy in 0..world.layout().sections() as i32 {
                    world.dirty_sections.insert((x, sy, z));
                }
            }
        }
    }
    /// A column received from a server, with the light it sent
    /// (`ClientPacketListener.handleLevelChunkWithLight`): nothing is
    /// relit, and it and its neighbours remesh (`enableChunkLight`).
    pub fn insert_remote(&mut self, world: &mut World, chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        self.lighting.remove(&key);
        self.light_snapshots.remove(&key);
        world.chunks.insert(key, Arc::new(chunk));
        self.lit.insert(key);
        self.generated_total += 1;
        for dz in -1..=1 {
            for dx in -1..=1 {
                for sy in 0..world.layout().sections() as i32 {
                    world.dirty_sections.insert((key.0 + dx, sy, key.1 + dz));
                }
            }
        }
    }
    /// A resident column a server sent again whole (it changed it in bulk):
    /// its new blocks taken, the sections that did not change kept as they
    /// were, and its light and its neighbours' worked out again here, off
    /// the game thread, as light is for the blocks a server changes one at a
    /// time; the sections that changed remesh (with theirs) once lit. Until
    /// then the sections that changed show the light the packet carried (a
    /// Horizon server sends the light from before its relight, then the
    /// new light; another server's is already right, which matters where
    /// the neighbours this light needs aren't all here).
    pub fn replace_remote(&mut self, world: &mut World, mut chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        let Some(old) = world.chunks.get(&key).cloned() else {
            return self.insert_remote(world, chunk);
        };
        if !self.lit.contains(&key) {
            return self.insert_remote(world, chunk);
        }
        // The cells whose blocks changed, by section: they remesh with the neighbours within a block of them (as
        // vanilla's `setBlocksDirty`), not every section round each section changed (which, for the few blocks a
        // section often changes in a bulk edit, is mostly sections that did not).
        let mut cells = crate::light::LightChanges::default();
        cells.keyed(world.layout());
        let kept = keep_unchanged_sections(&old, &mut chunk, &mut cells);
        chunk.light_loaded = old.light_loaded;
        chunk.lit = true;
        world.chunks.insert(key, Arc::new(chunk));
        world.revision = world.revision.wrapping_add(1);
        // A section with new biomes and the same blocks remeshes now (its tint), with its neighbours, whose tint
        // blends across the border: above and below too, as a block's tint can sample the biome quart of the
        // section next to it (`BiomeManager.getBiome`'s zoom reaches a quart beyond the block).
        Self::light_updated(world, key.0, key.1, &kept.biomes);
        // A section with the same blocks and new light takes the packet's light (vanilla's
        // `handleLevelChunkWithLight` always applies it) and remeshes as a light update would.
        Self::light_updated(world, key.0, key.1, &kept.light);
        if !kept.blocks {
            return;
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                world.dirty_light.insert((key.0 + dx, key.1 + dz));
            }
        }
        cells.dirty_sections(|section| {
            world.dirty_sections.insert(section);
        });
    }
    /// A server's light update for a resident column: the sections whose
    /// light changed and their neighbours remesh
    /// (`setSectionDirtyWithNeighbors`). A resent column's sections with new
    /// biomes remesh the same way.
    pub fn light_updated(world: &mut World, x: i32, z: i32, sections: &[usize]) {
        for &sy in sections {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let s = sy as i32 + dy;
                        if (0..world.layout().sections() as i32).contains(&s) {
                            world.dirty_sections.insert((x + dx, s, z + dz));
                        }
                    }
                }
            }
        }
    }
    /// Give back meshes the renderer has copied: a worker frees them, ahead of
    /// other jobs, as vanilla's compile task closes its mesh data on its own
    /// thread once copied (`SectionRenderDispatcher.RenderSection.CompileTask.doTask`).
    /// Freed in the frame, the hundreds of meshes a chunk border brings waited
    /// on the allocator behind every busy worker: up to 20 ms of a frame each
    /// time while flying at render distance 32.
    pub fn free(&mut self, meshes: Vec<((i32, i32, i32), SectionMesh)>) {
        if !meshes.is_empty() {
            self.submit(None, 0, Job::Free(meshes));
        }
    }
    /// A column the server unloaded (ClientboundForgetLevelChunkPacket).
    pub fn forget(&mut self, world: &mut World, x: i32, z: i32) -> Option<Arc<Chunk>> {
        self.lit.remove(&(x, z));
        self.meshed.remove(&(x, z));
        self.lighting.remove(&(x, z));
        // By key, not by a pass over every pending section: a moved or
        // shrunk storage forgets thousands of columns in one frame.
        for sy in 0..world.layout().sections() as i32 {
            self.meshing.remove(&(x, sy, z));
            self.compile_started.remove(&(x, sy, z));
            self.waiting.remove(&(x, sy, z));
        }
        world.dirty_sections.take_column(x, z);
        world.chunks.remove(&(x, z))
    }
    /// Queue a job for the column `at`, nearest first: its distance (`distance`)
    /// times four plus `bias`, which orders a column's jobs. A job without a
    /// column (a save, meshes to free) goes ahead of every other.
    fn submit(&mut self, at: Option<(i32, i32)>, bias: i64, job: Job) {
        self.order += 1;
        let priority = at.map_or(i64::MIN, |(x, z)| self.distance(x, z) * 4 + bias);
        self.shared.queue.lock().unwrap().push(Queued {
            priority,
            at,
            bias,
            order: self.order,
            job,
        });
        self.shared.ready.notify_one();
    }
    /// Rank the queued jobs again when what their priorities were computed
    /// from changed: the centre, the extra areas, the band, or the loading
    /// screen (ring order, `Ranges::rings`, whose values are far above the
    /// distances used after it). Vanilla's chunk task queue follows the
    /// current ticket levels in the same way. Without it, generation queued
    /// while the loading screen was up waited behind every job queued after
    /// it, and with it the decoration of the chunks around the player: at
    /// render distance 32 nothing was decorated until every proto chunk had
    /// been generated.
    fn requeue(&mut self) {
        let basis = (self.center, self.loading, self.band);
        if self.queue_basis.0 == basis && self.queue_basis.1 == self.extra {
            return;
        }
        self.queue_basis = (basis, self.extra.clone());
        let mut queue = self.shared.queue.lock().unwrap();
        rank_again(&mut queue, |x, z| self.distance(x, z));
    }
    fn receive_decorated(
        &mut self,
        x: i32,
        z: i32,
        region: [Proto; 9],
        finals: Vec<usize>,
        writes: FinalWrites,
        world: &mut World,
    ) {
        self.generation_scan.exhausted = false;
        self.decorating -= 1;
        for i in &finals {
            self.decorating_finals
                .remove(&(x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1));
        }
        self.decorated.insert((x, z));
        for (i, proto) in region.into_iter().enumerate() {
            // A final chunk keeps its own chunk: only what decoration wrote
            // into it goes there.
            if finals.contains(&i) {
                continue;
            }
            let key = (x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1);
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
            self.written.insert(key);
        }
        for ([bx, by, bz], state) in writes.blocks {
            let _ = world.set(bx, by, bz, horizon_world::registry::BlockStateId(state));
        }
        // After the blocks: the write of a block with a block entity made a
        // default one, which the generated one replaces.
        for (pos, entity) in writes.block_entities {
            if world.chunks.contains_key(&(pos.0 >> 4, pos.2 >> 4)) {
                world.block_entities.insert(pos, entity);
            }
        }
    }
    /// Saved proto chunks come back; those now out of range are dropped by
    /// the next `update`.
    fn receive_saved(&mut self, protos: Vec<((i32, i32), Proto)>) {
        self.generation_scan.exhausted = false;
        self.saving -= 1;
        for (key, proto) in protos {
            self.light_views.remove(&key);
            self.protos.insert(key, Some(proto));
        }
    }
    /// Check out the proto chunks at `keys` and save them on a worker.
    fn save_checked_out(&mut self, keys: impl IntoIterator<Item = (i32, i32)>) {
        let protos: Vec<((i32, i32), Proto)> = keys
            .into_iter()
            .filter_map(|k| {
                let proto = self.protos.get_mut(&k)?.take()?;
                self.light_views.remove(&k);
                Some((k, proto))
            })
            .collect();
        if !protos.is_empty() {
            self.saving += 1;
            // Ahead of every other job: a column generated again meanwhile
            // would not find them saved.
            self.submit(None, 0, Job::Save(protos));
        }
    }
    fn receive_generated(&mut self, mut chunk: Chunk, world: &mut World) {
        self.generation_scan.exhausted = false;
        let key = (chunk.x, chunk.z);
        self.generating.remove(&key);
        self.finishing.remove(&key);
        self.decorated.insert(key);
        // A column that finished after the player moved away is dropped
        // whole: its block entities and saved entities too, or they would
        // stay in the world (and the entities spawn again, duplicated, when
        // the column loads later). A generated one is saved first: its
        // neighbours were decorated around it, so generating it again would
        // not give it back.
        if !self.ranges().chunk_kept(key.0, key.1, self.decorates) {
            if chunk.fresh {
                self.dropped_new.insert(key);
                self.saves.save_finished(chunk);
            }
            return;
        }
        // Generated columns are saved like edited ones, as vanilla saves
        // every chunk it generates.
        if chunk.fresh {
            world.modified.insert(key);
        }
        world
            .block_entities
            .load_chunk(std::mem::take(&mut chunk.block_entities));
        if let Some(extras) = &chunk.saved {
            for (pos, attachments) in &extras.block_entity_attachments {
                world
                    .block_entities
                    .restore_attachments(*pos, attachments.clone());
            }
        }
        world
            .block_names
            .extend(std::mem::take(&mut chunk.block_names));
        let entities = std::mem::take(&mut chunk.entities);
        if !entities.is_empty() {
            world.loaded_entities.push(((chunk.x, chunk.z), entities));
        }
        self.generated_total += 1;
        // A chunk saved with its light needs no light job; its unlit
        // neighbours still light themselves from it.
        let loaded = chunk.light_loaded;
        let new = self.dropped_new.remove(&key) || chunk.generated;
        world.loaded_chunks.push((key.0, key.1, new));
        world.chunks.insert(key, Arc::new(chunk));
        if loaded {
            self.lit.insert(key);
            for sy in 0..world.layout().sections() as i32 {
                world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (key.0 + dx, key.1 + dz);
                if !loaded || world.chunks.get(&n).is_some_and(|c| !c.light_loaded) {
                    world.dirty_light.insert(n);
                }
            }
        }
    }
    /// The order jobs around the player run in, lowest first.
    fn distance(&self, x: i32, z: i32) -> i64 {
        self.ranges().priority(x, z, self.loading)
    }
    /// The 3x3 a light job reads: the finished columns, and for a neighbour
    /// that will not finish (past the area), its decorated proto chunk, as
    /// vanilla lights its last FULL ring against neighbours at FEATURES.
    /// A proto neighbour without its view yet is checked out to a worker,
    /// which builds the view off the server thread (`Job::LightView`); the
    /// region is asked for again once it is back.
    fn light_region(&mut self, world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        let mut complete = true;
        for dz in 0..3 {
            for dx in 0..3 {
                let n = (x + dx as i32 - 1, z + dz as i32 - 1);
                region[dz][dx] = match world.chunks.get(&n) {
                    Some(chunk) => Some(chunk.clone()),
                    None if !self.in_range(n.0, n.1, 1) && self.decorated.contains(&n) => {
                        if let Some(view) = self.light_views.get(&n) {
                            Some(view.clone())
                        } else {
                            let proto = self.protos.get_mut(&n)?.take()?;
                            self.viewing.insert(n);
                            self.submit(Some((x, z)), 0, Job::LightView(n.0, n.1, proto));
                            complete = false;
                            None
                        }
                    }
                    None => return None,
                };
            }
        }
        complete.then(|| Box::new(region))
    }
    /// A proto chunk back from its light view: the view waits in
    /// `light_views` for the light jobs that asked for it.
    fn receive_light_view(&mut self, x: i32, z: i32, proto: Proto, view: Option<Chunk>) {
        self.viewing.remove(&(x, z));
        self.protos.insert((x, z), Some(proto));
        if let Some(view) = view {
            self.light_views.insert((x, z), Arc::new(view));
        }
    }
    /// Whether `light_region` would find all nine columns: the same walk
    /// without building the snapshot, so the scheduler can rank the ready
    /// columns before any region is cloned. A proto neighbour whose light
    /// view turns out empty is skipped when its region is built, and asked
    /// for again, as `light_region` returning `None` always was.
    fn light_region_ready(&self, world: &World, x: i32, z: i32) -> bool {
        light_region_ready(
            world,
            |x, z| self.in_range(x, z, 1),
            &self.decorated,
            &self.protos,
            x,
            z,
        )
    }
    fn region(world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        for dz in 0..3 {
            for dx in 0..3 {
                region[dz][dx] = Some(
                    world
                        .chunks
                        .get(&(x + dx as i32 - 1, z + dz as i32 - 1))?
                        .clone(),
                );
            }
        }
        Some(Box::new(region))
    }
    pub fn is_lit(&self, x: i32, z: i32) -> bool {
        self.lit.contains(&(x, z))
    }
    /// The non-air sections of a column that is a decorated proto chunk
    /// here (vanilla's INITIALIZE_LIGHT and on), if it is one: a level
    /// chunk's light layers count them (#4294).
    pub fn proto_sections(&self, x: i32, z: i32) -> Option<SectionMask> {
        if !self.decorated.contains(&(x, z)) {
            return None;
        }
        (self.non_air_sections)(self.protos.get(&(x, z))?.as_ref()?)
    }
    /// `ChunkMap.getLatestStatus` for the loading screen's chunk map: the
    /// furthest step this column has reached, `None` before it is asked for.
    pub fn latest_status(
        &self,
        world: &World,
        x: i32,
        z: i32,
    ) -> Option<crate::level_loading::Status> {
        use crate::level_loading::Status;
        let key = (x, z);
        Some(if self.lit.contains(&key) {
            Status::Full
        } else if world.chunks.contains_key(&key) {
            Status::InitializeLight
        } else if self.decorated.contains(&key) || self.finishing.contains(&key) {
            Status::Features
        } else if self.protos.contains_key(&key) {
            Status::Terrain
        } else if self.generating.contains(&key) {
            Status::Empty
        } else {
            return None;
        })
    }
    /// The sequence the next mesh job will be dispatched after: a clock for
    /// `compile_start`, whose jobs all snapshot the world as they are
    /// dispatched (`Util.getNanos()` at the start of a compile task).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// When the section's latest accepted mesh started compiling, on the
    /// `sequence` clock (`CompiledSectionMesh.getCompileTaskStartTime`).
    pub fn compile_start(&self, section: (i32, i32, i32)) -> Option<u64> {
        self.compile_started.get(&section).copied()
    }
    pub fn is_meshing(&self, section: (i32, i32, i32)) -> bool {
        self.meshing.contains_key(&section)
    }
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }
    /// Whether any chunk is queued or on its way from generation to a mesh.
    pub fn busy(&self) -> bool {
        self.chunks_in_flight() || !self.meshing.is_empty() || self.pending() != 0
    }
    /// Whether the near terrain is still coming in: chunks generating,
    /// decorating, finishing or being lit (or their neighbours' light views),
    /// or sections meshing for the first time. Each of those is counted from
    /// when its job is queued, so queued jobs need no separate count.
    /// Remeshing sections already drawn (block, light and fluid updates) is
    /// not loading: it kept small machines' far terrain held back
    /// (`Lod::set_near_loading`) for as long as anything nearby changed.
    pub fn terrain_loading(&self) -> bool {
        self.chunks_in_flight()
            || self
                .meshing
                .keys()
                .any(|section| !self.compile_started.contains_key(section))
    }
    fn chunks_in_flight(&self) -> bool {
        !(self.generating.is_empty()
            && self.decorating == 0
            && self.viewing.is_empty()
            && self.finishing.is_empty()
            && self.lighting.is_empty())
    }
    /// How much work each stage holds: queued jobs, then what is in flight or
    /// waiting at each step from generation to meshing.
    pub fn stages(&self) -> [(&'static str, usize); 7] {
        [
            ("queued", self.pending()),
            ("generating", self.generating.len()),
            (
                "protos",
                self.protos.values().filter(|p| p.is_some()).count(),
            ),
            ("decorating", self.decorating),
            ("finishing", self.finishing.len()),
            ("lighting", self.lighting.len()),
            ("meshing", self.meshing.len()),
        ]
    }
    /// Advance streaming: collect finished work, then schedule more.
    /// Returns meshes ready for upload and columns that were unloaded.
    pub fn update(
        &mut self,
        world: &mut World,
    ) -> (Vec<((i32, i32, i32), SectionMesh)>, Vec<Arc<Chunk>>) {
        let mut uploads = Vec::new();
        if self.paused {
            return (uploads, Vec::new());
        }
        ExtraCover::refresh(&mut self.cover, &self.extra);
        // The sweeps below walk every loaded column and proto chunk. Run them
        // at once when the view moves and every eighth tick otherwise: a
        // column can stay a little past the edge without anyone noticing.
        let area = (self.center, self.radius);
        let sweep = area != self.scan_area || self.scan_age >= 8;
        self.scan_area = area;
        self.scan_age = if sweep { 0 } else { self.scan_age + 1 };
        self.requeue();
        self.relight_edits(world);
        let deferred = std::mem::take(&mut self.deferred);
        let received = self.shared.done.take();
        // Meshes made stale meanwhile, freed with the uploaded ones (`free`).
        let mut stale = Vec::new();
        for done in deferred.into_iter().chain(received) {
            match done {
                Done::Proto(x, z, proto, decorated) => {
                    self.generation_scan.exhausted = false;
                    self.generating.remove(&(x, z));
                    if self.in_range(x, z, 4) || self.near_finished(x, z, 2) {
                        self.light_views.remove(&(x, z));
                        self.protos.insert((x, z), Some(proto));
                        if decorated || !self.decorates {
                            self.decorated.insert((x, z));
                        }
                    }
                }
                Done::Decorated(x, z, region, finals, writes) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Done::Generated(chunk) => self.receive_generated(chunk, world),
                Done::Saved(protos) => self.receive_saved(protos),
                Done::LightView(x, z, proto, view) => self.receive_light_view(x, z, proto, view),
                Done::Lit(x, z, sections, seq) => {
                    if self.lighting.get(&(x, z)) != Some(&seq) {
                        continue;
                    }
                    self.lighting.remove(&(x, z));
                    let snapshot = self.light_snapshots.remove(&(x, z)).unwrap_or(u64::MAX);
                    if let Some(chunk) = world.chunks.get_mut(&(x, z)) {
                        let chunk = Arc::make_mut(chunk);
                        // Keep blocks edited since the snapshot; take only light.
                        // A chunk relit (a neighbour loaded or changed) remeshes
                        // only around the light that changed, as vanilla's light
                        // updates dirty only their sections (#270).
                        let first = self.lit.insert((x, z));
                        let mut changed = LightChanges::default();
                        if first {
                            apply_lighting(chunk, sections);
                        } else {
                            apply_relighting(chunk, sections, &mut changed);
                        }
                        // Saved unlit (at the edge of what was loaded), or its light
                        // changed: saved again.
                        if (first && !chunk.light_loaded) || !changed.is_empty() {
                            world.modified.insert((x, z));
                        }
                        chunk.lit = true;
                        if first {
                            for dz in -1..=1 {
                                for dx in -1..=1 {
                                    for sy in 0..world.layout().sections() as i32 {
                                        world.dirty_sections.insert((x + dx, sy, z + dz));
                                    }
                                }
                            }
                        } else {
                            Self::mark_light_changes(world, &changed);
                        }
                        self.replay_light_edits(world, x, z, snapshot);
                    }
                }
                Done::Meshed(x, sy, z, mesh, seq) => {
                    if self.meshing.get(&(x, sy, z)) != Some(&seq) {
                        stale.push(((x, sy, z), mesh));
                        continue;
                    }
                    // Its column unloaded while it was meshed: nothing would
                    // ever remove the uploaded section.
                    if !world.chunks.contains_key(&(x, z)) {
                        self.meshing.remove(&(x, sy, z));
                        stale.push(((x, sy, z), mesh));
                        continue;
                    }
                    self.meshing.remove(&(x, sy, z));
                    self.compile_started.insert((x, sy, z), seq);
                    self.meshed_total += 1;
                    uploads.push(((x, sy, z), mesh));
                }
            }
        }
        self.free(stale);
        let mut removed = Vec::new();
        if sweep {
            // Unload far columns.
            let far: Vec<_> = world
                .chunks
                .keys()
                .copied()
                .filter(|&(x, z)| !self.remote && self.ranges().unloaded(x, z, self.decorates))
                .collect();
            for key in &far {
                self.decorated.remove(key);
            }
            removed.reserve(far.len());
            for key in &far {
                if let Some(chunk) = world.chunks.remove(key) {
                    removed.push(chunk);
                }
                self.lit.remove(key);
                self.meshed.remove(key);
                self.lighting.remove(key);
                self.light_snapshots.remove(key);
            }
            if !far.is_empty() {
                let far: HashSet<(i32, i32)> = far.iter().copied().collect();
                self.meshing.retain(|k, _| !far.contains(&(k.0, k.2)));
                self.compile_started
                    .retain(|k, _| !far.contains(&(k.0, k.2)));
                self.waiting.retain(|k| !far.contains(&(k.0, k.2)));
                world
                    .dirty_sections
                    .retain_columns(|x, z| !far.contains(&(x, z)));
            }
            // Drop far proto chunks that are not checked out.
            let (center, r4) = (self.center, i64::from(self.radius + 4));
            let ranges = self.ranges();
            let far: Vec<(i32, i32)> = self
                .protos
                .iter()
                .filter(|&(&(x, z), p)| {
                    p.is_some()
                        && (self.radius < 0
                            || i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2)
                                > r4 * r4)
                        && !ranges.in_extra(x, z, 4)
                        && !ranges.in_band(x, z, 4)
                        && !ranges.forced.iter().any(|f| {
                            (x - f.0).abs() <= FORCED_TERRAIN_RING
                                && (z - f.1).abs() <= FORCED_TERRAIN_RING
                        })
                })
                .map(|(k, _)| *k)
                .collect();
            if !far.is_empty() {
                self.generation_scan.exhausted = false;
            }
            // Those decoration wrote to are saved first (see
            // `ChunkGenerator::save_protos`), and dropped once back.
            let mut written = Vec::new();
            for key in far {
                if self.written.remove(&key) {
                    written.push(key);
                } else {
                    self.protos.remove(&key);
                    self.light_views.remove(&key);
                }
            }
            self.save_checked_out(written);
        }
        let (protos, finishing) = (&self.protos, &self.finishing);
        self.decorated.retain(|k| {
            protos.contains_key(k) || finishing.contains(k) || world.chunks.contains_key(k)
        });
        // Finalize chunks whose neighbourhood is decorated.
        let candidates: Vec<(i32, i32)> = self.protos.keys().copied().collect();
        for (x, z) in candidates {
            if world.chunks.contains_key(&(x, z))
                || self.finishing.contains(&(x, z))
                || !self.in_range(x, z, 1)
            {
                continue;
            }
            let ready = (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    self.decorated.contains(&(x + dx, z + dz))
                        || world.chunks.contains_key(&(x + dx, z + dz))
                })
            });
            if ready && let Some(Some(_)) = self.protos.get(&(x, z)) {
                let proto = self
                    .protos
                    .remove(&(x, z))
                    .flatten()
                    .expect("present proto");
                self.light_views.remove(&(x, z));
                self.finishing.insert((x, z));
                self.written.remove(&(x, z));
                self.submit(Some((x, z)), 0, Job::Finish(proto));
            }
        }
        // Decorate chunks whose 3x3 proto neighbourhood is available, in
        // vanilla's order: decoration is order dependent where write regions
        // overlap (see `DecorationOrder`).
        let workers = self.threads.len();
        if self.decorates && self.order_area != Some((self.center, self.radius)) {
            let (decorated, chunks) = (&self.decorated, &world.chunks);
            // The render distance is the server view distance in singleplayer.
            self.decoration_order
                .reassign(self.center, self.radius, |x, z| {
                    decorated.contains(&(x, z)) || chunks.contains_key(&(x, z))
                });
            self.order_area = Some((self.center, self.radius));
            let ranges = Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::matching(&self.cover, &self.extra),
                band: self.band.as_ref(),
            };
            let (protos, generating) = (&self.protos, &self.generating);
            self.decoration_order.retain(|x, z| {
                ranges.in_range(x, z, 4)
                    || ranges.near_finished(x, z, 2)
                    || protos.contains_key(&(x, z))
                    || generating.contains(&(x, z))
            });
        }
        if self.decorates && self.decorating < workers * 2 {
            let wants = |s: &Self, x: i32, z: i32| {
                !s.decorated.contains(&(x, z))
                    && !world.chunks.contains_key(&(x, z))
                    && (s.in_range(x, z, 2) || s.near_finished(x, z, 1))
            };
            // A wanted chunk holds back higher-ranked ones only if it can be
            // decorated: its 3x3 neighbourhood is (or will be) generated as
            // protos, none of them already final.
            let waits_for = |s: &Self, x: i32, z: i32| {
                wants(s, x, z)
                    && (-1..=1).all(|dz| {
                        (-1..=1).all(|dx| {
                            let n = (x + dx, z + dz);
                            !world.chunks.contains_key(&n)
                                && (s.protos.contains_key(&n)
                                    || s.generating.contains(&n)
                                    || s.ranges().requested(n.0, n.1, true))
                        })
                    })
            };
            let keys: Vec<(i32, i32)> = self
                .protos
                .iter()
                .filter(|(k, p)| p.is_some() && wants(self, k.0, k.1))
                .map(|(k, _)| *k)
                .collect();
            let mut wanted: Vec<(u64, i64, i32, i32)> = keys
                .into_iter()
                .map(|(x, z)| (0, self.distance(x, z), x, z))
                .collect();
            // Chunks no ticket reached (forced and extra areas) go last, nearest first.
            wanted.sort_unstable_by_key(|w| w.1);
            for w in &mut wanted {
                w.0 = self.decoration_order.rank_or_append(w.2, w.3);
            }
            wanted.sort_unstable();
            for (_, _, x, z) in wanted {
                if self.decorating >= workers * 2 {
                    break;
                }
                if self
                    .decoration_order
                    .blocked(x, z, |nx, nz| waits_for(self, nx, nz))
                {
                    continue;
                }
                let keys: Vec<(i32, i32)> =
                    (0..9).map(|i| (x + (i % 3) - 1, z + (i / 3) - 1)).collect();
                // A final neighbour cannot be generated again: a generator that
                // can decorates beside it (a chunk of an older world), writing
                // into it as vanilla's features do; the others skip decoration
                // (worlds saved before proto chunks were saved get here: their
                // final chunks' neighbours are generated again).
                if keys.iter().any(|k| self.decorating_finals.contains(k)) {
                    continue;
                }
                let finals: Vec<(usize, Chunk)> = keys
                    .iter()
                    .enumerate()
                    .filter_map(|(i, k)| Some((i, (**world.chunks.get(k)?).clone())))
                    .collect();
                if !finals.is_empty() && !self.saves_beside_final {
                    self.decorated.insert((x, z));
                    continue;
                }
                if !keys.iter().enumerate().all(|(i, k)| {
                    finals.iter().any(|(f, _)| *f == i)
                        || matches!(self.protos.get(k), Some(Some(_)))
                }) {
                    continue;
                }
                let region: Vec<Proto> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| {
                        if finals.iter().any(|(f, _)| *f == i) {
                            return Box::new(()) as Proto;
                        }
                        self.light_views.remove(k);
                        self.protos
                            .insert(*k, None)
                            .flatten()
                            .expect("present proto")
                    })
                    .collect();
                let region: Box<[Proto; 9]> = match region.try_into() {
                    Ok(r) => Box::new(r),
                    Err(_) => continue,
                };
                self.decorating += 1;
                self.decorating_finals
                    .extend(finals.iter().map(|(i, _)| keys[*i]));
                self.submit(Some((x, z)), 1, Job::Decorate(x, z, region, finals));
            }
        }
        self.schedule_generation(world, workers);
        // Lighting once all neighbours exist, nearest first and only as
        // much of it as the workers can chew. Vanilla's light engine also
        // hands a bounded queue to its workers and reschedules what did
        // not fit by distance; submitted whole, the light jobs of every
        // unlit column at once — each carrying a 3x3 region snapshot —
        // grew with the render distance and outlived the columns they
        // were queued for (#3629). What stays in `dirty_light` is asked
        // for again, from the current centre, as capacity frees.
        let mut light: Vec<(i64, i32, i32)> = Vec::new();
        let mut gone: Vec<(i32, i32)> = Vec::new();
        for &(x, z) in world.dirty_light.iter() {
            if !world.chunks.contains_key(&(x, z)) {
                gone.push((x, z));
            } else if self.light_region_ready(world, x, z) {
                light.push((self.distance(x, z), x, z));
            }
        }
        for key in gone {
            world.dirty_light.remove(&key);
        }
        let room = (self.threads.len() * 4).saturating_sub(self.lighting.len());
        nearest_first(&mut light, room);
        for (_, x, z) in light {
            let Some(region) = self.light_region(world, x, z) else {
                continue;
            };
            world.dirty_light.remove(&(x, z));
            self.sequence += 1;
            self.lighting.insert((x, z), self.sequence);
            self.light_snapshots
                .insert((x, z), self.light_edit_sequence);
            self.submit(
                Some((x, z)),
                1,
                Job::Light(x, z, region, self.sequence, self.sky_light),
            );
        }
        // A server draws nothing: its sections are never meshed.
        if !self.mesh {
            world.dirty_sections.clear();
            return (uploads, removed);
        }
        // Meshing once the column and its neighbours are lit, and the column
        // is within the render distance (`in_view_distance`, the columns
        // vanilla draws). Sections set aside past it come back
        // when the player moves: a column that loads with its light gets no
        // other request (#228).
        let area = (self.center, self.radius, self.forced.len());
        if self.waiting_area != area {
            self.waiting_area = area;
            let ranges = Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::matching(&self.cover, &self.extra),
                band: self.band.as_ref(),
            };
            self.waiting.retain(|&(x, sy, z)| {
                let back = ranges.in_view(x, z);
                if back {
                    world.dirty_sections.insert((x, sy, z));
                }
                !back
            });
        }
        // Column by column: the checks below hold for all of a column's
        // sections, and columns waiting for light cost one look per frame.
        let mut candidates: Vec<(i64, i32, i32, SectionMask)> = Vec::new();
        let columns: Vec<_> = world.dirty_sections.columns().collect();
        for ((x, z), sections) in columns {
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_sections.take_column(x, z);
                continue;
            }
            if !self.ranges().in_view(x, z) {
                world.dirty_sections.take_column(x, z);
                for sy in sections.iter().map(|sy| sy as i32) {
                    self.waiting.insert((x, sy, z));
                }
                continue;
            }
            // A neighbour being lit dirties this column again when its light
            // job finishes (new chunks relight their eight neighbours), so
            // wait for it rather than mesh once per neighbour. Columns still
            // in `dirty_light` here can't be lit yet (a neighbour of theirs is
            // missing): waiting for them could last until the player comes
            // back.
            let ready = (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    let n = (x + dx, z + dz);
                    self.lit.contains(&n) && !self.lighting.contains_key(&n)
                })
            });
            if !ready {
                continue;
            }
            candidates.push((self.distance(x, z), x, z, sections));
        }
        // Nearest first, and only as many sections as the workers can chew.
        // Every ready column used to be submitted at once, so at a wide
        // render distance the queue — and `meshing`, and each job's 3x3
        // region snapshot — grew with the number of loaded columns and the
        // jobs went stale long before a worker reached them (#3629). What
        // stays dirty is asked for again, from the current centre, as
        // capacity frees; `requeue` then has a small queue to keep close.
        let room = (self.threads.len() * 32).saturating_sub(self.meshing.len());
        nearest_first(&mut candidates, room);
        for (_, x, z, sections) in candidates {
            let Some(region) = Self::region(world, x, z) else {
                continue;
            };
            world.dirty_sections.take_column(x, z);
            for sy in sections.iter().map(|sy| sy as i32) {
                let first = self.meshed.insert((x, z));
                self.sequence += 1;
                self.meshing.insert((x, sy, z), self.sequence);
                self.submit(
                    Some((x, z)),
                    if first { 2 } else { 0 },
                    Job::Mesh(
                        x,
                        sy,
                        z,
                        region.clone(),
                        self.sequence,
                        MeshSettings {
                            nether: self.nether_shading,
                            shader_pack_lighting: self.shader_pack_lighting,
                            zoom: self.biome_zoom,
                            blend_radius: self.biome_blend,
                            smooth_lighting: self.smooth_lighting,
                            cutout_leaves: self.cutout_leaves,
                            translucent_layers: self.translucent_layers.clone(),
                            leaf_culling: self.leaf_culling,
                        },
                    ),
                );
            }
        }
        (uploads, removed)
    }
    fn schedule_generation(&mut self, world: &World, workers: usize) {
        let input = GenerationInputs {
            ranges: Ranges {
                center: self.center,
                radius: self.radius,
                extra: &self.extra,
                forced: &self.forced,
                cover: ExtraCover::matching(&self.cover, &self.extra),
                band: self.band.as_ref(),
            },
            world,
            generating: &self.generating,
            protos: &self.protos,
            finishing: &self.finishing,
            decorates: self.decorates,
            remote: self.remote,
            loading: self.loading,
        };
        if let Some(wanted) = self.generation_scan.candidates(input, workers * 3) {
            for (_, x, z) in wanted {
                self.generating.insert((x, z));
                self.submit(Some((x, z)), 0, Job::Generate(x, z));
            }
        }
    }
    /// Relight block edits on lit chunks in place (vanilla's incremental light
    /// engine) instead of recomputing the 3x3 chunks around each edit.
    fn relight_edits(&mut self, world: &mut World) {
        if world.light_edits.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut world.light_edits);
        let positions: Vec<(i32, i32, i32)> = edits
            .iter()
            .filter(|(_, old, new)| self.light_tables.differs(*old, *new))
            .map(|(p, _, _)| *p)
            .collect();
        if positions.is_empty() {
            return;
        }
        let mut changed = LightChanges::default();
        crate::light::relight(
            &mut world.chunks,
            &self.light_tables,
            self.sky_light,
            &positions,
            false,
            &mut changed,
        );
        Self::mark_light_changes(world, &changed);
        Self::relight_beside_unlit(world, &changed);
        if !self.light_snapshots.is_empty() {
            for p in positions {
                self.light_edit_sequence += 1;
                self.light_log.push((self.light_edit_sequence, p));
            }
        }
    }
    /// A light job computed from blocks older than some edits: redo those
    /// edits on top of its result.
    fn replay_light_edits(&mut self, world: &mut World, x: i32, z: i32, snapshot: u64) {
        let positions: Vec<(i32, i32, i32)> = self
            .light_log
            .iter()
            .filter(|(seq, p)| {
                *seq > snapshot && ((p.0 >> 4) - x).abs() <= 1 && ((p.2 >> 4) - z).abs() <= 1
            })
            .map(|(_, p)| *p)
            .collect();
        if !positions.is_empty() {
            let mut changed = LightChanges::default();
            crate::light::relight(
                &mut world.chunks,
                &self.light_tables,
                self.sky_light,
                &positions,
                true,
                &mut changed,
            );
            Self::mark_light_changes(world, &changed);
            Self::relight_beside_unlit(world, &changed);
        }
        let oldest = self.light_snapshots.values().copied().min();
        match oldest {
            Some(oldest) => self.light_log.retain(|(seq, _)| *seq > oldest),
            None => self.light_log.clear(),
        }
    }
    /// A column whose light an edit changed beside a column not lit yet gets
    /// a full light job ([`crate::light::beside_unlit`]).
    fn relight_beside_unlit(world: &mut World, changed: &LightChanges) {
        let columns = crate::light::beside_unlit(changed, |x, z| {
            world.chunks.get(&(x, z)).is_some_and(|c| !c.lit)
        });
        world.dirty_light.extend(columns);
    }
    /// Sections whose meshes sample a changed light value (faces and corners
    /// read light from neighbouring blocks).
    fn mark_light_changes(world: &mut World, changed: &LightChanges) {
        if let Some(log) = &mut world.light_log {
            for (x, sy, z) in changed.sections() {
                log.entry((x, z)).or_default().insert(sy as usize);
            }
        }
        changed.dirty_sections(|section| {
            world.dirty_sections.insert(section);
        });
    }
    fn ranges(&self) -> Ranges<'_> {
        Ranges {
            center: self.center,
            radius: self.radius,
            extra: &self.extra,
            forced: &self.forced,
            cover: ExtraCover::matching(&self.cover, &self.extra),
            band: self.band.as_ref(),
        }
    }
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        self.ranges().near_finished(x, z, reach)
    }
    /// Save what generation holds outside the world: the proto chunks
    /// decoration wrote to. With `wait`, the decorations, columns and saves
    /// in flight are waited for first (their proto chunks are out in the
    /// jobs) and the protos are saved here, before the world's chunks;
    /// otherwise a worker saves them (vanilla's autosave hands its chunks to
    /// the IOWorker too), and those out in jobs are saved later, as they
    /// unload or at the next save.
    pub fn save(&mut self, world: &mut World, wait: bool) {
        if !wait {
            let written: Vec<(i32, i32)> = self.written.drain().collect();
            self.save_checked_out(written);
            return;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.decorating > 0
            || !self.finishing.is_empty()
            || self.saving > 0
            || !self.viewing.is_empty()
        {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Done::Decorated(x, z, region, finals, writes)) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Some(Done::Generated(chunk)) => self.receive_generated(chunk, world),
                Some(Done::Saved(protos)) => self.receive_saved(protos),
                Some(Done::LightView(x, z, proto, view)) => {
                    self.receive_light_view(x, z, proto, view)
                }
                Some(other) => self.deferred.push(other),
                None => {
                    log::warn!("Horizon: chunk generation did not finish before saving");
                    break;
                }
            }
        }
        let protos: Vec<&Proto> = self
            .written
            .iter()
            .filter_map(|k| self.protos.get(k)?.as_ref())
            .collect();
        if !protos.is_empty() {
            self.saves.save_protos(&protos);
        }
        self.written.clear();
    }
    /// Finish generation for a closing world without waiting on the event
    /// loop. No new generation is scheduled while the save screen is shown.
    /// Proto snapshots are checked out to workers before chunk writes, just as
    /// in `save(true)`, so older generation cannot overwrite the final chunks.
    pub fn poll_save(&mut self, world: &mut World) -> bool {
        let started = std::time::Instant::now();
        while let Some(done) = self.shared.done.try_recv() {
            match done {
                Done::Decorated(x, z, region, finals, writes) => {
                    self.receive_decorated(x, z, *region, finals, writes, world)
                }
                Done::Generated(chunk) => self.receive_generated(chunk, world),
                Done::Saved(protos) => self.receive_saved(protos),
                Done::LightView(x, z, proto, view) => self.receive_light_view(x, z, proto, view),
                other => self.deferred.push(other),
            }
            if started.elapsed() >= std::time::Duration::from_millis(2) {
                return false;
            }
        }
        if self.decorating > 0
            || !self.finishing.is_empty()
            || self.saving > 0
            || !self.viewing.is_empty()
        {
            return false;
        }
        let written: Vec<_> = self.written.drain().collect();
        self.save_checked_out(written);
        self.saving == 0
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.ranges().in_range(x, z, margin)
    }
    /// Rank the 3x3 a chunk loaded to FULL decorates, in one generation
    /// task's order (`ChunkGenerationTask`: x-outer, z-inner), after every
    /// chunk ranked before.
    pub fn rank_full_task(&mut self, x: i32, z: i32) {
        for dx in -1..=1 {
            for dz in -1..=1 {
                self.decoration_order.rank_or_append(x + dx, z + dz);
            }
        }
    }
    pub fn band(&self) -> Option<Band> {
        self.band
    }
    /// Rank the chunks `/forceload add` over chunks `(x0, z0)..=(x1, z1)`
    /// decorates in vanilla's order (`forced_order`), after every chunk
    /// ranked before.
    pub fn rank_forced(&mut self, x0: i32, z0: i32, x1: i32, z1: i32) {
        for (x, z) in horizon_world::worldgen::decoration_order::forced_order(x0, z0, x1, z1) {
            self.decoration_order.rank_or_append(x, z);
        }
    }
    /// Generate, light and keep the chunks of `band` (none: stop), and let
    /// go of what earlier bands held as it falls out of range. The chunks a
    /// band reaches are ranked for decoration row by row, as they first come
    /// within reach, so that a task decorates in the same order however its
    /// work is timed and shared among the workers.
    pub fn set_band(&mut self, band: Option<Band>) {
        if self.band == band {
            return;
        }
        self.band = band;
        self.generation_scan.exhausted = false;
        let Some(band) = band else {
            return;
        };
        // Decoration reaches two chunks past the band and generation three.
        const REACH: i32 = 4;
        let (x0, x1) = (band.x0 - REACH, band.x1 + REACH);
        let first = match self.band_ranked {
            Some((rx0, rx1, next)) if (rx0, rx1) == (x0, x1) => next,
            _ => band.z0 - REACH,
        };
        let end = band.z1 + REACH;
        for z in first..=end {
            for x in x0..=x1 {
                self.decoration_order.rank_or_append(x, z);
            }
        }
        self.band_ranked = Some((x0, x1, end.max(first - 1) + 1));
    }
    /// What holds up the columns `keys`, for a task that stopped advancing:
    /// how many are not loaded, loaded but unlit with a light job in flight,
    /// and unlit with none (those whose neighbours are all loaded are queued
    /// for light again).
    pub fn unstick_light(&mut self, world: &mut World, keys: &[(i32, i32)]) -> [usize; 3] {
        let mut counts = [0; 3];
        for &key in keys {
            if !world.chunks.contains_key(&key) {
                counts[0] += 1;
            } else if self.lit.contains(&key) {
            } else if self.lighting.contains_key(&key) {
                counts[1] += 1;
            } else {
                counts[2] += 1;
                if self.light_region(world, key.0, key.1).is_some()
                    || (-1..=1).any(|dz| {
                        (-1..=1).any(|dx| self.viewing.contains(&(key.0 + dx, key.1 + dz)))
                    })
                {
                    world.dirty_light.insert(key);
                }
            }
        }
        counts
    }
    /// Wait up to `timeout` for a worker to finish a job; the next `update`
    /// takes it. A server pumps generation with this between its ticks.
    pub fn wait(&mut self, timeout: std::time::Duration) {
        if let Some(done) = self.shared.done.recv_timeout(timeout) {
            self.deferred.push(done);
        }
    }
}
/// `ChunkTrackingView.isInViewDistance`: whether the column (x, z) is drawn
/// at this render distance around `center`, the camera's column
/// (`SectionOcclusionGraph.isInViewDistance`). Each axis gets a column of
/// slack, so this reaches past the circle of `radius` columns.
pub fn in_view_distance(center: (i32, i32), radius: i32, x: i32, z: i32) -> bool {
    let gap = |d: i32| i64::from((d.abs() - 1).max(0));
    let (dx, dz) = (gap(x - center.0), gap(z - center.1));
    dx * dx + dz * dz < i64::from(radius) * i64::from(radius)
}
/// How long a view's columns are waited for (`ViewArrival`): a server may never
/// fill it.
const VIEW_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// Whether the columns of a view (centre, radius) have all arrived, as a client
/// sees them come: a server forgets the old view's columns, then sends the new
/// ones nearest first, so the view's outermost ones come last. Single player
/// joins its integrated server the same way, so `Streaming::busy` cannot see
/// what is not sent yet.
#[derive(Default)]
pub struct ViewArrival {
    view: ((i32, i32), i32),
    since: Option<std::time::Instant>,
    arrived: bool,
}
impl ViewArrival {
    /// Whether columns of `view` have yet to arrive (`has`): its outermost ones,
    /// at both ends of each of its rows and columns (`in_view_distance`). Once
    /// they have, false until the view changes; false too once it is no longer
    /// `recent`, and for a radius below 0 (no view).
    pub fn arriving(
        &mut self,
        view: ((i32, i32), i32),
        now: std::time::Instant,
        has: impl Fn((i32, i32)) -> bool,
    ) -> bool {
        if self.since.is_none() || view != self.view {
            (self.view, self.since, self.arrived) = (view, Some(now), false);
        }
        let ((x, z), r) = view;
        if self.arrived || r < 0 || !self.recent(now) {
            return false;
        }
        self.arrived = (-r..=r).all(|d| {
            // How far row (and column) `d` reaches: `in_view_distance` solved for it.
            let gap = i64::from((d.abs() - 1).max(0));
            let room = i64::from(r).pow(2) - gap * gap;
            room <= 0 || {
                let e = (room - 1).isqrt() as i32 + 1;
                // Both ends of row `d`, and of column `d`.
                [-e, e]
                    .into_iter()
                    .all(|e| has((x + d, z + e)) && has((x + e, z + d)))
            }
        });
        !self.arrived
    }
    /// Whether the view `arriving` was last given changed less than `VIEW_WAIT`
    /// ago: until then sections meshed are its terrain coming in, later an edit's.
    pub fn recent(&self, now: std::time::Instant) -> bool {
        self.since
            .is_some_and(|since| now.saturating_duration_since(since) < VIEW_WAIT)
    }
}
/// The rings around a `/forceload`ed chunk, in chunks. Vanilla's FORCED
/// ticket is level 31 (`ChunkMap.FORCED_TICKET_LEVEL`), the level rises one
/// per ring (`ChunkTracker.computeLevelFromNeighbor`), and the level picks
/// the column's status (`ChunkLevel.generationStatus`): full (33) two rings
/// out, features (34) at three, terrain (35) at four.
const FORCED_FULL_RING: i32 = 2;
const FORCED_FEATURES_RING: i32 = 3;
const FORCED_TERRAIN_RING: i32 = 4;
/// The areas kept loaded: circles around the player and `extra`, and forced chunks.
struct Ranges<'a> {
    center: (i32, i32),
    radius: i32,
    extra: &'a [((i32, i32), i32)],
    forced: &'a HashSet<(i32, i32)>,
    /// `extra` as a grid, when one was made for it: `in_extra` without a
    /// scan of every area.
    cover: Option<&'a ExtraCover>,
    band: Option<&'a Band>,
}
/// The chunks a pregeneration task (`/pregen`) wants lit now: rows `z0..=z1`
/// of the columns `x0..=x1`, those within `circle` (a centre and a radius in
/// chunks) when there is one. Like an extra area, it finishes its chunks and
/// the ring around them, so that its own chunks can be lit, and generation
/// reaches past that. A task moves it down a strip of columns a row at a
/// time, so each chunk is generated once and only the rows in flight stay
/// loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub x0: i32,
    pub x1: i32,
    pub z0: i32,
    pub z1: i32,
    pub circle: Option<((i32, i32), i32)>,
}
impl Band {
    /// Whether (x, z) lies within `margin` (Chebyshev) of one of the band's
    /// chunks: the square of side `2 * margin + 1` around it meets the
    /// rectangle at a point inside the circle.
    pub fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (x0, x1) = (
            x.saturating_sub(margin).max(self.x0),
            x.saturating_add(margin).min(self.x1),
        );
        let (z0, z1) = (
            z.saturating_sub(margin).max(self.z0),
            z.saturating_add(margin).min(self.z1),
        );
        if x0 > x1 || z0 > z1 {
            return false;
        }
        self.circle.is_none_or(|((cx, cz), r)| {
            let dx = i64::from(cx.clamp(x0, x1)) - i64::from(cx);
            let dz = i64::from(cz.clamp(z0, z1)) - i64::from(cz);
            dx * dx + dz * dz <= i64::from(r) * i64::from(r)
        })
    }
    /// The band's chunks go after the players' (lower runs first), row by
    /// row, as the task wants them lit.
    fn priority(&self, x: i32, z: i32) -> i64 {
        const AFTER_PLAYERS: i64 = 1 << 40;
        AFTER_PLAYERS
            + (i64::from(z - self.z0 + 8).max(0) << 20)
            + i64::from(x - self.x0 + 8).max(0)
    }
}
/// For every column near the extra areas, how far it lies outside the
/// nearest one's square (Chebyshev distance less its radius; 0 inside). A server has an area per player, and streaming asks of every
/// loaded and wanted column whether one reaches it, several times a tick:
/// with fifty players, a scan of fifty squares per question.
struct ExtraCover {
    extra: Vec<((i32, i32), i32)>,
    x0: i32,
    z0: i32,
    width: u32,
    depth: u32,
    cells: Box<[i8]>,
}
impl ExtraCover {
    /// The largest margin the grid answers for; beyond it (and outside the
    /// grid) a column is further than that from every square.
    const REACH: i32 = 4;
    /// The largest grid, in columns (a mebibyte).
    const CELLS: u64 = 1 << 20;
    fn matching<'a>(
        cover: &'a Option<ExtraCover>,
        extra: &[((i32, i32), i32)],
    ) -> Option<&'a ExtraCover> {
        cover.as_ref().filter(|c| c.extra == extra)
    }
    /// Make the grid again if `extra` changed; none for one area or none
    /// (a scan is as quick), or areas too far apart.
    fn refresh(cover: &mut Option<ExtraCover>, extra: &[((i32, i32), i32)]) {
        if cover.as_ref().is_some_and(|c| c.extra == extra) {
            return;
        }
        *cover = Self::new(extra);
    }
    fn new(extra: &[((i32, i32), i32)]) -> Option<Self> {
        if extra.len() < 2 || extra.iter().any(|&(_, r)| !(0..=64).contains(&r)) {
            return None;
        }
        let k = i64::from(Self::REACH);
        let (mut x0, mut z0, mut x1, mut z1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &((x, z), r) in extra {
            let (x, z, r) = (i64::from(x), i64::from(z), i64::from(r) + k);
            (x0, z0, x1, z1) = (x0.min(x - r), z0.min(z - r), x1.max(x + r), z1.max(z + r));
        }
        let (width, depth) = ((x1 - x0 + 1) as u64, (z1 - z0 + 1) as u64);
        let fits = |lo: i64, hi: i64| lo >= i64::from(i32::MIN) && hi <= i64::from(i32::MAX);
        if width * depth > Self::CELLS || !fits(x0, x1) || !fits(z0, z1) {
            return None;
        }
        let (x0, z0, width, depth) = (x0 as i32, z0 as i32, width as u32, depth as u32);
        let mut cells = vec![(Self::REACH + 1) as i8; (width * depth) as usize].into_boxed_slice();
        // Inside a square counts as 0: no margin asked for is below it. Each
        // row of a square is a run of one value with a slope at either end.
        let lower = |run: &mut [i8], v: i8| run.iter_mut().for_each(|c| *c = (*c).min(v));
        for &((cx, cz), r) in extra {
            let reach = r + Self::REACH;
            for dz in -reach..=reach {
                let start = (cz + dz - z0) as usize * width as usize + (cx - x0) as usize;
                let row = &mut cells[start - reach as usize..=start + reach as usize];
                let flat = dz.abs().max(r);
                let centre = reach as usize;
                lower(
                    &mut row[centre - flat as usize..=centre + flat as usize],
                    (flat - r) as i8,
                );
                for d in flat + 1..=reach {
                    let v = (d - r) as i8;
                    lower(&mut row[centre - d as usize..=centre - d as usize], v);
                    lower(&mut row[centre + d as usize..=centre + d as usize], v);
                }
            }
        }
        Some(Self {
            extra: extra.to_vec(),
            x0,
            z0,
            width,
            depth,
            cells,
        })
    }
    /// Whether (x, z) is within `margin` of an area's square, for a
    /// `margin` up to `REACH`.
    fn within(&self, x: i32, z: i32, margin: i32) -> bool {
        let (dx, dz) = (
            x.wrapping_sub(self.x0) as u32,
            z.wrapping_sub(self.z0) as u32,
        );
        dx < self.width
            && dz < self.depth
            && i32::from(self.cells[(dz * self.width + dx) as usize]) <= margin
    }
}
impl Ranges<'_> {
    /// `distance`, or `rings` while the level loading screen is up.
    fn priority(&self, x: i32, z: i32, loading: bool) -> i64 {
        let own = if loading {
            self.rings(x, z)
        } else {
            self.distance(x, z)
        };
        match self.band {
            Some(band) if band.within(x, z, 4) => own.min(band.priority(x, z)),
            _ => own,
        }
    }
    /// Ring by ring: the Chebyshev distance to the nearest centre first, then
    /// the squared distance. Vanilla's chunk tasks run by ticket level, which
    /// spreads one level per ring of chunks (`ChunkTaskPriorityQueue`), so
    /// `LevelLoadingScreen`'s chunk map fills squares around the spawn.
    fn rings(&self, x: i32, z: i32) -> i64 {
        let d = |c: (i32, i32)| {
            let (dx, dz) = (i64::from(x - c.0), i64::from(z - c.1));
            let ring = dx.abs().max(dz.abs());
            (ring * ring) << 20 | (dx * dx + dz * dz)
        };
        self.extra
            .iter()
            .map(|(c, _)| d(*c))
            .fold(d(self.center), i64::min)
    }
    fn distance(&self, x: i32, z: i32) -> i64 {
        let d = |c: (i32, i32)| {
            let (dx, dz) = (i64::from(x - c.0), i64::from(z - c.1));
            dx * dx + dz * dz
        };
        self.extra
            .iter()
            .map(|(c, _)| d(*c))
            .fold(d(self.center), i64::min)
    }

    /// How far past the view distance chunks are generated, and how far around
    /// a finished chunk (`near_finished`): decorated chunks need their
    /// neighbours generated too.
    fn margins(decorates: bool) -> (i32, i32) {
        if decorates { (3, 2) } else { (1, 1) }
    }
    /// Chunks generation asks for: loaded from the save, or generated as protos.
    fn requested(&self, x: i32, z: i32, decorates: bool) -> bool {
        let (margin, reach) = Self::margins(decorates);
        self.in_range(x, z, margin) || self.near_finished(x, z, reach)
    }
    /// Finished columns kept when they arrive: within two of the view distance
    /// or next to a finished chunk, and every column generation asked for (a
    /// saved one loads finished; dropping it had it asked for and loaded again
    /// every frame).
    fn chunk_kept(&self, x: i32, z: i32, decorates: bool) -> bool {
        self.in_range(x, z, 2) || self.near_finished(x, z, 1) || self.requested(x, z, decorates)
    }
    /// Columns unloaded: past every column kept.
    fn unloaded(&self, x: i32, z: i32, decorates: bool) -> bool {
        !self.in_range(x, z, 3) && !self.near_finished(x, z, Self::margins(decorates).1)
    }
    /// Whether a chunk finished here (`in_range(_, _, 1)`) lies within `reach`
    /// chunks of (x, z) on both axes: its decoration needs its 3x3 neighbours
    /// decorated, which need their own 3x3 neighbours generated. The ranges are
    /// circles, so a fixed margin alone misses the corners of those squares
    /// (render distance 2 never finished the chunks next to the player).
    fn near_finished(&self, x: i32, z: i32, reach: i32) -> bool {
        let gap = |d: i32| i64::from((d.abs() - reach).max(0));
        let (dx, dz) = (gap(x - self.center.0), gap(z - self.center.1));
        let r = i64::from(self.radius + 1);
        // The extra areas are squares: their finished square grown by `reach`.
        (self.radius >= 0 && dx * dx + dz * dz <= r * r)
            || self.in_extra(x, z, 1 + reach)
            || self.in_band(x, z, 1 + reach)
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.in_main_range(x, z, margin)
            || self.in_extra(x, z, margin)
            || self.in_band(x, z, margin)
    }
    /// Within `margin` of the pregeneration band.
    fn in_band(&self, x: i32, z: i32, margin: i32) -> bool {
        self.band.is_some_and(|b| b.within(x, z, margin))
    }
    /// Within `margin` of an extra area's square (Chebyshev distance, as
    /// vanilla's ticket levels spread).
    fn in_extra(&self, x: i32, z: i32, margin: i32) -> bool {
        if let Some(cover) = self.cover
            && (0..=ExtraCover::REACH).contains(&margin)
        {
            return cover.within(x, z, margin);
        }
        self.extra
            .iter()
            .any(|&(c, radius)| (x - c.0).abs().max((z - c.1).abs()) <= radius + margin)
    }
    /// The columns meshed for drawing: those vanilla draws around the
    /// player (`in_view_distance`), and forced ones.
    fn in_view(&self, x: i32, z: i32) -> bool {
        in_view_distance(self.center, self.radius, x, z) || self.forced.contains(&(x, z))
    }
    fn in_main_range(&self, x: i32, z: i32, margin: i32) -> bool {
        let r = i64::from(self.radius + margin);
        let (dx, dz) = (i64::from(x - self.center.0), i64::from(z - self.center.1));
        if self.radius >= 0 && dx * dx + dz * dz <= r * r {
            return true;
        }
        // A forced chunk holds a FORCED ticket at level 31
        // (`ChunkMap.FORCED_TICKET_LEVEL`), and the level rises one per ring
        // (`ChunkTracker.computeLevelFromNeighbor`): the margin is the
        // vanilla status ring the caller asks for.
        let reach = match margin {
            1 => FORCED_FULL_RING,
            2 => FORCED_FEATURES_RING,
            3 => FORCED_TERRAIN_RING,
            _ => margin + 1,
        };
        self.forced
            .iter()
            .any(|f| (x - f.0).abs() <= reach && (z - f.1).abs() <= reach)
    }
}
impl Drop for Streaming {
    fn drop(&mut self) {
        // Saves in flight hold proto chunks nothing else would save.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.saving > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.shared.done.recv_timeout(left) {
                Some(Done::Saved(_)) => self.saving -= 1,
                Some(_) => {}
                None => break,
            }
        }
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.ready.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
/// The queued jobs' priorities computed again from each one's column
/// `distance` (`Streaming::submit`).
fn rank_again(queue: &mut BinaryHeap<Queued>, distance: impl Fn(i32, i32) -> i64) {
    if queue.is_empty() {
        return;
    }
    let mut jobs = std::mem::take(queue).into_vec();
    for job in &mut jobs {
        if let Some((x, z)) = job.at {
            job.priority = distance(x, z) * 4 + job.bias;
        }
    }
    *queue = BinaryHeap::from(jobs);
}
/// Whether the 3x3 a light job for (x, z) reads is all there: every
/// neighbour a finished column, or a decorated proto chunk past the area
/// (the last FULL ring lights against neighbours at FEATURES, as
/// `Streaming::light_region` reads them).
fn light_region_ready(
    world: &World,
    in_range: impl Fn(i32, i32) -> bool,
    decorated: &FxHashSet<(i32, i32)>,
    protos: &FxHashMap<(i32, i32), Option<Proto>>,
    x: i32,
    z: i32,
) -> bool {
    for dz in 0..3 {
        for dx in 0..3 {
            let n = (x + dx - 1, z + dz - 1);
            if world.chunks.contains_key(&n)
                || (!in_range(n.0, n.1)
                    && decorated.contains(&n)
                    && matches!(protos.get(&n), Some(Some(_))))
            {
                continue;
            }
            return false;
        }
    }
    true
}
/// The nearest `room` of the `candidates`, ordered: the light and mesh
/// stages submit bounded work, chosen from everything that is ready each
/// `update`, so what runs is always the nearest of what is left — chosen
/// again, from the current centre, as capacity frees, the way vanilla's
/// chunk task queues follow the current ticket levels. The selection is
/// O(n); the sort orders only what is submitted.
fn nearest_first<T: Ord>(candidates: &mut Vec<T>, room: usize) {
    if candidates.len() > room {
        candidates.select_nth_unstable(room);
        candidates.truncate(room);
    }
    candidates.sort_unstable();
}
/// What `keep_unchanged_sections` found changed.
#[derive(Debug, Default, PartialEq)]
struct Resent {
    /// Any section's blocks changed (in a way light or the mesher sees).
    blocks: bool,
    /// The sections whose blocks did not change but whose biomes did.
    biomes: Vec<usize>,
    /// The sections whose blocks did not change but whose light the packet changes.
    light: Vec<usize>,
}
/// `new`, a resident column `old` sent again, takes `old`'s sections whose
/// blocks it holds unchanged, with `new`'s biomes and light; the sections whose blocks
/// changed keep `old`'s light (until lit again) and their cells' classes go in
/// `cells`.
fn keep_unchanged_sections(
    old: &Chunk,
    new: &mut Chunk,
    cells: &mut crate::light::LightChanges,
) -> Resent {
    let (mut a, mut b) = (Box::new([0u32; 4096]), Box::new([0u32; 4096]));
    let mut out = Resent::default();
    let (x, z) = (new.x, new.z);
    for s in 0..new.sections.len().min(old.sections.len()) {
        // (sections stored alike hold the same blocks: most of a column sent again, found without unpacking)
        let same = old.sections[s].blocks.same_storage(&new.sections[s].blocks) || {
            old.sections[s].unpack(&mut a);
            new.sections[s].unpack(&mut b);
            let classes = crate::light::LightChanges::classes_differing(&a, &b);
            if classes != 0 {
                cells.add((x, s as i32, z), classes);
            }
            classes == 0
        };
        if !same {
            out.blocks = true;
            continue;
        }
        let biomes = new.sections[s].biomes.clone();
        let changed = !same_values(&old.sections[s].biomes, &biomes);
        let light = (!same_light(&old.sections[s].light, &new.sections[s].light))
            .then(|| new.sections[s].light.clone());
        new.sections[s] = old.sections[s].clone();
        if changed {
            Arc::make_mut(&mut new.sections[s]).biomes = biomes;
            out.biomes.push(s);
        }
        if let Some(light) = light {
            Arc::make_mut(&mut new.sections[s]).light = light;
            out.light.push(s);
        }
    }
    out
}
/// Whether two sections' packed light (4,096 cells) holds the same values.
fn same_light(a: &crate::chunk::Light, b: &crate::chunk::Light) -> bool {
    use crate::chunk::Light;
    match (a, b) {
        (Light::Uniform(x), Light::Uniform(y)) => x == y,
        (Light::Full(x), Light::Full(y)) => x == y,
        (Light::Uniform(v), Light::Full(f)) | (Light::Full(f), Light::Uniform(v)) => {
            f.iter().all(|c| c == v)
        }
    }
}
/// Whether two biome containers (64 cells) hold the same values.
fn same_values(
    a: &horizon_core::paletted::PalettedContainer,
    b: &horizon_core::paletted::PalettedContainer,
) -> bool {
    if a.same_storage(b) {
        return true;
    }
    let (mut x, mut y) = ([0u32; 64], [0u32; 64]);
    a.unpack(&mut x);
    b.unpack(&mut y);
    x == y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resent_column_keeps_its_sections_but_takes_new_biomes() {
        let mut old = Chunk::empty(2, -3);
        old.set(1, 10, 1, 1);
        old.set(1, 40, 1, 1);
        old.set_biome_cell(0, 0, 0, 1);
        let mut new = old.clone();
        // a new biome under the same blocks (section 0), and a block changed (section 6)
        new.set_biome_cell(0, 0, 0, 3);
        new.set(1, 40, 1, 2);
        let mut cells = crate::light::LightChanges::default();
        cells.keyed(crate::chunk::ColumnLayout::VANILLA);
        let kept = keep_unchanged_sections(&old, &mut new, &mut cells);
        assert_eq!(
            kept,
            Resent {
                blocks: true,
                biomes: vec![0],
                light: vec![],
            }
        );
        assert_eq!(new.biome(0, -64, 0), 3);
        assert_eq!(new.block(1, -54, 1).0, 0);
        assert_eq!(new.block(1, 40, 1).0, 2);
        // the sections alike are shared with the old column
        assert!(Arc::ptr_eq(&new.sections[1], &old.sections[1]));
        assert!(!Arc::ptr_eq(&new.sections[0], &old.sections[0]));
        // nothing changed: nothing to do
        let mut same = old.clone();
        let kept = keep_unchanged_sections(&old, &mut same, &mut cells);
        assert_eq!(kept, Resent::default());
        assert!(Arc::ptr_eq(&same.sections[0], &old.sections[0]));
        // the same blocks under new light: the packet's light, as vanilla applies it
        let mut lit = old.clone();
        Arc::make_mut(&mut lit.sections[2]).light = crate::chunk::Light::Uniform(0xF0);
        Arc::make_mut(&mut old.sections[2]).light = crate::chunk::Light::Uniform(0x50);
        let kept = keep_unchanged_sections(&old, &mut lit, &mut cells);
        assert_eq!(
            kept,
            Resent {
                light: vec![2],
                ..Resent::default()
            }
        );
        assert_eq!(lit.sections[2].light.get(0), 0xF0);
        assert!(Arc::ptr_eq(&lit.sections[1], &old.sections[1]));
    }

    #[test]
    fn mesh_pool_reuses_capacity_with_clean_geometry_state() {
        let mut mesh = SectionMesh::default();
        mesh.solid.resize(64, bytemuck::Zeroable::zeroed());
        mesh.translucent.resize(16, bytemuck::Zeroable::zeroed());
        mesh.block_entities.push(([1, 2, 3], 7));
        mesh.lights.push(([1, 2, 3], 7));
        mesh.visibility = 0;
        mesh.sides = crate::mesher::Sides::unsorted(16);
        let capacities = mesh.capacity_bytes();
        let pointers = (mesh.solid.as_ptr(), mesh.translucent.as_ptr());
        let mut pool = MeshPool::default();
        assert!(pool.put(mesh).is_none());
        assert_eq!(pool.bytes, capacities);
        let reused = pool.take();
        assert_eq!(reused.capacity_bytes(), capacities);
        assert_eq!(
            pointers,
            (reused.solid.as_ptr(), reused.translucent.as_ptr())
        );
        assert!(reused.solid.is_empty());
        assert!(reused.translucent.is_empty());
        assert!(reused.block_entities.is_empty());
        assert!(reused.lights.is_empty());
        assert_eq!(reused.visibility, crate::mesher::ALL_VISIBLE);
        assert_eq!(reused.sides, crate::mesher::Sides::default());
        assert_eq!(pool.bytes, 0);
    }

    #[test]
    fn mesh_pool_caps_retained_count_and_bytes() {
        let mut pool = MeshPool::default();
        for _ in 0..MeshPool::COUNT {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(1);
            assert!(pool.put(mesh).is_none());
        }
        let mut extra = SectionMesh::default();
        extra.solid.reserve_exact(1);
        assert!(pool.put(extra).is_some());
        for _ in 0..MeshPool::COUNT {
            pool.take();
        }
        assert_eq!(pool.bytes, 0);
        for i in 0..2 {
            let mut mesh = SectionMesh::default();
            mesh.solid.reserve_exact(
                MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() / 2 + 1,
            );
            assert_eq!(pool.put(mesh).is_some(), i == 1);
            assert!(pool.bytes <= MeshPool::BYTES);
            assert_eq!(pool.meshes.len(), 1);
        }
        pool.take();
        assert_eq!(pool.bytes, 0);
        let mut huge = SectionMesh::default();
        huge.solid.reserve_exact(
            MeshPool::BYTES / std::mem::size_of::<crate::mesher::TerrainVertex>() + 1,
        );
        assert!(pool.put(huge).is_some());
        assert!(pool.meshes.is_empty());
        assert_eq!(pool.bytes, 0);
    }

    /// The frame never waits for a worker: while one holds the mailbox, the
    /// frame takes nothing and finds the results, in order, the next time.
    #[test]
    fn the_frame_takes_finished_jobs_without_waiting() {
        let mailbox = Mailbox::default();
        let lit = |seq| Done::Lit(0, 0, Vec::new(), seq);
        mailbox.send(lit(1));
        mailbox.send(lit(2));
        let sending = mailbox.done.lock().unwrap();
        assert!(mailbox.take().is_empty());
        assert!(mailbox.try_recv().is_none());
        drop(sending);
        let seqs: Vec<u64> = mailbox
            .take()
            .into_iter()
            .map(|done| match done {
                Done::Lit(_, _, _, seq) => seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, [1, 2]);
        assert!(mailbox.take().is_empty());
        let waiting = std::time::Instant::now();
        assert!(
            mailbox
                .recv_timeout(std::time::Duration::from_millis(10))
                .is_none()
        );
        assert!(waiting.elapsed() >= std::time::Duration::from_millis(10));
    }

    #[test]
    fn queued_jobs_follow_the_centre_and_the_loading_screen() {
        let forced = HashSet::new();
        let ranges = |center| Ranges {
            center,
            radius: 32,
            extra: &[],
            forced: &forced,
            cover: None,
            band: None,
        };
        let generate = |x, z, priority, order| Queued {
            priority,
            at: Some((x, z)),
            bias: 0,
            order,
            job: Job::Generate(x, z),
        };
        let loading = ranges((0, 0));
        let queued = || {
            BinaryHeap::from(vec![
                // Queued while the loading screen was up: ring order.
                generate(3, 1, loading.priority(3, 1, true) * 4, 1),
                // Queued after it closed: plain distances, far below any ring past the first.
                generate(20, 0, loading.priority(20, 0, false) * 4, 2),
                Queued {
                    priority: i64::MIN,
                    at: None,
                    bias: 0,
                    order: 3,
                    job: Job::Save(Vec::new()),
                },
            ])
        };
        let pops = |mut queue: BinaryHeap<Queued>| {
            std::iter::from_fn(move || queue.pop())
                .map(|q| q.at)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pops(queued()),
            [None, Some((20, 0)), Some((3, 1))],
            "the stale order"
        );
        let mut queue = queued();
        rank_again(&mut queue, |x, z| loading.priority(x, z, false));
        assert_eq!(pops(queue), [None, Some((3, 1)), Some((20, 0))]);
        // The player moved: the job near the new centre goes first.
        let mut queue = BinaryHeap::from(vec![generate(3, 1, 0, 1), generate(20, 0, 1, 2)]);
        let moved = ranges((19, 0));
        rank_again(&mut queue, |x, z| moved.priority(x, z, false));
        assert_eq!(queue.pop().and_then(|q| q.at), Some((20, 0)));
    }

    /// The light and mesh stages submit bounded work, the nearest first:
    /// what exceeds the room stays dirty, to be chosen again — from the
    /// current centre — as capacity frees. Submitted whole, the queue once
    /// held every ready column's jobs and grew with the render distance
    /// (#3629); ties break by position, so the choice is deterministic.
    #[test]
    fn bounded_submission_takes_the_nearest_first() {
        let candidates: Vec<(i64, i32, i32, u32)> = (0..40i32)
            .map(|i| {
                let (x, z) = (i / 8 - 2, i % 8 - 3);
                (
                    i64::from((x - 3).pow(2) + (z - 2).pow(2)),
                    x,
                    z,
                    1 << (i % 24),
                )
            })
            .collect();
        let mut expected = candidates.clone();
        expected.sort_unstable();
        let mut none = candidates.clone();
        nearest_first(&mut none, 0);
        assert!(none.is_empty());
        let mut nearest = candidates;
        nearest_first(&mut nearest, 5);
        assert_eq!(nearest, expected[..5]);
        let mut past = expected.clone();
        nearest_first(&mut past, expected.len() + 10);
        assert_eq!(past, expected);
    }

    /// What the scheduler counts as ready to light: every neighbour a
    /// finished column, or a decorated proto chunk past the area — the
    /// walk `light_region` makes without its snapshot.
    #[test]
    fn light_readiness_follows_the_region_a_light_job_reads() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let chunk = Arc::new(Chunk::empty(0, 0));
        let nine = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)));
        for key in nine {
            world.chunks.insert(key, chunk.clone());
        }
        // The kept area is the cross around (0, 0); the corners of the
        // 3x3 light reads lie past it.
        let in_range = |x: i32, z: i32| x.abs() + z.abs() <= 1;
        let none = FxHashSet::<(i32, i32)>::default();
        let empty = FxHashMap::<(i32, i32), Option<Proto>>::default();
        assert!(light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A corner is missing, and nothing stands in for it yet.
        world.chunks.remove(&(1, 1));
        assert!(!light_region_ready(&world, in_range, &none, &empty, 0, 0));
        // A decorated, checked-in proto chunk past the area lights against.
        let decorated: FxHashSet<(i32, i32)> = [(1, 1)].into_iter().collect();
        let mut protos: FxHashMap<(i32, i32), Option<Proto>> = Default::default();
        protos.insert((1, 1), Some(Box::new(()) as Proto));
        assert!(light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // The proto is checked out to a job: it cannot be read.
        protos.insert((1, 1), None);
        assert!(!light_region_ready(
            &world, in_range, &decorated, &protos, 0, 0
        ));
        // A neighbour inside the area waits for the chunk itself.
        world.chunks.remove(&(1, 0));
        let mut inside = decorated.clone();
        inside.insert((1, 0));
        protos.insert((1, 0), Some(Box::new(()) as Proto));
        assert!(!light_region_ready(
            &world, in_range, &inside, &protos, 0, 0
        ));
    }

    #[test]
    fn view_distance_is_vanillas_shape() {
        // ChunkTrackingView.isInViewDistance at render distance 12: a column
        // of slack on each axis, so 533 columns rather than the 441 of a
        // circle of radius 12.
        let count = |r: i32| {
            (-40..=40)
                .flat_map(|x| (-40..=40).map(move |z| (x, z)))
                .filter(|&(x, z)| in_view_distance((5, -7), r, x + 5, z - 7))
                .count()
        };
        assert_eq!(count(12), 533);
        assert_eq!(count(2), 25);
        for (x, z, inside) in [(12, 0, true), (13, 0, false), (9, 9, true), (10, 9, false)] {
            assert_eq!(in_view_distance((0, 0), 12, x, z), inside, "({x}, {z})");
            assert_eq!(
                in_view_distance((0, 0), 12, -z, -x),
                inside,
                "({}, {})",
                -z,
                -x
            );
        }
    }

    /// A view has arrived once the columns at both ends of each of its rows and
    /// columns are in: exactly those with a neighbour outside it, at every radius.
    /// Then it stays arrived until it changes; a view is `recent`, and waited for,
    /// `VIEW_WAIT` at most, and one of no radius not at all.
    #[test]
    fn a_view_has_arrived_once_its_outermost_columns_are_in() {
        let start = std::time::Instant::now();
        let center = (3, -4);
        for r in 0..=16 {
            let view: FxHashSet<(i32, i32)> = (-r - 1..=r + 1)
                .flat_map(|x| (-r - 1..=r + 1).map(move |z| (center.0 + x, center.1 + z)))
                .filter(|&(x, z)| in_view_distance(center, r, x, z))
                .collect();
            let arriving = |has: &dyn Fn((i32, i32)) -> bool| {
                ViewArrival::default().arriving((center, r), start, has)
            };
            assert!(!arriving(&|c| view.contains(&c)), "radius {r}");
            for &(x, z) in &view {
                let edge = [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]
                    .iter()
                    .any(|c| !view.contains(c));
                let missing = arriving(&|c| c != (x, z) && view.contains(&c));
                assert_eq!(missing, edge, "radius {r}, column ({x}, {z})");
            }
        }
        let mut arrival = ViewArrival::default();
        let (none, all) = (|_| false, |_| true);
        assert!(arrival.arriving(((0, 0), 2), start, none));
        assert!(!arrival.arriving(((0, 0), 2), start, all));
        assert!(!arrival.arriving(((0, 0), 2), start, none));
        // Moved: waited for again, until `VIEW_WAIT` after the move.
        let (late, ms) = (start + VIEW_WAIT, std::time::Duration::from_millis(1));
        assert!(arrival.arriving(((1, 0), 2), start, none));
        assert!(arrival.arriving(((1, 0), 2), late - ms, none));
        assert!(arrival.recent(late - ms) && !arrival.recent(late));
        assert!(!arrival.arriving(((1, 0), 2), late, none));
        assert!(!arrival.arriving(((1, 0), -1), start, none));
    }

    struct GenerationFixture {
        world: World,
        center: (i32, i32),
        radius: i32,
        extra: Vec<((i32, i32), i32)>,
        forced: HashSet<(i32, i32)>,
        generating: FxHashSet<(i32, i32)>,
        protos: FxHashMap<(i32, i32), Option<Proto>>,
        finishing: FxHashSet<(i32, i32)>,
        decorates: bool,
        remote: bool,
    }
    impl GenerationFixture {
        fn new(radius: i32, decorates: bool) -> Self {
            Self {
                world: World::new(horizon_world::registry::BlockStateId(0), 0),
                center: (0, 0),
                radius,
                extra: Vec::new(),
                forced: HashSet::new(),
                generating: Default::default(),
                protos: Default::default(),
                finishing: Default::default(),
                decorates,
                remote: false,
            }
        }
        fn input(&self) -> GenerationInputs<'_> {
            GenerationInputs {
                ranges: Ranges {
                    center: self.center,
                    radius: self.radius,
                    extra: &self.extra,
                    forced: &self.forced,
                    cover: None,
                    band: None,
                },
                world: &self.world,
                generating: &self.generating,
                protos: &self.protos,
                finishing: &self.finishing,
                decorates: self.decorates,
                remote: self.remote,
                loading: false,
            }
        }
        fn fill(&mut self) {
            // Scheduler only reads membership. Share one real empty column to
            // keep a radius-32 fixture small; chunk content is never consulted.
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                self.world.chunks.insert((x, z), chunk.clone());
            }
        }
        fn fill_steady(&mut self) {
            let chunk = Arc::new(Chunk::empty(0, 0));
            for (_, x, z) in baseline_candidates(self.input(), usize::MAX) {
                if self.input().ranges.in_range(x, z, 1) {
                    self.world.chunks.insert((x, z), chunk.clone());
                } else {
                    // Generation's outer neighbour margin remains proto;
                    // only columns in the finish range become resident.
                    self.protos.insert((x, z), Some(Box::new(())));
                }
            }
        }
        fn assert_same(&self, gate: &mut GenerationScan, capacity: usize) -> bool {
            let expected = baseline_candidates(self.input(), capacity);
            let result = gate.candidates(self.input(), capacity);
            assert_eq!(result.as_deref().unwrap_or(&[]), expected);
            result.is_some()
        }
    }

    // Original scheduling scan, without snapshot bookkeeping, for differential
    // correctness and timing. Keep this independent of the production gate.
    fn baseline_candidates(input: GenerationInputs<'_>, capacity: usize) -> Vec<(i64, i32, i32)> {
        if input.remote || input.generating.len() >= capacity {
            return Vec::new();
        }
        let margin: i32 = if input.decorates { 3 } else { 1 };
        let reach = if input.decorates { 2 } else { 1 };
        let mut wanted = Vec::new();
        let areas: Vec<_> = std::iter::once((input.ranges.center, input.ranges.radius))
            .chain(input.ranges.extra.iter().copied())
            .collect();
        let missing = |x, z| {
            !input.world.chunks.contains_key(&(x, z))
                && !input.generating.contains(&(x, z))
                && !input.protos.contains_key(&(x, z))
                && !input.finishing.contains(&(x, z))
        };
        for (center, radius) in areas {
            let r = radius + margin.max(reach + 1);
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (center.0 + dx, center.1 + dz);
                    if (input.ranges.in_range(x, z, margin)
                        || input.ranges.near_finished(x, z, reach))
                        && missing(x, z)
                    {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        let forced_reach = if input.decorates {
            FORCED_TERRAIN_RING
        } else {
            FORCED_FEATURES_RING
        };
        for &(fx, fz) in input.ranges.forced {
            for dz in -forced_reach..=forced_reach {
                for dx in -forced_reach..=forced_reach {
                    let (x, z) = (fx + dx, fz + dz);
                    if missing(x, z) {
                        wanted.push((input.ranges.distance(x, z), x, z));
                    }
                }
            }
        }
        wanted.sort_unstable();
        wanted.dedup();
        wanted.truncate(capacity - input.generating.len());
        wanted
    }

    #[test]
    fn generation_gate_tracks_public_ranges_and_exact_membership() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            f.fill();
            let mut gate = GenerationScan::default();
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            // Light/mesh state and block content do not affect generation.
            f.world.dirty_light.insert((0, 0));
            f.world.dirty_sections.insert((0, 0, 0));
            Arc::make_mut(f.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            assert!(!f.assert_same(&mut gate, 6));
            // Same-count removal/replacement must not leave a hole unnoticed.
            let chunk = f.world.chunks.remove(&(0, 0)).unwrap();
            f.world.chunks.insert((100, 100), chunk.clone());
            assert!(f.assert_same(&mut gate, 6));
            f.world.chunks.remove(&(100, 100));
            f.world.chunks.insert((0, 0), chunk);
            assert!(f.assert_same(&mut gate, 6));
            assert!(!f.assert_same(&mut gate, 6));
            f.center = (1, -1);
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.radius += 1;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.push(((20, -20), 2));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.forced.insert((40, 40));
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.extra.clear();
            f.forced.clear();
            assert!(f.assert_same(&mut gate, 6));
            f.remote = true;
            assert!(!f.assert_same(&mut gate, 6));
            f.world.chunks.clear();
            f.remote = false;
            assert!(f.assert_same(&mut gate, 6));
            f.fill();
            f.assert_same(&mut gate, 6);
            f.world = World::new(horizon_world::registry::BlockStateId(0), 1);
            assert!(f.assert_same(&mut gate, 6));
        }
    }

    #[test]
    fn generation_gate_preserves_batches_stage_transitions_and_retries() {
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(2, decorates);
            let mut gate = GenerationScan::default();
            let mut scheduled = FxHashSet::default();
            loop {
                let expected = baseline_candidates(f.input(), 3);
                let batch = gate.candidates(f.input(), 3).unwrap_or_default();
                assert_eq!(batch, expected);
                if batch.is_empty() {
                    break;
                }
                for (_, x, z) in batch {
                    assert!(scheduled.insert((x, z)), "duplicate scheduling");
                    f.generating.insert((x, z));
                }
                if f.generating.len() == 3 {
                    assert!(!f.assert_same(&mut gate, 3)); // capacity pressure
                } else {
                    f.assert_same(&mut gate, 3);
                }
                for key in std::mem::take(&mut f.generating) {
                    // load miss -> proto -> checked out decoration -> finish.
                    f.protos.insert(key, None);
                }
                gate.exhausted = false; // Done::Proto/Decorated invalidation
            }
            assert!(!scheduled.is_empty());
            assert!(!f.assert_same(&mut gate, 3));
            let key = (0, 0);
            f.protos.remove(&key);
            f.finishing.insert(key);
            assert!(!f.assert_same(&mut gate, 3)); // eligibility unchanged
            f.finishing.remove(&key);
            f.world.chunks.insert(key, Arc::new(Chunk::empty(0, 0)));
            gate.exhausted = false; // Done::Generated
            assert!(f.assert_same(&mut gate, 3));
            assert!(!f.assert_same(&mut gate, 3));
            // A discarded/pruned proto must be requested again, even without
            // any range or loaded-membership change.
            let key = scheduled.iter().copied().find(|k| *k != (0, 0)).unwrap();
            f.protos.remove(&key);
            gate.exhausted = false;
            assert!(f.assert_same(&mut gate, 3));
            assert_eq!(baseline_candidates(f.input(), 3).len(), 1);
        }
    }

    #[test]
    fn generation_scan_looks_at_overlapping_areas_once() {
        // A server's players: many areas around one spot, some apart, one
        // touching another only at a corner, and part of the land present.
        for decorates in [false, true] {
            let mut f = GenerationFixture::new(10, decorates);
            for i in 0..40 {
                f.extra.push(((i % 7 - 3, i / 7 - 3), 19));
            }
            f.extra.push(((200, -150), 19));
            f.extra.push(((243, -107), 19));
            f.extra.push(((-60, 5), 4));
            f.forced.insert((90, 90));
            let chunk = Arc::new(Chunk::empty(0, 0));
            for x in -40i32..40 {
                for z in -40i32..40 {
                    match (x * 7 + z * 13).rem_euclid(5) {
                        0 => drop(f.world.chunks.insert((x, z), chunk.clone())),
                        1 => drop(f.protos.insert((x, z), None)),
                        2 if x % 3 == 0 => drop(f.generating.insert((x, z))),
                        _ => {}
                    }
                }
            }
            assert!(baseline_candidates(f.input(), usize::MAX).len() > 1000);
            for capacity in [usize::MAX, f.generating.len() + 7] {
                let mut gate = GenerationScan::default();
                assert!(f.assert_same(&mut gate, capacity));
            }
        }
    }

    #[test]
    #[ignore = "reproducible actual generation scheduler benchmark; run with --ignored --nocapture"]
    fn generation_scan_benchmark() {
        use std::{hint::black_box, time::Instant};
        let mut f = GenerationFixture::new(32, true);
        f.fill_steady();
        let mut gate = GenerationScan::default();
        f.assert_same(&mut gate, 12);
        assert!(!f.assert_same(&mut gate, 12));
        const FRAMES: usize = 2000;
        let measure = |optimized: bool, dynamic: bool| {
            let mut samples = Vec::new();
            let mut scans = 0;
            let mut jobs = 0;
            for repetition in 0..9 {
                let mut fixture = GenerationFixture::new(32, true);
                fixture.fill_steady();
                let saved = fixture.world.chunks.get(&(0, 0)).unwrap().clone();
                let mut gate = GenerationScan::default();
                gate.candidates(fixture.input(), 12);
                let start = Instant::now();
                let mut count = 0;
                let mut scheduled = 0;
                for frame in 0..FRAMES {
                    // Remove a column, queue generation, then accept a saved
                    // load completion. Both schedulers see identical events.
                    if dynamic && frame % 32 == 0 {
                        fixture.world.chunks.remove(&(0, 0));
                    }
                    if dynamic && frame % 32 == 1 {
                        fixture.generating.remove(&(0, 0));
                        fixture.world.chunks.insert((0, 0), saved.clone());
                        gate.exhausted = false; // Done::Generated
                    }
                    let result = if optimized {
                        let result = gate.candidates(black_box(fixture.input()), 12);
                        count += usize::from(result.is_some());
                        result.unwrap_or_default()
                    } else {
                        count += 1;
                        baseline_candidates(black_box(fixture.input()), 12)
                    };
                    let expected_jobs = usize::from(dynamic && frame % 32 == 0);
                    assert_eq!(result.len(), expected_jobs);
                    for (_, x, z) in black_box(result) {
                        assert_eq!((x, z), (0, 0));
                        fixture.generating.insert((x, z));
                        scheduled += 1;
                    }
                }
                if repetition != 0 {
                    samples.push(start.elapsed().as_secs_f64() * 1e9 / FRAMES as f64);
                }
                assert!(fixture.generating.is_empty());
                assert_eq!(fixture.world.chunks.len(), f.world.chunks.len());
                scans = count;
                jobs = scheduled;
            }
            samples.sort_by(f64::total_cmp);
            (samples[samples.len() / 2], scans, jobs)
        };
        println!(
            "radius=32 decorates=true positions/frame=5041 loaded_keys={} proto_keys={} frames/sample={FRAMES} warmup=1 measured=8",
            f.world.chunks.len(),
            f.protos.len()
        );
        for dynamic in [false, true] {
            let (before, baseline_scans, before_jobs) = measure(false, dynamic);
            let (after, optimized_scans, after_jobs) = measure(true, dynamic);
            assert_eq!(before_jobs, after_jobs);
            println!(
                "dynamic={dynamic}: baseline={before:.1}ns/frame optimized={after:.1}ns/frame speedup={:.2}x scans={baseline_scans}/{optimized_scans} jobs={before_jobs}/{after_jobs}",
                before / after
            );
        }
    }

    /// The ranges `Streaming::update` checks at each step, from generation to
    /// light, must let every chunk the player spawns on finish and light: the
    /// 3x3 columns around the player need their own 3x3 neighbours finished,
    /// each of those its 3x3 neighbours decorated, and each of those its 3x3
    /// neighbours generated and kept. Render distance 2 once stopped short.
    #[test]
    fn spawn_neighbourhood_completes_at_every_distance() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            let (margin, reach) = Ranges::margins(decorates);
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[],
                    forced: &forced,
                    cover: None,
                    band: None,
                };
                let square = |x: i32, z: i32, n: i32| {
                    (-n..=n).flat_map(move |dz| (-n..=n).map(move |dx| (x + dx, z + dz)))
                };
                let generated = |x, z| {
                    r.requested(x, z, decorates)
                        && (x - r.center.0).abs() <= radius + margin.max(reach + 1)
                        && (z - r.center.1).abs() <= radius + margin.max(reach + 1)
                };
                let proto_kept = |x, z| r.in_range(x, z, 4) || r.near_finished(x, z, 2);
                let decorated =
                    |x, z| !decorates || r.in_range(x, z, 2) || r.near_finished(x, z, 1);
                let chunk_kept = |x, z| r.chunk_kept(x, z, decorates);
                let unloaded = |x, z| r.unloaded(x, z, decorates);
                let finishes = |x: i32, z: i32| {
                    r.in_range(x, z, 1)
                        && chunk_kept(x, z)
                        && !unloaded(x, z)
                        && square(x, z, 1).all(|(nx, nz)| {
                            decorated(nx, nz)
                                && (!decorates
                                    || square(nx, nz, 1)
                                        .all(|(gx, gz)| generated(gx, gz) && proto_kept(gx, gz)))
                                && generated(nx, nz)
                                && proto_kept(nx, nz)
                        })
                };
                let (cx, cz) = r.center;
                for (x, z) in square(cx, cz, 1) {
                    for (nx, nz) in square(x, z, 1) {
                        assert!(
                            finishes(nx, nz),
                            "radius {radius} (decorates: {decorates}): ({nx}, {nz}) never finishes"
                        );
                    }
                }
            }
        }
    }

    /// A column generation asks for is kept when it arrives finished (loaded
    /// from the save) and is not unloaded right after: either way it would be
    /// asked for and loaded again on the next frame, forever (8 such columns
    /// after a teleport at render distance 16 held 8 of the in-flight slots).
    /// #770: while the loading screen is up, chunks go ring by ring (a
    /// square front, as vanilla's ticket levels), nearest first in a ring.
    #[test]
    fn loading_goes_ring_by_ring() {
        let forced = HashSet::new();
        let r = Ranges {
            center: (2, -1),
            radius: 12,
            extra: &[],
            forced: &forced,
            cover: None,
            band: None,
        };
        let at = |dx: i32, dz: i32| (2 + dx, -1 + dz);
        let key = |(x, z): (i32, i32), loading| r.priority(x, z, loading);
        // The corner of ring 3 comes before the edge of ring 4 only while loading.
        assert!(key(at(3, 3), true) < key(at(0, 4), true));
        assert!(key(at(3, 3), false) > key(at(0, 4), false));
        // Inside a ring, nearest first; rings never interleave.
        assert!(key(at(0, 3), true) < key(at(2, -3), true));
        assert!(key(at(-3, -3), true) < key(at(4, 0), true));
        for dx in -20..=20 {
            for dz in -20..=20 {
                assert_eq!(key(at(dx, dz), false), r.distance(2 + dx, -1 + dz));
            }
        }
    }
    #[test]
    fn requested_columns_stay_loaded() {
        let forced = HashSet::new();
        for decorates in [true, false] {
            for radius in 2..=32 {
                let r = Ranges {
                    center: (5, -7),
                    radius,
                    extra: &[((radius * 2 + 9, 3), 4)],
                    forced: &forced,
                    cover: None,
                    band: None,
                };
                let span = radius * 3 + 20;
                for x in -span..=span {
                    for z in -span..=span {
                        if r.requested(x, z, decorates) {
                            assert!(
                                r.chunk_kept(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is asked for but dropped"
                            );
                        }
                        if r.chunk_kept(x, z, decorates) {
                            assert!(
                                !r.unloaded(x, z, decorates),
                                "radius {radius} (decorates: {decorates}): ({x}, {z}) is kept then unloaded"
                            );
                        }
                    }
                }
            }
        }
    }

    /// An idle dedicated server keeps no area around its centre: only a spawn
    /// search's chunk (FULL alone: its 3x3 decorated, 5x5 generated) and the
    /// forced chunks.
    #[test]
    fn no_area_around_an_idle_servers_centre() {
        let forced = HashSet::from([(30, 30)]);
        let extra = [((5, -3), -1)];
        let r = Ranges {
            center: (5, -3),
            radius: -1,
            extra: &extra,
            forced: &forced,
            cover: None,
            band: None,
        };
        for x in -5i32..=15 {
            for z in -13i32..=7 {
                let d = (x - 5).abs().max((z + 3).abs());
                assert_eq!(r.in_range(x, z, 1), d == 0, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), d <= 2, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), d <= 2, "({x}, {z})");
            }
        }
        assert!(r.in_range(30, 30, 1));
        let idle = Ranges { extra: &[], ..r };
        assert!(!idle.requested(5, -3, true) && idle.unloaded(5, -3, true));
    }

    /// A server player's area (`lan_update_streaming`: the server view
    /// distance + 1) finishes vanilla's FULL square: PLAYER_LOADING tickets at
    /// level 31 out to the view distance, FULL (33) two chunks past it. Every
    /// chunk the player tracks has its 3x3 neighbourhood finished, so it can
    /// be lit and sent, and generation reaches vanilla's FEATURES (34) and
    /// TERRAIN (35) rings.
    #[test]
    fn server_player_areas_finish_vanillas_full_square() {
        let forced = HashSet::new();
        for view in 2..=32 {
            let extra = [((40, -9), view + 1)];
            let r = Ranges {
                // The window's own area, far away.
                center: (-400, 300),
                radius: 2,
                extra: &extra,
                forced: &forced,
                cover: None,
                band: None,
            };
            let level = |x: i32, z: i32| 31 + ((x - 40).abs().max((z + 9).abs()) - view).max(0);
            for x in 40 - view - 8..=40 + view + 8 {
                for z in -9 - view - 8..=-9 + view + 8 {
                    assert_eq!(
                        r.in_range(x, z, 1),
                        level(x, z) <= 33,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.requested(x, z, true),
                        level(x, z) <= 35,
                        "view {view}: ({x}, {z})"
                    );
                    assert_eq!(
                        r.unloaded(x, z, true),
                        level(x, z) > 35,
                        "view {view}: ({x}, {z})"
                    );
                    // ChunkTrackingView.contains, at any view distance the player may ask for.
                    let tracked = |v: i64| {
                        let gap = |d: i32| i64::from(((d).abs() - 2).max(0));
                        gap(x - 40).pow(2) + gap(z + 9).pow(2) < v * v
                    };
                    if tracked(i64::from(view)) {
                        for (dx, dz) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (dx, dz))) {
                            assert!(
                                r.in_range(x + dx, z + dz, 1),
                                "view {view}: ({x}, {z}) tracked, not lit"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A `/forceload`ed chunk holds a FORCED ticket at level 31
    /// (`ChunkMap.FORCED_TICKET_LEVEL`): the level rises one per ring
    /// (`ChunkTracker.computeLevelFromNeighbor`) and picks the column's
    /// status (`ChunkLevel.generationStatus`). So vanilla finishes the two
    /// rings around it (33), gives the third its features (34) and the
    /// fourth its terrain (35), and the rings past that are left alone.
    #[test]
    fn forced_chunks_reach_vanillas_rings() {
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        let r = Ranges {
            // The player's own area, far away.
            center: (-400, 300),
            radius: 2,
            extra: &[],
            forced: &forced,
            cover: None,
            band: None,
        };
        let level = |x: i32, z: i32| 31 + (x - 40).abs().max((z + 40).abs());
        for x in 40 - 6..=40 + 6 {
            for z in -40 - 6..=-40 + 6 {
                // The full ring: the column becomes a level chunk.
                assert_eq!(r.in_range(x, z, 1), level(x, z) <= 33, "({x}, {z})");
                // The features ring, and the terrain ring.
                assert_eq!(r.in_range(x, z, 2), level(x, z) <= 34, "({x}, {z})");
                assert_eq!(r.in_range(x, z, 3), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.requested(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.chunk_kept(x, z, true), level(x, z) <= 35, "({x}, {z})");
                assert_eq!(r.unloaded(x, z, true), level(x, z) > 35, "({x}, {z})");
            }
        }
        // Generation reaches the terrain ring: the features ring is
        // decorated from it.
        let mut fixture = GenerationFixture::new(2, true);
        fixture.center = (-400, 300);
        fixture.forced = forced;
        let candidates = GenerationScan::default()
            .candidates(fixture.input(), usize::MAX)
            .expect("a scan");
        for (dx, dz) in [(-4, 0), (4, 0), (0, -4), (0, 4), (3, 3)] {
            assert!(
                candidates
                    .iter()
                    .any(|&(_, x, z)| (x, z) == (40 + dx, -40 + dz)),
                "the terrain ring at ({}, {}) is not generated",
                40 + dx,
                -40 + dz
            );
        }
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    fn areas(seed: &mut u64, n: usize, spread: i32) -> Vec<((i32, i32), i32)> {
        let mut next = |m: i32| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % (2 * m as u64 + 1)) as i32 - m
        };
        (0..n)
            .map(|_| ((next(spread), next(spread)), 2 + next(14).abs()))
            .collect()
    }

    /// Every question streaming asks gets the same answer with the grid.
    ///
    /// A debug build checks every fourth cell of each row, shifted by row and case so the cases still cover every
    /// cell's position relative to the areas: the full check took 20 of the ~30 s of the crate's tests.
    /// `cargo test --release -p horizon-client --lib the_grid_answers_as_the_scan` checks every cell.
    #[test]
    fn the_grid_answers_as_the_scan() {
        let stride = if cfg!(debug_assertions) { 4 } else { 1 };
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let forced: HashSet<(i32, i32)> = [(40, -40)].into();
        for case in 0..60 {
            let extra = areas(&mut seed, 2 + case % 20, [4, 30, 120][case % 3]);
            let cover = ExtraCover::new(&extra).expect("a grid");
            let ranges = |cover| Ranges {
                center: (3, -2),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                band: None,
            };
            let (scan, grid) = (ranges(None), ranges(Some(&cover)));
            let span = [4, 30, 120][case % 3] + 40;
            for z in -span..=span {
                let first = -span + (z + case as i32).rem_euclid(stride);
                for x in (first..=span).step_by(stride as usize) {
                    for m in 0..=ExtraCover::REACH + 2 {
                        assert_eq!(
                            scan.in_extra(x, z, m),
                            grid.in_extra(x, z, m),
                            "case {case} ({x}, {z}) {m}"
                        );
                    }
                    for d in [false, true] {
                        assert_eq!(scan.unloaded(x, z, d), grid.unloaded(x, z, d));
                        assert_eq!(scan.requested(x, z, d), grid.requested(x, z, d));
                        assert_eq!(scan.chunk_kept(x, z, d), grid.chunk_kept(x, z, d));
                    }
                }
            }
        }
    }

    #[test]
    fn a_stale_or_impossible_grid_is_not_used() {
        let mut cover = None;
        ExtraCover::refresh(&mut cover, &[((0, 0), 11), ((5, 5), 11)]);
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 5), 11)]).is_some());
        assert!(ExtraCover::matching(&cover, &[((0, 0), 11), ((5, 6), 11)]).is_none());
        // Too far apart for a mebibyte of grid, and one area alone: scans.
        assert!(ExtraCover::new(&[((0, 0), 11), ((100_000, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((i32::MAX - 1, 0), 11), ((i32::MAX - 3, 0), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MAX - 1), 11), ((0, i32::MAX - 3), 11)]).is_none());
        assert!(ExtraCover::new(&[((0, i32::MIN + 1), 11), ((0, i32::MIN + 3), 11)]).is_none());
    }

    /// cargo test --release -p horizon-client --lib cover_tests::cover_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cover_cost() {
        let forced = HashSet::new();
        for players in [10usize, 25, 50] {
            // The benchmark's bots: most near spawn, a fifth flying outwards.
            let extra: Vec<_> = (0..players)
                .map(|i| {
                    let a = i as f64 * 2.399;
                    let d = if i % 5 == 0 { 30.0 } else { 3.0 };
                    (((a.cos() * d) as i32, (a.sin() * d) as i32), 11)
                })
                .collect();
            let cover = ExtraCover::new(&extra).unwrap();
            let ranges = |cover| Ranges {
                center: (0, 0),
                radius: 10,
                extra: &extra,
                forced: &forced,
                cover,
                band: None,
            };
            // The columns streaming asks about: everything kept, and the
            // generation scan's ring past it.
            let columns: Vec<(i32, i32)> = (-60..=60)
                .flat_map(|z| (-60..=60).map(move |x| (x, z)))
                .filter(|&(x, z)| {
                    ranges(None).requested(x, z, true) || !ranges(None).unloaded(x, z, true)
                })
                .collect();
            let rounds = 100;
            let time = |r: &Ranges| {
                let t = std::time::Instant::now();
                let mut n = 0usize;
                for _ in 0..rounds {
                    for &(x, z) in &columns {
                        n += usize::from(r.unloaded(x, z, true));
                        n += usize::from(r.requested(x, z, true));
                        n += usize::from(r.in_range(x, z, 1));
                    }
                }
                std::hint::black_box(n);
                t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds)
            };
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(ExtraCover::new(std::hint::black_box(&extra)));
            }
            let build = t.elapsed().as_secs_f64() * 1e3 / f64::from(rounds);
            println!(
                "{players} players, {} columns, 3 questions each: scan {:.3} ms, grid {:.3} ms (+ {build:.3} ms to build)",
                columns.len(),
                time(&ranges(None)),
                time(&ranges(Some(&cover))),
            );
        }
    }
}
