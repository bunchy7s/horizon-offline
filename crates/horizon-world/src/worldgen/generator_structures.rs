//! STRUCTURE_STARTS / STRUCTURE_REFERENCES for the chunk generator.
use super::{
    biome::BiomeId,
    block_feature::FeatureBlockBindings,
    generator::{ChunkWork, GeneratorData, GeneratorError},
    structure::{
        StructureResources,
        jigsaw::PoolPiece,
        start::{
            Piece, Rings, StartContext, StructureRegistry, StructureStart, beardifier,
            create_starts,
        },
        stronghold::RingSearch,
    },
};
use horizon_core::{density::Beardifier, position::ChunkPos};
use serde_json::Value;
use std::collections::BTreeMap;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Condvar, Mutex, OnceLock, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) type Starts = Arc<Vec<Arc<StructureStart>>>;
pub(super) struct StructureState {
    pub(super) registry: StructureRegistry,
    pub(super) resources: Arc<StructureResources>,
    pub(super) bindings: FeatureBlockBindings<'static>,
    possible_sets: Vec<usize>,
    /// Structure indices of each decoration step, in registry order.
    pub(super) by_step: Vec<Vec<usize>>,
    starts: StartCache,
    start_work: ChunkWork,
    /// Concentric ring positions, computed on first use and shared with
    /// later generators of the same seed and dimension.
    rings: Arc<OnceLock<Rings>>,
    /// Their biome searches while they are computed.
    ring_work: OnceLock<RingWork>,
    /// `StructureRegistry::zone_reach`.
    zone_reach: i32,
}

/// generateRingPositions' biome searches, shared out. Vanilla runs each as
/// its own task on its worker pool; here every thread that needs the rings
/// before they exist takes searches until none are left, then waits for the
/// ones others took. Done by the first thread alone, the 128 stronghold
/// searches kept every other worker blocked for seconds after each world load.
struct RingWork {
    /// The concentric-ring sets, some with no positions.
    sets: Vec<usize>,
    /// Set index and search, sets in `possible_sets` order.
    searches: Vec<(usize, RingSearch)>,
    next: AtomicUsize,
    found: Vec<OnceLock<ChunkPos>>,
    left: Mutex<usize>,
    finished: Condvar,
}
impl RingWork {
    fn new(sets: Vec<(usize, Vec<RingSearch>)>) -> Self {
        let indices = sets.iter().map(|(set, _)| *set).collect();
        let searches: Vec<(usize, RingSearch)> = sets
            .into_iter()
            .flat_map(|(set, searches)| searches.into_iter().map(move |s| (set, s)))
            .collect();
        Self {
            sets: indices,
            next: AtomicUsize::new(0),
            found: (0..searches.len()).map(|_| OnceLock::new()).collect(),
            left: Mutex::new(searches.len()),
            finished: Condvar::new(),
            searches,
        }
    }
    /// Run searches until none are left to take, then wait for the rest.
    fn help<'a>(
        &self,
        preferred: impl Fn(usize) -> &'a BTreeSet<BiomeId>,
        biomes: &mut dyn FnMut(&[[i32; 3]]) -> Vec<BiomeId>,
    ) {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let Some((set, search)) = self.searches.get(i) else {
                break;
            };
            let _ = self.found[i].set(search.find(preferred(*set), biomes));
            let mut left = self.left.lock().unwrap();
            *left -= 1;
            if *left == 0 {
                self.finished.notify_all();
            }
        }
        let mut left = self.left.lock().unwrap();
        while *left > 0 {
            left = self.finished.wait(left).unwrap();
        }
    }
    /// The positions by set name, once every search is done.
    fn rings(&self, name: impl Fn(usize) -> String) -> Rings {
        let mut rings: Rings = self
            .sets
            .iter()
            .map(|&set| (name(set), Vec::new()))
            .collect();
        for ((set, _), found) in self.searches.iter().zip(&self.found) {
            if let Some(positions) = rings.get_mut(&name(*set)) {
                positions.push(*found.get().expect("every search finished"));
            }
        }
        rings
    }
}
const START_CACHE_LIMIT: usize = 65536;
const START_SHARDS: usize = 64;

