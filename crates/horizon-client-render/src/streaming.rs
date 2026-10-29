//! Chunk streaming: a worker pool generates, lights and meshes chunk columns
//! around the player, closest first. The game thread owns the world and only
//! hands immutable `Arc` snapshots to workers; results carry a sequence number
//! so stale work is dropped.
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::{
    chunk::{Chunk, SECTIONS},
    light::{LightChanges, LightTables, apply_lighting, apply_relighting, light_chunk},
    mesher::{MeshTables, Mesher, Region, SectionMesh, borrow},
    world::World,
};
use horizon_core::biome::BiomeZoom;
use horizon_world::block_render::FaceVisibility;
use horizon_world::worldgen::decoration_order::DecorationOrder;
use std::{
    collections::{BinaryHeap, HashSet},
    sync::{
        Arc, Condvar, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

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
    fn proto(&self, worker: &mut Self::Worker, x: i32, z: i32) -> Proto;
    fn decorates(&self) -> bool {
        false
    }
    /// Decorate `region[4]`; the region is row-major (dz, dx) around it.
    fn decorate(&self, _worker: &mut Self::Worker, _region: &mut [Proto; 9]) {}
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
#[derive(Clone, Copy)]
struct MeshSettings {
    nether: bool,
    zoom: BiomeZoom,
    blend_radius: i32,
    smooth_lighting: bool,
    cutout_leaves: bool,
}
enum Job {
    Generate(i32, i32),
    Decorate(i32, i32, Box<[Proto; 9]>),
    Finish(Proto),
    Light(i32, i32, Box<Region>, u64, bool),
    Mesh(i32, i32, i32, Box<Region>, u64, MeshSettings),
    /// Proto chunks checked out to be saved (`ChunkGenerator::save_protos`),
    /// off the main thread like vanilla's IOWorker.
    Save(Vec<((i32, i32), Proto)>),
    /// Meshes the renderer is done with (`Streaming::free`): their memory
    /// is given back here, not in the frame.
    Free(Vec<((i32, i32, i32), SectionMesh)>),
}
pub enum Done {
    Generated(Chunk),
    /// A proto chunk, and whether it was decorated (loaded so from the save).
    Proto(i32, i32, Proto, bool),
    Decorated(i32, i32, Box<[Proto; 9]>),
    Lit(i32, i32, Vec<crate::chunk::Light>, u64),
    Meshed(i32, i32, i32, SectionMesh, u64),
    Saved(Vec<((i32, i32), Proto)>),
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
}
pub struct Streaming {
    shared: Arc<Shared>,
    light_view: Arc<dyn Fn(&Proto) -> Option<Chunk> + Send + Sync>,
    results: mpsc::Receiver<Done>,
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
    decorated: FxHashSet<(i32, i32)>,
    decorating: usize,
    /// Save jobs in flight.
    saving: usize,
    /// Vanilla's decoration order around the player, and the centre and
    /// radius it was last ranked for.
    decoration_order: DecorationOrder,
    order_area: Option<((i32, i32), i32)>,
    /// What the queued jobs' priorities were computed from (`requeue`): the
    /// centre, the loading screen, the band, and the extra areas.
    queue_basis: (((i32, i32), bool, Option<Band>), Vec<((i32, i32), i32)>),
    finishing: FxHashSet<(i32, i32)>,
    decorates: bool,
    lighting: FxHashMap<(i32, i32), u64>,
    meshing: FxHashMap<(i32, i32, i32), u64>,
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
    /// `BiomeManager`'s zoom seed (the hashed world seed) and `biomeBlendRadius`.
    pub biome_zoom: BiomeZoom,
    pub biome_blend: i32,
    /// The `ao` and `cutoutLeaves` options.
    pub smooth_lighting: bool,
    pub cutout_leaves: bool,
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
            // Forced chunks and their generation neighbourhood.
            let reach = margin - 1;
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
        });
        let (send, results) = mpsc::channel();
        let light_tables = context.light.clone();
        let saves: Arc<dyn Saves> = generator.clone();
        let threads = (0..threads.max(1))
            .map(|i| {
                let shared = shared.clone();
                let send = send.clone();
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
                        loop {
                            let job = {
                                let mut queue = shared.queue.lock().unwrap();
                                loop {
                                    if shared.stop.load(Ordering::Relaxed) {
                                        return;
                                    }
                                    if let Some(job) = queue.pop() {
                                        break job.job;
                                    }
                                    queue = shared.ready.wait(queue).unwrap();
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
                                Job::Decorate(x, z, mut region) => {
                                    generator.decorate(&mut worker, &mut region);
                                    Done::Decorated(x, z, region)
                                }
                                Job::Finish(proto) => Done::Generated(generator.finish(proto)),
                                Job::Save(protos) => {
                                    let refs: Vec<&Proto> = protos.iter().map(|(_, p)| p).collect();
                                    generator.save_protos(&refs);
                                    Done::Saved(protos)
                                }
                                Job::Free(meshes) => {
                                    drop(meshes);
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
                                        zoom: settings.zoom,
                                        blend_radius: settings.blend_radius,
                                        smooth_lighting: settings.smooth_lighting,
                                        cutout_leaves: settings.cutout_leaves,
                                        assets: &context.assets,
                                        render: &context.render,
                                        tables: &context.tables,
                                        biomes: &context.biomes,
                                        visibility,
                                    };
                                    let mut mesh = mesher.mesh(&borrow(&region), sy);
                                    mesh.sort_solid();
                                    faces = Some((context.tables.clone(), mesher.visibility));
                                    Done::Meshed(x, sy, z, mesh, seq)
                                }
                            };
                            if send.send(done).is_err() {
                                return;
                            }
                        }
                    })
                    .expect("spawn worker")
            })
            .collect();
        let light_view = {
            let generator = generator.clone();
            Arc::new(move |p: &Proto| generator.light_view(p))
        };
        Self {
            shared,
            light_view,
            results,
            threads,
            order: 0,
            scan_age: 0,
            scan_area: ((i32::MIN, i32::MIN), i32::MIN),
            generating: Default::default(),
            generation_scan: GenerationScan::default(),
            protos: Default::default(),
            decorated: Default::default(),
            decorating: 0,
            saving: 0,
            decoration_order: DecorationOrder::default(),
            order_area: None,
            queue_basis: Default::default(),
            finishing: Default::default(),
            decorates: generator.decorates(),
            lighting: Default::default(),
            meshing: Default::default(),
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
            biome_zoom: BiomeZoom::new(0),
            biome_blend: 2,
            smooth_lighting: true,
            cutout_leaves: true,
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
        for &(x, z) in world.chunks.keys() {
            if self.lit.contains(&(x, z)) {
                for sy in 0..SECTIONS as i32 {
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
                for sy in 0..SECTIONS as i32 {
                    world.dirty_sections.insert((key.0 + dx, sy, key.1 + dz));
                }
            }
        }
    }
    /// A server's light update for a resident column: the sections whose
    /// light changed and their neighbours remesh
    /// (`setSectionDirtyWithNeighbors`).
    pub fn light_updated(world: &mut World, x: i32, z: i32, sections: &[usize]) {
        for &sy in sections {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let s = sy as i32 + dy;
                        if (0..SECTIONS as i32).contains(&s) {
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
        for sy in 0..SECTIONS as i32 {
            self.meshing.remove(&(x, sy, z));
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
    fn receive_decorated(&mut self, x: i32, z: i32, region: [Proto; 9]) {
        self.generation_scan.exhausted = false;
        self.decorating -= 1;
        self.decorated.insert((x, z));
        for (i, proto) in region.into_iter().enumerate() {
            let key = (x + (i % 3) as i32 - 1, z + (i / 3) as i32 - 1);
            self.protos.insert(key, Some(proto));
            self.written.insert(key);
        }
    }
    /// Saved proto chunks come back; those now out of range are dropped by
    /// the next `update`.
    fn receive_saved(&mut self, protos: Vec<((i32, i32), Proto)>) {
        self.generation_scan.exhausted = false;
        self.saving -= 1;
        for (key, proto) in protos {
            self.protos.insert(key, Some(proto));
        }
    }
    /// Check out the proto chunks at `keys` and save them on a worker.
    fn save_checked_out(&mut self, keys: impl IntoIterator<Item = (i32, i32)>) {
        let protos: Vec<((i32, i32), Proto)> = keys
            .into_iter()
            .filter_map(|k| Some((k, self.protos.get_mut(&k)?.take()?)))
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
        world.chunks.insert(key, Arc::new(chunk));
        if loaded {
            self.lit.insert(key);
            for sy in 0..SECTIONS as i32 {
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
    fn light_region(&self, world: &World, x: i32, z: i32) -> Option<Box<Region>> {
        let mut region: Region = Default::default();
        for dz in 0..3 {
            for dx in 0..3 {
                let n = (x + dx as i32 - 1, z + dz as i32 - 1);
                region[dz][dx] = Some(match world.chunks.get(&n) {
                    Some(chunk) => chunk.clone(),
                    None if !self.in_range(n.0, n.1, 1) && self.decorated.contains(&n) => {
                        let proto = self.protos.get(&n)?.as_ref()?;
                        Arc::new((self.light_view)(proto)?)
                    }
                    None => return None,
                });
            }
        }
        Some(Box::new(region))
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
    pub fn is_meshing(&self, section: (i32, i32, i32)) -> bool {
        self.meshing.contains_key(&section)
    }
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }
    /// Whether any chunk is queued or on its way from generation to a mesh.
    pub fn busy(&self) -> bool {
        !(self.generating.is_empty()
            && self.decorating == 0
            && self.finishing.is_empty()
            && self.lighting.is_empty()
            && self.meshing.is_empty()
            && self.pending() == 0)
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
        let received = std::iter::from_fn(|| self.results.try_recv().ok());
        // Meshes made stale meanwhile, freed with the uploaded ones (`free`).
        let mut stale = Vec::new();
        for done in deferred.into_iter().chain(received.collect::<Vec<_>>()) {
            match done {
                Done::Proto(x, z, proto, decorated) => {
                    self.generation_scan.exhausted = false;
                    self.generating.remove(&(x, z));
                    if self.in_range(x, z, 4) || self.near_finished(x, z, 2) {
                        self.protos.insert((x, z), Some(proto));
                        if decorated || !self.decorates {
                            self.decorated.insert((x, z));
                        }
                    }
                }
                Done::Decorated(x, z, region) => self.receive_decorated(x, z, *region),
                Done::Generated(chunk) => self.receive_generated(chunk, world),
                Done::Saved(protos) => self.receive_saved(protos),
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
                                    for sy in 0..SECTIONS as i32 {
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
                        && i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2) > r4 * r4
                        && !ranges.in_extra(x, z, 4)
                        && !ranges.in_band(x, z, 4)
                        && !ranges
                            .forced
                            .iter()
                            .any(|f| (x - f.0).abs() <= 3 && (z - f.1).abs() <= 3)
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
                // A final neighbour cannot receive writes: skip decoration. Only
                // worlds saved before proto chunks were saved get here (their
                // final chunks' neighbours are generated again).
                if keys.iter().any(|k| world.chunks.contains_key(k)) {
                    self.decorated.insert((x, z));
                    continue;
                }
                if !keys
                    .iter()
                    .all(|k| matches!(self.protos.get(k), Some(Some(_))))
                {
                    continue;
                }
                let region: Vec<Proto> = keys
                    .iter()
                    .map(|k| {
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
                self.submit(Some((x, z)), 1, Job::Decorate(x, z, region));
            }
        }
        self.schedule_generation(world, workers);
        // Lighting once all neighbours exist.
        let light: Vec<_> = world.dirty_light.iter().copied().collect();
        for (x, z) in light {
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_light.remove(&(x, z));
                continue;
            }
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
        let columns: Vec<_> = world.dirty_sections.columns().collect();
        for ((x, z), sections) in columns {
            if !world.chunks.contains_key(&(x, z)) {
                world.dirty_sections.take_column(x, z);
                continue;
            }
            if !self.ranges().in_view(x, z) {
                world.dirty_sections.take_column(x, z);
                for sy in section_indexes(sections) {
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
            let Some(region) = Self::region(world, x, z) else {
                continue;
            };
            world.dirty_sections.take_column(x, z);
            for sy in section_indexes(sections) {
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
                            zoom: self.biome_zoom,
                            blend_radius: self.biome_blend,
                            smooth_lighting: self.smooth_lighting,
                            cutout_leaves: self.cutout_leaves,
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
        }
        let oldest = self.light_snapshots.values().copied().min();
        match oldest {
            Some(oldest) => self.light_log.retain(|(seq, _)| *seq > oldest),
            None => self.light_log.clear(),
        }
    }
    /// Sections whose meshes sample a changed light value (faces and corners
    /// read light from neighbouring blocks).
    fn mark_light_changes(world: &mut World, changed: &LightChanges) {
        if let Some(log) = &mut world.light_log {
            for (x, sy, z) in changed.sections() {
                *log.entry((x, z)).or_default() |= 1 << sy;
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
        while self.decorating > 0 || !self.finishing.is_empty() || self.saving > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.results.recv_timeout(left) {
                Ok(Done::Decorated(x, z, region)) => self.receive_decorated(x, z, *region),
                Ok(Done::Generated(chunk)) => self.receive_generated(chunk, world),
                Ok(Done::Saved(protos)) => self.receive_saved(protos),
                Ok(other) => self.deferred.push(other),
                Err(_) => {
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
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Decorated(x, z, region) => self.receive_decorated(x, z, *region),
                Done::Generated(chunk) => self.receive_generated(chunk, world),
                Done::Saved(protos) => self.receive_saved(protos),
                other => self.deferred.push(other),
            }
            if started.elapsed() >= std::time::Duration::from_millis(2) {
                return false;
            }
        }
        if self.decorating > 0 || !self.finishing.is_empty() || self.saving > 0 {
            return false;
        }
        let written: Vec<_> = self.written.drain().collect();
        self.save_checked_out(written);
        self.saving == 0
    }
    fn in_range(&self, x: i32, z: i32, margin: i32) -> bool {
        self.ranges().in_range(x, z, margin)
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
                if self.light_region(world, key.0, key.1).is_some() {
                    world.dirty_light.insert(key);
                }
            }
        }
        counts
    }
    /// Wait up to `timeout` for a worker to finish a job; the next `update`
    /// takes it. A server pumps generation with this between its ticks.
    pub fn wait(&mut self, timeout: std::time::Duration) {
        if let Ok(done) = self.results.recv_timeout(timeout) {
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
        dx * dx + dz * dz <= r * r
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
        if dx * dx + dz * dz <= r * r {
            return true;
        }
        let reach = (margin - 1).max(0);
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
            match self.results.recv_timeout(left) {
                Ok(Done::Saved(_)) => self.saving -= 1,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.ready.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
/// The section indexes set in a column's bits, lowest first.
fn section_indexes(mut bits: u32) -> impl Iterator<Item = i32> {
    std::iter::from_fn(move || {
        (bits != 0).then(|| {
            let sy = bits.trailing_zeros() as i32;
            bits &= bits - 1;
            sy
        })
    })
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
#[cfg(test)]
mod tests {
    use super::*;

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
        for &(fx, fz) in input.ranges.forced {
            for dz in -(margin - 1)..=(margin - 1) {
                for dx in -(margin - 1)..=(margin - 1) {
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
