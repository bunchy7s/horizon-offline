//! FEATURES status over a 3×3 neighborhood of generated chunks.
//!
//! Placed features are resolved by name from the bundled feature registries.
//! Roots whose algorithm or bindings are not implemented are left out of
//! execution (but keep their place in vanilla's per-step feature ordering, so
//! decoration seeds of the remaining features are unchanged).
use super::structure::{
    BoundingBox,
    start::{STEPS, place_in_chunk},
};
use super::{
    anchor::WorldGenerationContext,
    biome::BiomeId,
    biome_chunk::BiomeRegion,
    biome_predicate::BiomePredicateBindings,
    biome_source::ResolvedBiomeSource,
    biome_temperature::BiomeTemperature,
    biome_weather::BiomeWeatherBindings,
    block_feature::FeatureBlockBindings,
    decoration::{DecorationWorld, FeatureDecorationPlan},
    feature::{FeatureBlocks, FeatureWorld},
    feature_catalog::FeatureCatalog,
    feature_graph::PlacedFeatureDefinition,
    feature_region::{RegionError, TerrainRegion},
    feature_sort::FeatureIdentity,
    generator::{
        DecorateStats, GenBlockStates, GeneratorData, GeneratorError, ProtoChunk, WorkerState,
    },
    placement::{FeaturePlacer, LayerBlock, PlacementContext, PlacementQueryError},
    tags::TagDefinitions,
    terrain::PreLightChunkStage,
    write_zone::GenerationWriteZone,
};
use super::{
    generator_blocks::BlockBehavior,
    state_provider::{StateProviderBindings, StateProviderCatalog},
};
use crate::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    block_placement::BlockPlacementBindings,
    block_support::SupportType,
    block_survival::SurvivalWorld,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidKind, FluidState},
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
};
use horizon_core::random::{WorldgenRandom, XoroshiroRandom};
use horizon_core::{
    biome::obfuscate_seed,
    geometry::{Axis, Direction, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use serde_json::Value;
use std::collections::HashMap;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Decoration {
    plan: FeatureDecorationPlan,
    names: Vec<String>,
    /// Executable roots by identity, or why the root cannot be executed.
    features: Vec<Result<PlacedFeatureDefinition, String>>,
    /// Placed features of each biome's generation settings, by BiomeId.
    biome_features: Vec<BTreeSet<FeatureIdentity>>,
    bindings: FeatureBlockBindings<'static>,
    weather: Option<BiomeWeatherBindings>,
    blocks: FeatureBlocks,
    catalog: FeatureCatalog,
    /// Placed features referenced by name (feature pool elements), decoded lazily.
    named: std::sync::Mutex<HashMap<String, Option<Arc<PlacedFeatureDefinition>>>>,
    /// BlockBehaviour.Properties.postProcess: states whose worldgen writes mark
    /// their own position (mushrooms) or the one above (soul sand, magma).
    postprocess_self: Vec<BlockStateId>,
    postprocess_above: Vec<BlockStateId>,
}
fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}
/// Biomes in vanilla's possible-biome encounter order for this source.
fn possible_biomes(source: &ResolvedBiomeSource) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let ordered: Vec<String> = match source {
        ResolvedBiomeSource::MultiNoise(source) => source
            .parameters()
            .entries()
            .iter()
            .map(|(_, name)| name.clone())
            .collect(),
        ResolvedBiomeSource::End(source) => [
            &source.center,
            &source.highlands,
            &source.midlands,
            &source.islands,
            &source.barrens,
        ]
        .into_iter()
        .cloned()
        .collect(),
        other => other
            .possible_biomes()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    };
    ordered
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}
impl Decoration {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        source: &ResolvedBiomeSource,
    ) -> Result<Option<Self>, GeneratorError> {
        let Some(resources) = &data.feature_resources else {
            return Ok(None);
        };
        let registry = data.registry();
        let mut feature_tags = TagDefinitions::default();
        for (name, json) in &resources.feature_tags {
            feature_tags.apply_json(name, "vanilla", json)?;
        }
        let catalog =
            FeatureCatalog::from_resources(&resources.features, &resources.placed, &feature_tags)?;
        let biomes = Arc::new(BiomePredicateBindings::new(
            &data
                .biome_names()
                .iter()
                .enumerate()
                .map(|(i, name)| (name.to_string(), BiomeId(i as u32)))
                .collect(),
            &data.biome_tags,
        )?);
        let mut bindings =
            FeatureBlockBindings::with_tags(registry, data.integers().clone(), &data.block_tags)?
                .with_fluid_tags(&resources.fluid_tags)?
                .with_biomes(biomes.clone());
        if let Some(structures) = &data.structures {
            bindings = bindings.with_structures(structures.clone());
        }
        if let Some(blocks) = &resources.blocks {
            bindings = bindings
                .with_block_properties(&blocks.properties)?
                .with_state_providers(
                    StateProviderBindings::new(
                        registry,
                        data.integers().clone(),
                        &data.block_tags,
                    )?
                    .with_block_properties(&blocks.properties)?
                    .with_fluid_tags(&resources.fluid_tags)?
                    .with_biomes(biomes),
                    StateProviderCatalog::from_resources(&resources.state_providers)?,
                    BlockPlacementBindings::new(
                        registry,
                        &blocks.properties,
                        &blocks.double_plants,
                        &blocks.mossy_carpets,
                    )?,
                )?;
        }
        let tags = feature_tags
            .build(|name, _| resources.placed.contains_key(name).then(|| name.to_owned()))
            .map_err(|e| resource(format!("feature tags: {e:?}")))?
            .tags;
        let mut ids = BTreeMap::<String, FeatureIdentity>::new();
        let mut names = Vec::new();
        let mut features = Vec::new();
        let mut identity = |name: &str| -> FeatureIdentity {
            *ids.entry(name.to_owned()).or_insert_with(|| {
                let id = FeatureIdentity(names.len() as u32);
                names.push(name.to_owned());
                features.push(
                    catalog
                        .placed_feature_with_blocks(&Value::from(name).to_string(), &bindings)
                        .map_err(|e| e.to_string()),
                );
                id
            })
        };
        let mut biome_features = vec![BTreeSet::new(); data.biome_names().len()];
        let mut plan = Vec::new();
        for name in possible_biomes(source) {
            let id = data
                .biome_id(&name)
                .ok_or_else(|| resource(format!("unknown biome {name}")))?;
            let json: Value = serde_json::from_str(data.biome_json(id))?;
            let mut steps = Vec::new();
            for step in json["features"].as_array().into_iter().flatten() {
                let mut list = Vec::new();
                let entries = match step {
                    Value::String(tag) if tag.starts_with('#') => tags
                        .get(&super::tags::identifier(&tag[1..])?)
                        .cloned()
                        .ok_or_else(|| resource(format!("unknown feature tag {tag}")))?,
                    Value::String(name) => vec![name.clone()],
                    Value::Array(values) => values
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| resource("inline placed feature in biome"))
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(resource("biome feature step")),
                };
                for entry in entries {
                    let feature = identity(&super::tags::identifier(&entry)?);
                    list.push(feature);
                    biome_features[id.0 as usize].insert(feature);
                }
                steps.push(list);
            }
            plan.push((id, steps));
        }
        let plan = FeatureDecorationPlan::new(plan).map_err(|e| resource(e.to_string()))?;
        let weather = BiomeWeatherBindings::new(&bindings).ok();
        let states = |names: &[&str]| -> Vec<BlockStateId> {
            names
                .iter()
                .filter_map(|n| registry.default_state(n))
                .collect()
        };
        let postprocess_self = states(&["minecraft:brown_mushroom", "minecraft:red_mushroom"]);
        let postprocess_above = states(&["minecraft:soul_sand", "minecraft:magma_block"]);
        Ok(Some(Self {
            postprocess_self,
            postprocess_above,
            plan,
            names,
            features,
            biome_features,
            weather,
            blocks: FeatureBlocks::from_registry(registry)?,
            bindings,
            catalog,
            named: std::sync::Mutex::new(HashMap::new()),
        }))
    }
    fn named_feature(&self, name: &str) -> Option<Arc<PlacedFeatureDefinition>> {
        let mut named = self.named.lock().unwrap();
        named
            .entry(name.to_owned())
            .or_insert_with(|| {
                self.catalog
                    .placed_feature_with_blocks(&Value::from(name).to_string(), &self.bindings)
                    .ok()
                    .map(Arc::new)
            })
            .clone()
    }
    /// Vanilla's feature identity and the step-local index/step mixed into
    /// `set_feature_seed`.
    pub(super) fn seed(&self, name: &str) -> Option<(u32, i32, i32)> {
        let identity = self.names.iter().position(|candidate| candidate == name)? as u32;
        let (index, step) = self.plan.seed(FeatureIdentity(identity))?;
        Some((identity, index as i32, step as i32))
    }
    /// Replays a feature's placement chain and the feature itself on a no-op
    /// world. This advances vanilla's random stream but writes no blocks, so a
    /// caller can reproduce the exact origins without generating terrain.
    pub(super) fn origins(
        &self,
        world_seed: i64,
        chunk: ChunkPos,
        mut biome_allows: impl FnMut([i32; 3]) -> bool,
    ) -> Vec<[i32; 3]> {
        let Some((identity, index, step)) = self.seed("minecraft:end_island_decorated") else {
            return Vec::new();
        };
        let Ok(feature) = &self.features[identity as usize] else {
            return Vec::new();
        };
        let blocks = self.blocks;
        let mut random = WorldgenRandom::new(XoroshiroRandom::new(0));
        let seed = random.set_decoration_seed(world_seed, chunk.x * 16, chunk.z * 16);
        random.set_feature_seed(seed, index, step);
        let mut world = OriginWorld {
            air: blocks.air,
            generation: WorldGenerationContext::new(0, 256, 0),
            biome_allows: &mut biome_allows,
        };
        let mut origins = Vec::new();
        let mut placer = FeaturePlacer::new();
        let _ = placer.place_with_biome_check(
            &feature.placement,
            &mut world,
            &mut random,
            [chunk.x * 16, 0, chunk.z * 16],
            FeatureIdentity(identity),
            |world, random, position| {
                origins.push(position);
                feature
                    .feature
                    .place(world, random, position, blocks)
                    .map(|_| true)
                    .map_err(|_| ())
            },
        );
        origins
    }
    /// Placed-feature roots that cannot run, with their decode errors.
    pub(super) fn unsupported(&self) -> Vec<(String, String)> {
        self.names
            .iter()
            .zip(&self.features)
            .filter_map(|(name, feature)| feature.as_ref().err().map(|e| (name.clone(), e.clone())))
            .collect()
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decorate(
        &self,
        data: &Arc<GeneratorData>,
        seed: i64,
        generation: WorldGenerationContext,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
        level_random: &mut dyn super::structure::legacy::PieceRandom,
        structures: Option<&super::generator_structures::StructureState>,
    ) -> Result<DecorateStats, GeneratorError> {
        let center = region[4].position();
        for (i, chunk) in region.iter().enumerate() {
            let expected = ChunkPos::new(
                center.x.wrapping_add(i as i32 % 3 - 1),
                center.z.wrapping_add(i as i32 / 3 - 1),
            );
            if chunk.position() != expected {
                return Err(resource("decoration region is not a 3×3 neighborhood"));
            }
            if (chunk.terrain.generation_stage() as u8) < PreLightChunkStage::Terrain as u8 {
                return Err(resource("decoration requires terrain-stage chunks"));
            }
        }
        if region[4].is_decorated() {
            return Ok(DecorateStats::default());
        }
        let mut chunks = Vec::with_capacity(9);
        let mut biomes = Vec::with_capacity(9);
        let (min_y, height) = (region[4].terrain.min_y(), region[4].terrain.height());
        for chunk in region.iter_mut() {
            // A one-section stand-in while the region holds the terrain: it
            // is swapped back below, and allocating the full height for nine
            // stand-ins per region cost more than some features.
            let empty = super::terrain::TerrainChunk::new(
                chunk.position(),
                min_y,
                16,
                data.registry(),
                data.block_states().clone(),
            )
            .map_err(|e| resource(e.to_string()))?;
            chunks.push(std::mem::replace(&mut chunk.terrain, empty));
            biomes.push(chunk.biomes().clone());
        }
        let terrain = TerrainRegion::new(chunks, generation)
            .and_then(|region| region.with_block_bindings(&self.bindings))
            .map(|region| region.with_world_seed(seed));
        let terrain = match terrain {
            Ok(terrain) => terrain,
            Err(e) => return Err(resource(e.to_string())),
        };
        let structures_only = worker.structures_only;
        let references = region[4].structure_starts.clone();
        let mut block_entities = Vec::new();
        for chunk in region.iter_mut() {
            block_entities.append(&mut chunk.block_entities);
        }
        let mut world = World {
            block_entities,
            entities: Vec::new(),
            level_random,
            structure_mobs: worker.structure_mobs.clone(),
            starts: references.clone(),
            biome_names: data.biome_names(),
            blocks: data
                .feature_resources
                .as_ref()
                .and_then(|r| r.blocks.as_ref()),
            region: terrain,
            biomes: BiomeRegion::new(biomes, obfuscate_seed(seed))
                .map_err(|e| resource(e.to_string()))?,
            zone: GenerationWriteZone::new(center, 1),
            center,
            uniform_biomes: vec![UNKNOWN_BIOME; 9 * (height / 16).max(0) as usize],
            biome_quarts: region[4].biomes().quart_range(),
            decoration: self,
            temperatures: &mut worker.temperatures,
            behaviours: data.behaviours.as_deref(),
            // WorldGenRegion.isOutsideBuildHeight: the level's range, not the noise's.
            build: (min_y, min_y + height),
            shape_ticks: std::cell::RefCell::new(crate::level::Ticks::new(0)),
        };
        let mut stats = DecorateStats::default();
        let blocks = self.blocks;
        // The blocks' facts: `BlockState.isSolidRender` and the rest, the
        // structure pieces ask them of the blocks around them.
        let tree_blocks = self
            .bindings
            .tree_blocks()
            .map_err(|e| resource(e.to_string()))?;
        // ChunkGenerator.getWritableArea: the chunk's minimum Y + 1 to its maximum Y.
        let chunk_box = BoundingBox::new(
            [center.x * 16, min_y + 1, center.z * 16],
            [center.x * 16 + 15, min_y + height - 1, center.z * 16 + 15],
        );
        let mut structure_errors = 0;
        let result = self.plan.decorate(
            &mut world,
            center,
            seed,
            STEPS.len(),
            |world, random, decoration_seed, step| {
                let Some(structures) = structures else {
                    return Ok(());
                };
                let Some(in_step) = structures.by_step.get(step) else {
                    return Ok(());
                };
                for (index, &structure) in in_step.iter().enumerate() {
                    random.set_feature_seed(decoration_seed, index as i32, step as i32);
                    for start in references.iter().filter(|s| s.structure == structure) {
                        let mut place_feature =
                            |world: &mut World<'_>,
                             random: &mut WorldgenRandom<XoroshiroRandom>,
                             name: &str,
                             pos: [i32; 3]| {
                                match self.named_feature(name) {
                                    Some(feature) => feature
                                        .place(world, random, pos, blocks)
                                        .map_err(|e| PlacementQueryError::Failed(e.to_string())),
                                    None => Ok(false),
                                }
                            };
                        if place_in_chunk(
                            start,
                            world,
                            &structures.resources,
                            &tree_blocks,
                            random,
                            chunk_box,
                            &mut place_feature,
                        )
                        .is_err()
                        {
                            structure_errors += 1;
                        }
                    }
                }
                Ok(())
            },
            |world, placer, random, id, origin| {
                if structures_only {
                    return Ok(false);
                }
                let Ok(feature) = &self.features[id.0 as usize] else {
                    stats.unsupported += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                    return Ok(false);
                };
                stats.placed += 1;
                let placed = placer.place_with_biome_check(
                    &feature.placement,
                    world,
                    random,
                    origin,
                    id,
                    |world, random, p| feature.feature.place(world, random, p, blocks),
                );
                if placed.is_err() {
                    stats.failed += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                }
                Ok(placed.unwrap_or(false))
            },
        );
        for (pos, nbt) in std::mem::take(&mut world.block_entities) {
            let index = ((pos[2] >> 4).wrapping_sub(center.z) + 1) * 3
                + (pos[0] >> 4).wrapping_sub(center.x)
                + 1;
            if let Some(chunk) = usize::try_from(index).ok().and_then(|i| region.get_mut(i)) {
                chunk.block_entities.push((pos, nbt));
            }
        }
        // WorldGenRegion.addFreshEntity: into the chunk holding the position.
        for nbt in std::mem::take(&mut world.entities) {
            let Some(at) = block_of(&nbt) else {
                continue;
            };
            let index = ((at[2] >> 4).wrapping_sub(center.z) + 1) * 3
                + (at[0] >> 4).wrapping_sub(center.x)
                + 1;
            if let Some(chunk) = usize::try_from(index).ok().and_then(|i| region.get_mut(i)) {
                chunk.entities.push(nbt);
            }
        }
        for terrain in world.region.into_chunks() {
            let p = terrain.position();
            let index = (p.z.wrapping_sub(center.z) + 1) * 3 + p.x.wrapping_sub(center.x) + 1;
            region[index as usize].terrain = terrain;
        }
        result.map_err(|e| GeneratorError::Stage("features", e.to_string()))?;
        stats.failed += structure_errors;
        region[4]
            .terrain
            .set_generation_stage(PreLightChunkStage::Features);
        region[4].set_decorated();
        Ok(stats)
    }
}

