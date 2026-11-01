//! Speculative decoration is isolated from gameplay and never writes normal saves.
use super::*;
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lod::{Column, Span, Surface},
    streaming::LodDetail,
};
use horizon_world::{
    heightmap::HeightmapState,
    registry::BlockStateId,
    worldgen::{
        decoration_order::vanilla_order,
        generator::{GenBlockStates, GeneratorError},
    },
};
use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

mod islands;
mod vegetation;
use islands::EndIslands;
use vegetation::Vegetation;
const CACHE: usize = 1024;
/// Chunks waiting for decoration: enough batches to keep every thread busy.
const REQUESTS: usize = 1024;
/// Chunks a side a decoration batch emits. Each needs a two-chunk border of
/// proto chunks, which the batch shares: 4 built four protos per chunk, 8
/// builds 2.25, and the protos (noise, surface) are most of far decoration.
const BATCH: i32 = 8;
fn batch_origin((x, z): (i32, i32)) -> (i32, i32) {
    (x.div_euclid(BATCH) * BATCH, z.div_euclid(BATCH) * BATCH)
}
#[derive(Default)]
struct Requests {
    chunks: FxHashSet<(i32, i32)>,
    center: [i32; 2],
    /// The batches the decoration threads are working on.
    current: FxHashSet<(i32, i32)>,
    finished: FxHashSet<(i32, i32)>,
}
struct Shared {
    requests: Mutex<Requests>,
    wake: Condvar,
    stop: AtomicBool,
}
fn distance((x, z): (i32, i32), center: [i32; 2]) -> i64 {
    let dx = i64::from(x * 16 + 8 - center[0]);
    let dz = i64::from(z * 16 + 8 - center[1]);
    dx * dx + dz * dz
}
struct Builder {
    shared: Arc<Shared>,
    results: mpsc::Receiver<LodDetail>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Builder {
    fn new(terrain: Arc<VanillaTerrain>) -> Self {
        let shared = Arc::new(Shared {
            requests: Mutex::new(Requests::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (tx, results) = mpsc::channel();
        // Idle priority: decoration only takes time the near chunks, the
        // ticks and the frame leave, so more cores can share it.
        let count = thread::available_parallelism()
            .map_or(1, |n| n.get() / 3)
            .clamp(1, 12);
        let threads = (0..count)
            .map(|_| Self::spawn(terrain.clone(), shared.clone(), tx.clone()))
            .collect();
        Self {
            shared,
            results,
            threads,
        }
    }
    fn spawn(
        terrain: Arc<VanillaTerrain>,
        worker: Arc<Shared>,
        tx: mpsc::Sender<LodDetail>,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("Far decoration".into())
            .spawn(move || {
                horizon_threads::idle_current();
                // A superflat world's far ring is its flat ground (`FlatTerrain::lod_surface`);
                // a normal overworld rebuilt here would draw hills, oceans and trees.
                if terrain.is_flat() {
                    worker.stop.store(true, Ordering::Relaxed);
                    return;
                }
                // Structure pieces have mutable one-shot placement flags. Sharing
                // gameplay's seeded pipeline would consume its chests/spawners.
                let generator = match OverworldGenerator::for_dimension(
                    terrain.generator.data().clone(),
                    terrain.seed,
                    &terrain.preset,
                    &terrain.dimension,
                ) {
                    Ok(generator) => generator,
                    Err(error) => {
                        log::warn!("Horizon: far decoration unavailable: {error}");
                        worker.stop.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                let mut state = generator.worker();
                loop {
                    let key = {
                        let mut requests = worker.requests.lock().unwrap();
                        while requests.chunks.is_empty() && !worker.stop.load(Ordering::Relaxed) {
                            requests = worker.wake.wait(requests).unwrap();
                        }
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let key = *requests
                            .chunks
                            .iter()
                            .min_by_key(|&&key| (distance(key, requests.center), key))
                            .unwrap();
                        let origin = batch_origin(key);
                        requests.chunks.retain(|&key| batch_origin(key) != origin);
                        requests.current.insert(origin);
                        origin
                    };
                    let result = decorated_batch(&mut state, key, BATCH, &worker.stop, |chunk| {
                        let pos = chunk.position();
                        let key = (pos.x, pos.z);
                        worker.requests.lock().unwrap().finished.insert(key);
                        let columns = columns(chunk, &terrain.biome_map, &terrain.dimension);
                        if tx.send((key, columns)).is_err() {
                            worker.stop.store(true, Ordering::Relaxed);
                        }
                    });
                    worker.requests.lock().unwrap().current.remove(&key);
                    match result {
                        Ok(_) => {}
                        Err(error) => {
                            log::warn!(
                                "Horizon: far decoration failed at {} {}: {error}",
                                key.0,
                                key.1
                            );
                            worker.stop.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            })
            .expect("far decoration worker")
    }
    fn request(&self, chunks: &[(i32, i32)], center: [i32; 2]) {
        if self.shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let mut requests = self.shared.requests.lock().unwrap();
        requests.center = center;
        for &key in chunks {
            if !requests.current.contains(&batch_origin(key)) && !requests.finished.contains(&key) {
                requests.chunks.insert(key);
            }
        }
        if requests.chunks.len() > REQUESTS {
            let mut sorted: Vec<_> = requests.chunks.iter().copied().collect();
            sorted.sort_unstable_by_key(|&key| (distance(key, center), key));
            requests.chunks = sorted.into_iter().take(REQUESTS).collect();
        }
        self.shared.wake.notify_all();
    }
}
impl Drop for Builder {
    fn drop(&mut self) {
        // Under the lock the threads check it with: one between its check and
        // its wait missed the wake, and closing the world waited forever.
        let requests = self.shared.requests.lock();
        self.shared.stop.store(true, Ordering::Relaxed);
        drop(requests);
        self.shared.wake.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Share the two-chunk terrain halo and one-chunk feature halo across adjacent
/// targets. Emit each target only after all nine feature centers have run.
/// These proto chunks never enter World, lighting or normal save queues.
fn decorated_batch(
    worker: &mut GeneratorWorker,
    origin: (i32, i32),
    size: i32,
    stop: &AtomicBool,
    mut emit: impl FnMut(&ProtoChunk),
) -> Result<FxHashMap<(i32, i32), ProtoChunk>, GeneratorError> {
    // Each region starts afresh; flags from a previous speculative structure
    // placement must not suppress blocks in this region.
    worker.forget_structure_starts();
    let mut chunks = FxHashMap::default();
    for dx in -2..size + 2 {
        for dz in -2..size + 2 {
            if stop.load(Ordering::Relaxed) {
                return Ok(FxHashMap::default());
            }
            let key = (origin.0 + dx, origin.1 + dz);
            chunks.insert(key, worker.generate_proto(ChunkPos::new(key.0, key.1))?);
        }
    }
    let mut remaining: FxHashMap<_, u8> = (0..size)
        .flat_map(|x| (0..size).map(move |z| ((origin.0 + x, origin.1 + z), 9)))
        .collect();
    let center = (origin.0 + size / 2, origin.1 + size / 2);
    for (x, z) in vanilla_order(center, size / 2)
        .into_iter()
        .filter(|&(x, z)| {
            x >= origin.0 - 1 && x <= origin.0 + size && z >= origin.1 - 1 && z <= origin.1 + size
        })
    {
        if stop.load(Ordering::Relaxed) {
            return Ok(FxHashMap::default());
        }
        let mut region: [ProtoChunk; 9] = std::array::from_fn(|i| {
            chunks
                .remove(&(x + i as i32 % 3 - 1, z + i as i32 / 3 - 1))
                .expect("generated neighborhood")
        });
        worker.decorate(&mut region)?;
        for chunk in region {
            let pos = chunk.position();
            chunks.insert((pos.x, pos.z), chunk);
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                let key = (x + dx, z + dz);
                if let Some(left) = remaining.get_mut(&key) {
                    *left -= 1;
                    if *left == 0 {
                        emit(&chunks[&key]);
                    }
                }
            }
        }
    }
    assert!(
        remaining.values().all(|&left| left == 0),
        "incomplete far decoration halo"
    );
    Ok(chunks)
}

#[cfg(test)]
fn decorated_region(
    worker: &mut GeneratorWorker,
    center: (i32, i32),
    stop: &AtomicBool,
) -> Result<Option<ProtoChunk>, GeneratorError> {
    Ok(decorated_batch(worker, center, 1, stop, |_| {})?.remove(&center))
}
fn columns(chunk: &ProtoChunk, biome_map: &[u16], dimension: &str) -> Arc<[Column; 256]> {
    let states = chunk.data().block_states();
    let registry = chunk.data().registry();
    // The level's range, all of it inside its column; the column's bottom
    // stands for "no surface".
    let (min, height) = (chunk.terrain.min_y(), chunk.terrain.height());
    let max = min + height;
    let bottom = crate::chunk::ColumnLayout::of_level(min, height).min_y();
    let water = registry.default_state("minecraft:water");
    Arc::new(std::array::from_fn(|index| {
        let x = (index & 15) as i32;
        let z = (index >> 4) as i32;
        let mut column = Column::default();
        for y in min..max {
            let mut state = chunk.block(x, y, z);
            let properties = states.get(state);
            let mut name = registry.state(state).map_or("", |s| s.name.as_str());
            if properties.is_air()
                || (!properties.blocks_motion_in_heightmap()
                    && !properties.has_fluid()
                    && !matches!(name, "minecraft:chorus_plant" | "minecraft:chorus_flower"))
            {
                continue;
            }
            // Kelp and seagrass hold water: far off they are the water around
            // them, as saved chunks' far columns have them. As spans of their
            // own they hid the seabed with no top face of theirs, a hole
            // where the sky showed through (white dots across the sea).
            if !properties.blocks_motion_in_heightmap()
                && properties.has_fluid()
                && !matches!(name, "minecraft:water" | "minecraft:lava")
                && let Some(water) = water
            {
                state = water;
                name = "minecraft:water";
            }
            let fluid = matches!(name, "minecraft:water" | "minecraft:lava");
            let biome = biome_map
                .get(chunk.biome_id_at(x, y, z).0 as usize)
                .copied()
                .unwrap_or(0);
            // shortcut: no propagated far block light, use the normal lighting
            // engine if accurate distant block-light propagation is required.
            // Skylight is `shade_sky`'s, once the column is complete.
            let light = if dimension == "minecraft:the_nether" {
                0
            } else {
                240
            } | if name == "minecraft:lava" {
                15 << 12
            } else {
                0
            };
            column.push(Span {
                min: y as i16,
                max: (y + 1) as i16,
                state,
                biome,
                light,
            });
            if fluid {
                let surface = column.surface.get_or_insert(Surface {
                    height: bottom,
                    state: BlockStateId(0),
                    water: None,
                    biome,
                });
                surface.water = Some((y + 1, state));
            } else {
                column.surface = Some(Surface {
                    height: y + 1,
                    state,
                    water: None,
                    biome,
                });
            }
        }
        if dimension != "minecraft:the_nether" {
            shade_sky(&mut column, states);
        }
        column
    }))
}
/// Skylight as it reaches the top of each span straight down from the sky:
/// fluids and leaves take a level per block, as vanilla's dampening does.
/// Full skylight everywhere lit seabeds and the ground under canopies as if
/// they stood in the open.
// ponytail: straight down only. Solid blocks take none, though vanilla's
// stop it: the faces under them that far terrain shows are cliffs and
// overhangs, which vanilla lights sideways from the open air, and they went
// black. The lighting engine for exact light.
fn shade_sky(column: &mut Column, states: &GenBlockStates) {
    // Dimming blocks get a span a level, as a real chunk's light has them,
    // down to the dark: one lit span for a whole sea lit its bed, where
    // `Column::reduced` merges water and sand, as brightly as its surface.
    let mut sky = 15u32;
    let mut shaded = Vec::with_capacity(column.spans.len());
    for span in column.spans.iter().rev() {
        let lit = |min, max, sky: u32| Span {
            min,
            max,
            light: (span.light & !0xf0) | (sky << 4) as u16,
            ..*span
        };
        let state = states.get(span.state);
        if state.motion && state.no_leaves {
            shaded.push(lit(span.min, span.max, sky));
            continue;
        }
        let mut top = span.max;
        while top > span.min {
            let bottom = if sky == 0 { span.min } else { top - 1 };
            shaded.push(lit(bottom, top, sky));
            sky = sky.saturating_sub((top - bottom) as u32);
            top = bottom;
        }
    }
    shaded.reverse();
    column.spans = shaded.into();
}

/// A whole tile inside visited chunks needs no density work at all. This
/// used to evaluate the lattice first and throw all of it away in the per-cell
/// cache check below.
fn cached_grid(
    cache: &FxHashMap<(i32, i32), Arc<[Column; 256]>>,
    x: i32,
    z: i32,
    width: i32,
    side: usize,
) -> Option<Vec<Column>> {
    if width > 16 || cache.is_empty() {
        return None;
    }
    let mut columns = Vec::with_capacity(side * side);
    for dz in 0..side {
        for dx in 0..side {
            let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
            let cached = cache.get(&(wx >> 4, wz >> 4))?;
            columns.push(Column::reduced(
                cached,
                (wx & 15) as usize,
                (wz & 15) as usize,
                width as usize,
            ));
        }
    }
    Some(columns)
}

pub(super) struct VanillaLodSampler {
    generator: Arc<VanillaTerrain>,
    worker: GeneratorWorker,
    builder: Option<Builder>,
    vegetation: Arc<Vegetation>,
    islands: EndIslands,
    cache: FxHashMap<(i32, i32), Arc<[Column; 256]>>,
    order: VecDeque<(i32, i32)>,
    details: Vec<LodDetail>,
    /// Reduced far detail: `lod_grid` approximates wide cells.
    approximate: bool,
}
impl VanillaLodSampler {
    pub(super) fn new(generator: Arc<VanillaTerrain>) -> Self {
        let worker = generator.generator.worker();
        let builder = Some(Builder::new(generator.clone()));
        let vegetation = Arc::new(Vegetation::new(
            generator.generator.data(),
            &generator.biome_map,
            generator.seed,
        ));
        let islands = EndIslands::new(generator.clone());
        Self {
            generator,
            worker,
            builder,
            vegetation,
            islands,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
            details: Vec::new(),
            approximate: false,
        }
    }
}
impl LodSampler for VanillaLodSampler {
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            worker: self.generator.generator.worker(),
            builder: None,
            vegetation: self.vegetation.clone(),
            islands: self.islands.fork(),
            cache: FxHashMap::default(),
            order: VecDeque::new(),
            details: Vec::new(),
            approximate: self.approximate,
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            if let Some(columns) = self.cache.get(&key) {
                self.details.push((key, columns.clone()));
            } else {
                missing.push(key);
            }
        }
        if let Some(builder) = &self.builder {
            builder.request(&missing, center);
        }
    }
    fn drain_details(&mut self) -> Vec<LodDetail> {
        let Some(builder) = &self.builder else {
            return std::mem::take(&mut self.details);
        };
        while let Ok((key, columns)) = builder.results.try_recv() {
            if !self.cache.contains_key(&key) {
                self.order.push_back(key);
            }
            self.cache.insert(key, columns.clone());
            while self.cache.len() > CACHE {
                if let Some(old) = self.order.pop_front() {
                    self.cache.remove(&old);
                    builder
                        .shared
                        .requests
                        .lock()
                        .unwrap()
                        .finished
                        .remove(&old);
                }
            }
            self.details.push((key, columns));
        }
        std::mem::take(&mut self.details)
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> Column {
        if width <= 16
            && let Some(columns) = self.cache.get(&(x >> 4, z >> 4))
        {
            return Column::reduced(
                columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.grid(x, z, width, 1).pop().unwrap_or_default()
    }
    fn grid(&mut self, x: i32, z: i32, width: i32, side: usize) -> Vec<Column> {
        if let Some(columns) = cached_grid(&self.cache, x, z, width, side) {
            return columns;
        }
        let samples =
            match self
                .worker
                .lod_grid(x + width / 2, z + width / 2, width, side, self.approximate)
            {
                Ok(samples) => samples,
                Err(error) => {
                    log::warn!("Horizon: direct far sampling failed: {error}");
                    return vec![Column::default(); side * side];
                }
            };
        samples
            .into_iter()
            .enumerate()
            .map(|(index, (surface, spans))| {
                let (wx, wz) = (
                    x + (index % side) as i32 * width,
                    z + (index / side) as i32 * width,
                );
                if width <= 16
                    && let Some(columns) = self.cache.get(&(wx >> 4, wz >> 4))
                {
                    return Column::reduced(
                        columns,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        width as usize,
                    );
                }
                self.preview_column(wx + width / 2, wz + width / 2, width, surface, spans)
            })
            .collect()
    }
}
impl VanillaLodSampler {
    fn preview_column(
        &mut self,
        x: i32,
        z: i32,
        width: i32,
        surface: Option<SurfaceSample>,
        spans: Vec<(i32, i32, BlockStateId)>,
    ) -> Column {
        let surface = surface.map(|sample| self.generator.client_surface(sample));
        let biome = surface.map_or(0, |surface| surface.biome);
        let registry = self.generator.generator.data().registry();
        let mut column = Column {
            surface,
            spans: crate::lod::Spans::new(),
        };
        for (min, max, state) in spans.into_iter().rev() {
            column.push(Span {
                min: min as i16,
                max: max as i16,
                state,
                biome,
                light: if self.generator.dimension == "minecraft:the_nether" {
                    0
                } else {
                    240
                } | if registry
                    .state(state)
                    .is_some_and(|s| s.name == "minecraft:lava")
                {
                    15 << 12
                } else {
                    0
                },
            });
        }
        let nether = self.generator.dimension == "minecraft:the_nether";
        let light = if nether { 0 } else { 240 };
        self.vegetation.apply(&mut column, x, z, width, light);
        self.islands.apply(&mut column, x, z, width, light);
        if !nether {
            shade_sky(&mut column, self.generator.generator.data().block_states());
        }
        column
    }
}

#[cfg(test)]
mod tests {
    /// The all-cached path must return exactly the per-cell reductions the
    /// old override produced, including negative chunk coordinates.
    #[test]
    fn cached_grid_equals_each_cells_reduction() {
        let mut cache: FxHashMap<(i32, i32), Arc<[Column; 256]>> = FxHashMap::default();
        let columns: Arc<[Column; 256]> = Arc::new(std::array::from_fn(|i| {
            Column::from_surface(Some(Surface {
                height: 60 + i as i32 % 8,
                state: BlockStateId(i as u32 % 3 + 1),
                water: None,
                biome: 0,
            }))
        }));
        cache.insert((0, 0), columns.clone());
        cache.insert((-1, -1), columns.clone());
        for (x, z) in [(0, 0), (4, 4), (8, 8), (-8, -8), (-16, -16)] {
            let fast = cached_grid(&cache, x, z, 4, 2)
                .unwrap_or_else(|| panic!("inside cached chunks at {x},{z}"));
            for dz in 0..2 {
                for dx in 0..2 {
                    let (wx, wz) = (x + dx * 4, z + dz * 4);
                    assert_eq!(
                        fast[dz as usize * 2 + dx as usize],
                        Column::reduced(&columns, (wx & 15) as usize, (wz & 15) as usize, 4)
                    );
                }
            }
        }
        // One missing chunk falls back to the generator path.
        assert!(cached_grid(&cache, 12, 0, 4, 2).is_none());
        assert!(cached_grid(&cache, 0, 0, 32, 2).is_none());
    }

    use super::*;
    #[test]
    #[ignore = "requires prepared worldgen data in HORIZON_TEST_RESOURCES"]
    fn decorated_lod_refines_real_seed_canopies_and_all_dimensions() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(&root)?;
        let terrain = Arc::new(VanillaTerrain::new(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
        )?);
        let mut sampler = VanillaLodSampler::new(terrain.clone());
        let mut fork = sampler.fork().expect("independent far sampler");
        let direct = sampler.grid(-1024, 0, 32, 16);
        assert_eq!(
            direct,
            fork.grid(-1024, 0, 32, 16),
            "sampling workers must agree on seed/biome vegetation"
        );
        let grass = assets
            .registry
            .default_state("minecraft:grass_block")
            .unwrap();
        let planted = |ground, biome, width, x, z| {
            let mut column = Column::from_surface(Some(Surface {
                height: 80,
                state: ground,
                water: None,
                biome,
            }));
            sampler.vegetation.apply(&mut column, x, z, width, 240);
            column
        };
        let forest = biomes.id("minecraft:forest");
        let plains = biomes.id("minecraft:plains");
        let leaves = |column: &Column| {
            column.spans.iter().any(|span| {
                assets
                    .registry
                    .state(span.state)
                    .unwrap()
                    .name
                    .ends_with("_leaves")
            })
        };
        let mut forest_count = 0;
        let mut plains_count = 0;
        for z in 0..64 {
            for x in 0..64 {
                forest_count += usize::from(leaves(&planted(grass, forest, 1, x, z)));
                plains_count += usize::from(leaves(&planted(grass, plains, 1, x, z)));
            }
        }
        assert!(
            forest_count > 1000 && forest_count > plains_count * 4,
            "feature-derived vegetation density: forest={forest_count}, plains={plains_count}"
        );
        for width in [16, 32, 128, 512, 1024] {
            assert!(
                leaves(&planted(grass, forest, width, 0, 0)),
                "forest canopy missing at scale {width}"
            );
        }
        // The End's chorus plants come from the biome's own feature data: only
        // end highlands carry it, and only end stone is a valid ground.
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        let chorus_name = |state| {
            matches!(
                assets.registry.state(state).unwrap().name.as_str(),
                "minecraft:chorus_plant" | "minecraft:chorus_flower"
            )
        };
        let chorus = |column: &Column| column.spans.iter().any(|span| chorus_name(span.state));
        let highlands = biomes.id("minecraft:end_highlands");
        assert!(
            sampler.vegetation.chorus_count(highlands).unwrap_or(0.) > 0.,
            "end highlands keep the chorus feature"
        );
        assert_eq!(
            sampler.vegetation.chorus_count(forest),
            None,
            "a biome without the feature estimates none"
        );
        let mut highlands_count = 0;
        let mut control_count = 0;
        for z in 0..64 {
            for x in 0..64 {
                highlands_count += usize::from(chorus(&planted(end_stone, highlands, 1, x, z)));
                control_count += usize::from(chorus(&planted(end_stone, forest, 1, x, z)));
            }
        }
        assert!(
            highlands_count > 0 && control_count == 0,
            "feature-derived chorus density: highlands={highlands_count}, forest={control_count}"
        );
        let stem = (0..64)
            .flat_map(|z| (0..64).map(move |x| (x, z)))
            .map(|(x, z)| planted(end_stone, highlands, 1, x, z))
            .find(|column| chorus(column))
            .expect("a chorus column");
        let plant = stem
            .spans
            .iter()
            .find(|span| chorus_name(span.state))
            .unwrap();
        assert!(
            (2..=5).contains(&(i32::from(plant.max - plant.min))),
            "stem height follows the feature's 2..=5"
        );
        let top = stem.spans.last().unwrap();
        assert_eq!(
            assets.registry.state(top.state).unwrap().name,
            "minecraft:chorus_flower"
        );
        assert_eq!(stem.surface.unwrap().height, i32::from(top.max));
        let mut wide = 0;
        for width in [2, 4] {
            for z in (0..64).step_by(2) {
                for x in (0..64).step_by(2) {
                    wide += usize::from(chorus(&planted(end_stone, highlands, width, x, z)));
                }
            }
        }
        assert!(wide > 0, "nearby LOD cells still draw chorus");
        for z in (0..64).step_by(16) {
            for x in (0..64).step_by(16) {
                assert!(
                    !chorus(&planted(end_stone, highlands, 16, x, z)),
                    "a 16-block cell is too wide to read as one plant"
                );
            }
        }
        let key = (-64, 0);
        sampler.request_detail(&[key], [-1024, 0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let generated = loop {
            if let Some((_, columns)) = sampler.drain_details().into_iter().find(|(k, _)| *k == key)
            {
                break columns;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "decoration worker stalled"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let count = |suffix: &str| {
            generated
                .iter()
                .flat_map(|column| &column.spans)
                .filter(|span| {
                    assets
                        .registry
                        .state(span.state)
                        .unwrap()
                        .name
                        .ends_with(suffix)
                })
                .map(|span| i32::from(span.max - span.min))
                .sum::<i32>()
        };
        assert!(count("_leaves") > 0, "unexplored forest has a real canopy");
        assert!(count("_log") > 0, "canopy retains its trunks");
        let sky = |span: &Span| (span.light >> 4) & 15;
        assert!(
            generated
                .iter()
                .filter_map(|column| column.spans.last())
                .all(|top| sky(top) == 15),
            "the top of every column is in the open"
        );
        assert!(
            generated
                .iter()
                .flat_map(|column| &column.spans)
                .any(|span| (1..15).contains(&sky(span))),
            "ground under the canopy is shaded, not black"
        );
        assert!(generated.iter().all(|column| {
            column
                .spans
                .windows(2)
                .all(|pair| pair[0].max <= pair[1].min)
        }));
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    sampler.column(key.0 * 16 + x, key.1 * 16 + z),
                    generated[(z * 16 + x) as usize]
                );
            }
        }
        sampler.request_detail(&[key], [-1024, 0]);
        let cached = sampler.drain_details();
        assert!(
            Arc::ptr_eq(&cached[0].1, &generated),
            "reuses completed compact data"
        );
        drop(sampler);
        let mut worker = terrain.generator.worker();
        let started = std::time::Instant::now();
        for x in 0..BATCH {
            for z in 0..BATCH {
                decorated_region(&mut worker, (key.0 + x, key.1 + z), &AtomicBool::new(false))?
                    .unwrap();
            }
        }
        let separate = started.elapsed();
        let started = std::time::Instant::now();
        let mut emitted = FxHashMap::default();
        let region = decorated_batch(&mut worker, key, BATCH, &AtomicBool::new(false), |chunk| {
            let pos = chunk.position();
            assert!(
                emitted
                    .insert(
                        (pos.x, pos.z),
                        columns(chunk, &terrain.biome_map, "minecraft:overworld")
                    )
                    .is_none()
            );
        })?;
        eprintln!(
            "far generation: {} separate targets={separate:?}, shared targets={:?}",
            emitted.len(),
            started.elapsed()
        );
        assert_eq!(emitted.len(), (BATCH * BATCH) as usize);
        for (key, data) in emitted {
            assert_eq!(
                data,
                columns(&region[&key], &terrain.biome_map, "minecraft:overworld"),
                "emitted before all neighbouring features completed"
            );
        }
        assert_eq!(batch_origin((-1, -BATCH - 1)), (-BATCH, -2 * BATCH));
        let stop = AtomicBool::new(true);
        assert!(decorated_region(&mut terrain.generator.worker(), key, &stop)?.is_none());
        for (dimension, center) in [
            ("minecraft:the_nether", (0, 0)),
            ("minecraft:the_end", (16, 0)),
        ] {
            let generator = OverworldGenerator::for_dimension(
                terrain.generator.data().clone(),
                1234,
                "minecraft:normal",
                dimension,
            )?;
            let mut emitted = 0;
            let mut region = decorated_batch(
                &mut generator.worker(),
                center,
                BATCH,
                &AtomicBool::new(false),
                |_| emitted += 1,
            )?;
            assert_eq!(emitted, (BATCH * BATCH) as usize);
            let chunk = region.remove(&center).unwrap();
            let data = columns(&chunk, &terrain.biome_map, dimension);
            if dimension == "minecraft:the_nether" {
                assert!(
                    data.iter()
                        .flat_map(|c| &c.spans)
                        .all(|s| s.light & 0xf0 == 0)
                );
                assert!(data.iter().flat_map(|c| &c.spans).any(|s| {
                    assets.registry.state(s.state).unwrap().name == "minecraft:lava"
                        && s.light >> 12 == 15
                }));
            } else {
                assert!(
                    data.iter().all(|c| c.spans.is_empty()),
                    "End void stays empty"
                );
            }
        }
        // Reuse the saved-proto bastion fixture. Repeated speculative regions
        // must reconstruct structures after their one-shot placement flags ran.
        let generator = OverworldGenerator::for_dimension(
            terrain.generator.data().clone(),
            -1860902098141295193,
            "minecraft:normal",
            "minecraft:the_nether",
        )?;
        let mut worker = generator.worker();
        let mut previous = None;
        for _ in 0..2 {
            let chunk = decorated_region(&mut worker, (3, 7), &AtomicBool::new(false))?.unwrap();
            assert!(
                chunk
                    .structures()
                    .iter()
                    .any(|(name, _, _, _)| name.contains("bastion"))
            );
            let data = columns(&chunk, &terrain.biome_map, "minecraft:the_nether");
            let snapshot = (data, chunk.block_entities);
            if let Some(previous) = &previous {
                assert_eq!(
                    &snapshot, previous,
                    "speculative structure state leaked between regions"
                );
            }
            previous = Some(snapshot);
        }
        Ok(())
    }
    /// Small End islands are decoration over the void, so the direct sampler
    /// has to estimate them from `end_island_decorated`.
    #[test]
    #[ignore = "requires prepared worldgen data in HORIZON_TEST_RESOURCES"]
    fn end_small_islands_are_estimated_over_the_void() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(&root)?;
        let terrain = Arc::new(VanillaTerrain::for_dimension(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
            "minecraft:the_end",
        )?);
        let mut sampler = VanillaLodSampler::new(terrain.clone());
        let mut search = terrain.generator.worker();
        let mut center = None;
        'search: for x in (1024..=8192).step_by(512) {
            for z in (-8192..=8192).step_by(512) {
                let biome = search.noise_biomes(&[[x >> 2, 16, z >> 2]]);
                if biome.first().and_then(|name| name.as_deref())
                    != Some("minecraft:small_end_islands")
                {
                    continue;
                }
                if search.surface_sample(x, z).is_some() {
                    continue;
                }
                center = Some((x, z));
                break 'search;
            }
        }
        let (cx, cz) = center.expect("a small end islands region");
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        // The estimator itself, a wide void at one block per cell.
        let mut islands = 0;
        for z in cz - 128..cz + 128 {
            for x in cx - 128..cx + 128 {
                let mut column = Column::default();
                sampler.islands.apply(&mut column, x, z, 1, 240);
                let Some(surface) = column.surface else {
                    continue;
                };
                assert!(
                    (56..=71).contains(&surface.height),
                    "island top at {}",
                    surface.height
                );
                assert_eq!(surface.state, end_stone);
                assert!(column.spans.iter().any(|span| {
                    span.state == end_stone && i32::from(span.max) == surface.height
                }));
                assert!(
                    column.spans.iter().all(|span| i32::from(span.min) >= 45),
                    "an estimated island floats above the void"
                );
                islands += 1;
            }
        }
        assert!(islands > 0, "no estimated small islands in a 256² void");
        // And through the whole direct-sampling path with the real biome map.
        let sampled = sampler.grid(cx - 32, cz - 32, 1, 64);
        assert!(
            sampled.iter().any(|column| column
                .surface
                .is_some_and(|surface| surface.state == end_stone)),
            "direct sampling shows an island in the real small end islands biome"
        );
        Ok(())
    }
    /// The refined path must keep the real feature's end stone in its columns.
    #[test]
    #[ignore = "requires prepared worldgen data in HORIZON_TEST_RESOURCES"]
    fn end_small_islands_from_decoration_survive_far_columns() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(&root)?;
        let terrain = Arc::new(VanillaTerrain::for_dimension(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
            "minecraft:the_end",
        )?);
        let sampler = VanillaLodSampler::new(terrain.clone());
        let mut worker = terrain.generator.worker();
        let stop = AtomicBool::new(false);
        let mut centers = Vec::new();
        'search: for cx in (64..=512).step_by(4) {
            for cz in (-512..=512).step_by(4) {
                let (x, z) = (cx * 16 + 8, cz * 16 + 8);
                let biome = worker.noise_biomes(&[[x >> 2, 16, z >> 2]]);
                if biome.first().and_then(|name| name.as_deref())
                    != Some("minecraft:small_end_islands")
                {
                    continue;
                }
                if worker.surface_sample(x, z).is_some() {
                    continue;
                }
                centers.push((cx, cz));
                if centers.len() >= 200 {
                    break 'search;
                }
            }
        }
        for &(cx, cz) in &centers {
            // The replay is cheap; only decorate chunks it actually places in.
            let origins = sampler.islands.origins(cx, cz);
            if origins.is_empty() {
                continue;
            }
            let Some(chunk) = decorated_region(&mut worker, (cx, cz), &stop)? else {
                continue;
            };
            for [x, y, z] in &origins {
                assert!((55..=70).contains(y), "island origin at y {y} in {cx} {cz}");
                assert_eq!(
                    chunk.block_name(*x, *y, *z),
                    "minecraft:end_stone",
                    "exact origin {x} {y} {z} is the real island's top centre"
                );
            }
            let min = chunk.terrain.min_y();
            let max = min + chunk.terrain.height();
            let mut raw = 0;
            for y in min..max {
                for z in 0..16 {
                    for x in 0..16 {
                        raw += usize::from(chunk.block_name(x, y, z) == "minecraft:end_stone");
                    }
                }
            }
            let kept: usize = columns(&chunk, &terrain.biome_map, "minecraft:the_end")
                .iter()
                .flat_map(|column| &column.spans)
                .filter(|span| {
                    assets.registry.state(span.state).unwrap().name == "minecraft:end_stone"
                })
                .map(|span| (span.max - span.min) as usize)
                .sum();
            assert_eq!(
                raw, kept,
                "far columns keep the real small island in {cx} {cz}"
            );
            return Ok(());
        }
        panic!(
            "no exact small end island among {} candidates",
            centers.len()
        );
    }
    /// Vanilla decoration places chorus plants in the End; the far columns must
    /// keep them even though they are not motion-blocking blocks.
    #[test]
    #[ignore = "requires prepared worldgen data in HORIZON_TEST_RESOURCES"]
    fn end_far_columns_keep_real_chorus_plants() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(&root)?;
        let terrain = Arc::new(VanillaTerrain::for_dimension(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
            "minecraft:the_end",
        )?);
        let mut worker = terrain.generator.worker();
        let chorus = |state| {
            matches!(
                assets.registry.state(state).unwrap().name.as_str(),
                "minecraft:chorus_plant" | "minecraft:chorus_flower"
            )
        };
        let mut checked = 0;
        'search: for cx in (-160..=160).step_by(16) {
            for cz in (-160..=160).step_by(16) {
                let (x, z) = (cx * 16 + 8, cz * 16 + 8);
                let biome = worker.noise_biomes(&[[x >> 2, 16, z >> 2]]);
                if biome.first().and_then(|name| name.as_deref()) != Some("minecraft:end_highlands")
                {
                    continue;
                }
                if worker.surface_sample(x, z).is_none() {
                    continue;
                }
                checked += 1;
                if checked > 24 {
                    break 'search;
                }
                let Some(chunk) = decorated_region(&mut worker, (cx, cz), &AtomicBool::new(false))?
                else {
                    continue;
                };
                let min = chunk.terrain.min_y();
                let max = min + chunk.terrain.height();
                let mut raw = 0;
                for y in min..max {
                    for zz in 0..16 {
                        for xx in 0..16 {
                            raw += usize::from(chorus(chunk.block(xx, y, zz)));
                        }
                    }
                }
                if raw == 0 {
                    continue;
                }
                let kept: usize = columns(&chunk, &terrain.biome_map, "minecraft:the_end")
                    .iter()
                    .flat_map(|column| &column.spans)
                    .filter(|span| chorus(span.state))
                    .map(|span| (span.max - span.min) as usize)
                    .sum();
                assert_eq!(
                    raw, kept,
                    "far columns keep every real chorus block in {cx} {cz}"
                );
                return Ok(());
            }
        }
        panic!("no generated end_highlands chunk with chorus among {checked} candidates");
    }
}
