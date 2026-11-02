//! Entity occlusion affects mesh submission, never entity ticking or interaction.
use super::{Game, Vertex};
use horizon_client::{
    blocks::{BlockFacts, RenderShape, StateFacts},
    chunk::Chunk,
    entity_culling::Culler,
    world::World,
};
use std::{
    cell::RefCell,
    sync::{
        Arc, Weak,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::JoinHandle,
};

type ChunkKey = (i32, i32);
type SectionKey = (i32, i32, i32);
const MAX_SNAPSHOT_CHUNKS: i64 = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CullKey {
    camera: [u64; 3],
    min: [u64; 3],
    max: [u64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnapshotKey {
    world_revision: u64,
    chunks_revision: u64,
    bottom_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChunkBounds {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

/// Sorted weak chunk references avoid holding gameplay chunks or forcing
/// copy-on-write. A worker keeps only its current chunk strongly referenced.
#[derive(Clone)]
struct OcclusionSnapshot {
    chunks: Vec<(ChunkKey, Weak<Chunk>)>,
    opaque_states: Arc<[bool]>,
}

impl OcclusionSnapshot {
    fn chunk(&self, key: ChunkKey) -> Option<&Weak<Chunk>> {
        self.chunks
            .binary_search_by_key(&key, |(key, _)| *key)
            .ok()
            .map(|index| &self.chunks[index].1)
    }

    fn refresh_chunks(&mut self, world: &World, bounds: ChunkBounds) {
        self.chunks.clear();
        for x in bounds.min_x..=bounds.max_x {
            for z in bounds.min_z..=bounds.max_z {
                if let Some(chunk) = world.chunks.get(&(x, z)) {
                    self.chunks.push(((x, z), Arc::downgrade(chunk)));
                }
            }
        }
    }

    fn chunks_match(&self, world: &World) -> bool {
        self.chunks.iter().all(|(key, expected)| {
            world
                .chunks
                .get(key)
                .is_some_and(|current| std::ptr::eq(expected.as_ptr(), Arc::as_ptr(current)))
        })
    }

    fn opaque(&self, pos: [i32; 3], last_chunk: &mut LastChunk) -> bool {
        let key = (pos[0] >> 4, pos[2] >> 4);
        let Some(chunk) = last_chunk.get(self, key) else {
            return false;
        };
        let state = chunk.block(pos[0] & 15, pos[1], pos[2] & 15);
        self.opaque_states
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
}

#[derive(Default)]
struct LastChunk {
    key: Option<ChunkKey>,
    value: Option<Arc<Chunk>>,
}

impl LastChunk {
    fn get(&mut self, snapshot: &OcclusionSnapshot, key: ChunkKey) -> Option<&Arc<Chunk>> {
        if self.key != Some(key) {
            self.key = Some(key);
            self.value = snapshot.chunk(key).and_then(Weak::upgrade);
        }
        self.value.as_ref()
    }
}

struct OcclusionProof {
    sections: Vec<SectionKey>,
    chunks: Vec<(ChunkKey, Weak<Chunk>)>,
}

struct CullResults {
    stamp: SnapshotKey,
    hidden: Vec<(CullKey, OcclusionProof)>,
    scratch_queries: Vec<CullKey>,
}

struct CullJob {
    stamp: SnapshotKey,
    snapshot: Arc<OcclusionSnapshot>,
    queries: Vec<CullKey>,
}

/// Non-blocking entity occlusion. Results are reused only for byte-identical
/// camera and mesh bounds, with the block chunks and rendered sections that
/// proved occlusion revalidated before hiding anything.
pub(super) struct AsyncCuller {
    jobs: SyncSender<CullJob>,
    results_rx: Receiver<CullResults>,
    _thread: JoinHandle<()>,
    snapshot: Option<(SnapshotKey, ChunkBounds, Arc<OcclusionSnapshot>)>,
    opaque_states: Option<Arc<[bool]>>,
    results: Option<CullResults>,
    queries: RefCell<Vec<CullKey>>,
    camera: [f64; 3],
    stamp: SnapshotKey,
    pending: bool,
}

impl AsyncCuller {
    pub(super) fn new() -> std::io::Result<Self> {
        let (job_tx, job_rx) = mpsc::sync_channel::<CullJob>(1);
        let (result_tx, result_rx) = mpsc::channel::<CullResults>();
        let thread = std::thread::Builder::new()
            .name("entity-culling".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    if result_tx.send(cull_job(job)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            jobs: job_tx,
            results_rx: result_rx,
            _thread: thread,
            snapshot: None,
            opaque_states: None,
            results: None,
            queries: RefCell::new(Vec::new()),
            camera: [0.0; 3],
            stamp: SnapshotKey {
                world_revision: 0,
                chunks_revision: 0,
                bottom_y: 0,
            },
            pending: false,
        })
    }

    fn begin_frame(&mut self, camera: glam::DVec3, world: &World) {
        while let Ok(mut results) = self.results_rx.try_recv() {
            self.pending = false;
            let scratch = std::mem::take(&mut results.scratch_queries);
            if scratch.capacity() > self.queries.get_mut().capacity() {
                *self.queries.get_mut() = scratch;
            }
            self.results = Some(results);
        }
        self.camera = camera.to_array();
        self.stamp = SnapshotKey {
            world_revision: world.revision,
            chunks_revision: world.chunks.membership_revision(),
            bottom_y: world.bottom_y(),
        };
        if self
            .results
            .as_ref()
            .is_some_and(|result| result.stamp != self.stamp)
        {
            self.results = None;
        }
        self.queries.get_mut().clear();
    }

    fn query(&self, vertices: &[Vertex], start: usize) -> Option<CullKey> {
        let (min, max) = Culler::bounds(self.camera, vertices, start)?;
        Some(CullKey {
            camera: self.camera.map(f64::to_bits),
            min: min.map(f64::to_bits),
            max: max.map(f64::to_bits),
        })
    }

    fn proof(&self, key: &CullKey) -> Option<&OcclusionProof> {
        let hidden = &self.results.as_ref()?.hidden;
        let index = hidden.binary_search_by_key(key, |(key, _)| *key).ok()?;
        Some(&hidden[index].1)
    }

    fn queue(&self, key: CullKey) {
        self.queries.borrow_mut().push(key);
    }

    fn finish_frame(&mut self, world: &World, facts: &BlockFacts) {
        let queries = self.queries.get_mut();
        if self.pending {
            queries.clear();
            return;
        }
        if queries.is_empty() {
            return;
        }
        queries.sort_unstable();
        queries.dedup();
        let Some(bounds) = snapshot_bounds(self.camera, queries) else {
            queries.clear();
            return;
        };
        let opaque_states = Arc::clone(
            self.opaque_states
                .get_or_insert_with(|| facts.states.iter().map(opaque).collect::<Vec<_>>().into()),
        );
        let snapshot = match &mut self.snapshot {
            Some((stamp, cached_bounds, snapshot)) => {
                if stamp != &self.stamp || *cached_bounds != bounds || !snapshot.chunks_match(world)
                {
                    let snapshot = Arc::make_mut(snapshot);
                    snapshot.opaque_states = Arc::clone(&opaque_states);
                    snapshot.refresh_chunks(world, bounds);
                    *stamp = self.stamp;
                    *cached_bounds = bounds;
                }
                Arc::clone(snapshot)
            }
            None => {
                let mut snapshot = OcclusionSnapshot {
                    chunks: Vec::new(),
                    opaque_states,
                };
                snapshot.refresh_chunks(world, bounds);
                let snapshot = Arc::new(snapshot);
                self.snapshot = Some((self.stamp, bounds, Arc::clone(&snapshot)));
                snapshot
            }
        };
        let queries = std::mem::take(queries);
        let job = CullJob {
            stamp: self.stamp,
            snapshot,
            queries,
        };
        match self.jobs.try_send(job) {
            Ok(()) => self.pending = true,
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                *self.queries.get_mut() = job.queries;
            }
        }
    }
}

fn cull_job(mut job: CullJob) -> CullResults {
    let camera = job.queries[0].camera.map(f64::from_bits);
    let culler = Culler::new(camera, 8_192);
    let mut last_chunk = LastChunk::default();
    let mut hidden = Vec::with_capacity(job.queries.len());
    let mut covered = Vec::new();
    for &key in &job.queries {
        covered.clear();
        let min = key.min.map(f64::from_bits);
        let max = key.max.map(f64::from_bits);
        if culler.occluded(min, max, |pos| {
            let opaque = job.snapshot.opaque(pos, &mut last_chunk);
            if opaque {
                covered.push(pos);
            }
            opaque
        }) {
            let mut sections = Vec::with_capacity(covered.len());
            let mut proof_chunks: Vec<(ChunkKey, Weak<Chunk>)> = Vec::new();
            let mut valid = true;
            for &pos in &covered {
                let chunk_key = (pos[0] >> 4, pos[2] >> 4);
                let Some(y) = pos[1].checked_sub(job.stamp.bottom_y) else {
                    valid = false;
                    break;
                };
                sections.push((chunk_key.0, y.div_euclid(16), chunk_key.1));
                if let Err(index) = proof_chunks.binary_search_by_key(&chunk_key, |(key, _)| *key) {
                    let Some(chunk) = job.snapshot.chunk(chunk_key) else {
                        valid = false;
                        break;
                    };
                    if chunk.strong_count() == 0 {
                        valid = false;
                        break;
                    }
                    proof_chunks.insert(index, (chunk_key, chunk.clone()));
                }
            }
            if valid {
                sections.sort_unstable();
                sections.dedup();
                hidden.push((
                    key,
                    OcclusionProof {
                        sections,
                        chunks: proof_chunks,
                    },
                ));
            }
        }
    }
    job.queries.clear();
    CullResults {
        stamp: job.stamp,
        hidden,
        scratch_queries: job.queries,
    }
}

fn opaque(facts: &StateFacts) -> bool {
    facts.render_shape == RenderShape::Model
        && facts.can_occlude
        && facts.solid_render
        && facts.full_collision
}

fn snapshot_bounds(camera: [f64; 3], queries: &[CullKey]) -> Option<ChunkBounds> {
    let mut min_x = camera[0];
    let mut max_x = camera[0];
    let mut min_z = camera[2];
    let mut max_z = camera[2];
    for query in queries {
        let min = query.min.map(f64::from_bits);
        let max = query.max.map(f64::from_bits);
        min_x = min_x.min(min[0]);
        max_x = max_x.max(max[0]);
        min_z = min_z.min(min[2]);
        max_z = max_z.max(max[2]);
    }
    // Culler::occluded expands projected cells by EPSILON. One block of slack
    // also covers the camera's first cell when it lies exactly on a boundary.
    let min_x = snapshot_cell(min_x - 1.0)? >> 4;
    let max_x = snapshot_cell(max_x + 1.0)? >> 4;
    let min_z = snapshot_cell(min_z - 1.0)? >> 4;
    let max_z = snapshot_cell(max_z + 1.0)? >> 4;
    let width = i64::from(max_x) - i64::from(min_x) + 1;
    let depth = i64::from(max_z) - i64::from(min_z) + 1;
    if width <= 0 || depth <= 0 || width.checked_mul(depth)? > MAX_SNAPSHOT_CHUNKS {
        return None;
    }
    Some(ChunkBounds {
        min_x,
        max_x,
        min_z,
        max_z,
    })
}

fn snapshot_cell(value: f64) -> Option<i32> {
    let cell = value.floor();
    (cell.is_finite() && cell >= f64::from(i32::MIN) && cell <= f64::from(i32::MAX))
        .then_some(cell as i32)
}

impl Game<'_> {
    pub(super) fn prepare_entity_culling(&mut self, camera: glam::DVec3) {
        // Spans tell the pack's passes which entities are hidden; without them, nothing changes.
        self.pack_shadow_casters = match (&self.renderer, &self.pack_ids) {
            (Some(r), Some(_)) => r.pack_shadow_casters(),
            // A native pack's shadow pass has no spans to hide entities with: it only needs
            // the first-person model.
            (Some(r), None) => (false, r.native_shadow()),
            _ => (false, false),
        };
        let at = camera.to_array().map(|v| v.floor() as i32);
        let eligible = self.settings.horizon.entity_culling
            != horizon_client::graphics::EntityCullingMode::Off
            // Packs can replace an opaque block's model or texture.
            && self.model_packs_are_vanilla()
            && self.player.is_some()
            && !self.spectator()
            && !opaque(self.res.facts().get(self.world.block(at[0], at[1], at[2])));
        if !eligible {
            self.entity_culler = None;
            self.async_entity_culler = None;
            self.async_entity_culling_failed = false;
            return;
        }

        match self.settings.horizon.entity_culling {
            horizon_client::graphics::EntityCullingMode::Off => {
                self.entity_culler = None;
                self.async_entity_culler = None;
                self.async_entity_culling_failed = false;
            }
            horizon_client::graphics::EntityCullingMode::Synchronous => {
                self.entity_culler = Some(Culler::new(camera.to_array(), 8_192));
                self.async_entity_culler = None;
                self.async_entity_culling_failed = false;
            }
            horizon_client::graphics::EntityCullingMode::Asynchronous => {
                self.entity_culler = None;
                if self.async_entity_culler.is_none() && !self.async_entity_culling_failed {
                    match AsyncCuller::new() {
                        Ok(culler) => self.async_entity_culler = Some(culler),
                        Err(error) => {
                            log::warn!("Horizon: cannot start entity-culling worker: {error}");
                            self.async_entity_culling_failed = true;
                        }
                    }
                }
                if let Some(culler) = &mut self.async_entity_culler {
                    culler.begin_frame(camera, &self.world);
                } else {
                    // If the worker cannot start, use the existing same-frame path.
                    self.entity_culler = Some(Culler::new(camera.to_array(), 8_192));
                }
            }
        }
    }

    /// Use complete generated geometry so equipment, animation and custom
    /// models cannot stick out of an assumed collision-sized culling box.
    pub(super) fn cull_entity_vertices(&self, vertices: &mut Vec<Vertex>, start: usize) {
        if let Some(culler) = &self.entity_culler {
            let hidden = culler.hidden(vertices, start, |pos| self.occluder_ready(pos));
            // A shader pack's shadow pass still draws what the camera cannot see.
            if hidden
                && !(self.pack_shadow_casters.0 && self.hide_spans_from(start, vertices.len()))
            {
                vertices.truncate(start);
            }
            return;
        }

        let Some(culler) = &self.async_entity_culler else {
            return;
        };
        let Some(query) = culler.query(vertices, start) else {
            return;
        };
        let hidden = self
            .async_entity_culler
            .as_ref()
            .and_then(|culler| culler.proof(&query))
            .is_some_and(|proof| self.async_proof_is_current(proof));
        if hidden && !(self.pack_shadow_casters.0 && self.hide_spans_from(start, vertices.len())) {
            vertices.truncate(start);
        }
        if let Some(culler) = &self.async_entity_culler {
            culler.queue(query);
        }
    }

    fn async_proof_is_current(&self, proof: &OcclusionProof) -> bool {
        for (key, expected) in &proof.chunks {
            let Some(current) = self.world.chunks.get(key) else {
                return false;
            };
            if !std::ptr::eq(Arc::as_ptr(current), expected.as_ptr()) {
                return false;
            }
        }
        proof.sections.iter().all(|&section| {
            self.renderer
                .as_ref()
                .is_some_and(|r| r.has_section(section))
                && !self.world.dirty_sections.contains(&section)
                && !self.streaming.is_meshing(section)
                && !self.pending_uploads.iter().any(|(key, _)| *key == section)
        })
    }

    fn occluder_ready(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !opaque(self.res.facts().get(state)) {
            return false;
        }
        let Some(y) = pos[1].checked_sub(self.world.bottom_y()) else {
            return false;
        };
        let section = (pos[0] >> 4, y.div_euclid(16), pos[2] >> 4);
        self.renderer
            .as_ref()
            .is_some_and(|r| r.has_section(section))
            && !self.world.dirty_sections.contains(&section)
            && !self.streaming.is_meshing(section)
            && !self.pending_uploads.iter().any(|(key, _)| *key == section)
    }

    pub(super) fn finish_async_entity_culling(&mut self) {
        if let Some(culler) = &mut self.async_entity_culler {
            culler.finish_frame(&self.world, self.res.facts());
        }
    }
}

impl Game<'_> {
    /// Entity.setViewScale (LevelExtractor.extractVisibleEntities): the entity
    /// distance option times the render distance's boost — 1 at 8 chunks, 2.5
    /// at 20 or more. The render distance is the client setting, capped by
    /// the server's view distance when connected
    /// (Options.getEffectiveRenderDistance).
    pub(super) fn view_scale(&self) -> f64 {
        let distance = match &self.remote {
            Some(r) => self.settings.render_distance.min(r.view_distance),
            None => self.settings.render_distance,
        };
        entity_view_scale(distance, self.settings.entity_distance)
    }
}

/// Entity.setViewScale: `Mth.clamp(renderDistance / 8, 1, 2.5) *
/// entityDistanceScaling`. Every entity's draw distance is scaled by it, so
/// a small mob is drawn nearer than a big one and a longer render distance
/// draws both further away.
pub(super) fn entity_view_scale(render_distance: i32, entity_distance: f32) -> f64 {
    (f64::from(render_distance) / 8.0).clamp(1.0, 2.5) * f64::from(entity_distance)
}

/// Entity.shouldRenderAtSqrDistance's default range: 64 times the bounding
/// box's mean edge, scaled by the view scale.
pub(super) fn default_range(size: glam::DVec3, view_scale: f64) -> f64 {
    let edge = (size.x + size.y + size.z) / 3.0;
    let edge = if edge.is_nan() { 1.0 } else { edge };
    edge * 64.0 * view_scale
}

impl Game<'_> {
    /// EntityRenderer.shouldRender: the entity within `range` blocks of the
    /// camera, measured from its current `position`, and its culling box
    /// (`lo`..`hi`, interpolated, in world space) grown by 0.5 in the view.
    pub(super) fn entity_in_view(
        &self,
        camera: glam::DVec3,
        position: glam::DVec3,
        range: f64,
        lo: glam::DVec3,
        hi: glam::DVec3,
    ) -> bool {
        if (position - camera).length_squared() >= range * range {
            return false;
        }
        self.view_frustum.as_ref().is_none_or(|view| {
            view.intersects(
                (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
            )
        })
    }
}

#[cfg(test)]
mod async_tests {
    use super::*;
    use horizon_client::chunk::ColumnLayout;

    fn wall(hole: Option<[i32; 3]>) -> (Arc<OcclusionSnapshot>, Vec<Arc<Chunk>>) {
        let mut live = Vec::new();
        let mut chunks = Vec::new();
        for chunk_z in [-1, 0] {
            let mut chunk = Chunk::with_layout(0, chunk_z, ColumnLayout::VANILLA);
            for y in -1..=0 {
                for z in -1..=0 {
                    let pos = [4, y, z];
                    if hole != Some(pos) {
                        chunk.set(pos[0] & 15, pos[1], pos[2] & 15, 1);
                    }
                }
            }
            let chunk = Arc::new(chunk);
            chunks.push(((0, chunk_z), Arc::downgrade(&chunk)));
            live.push(chunk);
        }
        chunks.sort_unstable_by_key(|(key, _)| *key);
        (
            Arc::new(OcclusionSnapshot {
                chunks,
                opaque_states: Arc::from([false, true]),
            }),
            live,
        )
    }

    fn job(snapshot: Arc<OcclusionSnapshot>) -> CullJob {
        let camera = [0.0; 3];
        let min = [7.95, -0.55, -0.55];
        let max = [10.05, 0.55, 0.55];
        CullJob {
            stamp: SnapshotKey {
                world_revision: 1,
                chunks_revision: 1,
                bottom_y: ColumnLayout::VANILLA.min_y(),
            },
            snapshot,
            queries: vec![CullKey {
                camera: camera.map(f64::to_bits),
                min: min.map(f64::to_bits),
                max: max.map(f64::to_bits),
            }],
        }
    }

    #[test]
    fn async_snapshot_is_bounded_and_handles_negative_chunk_edges() {
        let camera = [-16.0, 0.0, -16.0];
        let key = CullKey {
            camera: camera.map(f64::to_bits),
            min: [-8.0, -1.0, -8.0].map(f64::to_bits),
            max: [8.0, 1.0, 8.0].map(f64::to_bits),
        };
        assert_eq!(
            snapshot_bounds(camera, &[key]),
            Some(ChunkBounds {
                min_x: -2,
                max_x: 0,
                min_z: -2,
                max_z: 0,
            })
        );

        let huge = CullKey {
            camera: [0.0; 3].map(f64::to_bits),
            min: [5_000.0, 0.0, 5_000.0].map(f64::to_bits),
            max: [5_001.0, 1.0, 5_001.0].map(f64::to_bits),
        };
        assert_eq!(snapshot_bounds([0.0; 3], &[huge]), None);
    }

    #[test]
    fn async_worker_only_hides_bounds_behind_a_complete_opaque_plane() {
        let (snapshot, live) = wall(None);
        let results = cull_job(job(snapshot));
        assert_eq!(results.hidden.len(), 1);
        assert_eq!(results.hidden[0].1.sections.len(), 4);
        assert_eq!(results.hidden[0].1.chunks.len(), 2);
        drop(live);

        let (snapshot, live) = wall(Some([4, 0, 0]));
        let results = cull_job(job(snapshot));
        assert!(results.hidden.is_empty());
        drop(live);
    }
}

/// An entity's position as a `DVec3`.
pub(super) fn at(p: horizon_core::geometry::Vec3) -> glam::DVec3 {
    glam::DVec3::new(p.x, p.y, p.z)
}

impl Game<'_> {
    /// `entity_in_view` for an entity whose box stands on its position,
    /// `width` wide and `height` tall, interpolated by `alpha` and grown by
    /// `grow` (a renderer's getBoundingBoxForCulling).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn standing_entity_in_view(
        &self,
        camera: glam::DVec3,
        previous: glam::DVec3,
        position: glam::DVec3,
        alpha: f64,
        width: f64,
        height: f64,
        range: f64,
        grow: f64,
    ) -> bool {
        let p = previous.lerp(position, alpha);
        let half = glam::DVec3::new(width / 2.0 + grow, grow, width / 2.0 + grow);
        let lo = p - half;
        let hi = p + glam::DVec3::new(half.x, height + grow, half.z);
        self.entity_in_view(camera, position, range, lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Entity.setViewScale`: the entity distance option times the render
    /// distance's boost — 1 at eight chunks, 1.5 at twelve, 2.5 at twenty.
    #[test]
    fn the_view_scale_follows_the_render_distance() {
        assert_eq!(entity_view_scale(4, 1.0), 1.0);
        assert_eq!(entity_view_scale(8, 1.0), 1.0);
        assert_eq!(entity_view_scale(12, 1.0), 1.5);
        assert_eq!(entity_view_scale(20, 1.0), 2.5);
        assert_eq!(entity_view_scale(32, 1.0), 2.5);
        // The entity distance option scales it down.
        assert_eq!(entity_view_scale(12, 0.5), 0.75);
    }

    /// `Entity.shouldRenderAtSqrDistance`: a chicken's box (0.4 by 0.7 by
    /// 0.4) is drawn up to 48 blocks away at the vanilla defaults, not the
    /// 32 blocks a missing view scale leaves (#2273).
    #[test]
    fn a_chicken_is_drawn_forty_eight_blocks_away() {
        let range = default_range(glam::DVec3::new(0.4, 0.7, 0.4), entity_view_scale(12, 1.0));
        assert_eq!(range, 48.0);
    }

    /// `Entity.shouldRenderAtSqrDistance` for a dropped item (`ItemEntity`
    /// keeps the default and its box is 0.25 on each side): 24 blocks at
    /// render distance 12, 32 at 16, 40 from 20 on; never 64.
    #[test]
    fn a_dropped_item_is_drawn_sixteen_blocks_times_the_view_scale() {
        let item = glam::DVec3::splat(0.25);
        assert_eq!(default_range(item, entity_view_scale(12, 1.0)), 24.0);
        assert_eq!(default_range(item, entity_view_scale(16, 1.0)), 32.0);
        assert_eq!(default_range(item, entity_view_scale(32, 1.0)), 40.0);
    }

    /// `Entity.shouldRenderAtSqrDistance` on a broken box: the size falls
    /// back to one block.
    #[test]
    fn a_broken_bounding_box_falls_back_to_one_block() {
        let range = default_range(
            glam::DVec3::new(f64::NAN, 0.7, 0.4),
            entity_view_scale(12, 1.0),
        );
        assert_eq!(range, 96.0);
    }
}
