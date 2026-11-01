//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera. As in
//! vanilla's `runUpdates`, a section leads on through whatever any of the faces
//! it was entered through connects to, and beyond 60 blocks a step is taken
//! only if the line from the section towards the camera stays in sections the
//! search has reached.
//!
//! As in vanilla, the full search runs off the render thread: a worker keeps
//! its own copy of every section's connectivity and searches over dense
//! arrays around the camera. Frames use the last finished graph and never wait
//! for the next one. A section meshed in between propagates into the graph the
//! frame has (vanilla's partial update), so what it opens shows at once; what
//! it closes goes at the next full search.
use super::SectionKey;
use crate::mesher::ALL_VISIBLE;
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Vanilla's `Direction` order: down, up, north, south, west, east.
const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// In `Graph::directions`: the cell has a node.
const REACHED: u8 = 1 << 7;
/// `MINIMUM_ADVANCED_CULLING_SECTION_DISTANCE`: `blockToSectionCoord(60)`.
const DISTANT_SECTIONS: i32 = 60 >> 4;
/// `MINIMUM_ADVANCED_CULLING_DISTANCE` (60 blocks), squared.
const DISTANT_SQR: f64 = 3600.0;
/// `CEILED_SECTION_DIAGONAL`: `ceil(sqrt(3) * 16)`.
const SECTION_DIAGONAL: f64 = 28.0;
/// In `Graph::rays`: the line from this corner stays in reached sections.
const SEES: u8 = u8::MAX;

/// VisGraph connectivity per column, one entry per section (bottom first).
/// A missing column or section connects every face.
struct Columns {
    /// The lowest block y of section 0 (`Level.getMinY`).
    min_y: i32,
    sections: i32,
    map: HashMap<(i32, i32), Box<[u64]>>,
}

impl Columns {
    fn new(min_y: i32, sections: i32) -> Self {
        Self {
            min_y,
            sections,
            map: HashMap::new(),
        }
    }
    /// `SectionPos.of`: the section holding `pos`.
    fn section_of(&self, pos: DVec3) -> SectionKey {
        (
            (pos.x.floor() as i32) >> 4,
            (pos.y.floor() as i32 - self.min_y) >> 4,
            (pos.z.floor() as i32) >> 4,
        )
    }
    /// Whether the section's connectivity changed.
    fn set(&mut self, key: SectionKey, vis: u64) -> bool {
        if !(0..self.sections).contains(&key.1) {
            return false;
        }
        let column = match self.map.get_mut(&(key.0, key.2)) {
            Some(column) => column,
            None if vis == ALL_VISIBLE => return false,
            None => self
                .map
                .entry((key.0, key.2))
                .or_insert_with(|| vec![ALL_VISIBLE; self.sections as usize].into()),
        };
        let slot = &mut column[key.1 as usize];
        if *slot == vis {
            return false;
        }
        *slot = vis;
        if vis == ALL_VISIBLE && column.iter().all(|&v| v == ALL_VISIBLE) {
            self.map.remove(&(key.0, key.2));
        }
        true
    }
    fn get(&self, key: SectionKey) -> u64 {
        self.map
            .get(&(key.0, key.2))
            .and_then(|column| column.get(usize::try_from(key.1).ok()?))
            .copied()
            .unwrap_or(ALL_VISIBLE)
    }
    fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(key, vis) => {
                self.set(key, vis);
            }
            Edit::Clear => self.map.clear(),
        }
    }
}

#[derive(Clone, Copy)]
enum Edit {
    Set(SectionKey, u64),
    Clear,
}

/// One search's result over the box around the camera: vanilla's
/// `GraphStorage.sectionToNodeMap`, as dense arrays.
#[derive(Default)]
struct Graph {
    from: SectionKey,
    radius: i32,
    /// The world: lowest block y and height in sections.
    min_y: i32,
    sections: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    /// Per cell: `REACHED`, and the directions travelled from the camera to
    /// it (`Node.directions`).
    directions: Vec<u8>,
    /// Per cell: the faces it was entered through, one bit per face
    /// (`Node.sourceDirections`, as the faces rather than the steps); none for
    /// the camera's.
    entered: Vec<u8>,
    /// Whether any section in the searched top layer was reached. Sections
    /// above a clamped box use this conservative sky-band answer.
    sky_reached: bool,
    /// Per section corner, during a full search: `SEES`, or how far the line
    /// from it towards the camera is known to stay in reached sections (the
    /// first sample that was not). Sections are only ever added during a
    /// search, so a line that stays in reached sections keeps doing so, and
    /// one that didn't need only be checked again from where it failed.
    rays: Vec<u8>,
}