/// `a`, and `b` when it differs.
fn span<T: PartialEq>(a: T, b: T) -> (T, Option<T>) {
    let b = (a != b).then_some(b);
    (a, b)
}
const UNKNOWN_BIOME: u32 = u32::MAX;
const MIXED_BIOMES: u32 = u32::MAX - 1;
impl World<'_> {
    /// The biome of a biome section when all its quarts hold the same one
    /// (and its chunk has biomes); `None` when mixed or outside the region.
    fn uniform_section(&mut self, cx: i32, cz: i32, section: usize) -> Option<BiomeId> {
        let (dx, dz) = (
            i64::from(cx) - i64::from(self.center.x) + 1,
            i64::from(cz) - i64::from(self.center.z) + 1,
        );
        if !(0..3).contains(&dx) || !(0..3).contains(&dz) {
            return None;
        }
        let sections = self.uniform_biomes.len() / 9;
        if section >= sections {
            return None;
        }
        let slot = (dz * 3 + dx) as usize * sections + section;
        if self.uniform_biomes[slot] == UNKNOWN_BIOME {
            let chunk = ChunkPos::new(cx, cz);
            let ready = self.region.chunk(cx, cz).is_ok_and(|c| {
                c.highest_generated_status() >= crate::chunk_status::ChunkStatus::Biomes
            });
            let values = self
                .biomes
                .chunk(chunk)
                .ok()
                .and_then(|c| c.section(section));
            self.uniform_biomes[slot] = match values {
                Some(values) if ready && values.iter().all(|b| *b == values[0]) => values[0].0,
                _ => MIXED_BIOMES,
            };
        }
        match self.uniform_biomes[slot] {
            MIXED_BIOMES => None,
            id => Some(BiomeId(id)),
        }
    }
    /// BiomeManager.getBiome without the zoom when every quart it can pick
    /// lies in biome sections of one and the same biome.
    fn uniform_biome(&mut self, [x, y, z]: [i32; 3]) -> Option<BiomeId> {
        let base = [x, y, z].map(|v| v.wrapping_sub(2) >> 2);
        let (bottom, top) = self.biome_quarts;
        let section = |q: i32| ((q.clamp(bottom, top) - bottom) >> 2) as usize;
        // The chunks and sections holding the quarts base..=base + 1.
        let (x0, x1) = span(base[0] >> 2, base[0].wrapping_add(1) >> 2);
        let (z0, z1) = span(base[2] >> 2, base[2].wrapping_add(1) >> 2);
        let (s0, s1) = span(section(base[1]), section(base[1].wrapping_add(1)));
        let found = self.uniform_section(x0, z0, s0)?;
        for cx in std::iter::once(x0).chain(x1) {
            for cz in std::iter::once(z0).chain(z1) {
                for s in std::iter::once(s0).chain(s1) {
                    if self.uniform_section(cx, cz, s)? != found {
                        return None;
                    }
                }
            }
        }
        Some(found)
    }
}
/// Advances a feature's random stream with no block storage; every write is
/// discarded. Placement modifiers still see the real biome and build bounds.
struct OriginWorld<'a> {
    air: BlockStateId,
    generation: WorldGenerationContext,
    biome_allows: &'a mut dyn FnMut([i32; 3]) -> bool,
}
impl FeatureWorld for OriginWorld<'_> {
    type Error = ();
    fn block(&mut self, _: [i32; 3]) -> Result<BlockStateId, ()> {
        Ok(self.air)
    }
    fn set_block(&mut self, _: [i32; 3], _: BlockStateId, _: i32) -> Result<bool, ()> {
        Ok(false)
    }
}
impl PlacementContext for OriginWorld<'_> {
    fn generation(&self) -> WorldGenerationContext {
        self.generation
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.generation.min_y)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Ok(self.generation.min_y + self.generation.height)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.generation.min_y + self.generation.height - 1)
    }
    fn biome_allows_feature(
        &mut self,
        position: [i32; 3],
        _: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        Ok((self.biome_allows)(position))
    }
}

