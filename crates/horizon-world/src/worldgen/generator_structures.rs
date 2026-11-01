//! STRUCTURE_STARTS / STRUCTURE_REFERENCES for the chunk generator.
use super::{
    biome::BiomeId,
    block_feature::FeatureBlockBindings,
    generator::{ChunkWork, GeneratorData, GeneratorError, shard_of},
    structure::{
        StructureResources,
        jigsaw::PoolPiece,
        start::{
            Piece, Rings, StartContext, StructureRegistry, StructureStart, beardifier,
            create_starts, try_generate,
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
        Arc, Condvar, Mutex, OnceLock, RwLock, TryLockError,
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
    /// The seed the rings are searched with when it is not the world's (a superflat world's 0).
    pub(super) ring_seed: Option<i64>,
    /// The biome source is a `FixedBiomeSource` (a single-biome world, a superflat one).
    pub(super) fixed_biome: bool,
    /// `StructureRegistry::zone_reach`.
    zone_reach: i32,
}

/// generateRingPositions' biome searches, shared out. Vanilla runs each as
/// its own task on its worker pool; here every thread that needs the rings
/// before they exist takes searches until none are left, then waits for the
/// ones others took. Done by the first thread alone, the 128 stronghold
/// searches kept every other worker blocked for seconds after each world load.
struct RingWork {
    /// The biome source is a `FixedBiomeSource`, which has its own horizontal search.
    fixed: bool,
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
            fixed: false,
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
            let found = if self.fixed {
                search.find_fixed(preferred(*set), biomes)
            } else {
                search.find(preferred(*set), biomes)
            };
            let _ = self.found[i].set(found);
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
        &self.shards[shard_of(key, START_SHARDS)]
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
        let mut bindings = data.feature_block_bindings()?;
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
            ring_seed: None,
            fixed_biome: false,
        }))
    }
    /// `FlatLevelGeneratorSettings.structureOverrides`: only the structure sets named
    /// (`minecraft:villages`) may generate.
    pub(super) fn restrict_to(&mut self, names: &[String]) {
        let registry = &self.registry;
        let bare = |name: &str| name.strip_prefix("minecraft:").unwrap_or(name).to_owned();
        // `lenientOptionalFieldOf`: a list naming a set that does not exist fails to decode,
        // which counts as the key being absent: every set generates.
        if names.iter().any(|name| {
            !registry
                .sets
                .iter()
                .any(|set| bare(&set.name) == bare(name))
        }) {
            return;
        }
        self.possible_sets.retain(|&set| {
            names.iter().any(|name| {
                name.strip_prefix("minecraft:").unwrap_or(name)
                    == registry.sets[set]
                        .name
                        .strip_prefix("minecraft:")
                        .unwrap_or(&registry.sets[set].name)
            })
        });
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
            let mut work = RingWork::new(self.registry.ring_searches(
                &self.possible_sets,
                self.ring_seed.unwrap_or_else(|| context.seed()),
            ));
            work.fixed = self.fixed_biome;
            work
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
        self.build_starts(context, chunk, true)
            .expect("waits for another worker's build")
    }
    /// The starts a chunk saved (`structures.starts`), which vanilla loads with their pieces
    /// (`StructureStart.loadStaticStart`) so that the neighbours place the parts that cross
    /// into them and the beardifier shapes the terrain around them. A jigsaw start (villages,
    /// outposts, bastions, ancient cities, trial chambers, trail ruins) is loaded from its
    /// saved pieces (`PoolPiece::from_saved`). Another structure's pieces have no loader yet:
    /// it is the start this generator makes in the chunk when that one has the same name and the
    /// same pieces' box (the same layout from the same seed), else the name and box alone (found
    /// by `/locate` and maps, but not placed).
    fn saved_starts_at(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Starts {
        let saved = context.saved_starts(chunk);
        let mut generated: Option<Vec<Arc<StructureStart>>> = None;
        let starts = saved
            .into_iter()
            .filter_map(|start| {
                let index = (self.registry.structures.iter()).position(|s| s.name == start.name)?;
                let def = &self.registry.structures[index];
                if !start.pieces.is_empty()
                    && let Some(pieces) = start
                        .pieces
                        .iter()
                        .map(|piece| self.saved_pool_piece(piece))
                        .collect::<Option<Vec<_>>>()
                {
                    return Some(Arc::new(StructureStart::new(index, def, chunk, pieces)));
                }
                if let Some(bounds) = start.bounds {
                    let made = generated.get_or_insert_with(|| {
                        let rings = self.rings_for(context, chunk);
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
                        .collect()
                    });
                    let same = made.iter().find(|made| {
                        made.name == start.name && pieces_box(&made.pieces) == Some(bounds)
                    });
                    if let Some(same) = same {
                        return Some(same.clone());
                    }
                }
                let [x0, y0, z0, x1, y1, z1] = start.bounds.unwrap_or([
                    chunk.x * 16,
                    0,
                    chunk.z * 16,
                    chunk.x * 16 + 15,
                    0,
                    chunk.z * 16 + 15,
                ]);
                Some(Arc::new(StructureStart {
                    structure: index,
                    name: start.name,
                    chunk,
                    pieces: Vec::new(),
                    bounding_box: crate::worldgen::structure::BoundingBox::new(
                        [x0, y0, z0],
                        [x1, y1, z1],
                    ),
                }))
            })
            .collect();
        Arc::new(starts)
    }
    /// A saved `minecraft:jigsaw` piece, or `None` for any other piece or one that does not decode.
    fn saved_pool_piece(&self, piece: &crate::nbt::Tag) -> Option<Piece> {
        let crate::nbt::Tag::Compound(map) = piece else {
            return None;
        };
        let id = map.get(&"id".into())?;
        if !matches!(id, crate::nbt::Tag::String(s) if s.to_string_lossy() == "minecraft:jigsaw") {
            return None;
        }
        PoolPiece::from_saved(&tag_json(piece), &self.resources, &self.bindings)
            .ok()
            .map(Piece::Pool)
    }
    /// `starts_at`, or `None` when another worker is building them and
    /// `wait` is false.
    fn build_starts(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
        wait: bool,
    ) -> Option<Starts> {
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return Some(found);
        }
        if context.is_old_chunk(chunk) {
            // Decoded once: the neighbours ask for these starts again and again.
            let starts = self.saved_starts_at(context, chunk);
            return Some(self.starts.insert_first((chunk.x, chunk.z), starts));
        }
        // Ring searches enlist other workers and wait for them: never hold a
        // chunk-work lock while helping, even when this chunk has no starts.
        let rings = self.rings_for(context, chunk);
        let work = self.start_work.at((chunk.x, chunk.z));
        let _work = match work.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) if !wait => return None,
            Err(_) => work.lock().unwrap(),
        };
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return Some(found);
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
        Some(self.starts.insert_first((chunk.x, chunk.z), starts))
    }
    /// `Structure.generate` for `/place structure`: the start `structure`
    /// builds in `chunk`, whether or not a structure set would place one
    /// there (vanilla calls the structure's own generate, not the set's
    /// placement, with its `biome -> true` predicate). Nothing is cached: the
    /// chunk's own starts are untouched.
    pub(super) fn structure_start(
        &self,
        context: &mut impl StartContext,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<StructureStart> {
        let index = (self.registry.structures.iter()).position(|s| s.name == structure)?;
        try_generate(
            &self.registry,
            &self.resources,
            &self.bindings,
            context,
            index,
            chunk,
            true,
        )
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
    /// its locate position (the start chunk's corner at y 0). `skip` says a
    /// start (structure name, its chunk) can't be referenced again
    /// (`StructureStart.canBeReferenced`): the search goes on past it.
    pub(super) fn locate(
        &self,
        context: &mut impl super::structure::start::StartContext,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
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
                .find(|s| wanted(s.structure) && !skip(&s.name, chunk))
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
                    let mut nearest: Option<(f64, ChunkPos, String)> = None;
                    for &c in chunks {
                        let d = dist([c.x * 16 + 8, 32, c.z * 16 + 8]);
                        if nearest.as_ref().is_none_or(|(n, ..)| d < *n)
                            && self
                                .registry
                                .is_structure_chunk(set, seed, c.x, c.z, &rings)
                            && let Some(name) = found_at(self, context, c)
                        {
                            nearest = Some((d, c, name));
                        }
                    }
                    if let Some((_, c, name)) = nearest {
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
        let around: Vec<ChunkPos> = (chunk.x - 8..=chunk.x + 8)
            .flat_map(|sx| (chunk.z - 8..=chunk.z + 8).map(move |sz| ChunkPos::new(sx, sz)))
            .collect();
        // Workers generating neighbouring chunks ask for the same starts in the
        // same order: build those no other worker is building first, and wait
        // for the others only then, rather than all queueing behind each one.
        let found: Vec<Option<Starts>> = around
            .iter()
            .map(|&at| self.build_starts(context, at, false))
            .collect();
        let mut out = Vec::new();
        for (&at, found) in around.iter().zip(found) {
            let starts = found.unwrap_or_else(|| self.starts_at(context, at));
            for start in starts.iter() {
                let b = start.bounding_box;
                if b.max[0] >= bx && b.min[0] <= bx + 15 && b.max[2] >= bz && b.min[2] <= bz + 15 {
                    out.push(start.clone());
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

/// The union of the pieces' boxes as `[min x, min y, min z, max x, max y, max z]`.
fn pieces_box(pieces: &[Piece]) -> Option<[i32; 6]> {
    let mut union: Option<crate::worldgen::structure::BoundingBox> = None;
    for piece in pieces {
        let b = piece.bounding_box();
        match union.as_mut() {
            Some(u) => u.encapsulate(&b),
            None => union = Some(b),
        }
    }
    union.map(|b| [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]])
}
/// A saved piece's tag as the JSON `PoolPiece::from_saved` reads: compounds and lists as they
/// are, numbers as numbers.
fn tag_json(tag: &crate::nbt::Tag) -> Value {
    use crate::nbt::Tag;
    match tag {
        Tag::Byte(v) => Value::from(i64::from(*v)),
        Tag::Short(v) => Value::from(i64::from(*v)),
        Tag::Int(v) => Value::from(i64::from(*v)),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => Value::from(f64::from(f32::from_bits(*bits))),
        Tag::Double(bits) => Value::from(f64::from_bits(*bits)),
        Tag::String(s) => Value::from(s.to_string_lossy()),
        Tag::List(items) => Value::Array(items.iter().map(tag_json).collect()),
        Tag::ByteArray(v) => Value::Array(v.iter().map(|b| Value::from(i64::from(*b))).collect()),
        Tag::IntArray(v) => Value::Array(v.iter().map(|i| Value::from(i64::from(*i))).collect()),
        Tag::LongArray(v) => Value::Array(v.iter().map(|i| Value::from(*i)).collect()),
        Tag::Compound(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
                .collect(),
        ),
        Tag::End => Value::Null,
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
