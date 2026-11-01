//! Binds `horizon_world`'s vanilla overworld generator (noise terrain,
//! aquifers, surface rules, carvers and placed features) to chunk streaming.
use crate::{
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Section},
    save::SavedProto,
    streaming::{ChunkGenerator, LodSampler, Proto},
};
use horizon_core::{
    paletted::PalettedContainer,
    position::ChunkPos,
    ticks::{ProtoChunkTicks, SavedTick, TickTypeId},
};
use horizon_world::{
    heightmap::HeightmapType,
    loot::PackOverlay,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{
        generator::{
            GeneratorData, GeneratorWorker, OverworldGenerator, ProtoChunk, SurfaceSample,
        },
        overrides::WorldgenOverrides,
        structure::legacy::PieceRandom,
        terrain::PreLightChunkStage,
    },
};
use std::{path::Path, sync::Arc};
mod lod;
mod structure_mobs;
use lod::VanillaLodSampler;
use structure_mobs::StructureMobFinalizer;

/// The references a saved chunk keeps for its structure start: `(structure, chunk x, chunk z)`.
pub type SavedReferences = std::sync::Arc<dyn Fn(&str, i32, i32) -> i32 + Send + Sync>;

pub struct VanillaTerrain {
    generator: OverworldGenerator,
    /// Generator biome index -> client biome table index.
    biome_map: Vec<u16>,
    /// Loot tables for generated containers.
    loot: Option<Arc<horizon_world::loot::LootData>>,
    structure_mobs: Arc<StructureMobFinalizer>,
    /// A worker for structure queries from the game thread.
    query: std::sync::Mutex<Option<GeneratorWorker>>,
    /// Block state id -> block index, the game's block tick type: worldgen
    /// schedules block ticks by default state id.
    tick_blocks: Vec<u32>,
    /// Block index -> default state id: `tick_blocks` the other way.
    tick_states: Vec<u32>,
    /// Block state id -> the block entity type of its block, if it has one.
    block_entity_types: Vec<Option<&'static str>>,
    seed: i64,
    /// The starts (structure name, chunk) an exploration map pointed at.
    referenced: std::sync::Mutex<std::collections::HashSet<(String, i32, i32)>>,
    reference_file: std::sync::Mutex<Option<std::path::PathBuf>>,
    saved_references: std::sync::Mutex<Option<SavedReferences>>,
    /// The world preset the terrain was generated with (`minecraft:normal`
    /// unless a saved world says otherwise); far decoration uses it too.
    preset: String,
    dimension: String,
    /// When the last proto chunk was generated, in milliseconds since
    /// `created`: the ring search waits for a pause in generation.
    last_proto: std::sync::atomic::AtomicU64,
    created: std::time::Instant,
    /// A superflat generator (`FlatLevelSource`).
    flat: bool,
}
/// A proto chunk, or the position of one whose generation failed.
enum VanillaProto {
    Ready(Box<ProtoChunk>),
    Failed(i32, i32),
}
/// WorldGenRegion.setBlock gives every block with a block entity a new
/// one: the blocks nothing placed data for (a sculk patch's sensors) get
/// vanilla's defaults.
fn add_default_block_entities(kinds: &[Option<&str>], out: &mut Chunk, raw: &mut Vec<Tag>) {
    let kind = |v: u32| kinds.get(v as usize).copied().flatten();
    let mut states = [0u32; 4096];
    for (si, section) in out.sections.iter().enumerate() {
        if !section.blocks.maybe_has(|v| kind(v).is_some()) {
            continue;
        }
        section.unpack(&mut states);
        for (i, &v) in states.iter().enumerate() {
            let Some(id) = kind(v) else {
                continue;
            };
            let i = i as i32;
            let pos = [
                out.x * 16 + (i & 15),
                out.min_y() + si as i32 * 16 + (i >> 8),
                out.z * 16 + ((i >> 4) & 15),
            ];
            let at = |t: &Tag| {
                ["x", "y", "z"]
                    .iter()
                    .zip(pos)
                    .all(|(k, p)| t.get(k) == Some(&Tag::Int(p)))
            };
            if raw.iter().any(at) || out.block_entities.iter().any(|(p, _)| *p == pos.into()) {
                continue;
            }
            let tag = Tag::compound([
                ("id", Tag::String(id.into())),
                ("x", Tag::Int(pos[0])),
                ("y", Tag::Int(pos[1])),
                ("z", Tag::Int(pos[2])),
            ]);
            let tag = crate::block_entity_nbt::canonical(&tag).unwrap_or(tag);
            match crate::save::decode_block_entity(&tag) {
                Some(modelled) => out.block_entities.push(modelled),
                None => raw.push(tag),
            }
        }
    }
}