/// WorldGenRegion-style view: reads anywhere in the 3×3 region, writes within
/// the FEATURES step's write radius, biome lookups through the biome zoom.
struct World<'a> {
    block_entities: Vec<([i32; 3], crate::nbt::Compound)>,
    entities: Vec<crate::nbt::Compound>,
    level_random: &'a mut dyn super::structure::legacy::PieceRandom,
    structure_mobs: Option<Arc<dyn super::generator::StructureMobs>>,
    starts: Vec<Arc<super::structure::start::StructureStart>>,
    biome_names: &'a [Arc<str>],
    blocks: Option<&'a BlockBehavior>,
    region: TerrainRegion<'static, GenBlockStates>,
    biomes: BiomeRegion<BiomeId>,
    zone: GenerationWriteZone,
    center: ChunkPos,
    /// Per biome section of the nine chunks (chunk-major): its one biome id,
    /// MIXED_BIOMES, or UNKNOWN_BIOME until first asked.
    uniform_biomes: Vec<u32>,
    /// The quart Ys biome lookups clamp to (`BiomeChunk::quart_range`).
    biome_quarts: (i32, i32),
    decoration: &'a Decoration,
    temperatures: &'a mut [BiomeTemperature],
    behaviours: Option<&'a crate::level::Behaviours>,
    /// Inclusive minimum and exclusive maximum build height.
    build: (i32, i32),
    /// The tick capture `shape_update` passes to `updateShape`.
    shape_ticks: std::cell::RefCell<crate::level::Ticks>,
}
/// Read-only `LevelAccess` view of the decoration region for
/// `BlockState.updateShape` (writes go through `World::set_block`).
struct ShapeLevel<'w, 'a> {
    world: &'w World<'a>,
    random: horizon_core::random::LegacyRandom,
}
impl crate::level::LevelAccess for ShapeLevel<'_, '_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world
            .region
            .block([pos.x, pos.y, pos.z])
            .unwrap_or(BlockStateId(0))
    }
    fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.region.block([pos.x, pos.y, pos.z]).is_ok()
    }
    fn min_y(&self) -> i32 {
        self.world.build.0
    }
    fn max_y(&self) -> i32 {
        self.world.build.1
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        0
    }
    /// Lighting has not run during FEATURES: the light engine has no data for
    /// these chunks, so sky light reads zero too (MushroomBlock.canSurvive).
    fn sky_light(&self, _: BlockPos) -> i32 {
        0
    }
}
impl World<'_> {
    /// `BlockState.updateShape` of the state at `pos` against `neighbor`.
    fn shape_update(
        &self,
        behaviours: &crate::level::Behaviours,
        state: BlockStateId,
        pos: [i32; 3],
        direction: Direction,
        neighbor_pos: [i32; 3],
        neighbor: BlockStateId,
    ) -> (BlockStateId, Vec<crate::level::ticks::CapturedTick>) {
        let mut level = ShapeLevel {
            world: self,
            random: horizon_core::random::LegacyRandom::new(0),
        };
        // The ticks updateShape schedules (LeavesBlock re-measuring its
        // distance, waterlogged blocks' fluid) go to the proto chunks. While
        // capturing, scheduling leaves the rest of `Ticks` untouched, so the
        // region keeps one instead of building one per update.
        let mut ticks = self.shape_ticks.borrow_mut();
        ticks.capture = Some(Vec::new());
        let updated = behaviours.update_shape(
            &mut level,
            &mut ticks,
            state,
            BlockPos::new(pos[0], pos[1], pos[2]),
            direction,
            BlockPos::new(neighbor_pos[0], neighbor_pos[1], neighbor_pos[2]),
            neighbor,
        );
        (updated, ticks.capture.take().unwrap_or_default())
    }
    /// Schedule ticks a shape update requested in the region's proto chunks.
    fn schedule_captured(
        &mut self,
        behaviours: &crate::level::Behaviours,
        ticks: Vec<crate::level::ticks::CapturedTick>,
    ) -> Result<(), PlacementQueryError> {
        for t in ticks {
            let at = [t.pos.x, t.pos.y, t.pos.z];
            if t.fluid {
                let fluid = crate::fluid::FluidKind::ALL[t.kind as usize];
                self.schedule_fluid_tick(at, fluid, t.delay)?;
            } else {
                let state = behaviours
                    .table
                    .default_state(crate::level::BlockId(t.kind as u16));
                self.schedule_block_tick(at, state, t.delay)?;
            }
        }
        Ok(())
    }
}
impl DecorationWorld for World<'_> {
    type Error = RegionError;
    fn min_section_y(&mut self) -> Result<i32, RegionError> {
        Ok(self
            .region
            .min_y()
            .map_err(|_| RegionError::InvalidChunks)?
            >> 4)
    }
    /// `BiomeRegion::present_in_chunk`: the distinct ids, ascending, from a
    /// bitset instead of sorting every quart of the column.
    fn chunk_biomes(&mut self, chunk: ChunkPos) -> Result<Vec<BiomeId>, RegionError> {
        let biomes = self.biomes.chunk(chunk).map_err(RegionError::Biome)?;
        let mut seen: Vec<u64> = Vec::new();
        let mut last = None;
        let mut section = 0;
        while let Some(values) = biomes.section(section) {
            for &id in values {
                if last == Some(id) {
                    continue;
                }
                last = Some(id);
                let word = id.0 as usize / 64;
                if seen.len() <= word {
                    seen.resize(word + 1, 0);
                }
                seen[word] |= 1 << (id.0 % 64);
            }
            section += 1;
        }
        let mut out = Vec::new();
        for (word, &bits) in seen.iter().enumerate() {
            let mut bits = bits;
            while bits != 0 {
                out.push(BiomeId(word as u32 * 64 + bits.trailing_zeros()));
                bits &= bits - 1;
            }
        }
        Ok(out)
    }
    fn set_currently_generating(&mut self, _: Option<FeatureIdentity>) -> Result<(), RegionError> {
        Ok(())
    }
}
impl FeatureWorld for World<'_> {
    type Error = RegionError;
    #[inline]
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, RegionError> {
        self.region.block(position)
    }
    fn scan_column(
        &mut self,
        position: [i32; 3],
        step: i32,
        limit: i32,
        keep: impl FnMut(BlockStateId) -> bool,
    ) -> Result<i32, RegionError> {
        self.region.scan_column(position, step, limit, keep)
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, RegionError> {
        if !self
            .zone
            .contains_block(BlockPos::new(position[0], position[1], position[2]))
        {
            return Ok(false);
        }
        let old = self.region.block(position)?;
        self.region.set_block_worldgen(position, state)?;
        // ProtoChunk keeps a block entity that already exists when the same kind
        // of block is set again (a chest turning waterlogged); other changes drop it.
        let registry = self.decoration.bindings.registry();
        let same_block = old == state
            || registry.state(old).map(|s| s.name.as_str())
                == registry.state(state).map(|s| s.name.as_str());
        if !same_block {
            self.block_entities.retain(|(p, _)| *p != position);
        }
        // WorldGenRegion.setBlock: BlockState.getPostProcessPos unless UPDATE_KNOWN_SHAPE.
        if flags & 16 == 0 {
            let [x, y, z] = position;
            let mark = if self.decoration.postprocess_self.contains(&state) {
                Some(y)
            } else if self.decoration.postprocess_above.contains(&state) {
                Some(y.wrapping_add(1))
            } else {
                None
            };
            if let Some(y) = mark {
                self.region
                    .chunk_mut(x >> 4, z >> 4)?
                    .mark_postprocessing(x, y, z);
            }
        }
        Ok(true)
    }
}
fn query(e: impl std::fmt::Display) -> PlacementQueryError {
    PlacementQueryError::Failed(e.to_string())
}
impl<'a> World<'a> {
    fn blocks(&self) -> Result<&'a BlockBehavior, PlacementQueryError> {
        self.blocks.ok_or(PlacementQueryError::Unavailable)
    }
}
/// Proto chunks hold no block entities, so dynamic shapes resolve to absent.
impl DynamicSupportWorld for World<'_> {
    type Error = PlacementQueryError;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, PlacementQueryError> {
        Ok(None)
    }
    fn moving_piston_shape(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<VoxelShape>, PlacementQueryError> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for World<'_> {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, PlacementQueryError> {
        Ok(None)
    }
}
impl SurvivalWorld for World<'_> {
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.region.fluid_state(position)
    }
    /// Unlit generation chunks report no sky or block light.
    fn raw_brightness(&mut self, _: [i32; 3], _: i32) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
}
impl PlacementContext for World<'_> {
    fn state_collision_empty(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        collision
            .collision_shape(self, state, position)
            .map(|shape| shape.is_empty())
            .map_err(query)
    }
    /// RandomizableContainer.setBlockEntityLootTable: only a container block that
    /// is actually there takes the table, and then it draws the loot seed.
    fn set_block_entity_loot_table(
        &mut self,
        position: [i32; 3],
        table: &str,
        random: &mut impl horizon_core::random::RandomSource,
    ) -> Result<(), PlacementQueryError> {
        use crate::nbt::Tag;
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        let Some(name) = registry.state(state).map(|s| s.name.to_string()) else {
            return Ok(());
        };
        if !matches!(
            name.as_str(),
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel"
        ) {
            return Ok(());
        }
        let mut nbt = crate::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String(name.as_str().into()));
        nbt.insert("LootTable".into(), Tag::String(table.into()));
        nbt.insert("LootTableSeed".into(), Tag::Long(random.next_long()));
        self.set_block_entity(position, nbt)
    }
    /// BeehiveDecorator: `getBlockEntity(pos, BEEHIVE)` then
    /// `storeBee(Occupant.create(ticks))` for each bee.
    fn store_beehive_occupants(
        &mut self,
        position: [i32; 3],
        ticks_in_hive: &[i32],
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if !matches!(
            registry.state(state).map(|s| s.name.as_str()),
            Some("minecraft:bee_nest" | "minecraft:beehive")
        ) {
            return Ok(());
        }
        self.set_block_entity(position, super::tree::beehive_occupants(ticks_in_hive))
    }
    /// SpawnerBlockEntity.setEntityId: the next spawn data and no potentials.
    fn configure_spawner(
        &mut self,
        position: [i32; 3],
        entity: &str,
    ) -> Result<(), PlacementQueryError> {
        use crate::nbt::Tag;
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if registry.state(state).map(|s| s.name.as_str()) != Some("minecraft:spawner") {
            return Ok(());
        }
        let mut mob = crate::nbt::Compound::new();
        mob.insert("id".into(), Tag::String(entity.into()));
        let mut data = crate::nbt::Compound::new();
        data.insert("entity".into(), Tag::Compound(mob));
        let mut nbt = crate::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:mob_spawner".into()));
        nbt.insert("SpawnData".into(), Tag::Compound(data));
        nbt.insert("SpawnPotentials".into(), Tag::List(Vec::new()));
        self.set_block_entity(position, nbt)
    }
    fn attachment_at(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_attach_to(self, position, direction)
            .map_err(query)
    }
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Ok(self.level_random.boolean())
    }
    fn next_world_float(&mut self) -> Result<f32, PlacementQueryError> {
        Ok(self.level_random.float())
    }
    fn next_world_int(&mut self, bound: i32) -> Result<i32, PlacementQueryError> {
        Ok(self.level_random.int(bound))
    }
    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            let mut level = ShapeLevel {
                world: self,
                random: horizon_core::random::LegacyRandom::new(0),
            };
            let mut ticks = crate::level::Ticks::new(0);
            let pos = BlockPos::new(position[0], position[1], position[2]);
            return Ok(behaviours.can_survive(&mut level, &mut ticks, pos, state));
        }
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_survive(self, state, position)
            .map_err(query)
    }
    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.region.block_state(position)?;
        self.state_face_sturdy(state, position, direction)
    }
    fn state_face_sturdy(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        // Without the exported support tables, fall back to the gameplay
        // behaviour's empty-level sturdiness cache.
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            return Ok(behaviours.is_face_sturdy(state, direction));
        }
        let support = self.blocks()?.support.clone();
        support
            .is_face_sturdy(self, state, position, direction, SupportType::Full)
            .map_err(query)
    }
    fn center_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.region.block_state(position)?;
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            return Ok(behaviours.is_face_sturdy_center(state, direction));
        }
        let support = self.blocks()?.support.clone();
        support
            .is_face_sturdy(self, state, position, direction, SupportType::Center)
            .map_err(query)
    }
    fn can_occlude(&mut self, state: BlockStateId) -> Result<bool, PlacementQueryError> {
        if let Some(behaviours) = self.behaviours {
            return Ok(behaviours.has_flag(state, crate::level::data::state_flags::CAN_OCCLUDE));
        }
        let properties = &self.blocks()?.properties;
        let name = properties
            .registry()
            .state(state)
            .ok_or(PlacementQueryError::Unavailable)?
            .name
            .clone();
        Ok(properties.definition(&name).is_some_and(|d| d.can_occlude))
    }
    fn collision_full_block(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        let shape = collision
            .collision_shape(self, state, position)
            .map_err(query)?;
        let [nx, ny, nz] = shape.sizes();
        Ok(!shape.is_empty()
            && [Axis::X, Axis::Y, Axis::Z]
                .into_iter()
                .all(|axis| shape.min(axis) == 0.0 && shape.max(axis) == 1.0)
            && (0..nx).all(|x| {
                (0..ny).all(|y| (0..nz).all(|z| shape.is_full(x as i32, y as i32, z as i32)))
            }))
    }
    fn add_entity(&mut self, mut nbt: crate::nbt::Compound) -> Result<(), PlacementQueryError> {
        if let Some(mobs) = self.structure_mobs.clone()
            && nbt.contains_key(&"horizon:finalize".into())
        {
            let at = block_of(&nbt).unwrap_or([0; 3]);
            let biome = self
                .biome_identity(at)
                .ok()
                .and_then(|b| self.biome_names.get(b.0 as usize))
                .map_or("", |n| &**n)
                .to_owned();
            let structures: Vec<String> = self
                .starts
                .iter()
                .filter(|s| s.pieces.iter().any(|p| p.bounding_box().is_inside(at)))
                .map(|s| s.name.clone())
                .collect();
            if mobs.finalize(&mut nbt, &biome, &structures, self.level_random) {
                nbt.remove(&"horizon:finalize".into());
            }
        }
        self.entities.push(nbt);
        Ok(())
    }
    /// SpikeFeature.placeSpike: the crystal is a fresh entity of the region,
    /// saved with the chunk.
    fn spawn_end_crystal(
        &mut self,
        position: [f64; 3],
        yaw: f32,
        beam_target: Option<[i32; 3]>,
        invulnerable: bool,
    ) -> Result<(), PlacementQueryError> {
        use crate::nbt::Tag;
        let mut nbt = crate::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:end_crystal".into()));
        nbt.insert(
            "Pos".into(),
            Tag::List(position.iter().map(|v| Tag::Double(v.to_bits())).collect()),
        );
        nbt.insert(
            "Rotation".into(),
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(0)]),
        );
        nbt.insert("Invulnerable".into(), Tag::Byte(i8::from(invulnerable)));
        nbt.insert("ShowBottom".into(), Tag::Byte(1));
        if let Some(b) = beam_target {
            nbt.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
        }
        self.add_entity(nbt)
    }
    fn set_block_entity(
        &mut self,
        position: [i32; 3],
        nbt: crate::nbt::Compound,
    ) -> Result<(), PlacementQueryError> {
        self.block_entities.retain(|(p, _)| *p != position);
        self.block_entities.push((position, nbt));
        Ok(())
    }
    /// WorldGenRegion.getBlockEntity creates the gateway's block entity when the
    /// block is an End gateway; TheEndGatewayBlockEntity.setExitPosition.
    fn configure_end_gateway(
        &mut self,
        position: [i32; 3],
        exit: [i32; 3],
        exact: bool,
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if registry.state(state).map(|s| s.name.as_str()) != Some("minecraft:end_gateway") {
            return Ok(());
        }
        use crate::nbt::Tag;
        let mut nbt = crate::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:end_gateway".into()));
        nbt.insert("Age".into(), Tag::Long(0));
        nbt.insert("exit_portal".into(), Tag::IntArray(exit.to_vec()));
        if exact {
            nbt.insert("ExactTeleport".into(), Tag::Byte(1));
        }
        self.set_block_entity(position, nbt)
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        self.region.world_seed()
    }
    fn update_shape_at_face(
        &mut self,
        position: [i32; 3],
        direction: Direction,
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let Some(behaviours) = self.behaviours else {
            return Ok(());
        };
        let n = direction.normal();
        let neighbor_pos = [position[0] + n[0], position[1] + n[1], position[2] + n[2]];
        let state = self.region.block(position).map_err(query)?;
        let neighbor = self.region.block(neighbor_pos).map_err(query)?;
        let (new_state, ticks) = self.shape_update(
            behaviours,
            state,
            position,
            direction,
            neighbor_pos,
            neighbor,
        );
        self.schedule_captured(behaviours, ticks)?;
        if new_state != state {
            self.set_block(position, new_state, flags & !1)
                .map_err(query)?;
        }
        let (new_neighbor, ticks) = self.shape_update(
            behaviours,
            neighbor,
            neighbor_pos,
            direction.opposite(),
            position,
            new_state,
        );
        self.schedule_captured(behaviours, ticks)?;
        if new_neighbor != neighbor {
            self.set_block(neighbor_pos, new_neighbor, flags & !1)
                .map_err(query)?;
        }
        Ok(())
    }
    fn update_from_neighbour_shapes(
        &mut self,
        position: [i32; 3],
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let Some(behaviours) = self.behaviours else {
            return Ok(());
        };
        let state = self.region.block(position).map_err(query)?;
        // Block.UPDATE_SHAPE_ORDER.
        let mut new_state = state;
        for direction in [
            Direction::West,
            Direction::East,
            Direction::North,
            Direction::South,
            Direction::Down,
            Direction::Up,
        ] {
            let n = direction.normal();
            let neighbor_pos = [position[0] + n[0], position[1] + n[1], position[2] + n[2]];
            let neighbor = self.region.block(neighbor_pos).map_err(query)?;
            let (updated, ticks) = self.shape_update(
                behaviours,
                new_state,
                position,
                direction,
                neighbor_pos,
                neighbor,
            );
            self.schedule_captured(behaviours, ticks)?;
            new_state = updated;
        }
        if new_state != state {
            self.set_block(position, new_state, flags).map_err(query)?;
        }
        Ok(())
    }
    fn ensure_can_write(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(self
            .zone
            .contains_block(BlockPos::new(position[0], position[1], position[2])))
    }
    /// ProtoChunk.setBlockState: the section and the stage's heightmaps.
    fn set_chunk_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region
            .set_block_worldgen(position, state)
            .map_err(|e| query(format!("{e:?}")))?;
        Ok(())
    }
    /// BulkSectionAccess write: section storage only, no heightmap updates.
    fn set_section_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_raw(x, y, z, state)
            .map_err(|e| query(format!("{e:?}")))?;
        Ok(())
    }
    /// Generation regions contain no entities.
    fn unobstructed_block(&mut self, _: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(true)
    }
    fn destroy_block_without_entity(
        &mut self,
        position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        if chunk.block_properties(state).is_air() {
            return Ok(false);
        }
        let air = self
            .decoration
            .bindings
            .registry()
            .default_state("minecraft:air")
            .ok_or(PlacementQueryError::Unavailable)?;
        self.set_block(position, air, 3).map_err(query)
    }
    fn biome_should_freeze(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
        require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_freeze(self, &mut temperature, position, require_edge);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    fn biome_should_snow(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_snow(self, &mut temperature, position);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    /// Lighting has not run during FEATURES; block light reads zero.
    fn block_brightness(&mut self, _: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.region.fluid_state(position)
    }
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.sea_level()
    }
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_fluid_tick(position, fluid, delay)
    }
    fn mark_postprocessing(&mut self, position: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.region.mark_postprocessing(position)
    }
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.region.water_at(position)
    }
    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_block_tick(position, block, delay)
    }
    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        if let Some(biome) = self.uniform_biome(position) {
            return Ok(biome);
        }
        self.region
            .biome_at(
                &self.biomes,
                BlockPos::new(position[0], position[1], position[2]),
            )
            .copied()
            .map_err(Into::into)
    }
    fn biome_allows_feature(
        &mut self,
        position: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        let biome = self.biome_identity(position)?;
        Ok(self.decoration.biome_features[biome.0 as usize].contains(&feature))
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        self.region.block_matches_tag(position, tag)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        self.region.layer_block(position)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.region.height(kind, x, z)
    }
    /// OreFeature's search for a column under its blob, without a trait call
    /// per column. Defers to `height` when a heightmap still has to be primed.
    fn any_height_at_least(
        &mut self,
        kind: HeightmapType,
        [x0, z0]: [i32; 2],
        size: i32,
        y: i32,
    ) -> Result<Option<bool>, PlacementQueryError> {
        let (x1, z1) = (x0.wrapping_add(size), z0.wrapping_add(size));
        if size < 0 || x1 < x0 || z1 < z0 {
            return Ok(None);
        }
        for cx in x0 >> 4..=x1 >> 4 {
            for cz in z0 >> 4..=z1 >> 4 {
                if !self
                    .region
                    .chunk(cx, cz)
                    .is_ok_and(|c| c.heightmap(kind).is_some())
                {
                    return Ok(None);
                }
            }
        }
        // Any column will do, so each chunk's part of the square is read
        // from its heightmap in one go.
        for cx in x0 >> 4..=x1 >> 4 {
            let xs = [x0.max(cx << 4) & 15, x1.min((cx << 4) + 15) & 15];
            for cz in z0 >> 4..=z1 >> 4 {
                let zs = [z0.max(cz << 4) & 15, z1.min((cz << 4) + 15) & 15];
                let map = self.region.chunk(cx, cz)?.heightmap(kind).unwrap();
                if map.any_first_available_at_least(xs, zs, y) {
                    return Ok(Some(true));
                }
            }
        }
        Ok(Some(false))
    }
}

/// The block an entity's `Pos` is in.
fn block_of(nbt: &crate::nbt::Compound) -> Option<[i32; 3]> {
    let Some(crate::nbt::Tag::List(p)) = nbt.get(&"Pos".into()) else {
        return None;
    };
    Some(std::array::from_fn(|i| match p.get(i) {
        Some(crate::nbt::Tag::Double(b)) => f64::from_bits(*b).floor() as i32,
        _ => 0,
    }))
}
