//! Bounded voxel-column terrain for distances beyond the resident chunk area.
//! Far tiles never enter World, lighting, simulation or the section cave graph.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    fast_hash::{FxHashMap, FxHashSet},
    mesher::SectionMesh,
    streaming::{LodSource, WorkerContext},
};
mod mesh;
use horizon_world::registry::BlockStateId;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

const CELLS: usize = 16;
const MAX_SAVED: usize = 1024;
/// How many resident chunks may wait for the worker's summary at once
/// (`Lod::observing`).
pub const OBSERVE_AHEAD: usize = 8;
const SAMPLE_JOBS: usize = 4;
/// Threads that sample far tiles beside the mesher. Machines with 12 or more
/// threads use up to 6 (measured: a cold 1024-chunk LoD loads about a quarter
/// sooner than with 3); the rest keep a quarter of their threads, at most 3.
fn sampling_threads(cores: usize) -> usize {
    if cores >= 12 {
        (cores / 2).min(6)
    } else {
        (cores / 4).clamp(1, 3)
    }
}
/// While the near terrain loads on a machine with four threads or fewer, far
/// terrain starts one sample job at most this often and no detail: it would
/// otherwise take the cores the near chunks and the frame need.
const HELD_SAMPLE_STEP: std::time::Duration = std::time::Duration::from_millis(500);
/// The coarsest tile level: `Plan` roots cover at most 1024 chunks around.
const MAX_LEVEL: u8 = 10;
/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
/// A half-open material run; gaps between runs remain air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub min: i16,
    pub max: i16,
    pub state: BlockStateId,
    pub biome: u16,
    pub light: u16,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub surface: Option<Surface>,
    pub spans: Vec<Span>,
}
impl Column {
    /// Reduce an entire horizontal footprint, rather than inflating its corner
    /// column (a single leaf used to become a whole distant tree-sized slab).
    pub fn reduced(columns: &[Column; 256], x: usize, z: usize, width: usize) -> Self {
        if width == 1 {
            return columns[z * 16 + x].clone();
        }
        let mut events = Vec::new();
        for dz in z..z + width {
            for dx in x..x + width {
                for span in &columns[dz * 16 + dx].spans {
                    let material = (span.state, span.biome, span.light);
                    events.push((span.min, material, 1));
                    events.push((span.max, material, -1));
                }
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut result = Self::default();
        let mut votes = Vec::<((BlockStateId, u16), i32)>::new();
        let mut sky = [0i32; 16];
        let mut block = [0i32; 16];
        let mut occupied = 0;
        let mut bottom = MIN_Y as i16;
        for (y, material, change) in events {
            if bottom < y {
                if occupied * 2 >= (width * width) as i32
                    && let Some(&((state, biome), _)) =
                        votes.iter().max_by_key(|&(material, count)| {
                            (*count, std::cmp::Reverse((material.0.0, material.1)))
                        })
                {
                    // Lighting must not split votes for the same material.
                    // Reduction can expose buried faces; retain the brightest
                    // actual light in this occupied footprint, per channel.
                    let light = (sky.iter().rposition(|&n| n > 0).unwrap_or(0) << 4)
                        | (block.iter().rposition(|&n| n > 0).unwrap_or(0) << 12);
                    result.push(Span {
                        min: bottom,
                        max: y,
                        state,
                        biome,
                        light: light as u16,
                    });
                }
                bottom = y;
            }
            occupied += change;
            sky[usize::from((material.2 >> 4) & 15)] += change;
            block[usize::from((material.2 >> 12) & 15)] += change;
            let material = (material.0, material.1);
            if let Some(vote) = votes.iter_mut().find(|vote| vote.0 == material) {
                vote.1 += change;
            } else {
                votes.push((material, change));
            }
        }
        result
    }
    pub fn from_surface(surface: Option<Surface>) -> Self {
        let mut column = Self {
            surface,
            spans: Vec::new(),
        };
        if let Some(s) = surface {
            if s.height > MIN_Y {
                column.spans.push(Span {
                    min: MIN_Y as i16,
                    max: s.height as i16,
                    state: s.state,
                    biome: s.biome,
                    light: 240,
                });
            }
            if let Some((height, state)) = s.water {
                column.spans.push(Span {
                    min: s.height as i16,
                    max: height as i16,
                    state,
                    biome: s.biome,
                    light: 240,
                });
            }
        }
        column
    }
    pub fn push(&mut self, span: Span) {
        if let Some(last) = self.spans.last_mut()
            && last.max == span.min
            && last.state == span.state
            && last.biome == span.biome
            && last.light == span.light
        {
            last.max = span.max;
        } else {
            self.spans.push(span);
        }
    }
    /// `kinds`: `state_kinds`, per block state.
    fn from_chunk(chunk: &Chunk, kinds: &[u8], x: i32, z: i32) -> Self {
        let kind = |state: BlockStateId| kinds.get(state.0 as usize).copied().unwrap_or(0);
        let face_light = |y: i32| {
            let mut sky = 0;
            let mut block = 0;
            for (dx, dy, dz) in [
                (0, 0, 0),
                (-1, 0, 0),
                (1, 0, 0),
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
            ] {
                if !(0..16).contains(&(x + dx)) || !(0..16).contains(&(z + dz)) {
                    continue;
                }
                let value = chunk.light(x + dx, y + dy, z + dz);
                sky = sky.max(value & 0xf0);
                block = block.max(value & 15);
            }
            sky | block
        };
        let mut column = Self::default();
        for (sy, section) in chunk.sections.iter().enumerate() {
            let min = MIN_Y + sy as i32 * 16;
            let uniform = section.blocks.bits() == 0
                && section.biomes.bits() == 0
                && matches!(section.light, crate::chunk::Light::Uniform(_));
            let uniform = uniform
                && face_light(min) == chunk.light(x, min, z)
                && face_light(min + 15) == chunk.light(x, min, z);
            let step = if uniform { 16 } else { 1 };
            for offset in (0..16).step_by(step) {
                let y = min + offset;
                let state = BlockStateId(chunk.get(x, y, z));
                let kind = kind(state);
                if kind & KEPT == 0 {
                    continue;
                }
                let light = face_light(y);
                column.push(Span {
                    min: y as i16,
                    max: (y + step as i32) as i16,
                    state,
                    biome: chunk.biome(x, y, z),
                    light: u16::from(light & 0xf0) | (u16::from((light & 15).max(kind >> 4)) << 12),
                });
            }
        }
        let mut water = None;
        for span in column.spans.iter().rev() {
            if kind(span.state) & FLUID != 0 {
                water.get_or_insert((i32::from(span.max), span.state));
            } else {
                column.surface = Some(Surface {
                    height: i32::from(span.max),
                    state: span.state,
                    water,
                    biome: span.biome,
                });
                break;
            }
        }
        if column.surface.is_none()
            && let Some((height, state)) = water
        {
            column.surface = Some(Surface {
                height: MIN_Y,
                state: BlockStateId(
                    kinds.iter().position(|&kind| kind & AIR != 0).unwrap_or(0) as u32
                ),
                water: Some((height, state)),
                biome: chunk.biome(x, height - 1, z),
            });
        }
        column
    }
}
/// A chunk's 256 columns (`Column::from_chunk`), on the far-terrain worker.
fn summarize(chunk: &Chunk, kinds: &[u8]) -> Arc<[Column; 256]> {
    Arc::new(std::array::from_fn(|i| {
        Column::from_chunk(chunk, kinds, (i & 15) as i32, (i >> 4) as i32)
    }))
}
/// `state_kinds` bits: kept in columns, a fluid, the air state; the light
/// emission is in the high nibble.
const KEPT: u8 = 1;
const FLUID: u8 = 2;
const AIR: u8 = 4;
/// What `Column::from_chunk` needs of each block state, read from the facts
/// once: matching their names for every block made observing a chunk slow.
fn state_kinds(facts: &BlockFacts) -> Vec<u8> {
    (0..facts.states.len())
        .map(|i| {
            let state = BlockStateId(i as u32);
            let f = facts.get(state);
            let fluid = matches!(&*f.name, "minecraft:water" | "minecraft:lava");
            // Chorus is thin and non-solid but a visible part of the far End.
            let chorus = matches!(
                &*f.name,
                "minecraft:chorus_plant" | "minecraft:chorus_flower"
            );
            let kept = !facts.is_air(state)
                && (f.solid_render
                    || f.full_collision
                    || f.name.ends_with("_leaves")
                    || chorus
                    || fluid);
            (u8::from(kept) * KEPT)
                | (u8::from(fluid) * FLUID)
                | (u8::from(state == facts.air) * AIR)
                | (f.emission.min(15) << 4)
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub level: u8,
}
impl TileKey {
    pub fn size(self) -> i32 {
        16 << self.level
    }
    pub fn origin(self) -> [i32; 3] {
        [self.x * self.size(), 0, self.z * self.size()]
    }
    fn parent(self) -> Self {
        Self {
            x: self.x.div_euclid(2),
            z: self.z.div_euclid(2),
            level: self.level + 1,
        }
    }
    fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            x: self.x * 2 + (i & 1) as i32,
            z: self.z * 2 + (i >> 1) as i32,
            level: self.level - 1,
        })
    }
    fn distance(self, camera: [i32; 2]) -> i32 {
        let [x, _, z] = self.origin();
        (x - camera[0])
            .max(camera[0] - x - self.size())
            .max(z - camera[1])
            .max(camera[1] - z - self.size())
            .max(0)
    }
    fn in_view(self, camera: [i32; 2], yaw: f32) -> bool {
        let [x, _, z] = self.origin();
        let (sin, cos) = yaw.sin_cos();
        let mut forward = f32::NEG_INFINITY;
        let mut left = f32::NEG_INFINITY;
        let mut right = f32::NEG_INFINITY;
        for (dx, dz) in [
            (0, 0),
            (self.size(), 0),
            (0, self.size()),
            (self.size(), self.size()),
        ] {
            let (x, z) = ((x + dx - camera[0]) as f32, (z + dz - camera[1]) as f32);
            let f = -x * sin + z * cos;
            let side = x * cos + z * sin;
            forward = forward.max(f);
            left = left.max(f * 0.866 + side * 0.5);
            right = right.max(f * 0.866 - side * 0.5);
        }
        forward > 0. && left > 0. && right > 0.
    }
}
#[derive(Default)]
struct Plan {
    roots: Vec<TileKey>,
    nodes: FxHashSet<TileKey>,
    split: FxHashSet<TileKey>,
    elevation: i32,
}
impl Plan {
    #[cfg(test)]
    fn new(camera: [i32; 2], detail: i32, distance: u32) -> Self {
        Self::with_previous(camera, detail, distance, &Self::default(), 0, false)
    }
    /// `reduced`: Reduced far detail (`graphics::FarDetail`), each level from
    /// half the distance.
    fn with_previous(
        camera: [i32; 2],
        detail: i32,
        distance: u32,
        previous: &Self,
        elevation: i32,
        reduced: bool,
    ) -> Self {
        let range = distance.min(1024) as i32 * 16;
        let mut level = 0;
        while (16 << level) < range {
            level += 1;
        }
        let size = 16 << level;
        let mut result = Self {
            elevation,
            ..Self::default()
        };
        for z in (camera[1] - range).div_euclid(size)..=(camera[1] + range).div_euclid(size) {
            for x in (camera[0] - range).div_euclid(size)..=(camera[0] + range).div_euclid(size) {
                let key = TileKey { x, z, level };
                result.roots.push(key);
                result.visit(key, camera, detail, range, previous, reduced);
            }
        }
        result
    }
    fn visit(
        &mut self,
        key: TileKey,
        camera: [i32; 2],
        detail: i32,
        range: i32,
        previous: &Self,
        reduced: bool,
    ) {
        if key.distance(camera) > range {
            return;
        }
        self.nodes.insert(key);
        // Keep distant cells small on screen; finest tiles retain individual
        // blocks. Reduced keeps them twice as large (a quarter of the cells).
        let split = if previous.split.contains(&key) { 10 } else { 8 };
        let threshold = key.size() * if reduced { split / 2 } else { split };
        if key.level > 0
            && (key.distance(camera) < detail * 16 + 16
                || key.distance(camera).max(self.elevation) < threshold)
        {
            self.split.insert(key);
            for child in key.children() {
                self.visit(child, camera, detail, range, previous, reduced);
            }
        }
    }
    fn select(
        &self,
        key: TileKey,
        ready: &FxHashMap<TileKey, impl Sized>,
        out: &mut Vec<TileKey>,
    ) -> bool {
        if !self.nodes.contains(&key) {
            return true;
        }
        if self.split.contains(&key) {
            // Visit every child: a missing sibling must not hide ready detail.
            let complete = key
                .children()
                .into_iter()
                .map(|child| self.select(child, ready, out))
                .fold(true, |complete, child| complete & child);
            if complete {
                return true;
            }
            if ready.contains_key(&key) {
                // Its mesh is clipped around every selected finer tile.
                out.push(key);
                return true;
            }
            false
        } else if ready.contains_key(&key) {
            out.push(key);
            true
        } else {
            false
        }
    }
    /// Whether chunk column (x, z) is drawn by the tile planned there (ready;
    /// at level 0, also `filled`) rather than by a coarser ancestor standing
    /// in for it, or is past the far terrain.
    fn replaces(
        &self,
        (x, z): (i32, i32),
        ready: &FxHashMap<TileKey, impl Sized>,
        filled: impl Fn(TileKey) -> bool,
    ) -> bool {
        let Some(key) = (0..=MAX_LEVEL)
            .map(|level| TileKey {
                x: (x * 16).div_euclid(16 << level),
                z: (z * 16).div_euclid(16 << level),
                level,
            })
            .find(|key| self.nodes.contains(key))
        else {
            return true;
        };
        // A split node's child there is out of range: nothing is drawn.
        self.split.contains(&key) || (ready.contains_key(&key) && (key.level > 0 || filled(key)))
    }
}
/// Visited chunks' summaries (`Lod::saved`) and their reductions.
type Saved = FxHashMap<(i32, i32), Arc<[Column; 256]>>;
type Reduced = FxHashMap<(i32, i32), Arc<ReducedColumns>>;
/// What a mesh job reads, as it was when the job began: the worker gathers
/// every tile's inputs from it (gathering on the render thread stalled the
/// frame, tens of milliseconds for a large job on a 4-thread machine).
struct Snapshot {
    active: FxHashSet<TileKey>,
    samples: FxHashMap<TileKey, Arc<Vec<Column>>>,
    saved: Saved,
    reduced: Reduced,
}
struct MeshInput {
    key: TileKey,
    /// The tile's samples, with `visited` replacing some of them (`columns`).
    samples: Arc<Vec<Column>>,
    visited: Vec<(usize, Column)>,
    neighbors: [Vec<mesh::EdgeColumn>; 4],
    finer: Vec<TileKey>,
}
struct ReducedColumns {
    columns: Arc<[Column; 256]>,
    levels: [Vec<Column>; 4],
}
impl MeshInput {
    /// Built on the worker: cloning every tile's samples on the render
    /// thread stalled it for tens of milliseconds after a move.
    fn columns(&self) -> Vec<Column> {
        let mut columns = (*self.samples).clone();
        for (index, column) in &self.visited {
            columns[*index] = column.clone();
        }
        columns
    }
}
impl ReducedColumns {
    fn new(columns: Arc<[Column; 256]>) -> Self {
        let levels = std::array::from_fn(|level| {
            let width = 2 << level;
            (0..16 / width)
                .flat_map(|z| {
                    let columns = &columns;
                    (0..16 / width)
                        .map(move |x| Column::reduced(columns, x * width, z * width, width))
                })
                .collect()
        });
        Self { columns, levels }
    }
    fn get(&self, x: usize, z: usize, width: usize) -> &Column {
        if width == 1 {
            &self.columns[z * 16 + x]
        } else {
            &self.levels[width.trailing_zeros() as usize - 1]
                [(z / width) * (16 / width) + x / width]
        }
    }
}
enum Job {
    Sample(TileKey),
    /// Summarize the chunk `Shared::observed` holds for this column.
    Observe((i32, i32)),
    RequestDetail {
        chunks: Vec<(i32, i32)>,
        center: [i32; 2],
    },
    Mesh {
        snapshot: Snapshot,
        keys: Vec<TileKey>,
        finer: FxHashMap<TileKey, Vec<TileKey>>,
        covered: FxHashSet<(i32, i32)>,
        bounds: Option<[i32; 4]>,
    },
}
enum Done {
    Detail((i32, i32), ReducedColumns),
    /// A visited chunk's summary (`Job::Observe`).
    Observed((i32, i32), ReducedColumns),
    DetailRequested,
    Sample {
        key: TileKey,
        samples: Vec<Column>,
    },
    Mesh {
        active: FxHashSet<TileKey>,
        meshes: Vec<(TileKey, SectionMesh)>,
        covered: FxHashSet<(i32, i32)>,
    },
}
struct Shared {
    jobs: Mutex<VecDeque<Job>>,
    /// The latest chunk to summarize per column, queued as `Job::Observe`
    /// once: edits made before the worker gets to it are summarized once.
    observed: Mutex<FxHashMap<(i32, i32), Arc<Chunk>>>,
    /// `state_kinds`, from the first observed chunk's facts.
    kinds: std::sync::OnceLock<Vec<u8>>,
    wake: Condvar,
    stop: AtomicBool,
    /// Reduced far detail: samplers approximate wide cells.
    approximate: AtomicBool,
}
/// One background worker and at most two outstanding sample jobs. Only the
/// current view's quadtree and a bounded set of visited summaries are retained.
pub struct Lod {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    results: mpsc::Receiver<Done>,
    #[cfg(test)]
    context: Arc<WorkerContext>,
    meshing: bool,
    /// Something `select` reads changed since it last ran.
    reselect: bool,
    /// The plan or the covered columns changed: covered leaves need stand-ins.
    replanned: bool,
    invalidated: FxHashSet<(i32, i32)>,
    area: Option<([i32; 2], i32, u32, i32, bool)>,
    plan: Plan,
    /// Reduced far detail (`set_reduced_detail`).
    reduced_detail: bool,
    pending: FxHashSet<TileKey>,
    sample_nearby: bool,
    detail_masks: FxHashMap<TileKey, [u64; 4]>,
    detail_request_pending: bool,
    detail_request_at: std::time::Instant,
    /// Four threads or fewer: far work waits for the near terrain (`hold`).
    small: bool,
    /// Sample jobs that may be queued or running at once.
    sample_jobs: usize,
    /// The near terrain is loading (`set_near_loading`).
    near_loading: bool,
    /// When `request_samples` last started a job while held.
    held_sample_at: std::time::Instant,
    /// Each tile's samples, shared with mesh jobs in flight.
    ready: FxHashMap<TileKey, Arc<Vec<Column>>>,
    active: FxHashSet<TileKey>,
    dirty: FxHashSet<TileKey>,
    removed: Vec<TileKey>,
    /// The columns left out: `full` and `retiring`.
    covered: FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    /// The full-detail columns `update` was last given.
    full: FxHashSet<(i32, i32)>,
    /// Columns that left the full-detail area, still left out (their
    /// full-detail meshes still drawn) until the tile planned there is ready
    /// (`Plan::replaces`). Tiles inside that area are never sampled: when the
    /// render distance shrinks, their coarsest ancestors would otherwise
    /// stand in for it for minutes.
    retiring: FxHashSet<(i32, i32)>,
    /// The covered columns the active meshes leave out, when a mesh job
    /// has finished since the last `take_cutout`.
    cutout: Option<FxHashSet<(i32, i32)>>,
    saved: Saved,
    reduced: Reduced,
    saved_order: VecDeque<(i32, i32)>,
}
impl Lod {
    pub fn new(source: Arc<dyn LodSource>, context: Arc<WorkerContext>) -> Self {
        let cores = thread::available_parallelism().map_or(1, |n| n.get());
        let shared = Arc::new(Shared {
            jobs: Mutex::new(VecDeque::new()),
            observed: Mutex::new(FxHashMap::default()),
            kinds: std::sync::OnceLock::new(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            approximate: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let mesh_context = context.clone();
        let (tx, results) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("Far terrain".into())
            .spawn(move || {
                // Ticks and frames come first (horizon_threads).
                horizon_threads::behind_server();
                let mut sampler = source.sampler();
                let count = sampling_threads(cores);
                let mut sampling = Vec::new();
                for _ in 0..count {
                    let Some(mut sample) = sampler.fork() else {
                        break;
                    };
                    let shared = worker.clone();
                    let tx = tx.clone();
                    sampling.push(
                        thread::Builder::new()
                            .name("Far sampling".into())
                            .spawn(move || {
                                horizon_threads::behind_server();
                                loop {
                                    let key = {
                                        let mut jobs = shared.jobs.lock().unwrap();
                                        loop {
                                            if shared.stop.load(Ordering::Relaxed) {
                                                return;
                                            }
                                            if let Some(index) = jobs
                                                .iter()
                                                .position(|job| matches!(job, Job::Sample(_)))
                                            {
                                                let Some(Job::Sample(key)) = jobs.remove(index)
                                                else {
                                                    unreachable!()
                                                };
                                                break key;
                                            }
                                            jobs = shared.wake.wait(jobs).unwrap();
                                        }
                                    };
                                    let [x, _, z] = key.origin();
                                    sample.set_approximate(
                                        shared.approximate.load(Ordering::Relaxed),
                                    );
                                    let samples =
                                        sample.grid(x, z, key.size() / CELLS as i32, CELLS);
                                    if shared.stop.load(Ordering::Relaxed) {
                                        return;
                                    }
                                    if tx.send(Done::Sample { key, samples }).is_err() {
                                        return;
                                    }
                                }
                            })
                            .expect("far sampling worker"),
                    );
                }
                let mut mesher = mesh::Mesher::default();
                'work: loop {
                    for (key, columns) in sampler.drain_details() {
                        if tx
                            .send(Done::Detail(key, ReducedColumns::new(columns)))
                            .is_err()
                        {
                            break 'work;
                        }
                    }
                    let job = {
                        let mut jobs = worker.jobs.lock().unwrap();
                        if (jobs.is_empty()
                            || (!sampling.is_empty()
                                && jobs.iter().all(|job| matches!(job, Job::Sample(_)))))
                            && !worker.stop.load(Ordering::Relaxed)
                        {
                            jobs = worker
                                .wake
                                .wait_timeout(jobs, std::time::Duration::from_millis(100))
                                .unwrap()
                                .0;
                        }
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        if sampling.is_empty() {
                            jobs.pop_front()
                        } else {
                            jobs.iter()
                                .position(|job| !matches!(job, Job::Sample(_)))
                                .and_then(|index| jobs.remove(index))
                        }
                    };
                    let Some(job) = job else { continue };
                    let done = match job {
                        Job::Observe(key) => {
                            let chunk = worker.observed.lock().unwrap().remove(&key);
                            let (Some(chunk), Some(kinds)) = (chunk, worker.kinds.get()) else {
                                continue;
                            };
                            Done::Observed(key, ReducedColumns::new(summarize(&chunk, kinds)))
                        }
                        Job::RequestDetail { chunks, center } => {
                            sampler.request_detail(&chunks, center);
                            Done::DetailRequested
                        }
                        Job::Sample(key) => {
                            let [x, _, z] = key.origin();
                            sampler.set_approximate(worker.approximate.load(Ordering::Relaxed));
                            let samples = sampler.grid(x, z, key.size() / CELLS as i32, CELLS);
                            Done::Sample { key, samples }
                        }
                        Job::Mesh {
                            snapshot,
                            keys,
                            mut finer,
                            covered,
                            bounds,
                        } => {
                            let mut meshes = Vec::with_capacity(keys.len());
                            for key in keys {
                                if worker.stop.load(Ordering::Relaxed) {
                                    break 'work;
                                }
                                let finer = finer.remove(&key).unwrap_or_default();
                                let input = snapshot.mesh_input(key, finer);
                                let mut mesh = mesher.mesh(
                                    input.key,
                                    &input.columns(),
                                    &covered,
                                    bounds,
                                    &mesh_context,
                                    &input.neighbors,
                                    &input.finer,
                                );
                                mesh.sort_solid();
                                meshes.push((input.key, mesh));
                            }
                            Done::Mesh {
                                active: snapshot.active,
                                meshes,
                                covered,
                            }
                        }
                    };
                    if tx.send(done).is_err() {
                        break;
                    }
                }
                worker.stop.store(true, Ordering::Relaxed);
                worker.wake.notify_all();
                for handle in sampling {
                    let _ = handle.join();
                }
            })
            .expect("far terrain worker");
        Self {
            shared,
            thread: Some(handle),
            results,
            #[cfg(test)]
            context,
            meshing: false,
            reselect: true,
            replanned: true,
            invalidated: FxHashSet::default(),
            area: None,
            reduced_detail: false,
            plan: Plan::default(),
            pending: FxHashSet::default(),
            sample_nearby: true,
            detail_masks: FxHashMap::default(),
            detail_request_pending: false,
            detail_request_at: std::time::Instant::now(),
            small: cores <= 4,
            sample_jobs: SAMPLE_JOBS.max(sampling_threads(cores) + 1),
            near_loading: false,
            held_sample_at: std::time::Instant::now(),
            ready: FxHashMap::default(),
            active: FxHashSet::default(),
            dirty: FxHashSet::default(),
            removed: Vec::new(),
            covered: FxHashSet::default(),
            covered_bounds: None,
            full: FxHashSet::default(),
            retiring: FxHashSet::default(),
            cutout: None,
            saved: FxHashMap::default(),
            reduced: FxHashMap::default(),
            saved_order: VecDeque::new(),
        }
    }
    /// The solid ground at block column (x, z): its top face's height and
    /// its block, from the finest tile sampled there. For aiming past the
    /// resident chunks (`game.raycast`); water is passed, as the view's ray
    /// passes it.
    pub fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockStateId)> {
        (0..=MAX_LEVEL).find_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: x.div_euclid(size),
                z: z.div_euclid(size),
                level,
            };
            let columns = self.ready.get(&key).filter(|c| c.len() == CELLS * CELLS)?;
            let cell = size / CELLS as i32;
            let (cx, cz) = ((x - key.x * size) / cell, (z - key.z * size) / cell);
            let surface = columns[cz as usize * CELLS + cx as usize].surface?;
            Some((surface.height, surface.state))
        })
    }
    pub fn stats(&self) -> (usize, usize) {
        (self.active.len(), self.ready.len())
    }
    pub fn progress(&self) -> (usize, usize) {
        let mut desired = 0;
        let mut sampled = 0;
        for key in &self.plan.nodes {
            if !self.plan.split.contains(key) {
                desired += 1;
                sampled += usize::from(self.ready.contains_key(key));
            }
        }
        (desired, sampled)
    }
    pub fn refined_samples(&self) -> usize {
        self.active
            .iter()
            .filter_map(|key| self.detail_masks.get(key))
            .flat_map(|mask| mask.iter())
            .map(|bits| bits.count_ones() as usize)
            .sum()
    }
    pub fn removed(&mut self) -> Vec<TileKey> {
        std::mem::take(&mut self.removed)
    }
    /// The full-detail columns the active meshes leave out, once per finished
    /// mesh job. Until a column leaves this set, nothing distant stands in for
    /// it: its full-detail terrain must still be drawn, in or past the render
    /// distance, or a hole opens there.
    pub fn take_cutout(&mut self) -> Option<FxHashSet<(i32, i32)>> {
        self.cutout.take()
    }
    pub fn invalidate_chunk(&mut self, x: i32, z: i32) {
        if self.meshing {
            self.invalidated.insert((x, z));
        }
        // The tiles touching the chunk: a few per level, not every active one.
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            for kz in (z * 16).div_euclid(size) - 1..=((z + 1) * 16).div_euclid(size) {
                for kx in (x * 16).div_euclid(size) - 1..=((x + 1) * 16).div_euclid(size) {
                    let key = TileKey {
                        x: kx,
                        z: kz,
                        level,
                    };
                    let [ox, _, oz] = key.origin();
                    if x * 16 <= ox + size
                        && (x + 1) * 16 >= ox
                        && z * 16 <= oz + size
                        && (z + 1) * 16 >= oz
                        && self.active.contains(&key)
                    {
                        self.dirty.insert(key);
                    }
                }
            }
        }
    }
    /// Capture actual block columns, including canopy gaps and material
    /// layers: the worker summarizes the chunk (`summarize`, a few
    /// milliseconds a chunk), then `finish_observed` keeps it.
    pub fn observe(&mut self, chunk: Arc<Chunk>, facts: &BlockFacts) {
        self.shared.kinds.get_or_init(|| state_kinds(facts));
        let key = (chunk.x, chunk.z);
        if self
            .shared
            .observed
            .lock()
            .unwrap()
            .insert(key, chunk)
            .is_none()
        {
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::Observe(key));
            self.shared.wake.notify_all();
        }
    }
    /// Chunks waiting for the worker to summarize them. A resident chunk
    /// held here is copied by its next edit (`Arc::make_mut`), so callers
    /// hand over a few at a time (`OBSERVE_AHEAD`).
    pub fn observing(&self) -> usize {
        self.shared.observed.lock().unwrap().len()
    }
    /// Keeps a visited chunk's summary, unless it is unchanged.
    fn finish_observed(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if self
            .saved
            .get(&key)
            .is_some_and(|saved| **saved == *columns.columns)
        {
            return;
        }
        if !self.saved.contains_key(&key) {
            while self.saved.len() >= MAX_SAVED {
                if let Some(old) = self.saved_order.pop_front() {
                    self.saved.remove(&old);
                    self.reduced.remove(&old);
                }
            }
            self.saved_order.push_back(key);
        }
        self.saved.insert(key, columns.columns.clone());
        self.finish_detail(key, columns);
    }
    fn apply_fine(&mut self, (cx, cz): (i32, i32), columns: &[Column; 256]) {
        let fine = TileKey {
            x: cx,
            z: cz,
            level: 0,
        };
        if self.plan.nodes.contains(&fine) || self.ready.contains_key(&fine) {
            self.ready.insert(fine, Arc::new(columns.to_vec()));
            self.reselect = true;
            self.detail_masks.insert(fine, [u64::MAX; 4]);
        }
    }
    fn apply_detail(&mut self, (cx, cz): (i32, i32), columns: &ReducedColumns) {
        self.apply_fine((cx, cz), &columns.columns);
        // A single generated chunk cannot describe a larger cell's area
        // (strides over 16 blocks, levels over 4); at each level up to that,
        // only the tile holding the chunk overlaps it.
        for level in 0..=4u8 {
            let size = 16 << level;
            let key = TileKey {
                x: (cx * 16).div_euclid(size),
                z: (cz * 16).div_euclid(size),
                level,
            };
            let Some(samples) = self.ready.get_mut(&key) else {
                continue;
            };
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let x0 = ((cx * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let x1 = (((cx + 1) * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z0 = ((cz * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z1 = (((cz + 1) * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            if x0 == x1 || z0 == z1 {
                continue;
            }
            let samples = Arc::make_mut(samples);
            let mask = self.detail_masks.entry(key).or_default();
            for z in z0..z1 {
                for x in x0..x1 {
                    let index = (z * 16 + x) as usize;
                    samples[index] = columns
                        .get(
                            (ox + x * stride).rem_euclid(16) as usize,
                            (oz + z * stride).rem_euclid(16) as usize,
                            stride as usize,
                        )
                        .clone();
                    mask[index / 64] |= 1 << (index % 64);
                }
            }
        }
    }
    fn finish_sample(&mut self, key: TileKey, samples: Vec<Column>) {
        self.pending.remove(&key);
        if self.plan.nodes.contains(&key) && samples.len() == CELLS * CELLS {
            let mut samples = samples;
            if let (Some(old), Some(mask)) = (self.ready.get(&key), self.detail_masks.get(&key)) {
                for i in 0..256 {
                    if mask[i / 64] & (1 << (i % 64)) != 0 {
                        samples[i] = old[i].clone();
                    }
                }
            }
            self.ready.insert(key, Arc::new(samples));
            self.reselect = true;
        }
    }
    fn finish_detail(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if let Some(visited) = self.saved.get(&key).cloned()
            && !Arc::ptr_eq(&visited, &columns.columns)
        {
            // Its latest reduction is queued; never restore old edits or seed data.
            self.apply_fine(key, &visited);
            return;
        }
        self.apply_detail(key, &columns);
        if self.saved.contains_key(&key) {
            self.reduced.insert(key, Arc::new(columns));
        }
        self.invalidate_chunk(key.0, key.1);
    }
    /// Whether the near terrain is still loading: on small machines far
    /// terrain then holds back (`HELD_SAMPLE_STEP`).
    pub fn set_near_loading(&mut self, loading: bool) {
        self.near_loading = loading;
    }
    /// Reduced far detail (`graphics::FarDetail::Reduced`): the next update
    /// replans when it changes, and tiles sampled from then on approximate
    /// wide cells (`GeneratorWorker::lod_grid`).
    pub fn set_reduced_detail(&mut self, reduced: bool) {
        self.reduced_detail = reduced;
        self.shared.approximate.store(reduced, Ordering::Relaxed);
    }
    fn held(&self) -> bool {
        self.small && self.near_loading
    }
    fn request_detail(&mut self, camera: [i32; 2]) {
        if self.held()
            || self.detail_request_pending
            || self.detail_request_at.elapsed() < std::time::Duration::from_millis(250)
        {
            return;
        }
        self.detail_request_at = std::time::Instant::now();
        let mut keys: Vec<_> = self
            .active
            .iter()
            .filter(|key| {
                key.level <= 1
                    && !self.plan.split.contains(key)
                    && !(key.level == 0 && self.covered.contains(&(key.x, key.z)))
                    && self.detail_masks.get(key) != Some(&[u64::MAX; 4])
            })
            .copied()
            .collect();
        keys.sort_unstable_by_key(|key| (key.distance(camera), *key));
        let mut chunks = FxHashSet::default();
        for key in keys {
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let mask = self.detail_masks.get(&key).copied().unwrap_or_default();
            for index in 0..256 {
                if mask[index / 64] & (1 << (index % 64)) == 0 {
                    let chunk = (
                        (ox + (index as i32 % 16) * stride).div_euclid(16),
                        (oz + (index as i32 / 16) * stride).div_euclid(16),
                    );
                    // A retiring column's tile is drawn soon: it needs its detail.
                    if self.full.contains(&chunk) {
                        self.detail_masks.entry(key).or_default()[index / 64] |= 1 << (index % 64);
                    } else {
                        chunks.insert(chunk);
                    }
                }
            }
            if chunks.len() >= 32 {
                break;
            }
        }
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|&(x, z)| {
            let dx = i64::from(x * 16 + 8 - camera[0]);
            let dz = i64::from(z * 16 + 8 - camera[1]);
            dx * dx + dz * dz
        });
        chunks.truncate(32);
        if !chunks.is_empty() {
            self.detail_request_pending = true;
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::RequestDetail {
                    chunks,
                    center: camera,
                });
            self.shared.wake.notify_all();
        }
    }
    pub fn update(
        &mut self,
        camera: [f64; 3],
        yaw: f32,
        detail: i32,
        distance: u32,
        covered: Option<FxHashSet<(i32, i32)>>,
    ) -> Vec<(TileKey, SectionMesh)> {
        let elevation =
            (((camera[1] - f64::from(MIN_Y + crate::chunk::HEIGHT)).clamp(0., 16384.)) as i32 / 32)
                * 32;
        let camera = [camera[0].floor() as i32, camera[2].floor() as i32];
        // Fill the view ahead first; a nearest-only ring spends most work out
        // of sight while visible forests remain undecorated noise terrain.
        let ahead = (detail * 16 + 128).max(elevation * 2).min(4096) as f32;
        let yaw = yaw.to_radians();
        let focus = [
            camera[0] - (yaw.sin() * ahead) as i32,
            camera[1] + (yaw.cos() * ahead) as i32,
        ];
        // Replan at chunk boundaries, not on every camera movement.
        let area = (
            [camera[0].div_euclid(16), camera[1].div_euclid(16)],
            detail,
            distance,
            elevation,
            self.reduced_detail,
        );
        if self.area != Some(area) {
            crate::spans::time("lod_plan", || {
                self.area = Some(area);
                self.plan = Plan::with_previous(
                    camera,
                    detail,
                    distance,
                    &self.plan,
                    elevation,
                    self.reduced_detail,
                );
                self.ready.retain(|key, _| self.plan.nodes.contains(key));
                self.detail_masks
                    .retain(|key, _| self.plan.nodes.contains(key));
                self.replanned = true;
                self.reselect = true;
            });
        }
        let mut meshes = Vec::new();
        crate::spans::time("lod_results", || self.receive(&mut meshes));
        crate::spans::time("lod_covered", || self.cover(covered));
        if !self.meshing {
            crate::spans::time("lod_select", || self.select());
        }
        crate::spans::time("lod_request", || self.request_detail(focus));
        crate::spans::time("lod_wanted", || {
            self.request_samples(camera, focus, yaw, detail, elevation)
        });
        meshes
    }
    fn receive(&mut self, meshes: &mut Vec<(TileKey, SectionMesh)>) {
        while let Ok(done) = self.results.try_recv() {
            match done {
                Done::Detail(key, columns) => {
                    crate::spans::time("lod_detail", || self.finish_detail(key, columns))
                }
                Done::Observed(key, columns) => {
                    crate::spans::time("lod_detail", || self.finish_observed(key, columns))
                }
                Done::DetailRequested => self.detail_request_pending = false,
                Done::Sample { key, samples } => self.finish_sample(key, samples),
                Done::Mesh {
                    active,
                    meshes: completed,
                    covered,
                } => {
                    self.cutout = Some(covered);
                    // Retire parents only when the entire replacement is ready.
                    self.removed
                        .extend(self.active.difference(&active).copied());
                    self.active = active;
                    self.meshing = false;
                    self.reselect = true;
                    for (x, z) in std::mem::take(&mut self.invalidated) {
                        self.invalidate_chunk(x, z);
                    }
                    meshes.extend(completed);
                }
            }
        }
    }
    /// Takes the new full-detail columns, when they changed (`None`: not).
    fn cover(&mut self, covered: Option<FxHashSet<(i32, i32)>>) {
        if let Some(full) = covered
            && self.full != full
        {
            self.retiring.extend(self.full.difference(&full).copied());
            self.retiring.retain(|column| !full.contains(column));
            self.full = full;
            self.leave_out();
        }
        // Complete resident columns need no duplicate noise work. Their real
        // summaries are retained when the full-detail chunks leave the view.
        if std::mem::take(&mut self.replanned) {
            for &key in &self.plan.nodes {
                if key.level == 0 && self.covered.contains(&(key.x, key.z)) {
                    self.ready
                        .entry(key)
                        .or_insert_with(|| Arc::new(vec![Column::default(); CELLS * CELLS]));
                }
            }
        }
    }
    /// Leaves out the full-detail and retiring columns: the tiles touching
    /// one that changed are meshed again.
    fn leave_out(&mut self) {
        let covered: FxHashSet<_> = self.full.union(&self.retiring).copied().collect();
        if self.covered == covered {
            return;
        }
        let changes: Vec<_> = self
            .covered
            .symmetric_difference(&covered)
            .copied()
            .collect();
        for (x, z) in changes {
            self.invalidate_chunk(x, z);
        }
        self.covered_bounds = covered.iter().fold(None, |bounds, &(x, z)| {
            Some(match bounds {
                None => [x * 16, z * 16, (x + 1) * 16, (z + 1) * 16],
                Some([x0, z0, x1, z1]) => [
                    x0.min(x * 16),
                    z0.min(z * 16),
                    x1.max((x + 1) * 16),
                    z1.max((z + 1) * 16),
                ],
            })
        });
        // Only new covered columns need stand-ins.
        self.replanned |= !covered.is_subset(&self.covered);
        self.covered = covered;
        self.reselect = true;
    }
    /// Chooses the tiles to draw and meshes those that changed, when the
    /// plan, the samples, the covered columns or the active tiles changed.
    fn select(&mut self) {
        if !std::mem::take(&mut self.reselect) && self.dirty.is_empty() {
            return;
        }
        if !self.retiring.is_empty() {
            let (plan, ready, saved, masks) =
                (&self.plan, &self.ready, &self.saved, &self.detail_masks);
            // A level-0 tile there is an empty stand-in (`cover`) until the
            // chunk's summary or generated detail fills it (`apply_fine`).
            self.retiring.retain(|&column| {
                !plan.replaces(column, ready, |key| {
                    saved.contains_key(&column) || masks.get(&key) == Some(&[u64::MAX; 4])
                })
            });
            self.leave_out();
        }
        {
            let mut selected = Vec::new();
            for &key in &self.plan.roots {
                self.plan.select(key, &self.ready, &mut selected);
            }
            // Their normal meshes already provide coverage. Empty stand-ins
            // must not become hundreds of redundant fallback clipping masks.
            selected.retain(|key| key.level != 0 || !self.covered.contains(&(key.x, key.z)));
            let next: FxHashSet<_> = selected.iter().copied().collect();
            let changed: Vec<_> = self.active.symmetric_difference(&next).copied().collect();
            for &tile in &next {
                let [x, _, z] = tile.origin();
                let size = tile.size();
                if changed.iter().any(|other| {
                    let [ox, _, oz] = other.origin();
                    let os = other.size();
                    (x < ox + os && ox < x + size && z < oz + os && oz < z + size)
                        || (((x + size == ox) || (ox + os == x)) && z < oz + os && oz < z + size)
                        || (((z + size == oz) || (oz + os == z)) && x < ox + os && ox < x + size)
                }) {
                    self.dirty.insert(tile);
                }
            }
            self.dirty.extend(next.difference(&self.active).copied());
            self.dirty.retain(|key| next.contains(key));
            if !self.dirty.is_empty() || next != self.active {
                let finer = Self::finer(&next, &self.dirty);
                let samples = next
                    .iter()
                    .filter_map(|key| Some((*key, self.ready.get(key)?.clone())))
                    .collect();
                let keys: Vec<_> = self.dirty.drain().collect();
                self.meshing = true;
                self.shared.jobs.lock().unwrap().push_front(Job::Mesh {
                    snapshot: Snapshot {
                        active: next,
                        samples,
                        saved: self.saved.clone(),
                        reduced: self.reduced.clone(),
                    },
                    keys,
                    finer,
                    covered: self.covered.clone(),
                    bounds: self.covered_bounds,
                });
                self.shared.wake.notify_all();
            }
        }
    }
    fn request_samples(
        &mut self,
        camera: [i32; 2],
        focus: [i32; 2],
        yaw: f32,
        detail: i32,
        elevation: i32,
    ) {
        // Every sample slot busy: no tile can be requested this frame.
        let free = if self.held() {
            usize::from(
                self.pending.is_empty() && self.held_sample_at.elapsed() >= HELD_SAMPLE_STEP,
            )
        } else {
            // The extra sampling threads wait until the near terrain is in.
            let slots = if self.near_loading {
                SAMPLE_JOBS
            } else {
                self.sample_jobs
            };
            slots.saturating_sub(self.pending.len())
        };
        if free == 0 {
            return;
        }
        if self.held() {
            self.held_sample_at = std::time::Instant::now();
        }
        // Load desired nearby leaves directly, alongside broad fallback coverage.
        // Waiting for every ancestor/sibling made full detail take minutes to appear.
        let mut wanted = Vec::new();
        for &root in &self.plan.roots {
            if self.plan.nodes.contains(&root) && !self.ready.contains_key(&root) {
                wanted.push((0, 0, root));
            }
        }
        for &key in &self.plan.nodes {
            if !self.plan.split.contains(&key) && !self.ready.contains_key(&key) {
                wanted.push((
                    if key.in_view(camera, yaw) { 1 } else { 3 },
                    i64::from(key.distance(focus)) * 4 + i64::from(key.distance(camera)),
                    key,
                ));
            }
        }
        let mut fallback = Vec::new();
        for &parent in &self.active {
            if self.plan.split.contains(&parent) {
                for child in parent.children() {
                    if self.plan.nodes.contains(&child) && !self.ready.contains_key(&child) {
                        // Covered near terrain bounds projected error. Using
                        // zero distance for a parent touching the camera kept
                        // refining it while huge visible walls waited elsewhere.
                        let priority =
                            i64::from(child.distance(camera).max((detail * 16).max(elevation)))
                                * 1024
                                / i64::from(child.size());
                        fallback.push((
                            if child.in_view(camera, yaw) { 2 } else { 4 },
                            priority,
                            child,
                        ));
                    }
                }
            }
        }
        // Only the first `free` keys not already pending are taken. A key is
        // listed at most twice (as a root and as a node), so the best
        // `2 * (sample_jobs + pending)` entries hold them, in order.
        let best = 2 * (self.sample_jobs + self.pending.len());
        if wanted.len() > best {
            wanted.select_nth_unstable(best);
            wanted.truncate(best);
        }
        wanted.sort_unstable();
        // Alternate one nearby final tile with one broad refinement tile.
        if let Some(fallback) = fallback
            .into_iter()
            .filter(|(_, _, k)| !self.pending.contains(k))
            .min()
        {
            let index = if self.sample_nearby {
                wanted
                    .iter()
                    .position(|(_, _, k)| !self.pending.contains(k))
                    .map_or(0, |i| i + 1)
            } else {
                0
            };
            wanted.insert(index, fallback);
        }
        let mut unique = FxHashSet::default();
        let wanted: Vec<_> = wanted
            .into_iter()
            .map(|(_, _, key)| key)
            .filter(|key| !self.pending.contains(key) && unique.insert(*key))
            .collect();
        if !wanted.is_empty() {
            let mut jobs = self.shared.jobs.lock().unwrap();
            for key in wanted.into_iter().take(free) {
                self.pending.insert(key);
                self.sample_nearby = !self.sample_nearby;
                jobs.push_back(Job::Sample(key));
            }
            drop(jobs);
            self.shared.wake.notify_all();
        }
    }
    /// The active tiles inside each of `keys` (finer, so each a descendant).
    fn finer(
        active: &FxHashSet<TileKey>,
        keys: &FxHashSet<TileKey>,
    ) -> FxHashMap<TileKey, Vec<TileKey>> {
        let mut finer: FxHashMap<TileKey, Vec<TileKey>> = FxHashMap::default();
        for &tile in active {
            let mut parent = tile;
            while parent.level < MAX_LEVEL {
                parent = parent.parent();
                if keys.contains(&parent) {
                    finer.entry(parent).or_default().push(tile);
                }
            }
        }
        finer
    }
    #[cfg(test)]
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            active: self.active.clone(),
            samples: self.ready.clone(),
            saved: self.saved.clone(),
            reduced: self.reduced.clone(),
        }
    }
    #[cfg(test)]
    fn mesh(&self, key: TileKey) -> SectionMesh {
        let finer = Self::finer(&self.active, &[key].into_iter().collect())
            .remove(&key)
            .unwrap_or_default();
        let input = self.snapshot().mesh_input(key, finer);
        mesh::Mesher::default().mesh(
            key,
            &input.columns(),
            &self.covered,
            self.covered_bounds,
            &self.context,
            &input.neighbors,
            &input.finer,
        )
    }
}
impl Snapshot {
    fn visited_column(&self, wx: i32, wz: i32, width: i32) -> Option<&Column> {
        let key = (wx.div_euclid(16), wz.div_euclid(16));
        if width == 1 {
            self.saved
                .get(&key)
                .map(|columns| &columns[(wz.rem_euclid(16) * 16 + wx.rem_euclid(16)) as usize])
        } else if width <= 16 {
            self.reduced.get(&key).map(|columns| {
                columns.get(
                    wx.rem_euclid(16) as usize,
                    wz.rem_euclid(16) as usize,
                    width as usize,
                )
            })
        } else {
            None
        }
    }
    fn neighbor(&self, wx: i32, wz: i32) -> Option<(Column, i32)> {
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            let key = TileKey {
                x: wx.div_euclid(size),
                z: wz.div_euclid(size),
                level,
            };
            if !self.active.contains(&key) {
                continue;
            }
            let stride = 1 << level;
            let [ox, _, oz] = key.origin();
            let x = (wx - ox) / stride;
            let z = (wz - oz) / stride;
            let (sx, sz) = (ox + x * stride, oz + z * stride);
            if let Some(column) = self.visited_column(sx, sz, stride) {
                return Some((column.clone(), stride));
            }
            return self
                .samples
                .get(&key)
                .map(|columns| (columns[(z * 16 + x) as usize].clone(), stride));
        }
        None
    }
    fn mesh_input(&self, key: TileKey, finer: Vec<TileKey>) -> MeshInput {
        let stride = key.size() / CELLS as i32;
        let [ox, _, oz] = key.origin();
        let mut visited = Vec::new();
        for z in 0..CELLS {
            for x in 0..CELLS {
                let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
                if let Some(column) = self.visited_column(wx, wz, stride) {
                    visited.push((z * CELLS + x, column.clone()));
                }
            }
        }
        let neighbors = std::array::from_fn(|direction| {
            let mut edge = Vec::new();
            let mut offset = 0;
            while offset < key.size() {
                let (wx, wz) = match direction {
                    0 => (ox - 1, oz + offset),
                    1 => (ox + key.size(), oz + offset),
                    2 => (ox + offset, oz - 1),
                    _ => (ox + offset, oz + key.size()),
                };
                let (column, neighbor_stride) = self
                    .neighbor(wx, wz)
                    .unwrap_or_else(|| (Column::default(), stride));
                let coordinate = if direction < 2 { wz } else { wx };
                let width = (neighbor_stride - coordinate.rem_euclid(neighbor_stride))
                    .min(key.size() - offset);
                edge.push(mesh::EdgeColumn {
                    start: offset,
                    end: offset + width,
                    column,
                });
                offset += width;
            }
            edge
        });
        MeshInput {
            key,
            samples: self.samples[&key].clone(),
            visited,
            neighbors,
            finer,
        }
    }
}
impl Drop for Lod {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;
    #[test]
    fn reduced_far_detail_only_merges_tiles_away_from_the_near_terrain() {
        let leaves = |plan: &Plan| {
            plan.nodes
                .iter()
                .filter(|k| !plan.split.contains(k))
                .count()
        };
        for camera in [[0, 0], [100, -40]] {
            for distance in [64, 128, 512, 1024] {
                let full = Plan::new(camera, 12, distance);
                let reduced = Plan::with_previous(camera, 12, distance, &Plan::default(), 0, true);
                // Never finer than Full anywhere, and as fine next to the
                // full-detail area, whose edge the finest tiles match.
                assert!(reduced.split.is_subset(&full.split));
                assert_eq!(reduced.roots, full.roots);
                for key in reduced.nodes.iter().filter(|k| !reduced.split.contains(k)) {
                    if key.distance(camera) < 12 * 16 + 16 {
                        assert_eq!(key.level, 0, "{key:?}");
                    }
                }
                let (full, reduced) = (leaves(&full), leaves(&reduced));
                assert!(
                    reduced * 2 < full,
                    "{reduced} of {full} tiles at {distance}"
                );
            }
        }
    }
    #[test]
    fn far_plan_is_bounded_and_near_tiles_align_with_chunks() {
        for camera in [[0, 0], [-1, -17], [1023, -4097]] {
            for distance in [64, 256, 1024] {
                let plan = Plan::new(camera, 12, distance);
                assert!(plan.nodes.len() < 12000, "{} nodes", plan.nodes.len());
                assert!(plan.roots.iter().all(|key| key.level <= MAX_LEVEL));
                for key in &plan.nodes {
                    if !plan.split.contains(key) && key.distance(camera) < 12 * 16 {
                        assert_eq!(key.level, 0);
                    }
                }
                let cells = plan
                    .nodes
                    .iter()
                    .filter(|k| !plan.split.contains(k))
                    .count();
                assert!(cells < 10000);
            }
        }
        assert_eq!(
            TileKey {
                x: -1,
                z: -2,
                level: 1
            }
            .origin(),
            [-32, 0, -64]
        );
    }
    #[test]
    fn aerial_distance_avoids_unseen_full_detail_outside_the_near_area() {
        let ground = Plan::new([0, 0], 12, 1024);
        let high = Plan::with_previous([0, 0], 12, 1024, &Plan::default(), 1024, false);
        assert!(high.nodes.len() < ground.nodes.len());
        assert!(
            high.nodes.contains(&TileKey {
                x: 0,
                z: 0,
                level: 0
            }),
            "near full-detail footprint stays exact"
        );
        assert!(
            !high.nodes.contains(&TileKey {
                x: 20,
                z: 0,
                level: 0
            }),
            "far terrain viewed from high above needs distance-appropriate detail"
        );
    }
    #[test]
    fn late_parent_keeps_ready_detail_and_only_fills_missing_children() {
        let parent = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        let mut plan = Plan::default();
        plan.roots.push(parent);
        plan.nodes.insert(parent);
        plan.split.insert(parent);
        plan.nodes.extend(parent.children());
        let mut ready: FxHashMap<TileKey, Vec<Column>> = FxHashMap::default();
        ready.insert(parent, vec![]);
        ready.insert(parent.children()[0], vec![]);
        let mut selected = Vec::new();
        ready.remove(&parent);
        assert!(!plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0]]);
        ready.insert(parent, vec![]);
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0], parent]);
        for child in parent.children() {
            ready.insert(child, vec![]);
        }
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, parent.children());
    }
    #[test]
    fn a_shrunk_render_distance_keeps_columns_until_their_planned_tiles_are_ready() {
        // Render distance 64 under LOD 64: every tile inside it is a level-0
        // stand-in for a covered column, and only the roots are sampled.
        let camera = [8, 8];
        let before = Plan::new(camera, 64, 64);
        let after = Plan::with_previous(camera, 8, 64, &before, 0, false);
        let mut ready: FxHashMap<TileKey, ()> = before
            .nodes
            .iter()
            .filter(|key| key.level == 0 || before.roots.contains(key))
            .map(|&key| (key, ()))
            .collect();
        ready.retain(|key, _| after.nodes.contains(key));
        let contains = |key: &TileKey, (x, z): (i32, i32)| {
            let [ox, _, oz] = key.origin();
            (ox..ox + key.size()).contains(&(x * 16)) && (oz..oz + key.size()).contains(&(z * 16))
        };
        // 24 chunks out, now past the render distance: its planned tile was
        // split before, so never sampled, and a root stands in for it.
        let column = (24, 0);
        let mut selected = Vec::new();
        for &root in &after.roots {
            after.select(root, &ready, &mut selected);
        }
        let drawn = selected.iter().filter(|key| contains(key, column));
        assert_eq!(drawn.map(|key| key.level).min(), Some(6));
        assert!(!after.replaces(column, &ready, |_| true));
        let planned = TileKey {
            x: 12,
            z: 0,
            level: 1,
        };
        assert!(after.nodes.contains(&planned) && before.split.contains(&planned));
        ready.insert(planned, ());
        assert!(after.replaces(column, &ready, |_| true));
        // 12 chunks out, its level-0 tile is the stand-in until filled.
        assert!(!after.replaces((12, 0), &ready, |_| false));
        assert!(after.replaces((12, 0), &ready, |_| true));
        // Past the far terrain nothing waits.
        assert!(after.replaces((100, 0), &ready, |_| false));
    }
    #[test]
    fn detail_survives_movement_across_a_split_threshold() {
        let before = Plan::new([0, 0], 2, 256);
        let after = Plan::with_previous([16, 0], 2, 256, &before, 0, false);
        let mut retained = 0;
        for key in &before.split {
            if key.distance([16, 0]) < key.size() * 10 {
                assert!(after.split.contains(key));
                retained += 1;
            }
        }
        assert!(retained > 100);
        assert!(after.nodes.len() < 16000);
    }
    #[test]
    fn reduced_material_votes_ignore_light_and_retain_exposure() {
        let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
        for i in 0..16 {
            columns[(i / 4) * 16 + i % 4].spans.push(Span {
                min: 60,
                max: 64,
                state: BlockStateId(if i < 10 { 2 } else { 1 }),
                biome: 0,
                light: if i < 10 { (i as u16 + 6) << 4 } else { 0 },
            });
        }
        let coarse = Column::reduced(&columns, 0, 0, 4);
        assert_eq!(coarse.spans.len(), 1);
        assert_eq!(
            coarse.spans[0].state,
            BlockStateId(2),
            "majority grass must beat dark dirt"
        );
        assert_eq!(coarse.spans[0].light, 240, "retain sampled sky exposure");
        for column in &mut columns {
            for span in &mut column.spans {
                span.light = 0;
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            0,
            "unlit caves stay unlit"
        );
        columns[0].spans[0].light = 7 << 12;
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            7 << 12,
            "block light does not invent skylight"
        );
    }
    #[test]
    fn coarse_voxels_reduce_the_area_instead_of_inflating_one_leaf() {
        let span = |min, max, state| Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light: 240,
        };
        let mut columns = std::array::from_fn(|_| Column {
            surface: None,
            spans: vec![span(0, 4, 1)],
        });
        columns[0].spans.push(span(8, 10, 2));
        assert_eq!(Column::reduced(&columns, 0, 0, 4).spans, [span(0, 4, 1)]);
        for z in 0..4 {
            for x in 0..4 {
                columns[z * 16 + x].spans.push(span(12, 14, 3));
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans,
            [span(0, 4, 1), span(12, 14, 3)]
        );
        assert_eq!(Column::reduced(&columns, 0, 0, 1), columns[0]);
        let reduced = ReducedColumns::new(Arc::new(columns));
        assert_eq!(
            reduced.get(0, 0, 1).spans,
            [span(0, 4, 1), span(8, 10, 2), span(12, 14, 3)]
        );
        assert_eq!(reduced.get(0, 0, 4).spans, [span(0, 4, 1), span(12, 14, 3)]);
        assert_eq!(reduced.get(4, 8, 4).spans, [span(0, 4, 1)]);
        assert_eq!(reduced.get(0, 0, 16).spans, [span(0, 4, 1)]);
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn real_mesh_clips_near_chunks_and_preserves_water_and_edited_surfaces() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables,
            testgen::TestTerrain,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let biomes = Arc::new(Biomes::load(&root)?);
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = Arc::new(WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            render,
            biomes: biomes.clone(),
        });
        let source = Arc::new(TestTerrain::new(assets.registry, &biomes)?);
        let mut lod = Lod::new(source, context);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let key = TileKey {
            x: -1,
            z: -1,
            level: 5,
        };
        lod.ready.insert(
            key,
            Arc::new(vec![
                Column::from_surface(Some(Surface {
                    height: 70,
                    state: stone,
                    water: Some((80, water)),
                    biome: biomes.id("minecraft:plains"),
                }));
                CELLS * CELLS
            ]),
        );
        // A chunk covers only one quarter of this parent's 32² cell. Both
        // opaque terrain and water must leave precisely that 16² footprint.
        lod.covered.insert((-1, -2));
        lod.covered_bounds = Some([-16, -32, 0, -16]);
        let mesh = lod.mesh(key);
        let top_area = |vertices: &[TerrainVertex], y: f32| {
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| quad.iter().all(|v| (v.pos[1] - y).abs() < 0.001))
                .map(|quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    let b = glam::Vec3::from(quad[1].pos);
                    let c = glam::Vec3::from(quad[2].pos);
                    let normal = (b - a).cross(c - a);
                    assert!(normal.y > 0.0, "surface must face the camera from above");
                    let center = (a + c) * 0.5;
                    let [ox, _, oz] = key.origin();
                    let chunk = (
                        ((center.x as i32) + ox).div_euclid(16),
                        ((center.z as i32) + oz).div_euclid(16),
                    );
                    assert!(
                        !lod.covered.contains(&chunk),
                        "LOD overlaps a full-detail chunk"
                    );
                    normal.y
                })
                .sum::<f32>()
        };
        let expected = (key.size() * key.size() - 16 * 16) as f32;
        assert_eq!(top_area(&mesh.solid, 70.0), expected);
        assert_eq!(top_area(&mesh.translucent, 80.0 - 1.0 / 9.0), expected);
        assert!(mesh.translucent.iter().all(|v| v.light == 240));
        let side = |axis: usize, plane: f32, low: f32, high: f32| {
            let other = if axis == 0 { 2 } else { 0 };
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| {
                    quad.iter().all(|v| v.pos[axis] == plane)
                        && quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min)
                            == MIN_Y as f32
                        && quad
                            .iter()
                            .map(|v| v.pos[1])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == 70.0
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::INFINITY, f32::min)
                            == low
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == high
                })
                .map(move |quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    (glam::Vec3::from(quad[1].pos) - a).cross(glam::Vec3::from(quad[2].pos) - a)
                        [axis]
                })
                .collect::<Vec<_>>()
        };
        // The two exposed edges face the missing full-detail chunk and seal
        // the height mismatch all the way down. Shared subpieces add no walls.
        let east_cut = side(0, 496.0, 480.0, 496.0);
        assert_eq!(east_cut.len(), 1);
        assert!(east_cut[0] > 0.0);
        let north_cut = side(2, 496.0, 496.0, 512.0);
        assert_eq!(north_cut.len(), 1);
        assert!(north_cut[0] < 0.0);
        assert!(side(0, 496.0, 496.0, 512.0).is_empty());

        // Block edits do not refresh Chunk.heights; observation must scan the
        // actual blocks, including builds above the old heightmap. The worker
        // summarizes the chunk; its result is kept on receipt.
        let receive_observed = |lod: &mut Lod| {
            let Done::Observed(key, columns) = lod
                .results
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            else {
                panic!("expected the observed chunk's summary");
            };
            lod.finish_observed(key, columns);
        };
        let mut chunk = Chunk::empty(-1, -1);
        chunk.set(0, 120, 0, stone.0);
        chunk.set(4, 54, 4, stone.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        // Chorus is thin and non-solid but must survive the visit summary.
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        let chorus = assets
            .registry
            .default_state("minecraft:chorus_plant")
            .unwrap();
        let flower = assets
            .registry
            .default_state("minecraft:chorus_flower")
            .unwrap();
        chunk.set(8, 70, 8, end_stone.0);
        chunk.set(8, 71, 8, chorus.0);
        chunk.set(8, 72, 8, chorus.0);
        chunk.set(8, 73, 8, flower.0);
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        let observed = &lod.saved[&(-1, -1)];
        assert_eq!(observed[0].surface.unwrap().height, 121);
        assert_eq!(observed[0].surface.unwrap().state, stone);
        assert_eq!(observed[68].surface.unwrap().height, 55);
        assert_eq!(observed[68].surface.unwrap().water, Some((64, water)));
        let chorus_column = &observed[8 * 16 + 8];
        assert_eq!(chorus_column.surface.unwrap().state, flower);
        assert_eq!(
            chorus_column
                .spans
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![(70, 71, end_stone), (71, 73, chorus), (73, 74, flower)]
        );

        let fine = TileKey {
            x: -1,
            z: -1,
            level: 0,
        };
        lod.ready
            .insert(fine, Arc::new(vec![Column::default(); 256]));
        lod.finish_detail(
            (-1, -1),
            ReducedColumns::new(Arc::new(std::array::from_fn(|_| Column::default()))),
        );
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "late seed generation cannot overwrite a visited build"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        lod.plan.nodes.insert(fine);
        lod.pending.insert(fine);
        lod.finish_sample(fine, vec![Column::default(); 256]);
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "a late coarse sampling job cannot erase completed detail"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        lod.plan.nodes.remove(&fine);

        // The worker's summary comes with the reusable pyramid; a newer
        // observation replaces both, and an unchanged one changes nothing.
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        assert!(
            lod.snapshot()
                .visited_column(-16, -16, 2)
                .unwrap()
                .spans
                .is_empty()
        );
        let old = lod.saved[&(-1, -1)].clone();
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert!(Arc::ptr_eq(&lod.saved[&(-1, -1)], &old));
        chunk.set(0, 130, 0, stone.0);
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        // A seed reduction arriving after the edit cannot restore old data.
        lod.finish_detail((-1, -1), ReducedColumns::new(old));
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);

        // Exercise the actual worker/update seam: a parent stays displayed
        // until all four child meshes can replace it in the same update.
        let camera = [-256.0, 100.0, -256.0];
        lod.area = Some(([-16, -16], 2, 64, 0, false));
        lod.plan.roots = vec![key];
        lod.plan.nodes.insert(key);
        // As a replan does (`update` only selects after a change).
        lod.reselect = true;
        let await_meshes = |lod: &mut Lod| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let meshes = lod.update(camera, 0.0, 2, 64, None);
                if !meshes.is_empty() {
                    return meshes;
                }
                assert!(std::time::Instant::now() < deadline, "mesh worker stalled");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 1);
        assert_eq!(uploaded[0].0, key);
        assert!(lod.active.contains(&key));
        lod.plan.split.insert(key);
        for child in key.children() {
            lod.plan.nodes.insert(child);
            lod.ready.insert(child, lod.ready[&key].clone());
        }
        lod.reselect = true;
        assert!(lod.update(camera, 0.0, 2, 64, None).is_empty());
        assert!(
            lod.active.contains(&key),
            "parent retired before child meshes"
        );
        assert!(lod.removed().is_empty());
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 4);
        assert!(
            key.children()
                .iter()
                .all(|child| lod.active.contains(child))
        );
        assert_eq!(lod.removed(), [key]);

        let covered_fine = TileKey {
            x: -1,
            z: -2,
            level: 0,
        };
        lod.plan = Plan::default();
        lod.plan.roots.push(covered_fine);
        lod.plan.nodes.insert(covered_fine);
        lod.ready
            .insert(covered_fine, Arc::new(vec![Column::default(); 256]));
        lod.covered.insert((-1, -2));
        lod.reselect = true;
        lod.replanned = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            lod.update(camera, 0., 2, 64, None);
            if !lod.meshing && lod.active.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "covered normal chunk remained an active LOD mask"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(lod.progress(), (1, 1));

        // Keep one sample outstanding, as on ordinary frames. Completing just
        // one job at a time must still let both loading queues make progress.
        lod.shared.stop.store(true, Ordering::Relaxed);
        lod.shared.wake.notify_all();
        lod.thread.take().unwrap().join().unwrap();
        while lod.results.try_recv().is_ok() {}
        lod.shared.jobs.lock().unwrap().clear();
        lod.plan = Plan::new([-256, -256], 2, 64);
        let parent = TileKey {
            x: -2,
            z: -2,
            level: 3,
        };
        lod.plan.roots = vec![parent];
        lod.ready.clear();
        lod.ready
            .insert(parent, Arc::new(vec![Column::default(); 256]));
        lod.active = [parent].into_iter().collect();
        lod.sample_jobs = SAMPLE_JOBS;
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.sample_nearby = true;
        lod.detail_request_pending = true;
        lod.meshing = true;
        lod.update(camera, 0.0, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(nearby)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("nearby detail was not scheduled");
        };
        assert_eq!(nearby.level, 0);
        assert!(
            nearby.origin()[2] > -256 + 64,
            "detail should load ahead of the view"
        );
        lod.finish_sample(nearby, vec![Column::default(); 256]);
        lod.update(camera, 0.0, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(fallback)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("fallback refinement was starved");
        };
        assert!(fallback.level > 0);
        lod.pending.remove(&fallback);
        lod.sample_nearby = true;
        lod.update(camera, 90.0, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(turned)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("turned view was not scheduled");
        };
        assert!(
            turned.origin()[0] < -256 - 64,
            "loading priority should follow a turn"
        );
        lod.shared.jobs.lock().unwrap().clear();
        let near = TileKey {
            x: -4,
            z: -4,
            level: 2,
        };
        let far = TileKey {
            x: 0,
            z: 0,
            level: 6,
        };
        lod.plan = Plan::default();
        lod.plan.roots = vec![near, far];
        lod.ready.clear();
        for parent in [near, far] {
            lod.plan.nodes.insert(parent);
            lod.plan.split.insert(parent);
            lod.plan.nodes.extend(parent.children());
            lod.ready
                .insert(parent, Arc::new(vec![Column::default(); 256]));
        }
        lod.active = [near, far].into_iter().collect();
        lod.shared.jobs.lock().unwrap().clear();
        lod.sample_jobs = SAMPLE_JOBS;
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.sample_nearby = false;
        lod.update(camera, 0., 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(broad)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("projected-error refinement missing")
        };
        assert_eq!(
            broad.level, 5,
            "a near tile touching the camera must not starve a huge visible distant tile"
        );
        lod.shared.jobs.lock().unwrap().clear();
        lod.active = (0..40).map(|x| TileKey { x, z: 0, level: 0 }).collect();
        lod.plan.split.clear();
        lod.detail_masks.clear();
        lod.covered = [(0, 0)].into_iter().collect();
        lod.detail_request_pending = false;
        lod.detail_request_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
        lod.request_detail([0, 0]);
        let Some(Job::RequestDetail { chunks, .. }) = lod.shared.jobs.lock().unwrap().pop_front()
        else {
            panic!("detail batch was not requested");
        };
        assert_eq!(
            chunks,
            (1..=32).map(|x| (x, 0)).collect::<Vec<_>>(),
            "one unfinished fine tile must not starve neighbouring requests"
        );

        // The extra sampling threads' slots open once the near terrain is in.
        lod.small = false;
        lod.sample_jobs = SAMPLE_JOBS + 3;
        lod.plan = Plan::new([-256, -256], 2, 64);
        lod.ready.clear();
        lod.active.clear();
        lod.pending.clear();
        lod.meshing = true;
        lod.shared.jobs.lock().unwrap().clear();
        let queued = |lod: &mut Lod, near_loading: bool| {
            lod.set_near_loading(near_loading);
            lod.shared.jobs.lock().unwrap().clear();
            lod.pending.clear();
            lod.update([-256., 64., -256.], 0., 2, 64, Some(FxHashSet::default()));
            let jobs = lod.shared.jobs.lock().unwrap();
            jobs.iter()
                .filter(|job| matches!(job, Job::Sample(_)))
                .count()
        };
        assert_eq!(queued(&mut lod, true), SAMPLE_JOBS);
        assert_eq!(queued(&mut lod, false), SAMPLE_JOBS + 3);
        Ok(())
    }

    #[test]
    fn extra_sampling_threads_only_on_large_machines() {
        assert_eq!([1, 2, 4, 8, 11].map(sampling_threads), [1, 1, 1, 2, 2]);
        assert_eq!([12, 16, 64].map(sampling_threads), [6, 6, 6]);
    }
}
