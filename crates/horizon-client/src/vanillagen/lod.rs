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
    worldgen::{decoration_order::vanilla_order, generator::GeneratorError},
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
const REQUESTS: usize = 64;
const BATCH: i32 = 4;
fn batch_origin((x, z): (i32, i32)) -> (i32, i32) {
    (x.div_euclid(BATCH) * BATCH, z.div_euclid(BATCH) * BATCH)
}
#[derive(Default)]
struct Requests {
    chunks: FxHashSet<(i32, i32)>,
    center: [i32; 2],
    current: Option<(i32, i32)>,
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
    thread: Option<thread::JoinHandle<()>>,
}
impl Builder {
    fn new(terrain: Arc<VanillaTerrain>) -> Self {
        let shared = Arc::new(Shared {
            requests: Mutex::new(Requests::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let (tx, results) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("Far decoration".into())
            .spawn(move || {
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
                        requests.current = Some(origin);
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
                    worker.requests.lock().unwrap().current = None;
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
            .expect("far decoration worker");
        Self {
            shared,
            results,
            thread: Some(thread),
        }
    }
    fn request(&self, chunks: &[(i32, i32)], center: [i32; 2]) {
        if self.shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let mut requests = self.shared.requests.lock().unwrap();
        requests.center = center;
        for &key in chunks {
            if requests.current != Some(batch_origin(key)) && !requests.finished.contains(&key) {
                requests.chunks.insert(key);
            }
        }
        if requests.chunks.len() > REQUESTS {
            let mut sorted: Vec<_> = requests.chunks.iter().copied().collect();
            sorted.sort_unstable_by_key(|&key| (distance(key, center), key));
            requests.chunks = sorted.into_iter().take(REQUESTS).collect();
        }
        self.shared.wake.notify_one();
    }
}
impl Drop for Builder {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
        if let Some(thread) = self.thread.take() {
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
    let min = chunk.terrain.min_y().max(MIN_Y);
    let max = (chunk.terrain.min_y() + chunk.terrain.height()).min(MIN_Y + HEIGHT);
    Arc::new(std::array::from_fn(|index| {
        let x = (index & 15) as i32;
        let z = (index >> 4) as i32;
        let mut column = Column::default();
        for y in min..max {
            let state = chunk.block(x, y, z);
            let properties = states.get(state);
            let name = registry.state(state).map_or("", |s| s.name.as_str());
            if properties.is_air()
                || (!properties.blocks_motion_in_heightmap()
                    && !properties.has_fluid()
                    && !matches!(name, "minecraft:chorus_plant" | "minecraft:chorus_flower"))
            {
                continue;
            }
            let fluid = matches!(name, "minecraft:water" | "minecraft:lava");
            let biome = biome_map
                .get(chunk.biome_id_at(x, y, z).0 as usize)
                .copied()
                .unwrap_or(0);
            // shortcut: no propagated far lighting, use the normal lighting
            // engine if accurate distant block-light propagation is required.
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
                    height: MIN_Y,
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
        column
    }))
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
            spans: Vec::new(),
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
        let light = if self.generator.dimension == "minecraft:the_nether" {
            0
        } else {
            240
        };
        self.vegetation.apply(&mut column, x, z, width, light);
        self.islands.apply(&mut column, x, z, width, light);
        column
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(batch_origin((-1, -5)), (-4, -8));
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