impl Graph {
    fn index(&self, key: SectionKey) -> Option<usize> {
        let (dx, dz, dy) = (key.0 - self.from.0, key.2 - self.from.2, key.1 - self.y0);
        let side = 2 * self.radius + 1;
        if dx.abs() > self.radius || dz.abs() > self.radius || !(0..self.height).contains(&dy) {
            return None;
        }
        Some((((dz + self.radius) * side + dx + self.radius) * self.height + dy) as usize)
    }
    fn key(&self, index: usize) -> SectionKey {
        let side = 2 * self.radius + 1;
        let i = index as i32;
        let column = i / self.height;
        (
            self.from.0 + column % side - self.radius,
            self.y0 + i % self.height,
            self.from.2 + column / side - self.radius,
        )
    }
    fn contains(&self, key: SectionKey) -> bool {
        if let Some(i) = self.index(key) {
            self.directions[i] & REACHED != 0
        } else if (0..=self.sections).contains(&key.1)
            && key.1 >= self.y0 + self.height
            && (key.0 - self.from.0).abs() <= self.radius
            && (key.2 - self.from.2).abs() <= self.radius
        {
            // Everything above the highest non-air section is open sky. A
            // full search can reach that layer from a neighbouring column,
            // so checking only this column's top cell can hide an upper
            // section that is visible through the open layer. Once any top
            // cell is reached, conservatively keep the whole sky band visible.
            self.sky_reached
        } else {
            false
        }
    }
    /// The full search from the camera (vanilla's `scheduleFullUpdate`).
    fn search(
        &mut self,
        columns: &Columns,
        camera: DVec3,
        radius: i32,
        vis: &mut Vec<u64>,
        queue: &mut VecDeque<u32>,
    ) {
        let from = columns.section_of(camera);
        self.from = from;
        self.radius = radius;
        self.min_y = columns.min_y;
        self.sections = columns.sections;
        self.y0 = -1;
        if from.1 >= columns.sections {
            self.height = columns.sections + 2;
        } else {
            let mut max_non_air_y = 0;
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                        continue;
                    };
                    for (y, &v) in column.iter().enumerate().rev() {
                        let y = y as i32;
                        if y <= max_non_air_y {
                            break;
                        }
                        if v != ALL_VISIBLE {
                            max_non_air_y = y;
                            break;
                        }
                    }
                }
            }
            let top_y = max_non_air_y.max(from.1);
            self.height = (top_y + 3).max(3).min(columns.sections + 2);
        }
        let side = (2 * radius + 1) as usize;
        let size = side * side * self.height as usize;
        self.directions.clear();
        self.directions.resize(size, 0);
        self.entered.clear();
        self.entered.resize(size, 0);
        self.sky_reached = false;
        self.rays.clear();
        self.rays
            .resize((side + 1) * (side + 1) * (self.height + 1) as usize, 0);
        vis.clear();
        vis.resize(size, ALL_VISIBLE);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                    continue;
                };
                let base = self
                    .index((from.0 + dx, 0, from.2 + dz))
                    .expect("column inside the searched box");
                let len = (column.len() as i32).min(self.height - 1) as usize;
                vis[base..base + len].copy_from_slice(&column[..len]);
            }
        }
        queue.clear();
        if (0..columns.sections).contains(&from.1) {
            let start = self.index(from).expect("camera inside the searched box");
            self.directions[start] = REACHED;
            queue.push_back(start as u32);
        } else {
            self.seed_outside(camera, queue);
        }
        self.propagate(queue, camera, true, |i, _| vis[i]);
    }
    /// `initializeQueueForFullUpdate` with the camera above or below the
    /// world (no render section there): every section of the world's top
    /// layer (bottom layer from below) is entered through its outer face, its
    /// travelled directions pointing away from the camera's column, sorted
    /// by the squared distance from the camera's block to the section's
    /// centre (a stable sort over vanilla's x-then-z loop). Vanilla seeds only
    /// columns in view distance; this search covers its whole box, as it does
    /// from inside the world.
    fn seed_outside(&mut self, camera: DVec3, queue: &mut VecDeque<u32>) {
        // STEPS indexes: down, up, north, south, west, east.
        let (y, travel) = if self.from.1 < 0 {
            (0, 1)
        } else {
            (self.sections - 1, 0)
        };
        let block = (camera.x.floor() as i64, camera.z.floor() as i64);
        let mut seeds = Vec::with_capacity(((2 * self.radius + 1) as usize).pow(2));
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let mut directions = REACHED | 1 << travel;
                directions |= match dx.signum() {
                    1 => 1 << 5,
                    -1 => 1 << 4,
                    _ => 0,
                };
                directions |= match dz.signum() {
                    1 => 1 << 3,
                    -1 => 1 << 2,
                    _ => 0,
                };
                // The y offset is the same for every seed.
                let centre = |section: i32| i64::from(section) * 16 + 8;
                let (ox, oz) = (
                    block.0 - centre(self.from.0 + dx),
                    block.1 - centre(self.from.2 + dz),
                );
                seeds.push((ox * ox + oz * oz, dx, dz, directions));
            }
        }
        seeds.sort_by_key(|seed| seed.0);
        for (_, dx, dz, directions) in seeds {
            let i = self
                .index((self.from.0 + dx, y, self.from.2 + dz))
                .expect("seed inside the searched box");
            self.directions[i] = directions;
            // Entered through the face opposite the travel (`sourceDirection`).
            self.entered[i] = 1 << (travel ^ 1);
            queue.push_back(i as u32);
        }
    }
    /// Vanilla's `runUpdates`: take nodes from the queue and step through
    /// every face their entry faces connect to, away from the camera. A step
    /// into a section already reached adds the face it enters through.
    /// `full`: the search from `camera` (`rays` holds its lines), not a
    /// partial update.
    fn propagate(
        &mut self,
        queue: &mut VecDeque<u32>,
        camera: DVec3,
        full: bool,
        vis: impl Fn(usize, SectionKey) -> u64,
    ) {
        let eye = (
            (camera.x.floor() as i32) >> 4,
            (camera.y.floor() as i32 - self.min_y) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        // `SectionPos.center` of the camera's section.
        let centre = [eye.0 * 16 + 8, eye.1 * 16 + self.min_y + 8, eye.2 * 16 + 8];
        while let Some(index) = queue.pop_front() {
            let i = index as usize;
            let key = self.key(i);
            if key.1 == self.y0 + self.height - 1 {
                self.sky_reached = true;
            }
            let directions = self.directions[i];
            let entered = self.entered[i];
            let exits = if entered == 0 {
                0x3f
            } else {
                let vis = vis(i, key);
                (0..6)
                    .filter(|face| entered & (1 << face) != 0)
                    .fold(0, |exits, face| exits | (vis >> (face * 6)) as u8 & 0x3f)
            };
            let distant = (key.0 - eye.0).abs() > DISTANT_SECTIONS
                || (key.1 - eye.1).abs() > DISTANT_SECTIONS
                || (key.2 - eye.2).abs() > DISTANT_SECTIONS;
            // The distant test once per axis: both steps along an axis start
            // from the same corner. While the camera is in the section the
            // graph was searched from, every step leads away from it or
            // sideways, so no point of the line falls in a neighbour this loop
            // adds. A partial update after the camera moved tests every step.
            let reuse = eye == self.from;
            let mut seen = [None; 3];
            for (d, step) in STEPS.iter().enumerate() {
                if exits & (1 << d) == 0 || directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > self.sections {
                    continue;
                }
                let Some(n) = self.index(next) else { continue };
                if distant {
                    let sees = match seen[d / 2] {
                        Some(sees) if reuse => sees,
                        _ => self.sees_camera(key, d, centre, camera, full),
                    };
                    seen[d / 2] = Some(sees);
                    if !sees {
                        continue;
                    }
                }
                if self.directions[n] & REACHED == 0 {
                    self.directions[n] = directions | 1 << d;
                    self.entered[n] = 1 << (d ^ 1);
                    queue.push_back(n as u32);
                } else {
                    self.entered[n] |= 1 << (d ^ 1);
                }
            }
        }
    }
    /// Vanilla's distant step test: from the corner of `key` that the step `d`
    /// picks, walk towards the camera in section diagonals while farther than
    /// 60 blocks. Every point inside the world has to fall in a section the
    /// search has reached. Same arithmetic as JOML's `Vector3d` (no FMA).
    /// `full`: remember the answer per corner in `rays`.
    fn sees_camera(
        &mut self,
        key: SectionKey,
        d: usize,
        centre: [i32; 3],
        camera: DVec3,
        full: bool,
    ) -> bool {
        let origin = [key.0 * 16, key.1 * 16 + self.min_y, key.2 * 16];
        // The step's axis in x, y, z order: down/up, north/south, west/east.
        let axis = [1, 2, 0][d / 2];
        let far = |c: usize| {
            i32::from(if c == axis {
                centre[c] > origin[c]
            } else {
                centre[c] < origin[c]
            })
        };
        let corner = (key.0 + far(0), key.1 + far(1), key.2 + far(2));
        let slot = full.then(|| {
            let side = 2 * self.radius + 2;
            let (x, z) = (
                corner.0 - self.from.0 + self.radius,
                corner.2 - self.from.2 + self.radius,
            );
            ((z * side + x) * (self.height + 1) + corner.1 - self.y0) as usize
        });
        let known = slot.map_or(0, |slot| self.rays[slot]);
        if known == SEES {
            return true;
        }
        let failed = self.walk(corner, camera, known);
        if let Some(slot) = slot {
            self.rays[slot] = failed.unwrap_or(SEES);
        }
        failed.is_none()
    }
    /// The line from the section corner `corner` towards the camera; `None`
    /// if every sample is in a reached section, else the first one that is
    /// not (counted from 1). The first `known - 1` samples are known to be
    /// reached: they are stepped over (the same additions, so the same
    /// points) without looking them up.
    fn walk(&self, corner: SectionKey, camera: DVec3, known: u8) -> Option<u8> {
        let mut check = [
            f64::from(corner.0 * 16),
            f64::from(corner.1 * 16 + self.min_y),
            f64::from(corner.2 * 16),
        ];
        let camera = [camera.x, camera.y, camera.z];
        let to = [
            camera[0] - check[0],
            camera[1] - check[1],
            camera[2] - check[2],
        ];
        let inverse = 1.0 / (to[0] * to[0] + (to[1] * to[1] + to[2] * to[2])).sqrt();
        let step = to.map(|t| t * inverse * SECTION_DIAGONAL);
        let (min_y, max_y) = (
            f64::from(self.min_y),
            f64::from(self.min_y + self.sections * 16 - 1),
        );
        let far = |p: &[f64; 3]| {
            let (x, y, z) = (p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]);
            x * x + (y * y + z * z) > DISTANT_SQR
        };
        let mut sample = 0u8;
        while sample + 1 < known {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample += 1;
        }
        while far(&check) {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample = sample.saturating_add(1);
            if check[1] > max_y || check[1] < min_y {
                break;
            }
            let section = (
                floor(check[0]) >> 4,
                (floor(check[1]) - self.min_y) >> 4,
                floor(check[2]) >> 4,
            );
            if !self.contains(section) {
                // A count too long to keep (`SEES`) starts over next time.
                return Some(if sample == SEES { 0 } else { sample });
            }
        }
        None
    }
}