impl VanillaTerrain {
    fn client_surface(&self, surface: SurfaceSample) -> crate::lod::Surface {
        crate::lod::Surface {
            height: surface.height,
            state: surface.state,
            water: surface.water,
            biome: self
                .biome_map
                .get(surface.biome.0 as usize)
                .copied()
                .unwrap_or(0),
        }
    }
    pub fn new(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
    ) -> crate::Result<Self> {
        Self::for_dimension(dir, registry, biomes, seed, "minecraft:overworld")
    }
    pub fn for_dimension(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        dimension: &str,
    ) -> crate::Result<Self> {
        Self::for_world(
            dir,
            registry,
            biomes,
            seed,
            "minecraft:normal",
            dimension,
            true,
        )
    }
    /// A dimension of a world preset (`worldgen/world_preset`) whose
    /// `generate_structures` is `structures` (`ChunkGenerator.createStructures`
    /// and `findNearestMapStructure` do nothing without).
    pub fn for_world(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> crate::Result<Self> {
        let none = WorldgenOverrides::default();
        Self::for_world_with(
            dir, &none, registry, biomes, seed, preset, dimension, structures,
        )
    }
    /// `for_world` with the world's data packs over the bundle at `dir`
    /// (`datapack::worldgen_overrides`).
    pub fn for_world_with(
        dir: &Path,
        overrides: &WorldgenOverrides,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_data_with(dir, overrides, registry)?;
        let generator = OverworldGenerator::for_world(data, seed, preset, dimension, structures)?;
        Self::assemble(dir, registry, biomes, generator, seed, preset, dimension)
    }
    /// A superflat overworld with the structures its settings allow
    /// (`FlatLevelSource`: its layers, `structure_overrides` and the fixed biome).
    #[allow(clippy::too_many_arguments)]
    pub fn for_flat(
        dir: &Path,
        overrides: &WorldgenOverrides,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        flat: &horizon_world::worldgen::generator::FlatSettings,
        structures: bool,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_data_with(dir, overrides, registry)?;
        let generator = OverworldGenerator::flat(data, seed, flat, structures)?;
        let mut terrain = Self::assemble(
            dir,
            registry,
            biomes,
            generator,
            seed,
            "minecraft:normal",
            "minecraft:overworld",
        )?;
        terrain.flat = true;
        Ok(terrain)
    }
    /// Whether this is a superflat generator, whose `spawnOriginalMobs` places nothing.
    pub fn is_flat(&self) -> bool {
        self.flat
    }
    /// The terrain around a generator, with the tables the client's chunks need.
    fn assemble(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        generator: OverworldGenerator,
        seed: i64,
        preset: &str,
        dimension: &str,
    ) -> crate::Result<Self> {
        let biome_map = generator
            .data()
            .biome_names()
            .iter()
            .map(|name| biomes.id(name))
            .collect();
        if let Some((asked, kept)) = generator.noise_clamped() {
            log::warn!(
                "Horizon: {dimension}'s noise settings reach y {}..{}, past its build height; \
                 only y {}..{} generates",
                asked.min_y,
                asked.min_y + asked.height,
                kept.min_y,
                kept.min_y + kept.height
            );
        }
        let unsupported = generator.unsupported_features();
        if !unsupported.is_empty() {
            log::warn!(
                "Horizon: {} placed features cannot run and will be skipped",
                unsupported.len()
            );
            if !dir.join("blocks/block-properties.nbt").exists() {
                log::warn!(
                    "Horizon: the worldgen bundle lacks the block behavior exports (worldgen/blocks/*.nbt); \
                     prepare again (the launcher, or horizon-prepare --prepare) to add them"
                );
            }
        }
        let loot = dir
            .parent()
            .and_then(|root| crate::structure_blocks::loot_data(root, &PackOverlay::NONE));
        if let Some(root) = dir.parent() {
            crate::mobs::trial_spawner::load_registry(root);
        }
        let mut tick_blocks = Vec::with_capacity(registry.len());
        let mut tick_states = Vec::new();
        let mut block_entity_types = Vec::with_capacity(registry.len());
        let mut last: Option<&str> = None;
        let mut index = 0u32;
        for id in 0..registry.len() {
            let name = registry
                .state(horizon_world::registry::BlockStateId(id as u32))
                .map_or("", |s| s.name.as_str());
            if last.is_some_and(|l| l != name) {
                index += 1;
            }
            if last != Some(name) {
                tick_states.push(registry.default_state(name).map_or(id as u32, |s| s.0));
            }
            last = Some(name);
            tick_blocks.push(index);
            block_entity_types.push(crate::lan::chunks::block_entity_type(name));
        }
        Ok(Self {
            generator,
            biome_map,
            loot,
            structure_mobs: Arc::new(StructureMobFinalizer::new(
                dir.parent().unwrap_or(dir).to_owned(),
                dimension,
            )),
            query: std::sync::Mutex::new(None),
            tick_blocks,
            tick_states,
            block_entity_types,
            seed,
            referenced: Default::default(),
            reference_file: Default::default(),
            saved_references: Default::default(),
            preset: preset.to_owned(),
            dimension: dimension.to_owned(),
            last_proto: std::sync::atomic::AtomicU64::new(0),
            created: std::time::Instant::now(),
            flat: false,
        })
    }
    /// The column the dimension's chunks hold (its build range's).
    fn layout(&self) -> ColumnLayout {
        let (min_y, height) = self.generator.level();
        ColumnLayout::of_level(min_y, height)
    }
    /// The difficulty (0 peaceful to 3 hard) and overworld clock structure
    /// mobs are finalized with.
    pub fn set_spawn_conditions(&self, difficulty: u8, day_time: i64) {
        self.structure_mobs.set_conditions(difficulty, day_time);
    }
    /// The random a chunk's generation steps draw from (`WorldGenRegion`).
    pub fn region_random(&self, cx: i32, cz: i32) -> Box<dyn PieceRandom> {
        self.generator.region_random(ChunkPos::new(cx, cz))
    }
    /// A mob chunk generation spawns, as the chunk saves it
    /// (`StructureMobFinalizer::spawn_generated`).
    pub fn spawn_generated(
        &self,
        kind: &str,
        position: horizon_core::geometry::Vec3,
        yaw: f32,
        biome: &str,
        random: &mut dyn PieceRandom,
        group: &mut Option<crate::mobs::finalize::Group>,
    ) -> Option<horizon_world::nbt::Compound> {
        self.structure_mobs
            .spawn_generated(kind, position, yaw, biome, random, group)
    }
    /// Only the generated range of a proto chunk (the Nether is 0..256)
    /// is copied into a column.
    fn generated_range(chunk: &ProtoChunk, out: &Chunk) -> (i32, i32) {
        (
            chunk.terrain.min_y().max(out.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(out.max_y()),
        )
    }
    /// A proto chunk's blocks alone as a column: what a light job reads of
    /// its neighbours.
    fn block_column(&self, chunk: &ProtoChunk) -> Chunk {
        let pos = chunk.position();
        let mut out = Chunk::with_layout(pos.x, pos.z, self.layout());
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        let (lo, hi) = Self::generated_range(chunk, &out);
        let mut values = Box::new([0u32; 4096]);
        for si in 0..out.sections.len() as i32 {
            let base = out.min_y() + si * 16;
            if base + 16 <= lo || base >= hi {
                continue;
            }
            let mut any = false;
            // A whole generated section is read in one pass (it runs on the
            // server thread for every light view); a partly generated one
            // block by block.
            if base >= lo && base + 16 <= hi && chunk.terrain.section_states(base, &mut values) {
                any = values.iter().any(|&id| id != 0);
            } else {
                values.fill(0);
                for y in base.max(lo)..(base + 16).min(hi) {
                    for z in 0..16 {
                        for x in 0..16 {
                            let id = chunk.block(bx + x, y, bz + z).0;
                            values[((y & 15) << 8 | z << 4 | x) as usize] = id;
                            any |= id != 0;
                        }
                    }
                }
            }
            if any {
                Arc::make_mut(&mut out.sections[si as usize]).blocks =
                    Section::from_blocks(&values);
            }
        }
        out
    }
    /// A proto chunk's blocks, biomes, scheduled ticks and post-processing
    /// marks as a column.
    fn column(&self, chunk: &ProtoChunk) -> Chunk {
        let mut out = self.block_column(chunk);
        let pos = chunk.position();
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        let (lo, hi) = Self::generated_range(chunk, &out);
        out.structure_refs = chunk
            .structures()
            .into_iter()
            .map(|(id, start, _, _)| (Arc::from(id), (start.x, start.z)))
            .collect();
        out.structure_starts = chunk.own_starts();
        // Ticks the features scheduled (leaves reshaped at a tree's edge,
        // fluids of lakes and springs) run once the chunk is live, as
        // LevelChunk takes over ProtoChunk's ticks.
        let block_ticks: Vec<_> = chunk
            .terrain
            .block_ticks()
            .pack(0)
            .iter()
            .filter_map(|t| {
                let kind = *self.tick_blocks.get(t.kind.0 as usize)?;
                Some(horizon_core::ticks::SavedTick {
                    kind: horizon_core::ticks::TickTypeId(kind),
                    ..*t
                })
            })
            .collect();
        let fluid_ticks = chunk.terrain.fluid_ticks().pack(0).to_vec();
        if !block_ticks.is_empty() || !fluid_ticks.is_empty() {
            out.saved = Some(Arc::new(crate::save::ChunkExtras {
                block_ticks,
                fluid_ticks,
                ..Default::default()
            }));
        }
        let base_section = chunk.terrain.min_y() >> 4;
        for (i, marks) in chunk.terrain.postprocessing().iter().enumerate() {
            let sy = (base_section + i as i32) * 16;
            for &packed in marks {
                let p = i32::from(packed);
                let y = sy + ((p >> 4) & 15);
                if (lo..hi).contains(&y) {
                    out.postprocess
                        .push([bx + (p & 15), y, bz + ((p >> 8) & 15)]);
                }
            }
        }
        for si in 0..out.sections.len() as i32 {
            let mut cells = [0u32; 64];
            for (j, cell) in cells.iter_mut().enumerate() {
                let (qx, qz, qy) = (j as i32 & 3, (j as i32 >> 2) & 3, si * 4 + (j as i32 >> 4));
                let y = (out.min_y() + qy * 4).clamp(lo, hi - 1);
                let id = chunk.biome_id_at(bx + qx * 4, y, bz + qz * 4);
                *cell = u32::from(self.biome_map.get(id.0 as usize).copied().unwrap_or(0));
            }
            Arc::make_mut(&mut out.sections[si as usize]).biomes =
                PalettedContainer::from_values(BIOME_STRATEGY, &cells);
        }
        out
    }
    /// The structure starts owned by a chunk (cached by the generator).
    pub fn starts_at(
        &self,
        x: i32,
        z: i32,
    ) -> Vec<Arc<horizon_world::worldgen::structure::start::StructureStart>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.structure_starts(ChunkPos::new(x, z))
    }
    /// `/place structure`: `Structure.generate` for `structure` in chunk
    /// `(x, z)`, whatever the chunk's structure sets place there.
    pub fn structure_start(
        &self,
        structure: &str,
        x: i32,
        z: i32,
    ) -> Option<Arc<horizon_world::worldgen::structure::start::StructureStart>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.structure_start(structure, ChunkPos::new(x, z))
    }
    /// `/locate structure`: the nearest start of one of `names` within
    /// `radius` placement sectors of block `from`.
    pub fn locate(
        &self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.generator.worker().locate(names, from, radius)
    }
    /// The chunks an older version saved, which new chunks blend with
    /// (`Blender.of`).
    pub fn set_old_world(&self, world: Arc<dyn horizon_world::worldgen::blending::OldWorld>) {
        self.generator.set_old_world(world);
    }
    /// The generator's resources, for what reads saved chunks the way it does.
    pub fn generator_data(&self) -> &Arc<GeneratorData> {
        self.generator.data()
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: F3's `Density` line at
    /// the feet. It reads the samplers only, not the shared query worker, so
    /// a long `/locate biome` or `/place jigsaw` never holds it up.
    pub fn density_debug_line(&self, feet: [i32; 3]) -> Option<String> {
        self.generator.debug_density_line(feet)
    }
    /// `ChunkGenerator.findNearestMapStructure` for an exploration map: with
    /// `skip_known` (`skip_existing_chunks`) the nearest start no map pointed
    /// at yet, which then counts as pointed at (`StructureStart.addReference`,
    /// at most one: `getMaxReferences`).
    pub fn locate_for_map(
        &self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip_known: bool,
    ) -> Option<(String, [i32; 3])> {
        if !skip_known {
            return self.locate(names, from, radius);
        }
        let saved = self
            .saved_references
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut referenced = self.referenced.lock().unwrap_or_else(|e| e.into_inner());
        let found =
            self.generator
                .worker()
                .locate_skipping(names, from, radius, &|name, chunk| {
                    referenced.contains(&(name.to_owned(), chunk.x, chunk.z))
                        || saved
                            .as_ref()
                            .is_some_and(|f| f(name, chunk.x, chunk.z) > 0)
                })?;
        referenced.insert((found.0.clone(), found.1[0] >> 4, found.1[2] >> 4));
        self.save_references(&referenced);
        Some(found)
    }
    /// Where the starts maps pointed at are kept, and the ones kept there:
    /// vanilla keeps the count in each start's chunk (`references`), which a
    /// start nobody visited has no file for.
    pub fn set_reference_file(&self, path: std::path::PathBuf) {
        let mut referenced = self.referenced.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(String, i32, i32)>>(&b).ok())
        {
            referenced.extend(list);
        }
        *self
            .reference_file
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(path);
    }
    /// StructureCheck's `onStructureLoad` and `tryLoadFromStorage`: the
    /// references a chunk saved for its starts (a world saved by vanilla, or
    /// by an older run) count as known, besides the ones kept here.
    pub fn set_saved_references(&self, saved: SavedReferences) {
        *self
            .saved_references
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(saved);
    }
    fn save_references(&self, referenced: &std::collections::HashSet<(String, i32, i32)>) {
        let guard = self
            .reference_file
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(path) = guard.as_ref() else { return };
        let mut list: Vec<_> = referenced.iter().collect();
        list.sort();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written beside the file and renamed over it: a crash half way must
        // not leave truncated JSON, which loads as no references at all.
        let tmp = path.with_extension("json.tmp");
        let saved = serde_json::to_vec(&list)
            .map_err(std::io::Error::from)
            .and_then(|b| std::fs::write(&tmp, b))
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = saved {
            log::warn!("Horizon: could not save {}: {e}", path.display());
        }
    }
    /// `/locate biome`: `BiomeSource.findClosestBiome3d`.
    pub fn find_closest_biome(
        &self,
        search: &horizon_world::worldgen::generator::BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.find_closest_biome(search)
    }
    /// `BiomeSource.getNoiseBiome` at quart positions, for places that may not
    /// be generated yet (`MapItem.renderBiomePreviewMap`).
    pub fn noise_biomes(&self, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.noise_biomes(quarts)
    }
    /// `/place jigsaw`: `JigsawPlacement.addPieces`.
    pub fn jigsaw_pieces(
        &self,
        query: &horizon_world::worldgen::generator::JigsawQuery<'_>,
    ) -> Option<horizon_world::worldgen::generator::JigsawPieces> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.jigsaw_pieces(query)
    }
    /// NoiseBasedChunkGenerator.getOrigin: where the world spawn search starts.
    pub fn origin(&self) -> crate::Result<ChunkPos> {
        Ok(self.generator.origin()?)
    }
    /// Concentric ring positions of a structure set (strongholds), searched
    /// for now if no chunk needed them yet.
    pub fn ring_positions(&self, set: &str) -> Option<Vec<ChunkPos>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.ring_positions(set)
    }
    /// Searches the concentric rings on a thread of their own, as vanilla's
    /// background executor does, once generation pauses for two seconds
    /// (at the latest a minute after the world opens) and in the idle
    /// class: on a laptop's few cores, sharing them with the first chunks,
    /// which don't need the rings, made those 3 s slower. An eye of ender
    /// thrown later finds the rings ready, and a chunk or a query that
    /// needs them first shares the remaining searches with this thread.
    pub fn search_rings_behind(self: &Arc<Self>) {
        const QUIET: u64 = 2_000;
        const LATEST: u64 = 60_000;
        let terrain = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name("Ring search".into())
            .spawn(move || {
                horizon_threads::idle_current();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    // The world closed: nothing to search for.
                    let Some(terrain) = terrain.upgrade() else {
                        return;
                    };
                    let now = terrain.elapsed_ms();
                    let last = terrain
                        .last_proto
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if now >= LATEST || (last > 0 && now - last >= QUIET) {
                        let _ = terrain
                            .generator
                            .worker()
                            .ring_positions("minecraft:strongholds");
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("ring search thread: {e}");
        }
    }
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.created.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}
/// A bundle directory, the data packs over it, and what loading them gave.
type SharedEntry = (std::path::PathBuf, WorldgenOverrides, Arc<GeneratorData>);
static SHARED_DATA: std::sync::Mutex<Vec<SharedEntry>> = std::sync::Mutex::new(Vec::new());
/// Bundles with data packs kept besides the plain ones: a world's dimensions
/// share one, and returning to a world finds it.
const SHARED_WITH_PACKS: usize = 2;
/// Let go of the bundles `shared_data` loaded: for a preload no world used
/// (game/world_loading.rs). A generator that holds one keeps it.
pub fn forget_shared_data() {
    SHARED_DATA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}
/// The worldgen bundle at `dir`, loaded once and shared by every generator
/// and by live feature placement (sapling growth, bone meal).
pub fn shared_data(
    dir: &Path,
    registry: &'static BlockRegistry,
) -> crate::Result<Arc<GeneratorData>> {
    shared_data_with(dir, &WorldgenOverrides::default(), registry)
}
/// `shared_data` with a world's data packs over the bundle: loaded once for
/// each set of packs' entries, the most recent few kept.
pub fn shared_data_with(
    dir: &Path,
    overrides: &WorldgenOverrides,
    registry: &'static BlockRegistry,
) -> crate::Result<Arc<GeneratorData>> {
    let mut cache = SHARED_DATA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, _, data)) = cache
        .iter()
        .find(|(path, o, _)| path == dir && o == overrides)
    {
        return Ok(data.clone());
    }
    let started = std::time::Instant::now();
    let data = GeneratorData::load_with(dir, registry, overrides)?;
    log::info!("Horizon: worldgen data loaded in {:?}", started.elapsed());
    for warning in data.warnings() {
        log::warn!("Horizon: data pack worldgen: {warning}");
    }
    if !overrides.is_empty()
        && cache.iter().filter(|e| !e.1.is_empty()).count() >= SHARED_WITH_PACKS
    {
        let oldest = cache.iter().position(|e| !e.1.is_empty());
        cache.remove(oldest.expect("counted"));
    }
    cache.push((dir.to_owned(), overrides.clone(), data.clone()));
    Ok(data)
}
impl ChunkGenerator for VanillaTerrain {
    type Worker = GeneratorWorker;
    fn worker(&self) -> GeneratorWorker {
        let mut worker = self.generator.worker();
        worker.set_structure_mobs(self.structure_mobs.clone());
        worker
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        Box::new(VanillaLodSampler::new(self))
    }
    fn idle(&self, worker: &mut GeneratorWorker) {
        worker.release();
    }
    fn lod_surface(
        &self,
        worker: &mut GeneratorWorker,
        x: i32,
        z: i32,
    ) -> Option<crate::lod::Surface> {
        Some(self.client_surface(worker.surface_sample(x, z)?))
    }
    /// The structure starts a saved chunk references, as vanilla reads them
    /// with the chunk: built here, on the loading worker, into the
    /// generator's cache, rather than on the game thread the first time
    /// spawning or a mob asks which structures hold a position (a village
    /// start took 70 to 110 ms there, one tick each, after a world loaded).
    fn prepare_loaded(&self, worker: &mut GeneratorWorker, chunk: &Chunk) {
        let mut starts: Vec<(i32, i32)> = chunk.structure_refs.iter().map(|(_, at)| *at).collect();
        starts.sort_unstable();
        starts.dedup();
        for (x, z) in starts {
            worker.structure_starts(ChunkPos::new(x, z));
        }
    }
    fn proto(&self, worker: &mut GeneratorWorker, x: i32, z: i32) -> Proto {
        self.last_proto.store(
            self.elapsed_ms().max(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        match worker.generate_proto(ChunkPos::new(x, z)) {
            Ok(chunk) => Box::new(VanillaProto::Ready(Box::new(chunk))),
            Err(e) => {
                log::warn!("Horizon: generating chunk {x} {z} failed: {e}");
                Box::new(VanillaProto::Failed(x, z))
            }
        }
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorates_beside_final(&self) -> bool {
        true
    }
    /// A final chunk's proto chunk: terrain generated again with the final
    /// chunk's blocks over it, so that features read and write it as they do
    /// the chunks around it (`ChunkAccess` of a full chunk in the region).
    fn proto_beside_final(&self, worker: &mut GeneratorWorker, snapshot: &Chunk) -> Option<Proto> {
        let mut proto = self.proto(worker, snapshot.x, snapshot.z);
        self.restore_proto(
            &mut proto,
            SavedProto {
                column: snapshot.clone(),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            },
        );
        Some(proto)
    }
    fn final_writes(&self, snapshot: &Chunk, proto: &Proto) -> Vec<([i32; 3], u32)> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return Vec::new();
        };
        let (bx, bz) = (snapshot.x * 16, snapshot.z * 16);
        let (lo, hi) = (
            chunk.terrain.min_y().max(snapshot.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(snapshot.max_y()),
        );
        let mut writes = Vec::new();
        for y in lo..hi {
            for z in 0..16 {
                for x in 0..16 {
                    let state = chunk.block(bx + x, y, bz + z).0;
                    if state != snapshot.get(x, y, z) {
                        writes.push(([bx + x, y, bz + z], state));
                    }
                }
            }
        }
        writes
    }
    fn final_block_entities(
        &self,
        proto: &Proto,
        writes: &[([i32; 3], u32)],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return Vec::new();
        };
        if chunk.block_entities.is_empty() {
            return Vec::new();
        }
        let (made, _rolled) = crate::structure_blocks::block_entities(chunk, self.loot.as_deref());
        made.into_iter()
            .filter(|((x, y, z), _)| writes.iter().any(|(p, _)| *p == [*x, *y, *z]))
            .collect()
    }
    fn decorate(&self, worker: &mut GeneratorWorker, region: &mut [Proto; 9]) {
        let taken: Vec<Box<VanillaProto>> = region
            .iter_mut()
            .map(|p| {
                std::mem::replace(p, Box::new(()))
                    .downcast::<VanillaProto>()
                    .expect("vanilla proto")
            })
            .collect();
        let all_ready = taken.iter().all(|p| matches!(**p, VanillaProto::Ready(_)));
        if all_ready {
            let chunks: Vec<ProtoChunk> = taken
                .into_iter()
                .map(|p| match *p {
                    VanillaProto::Ready(c) => *c,
                    VanillaProto::Failed(..) => unreachable!(),
                })
                .collect();
            let mut array: [ProtoChunk; 9] = match chunks.try_into() {
                Ok(a) => a,
                Err(_) => unreachable!(),
            };
            if let Err(e) = worker.decorate(&mut array) {
                log::warn!("Horizon: decorating chunk failed: {e}");
            }
            for (slot, chunk) in region.iter_mut().zip(array) {
                *slot = Box::new(VanillaProto::Ready(Box::new(chunk)));
            }
        } else {
            for (slot, p) in region.iter_mut().zip(taken) {
                *slot = p;
            }
        }
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        matches!(proto.downcast_ref::<VanillaProto>(), Some(VanillaProto::Ready(c)) if c.is_decorated())
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        match proto.downcast_ref::<VanillaProto>() {
            Some(VanillaProto::Ready(chunk)) => Some(self.block_column(chunk)),
            _ => None,
        }
    }
    fn non_air_sections(&self, proto: &Proto) -> Option<crate::section_mask::SectionMask> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return None;
        };
        // The proto's sections from its own minimum Y; the column's from the
        // layout's.
        let layout = self.layout();
        let below = (chunk.terrain.min_y() - layout.min_y()) >> 4;
        let mut mask = crate::section_mask::SectionMask::EMPTY;
        for i in chunk.terrain.filled_sections() {
            if let Ok(i) = usize::try_from(i as i32 + below)
                && i < layout.sections()
            {
                mask.insert(i);
            }
        }
        Some(mask)
    }
    fn saved_proto(&self, proto: &Proto) -> Option<SavedProto> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return None;
        };
        let block_entities = chunk
            .block_entities
            .iter()
            .map(|(pos, nbt)| {
                let mut nbt = nbt.clone();
                // Placed data can lack its type (a brushable block's loot table
                // alone); vanilla saves every pending block entity with its id,
                // and skips one without when the chunk finishes.
                if let Some(id) =
                    crate::lan::chunks::block_entity_type(chunk.block_name(pos[0], pos[1], pos[2]))
                {
                    nbt.insert("id".into(), Tag::String(id.into()));
                }
                for (key, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    nbt.insert(key.into(), Tag::Int(*v));
                }
                Tag::Compound(nbt)
            })
            .collect();
        Some(SavedProto {
            column: self.column(chunk),
            decorated: chunk.is_decorated(),
            block_entities,
            entities: chunk.entities.iter().cloned().map(Tag::Compound).collect(),
        })
    }
    /// The saved state replaces what decoration can change; biomes, structure
    /// starts and references are generated again as they were.
    fn restore_proto(&self, proto: &mut Proto, saved: SavedProto) {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_mut::<VanillaProto>() else {
            return;
        };
        let column = &saved.column;
        let (bx, bz) = (column.x * 16, column.z * 16);
        let (lo, hi) = (
            chunk.terrain.min_y().max(column.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(column.max_y()),
        );
        for y in lo..hi {
            for z in 0..16 {
                for x in 0..16 {
                    let state = column.block(x, y, z);
                    if chunk.block(bx + x, y, bz + z) != state {
                        let _ = chunk.terrain.set_block_raw(bx + x, y, bz + z, state);
                    }
                }
            }
        }
        if saved.decorated {
            chunk
                .terrain
                .set_generation_stage(PreLightChunkStage::Features);
            chunk.set_decorated();
        }
        // Heightmaps follow the blocks (ProtoChunk primes missing ones on read).
        let stage = chunk.terrain.generation_stage().heightmaps();
        let kinds: Vec<HeightmapType> = HeightmapType::ALL
            .into_iter()
            .filter(|&k| chunk.terrain.heightmap(k).is_some() || stage.contains(&k))
            .collect();
        let _ = chunk.terrain.prime_heightmaps(&kinds);
        let ticks = |ticks: &[SavedTick], kind: &dyn Fn(u32) -> Option<u32>| {
            ProtoChunkTicks::from_saved(ticks.iter().filter_map(|t| {
                Some(SavedTick {
                    kind: TickTypeId(kind(t.kind.0)?),
                    ..*t
                })
            }))
        };
        if let Some(extras) = column.saved.as_deref() {
            *chunk.terrain.block_ticks_mut() = ticks(&extras.block_ticks, &|k| {
                self.tick_states.get(k as usize).copied()
            });
            *chunk.terrain.fluid_ticks_mut() = ticks(&extras.fluid_ticks, &Some);
        }
        chunk.terrain.clear_postprocessing();
        for &[x, y, z] in &column.postprocess {
            chunk.terrain.mark_postprocessing(x, y, z);
        }
        chunk.block_entities = saved
            .block_entities
            .into_iter()
            .filter_map(|tag| {
                let Tag::Compound(mut nbt) = tag else {
                    return None;
                };
                let mut coordinate = |key: &str| match nbt.remove(&key.into()) {
                    Some(Tag::Int(v)) => Some(v),
                    _ => None,
                };
                let pos = [coordinate("x")?, coordinate("y")?, coordinate("z")?];
                Some((pos, nbt))
            })
            .collect();
        chunk.entities = saved
            .entities
            .into_iter()
            .filter_map(|tag| match tag {
                Tag::Compound(nbt) => Some(nbt),
                _ => None,
            })
            .collect();
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let proto = proto.downcast::<VanillaProto>().expect("vanilla proto");
        let mut chunk = match *proto {
            VanillaProto::Ready(c) => c,
            VanillaProto::Failed(x, z) => return Chunk::with_layout(x, z, self.layout()),
        };
        let mut out = self.column(&chunk);
        let (block_entities, rolled_loot) =
            crate::structure_blocks::block_entities(&chunk, self.loot.as_deref());
        out.block_entities = block_entities;
        // The block entities Horizon does not model (decorated pots with their
        // sherds and loot table) stay as placed, like a
        // loaded chunk's.
        let mut unmodelled: Vec<Tag> = chunk
            .block_entities
            .iter()
            .filter(|(pos, _)| {
                !out.block_entities
                    .iter()
                    .any(|(p, _)| *p == (pos[0], pos[1], pos[2]))
            })
            .filter_map(|(pos, nbt)| {
                let id = crate::lan::chunks::block_entity_type(
                    chunk.block_name(pos[0], pos[1], pos[2]),
                )?;
                let mut nbt = nbt.clone();
                nbt.insert("id".into(), Tag::String(id.into()));
                for (key, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    nbt.insert(key.into(), Tag::Int(*v));
                }
                Some(Tag::Compound(nbt))
            })
            .collect();
        add_default_block_entities(&self.block_entity_types, &mut out, &mut unmodelled);
        if !unmodelled.is_empty() || !rolled_loot.is_empty() {
            let mut extras = out
                .saved
                .take()
                .map(Arc::unwrap_or_clone)
                .unwrap_or_default();
            extras.block_entities = unmodelled;
            extras.rolled_loot = rolled_loot;
            out.saved = Some(Arc::new(extras));
        }
        let entities = std::mem::take(&mut chunk.entities);
        // Chest minecarts keep their loot table; the game rolls it on load.
        out.entities = entities
            .into_iter()
            .map(horizon_world::nbt::Tag::Compound)
            .collect();
        for s in &mut out.sections {
            Arc::make_mut(s).compact();
        }
        out.fresh = true;
        out
    }
}

#[cfg(test)]
mod default_block_entity_tests {
    use super::*;

    #[test]
    fn generated_blocks_get_their_block_entities() {
        let mut out = Chunk::empty(-2, 3);
        out.set(4, -40, 9, 7);
        out.set(5, -40, 9, 6);
        let mut kinds = vec![None; 8];
        kinds[6] = Some("minecraft:conduit");
        kinds[7] = Some("minecraft:sculk_sensor");
        let mut raw = vec![];
        add_default_block_entities(&kinds, &mut out, &mut raw);
        assert!(matches!(
            out.block_entities[..],
            [((-28, -40, 57), crate::containers::BlockEntity::Sculk(_))]
        ));
        assert_eq!(raw.len(), 1);
        assert_eq!(
            raw[0].get("id"),
            Some(&Tag::String("minecraft:conduit".into()))
        );
        add_default_block_entities(&kinds, &mut out, &mut raw);
        assert_eq!(
            (out.block_entities.len(), raw.len()),
            (1, 1),
            "a placed block entity is kept, not doubled"
        );
    }
}