/// The built starts by chunk, in shards: a lookup locks one shard for
/// reading. Each generated chunk reads the 289 start chunks around it
/// (`references`) and the server thread reads the starts of the chunks its
/// mobs and players are in; with one lock for all, the server thread waited
/// on the workers.
struct StartCache {
    shards: Box<[RwLock<HashMap<(i32, i32), Starts>>]>,
}
impl StartCache {
    fn new() -> Self {
        Self {
            shards: (0..START_SHARDS).map(|_| RwLock::default()).collect(),
        }
    }
    fn shard(&self, key: (i32, i32)) -> &RwLock<HashMap<(i32, i32), Starts>> {
        let hash =
            (key.0 as u32).wrapping_mul(0x9E37_79B9) ^ (key.1 as u32).wrapping_mul(0x85EB_CA6B);
        &self.shards[(hash >> (32 - START_SHARDS.trailing_zeros())) as usize]
    }
    fn get(&self, key: (i32, i32)) -> Option<Starts> {
        self.shard(key).read().unwrap().get(&key).cloned()
    }
    /// Keeps `starts` unless another thread built the same chunk's first:
    /// everyone keeps the first, so placement state on its pieces is shared.
    fn insert_first(&self, key: (i32, i32), starts: Starts) -> Starts {
        let mut shard = self.shard(key).write().unwrap();
        if shard.len() >= START_CACHE_LIMIT / START_SHARDS {
            shard.clear();
        }
        shard.entry(key).or_insert(starts).clone()
    }
    fn replace(&self, key: (i32, i32), starts: Starts) {
        self.shard(key).write().unwrap().insert(key, starts);
    }
    fn clear(&self) {
        for shard in &self.shards {
            shard.write().unwrap().clear();
        }
    }
}