/// `Mth.floor` for coordinates inside the world, without a libm call (the
/// x86-64 baseline has no rounding instruction).
fn floor(x: f64) -> i32 {
    let i = x as i32;
    i - i32::from(f64::from(i) > x)
}

struct Job {
    edits: Vec<Edit>,
    camera: DVec3,
    radius: i32,
    /// Buffers to reuse.
    graph: Graph,
}

/// The thread that runs full searches over its own copy of the columns.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Graph, Vec<Edit>)>,
}

impl Worker {
    fn spawn(min_y: i32, sections: i32) -> Option<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("occlusion".into())
            .spawn(move || {
                let mut columns = Columns::new(min_y, sections);
                let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
                while let Ok(mut job) = job_rx.recv() {
                    for edit in job.edits.drain(..) {
                        columns.apply(edit);
                    }
                    job.graph
                        .search(&columns, job.camera, job.radius, &mut vis, &mut queue);
                    if done_tx.send((job.graph, job.edits)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

pub struct Occlusion {
    /// The render thread's copy, for partial updates (and searches when the
    /// worker is gone).
    columns: Columns,
    /// Edits the worker has not been sent yet.
    edits: Vec<Edit>,
    /// A full search is due: sections changed since the last one started.
    dirty: bool,
    /// The last finished search; `None` until one finishes, or after the
    /// columns were cleared.
    graph: Option<Graph>,
    spare: Graph,
    worker: Option<Worker>,
    /// The camera section and radius of the running search.
    running: Option<(SectionKey, i32)>,
    /// The running search started before the columns were cleared: drop it.
    discard: bool,
    /// The camera section and radius `graph`'s search started from (or is
    /// starting from).
    searched: Option<(SectionKey, i32)>,
    /// Sections changed since `graph`'s search started, to propagate from.
    changed: Vec<SectionKey>,
    /// Sections changed since the running search started: they propagate into
    /// its graph when it lands.
    changed_late: Vec<SectionKey>,
    vis: Vec<u64>,
    queue: VecDeque<u32>,
}

impl Occlusion {
    /// The world's lowest block y and its height in sections.
    pub fn new(min_y: i32, sections: i32) -> Self {
        Self {
            columns: Columns::new(min_y, sections),
            edits: Vec::new(),
            dirty: true,
            graph: None,
            spare: Graph::default(),
            worker: Worker::spawn(min_y, sections),
            running: None,
            discard: false,
            searched: None,
            changed: Vec::new(),
            changed_late: Vec::new(),
            vis: Vec::new(),
            queue: VecDeque::new(),
        }
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if self.columns.set(key, vis) {
            self.changed(key, vis);
        }
    }
    fn changed(&mut self, key: SectionKey, vis: u64) {
        self.edits.push(Edit::Set(key, vis));
        self.dirty = true;
        self.changed.push(key);
        if self.running.is_some() {
            self.changed_late.push(key);
        }
    }
    /// Every section of the column `x`, `z` goes (sees through every face).
    pub fn remove_column(&mut self, x: i32, z: i32) {
        let Some(column) = self.columns.map.remove(&(x, z)) else {
            return;
        };
        for (y, &vis) in column.iter().enumerate() {
            if vis != ALL_VISIBLE {
                self.changed((x, y as i32, z), ALL_VISIBLE);
            }
        }
    }
    pub fn clear(&mut self) {
        self.columns.map.clear();
        self.edits.clear();
        self.edits.push(Edit::Clear);
        self.dirty = true;
        // Another world: the old graph says nothing about it.
        if let Some(graph) = self.graph.take() {
            self.spare = graph;
        }
        self.searched = None;
        self.discard = self.running.is_some();
        self.changed.clear();
        self.changed_late.clear();
    }
    /// Whether `key` is visible from the camera. Without a usable graph
    /// (before the first search lands, or with the camera somewhere the graph
    /// never reached) every section is.
    pub fn contains(&self, key: SectionKey) -> bool {
        self.graph.as_ref().is_none_or(|g| g.contains(key))
    }
    /// Take a finished search, propagate changed sections into the graph,
    /// and start a search when sections changed or the camera moved to
    /// another section. Never waits for the worker.
    pub fn update(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.collect(false);
        self.propagate_changes(camera);
        if self.running.is_none() && (self.dirty || self.searched != Some((from, radius))) {
            self.start(camera, radius);
        }
        // A graph that never reached the camera's section hides everything
        // around it: until the new one lands, cull by the frustum alone.
        if self
            .graph
            .as_ref()
            .is_some_and(|g| g.from != from && !g.contains(from))
            && let Some(graph) = self.graph.take()
        {
            self.spare = graph;
        }
    }
    fn start(&mut self, camera: DVec3, radius: i32) {
        let from = self.columns.section_of(camera);
        self.dirty = false;
        self.searched = Some((from, radius));
        let graph = std::mem::take(&mut self.spare);
        if let Some(worker) = &self.worker {
            let job = Job {
                edits: std::mem::take(&mut self.edits),
                camera,
                radius,
                graph,
            };
            if worker.jobs.send(job).is_ok() {
                self.running = Some((from, radius));
                self.changed_late.clear();
                return;
            }
            log::warn!("Horizon: the occlusion worker stopped; culling on the render thread");
            self.worker = None;
        }
        self.edits.clear();
        let mut graph = Graph::default();
        graph.search(
            &self.columns,
            camera,
            radius,
            &mut self.vis,
            &mut self.queue,
        );
        self.land(graph);
    }
    fn collect(&mut self, wait: bool) {
        let Some(worker) = &self.worker else { return };
        if self.running.is_none() {
            return;
        }
        let result = if wait {
            worker.done.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            worker.done.try_recv()
        };
        match result {
            Ok((graph, edits)) => {
                self.running = None;
                if self.edits.is_empty() {
                    self.edits = edits;
                }
                if std::mem::take(&mut self.discard) {
                    self.spare = graph;
                    return;
                }
                self.land(graph);
                self.changed.clear();
                self.changed.append(&mut self.changed_late);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The edits it held are lost with it: search here from now on.
                self.worker = None;
                self.running = None;
                self.discard = false;
                self.dirty = true;
            }
        }
    }
    fn land(&mut self, graph: Graph) {
        if let Some(old) = self.graph.replace(graph) {
            self.spare = old;
        }
    }
    /// Vanilla's `runPartialUpdate`: what changed sections now connect to.
    fn propagate_changes(&mut self, camera: DVec3) {
        let Some(graph) = &mut self.graph else {
            self.changed.clear();
            return;
        };
        self.queue.clear();
        for key in self.changed.drain(..) {
            if let Some(i) = graph.index(key)
                && graph.directions[i] & REACHED != 0
            {
                self.queue.push_back(i as u32);
            }
        }
        let columns = &self.columns;
        graph.propagate(&mut self.queue, camera, false, |_, key| columns.get(key));
    }
    /// Wait for the running search: tests, and A/B timing of the old
    /// blocking search.
    #[cfg(test)]
    fn settle(&mut self, camera: DVec3, radius: i32) {
        self.update(camera, radius);
        self.collect(true);
        self.propagate_changes(camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The centre of a section, in a world whose section 0 starts at y 0.
    fn at(key: SectionKey) -> DVec3 {
        DVec3::new(
            f64::from(key.0) * 16.0 + 8.0,
            f64::from(key.1) * 16.0 + 8.0,
            f64::from(key.2) * 16.0 + 8.0,
        )
    }

    #[test]
    fn constants_are_vanillas() {
        assert_eq!(SECTION_DIAGONAL, (3f64.sqrt() * 16.0).ceil());
        assert_eq!(DISTANT_SECTIONS, 3);
    }

    #[test]
    fn floor_is_std_floor() {
        let mut x = -40_000.0_f64;
        while x < 40_000.0 {
            for v in [x, x + 0.5, x - 1e-9, x + 1e-9, -x] {
                assert_eq!(floor(v), v.floor() as i32, "{v}");
            }
            x += 0.37;
        }
    }

    #[test]
    fn open_space_reaches_the_whole_box() {
        let mut o = Occlusion::new(0, 4);
        o.settle(at((0, 1, 0)), 2);
        for x in -2..=2 {
            for z in -2..=2 {
                for y in -1..=4 {
                    assert!(o.contains((x, y, z)), "{x} {y} {z}");
                }
            }
        }
        assert!(!o.contains((3, 1, 0)));
    }

    #[test]
    fn closed_neighbours_hide_what_is_behind_them() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        assert!(o.contains((1, 1, 0)), "a neighbour is seen");
        assert!(!o.contains((2, 1, 0)), "nothing shows through it");
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle(at((0, 1, 0)), 3);
        assert!(o.contains((2, 1, 0)));
    }

    /// Faces `a` and `b` of a section see each other, nothing else does.
    fn connects(a: usize, b: usize) -> u64 {
        1 << (a * 6 + b) | 1 << (b * 6 + a)
    }

    /// #629 / #782 rule 1: a section first entered through a face that leads
    /// nowhere still leads on through the face it is entered through later.
    #[test]
    fn a_section_leads_on_through_every_face_it_is_entered_through() {
        let mut o = Occlusion::new(0, 4);
        // (1, 1, 1) is reached through its west face first (from (0, 1, 1),
        // queued before (1, 1, 0)), then through its north face. Only north
        // leads on, to the east.
        o.set((1, 1, 1), connects(2, 5));
        // Every other way into (2, 1, 1) is closed.
        for key in [(2, 1, 0), (2, 0, 1), (2, 2, 1)] {
            o.set(key, 0);
        }
        o.settle(at((0, 1, 0)), 3);
        let graph = o.graph.as_ref().expect("searched");
        let i = graph.index((1, 1, 1)).unwrap();
        assert_eq!(
            graph.entered[i],
            1 << 2 | 1 << 4,
            "entered from north and west"
        );
        assert!(o.contains((2, 1, 1)), "seen through the north face");
    }

    #[test]
    fn frames_never_wait_and_partial_updates_open_at_once() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 3);
        assert!(!o.contains((2, 1, 0)));
        // Opening the wall shows what is behind it in the same frame, from the
        // graph the frame has, whether or not the full search has finished.
        o.set((1, 1, 0), ALL_VISIBLE);
        o.update(camera, 3);
        assert!(o.contains((2, 1, 0)));
        assert!(o.contains((3, 1, 0)));
        // Closing it again lasts until the next full search lands.
        o.set((1, 1, 0), 0);
        o.settle(camera, 3);
        o.settle(camera, 3);
        assert!(!o.contains((2, 1, 0)));
    }

    #[test]
    fn a_graph_that_lost_the_camera_culls_nothing() {
        let mut o = Occlusion::new(0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3);
        assert!(!o.contains((3, 1, 0)));
        // The camera jumped past the wall; the next search is still running.
        let o = &mut o;
        o.update(at((3, 1, 0)), 3);
        if o.running.is_some() {
            assert!(o.contains((3, 1, 0)));
        }
        o.settle(at((3, 1, 0)), 3);
        assert!(o.contains((3, 1, 0)));
        assert!(
            !o.contains((0, 1, 0)),
            "the old camera section is walled in"
        );
    }

    /// #1037: from far above or below the world, the search starts from the
    /// world's top or bottom layer (`initializeQueueForFullUpdate`), so the
    /// ground stays drawn at any height.
    #[test]
    fn a_camera_outside_the_world_sees_it_from_its_top_or_bottom() {
        for from in [(0, 6, 0), (0, 9, 0), (2, 40, -1), (0, -1, 0), (1, -7, 0)] {
            let mut o = Occlusion::new(0, 6);
            o.settle(at(from), 3);
            for x in -1..=5 {
                for z in -4..=2 {
                    for y in 0..6 {
                        let inside = (x - from.0).abs() <= 3 && (z - from.2).abs() <= 3;
                        assert_eq!(o.contains((x, y, z)), inside, "{from:?}: {x} {y} {z}");
                    }
                }
            }
            assert!(
                !o.contains(from),
                "{from:?}: the camera's own section is not in the world"
            );
        }
    }

    /// Seeds enter through the outer face and never turn back up (down)
    /// or towards the camera's column.
    #[test]
    fn a_closed_top_layer_hides_the_world_from_above() {
        let mut o = Occlusion::new(0, 6);
        for x in -3..=3 {
            for z in -3..=3 {
                o.set((x, 5, z), 0);
            }
        }
        o.settle(at((0, 30, 0)), 3);
        assert!(
            o.contains((0, 5, 0)) && o.contains((3, 5, -3)),
            "the top is seen"
        );
        assert!(
            !o.contains((0, 4, 0)) && !o.contains((2, 1, 2)),
            "nothing under it"
        );
        assert!(!o.contains((0, 6, 0)), "seeds don't step back up");
        // From below, the open bottom layer shows everything up to the lid.
        o.settle(at((0, -9, 0)), 3);
        assert!(o.contains((0, 0, 0)) && o.contains((1, 4, 1)) && o.contains((1, 5, 1)));
        assert!(!o.contains((0, -1, 0)), "seeds don't step back down");
    }

    /// Vanilla's `runUpdates` as written (a map of nodes, a FIFO queue,
    /// `Vector3d` maths), over the same box: whether each cell is reached.
    /// `rays: false` leaves out the distant step test.
    fn reference(columns: &Columns, camera: DVec3, radius: i32, rays: bool) -> Vec<bool> {
        struct Node {
            directions: u8,
            sources: u8,
        }
        let from = columns.section_of(camera);
        let y0 = -1;
        let height = columns.sections + 2;
        let side = 2 * radius + 1;
        let index = |k: SectionKey| {
            let (dx, dz, dy) = (k.0 - from.0, k.2 - from.2, k.1 - y0);
            (dx.abs() <= radius && dz.abs() <= radius && (0..height).contains(&dy))
                .then(|| (((dz + radius) * side + dx + radius) * height + dy) as usize)
        };
        let block = |k: SectionKey| [k.0 * 16, k.1 * 16 + columns.min_y, k.2 * 16];
        // Direction.getAxis in x, y, z order.
        let axis = |d: usize| [1, 2, 0][d / 2];
        let centre = block(from).map(|c| c + 8);
        let mut nodes: HashMap<SectionKey, Node> = HashMap::new();
        let mut queue = VecDeque::new();
        if (0..columns.sections).contains(&from.1) {
            nodes.insert(
                from,
                Node {
                    directions: 0,
                    sources: 0,
                },
            );
            queue.push_back(from);
        } else {
            // initializeQueueForFullUpdate from outside the world.
            let above = from.1 >= columns.sections;
            let (y, travel) = if above {
                (columns.sections - 1, 0)
            } else {
                (0, 1)
            };
            let camera_block = [camera.x.floor() as i64, camera.z.floor() as i64];
            let mut seeds = Vec::new();
            for x in -radius..=radius {
                for z in -radius..=radius {
                    let key = (from.0 + x, y, from.2 + z);
                    let mut directions = 1 << travel;
                    if x > 0 {
                        directions |= 1 << 5;
                    } else if x < 0 {
                        directions |= 1 << 4;
                    }
                    if z > 0 {
                        directions |= 1 << 3;
                    } else if z < 0 {
                        directions |= 1 << 2;
                    }
                    let origin = block(key);
                    let dx = camera_block[0] - i64::from(origin[0] + 8);
                    let dz = camera_block[1] - i64::from(origin[2] + 8);
                    seeds.push((dx * dx + dz * dz, key, directions));
                }
            }
            seeds.sort_by_key(|seed| seed.0);
            for (_, key, directions) in seeds {
                nodes.insert(
                    key,
                    Node {
                        directions,
                        sources: 1 << travel,
                    },
                );
                queue.push_back(key);
            }
        }
        while let Some(key) = queue.pop_front() {
            let node = &nodes[&key];
            let (directions, sources) = (node.directions, node.sources);
            let vis = columns.get(key);
            let distant = (key.0 - from.0).abs() > 3
                || (key.1 - from.1).abs() > 3
                || (key.2 - from.2).abs() > 3;
            for (d, step) in STEPS.iter().enumerate() {
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if next.1 < -1 || next.1 > columns.sections || index(next).is_none() {
                    continue;
                }
                if directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                // facesCanSeeEachother(source.getOpposite(), direction).
                if sources != 0
                    && !(0..6)
                        .any(|s| sources & (1 << s) != 0 && vis & (1 << ((s ^ 1) * 6 + d)) != 0)
                {
                    continue;
                }
                if rays && distant {
                    let origin = block(key);
                    let max = |c: usize| {
                        if c == axis(d) {
                            centre[c] > origin[c]
                        } else {
                            centre[c] < origin[c]
                        }
                    };
                    let mut check = DVec3::new(
                        f64::from(origin[0] + if max(0) { 16 } else { 0 }),
                        f64::from(origin[1] + if max(1) { 16 } else { 0 }),
                        f64::from(origin[2] + if max(2) { 16 } else { 0 }),
                    );
                    let length = |v: DVec3| (v.x * v.x + (v.y * v.y + v.z * v.z)).sqrt();
                    let to = camera - check;
                    let step = to * (1.0 / length(to)) * 28.0;
                    let mut visible = true;
                    while length(check - camera).powi(2) > 3600.0 {
                        check += step;
                        let max_y = columns.min_y + columns.sections * 16 - 1;
                        if check.y > f64::from(max_y) || check.y < f64::from(columns.min_y) {
                            break;
                        }
                        let section = columns.section_of(check);
                        if !nodes.contains_key(&section) {
                            visible = false;
                            break;
                        }
                    }
                    if !visible {
                        continue;
                    }
                }
                match nodes.get_mut(&next) {
                    Some(existing) => existing.sources |= 1 << d,
                    None => {
                        nodes.insert(
                            next,
                            Node {
                                directions: directions | 1 << d,
                                sources: 1 << d,
                            },
                        );
                        queue.push_back(next);
                    }
                }
            }
        }
        let mut reached = vec![false; (side * side * height) as usize];
        for key in nodes.keys() {
            reached[index(*key).unwrap()] = true;
        }
        reached
    }

    fn random_world(o: &mut Occlusion, seed: u64, count: usize, spread: i32) {
        let mut seed = seed;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let span = (2 * spread + 1) as u64;
        for _ in 0..count {
            let r = next();
            let key = (
                (r % span) as i32 - spread,
                ((r >> 8) % 6) as i32,
                ((r >> 16) % span) as i32 - spread,
            );
            // Open, half open or nearly closed sections.
            let vis = match r >> 40 & 3 {
                0 => next() & next() & next(),
                1 => next(),
                _ => next() | next(),
            };
            o.set(key, vis & ALL_VISIBLE);
        }
    }

    fn reached(o: &Occlusion) -> Vec<bool> {
        let graph = o.graph.as_ref().expect("searched");
        graph.directions.iter().map(|&d| d & REACHED != 0).collect()
    }

    /// The graph is vanilla's, cell for cell, near the camera and beyond the
    /// distant step test's 60 blocks.
    #[test]
    fn same_sections_as_vanilla() {
        let (mut hidden, mut rayed) = (0, 0);
        for seed in [0x9e37_79b9, 7, 0xdead_beef] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 6);
                random_world(&mut o, seed, count, spread);
                for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4), (1, -1, 1), (0, 7, 0)] {
                    // Off-centre, so the rays are not axis-aligned.
                    let camera = at(from) + DVec3::new(3.3, -5.7, 1.9);
                    o.settle(camera, radius);
                    o.settle(camera, radius);
                    let reached = reached(&o);
                    assert_eq!(
                        reached,
                        reference(&o.columns, camera, radius, true),
                        "{seed} {from:?} {radius}"
                    );
                    let no_rays = reference(&o.columns, camera, radius, false);
                    rayed += no_rays
                        .iter()
                        .zip(&reached)
                        .filter(|(a, b)| **a && !**b)
                        .count();
                    hidden += reached.iter().filter(|&&r| !r).count();
                }
            }
        }
        assert!(hidden > 1000, "the worlds hide sections ({hidden})");
        assert!(
            rayed > 100,
            "the distant step test culls sections ({rayed})"
        );
    }

    /// A clamped search may conservatively show sections above its searched
    /// box, but it must never hide one that the full-height search reaches.
    #[test]
    fn vertical_clamping_only_adds_overdraw() {
        let mut compared = 0;
        let mut overdraw = 0;
        let mut clamped = 0;
        for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 24);
                random_world(&mut o, seed, count, spread);
                let camera = at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9);
                o.settle(camera, radius);
                let graph = o.graph.as_ref().expect("searched");
                clamped += usize::from(graph.height < o.columns.sections + 2);
                let full = reference(&o.columns, camera, radius, true);
                let from = o.columns.section_of(camera);
                let side = 2 * radius + 1;
                let height = o.columns.sections + 2;
                let full_contains = |key: SectionKey| {
                    let (dx, dz, dy) = (key.0 - from.0, key.2 - from.2, key.1 + 1);
                    if dx.abs() > radius || dz.abs() > radius || !(0..height).contains(&dy) {
                        return false;
                    }
                    full[(((dz + radius) * side + dx + radius) * height + dy) as usize]
                };
                for x in from.0 - radius..=from.0 + radius {
                    for z in from.2 - radius..=from.2 + radius {
                        for y in -1..=o.columns.sections {
                            let key = (x, y, z);
                            let expected = full_contains(key);
                            let actual = o.contains(key);
                            compared += 1;
                            assert!(
                                !expected || actual,
                                "clamped search hid {key:?} for seed {seed:#x}, spread {spread}, radius {radius}; from={from:?}, y0={}, height={}, expected={expected}, actual={actual}",
                                graph.y0,
                                graph.height
                            );
                            overdraw += usize::from(actual && !expected);
                            if actual && !expected {
                                assert!(
                                    y >= graph.y0 + graph.height,
                                    "overdraw must be above the clamped box: {key:?}, y0={}, height={}",
                                    graph.y0,
                                    graph.height
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(clamped > 0, "the comparison exercised a clamped search");
        assert!(compared > 50_000, "compared {compared} sections");
        assert!(overdraw > 0, "the conservative top-cell rule was exercised");
    }

    /// A line checked again from the sample it failed at, after more
    /// sections were reached, ends where checking it from its corner does.
    #[test]
    fn a_line_checked_again_resumes_where_it_failed() {
        let mut seed = 0x2545_f491_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let columns = Columns::new(-64, 24);
        let mut graph = Graph::default();
        let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
        let (mut resumed, mut seen) = (0, 0);
        for _ in 0..40 {
            let camera = DVec3::new(
                (next() % 3000) as f64 / 100.0 - 15.0,
                (next() % 30000) as f64 / 100.0 - 60.0,
                (next() % 3000) as f64 / 100.0 - 15.0,
            );
            // Open air: every section reached; then holes along the lines.
            graph.search(&columns, camera, 12, &mut vis, &mut queue);
            for cell in graph.directions.iter_mut() {
                if next() % 4 == 0 {
                    *cell = 0;
                }
            }
            let from = graph.from;
            for _ in 0..25 {
                let corner = (
                    from.0 + (next() % 25) as i32 - 12,
                    graph.y0 + (next() % graph.height as u64) as i32,
                    from.2 + (next() % 25) as i32 - 12,
                );
                let mut known = 0;
                while let Some(failed) = graph.walk(corner, camera, known) {
                    for earlier in 0..=failed {
                        assert_eq!(graph.walk(corner, camera, earlier), Some(failed));
                    }
                    known = failed;
                    resumed += 1;
                    // More sections reached, as the search goes on.
                    for cell in graph.directions.iter_mut() {
                        if next() % 3 == 0 {
                            *cell |= REACHED;
                        }
                    }
                    assert_eq!(
                        graph.walk(corner, camera, known),
                        graph.walk(corner, camera, 0)
                    );
                }
                seen += usize::from(known > 1);
            }
        }
        assert!(resumed > 200 && seen > 50, "{resumed} {seen}");
    }

    /// #782 rule 2: behind a wall, a section the search reaches around its
    /// edge is culled when the line to the camera crosses the wall's shadow.
    #[test]
    fn distant_sections_behind_a_wall_are_culled() {
        let mut o = Occlusion::new(0, 4);
        // A closed wall at x 2, z -2..=2, the whole height.
        for z in -2..=2 {
            for y in -1..=4 {
                o.set((2, y, z), 0);
            }
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 12);
        let no_rays = reference(&o.columns, camera, 12, false);
        let graph = o.graph.as_ref().expect("searched");
        let behind = graph.index((9, 1, 3)).unwrap();
        assert!(no_rays[behind], "reached around the wall's edge");
        assert!(
            !o.contains((9, 1, 3)),
            "but its line to the camera is hidden"
        );
        assert!(o.contains((9, 1, 9)), "a section off to the side is seen");
    }

    /// `cargo test --profile fast -p horizon-client --lib occlusion::tests::search_cost -- --ignored --nocapture`:
    /// full searches at render distance 32 over a world with open sky above
    /// y 64 and caves below it.
    #[test]
    #[ignore]
    fn search_cost() {
        let mut o = Occlusion::new(-64, 24);
        let mut seed = 0x9e37_79b9_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for x in -40..=40 {
            for z in -40..=40 {
                for y in 0..24 {
                    let vis = if y >= 8 {
                        ALL_VISIBLE
                    } else if next() % 5 == 0 {
                        next() & ALL_VISIBLE
                    } else {
                        0
                    };
                    o.set((x, y, z), vis);
                }
            }
        }
        let camera = DVec3::new(3.3, 70.2, -5.6);
        let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
        let mut graph = Graph::default();
        let mut best = f64::MAX;
        for _ in 0..12 {
            let started = std::time::Instant::now();
            graph.search(&o.columns, camera, 34, &mut vis, &mut queue);
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        }
        let reached = graph
            .directions
            .iter()
            .filter(|&&d| d & REACHED != 0)
            .count();
        println!("search_cost: best {best:.2} ms over 12, {reached} reached");
    }

    /// The worker's graph is the one the render thread would compute.
    #[test]
    fn the_worker_matches_a_search_on_the_render_thread() {
        let mut o = Occlusion::new(0, 6);
        random_world(&mut o, 0x9e37_79b9, 800, 6);
        for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4)] {
            let camera = at(from);
            o.settle(camera, 6);
            o.settle(camera, 6);
            let mut local = Graph::default();
            local.search(&o.columns, camera, 6, &mut Vec::new(), &mut VecDeque::new());
            let graph = o.graph.as_ref().expect("searched");
            assert_eq!(graph.directions, local.directions);
            assert_eq!(graph.entered, local.entered);
        }
    }

    #[test]
    fn vertical_clamping_bounds_height_and_reaches_open_sky() {
        let mut o = Occlusion::new(0, 24);
        o.set((0, 5, 0), 0); // Wall at section 5
        o.settle(at((0, 2, 0)), 4);
        let graph = o.graph.as_ref().expect("searched");
        assert_eq!(graph.height, 5 + 3);
        assert!(o.contains((0, 2, 0)), "camera section is reached");
        assert!(o.contains((1, 20, 1)), "open air column is reached");

        // An enclosed cave cannot reach the sky
        let mut cave = Occlusion::new(0, 24);
        for (dx, dy, dz) in STEPS {
            cave.set((dx, 1 + dy, dz), 0);
        }
        cave.settle(at((0, 1, 0)), 4);
        assert!(
            !cave.contains((0, 10, 0)),
            "enclosed cave does not see the sky"
        );
    }
}