fn biome_set(data: &GeneratorData, value: &Value) -> Result<BTreeSet<BiomeId>, super::DecodeError> {
    let names: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut out = BTreeSet::new();
    for name in names {
        match name.strip_prefix('#') {
            Some(tag) => {
                for biome in data.biome_tags.get(tag).into_iter().flatten() {
                    out.extend(data.biome_id(biome));
                }
            }
            None => out.extend(data.biome_id(name)),
        }
    }
    Ok(out)
}
impl StructureState {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        possible_biomes: &BTreeSet<BiomeId>,
        rings: Arc<OnceLock<Rings>>,
    ) -> Result<Option<Self>, GeneratorError> {
        let (Some(resources), Some(raw)) = (&data.structures, &data.structure_resources) else {
            return Ok(None);
        };
        let registry =
            StructureRegistry::new(&raw.structures, &raw.sets, raw.pools.clone(), &|value| {
                biome_set(data, value)
            })?;
        let possible_sets = registry
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| {
                set.structures
                    .iter()
                    .any(|(s, _)| !registry.structures[*s].biomes.is_disjoint(possible_biomes))
            })
            .map(|(i, _)| i)
            .collect();
        let mut by_step = vec![Vec::new(); super::structure::start::STEPS.len()];
        for (i, def) in registry.structures.iter().enumerate() {
            by_step[def.step].push(i);
        }
        let mut bindings = FeatureBlockBindings::with_tags(
            data.registry(),
            data.integers().clone(),
            &data.block_tags,
        )?;
        // Processors that test what a block is made of read canOcclude.
        if let Some(blocks) = data
            .feature_resources
            .as_ref()
            .and_then(|r| r.blocks.as_ref())
        {
            bindings = bindings.with_block_properties(&blocks.properties)?;
        }
        let zone_reach = registry.zone_reach();
        Ok(Some(Self {
            zone_reach,
            registry,
            resources: resources.clone(),
            bindings,
            possible_sets,
            by_step,
            starts: StartCache::new(),
            start_work: ChunkWork::new(),
            rings,
            ring_work: OnceLock::new(),
        }))
    }
    /// Concentric ring positions of a structure set (vanilla's
    /// getRingPositionsFor), computed now if no lookup needed them yet.
    pub(super) fn ring_positions(
        &self,
        context: &mut impl StartContext,
        set: &str,
    ) -> Option<Vec<ChunkPos>> {
        self.rings(context).get(set).cloned()
    }
    fn ring_work(&self, context: &impl StartContext) -> &RingWork {
        self.ring_work.get_or_init(|| {
            RingWork::new(
                self.registry
                    .ring_searches(&self.possible_sets, context.seed()),
            )
        })
    }
    /// The concentric ring positions, computing them with the threads that
    /// need them first (see `RingWork`).
    fn rings(&self, context: &mut impl StartContext) -> &Rings {
        if let Some(rings) = self.rings.get() {
            return rings;
        }
        let work = self.ring_work(context);
        let registry = &self.registry;
        work.help(|set| registry.ring_biomes(set), &mut |quarts| {
            context.noise_biomes(quarts)
        });
        self.rings
            .get_or_init(|| work.rings(|set| registry.sets[set].name.clone()))
    }
    /// The rings a chunk's starts are decided with. Before they are known,
    /// a chunk no ring search can end near (with the exclusion zones' reach)
    /// is decided without them: no ring chunk is near it, which is what the
    /// rings would say. Strongholds lie 1,400 blocks out or more, so a new
    /// world's first chunks don't wait for the 128 biome searches (seconds
    /// on four threads); they run when a chunk near a ring needs them.
    fn rings_for(&self, context: &mut impl StartContext, chunk: ChunkPos) -> &Rings {
        static NONE: Rings = Rings::new();
        if let Some(rings) = self.rings.get() {
            return rings;
        }
        let near = self
            .ring_work(context)
            .searches
            .iter()
            .any(|(_, search)| search.may_land_near(chunk.x, chunk.z, self.zone_reach));
        if near { self.rings(context) } else { &NONE }
    }
    /// Drop the cached starts, so that later chunks build fresh pieces (the
    /// pieces keep per-start placement state such as a shipwreck's height).
    pub(super) fn forget_starts(&self) {
        // As before, a build already in flight can publish after this clear.
        // Do not wait for work locks or retain initialized entries here.
        self.starts.clear();
    }
    /// The (cached) structure starts owned by a chunk.
    pub(super) fn starts_at(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Starts {
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return found;
        }
        // Ring searches enlist other workers and wait for them: never hold a
        // chunk-work lock while helping, even when this chunk has no starts.
        let rings = self.rings_for(context, chunk);
        let work = self.start_work.at((chunk.x, chunk.z));
        let _work = work.lock().unwrap();
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return found;
        }
        let starts: Starts = Arc::new(
            create_starts(
                &self.registry,
                &self.resources,
                &self.bindings,
                context,
                &self.possible_sets,
                rings,
                chunk,
            )
            .into_iter()
            .map(Arc::new)
            .collect(),
        );
        self.starts.insert_first((chunk.x, chunk.z), starts)
    }
    /// Puts a saved start of `structure` in `chunk`, in place of the one
    /// this generator builds there from the same structure sets (a set
    /// starts one structure per chunk): `pieces` as a save lists them (see
    /// `PoolPiece::from_saved`). Until the starts are forgotten.
    pub(super) fn use_saved_start(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[Value],
    ) -> Result<(), String> {
        let index = (self.registry.structures.iter())
            .position(|s| s.name == structure)
            .ok_or_else(|| format!("unknown structure {structure}"))?;
        let pieces = (pieces.iter())
            .map(|p| PoolPiece::from_saved(p, &self.resources, &self.bindings).map(Piece::Pool))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{structure} at {chunk:?}: {e}"))?;
        if pieces.is_empty() {
            return Err(format!("{structure} at {chunk:?}: no pieces"));
        }
        let def = &self.registry.structures[index];
        let sets: Vec<_> = (self.registry.sets.iter())
            .filter(|set| set.structures.iter().any(|(s, _)| *s == index))
            .collect();
        let same_set = |s: usize| {
            s == index
                || sets
                    .iter()
                    .any(|set| set.structures.iter().any(|(t, _)| *t == s))
        };
        let mut starts: Vec<_> = (self.starts_at(context, chunk).iter())
            .filter(|s| !same_set(s.structure))
            .cloned()
            .collect();
        starts.push(Arc::new(StructureStart::new(index, def, chunk, pieces)));
        self.starts.replace((chunk.x, chunk.z), Arc::new(starts));
        Ok(())
    }
    /// ChunkGenerator.findNearestMapStructure: the nearest start of one of
    /// `names` from block `from` (rings: every ring chunk; random spreads:
    /// sectors in growing squares up to `radius`). Returns the structure and
    /// its locate position (the start chunk's corner at y 0).
    pub(super) fn locate(
        &self,
        context: &mut impl super::structure::start::StartContext,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        let seed = context.seed();
        let rings = self.rings(context).clone();
        let wanted = |s: usize| names.iter().any(|n| *n == self.registry.structures[s].name);
        let dist = |p: [i32; 3]| {
            let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
            dx * dx + dz * dz
        };
        let mut best: Option<(f64, String, [i32; 3])> = None;
        let found_at = |this: &Self, context: &mut _, chunk: ChunkPos| -> Option<String> {
            this.starts_at(context, chunk)
                .iter()
                .find(|s| wanted(s.structure))
                .map(|s| s.name.clone())
        };
        for &si in &self.possible_sets {
            let set = &self.registry.sets[si];
            if !set.structures.iter().any(|(s, _)| wanted(*s)) {
                continue;
            }
            match set.spacing() {
                None => {
                    // Rings: nearest by the chunk's (8, 32, 8) block.
                    let Some(chunks) = rings.get(&set.name) else {
                        continue;
                    };
                    let mut nearest: Option<(f64, ChunkPos)> = None;
                    for &c in chunks {
                        let d = dist([c.x * 16 + 8, 32, c.z * 16 + 8]);
                        if nearest.is_none_or(|(n, _)| d < n)
                            && self
                                .registry
                                .is_structure_chunk(set, seed, c.x, c.z, &rings)
                        {
                            nearest = Some((d, c));
                        }
                    }
                    if let Some((_, c)) = nearest
                        && let Some(name) = found_at(self, context, c)
                    {
                        let p = set.locate_pos(c);
                        let d = dist(p);
                        if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                            best = Some((d, name, p));
                        }
                    }
                }
                Some(spacing) => {
                    let (cx, cz) = (from[0] >> 4, from[2] >> 4);
                    'search: for r in 0..=radius {
                        for dx in -r..=r {
                            let x_edge = dx == -r || dx == r;
                            for dz in -r..=r {
                                if !(x_edge || dz == -r || dz == r) {
                                    continue;
                                }
                                let (sx, sz) = (cx + spacing * dx, cz + spacing * dz);
                                let Some(c) = StructureRegistry::potential_chunk(set, seed, sx, sz)
                                else {
                                    continue;
                                };
                                if !self
                                    .registry
                                    .is_structure_chunk(set, seed, c.x, c.z, &rings)
                                {
                                    continue;
                                }
                                if let Some(name) = found_at(self, context, c) {
                                    let p = set.locate_pos(c);
                                    let d = dist(p);
                                    if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                                        best = Some((d, name, p));
                                    }
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
        }
        best.map(|(_, n, p)| (n, p))
    }
    /// ChunkGenerator.createReferences: starts within 8 chunks whose box
    /// intersects this chunk's column.
    pub(super) fn references(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Vec<Arc<StructureStart>> {
        let (bx, bz) = (chunk.x * 16, chunk.z * 16);
        let mut out = Vec::new();
        for sx in chunk.x - 8..=chunk.x + 8 {
            for sz in chunk.z - 8..=chunk.z + 8 {
                for start in self.starts_at(context, ChunkPos::new(sx, sz)).iter() {
                    let b = start.bounding_box;
                    if b.max[0] >= bx
                        && b.min[0] <= bx + 15
                        && b.max[2] >= bz
                        && b.min[2] <= bz + 15
                    {
                        out.push(start.clone());
                    }
                }
            }
        }
        // Vanilla keeps each structure's references in a LongOpenHashSet and
        // places the starts in its iteration order; reorder each structure's
        // starts within the slots they already occupy.
        let mut slots: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, start) in out.iter().enumerate() {
            slots.entry(start.structure).or_default().push(i);
        }
        let original = out.clone();
        for slots in slots.values().filter(|s| s.len() > 1) {
            let keys: Vec<i64> = slots
                .iter()
                .map(|&i| {
                    let c = original[i].chunk;
                    i64::from(c.x as u32) | (i64::from(c.z as u32) << 32)
                })
                .collect();
            for (&slot, &from) in slots.iter().zip(&super::long_set::iteration_order(&keys)) {
                out[slot] = original[slots[from]].clone();
            }
        }
        out
    }
    pub(super) fn beardifier(
        &self,
        references: &[Arc<StructureStart>],
        chunk: ChunkPos,
    ) -> Option<Beardifier> {
        beardifier(&self.registry, references, chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::structure::stronghold::{ring_positions, ring_searches};

    /// The start cache keeps the first starts built for a chunk, spreads a
    /// chunk's 289 reference lookups over the shards, and forgets on clear.
    #[test]
    fn start_cache_keeps_first_and_spreads_neighbours() {
        let cache = StartCache::new();
        let first: Starts = Arc::new(Vec::new());
        let second: Starts = Arc::new(Vec::new());
        assert!(cache.get((3, -4)).is_none());
        assert!(Arc::ptr_eq(
            &cache.insert_first((3, -4), first.clone()),
            &first
        ));
        assert!(Arc::ptr_eq(&cache.insert_first((3, -4), second), &first));
        assert!(Arc::ptr_eq(&cache.get((3, -4)).unwrap(), &first));
        for (cx, cz) in [(0, 0), (-1000, 77), (i32::MAX - 8, i32::MIN + 8)] {
            let mut used = vec![0; START_SHARDS];
            for x in cx - 8..=cx + 8 {
                for z in cz - 8..=cz + 8 {
                    let shard = cache.shard((x, z)) as *const _;
                    let i = cache
                        .shards
                        .iter()
                        .position(|s| std::ptr::eq(s, shard))
                        .unwrap();
                    used[i] += 1;
                }
            }
            assert!(used.iter().all(|&n| n > 0 && n < 20), "{used:?}");
        }
        cache.clear();
        assert!(cache.get((3, -4)).is_none());
    }

    /// A ring search ends where `may_land_near` allows: on its ring chunk
    /// or in the square it samples, never further.
    #[test]
    fn ring_searches_end_where_they_may() {
        let none = BTreeSet::new();
        for seed in [0, 1234, -77, i64::MAX] {
            for (b, search) in ring_searches(seed, 32, 3, 128).iter().enumerate() {
                // Preferred biomes on one quart at a time, in turn at each
                // corner of the square, else nowhere (the ring chunk).
                let corner = (b % 5) as i32;
                let biomes = |quarts: &[[i32; 3]]| -> Vec<BiomeId> {
                    let (first, last) = (quarts[0], quarts[quarts.len() - 1]);
                    let pick = match corner {
                        0 => first,
                        1 => [last[0], 0, first[2]],
                        2 => [first[0], 0, last[2]],
                        3 => last,
                        _ => [i32::MIN, 0, i32::MIN],
                    };
                    quarts
                        .iter()
                        .map(|q| BiomeId(u32::from(*q == pick)))
                        .collect()
                };
                let preferred: BTreeSet<BiomeId> = [BiomeId(1)].into();
                for wanted in [&preferred, &none] {
                    let found = search.find(wanted, &mut { biomes });
                    assert!(search.may_land_near(found.x, found.z, 0));
                    assert!(!search.may_land_near(found.x + 17, found.z, 0));
                }
            }
        }
    }

    /// Searches shared between threads give the positions of one thread
    /// running them in order, set by set.
    #[test]
    fn ring_work_shared_between_threads_matches_serial_search() {
        let preferred: BTreeSet<BiomeId> = (0..20).map(|b| BiomeId(b * 3)).collect();
        let biomes = |quarts: &[[i32; 3]]| -> Vec<BiomeId> {
            quarts
                .iter()
                .map(|q| {
                    BiomeId((q[0].wrapping_mul(31) ^ q[2].wrapping_mul(17)).rem_euclid(64) as u32)
                })
                .collect()
        };
        let none = BTreeSet::new();
        for seed in [0, 1234, -77, i64::MAX] {
            let work = RingWork::new(vec![
                (0, ring_searches(seed, 32, 3, 128)),
                (1, ring_searches(seed ^ 5, 10, 4, 0)),
                (2, ring_searches(seed.wrapping_add(1), 20, 5, 40)),
            ]);
            let sets = [&preferred, &none, &preferred];
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| work.help(|set| sets[set], &mut { biomes }));
                }
            });
            let rings = work.rings(|set| format!("set{set}"));
            let serial = |s: i64, distance, spread, count, preferred| {
                ring_positions(s, distance, spread, count, preferred, &mut { biomes })
            };
            assert_eq!(rings["set0"], serial(seed, 32, 3, 128, &preferred));
            assert_eq!(rings["set1"], Vec::new());
            assert_eq!(
                rings["set2"],
                serial(seed.wrapping_add(1), 20, 5, 40, &preferred)
            );
        }
    }
}
