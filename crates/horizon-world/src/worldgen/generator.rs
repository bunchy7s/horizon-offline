//! Production noise-chunk generation for one dimension of a world preset.
//!
//! `GeneratorData` holds seed-independent resources parsed once from a bundle
//! directory (see `GeneratorData::load`). `OverworldGenerator` binds a world seed
//! and is shared across threads; each thread owns a `GeneratorWorker`.
//! `generate_proto` runs vanilla's BIOMES and TERRAIN statuses (noise fill with
//! aquifers, material surface, carvers) and primes the post-terrain heightmaps.
//! `decorate` runs the FEATURES status for the center of a 3×3 neighborhood with
//! whatever feature algorithms are implemented; structures are not generated.
use super::{
    DecodeError,
    anchor::WorldGenerationContext,
    aquifer::{
        AquiferConfig, AquiferField, AquiferSamplers, DisabledAquifer, GlobalFluidPicker,
        NoiseBasedAquifer, RegistryAquiferBlockStates,
    },
    biome::BiomeId,
    biome_chunk::{BiomeChunk, BiomeRegion},
    biome_source::{BiomeSourceCatalog, ResolvedBiomeSource},
    biome_temperature::{BiomeTemperature, BiomeTemperatureSettings},
    carver::{CarverCatalog, WorldCarver, generate_carving_mask},
    carving::{CarvingBlocks, apply_carving_mask},
    climate::{ClimateSampler, decode_parameter, quantize},
    decode_noise,
    density::DensityRegistry,
    generator_structures::Starts,
    material_condition::{
        MaterialConditionRegistry, MaterialConditions, MaterialDensitySources, MaterialRule,
        MaterialRuleError, MaterialRuleLibrary, MaterialSurfaceQueries,
    },
    material_extensions::{ExtensionBlocks, ExtensionNoises, MaterialExtensions},
    material_stage::{SurfaceInputs, TopMaterialInputs, build_surface},
    material_surface::{ClayBandBlocks, MaterialSurface},
    overrides::{self, WorldgenOverrides},
    router::{NoiseRouter, RouterField, RouterSamplers},
    settings::NoiseSettings,
    spawn_finder::{SpawnTargetPoint, find_spawn_position},
    tags::TagDefinitions,
    terrain::{PreLightChunkStage, TerrainAquifer, TerrainBlockStates, TerrainChunk, fill_noise},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    density::{DensitySampler, SamplerContext},
    noise::{DensityVolume, Noise},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Debug)]
pub enum GeneratorError {
    Io(io::Error),
    Decode(DecodeError),
    /// A resource is missing or inconsistent with the block registry.
    Resource(String),
    /// A generation stage failed; the chunk is incomplete.
    Stage(&'static str, String),
}
impl std::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Decode(e) => e.fmt(f),
            Self::Resource(e) => write!(f, "invalid generator resources: {e}"),
            Self::Stage(stage, e) => write!(f, "{stage} failed: {e}"),
        }
    }
}
impl std::error::Error for GeneratorError {}
impl From<io::Error> for GeneratorError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<DecodeError> for GeneratorError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}
impl From<serde_json::Error> for GeneratorError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e.into())
    }
}
fn stage<E: std::fmt::Display>(name: &'static str) -> impl FnOnce(E) -> GeneratorError {
    move |e| GeneratorError::Stage(name, e.to_string())
}
fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}

/// Heightmap-relevant block behavior, one entry per registry state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenBlockState {
    pub air: bool,
    pub air_block: bool,
    pub fluid: bool,
    pub motion: bool,
    pub no_leaves: bool,
}
impl HeightmapState for GenBlockState {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
/// Shared, cheaply cloned state table bound into every generated chunk.
#[derive(Clone, Debug)]
pub struct GenBlockStates(Arc<[GenBlockState]>);
impl GenBlockStates {
    pub fn get(&self, id: BlockStateId) -> GenBlockState {
        self.0.get(id.0 as usize).copied().unwrap_or_default()
    }
}
impl TerrainBlockStates for GenBlockStates {
    type State = GenBlockState;
    fn heightmap_state(&self, id: BlockStateId) -> GenBlockState {
        self.get(id)
    }
}

/// Feature resources for the FEATURES status, keyed by resource identifier.
pub(super) struct FeatureResources {
    pub(super) features: BTreeMap<String, String>,
    pub(super) placed: BTreeMap<String, String>,
    pub(super) feature_tags: BTreeMap<String, String>,
    pub(super) fluid_tags: BTreeMap<String, Vec<String>>,
    pub(super) state_providers: BTreeMap<String, String>,
    /// Block properties, support, collision and survival for placement checks.
    pub(super) blocks: Option<super::generator_blocks::BlockBehavior>,
}

/// Raw structure, structure-set and template-pool resources.
pub(super) struct StructureBundles {
    pub(super) structures: BTreeMap<String, String>,
    pub(super) sets: BTreeMap<String, String>,
    pub(super) pools: BTreeMap<String, String>,
}

/// Seed-independent generation resources, loaded once and shared by every
/// generator. Biome identities are indices into the sorted biome registry.
pub struct GeneratorData {
    registry: &'static BlockRegistry,
    states: GenBlockStates,
    integers: BTreeSet<(String, String)>,
    noises: BTreeMap<String, String>,
    densities: BTreeMap<String, String>,
    rules: BTreeMap<String, String>,
    settings: BTreeMap<String, String>,
    presets: BTreeMap<String, String>,
    world_presets: BTreeMap<String, String>,
    /// Data packs' level stems (`dimension/<id>.json`), by dimension id:
    /// they replace the world preset's.
    dimensions: BTreeMap<String, String>,
    /// Data packs' dimension types (`dimension_type/<id>.json`), by id:
    /// they replace the vanilla ones' build ranges.
    dimension_types: BTreeMap<String, String>,
    /// What loading the data packs' entries left out, for the log.
    warnings: Vec<String>,
    conditions: MaterialConditionRegistry,
    source_catalog: BiomeSourceCatalog,
    biome_names: Vec<Arc<str>>,
    biome_ids: BTreeMap<String, BiomeId>,
    biome_json: Vec<String>,
    temperatures: Vec<BiomeTemperatureSettings>,
    biome_carvers: Vec<Vec<Arc<WorldCarver>>>,
    carving_blocks: CarvingBlocks,
    pub(super) block_tags: BTreeMap<String, Vec<String>>,
    /// `block_tags` bound to block states, once for the feature bindings of
    /// decoration, structures and live features (`feature_block_bindings`).
    bound_block_tags: std::sync::OnceLock<super::block_feature::BoundBlockTags>,
    pub(super) biome_tags: BTreeMap<String, Vec<String>>,
    pub(super) feature_resources: Option<FeatureResources>,
    /// Templates, processor lists and per-state template transforms.
    pub(super) structures: Option<Arc<super::structure::StructureResources>>,
    pub(super) structure_resources: Option<StructureBundles>,
    /// Block shape-update behaviour (`block-behaviours.json` next to the
    /// worldgen directory) for template placement's neighbor updates.
    pub(super) behaviours: Option<Arc<crate::level::Behaviours>>,
    /// Concentric ring positions by seed and dimension generator (see
    /// `rings_for`), most recent last.
    ring_cache: Mutex<Vec<RingCacheEntry>>,
    /// Placed-feature decoration by biome source (see `decoration_for`),
    /// most recent last.
    decorations: Mutex<Vec<(String, DecorationSlot)>>,
    /// What `BlendingData` asks of the blocks of an old chunk, by state.
    old_chunk_facts: OnceLock<OldChunkFacts>,
}
/// `BlendingData.isGround` and `SURFACE_BLOCKS`, by block state id.
struct OldChunkFacts {
    ground: Vec<bool>,
    surface: Vec<bool>,
}
type RingCacheEntry = ((i64, String), Arc<OnceLock<super::structure::start::Rings>>);
/// Generators whose ring positions are kept: the dimensions of a few worlds.
const RING_CACHE_LIMIT: usize = 8;
type DecorationSlot =
    Arc<OnceLock<Result<Option<Arc<super::generator_decoration::Decoration>>, String>>>;
/// Biome sources whose decoration is kept: the three dimensions and a spare.
const DECORATION_CACHE_LIMIT: usize = 4;
type Bundle = BTreeMap<String, String>;
/// The bundle `name` with the data packs' entries over it.
fn read_bundle(
    dir: &Path,
    overrides: &WorldgenOverrides,
    name: &str,
) -> Result<Bundle, GeneratorError> {
    let path = dir.join(format!("{name}.json"));
    let text =
        fs::read_to_string(&path).map_err(|e| resource(format!("{}: {e}", path.display())))?;
    Ok(overrides.apply(name, serde_json::from_str(&text)?))
}
fn read_optional_bundle(
    dir: &Path,
    overrides: &WorldgenOverrides,
    name: &str,
) -> Result<Option<Bundle>, GeneratorError> {
    if dir.join(format!("{name}.json")).exists() {
        read_bundle(dir, overrides, name).map(Some)
    } else {
        Ok(None)
    }
}
/// The tags of the bundle `name`, the data packs' tag files after the
/// prepared ones. A tag naming what is not there fails on its own, as
/// vanilla's TagLoader leaves it out. Without packs the prepared tags must
/// all resolve; with them (a pack's tag can also break a prepared tag that
/// includes it), each failed tag goes into `dropped` for the log.
fn build_tags(
    raw: &Bundle,
    overrides: &WorldgenOverrides,
    name: &str,
    dropped: &mut Vec<String>,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, GeneratorError> {
    let mut definitions = TagDefinitions::default();
    for (name, json) in raw {
        definitions.apply_json(name, "vanilla", json)?;
    }
    for (id, pack, json) in overrides.tags(name) {
        definitions
            .apply_json(id, pack, json)
            .map_err(|e| resource(format!("{pack}: tag {id}: {e}")))?;
    }
    let built = definitions
        .build(|id, _| known(id).then(|| id.to_owned()))
        .map_err(|e| resource(format!("tags: {e:?}")))?;
    if overrides.tags(name).next().is_none() && !built.missing.is_empty() {
        return Err(resource(format!(
            "unresolved tag entries in {:?}",
            built.missing.keys().collect::<Vec<_>>()
        )));
    }
    for (tag, entries) in &built.missing {
        dropped.push(format!(
            "{name}: tag {tag} names what is missing: {}",
            entries
                .iter()
                .map(|e| format!("{:?} ({})", e.entry, e.source))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(built.tags)
}
/// Convert a data-generator `biome_parameters` report into the quantized
/// preset table format, using vanilla's float `quantizeCoord`.
fn quantize_parameter_report(json: &str) -> Result<String, GeneratorError> {
    let report: Value = serde_json::from_str(json)?;
    let float = |v: &Value| -> Result<i64, GeneratorError> {
        let text = v.to_string();
        text.parse::<f32>()
            .map(quantize)
            .map_err(|_| resource("biome parameter value"))
    };
    let mut entries = Vec::new();
    for entry in report["biomes"]
        .as_array()
        .ok_or_else(|| resource("biome parameter report"))?
    {
        let parameters = &entry["parameters"];
        let mut coordinates = Vec::new();
        for key in [
            "temperature",
            "humidity",
            "continentalness",
            "erosion",
            "depth",
            "weirdness",
        ] {
            let value = &parameters[key];
            coordinates.push(match value.as_array() {
                Some(range) if range.len() == 2 => {
                    serde_json::json!([float(&range[0])?, float(&range[1])?])
                }
                None if value.is_number() => {
                    let v = float(value)?;
                    serde_json::json!([v, v])
                }
                _ => return Err(resource("biome parameter interval")),
            });
        }
        entries.push(serde_json::json!({
            "coordinates": coordinates,
            "offset": float(&parameters["offset"])?,
            "biome": entry["biome"],
        }));
    }
    Ok(Value::Array(entries).to_string())
}
/// `load` on a thread of its own ("Worldgen loader").
fn loading<T: Send + 'static>(
    load: impl FnOnce() -> T + Send + 'static,
) -> io::Result<std::thread::JoinHandle<T>> {
    std::thread::Builder::new()
        .name("Worldgen loader".into())
        .spawn(load)
}
/// A loading thread's result; its panic goes on in this thread.
fn joined<T>(thread: std::thread::JoinHandle<T>) -> T {
    thread
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
impl GeneratorData {
    /// Load a worldgen bundle directory. Required files are JSON objects mapping
    /// resource identifiers to the resource's JSON text: `noises`, `densities`,
    /// `rules` (material rules), `conditions` (material conditions), `settings`
    /// (noise settings), `carvers`, `biomes`, `biome_tags`, `block_tags`,
    /// `world_presets`; `block_flags.json` is an array of `[is_air, has_fluid]`
    /// per registry state. Multi-noise tables come from `presets.json`
    /// (quantized) or `biome_parameters/<name>.json` data-generator reports.
    /// Optional decoration inputs: `features`, `placed_features`,
    /// `feature_tags`, `fluid_tags`, `state_providers` and the Java-exported
    /// block behavior files in `blocks/` (see `generator_blocks`). Integer block properties are derived from
    /// the registry: properties whose every value is a decimal integer.
    pub fn load(dir: &Path, registry: &'static BlockRegistry) -> Result<Arc<Self>, GeneratorError> {
        Self::load_with(dir, registry, &WorldgenOverrides::default())
    }
    /// `load` with a world's data packs over the bundles (see `overrides`).
    pub fn load_with(
        dir: &Path,
        registry: &'static BlockRegistry,
        ov: &WorldgenOverrides,
    ) -> Result<Arc<Self>, GeneratorError> {
        let mut warnings = Vec::new();
        let flags: Vec<(bool, bool)> =
            serde_json::from_str(&fs::read_to_string(dir.join("block_flags.json"))?)?;
        if flags.len() != registry.len() {
            return Err(resource("block_flags.json does not match the registry"));
        }
        let block_tags = build_tags(
            &read_bundle(dir, ov, "block_tags")?,
            ov,
            "block_tags",
            &mut warnings,
            |id| registry.default_state(id).is_some(),
        )?;
        let fluid_tags = match read_optional_bundle(dir, ov, "fluid_tags")? {
            Some(raw) => build_tags(&raw, ov, "fluid_tags", &mut warnings, |id| {
                matches!(
                    id,
                    "minecraft:water"
                        | "minecraft:flowing_water"
                        | "minecraft:lava"
                        | "minecraft:flowing_lava"
                        | "minecraft:empty"
                )
            })?,
            None => BTreeMap::new(),
        };
        // The three biggest reads (the block tables, the structure templates
        // and the block behaviours) need only the tags: they run on threads of
        // their own while this one reads the rest. A new world's first chunks
        // wait for all of it, with the chunk workers idle.
        let blocks = loading({
            let (dir, tags, fluids) = (dir.join("blocks"), block_tags.clone(), fluid_tags.clone());
            move || super::generator_blocks::BlockBehavior::load(&dir, registry, &tags, &fluids)
        })?;
        let structures = loading({
            let (dir, tags, ov) = (dir.to_owned(), block_tags.clone(), ov.clone());
            move || -> Result<_, GeneratorError> {
                let Ok(json) = fs::read_to_string(dir.join("blocks/structure-blocks.json")) else {
                    return Ok(None);
                };
                Ok(Some(Arc::new(super::structure::StructureResources::new(
                    registry,
                    super::structure::StateTransforms::from_json(&json, registry.len())?,
                    &dir.join("structure"),
                    read_optional_bundle(&dir, &ov, "processor_lists")?.unwrap_or_default(),
                    &tags,
                ))))
            }
        })?;
        let behaviours = loading({
            let root = dir.parent().map(Path::to_owned);
            move || {
                root.filter(|root| root.join("block-behaviours.json").exists())
                    .and_then(|root| crate::level::Behaviours::load(registry, &root).ok())
                    .map(Arc::new)
            }
        })?;
        let tag_set = |name: &str| -> Result<BTreeSet<&str>, GeneratorError> {
            Ok(block_tags
                .get(name)
                .ok_or_else(|| resource(format!("missing block tag {name}")))?
                .iter()
                .map(String::as_str)
                .collect())
        };
        let motion = tag_set("minecraft:blocks_motion_in_heightmap")?;
        let no_leaves = tag_set("minecraft:blocks_motion_in_heightmap_no_leaves")?;
        let mut values = BTreeMap::<(String, String), bool>::new();
        let states = (0..registry.len())
            .map(|i| {
                let state = registry.state(BlockStateId(i as u32)).unwrap();
                for (key, value) in &state.properties {
                    let integer = !value.is_empty()
                        && value
                            .trim_start_matches('-')
                            .bytes()
                            .all(|b| b.is_ascii_digit());
                    *values
                        .entry((state.name.clone(), key.clone()))
                        .or_insert(true) &= integer;
                }
                GenBlockState {
                    air: flags[i].0,
                    air_block: state.name == "minecraft:air",
                    fluid: flags[i].1,
                    motion: motion.contains(state.name.as_str()),
                    no_leaves: no_leaves.contains(state.name.as_str()),
                }
            })
            .collect::<Vec<_>>();
        let integers = values
            .into_iter()
            .filter_map(|(key, integer)| integer.then_some(key))
            .collect();
        let biomes = read_bundle(dir, ov, "biomes")?;
        let biome_tags = build_tags(
            &read_bundle(dir, ov, "biome_tags")?,
            ov,
            "biome_tags",
            &mut warnings,
            |id| biomes.contains_key(id),
        )?;
        let biome_set = biomes.keys().cloned().collect::<BTreeSet<_>>();
        let conditions = MaterialConditionRegistry::from_json_with_biome_tags(
            &read_bundle(dir, ov, "conditions")?,
            &biome_set,
            &biome_tags,
        )?;
        let mut presets = read_optional_bundle(dir, ov, "presets")?.unwrap_or_default();
        let reports = dir.join("biome_parameters");
        if reports.is_dir() {
            for entry in fs::read_dir(&reports)? {
                let path = entry?.path();
                let Some(stem) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|_| path.extension().is_some_and(|e| e == "json"))
                else {
                    continue;
                };
                presets
                    .entry(format!("minecraft:{stem}"))
                    .or_insert(quantize_parameter_report(&fs::read_to_string(&path)?)?);
            }
        }
        let source_catalog =
            BiomeSourceCatalog::new(&biome_set, &biome_tags, &presets.keys().cloned().collect())?;
        let carvers = CarverCatalog::from_json(&read_bundle(dir, ov, "carvers")?)?;
        let mut biome_names = Vec::new();
        let mut biome_ids = BTreeMap::new();
        let mut biome_json = Vec::new();
        let mut temperatures = Vec::new();
        let mut biome_carvers = Vec::new();
        for (name, json) in &biomes {
            biome_ids.insert(name.clone(), BiomeId(biome_names.len() as u32));
            biome_names.push(Arc::<str>::from(name.as_str()));
            temperatures.push(BiomeTemperatureSettings::from_json(json)?);
            let value: Value = serde_json::from_str(json)?;
            biome_carvers.push(carvers.decode_set(&value["carvers"].to_string())?);
            biome_json.push(json.clone());
        }
        let carving_blocks = CarvingBlocks::from_registry(
            registry,
            &tag_set("minecraft:uncarvable")?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
        .map_err(|e| resource(format!("carving blocks: {e}")))?;
        let feature_resources = match (
            read_optional_bundle(dir, ov, "features")?,
            read_optional_bundle(dir, ov, "placed_features")?,
        ) {
            (Some(features), Some(placed)) => Some(FeatureResources {
                features,
                placed,
                feature_tags: read_optional_bundle(dir, ov, "feature_tags")?.unwrap_or_default(),
                blocks: joined(blocks)?,
                fluid_tags,
                state_providers: read_optional_bundle(dir, ov, "state_providers")?
                    .unwrap_or_default(),
            }),
            _ => None,
        };
        let structures = joined(structures)?;
        let structure_resources = match (
            read_optional_bundle(dir, ov, "structures")?,
            read_optional_bundle(dir, ov, "structure_sets")?,
            read_optional_bundle(dir, ov, "template_pools")?,
        ) {
            (Some(structures), Some(sets), Some(pools)) => Some(StructureBundles {
                structures,
                sets,
                pools,
            }),
            _ => None,
        };
        let behaviours = joined(behaviours);
        Ok(Arc::new(Self {
            behaviours,
            structure_resources,
            structures,
            registry,
            states: GenBlockStates(states.into()),
            integers,
            noises: read_bundle(dir, ov, "noises")?,
            densities: read_bundle(dir, ov, "densities")?,
            rules: read_bundle(dir, ov, "rules")?,
            settings: read_bundle(dir, ov, "settings")?,
            world_presets: read_bundle(dir, ov, "world_presets")?,
            dimensions: ov.apply(overrides::DIMENSIONS, BTreeMap::new()),
            dimension_types: ov.apply(overrides::DIMENSION_TYPES, BTreeMap::new()),
            warnings,
            presets,
            conditions,
            source_catalog,
            biome_names,
            biome_ids,
            biome_json,
            temperatures,
            biome_carvers,
            carving_blocks,
            block_tags,
            biome_tags,
            feature_resources,
            ring_cache: Mutex::new(Vec::new()),
            decorations: Mutex::new(Vec::new()),
            old_chunk_facts: OnceLock::new(),
            bound_block_tags: std::sync::OnceLock::new(),
        }))
    }
    /// The concentric ring positions (strongholds) of the generator for
    /// `seed` and the dimension `generator` definition, shared by every
    /// generator built for them. They depend on nothing else, so returning
    /// to a dimension, which builds its generator again, finds them done.
    pub(super) fn rings_for(
        &self,
        seed: i64,
        generator: &Value,
    ) -> Arc<OnceLock<super::structure::start::Rings>> {
        let key = (seed, generator.to_string());
        let mut cache = self.ring_cache.lock().unwrap_or_else(|e| e.into_inner());
        let rings = match cache.iter().position(|(k, _)| *k == key) {
            Some(i) => cache.remove(i).1,
            None => Arc::new(OnceLock::new()),
        };
        if cache.len() >= RING_CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push((key, rings.clone()));
        rings
    }
    /// The placed-feature decoration of the biome source `source_json`
    /// (resolved: `source`), shared by every generator that uses it. It
    /// depends on these resources and the source alone, not on the seed, and
    /// building it (decoding the placed features of every biome the source
    /// can place) took about 140 ms of each world opening. A caller that
    /// arrives while another builds it waits for that build.
    pub(super) fn decoration_for(
        self: &Arc<Self>,
        source_json: &str,
        source: &ResolvedBiomeSource,
    ) -> Result<Option<Arc<super::generator_decoration::Decoration>>, GeneratorError> {
        let slot = {
            let mut cache = self.decorations.lock().unwrap_or_else(|e| e.into_inner());
            let slot = match cache.iter().position(|(k, _)| k == source_json) {
                Some(i) => cache.remove(i).1,
                None => Arc::new(OnceLock::new()),
            };
            if cache.len() >= DECORATION_CACHE_LIMIT {
                cache.remove(0);
            }
            cache.push((source_json.to_owned(), slot.clone()));
            slot
        };
        slot.get_or_init(|| {
            super::generator_decoration::Decoration::new(self, source)
                .map(|d| d.map(Arc::new))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(resource)
    }
    /// Builds the decoration of `preset`'s `dimension` generator before a
    /// world needs it (the world screens' preload), so opening the world
    /// finds it ready.
    pub fn prepare_decoration(
        self: &Arc<Self>,
        preset: &str,
        dimension: &str,
    ) -> Result<(), GeneratorError> {
        let preset_json: Value = serde_json::from_str(
            self.world_presets
                .get(preset)
                .ok_or_else(|| resource(format!("missing world preset {preset}")))?,
        )?;
        let source_json =
            preset_json["dimensions"][dimension]["generator"]["biome_source"].to_string();
        let source = self
            .source_catalog
            .decode(&source_json)?
            .resolve(&self.presets)
            .map_err(|e| resource(format!("biome source: {e}")))?;
        self.decoration_for(&source_json, &source).map(drop)
    }
    pub fn registry(&self) -> &'static BlockRegistry {
        self.registry
    }
    /// What loading the data packs' entries left out (tags naming what is
    /// missing), for the log.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn block_states(&self) -> &GenBlockStates {
        &self.states
    }
    /// Biome identifiers in registry (identifier-sorted) order.
    pub fn configured_feature_json(&self, name: &str) -> Option<&str> {
        self.feature_resources
            .as_ref()?
            .features
            .get(name)
            .map(String::as_str)
    }
    pub fn placed_feature_json(&self, name: &str) -> Option<&str> {
        self.feature_resources
            .as_ref()?
            .placed
            .get(name)
            .map(String::as_str)
    }
    pub fn biome_names(&self) -> &[Arc<str>] {
        &self.biome_names
    }
    fn old_chunk_facts(&self) -> &OldChunkFacts {
        self.old_chunk_facts.get_or_init(|| {
            let members = |tag: &str| -> std::collections::HashSet<&str> {
                self.block_tags
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .map(String::as_str)
                    .collect()
            };
            let (leaves, logs) = (members("minecraft:leaves"), members("minecraft:logs"));
            let collision = self
                .feature_resources
                .as_ref()
                .and_then(|resources| resources.blocks.as_ref())
                .map(|blocks| &blocks.collision);
            let surface_names = [
                "minecraft:podzol",
                "minecraft:gravel",
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:coarse_dirt",
                "minecraft:sand",
                "minecraft:red_sand",
                "minecraft:mycelium",
                "minecraft:snow_block",
                "minecraft:terracotta",
                "minecraft:dirt",
            ];
            let (mut ground, mut surface) = (Vec::new(), Vec::new());
            for index in 0..self.registry.len() {
                let id = BlockStateId(index as u32);
                let name = self.registry.state(id).map_or("", |s| s.name.as_str());
                surface.push(surface_names.contains(&name));
                // An empty collision shape: air, the liquids (no entity stands
                // on them), powder snow and the like; and leaves, logs and
                // huge mushrooms, which `isGround` leaves out.
                let excluded = self.states.get(id).air
                    || leaves.contains(name)
                    || logs.contains(name)
                    || matches!(
                        name,
                        "minecraft:brown_mushroom_block"
                            | "minecraft:red_mushroom_block"
                            | "minecraft:water"
                            | "minecraft:lava"
                            | "minecraft:powder_snow"
                    );
                ground.push(
                    !excluded
                        && match collision {
                            Some(registry) => registry
                                .collision_block(id)
                                .map_or(true, |block| !block.empty_shape),
                            None => self.states.get(id).motion,
                        },
                );
            }
            OldChunkFacts { ground, surface }
        })
    }
    /// `BlendingData.isGround` of a block state of an old chunk.
    pub fn is_old_chunk_ground(&self, state: BlockStateId) -> bool {
        self.old_chunk_facts()
            .ground
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    /// Whether the block state is one of `BlendingData.SURFACE_BLOCKS`.
    pub fn is_old_chunk_surface(&self, state: BlockStateId) -> bool {
        self.old_chunk_facts()
            .surface
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn biome_name(&self, id: BiomeId) -> &str {
        &self.biome_names[id.0 as usize]
    }
    pub fn biome_id(&self, name: &str) -> Option<BiomeId> {
        self.biome_ids.get(name).copied()
    }
    /// The biome's resource JSON (climate, effects such as water/sky/foliage
    /// colors, attributes, carvers and features).
    pub fn biome_json(&self, id: BiomeId) -> &str {
        &self.biome_json[id.0 as usize]
    }
    pub(super) fn integers(&self) -> &BTreeSet<(String, String)> {
        &self.integers
    }
    /// Feature block bindings over this data's blocks and block tags. The tags'
    /// state sets are bound once and shared: each binding held 5 MB of them.
    pub(super) fn feature_block_bindings(
        &self,
    ) -> Result<super::block_feature::FeatureBlockBindings<'static>, DecodeError> {
        use super::block_feature::FeatureBlockBindings;
        let bound = match self.bound_block_tags.get() {
            Some(bound) => bound.clone(),
            None => {
                let bound = FeatureBlockBindings::bind_tags(self.registry, &self.block_tags)?;
                self.bound_block_tags.get_or_init(|| bound).clone()
            }
        };
        FeatureBlockBindings::with_bound_tags(self.registry, self.integers.clone(), bound)
    }
}

/// A generated chunk with its quart biomes. Blocks and heightmaps live in
/// `terrain`; after `generate_proto` it is at the TERRAIN status with the
/// WORLD_SURFACE(_WG), OCEAN_FLOOR(_WG) and MOTION_BLOCKING(_NO_LEAVES)
/// heightmaps primed. Fluid positions scheduled for postprocessing are kept in
/// `terrain.postprocessing()`.
pub struct ProtoChunk {
    pub terrain: TerrainChunk<'static, GenBlockStates>,
    biomes: BiomeChunk<BiomeId>,
    data: Arc<GeneratorData>,
    decorated: bool,
    /// Block-entity data placed by structures and template features, in
    /// placement order (positions are absolute block coordinates).
    pub block_entities: Vec<([i32; 3], crate::nbt::Compound)>,
    /// Structure starts referencing this chunk (STRUCTURE_REFERENCES).
    pub(super) structure_starts: Vec<Arc<super::structure::start::StructureStart>>,
    /// The starts made in this chunk (STRUCTURE_STARTS), also those none of
    /// whose pieces reach it (a giant ruined portal next door).
    own_starts: Starts,
    /// Entities placed by structures (`ProtoChunk.addEntity`), as NBT with
    /// absolute `Pos`.
    pub entities: Vec<crate::nbt::Compound>,
}
impl ProtoChunk {
    pub fn position(&self) -> ChunkPos {
        self.terrain.position()
    }
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.terrain.block(x, y, z)
    }
    pub fn block_name(&self, x: i32, y: i32, z: i32) -> &str {
        &self
            .data
            .registry
            .state(self.block(x, y, z))
            .expect("registered state")
            .name
    }
    /// Stored quart biome (4×4×4 cell) containing this block, without the
    /// client's biome zoom. X/Z wrap to this chunk; Y clamps to its range.
    pub fn biome_id_at(&self, x: i32, y: i32, z: i32) -> BiomeId {
        *self.biomes.noise_biome([x >> 2, y >> 2, z >> 2])
    }
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> &str {
        self.data.biome_name(self.biome_id_at(x, y, z))
    }
    pub fn biomes(&self) -> &BiomeChunk<BiomeId> {
        &self.biomes
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    /// Structure starts referencing this chunk: (structure id, start chunk,
    /// inclusive bounding box min/max).
    pub fn structures(&self) -> Vec<(&str, ChunkPos, [i32; 3], [i32; 3])> {
        self.structure_starts
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.chunk,
                    s.bounding_box.min,
                    s.bounding_box.max,
                )
            })
            .collect()
    }
    pub fn is_decorated(&self) -> bool {
        self.decorated
    }
    /// The structure starts this chunk holds itself (`structures.starts`).
    pub fn own_starts(&self) -> Vec<Arc<super::structure::start::StructureStart>> {
        self.own_starts.to_vec()
    }
    /// Also restores a saved proto chunk that was decorated before it was saved.
    pub fn set_decorated(&mut self) {
        self.decorated = true;
    }
    /// Highest non-air block + 1 in the column (WORLD_SURFACE).
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.terrain
            .heightmap(HeightmapType::WorldSurface)
            .and_then(|map| map.first_available(x & 15, z & 15).ok())
            .unwrap_or(self.terrain.min_y())
    }
}

/// Per-noise-settings values resolved from the dimension's generator JSON.
struct Settings {
    noise_min_y: i32,
    noise_height: i32,
    /// The dimension type's build range (LevelHeightAccessor): proto chunks,
    /// their heightmaps and the decoration region span it, while the noise
    /// only fills `noise_min_y..noise_min_y + noise_height`.
    level_min_y: i32,
    level_height: i32,
    sea_level: i32,
    default_block: BlockStateId,
    default_fluid: BlockStateId,
    legacy: bool,
}

/// The seed-bound pipeline for one random-source kind.
struct Pipeline<R: RandomSource> {
    data: Arc<GeneratorData>,
    seed: i64,
    settings: Settings,
    root: R::Positional,
    router: RouterSamplers,
    climate: ClimateSampler,
    aquifer: Option<[Arc<dyn DensitySampler>; 6]>,
    rule: MaterialRule,
    preliminary: Arc<dyn DensitySampler>,
    source: ResolvedBiomeSource,
    surface: MaterialSurface<R::Positional>,
    noises: BTreeMap<String, Arc<dyn Noise>>,
    extensions: MaterialExtensions<R::Positional>,
    picker: GlobalFluidPicker,
    zoom: BiomeZoom,
    biome_cache: BiomeCache,
    biome_work: ChunkWork,
    carver_cache: Mutex<HashMap<(i32, i32), BiomeId>>,
    pub(super) decoration: Option<Arc<super::generator_decoration::Decoration>>,
    pub(super) structures: Option<super::generator_structures::StructureState>,
    /// The noise settings' `spawn_target` points.
    spawn_target: Vec<SpawnTargetPoint>,
    /// The chunks an older version generated, which new chunks blend with.
    old_world: std::sync::OnceLock<Arc<dyn super::blending::OldWorld>>,
    /// The noise settings' `debug_functions`: label and sampler.
    debug_density: Vec<(String, Arc<dyn DensitySampler>)>,
    spawn_position: std::sync::OnceLock<[i32; 2]>,
    /// A superflat world: its layers stand where the noise terrain would.
    flat: Option<FlatLayers>,
}
/// `FlatLevelGeneratorSettings.getLayers` as `FlatLevelSource` reads them: the states from the
/// level's minimum Y up, with the layers `adjustGenerationSettings` replaced by `null`
/// (not opaque to `MOTION_BLOCKING`: a feature fills them later) marked.
struct FlatLayers {
    min_y: i32,
    states: Vec<BlockStateId>,
    /// `layers.get(i) == null`: `getBaseHeight` skips it, `getBaseColumn` reads it as air, and
    /// `buildTerrain` leaves it for the `FillLayerFeature` that runs after the structures.
    null: Vec<bool>,
}
/// A superflat world's generator settings (`FlatLevelGeneratorSettings`).
#[derive(Clone, Debug)]
pub struct FlatSettings {
    /// The layers, bottom up: block id and height.
    pub layers: Vec<(String, i32)>,
    /// The biome the whole world is.
    pub biome: String,
    /// `structure_overrides`: the structure sets that may generate (all of them when `None`).
    pub structure_overrides: Option<Vec<String>>,
}
const CACHE_LIMIT: usize = 16384;
/// Coordinates only equal missing neighbour keys. Weak entries retain no
/// computed values across eviction or explicit invalidation. Only one work
/// lock may be held at a time; finish shared prerequisites before acquiring it.
/// Sharded by key: every worker asks for the locks of the same neighbourhoods.
pub(super) struct ChunkWork(Box<[Mutex<HashMap<(i32, i32), std::sync::Weak<Mutex<()>>>>]>);
const WORK_SHARDS: usize = 64;
/// The shard of `key` among `shards` (a power of two).
pub(super) fn shard_of(key: (i32, i32), shards: usize) -> usize {
    let hash = (key.0 as u32).wrapping_mul(0x9E37_79B9) ^ (key.1 as u32).wrapping_mul(0x85EB_CA6B);
    (hash >> (32 - shards.trailing_zeros())) as usize
}
impl ChunkWork {
    pub(super) fn new() -> Self {
        Self((0..WORK_SHARDS).map(|_| Mutex::default()).collect())
    }
    pub(super) fn at(&self, key: (i32, i32)) -> Arc<Mutex<()>> {
        let mut work = self.0[shard_of(key, WORK_SHARDS)].lock().unwrap();
        if let Some(lock) = work.get(&key).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        if work.len() >= 4096 / WORK_SHARDS {
            // Keep every active builder/waiter on its original lock, including
            // when the result cache was cleared while it was in flight.
            work.retain(|_, lock| lock.strong_count() > 0);
        }
        let lock = Arc::new(Mutex::new(()));
        work.insert(key, Arc::downgrade(&lock));
        lock
    }
}

type BiomeCache = Mutex<HashMap<(i32, i32), Arc<BiomeChunk<BiomeId>>>>;

fn state(registry: &BlockRegistry, raw: &Value) -> Result<BlockStateId, GeneratorError> {
    let name = raw
        .get("Name")
        .and_then(Value::as_str)
        .or_else(|| raw.as_str())
        .ok_or_else(|| resource("block state"))?;
    let properties = raw.get("Properties").and_then(Value::as_object);
    match properties {
        Some(properties) if !properties.is_empty() => {
            let descriptor = crate::chunk::BlockState {
                name: super::tags::identifier(name)?,
                properties: properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect(),
            };
            registry
                .resolve(&descriptor)
                .map_err(|e| resource(format!("block state {name}: {e}")))
        }
        _ => registry
            .default_state(&super::tags::identifier(name)?)
            .ok_or_else(|| resource(format!("unknown block {name}"))),
    }
}

impl<R: RandomSource> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    fn new(
        data: Arc<GeneratorData>,
        seed: i64,
        mut random: R,
        generator: &Value,
        settings_json: &Value,
        with_structures: bool,
        level: Option<(i32, i32)>,
    ) -> Result<Self, GeneratorError> {
        let registry = data.registry;
        let noise = &settings_json["noise"];
        let min_y = noise["min_y"]
            .as_i64()
            .ok_or_else(|| resource("noise min_y"))? as i32;
        let height = noise["height"]
            .as_i64()
            .ok_or_else(|| resource("noise height"))? as i32;
        let (level_min_y, level_height) = level.unwrap_or((min_y, height));
        if min_y < level_min_y || min_y + height > level_min_y + level_height {
            return Err(resource("noise range outside the dimension's build height"));
        }
        let settings = Settings {
            noise_min_y: min_y,
            noise_height: height,
            level_min_y,
            level_height,
            sea_level: settings_json["sea_level"]
                .as_i64()
                .ok_or_else(|| resource("sea_level"))? as i32,
            default_block: state(registry, &settings_json["default_block"])?,
            default_fluid: state(registry, &settings_json["default_fluid"])?,
            legacy: settings_json["legacy_random_source"]
                .as_bool()
                .unwrap_or(false),
        };
        let root = random.fork_positional();
        let codec = crate::registry::BlockStateJsonCodec::new(registry, data.integers.clone())
            .map_err(|e| resource(e.to_string()))?;
        let mut densities = DensityRegistry::from_json(&data.noises, &data.densities)?;
        let router =
            NoiseRouter::from_json(&settings_json["noise_router"].to_string(), &mut densities)?;
        // NoiseGeneratorSettings.debugFunctions: a registered density or an
        // inline one, each with its label.
        let debug_functions = settings_json
            .get("debug_functions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|entry| {
                let label = entry["label"]
                    .as_str()
                    .ok_or_else(|| resource("debug label"))?;
                let id = match &entry["function"] {
                    Value::String(name) => densities
                        .reference(name)
                        .ok_or_else(|| resource(format!("debug function {name}")))?,
                    inline => densities.decode_functions(&[&inline.to_string()])?[0],
                };
                Ok((label.to_owned(), id))
            })
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let aquifers = settings_json
            .get("aquifers")
            .filter(|v| !v.is_null())
            .map(|v| AquiferConfig::from_json(&v.to_string(), &mut densities))
            .transpose()?;
        let mut library = MaterialRuleLibrary::from_json(
            &data.rules,
            &data.conditions,
            &codec,
            densities,
            seed,
            settings.legacy,
        )?;
        let router = library.compile_router(router)?;
        let debug_density = debug_functions
            .into_iter()
            .map(|(label, id)| Ok((label, library.function_sampler(id)?)))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let aquifer = aquifers
            .map(|config| library.compile_aquifer(config))
            .transpose()?
            .map(|samplers| AquiferField::ALL.map(|field| samplers.get(field).clone()));
        let rule_name = settings_json["material_rule"]
            .as_str()
            .ok_or_else(|| resource("material_rule"))?;
        let rule = library.rule(rule_name)?;
        let preliminary = router.get(RouterField::ChunkSurfaceLevel).clone();
        let spawn_target = match settings_json.get("spawn_target") {
            Some(Value::Array(points)) => points
                .iter()
                .map(|point| {
                    let Value::Object(fields) = point else {
                        return Err(resource("spawn_target point"));
                    };
                    let parameters = fields
                        .iter()
                        .map(|(name, interval)| {
                            Ok((
                                library.named_density(name)?,
                                decode_parameter(&interval.to_string())?,
                            ))
                        })
                        .collect::<Result<_, GeneratorError>>()?;
                    Ok(SpawnTargetPoint { parameters })
                })
                .collect::<Result<_, GeneratorError>>()?,
            _ => Vec::new(),
        };
        let source_json = generator["biome_source"].to_string();
        let source = data
            .source_catalog
            .decode(&source_json)?
            .resolve(&data.presets)
            .map_err(|e| resource(format!("biome source: {e}")))?;
        let climate = router.climate_sampler();
        let mut noises = BTreeMap::new();
        for (name, json) in &data.noises {
            let noise: Arc<dyn Noise> = Arc::new(
                decode_noise(json)?
                    .create(&mut root.from_hash_of(name))
                    .map_err(DecodeError::from)?,
            );
            noises.insert(name.clone(), noise);
        }
        let noise = |name: &str| -> Result<Arc<dyn Noise>, GeneratorError> {
            noises
                .get(name)
                .cloned()
                .ok_or_else(|| resource(format!("missing noise {name}")))
        };
        let surface = MaterialSurface::new(
            root.clone(),
            ClayBandBlocks::from_registry(registry).ok_or_else(|| resource("clay bands"))?,
            noise("minecraft:clay_bands_offset")?,
            noise("minecraft:surface")?,
            noise("minecraft:surface_secondary")?,
        );
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| resource(format!("missing block {name}")))
        };
        let extensions = MaterialExtensions::new(
            root.clone(),
            ExtensionBlocks {
                default: settings.default_block,
                water: default("minecraft:water")?,
                packed_ice: default("minecraft:packed_ice")?,
                snow: default("minecraft:snow_block")?,
            },
            ExtensionNoises {
                badlands_surface: noise("minecraft:badlands_surface")?,
                badlands_pillar: noise("minecraft:badlands_pillar")?,
                badlands_roof: noise("minecraft:badlands_pillar_roof")?,
                iceberg_surface: noise("minecraft:iceberg_surface")?,
                iceberg_pillar: noise("minecraft:iceberg_pillar")?,
                iceberg_roof: noise("minecraft:iceberg_pillar_roof")?,
            },
            settings.sea_level,
        );
        let picker = GlobalFluidPicker::new(
            settings.sea_level,
            settings.default_fluid,
            default("minecraft:lava")?,
            default("minecraft:air")?,
        );
        let mut pipeline = Self {
            zoom: BiomeZoom::new(obfuscate_seed(seed)),
            data,
            seed,
            settings,
            root,
            router,
            climate,
            aquifer,
            rule,
            preliminary,
            source,
            surface,
            noises,
            extensions,
            picker,
            biome_cache: Mutex::new(HashMap::new()),
            biome_work: ChunkWork::new(),
            carver_cache: Mutex::new(HashMap::new()),
            decoration: None,
            structures: None,
            spawn_target,
            old_world: std::sync::OnceLock::new(),
            debug_density,
            spawn_position: std::sync::OnceLock::new(),
            flat: None,
        };
        pipeline.decoration = pipeline
            .data
            .decoration_for(&source_json, &pipeline.source)?;
        if with_structures {
            let possible = pipeline
                .source
                .possible_biomes()
                .into_iter()
                .filter_map(|name| pipeline.data.biome_id(name))
                .collect();
            let rings = pipeline.data.rings_for(seed, generator);
            pipeline.structures =
                super::generator_structures::StructureState::new(&pipeline.data, &possible, rings)?;
            if let Some(state) = pipeline.structures.as_mut() {
                state.fixed_biome = generator["biome_source"]["type"] == "minecraft:fixed";
            }
        }
        Ok(pipeline)
    }
    fn generation(&self) -> WorldGenerationContext {
        // `new WorldGenerationContext(generator, level)`: the larger minimum Y and the smaller
        // height of the generator and the level (`FlatLevelSource`: 0 and 384).
        if self.flat.is_some() {
            return WorldGenerationContext::new(
                self.settings.level_min_y.max(0),
                self.settings.level_height.min(384),
                self.settings.sea_level,
            );
        }
        WorldGenerationContext::new(
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        )
    }
    /// `Blender.of(region)` for the chunk at `pos`.
    fn blender_at(&self, pos: ChunkPos) -> Arc<super::blending::Blender> {
        // `SharedConstants.DEBUG_DISABLE_BLENDING`.
        static DISABLED: OnceLock<bool> = OnceLock::new();
        if *DISABLED.get_or_init(|| std::env::var_os("HORIZON_DISABLE_BLENDING").is_some()) {
            return Arc::new(super::blending::Blender::empty());
        }
        Arc::new(match self.old_world.get() {
            Some(world) => super::blending::Blender::of(world.as_ref(), pos),
            None => super::blending::Blender::empty(),
        })
    }
    fn biome_chunk(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<Arc<BiomeChunk<BiomeId>>, GeneratorError> {
        self.biome_chunk_or_busy(worker, pos, true)
            .map(|chunk| chunk.expect("waited for the builder"))
    }
    /// The chunk's biomes, built here if no other thread is building them.
    /// With `wait` false, None when another thread is building them now.
    fn biome_chunk_or_busy(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
        wait: bool,
    ) -> Result<Option<Arc<BiomeChunk<BiomeId>>>, GeneratorError> {
        if let Some(chunk) = self.biome_cache.lock().unwrap().get(&(pos.x, pos.z)) {
            return Ok(Some(chunk.clone()));
        }
        // An old chunk keeps the biomes it was saved with.
        if let Some(old) = self
            .old_world
            .get()
            .and_then(|world| world.old_biomes(pos.x, pos.z))
        {
            return Ok(Some(old));
        }
        let work = self.biome_work.at((pos.x, pos.z));
        let _work = if wait {
            work.lock().unwrap()
        } else {
            match work.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
                Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
            }
        };
        if let Some(chunk) = self.biome_cache.lock().unwrap().get(&(pos.x, pos.z)) {
            return Ok(Some(chunk.clone()));
        }
        let (min, height) = (self.settings.level_min_y, self.settings.level_height);
        let mut chunk = BiomeChunk::new(pos, min, height, BiomeId(0)).map_err(stage("biomes"))?;
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let context = &mut worker.biome_context;
        let volume = resolver
            .prepare_volume(
                context,
                [
                    pos.x.wrapping_shl(4) >> 2,
                    min >> 2,
                    pos.z.wrapping_shl(4) >> 2,
                ],
                [4, height >> 2, 4],
            )
            .map_err(stage("biomes"))?;
        let ids = &self.data.biome_ids;
        let blender = self.blender_at(pos);
        // Names point into the source's parameter list: neighbouring quarts
        // mostly resolve to the same entry, so skip the map lookup for it.
        let mut last: Option<(*const str, BiomeId)> = None;
        chunk
            .fill(min, min + height - 1, |quart| -> Result<BiomeId, String> {
                // `Blender.getBiomeResolver`: an old chunk's biome near it.
                if !blender.is_empty()
                    && let Some(old) = blender.blend_biome(quart[0], quart[1], quart[2])
                {
                    return Ok(old);
                }
                let name = resolver
                    .sample_prepared(context, volume.as_ref(), quart)
                    .map_err(|e| e.to_string())?;
                if let Some((pointer, id)) = last {
                    if std::ptr::eq(pointer, name) {
                        return Ok(id);
                    }
                }
                let id = ids.get(name).copied().ok_or_else(|| name.to_owned())?;
                last = Some((name as *const str, id));
                Ok(id)
            })
            .map_err(stage("biomes"))?;
        let chunk = Arc::new(chunk);
        let mut cache = self.biome_cache.lock().unwrap();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert((pos.x, pos.z), chunk.clone());
        Ok(Some(chunk))
    }
    fn carver_biome(&self, worker: &mut WorkerState, origin: ChunkPos) -> Result<BiomeId, String> {
        let key = (origin.x, origin.z);
        if let Some(id) = worker.carver_biomes.get(&key) {
            return Ok(*id);
        }
        if worker.carver_biomes.len() >= 4096 {
            worker.carver_biomes.clear();
        }
        if let Some(id) = self.carver_cache.lock().unwrap().get(&key) {
            worker.carver_biomes.insert(key, *id);
            return Ok(*id);
        }
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let quart = [
            origin.x.wrapping_shl(4) >> 2,
            0,
            origin.z.wrapping_shl(4) >> 2,
        ];
        let name = resolver
            .sample(&mut worker.carver_context, quart)
            .map_err(|e| e.to_string())?;
        let id = self.data.biome_id(name).ok_or_else(|| name.to_owned())?;
        let mut cache = self.carver_cache.lock().unwrap();
        if cache.len() >= CACHE_LIMIT * 4 {
            cache.clear();
        }
        cache.insert(key, id);
        worker.carver_biomes.insert(key, id);
        Ok(id)
    }
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError> {
        let mut context = std::mem::take(&mut worker.terrain_context);
        let result = self.generate_with_context(worker, pos, &mut context);
        // Release chunk-specific bindings and caches even after a sampling error.
        context.reset();
        worker.terrain_context = context;
        result
    }
    /// With the native End source there are no badlands/iceberg extensions.
    /// Disabled aquifers with air as their default fluid can only add lava or
    /// air to the default solid, so this constant rule would rewrite each solid
    /// with the state it already has. Check their imported material flags too.
    /// Noise filling already built the WG maps; these writes add no fluid marks.
    fn identity_end_surface(&self, pos: ChunkPos) -> bool {
        if !matches!(self.source, ResolvedBiomeSource::End(_))
            // At wrapped block coordinates the ordinary biome-region lookup
            // can fail; retain that lookup and its error outside this range.
            || ![pos.x, pos.z].into_iter().all(|c| {
                c.checked_mul(16).is_some_and(|b| {
                    b.checked_sub(16).is_some() && b.checked_add(31).is_some()
                })
            })
            || self.aquifer.is_some()
            || Some(self.settings.default_block)
                != self.data.registry.default_state("minecraft:end_stone")
            || Some(self.settings.default_fluid)
                != self.data.registry.default_state("minecraft:air")
            || !self.data.states.get(self.settings.default_fluid).air
            || !self
                .data
                .registry
                .default_state("minecraft:lava")
                .is_some_and(|lava| self.data.states.get(lava).fluid)
        {
            return false;
        }
        let mut rule = &self.rule;
        // Restrict the shortcut to shallow block references; other rules still
        // run their normal compilation, queries and error handling.
        for _ in 0..32 {
            match rule {
                MaterialRule::Reference(next) => rule = next,
                MaterialRule::Block(state) => return *state == self.settings.default_block,
                _ => return false,
            }
        }
        false
    }
    fn generate_with_context(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
        context: &mut SamplerContext,
    ) -> Result<ProtoChunk, GeneratorError> {
        let data = &self.data;
        let (min, height, sea) = (
            self.settings.noise_min_y,
            self.settings.noise_height,
            self.settings.sea_level,
        );
        let mut clock = std::time::Instant::now();
        let mut neighbors = Vec::with_capacity(9);
        // The biome ids present, as a bitset: ascending like a sorted set.
        let mut seen: Vec<u64> = Vec::new();
        // Build the neighbours no other thread is building first, then wait
        // for the rest (each is built once, by whichever thread gets it).
        let around =
            |i: i32| ChunkPos::new(pos.x.wrapping_add(i % 3 - 1), pos.z.wrapping_add(i / 3 - 1));
        let mut ready: [Option<Arc<BiomeChunk<BiomeId>>>; 9] = Default::default();
        for (i, slot) in ready.iter_mut().enumerate() {
            *slot = self.biome_chunk_or_busy(worker, around(i as i32), false)?;
        }
        for (i, slot) in ready.iter_mut().enumerate() {
            let chunk = match slot.take() {
                Some(chunk) => chunk,
                None => self.biome_chunk(worker, around(i as i32))?,
            };
            {
                let mut section = 0;
                let mut last = None;
                while let Some(biomes) = chunk.section(section) {
                    for &id in biomes {
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
                neighbors.push((*chunk).clone());
            }
        }
        let possible = seen.iter().enumerate().flat_map(|(word, &bits)| {
            let mut bits = bits;
            std::iter::from_fn(move || {
                (bits != 0).then(|| {
                    let bit = bits.trailing_zeros();
                    bits &= bits - 1;
                    BiomeId(word as u32 * 64 + bit)
                })
            })
        });
        let center = neighbors[4].clone();
        let biomes =
            BiomeRegion::new(neighbors, obfuscate_seed(self.seed)).map_err(stage("biomes"))?;
        let possible = possible.map(|id| data.biome_name(id).to_owned()).collect();
        worker.timings.biomes += lap(&mut clock);
        let mut chunk = TerrainChunk::new(
            pos,
            self.settings.level_min_y,
            self.settings.level_height,
            data.registry,
            data.states.clone(),
        )
        .map_err(stage("noise"))?;
        chunk.set_generation_stage(PreLightChunkStage::Biomes);
        let (references, own_starts) = match &self.structures {
            Some(structures) => {
                let mut queries = StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                };
                (
                    structures.references(&mut queries, pos),
                    structures.starts_at(&mut queries, pos),
                )
            }
            None => (Vec::new(), Starts::default()),
        };
        // `FlatLevelSource.buildTerrain`: the layers, and nothing else (no noise, surface or carvers).
        if let Some(flat) = &self.flat {
            for (i, &state) in flat.states.iter().enumerate() {
                let y = flat.min_y + i as i32;
                if y >= self.settings.level_min_y + self.settings.level_height {
                    break;
                }
                if flat.null[i] || data.states.get(state).is_air() {
                    continue;
                }
                for z in 0..16 {
                    for x in 0..16 {
                        chunk
                            .set_block_raw(
                                pos.x.wrapping_shl(4) + x,
                                y,
                                pos.z.wrapping_shl(4) + z,
                                state,
                            )
                            .map_err(stage("flat"))?;
                    }
                }
            }
            chunk
                .prime_heightmaps(&[
                    HeightmapType::WorldSurfaceWg,
                    HeightmapType::OceanFloorWg,
                    HeightmapType::MotionBlocking,
                    HeightmapType::MotionBlockingNoLeaves,
                    HeightmapType::OceanFloor,
                    HeightmapType::WorldSurface,
                ])
                .map_err(stage("heightmaps"))?;
            chunk.set_generation_stage(PreLightChunkStage::Terrain);
            return Ok(ProtoChunk {
                terrain: chunk,
                biomes: center,
                data: data.clone(),
                decorated: false,
                block_entities: Vec::new(),
                structure_starts: references,
                own_starts,
                entities: Vec::new(),
            });
        }
        let volume = DensityVolume::new(
            [16, height, 16],
            [pos.x.wrapping_shl(4), min, pos.z.wrapping_shl(4)],
            [1; 3],
        )
        .map_err(stage("noise"))?;
        let blender = self.blender_at(pos);
        let beard = self
            .structures
            .as_ref()
            .and_then(|s| s.beardifier(&references, pos));
        if beard.is_some() || !blender.is_empty() {
            let mut fields = horizon_core::density::WorldFields::default();
            if let Some(beard) = beard {
                fields = fields.with_field(
                    horizon_core::density::WorldField::Beardifier,
                    Arc::new(beard),
                );
            }
            if !blender.is_empty() {
                let buffer = Arc::new(blender.output_buffer(volume));
                for (field, offset) in [
                    (horizon_core::density::WorldField::BlendAlpha, false),
                    (horizon_core::density::WorldField::BlendOffset, true),
                ] {
                    fields = fields.with_field(
                        field,
                        Arc::new(super::blending::BlendFieldSampler {
                            buffer: buffer.clone(),
                            offset,
                        }),
                    );
                }
                fields = fields.with_blender(blender.clone());
            }
            *context = std::mem::take(context).with_world_fields(fields);
        }
        worker.timings.structures += lap(&mut clock);
        worker.timings.noise += lap(&mut clock);
        let mut aquifer: Box<dyn TerrainAquifer> = match &self.aquifer {
            Some(samplers) => Box::new(
                NoiseBasedAquifer::new(
                    AquiferSamplers::new(samplers.clone()),
                    self.picker,
                    self.root
                        .from_hash_of("minecraft:aquifer")
                        .fork_positional(),
                    RegistryAquiferBlockStates::new(data.registry).map_err(stage("aquifer"))?,
                    volume,
                    context,
                )
                .map_err(stage("aquifer"))?,
            ),
            None => Box::new(DisabledAquifer::new(
                self.picker,
                data.registry.default_state("minecraft:air").unwrap(),
            )),
        };
        worker.timings.noise += lap(&mut clock);
        fill_noise(
            &mut chunk,
            volume,
            self.router.get(RouterField::FinalDensity).as_ref(),
            context,
            aquifer.as_mut(),
            self.settings.default_block,
        )
        .map_err(stage("noise"))?;
        let generation = self.generation();
        worker.timings.noise += lap(&mut clock);
        if !self.identity_end_surface(pos) {
            build_surface(
                &mut chunk,
                self.rule.clone(),
                SurfaceInputs::<R, _> {
                    generation,
                    noise_volume: volume,
                    root: self.root.clone(),
                    surface: SurfaceQueries {
                        pipeline: self,
                        temperatures: &mut worker.temperatures,
                        biome: Biomes::Region(&biomes),
                        uniform_biomes: Some(UniformSurfaceBiomes::new(
                            pos,
                            self.settings.level_min_y,
                            self.settings.level_height,
                        )),
                        possible: Some(possible),
                    },
                    preliminary_surface: self.preliminary.clone(),
                    density_context: context,
                    write_min_y: min,
                    write_max_y: min + height - 1,
                },
                &self.extensions,
            )
            .map_err(stage("surface"))?;
        }
        worker.timings.surface += lap(&mut clock);
        let carvers = &data.biome_carvers;
        let mask = generate_carving_mask(self.seed, pos, generation, false, |origin| {
            self.carver_biome(worker, origin)
                .map(|id| carvers[id.0 as usize].as_slice())
        })
        .map_err(stage("carvers"))?;
        worker.timings.carvers += lap(&mut clock);
        if !mask.is_empty() {
            let mut resolver = self.source.resolver(
                &self.climate,
                self.router.get(RouterField::Erosion).as_ref(),
            );
            let mut biome_context = SamplerContext::default();
            apply_carving_mask(
                &mut chunk,
                &mask,
                self.rule.clone(),
                TopMaterialInputs::<R, _> {
                    generation,
                    root: self.root.clone(),
                    surface: SurfaceQueries {
                        pipeline: self,
                        temperatures: &mut worker.temperatures,
                        biome: Biomes::Resolver(&mut resolver, &mut biome_context),
                        uniform_biomes: None,
                        possible: None,
                    },
                    preliminary_surface: self.preliminary.clone(),
                    density_context: context,
                },
                aquifer.as_mut(),
                &data.carving_blocks,
                |[x, y, z]| blender.carving_filter(x, y, z).unwrap_or(true),
            )
            .map_err(stage("carvers"))?;
        }
        worker.timings.carvers += lap(&mut clock);
        chunk
            .prime_heightmaps(&[
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
                HeightmapType::OceanFloor,
                HeightmapType::WorldSurface,
            ])
            .map_err(stage("heightmaps"))?;
        chunk.set_generation_stage(PreLightChunkStage::Terrain);
        let _ = sea;
        Ok(ProtoChunk {
            terrain: chunk,
            biomes: center,
            data: data.clone(),
            decorated: false,
            block_entities: Vec::new(),
            structure_starts: references,
            own_starts,
            entities: Vec::new(),
        })
    }
}

impl<R: RandomSource> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    /// WorldGenRegion's level random, positioned at the center chunk.
    fn region_random_at(&self, center: ChunkPos) -> R {
        self.root
            .from_hash_of("minecraft:worldgen_region_random")
            .fork_positional()
            .at(BlockPos::new(
                center.x.wrapping_shl(4),
                0,
                center.z.wrapping_shl(4),
            ))
    }
    /// The Y range `getBaseHeight` and `getBaseColumn` answer for: the noise's, or a superflat
    /// world's whole level (its layers are not bound by any noise settings).
    fn column_range(&self) -> (i32, i32) {
        if self.flat.is_some() {
            (self.settings.level_min_y, self.settings.level_height)
        } else {
            (self.settings.noise_min_y, self.settings.noise_height)
        }
    }
    /// ChunkGenerator.getBaseHeight: the first free Y above the noise column
    /// (with aquifers, without terrain adaptation) for a heightmap type.
    fn base_height(&self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        let (min, _) = self.column_range();
        let mut found = min;
        self.noise_column(x, z, &mut |y, state| {
            if kind.is_opaque(self.data.states.get(state)) {
                found = y + 1;
                return true;
            }
            false
        });
        found
    }
    /// ChunkGenerator.getBaseColumn: opacity of each noise-column state for
    /// a heightmap type, indexed from the minimum Y.
    fn base_column(&self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        self.base_columns(x, z, &[kind]).pop().unwrap_or_default()
    }
    /// One noise-column pass, as opacities for each heightmap kind.
    fn base_columns(&self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>> {
        let (min, height) = self.column_range();
        let mut columns = vec![vec![false; height as usize]; kinds.len()];
        self.noise_column(x, z, &mut |y, state| {
            let state = self.data.states.get(state);
            for (column, kind) in columns.iter_mut().zip(kinds) {
                column[(y - min) as usize] = kind.is_opaque(state);
            }
            false
        });
        columns
    }
    /// ChunkGenerator.getBaseColumn's blocks, by what structures test
    /// (BlockState.isSolid is blocksMotion for the noise's blocks, which
    /// are never cobwebs or bamboo saplings).
    fn noise_blocks(&self, x: i32, z: i32) -> super::structure::start::NoiseColumn {
        use super::structure::start::{NoiseBlock, NoiseColumn};
        let (min, height) = self.column_range();
        let mut blocks = vec![NoiseBlock::Air; height as usize];
        let registry = self.data.registry;
        self.noise_column(x, z, &mut |y, state| {
            let props = self.data.states.get(state);
            blocks[(y - min) as usize] = if props.is_air() {
                NoiseBlock::Air
            } else {
                match registry.state(state).map(|s| s.name.as_str()) {
                    Some("minecraft:water") => NoiseBlock::Water,
                    Some("minecraft:lava") => NoiseBlock::Lava,
                    _ if props.blocks_motion_in_heightmap() => NoiseBlock::Solid,
                    _ => NoiseBlock::Other,
                }
            };
            false
        });
        NoiseColumn { min, blocks }
    }
    /// iterateNoiseColumn from the top down until `visit` returns true.
    fn noise_column(&self, x: i32, z: i32, visit: &mut dyn FnMut(i32, BlockStateId) -> bool) {
        // `FlatLevelSource.getBaseHeight` and `getBaseColumn`: the layers, and air above them.
        if let Some(flat) = &self.flat {
            let air = self.data.registry.default_state("minecraft:air");
            let top = self.settings.level_min_y + self.settings.level_height;
            for y in (self.settings.level_min_y..top).rev() {
                let i = (y - flat.min_y) as usize;
                let state = match flat.states.get(i) {
                    Some(&state) if !flat.null[i] => state,
                    _ => match air {
                        Some(air) => air,
                        None => return,
                    },
                };
                if visit(y, state) {
                    return;
                }
            }
            return;
        }
        let (min, height) = (self.settings.noise_min_y, self.settings.noise_height);
        let Ok(volume) = DensityVolume::new([1, height, 1], [x, min, z], [1; 3]) else {
            return;
        };
        let mut context = SamplerContext::cached();
        let mut aquifer: Box<dyn TerrainAquifer> = match &self.aquifer {
            Some(samplers) => match RegistryAquiferBlockStates::new(self.data.registry)
                .ok()
                .and_then(|blocks| {
                    NoiseBasedAquifer::new(
                        AquiferSamplers::new(samplers.clone()),
                        self.picker,
                        self.root
                            .from_hash_of("minecraft:aquifer")
                            .fork_positional(),
                        blocks,
                        volume,
                        &mut context,
                    )
                    .ok()
                }) {
                Some(aquifer) => Box::new(aquifer),
                None => return,
            },
            None => Box::new(DisabledAquifer::new(
                self.picker,
                self.data.registry.default_state("minecraft:air").unwrap(),
            )),
        };
        let mut density = vec![0.; height as usize];
        if self
            .router
            .get(RouterField::FinalDensity)
            .sample_volume(&mut context, &mut density, volume)
            .is_err()
        {
            return;
        }
        for y in (0..height).rev() {
            let state = aquifer
                .compute_substance(
                    &mut context,
                    [x, min + y, z],
                    f64::from(density[y as usize]),
                )
                .ok()
                .flatten()
                .unwrap_or(self.settings.default_block);
            if visit(min + y, state) {
                return;
            }
        }
    }
}
/// Biome and height queries for structure generation on one worker.
pub(super) struct StructureQueries<'a, R: RandomSource> {
    pipeline: &'a Pipeline<R>,
    worker: &'a mut WorkerState,
    valid: BTreeSet<BiomeId>,
}
impl<R: RandomSource> super::structure::jigsaw::JigsawContext for StructureQueries<'_, R>
where
    R::Positional: Clone + Send + Sync,
{
    // Structure.GenerationContext.heightAccessor: the chunk, so the level's range.
    fn min_y(&self) -> i32 {
        self.pipeline.settings.level_min_y
    }
    fn max_y(&self) -> i32 {
        self.pipeline.settings.level_min_y + self.pipeline.settings.level_height - 1
    }
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        if let Some(h) = self.worker.heights.get(&(x, z, kind as u8)) {
            return *h;
        }
        let h = self.pipeline.base_height(x, z, kind);
        if self.worker.heights.len() > 65536 {
            self.worker.heights.clear();
        }
        self.worker.heights.insert((x, z, kind as u8), h);
        h
    }
    /// JigsawPlacement.couldStructureExistInColumn: one resolver for the
    /// column (createResolverForChunk, a bulk climate volume of 1 x n x 1
    /// quarts, as the biome stage samples a chunk), then each quart from the
    /// bottom. One volume costs about what a few single climate samples do;
    /// a column without a valid biome took one sample per quart (~500 in To
    /// The Max).
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let (qx, qz, qmin, qmax) = (x >> 2, z >> 2, min_y >> 2, max_y >> 2);
        if qmin > qmax {
            return false;
        }
        let pipeline = self.pipeline;
        let mut resolver = pipeline.source.resolver(
            &pipeline.climate,
            pipeline.router.get(RouterField::Erosion).as_ref(),
        );
        let context = &mut self.worker.biome_context;
        let volume = resolver
            .prepare_volume(context, [qx, qmin, qz], [1, qmax - qmin + 1, 1])
            .ok()
            .flatten();
        (qmin..=qmax).any(|qy| {
            let id = resolver
                .sample_prepared(context, volume.as_ref(), [qx, qy, qz])
                .ok()
                .and_then(|name| pipeline.data.biome_id(name))
                .unwrap_or(BiomeId(0));
            self.valid.contains(&id)
        })
    }
}
impl<R: RandomSource> super::structure::start::StartContext for StructureQueries<'_, R>
where
    R::Positional: Clone + Send + Sync,
{
    fn is_old_chunk(&self, chunk: ChunkPos) -> bool {
        self.pipeline
            .old_world
            .get()
            .is_some_and(|world| world.is_old_chunk_around(chunk, 0))
    }
    fn saved_starts(&self, chunk: ChunkPos) -> Vec<super::blending::SavedStart> {
        self.pipeline
            .old_world
            .get()
            .map(|world| world.saved_starts(chunk.x, chunk.z))
            .unwrap_or_default()
    }
    fn seed(&self) -> i64 {
        self.pipeline.seed
    }
    fn sea_level(&self) -> i32 {
        self.pipeline.settings.sea_level
    }
    fn generator_min_y(&self) -> i32 {
        if self.pipeline.flat.is_some() {
            0
        } else {
            self.pipeline.settings.noise_min_y
        }
    }
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        self.pipeline.base_column(x, z, kind)
    }
    fn base_columns(&mut self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>> {
        self.pipeline.base_columns(x, z, kinds)
    }
    fn noise_column(&mut self, x: i32, z: i32) -> super::structure::start::NoiseColumn {
        self.pipeline.noise_blocks(x, z)
    }
    fn cold_enough_to_snow(&mut self, biome: BiomeId, pos: [i32; 3]) -> bool {
        let sea = self.pipeline.settings.sea_level;
        self.worker.temperatures[biome.0 as usize]
            .cold_enough_to_snow(BlockPos::new(pos[0], pos[1], pos[2]), sea)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.pipeline.generation()
    }
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId {
        if let Some(&id) = self.worker.structure_biomes.get(&quart) {
            return id;
        }
        let id = self.noise_biomes(&[quart])[0];
        if self.worker.structure_biomes.len() >= 1 << 16 {
            self.worker.structure_biomes.clear();
        }
        self.worker.structure_biomes.insert(quart, id);
        id
    }
    fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<BiomeId> {
        let mut resolver = self.pipeline.source.resolver(
            &self.pipeline.climate,
            self.pipeline.router.get(RouterField::Erosion).as_ref(),
        );
        quarts
            .iter()
            .map(|&quart| {
                resolver
                    .sample(&mut self.worker.carver_context, quart)
                    .ok()
                    .and_then(|name| self.pipeline.data.biome_id(name))
                    .unwrap_or(BiomeId(0))
            })
            .collect()
    }
    /// `None` is `/place structure`'s `biome -> true`: every biome the bundle
    /// knows counts as valid.
    fn set_valid_biomes(&mut self, biomes: Option<&BTreeSet<BiomeId>>) {
        self.valid = match biomes {
            Some(biomes) => biomes.clone(),
            None => (0..self.pipeline.data.biome_names().len() as u32)
                .map(BiomeId)
                .collect(),
        };
    }
}

enum Biomes<'a, 'b> {
    Region(&'a BiomeRegion<BiomeId>),
    Resolver(
        &'a mut super::biome_source::BiomeSourceResolver<'b>,
        &'a mut SamplerContext,
    ),
    /// A resolver whose answers are kept per quart: far sampling paints eight
    /// depths of every column, and neighbouring columns share quarts, so most
    /// queries repeat one (each is a full climate sample and parameter search).
    Memo(
        &'a mut super::biome_source::BiomeSourceResolver<'b>,
        &'a mut SamplerContext,
        &'a mut BiomeMemo,
    ),
}
/// Resolved biomes by quart (`Biomes::Memo`).
struct BiomeMemo {
    biomes: HashMap<[i32; 3], BiomeId>,
    /// Quarts per side of the columns sharing one biome, the first one asked
    /// (its surface's); 0 for one per quart. Far cells wider than a quart
    /// don't show a biome change a few blocks down, and climate changes over
    /// hundreds of blocks: a far tile shares one per 4x4 of its columns, in
    /// cells up to 64 blocks wide.
    /// (Painting asks through the biome zoom, which strays into neighbouring
    /// quarts: per quart, those strays were most of the climate samples.)
    cell: i32,
}
/// The biome at `quart`, resolved once per quart (`Biomes::Memo`).
fn memo_biome(
    data: &GeneratorData,
    resolver: &mut super::biome_source::BiomeSourceResolver<'_>,
    context: &mut SamplerContext,
    memo: &mut BiomeMemo,
    quart: [i32; 3],
) -> Result<BiomeId, MaterialRuleError> {
    let key = if memo.cell > 0 {
        [
            quart[0].div_euclid(memo.cell),
            0,
            quart[2].div_euclid(memo.cell),
        ]
    } else {
        quart
    };
    if let Some(&id) = memo.biomes.get(&key) {
        return Ok(id);
    }
    let name = resolver
        .sample(context, quart)
        .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
    let id = data.biome_id(name).ok_or(MaterialRuleError::MissingBiome)?;
    memo.biomes.insert(key, id);
    Ok(id)
}
// Per quart cell of a block-biome lookup: a lazy equality proof for its
// eight corners, or (when they differ) their jitter and biomes, so the blocks
// of the cell only redo the distance comparison.
#[derive(Clone, Copy, PartialEq)]
enum SurfaceCell {
    Unknown,
    /// A corner lookup failed: only the original lookup may report errors.
    Lookup,
    Uniform(BiomeId),
    Mixed(u32),
}
struct MixedCell {
    jitter: [[f64; 3]; 8],
    biomes: [BiomeId; 8],
}
struct UniformSurfaceBiomes {
    origin: [i32; 3],
    height: i32,
    cells: Vec<SurfaceCell>,
    mixed: Vec<MixedCell>,
    /// The cell of the previous lookup (columns are walked down in y).
    last: ([i32; 3], SurfaceCell),
}
impl UniformSurfaceBiomes {
    fn new(pos: ChunkPos, min_y: i32, height: i32) -> Self {
        let height = (height >> 2) + 1;
        Self {
            origin: [
                pos.x.wrapping_shl(4).wrapping_sub(2) >> 2,
                min_y.wrapping_sub(2) >> 2,
                pos.z.wrapping_shl(4).wrapping_sub(2) >> 2,
            ],
            height,
            cells: vec![SurfaceCell::Unknown; 25 * height as usize],
            mixed: Vec::new(),
            last: ([0; 3], SurfaceCell::Unknown),
        }
    }
    #[inline]
    fn get(&mut self, region: &BiomeRegion<BiomeId>, p: BlockPos) -> Option<BiomeId> {
        let base = [p.x, p.y, p.z].map(|v| v.wrapping_sub(2) >> 2);
        let cell = if self.last.0 == base && self.last.1 != SurfaceCell::Unknown {
            self.last.1
        } else {
            let cell = self.cell(region, base);
            self.last = (base, cell);
            cell
        };
        match cell {
            SurfaceCell::Uniform(id) => Some(id),
            SurfaceCell::Mixed(index) => {
                let mixed = &self.mixed[index as usize];
                Some(mixed.biomes[BiomeZoom::select_corner(p, &mixed.jitter)])
            }
            SurfaceCell::Unknown | SurfaceCell::Lookup => None,
        }
    }
    fn cell(&mut self, region: &BiomeRegion<BiomeId>, base: [i32; 3]) -> SurfaceCell {
        let d: [i32; 3] = std::array::from_fn(|i| base[i].wrapping_sub(self.origin[i]));
        if !(0..5).contains(&d[0]) || !(0..self.height).contains(&d[1]) || !(0..5).contains(&d[2]) {
            return SurfaceCell::Lookup;
        }
        let slot = &mut self.cells[((d[0] * 5 + d[2]) * self.height + d[1]) as usize];
        if *slot != SurfaceCell::Unknown {
            return *slot;
        }
        // These are immutable stored lookups, in quart_at_block's corner order.
        let mut biomes = [BiomeId(0); 8];
        for (corner, biome) in biomes.iter_mut().enumerate() {
            let offsets = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            match region.noise_biome(std::array::from_fn(|i| {
                base[i].wrapping_add(offsets[i] as i32)
            })) {
                Ok(id) => *biome = *id,
                Err(_) => {
                    *slot = SurfaceCell::Lookup;
                    return SurfaceCell::Lookup;
                }
            }
        }
        let value = if biomes.iter().all(|id| *id == biomes[0]) {
            SurfaceCell::Uniform(biomes[0])
        } else {
            self.mixed.push(MixedCell {
                jitter: region.zoom().corner_jitter(base),
                biomes,
            });
            SurfaceCell::Mixed(self.mixed.len() as u32 - 1)
        };
        *slot = value;
        value
    }
}
/// Material-rule world queries for one chunk: shared seed-bound noises, the
/// worker's temperature caches and either stored or raw-resolver biomes.
struct SurfaceQueries<'a, 'b, R: RandomSource> {
    pipeline: &'a Pipeline<R>,
    temperatures: &'a mut [BiomeTemperature],
    biome: Biomes<'a, 'b>,
    possible: Option<BTreeSet<String>>,
    uniform_biomes: Option<UniformSurfaceBiomes>,
}
impl<R: RandomSource> SurfaceQueries<'_, '_, R> {
    fn temperature(&mut self, name: &str) -> Result<&mut BiomeTemperature, MaterialRuleError> {
        let id = self
            .pipeline
            .data
            .biome_id(name)
            .ok_or(MaterialRuleError::MissingBiome)?;
        Ok(&mut self.temperatures[id.0 as usize])
    }
}
impl<R: RandomSource> MaterialSurfaceQueries for SurfaceQueries<'_, '_, R> {
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.pipeline.surface.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.pipeline.surface.surface_secondary(x, z)
    }
    fn band(&mut self, p: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.pipeline
            .surface
            .band([p.x, p.y, p.z])
            .ok_or(MaterialRuleError::InvalidBandIndex)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        self.pipeline
            .noises
            .get(&super::tags::identifier(name)?)
            .cloned()
            .ok_or(DecodeError::Field("unknown material noise"))
    }
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        self.possible.clone()
    }
    fn biome(&mut self, p: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        Ok(self
            .biome_id(p)?
            .map(|id| self.pipeline.data.biome_names[id as usize].clone()))
    }
    fn biome_index(&self, name: &str) -> Option<Option<u32>> {
        Some(self.pipeline.data.biome_id(name).map(|id| id.0))
    }
    fn biome_id(&mut self, p: BlockPos) -> Result<Option<u32>, MaterialRuleError> {
        let data = &self.pipeline.data;
        let id = match &mut self.biome {
            Biomes::Region(region) => match self
                .uniform_biomes
                .as_mut()
                .and_then(|cache| cache.get(region, p))
            {
                Some(id) => id,
                None => *region
                    .biome(p)
                    .map_err(|_| MaterialRuleError::MissingBiome)?,
            },
            Biomes::Resolver(resolver, context) => {
                let name = resolver
                    .sample(context, self.pipeline.zoom.quart_at_block(p))
                    .map_err(|_| MaterialRuleError::BiomeQueryFailed)?;
                data.biome_id(name).ok_or(MaterialRuleError::MissingBiome)?
            }
            Biomes::Memo(resolver, context, memo) => memo_biome(
                data,
                resolver,
                context,
                memo,
                self.pipeline.zoom.quart_at_block(p),
            )?,
        };
        Ok(Some(id.0))
    }
    fn cold_enough_to_snow(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self.temperature(name)?.cold_enough_to_snow(p, sea))
    }
    fn melts_frozen_ocean(
        &mut self,
        name: &str,
        p: BlockPos,
        sea: i32,
    ) -> Result<bool, MaterialRuleError> {
        Ok(self
            .temperature(name)?
            .should_melt_frozen_ocean_iceberg_slightly(p, sea))
    }
}

fn lap(clock: &mut std::time::Instant) -> std::time::Duration {
    let now = std::time::Instant::now();
    let elapsed = now - *clock;
    *clock = now;
    elapsed
}
/// Cumulative wall time spent by one worker in each proto-chunk stage.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub chunks: u64,
    pub biomes: std::time::Duration,
    /// Noise fill including aquifer setup.
    pub noise: std::time::Duration,
    pub surface: std::time::Duration,
    /// Structure starts/references and terrain adaptation.
    pub structures: std::time::Duration,
    /// Carving mask generation/application and final heightmap priming.
    pub carvers: std::time::Duration,
}
/// Worker-owned mutable caches. Biome temperature caches mirror vanilla's
/// thread-local caches and never change results.
pub(super) struct WorkerState {
    pub(super) temperatures: Vec<BiomeTemperature>,
    terrain_context: SamplerContext,
    biome_context: SamplerContext,
    carver_context: SamplerContext,
    /// Seed/dimension-bound lookups avoid the shared mutex for overlapping
    /// carver neighborhoods. Bounded independently of the shared cache.
    carver_biomes: HashMap<(i32, i32), BiomeId>,
    /// Noise biomes asked one at a time by structure placement checks
    /// (`StartContext::noise_biome`): several structures test the same
    /// quarts, column checks whole columns, and each is a full climate sample.
    structure_biomes: HashMap<[i32; 3], BiomeId>,
    timings: StageTimings,
    heights: HashMap<(i32, i32, u8), i32>,
    /// Decorate with structure pieces only (for structure parity checks).
    pub(super) structures_only: bool,
    pub(super) structure_mobs: Option<Arc<dyn StructureMobs>>,
}
/// `Mob.finalizeSpawn(level, difficulty, STRUCTURE, null)` of the entities a
/// structure places with finalizeEntities, run by the game, which owns mobs.
pub trait StructureMobs: Send + Sync {
    /// Finalizes the entity in `nbt` if it is a mob, drawing from the
    /// region's random. `biome` is the biome at its block, `structures` the
    /// structures with a piece there. False when it could not: the entity
    /// keeps its `horizon:finalize` for the game to finish on load.
    fn finalize(
        &self,
        nbt: &mut crate::nbt::Compound,
        biome: &str,
        structures: &[String],
        random: &mut dyn super::structure::legacy::PieceRandom,
    ) -> bool;
}

trait SeededPipeline: Send + Sync {
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError>;
    fn decorate(
        &self,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError>;
    fn data(&self) -> &Arc<GeneratorData>;
    /// The exact `end_island_decorated` origins in a chunk, without terrain.
    fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]>;
    fn sea_level(&self) -> i32;
    /// The level's build range (minimum Y, height).
    fn level(&self) -> (i32, i32);
    fn surface_sample(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        spans: Option<&mut Vec<(i32, i32, BlockStateId)>>,
    ) -> Option<SurfaceSample>;
    fn lod_grid(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError>;
    fn structure_starts(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>>;
    /// `Structure.generate` for one structure in one chunk (`/place
    /// structure`), outside any structure set's placement.
    fn structure_start(
        &self,
        worker: &mut WorkerState,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>>;
    fn unsupported_features(&self) -> Vec<(String, String)>;
    fn use_saved_start(
        &self,
        worker: &mut WorkerState,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String>;
    fn forget_structure_starts(&self);
    fn ring_positions(&self, worker: &mut WorkerState, set: &str) -> Option<Vec<ChunkPos>>;
    fn spawn_position(&self) -> Result<[i32; 2], GeneratorError>;
    fn region_random(&self, center: ChunkPos) -> Box<dyn super::structure::legacy::PieceRandom>;
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])>;
    /// The chunks an older version generated, for `Blender.of`.
    fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>);
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`.
    fn debug_density_line(&self, feet: [i32; 3]) -> Option<String>;
    fn find_closest_biome(
        &self,
        worker: &mut WorkerState,
        search: &BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])>;
    /// `BiomeSource.getNoiseBiome` at each quart position (unloaded chunks).
    fn noise_biomes(&self, worker: &mut WorkerState, quarts: &[[i32; 3]]) -> Vec<Option<String>>;
    fn jigsaw_pieces(
        &self,
        worker: &mut WorkerState,
        query: &JigsawQuery<'_>,
    ) -> Option<JigsawPieces>;
}
impl<R: RandomSource + 'static> SeededPipeline for Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    fn generate(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Result<ProtoChunk, GeneratorError> {
        Pipeline::generate(self, worker, pos)
    }
    fn decorate(
        &self,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError> {
        match &self.decoration {
            Some(decoration) => {
                let mut random = self.region_random_at(region[4].position());
                decoration.decorate(
                    &self.data,
                    self.seed,
                    self.generation(),
                    worker,
                    region,
                    &mut random,
                    self.structures.as_ref(),
                )
            }
            None => Err(resource("decoration resources were not loaded")),
        }
    }
    fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]> {
        let Some(decoration) = &self.decoration else {
            return Vec::new();
        };
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let mut context = SamplerContext::cached();
        decoration.origins(self.seed, chunk, |position| {
            let quart = [position[0] >> 2, position[1] >> 2, position[2] >> 2];
            resolver.sample(&mut context, quart).ok() == Some("minecraft:small_end_islands")
        })
    }
    fn sea_level(&self) -> i32 {
        self.settings.sea_level
    }
    fn level(&self) -> (i32, i32) {
        (self.settings.level_min_y, self.settings.level_height)
    }
    fn structure_starts(
        &self,
        worker: &mut WorkerState,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>> {
        match &self.structures {
            Some(structures) => structures
                .starts_at(
                    &mut StructureQueries {
                        pipeline: self,
                        worker,
                        valid: BTreeSet::new(),
                    },
                    pos,
                )
                .to_vec(),
            None => Vec::new(),
        }
    }
    fn structure_start(
        &self,
        worker: &mut WorkerState,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>> {
        self.structures
            .as_ref()?
            .structure_start(
                &mut StructureQueries {
                    pipeline: self,
                    worker,
                    valid: BTreeSet::new(),
                },
                structure,
                chunk,
            )
            .map(Arc::new)
    }
    fn unsupported_features(&self) -> Vec<(String, String)> {
        self.decoration
            .as_ref()
            .map(|decoration| decoration.unsupported())
            .unwrap_or_default()
    }
    fn use_saved_start(
        &self,
        worker: &mut WorkerState,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String> {
        let structures = (self.structures.as_ref()).ok_or("structures were not loaded")?;
        let mut queries = StructureQueries {
            pipeline: self,
            worker,
            valid: BTreeSet::new(),
        };
        structures.use_saved_start(&mut queries, chunk, structure, pieces)
    }
    fn forget_structure_starts(&self) {
        if let Some(structures) = &self.structures {
            structures.forget_starts();
        }
    }
    fn ring_positions(&self, worker: &mut WorkerState, set: &str) -> Option<Vec<ChunkPos>> {
        self.structures.as_ref()?.ring_positions(
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            set,
        )
    }
    fn region_random(&self, center: ChunkPos) -> Box<dyn super::structure::legacy::PieceRandom> {
        Box::new(self.region_random_at(center))
    }
    fn spawn_position(&self) -> Result<[i32; 2], GeneratorError> {
        if let Some(position) = self.spawn_position.get() {
            return Ok(*position);
        }
        // ChunkGenerator.getOrigin is 0 0 without a spawn target.
        let position = if self.spawn_target.is_empty() {
            [0, 0]
        } else {
            find_spawn_position(&self.spawn_target, &mut SamplerContext::cached())
                .map_err(|e| GeneratorError::Stage("spawn search", e.to_string()))?
        };
        Ok(*self.spawn_position.get_or_init(|| position))
    }
    fn locate(
        &self,
        worker: &mut WorkerState,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])> {
        self.structures.as_ref()?.locate(
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            names,
            from,
            radius,
            skip,
        )
    }
    fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>) {
        let _ = self.old_world.set(world);
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the labelled values of
    /// the settings' `debug_functions` at `feet`, three decimals each.
    fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        if self.debug_density.is_empty() {
            return None;
        }
        let mut context = SamplerContext::cached();
        let mut line = String::from("Density ");
        for (label, sampler) in &self.debug_density {
            let value = sampler.sample_value(&mut context, feet).ok()?;
            line.push_str(&format!("{label}: {:.3} ", f64::from(value)));
        }
        line.pop();
        Some(line)
    }
    fn find_closest_biome(
        &self,
        worker: &mut WorkerState,
        search: &BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])> {
        // BiomeSource.findClosestBiome3d.
        let candidates: BTreeSet<&str> = self
            .source
            .possible_biomes()
            .into_iter()
            .filter(|name| (search.allowed)(name))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let ys = out_from_origin(
            search.origin[1],
            search.min_y + 1,
            search.max_y + 1,
            search.vertical,
        );
        let radius = search.radius.div_euclid(search.horizontal);
        for [sx, sz] in spiral_around(radius) {
            let x = search.origin[0].wrapping_add(sx.wrapping_mul(search.horizontal));
            let z = search.origin[2].wrapping_add(sz.wrapping_mul(search.horizontal));
            for &y in &ys {
                let quart = [x >> 2, y >> 2, z >> 2];
                let Ok(name) = resolver.sample(&mut worker.carver_context, quart) else {
                    continue;
                };
                if candidates.contains(name) {
                    return Some((name.to_owned(), [x, y, z]));
                }
            }
        }
        None
    }
    fn surface_sample(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        mut spans: Option<&mut Vec<(i32, i32, BlockStateId)>>,
    ) -> Option<SurfaceSample> {
        let mut top = None;
        let mut water = None;
        let mut below = 0;
        let mut surface_complete = false;
        let mut visit = |y, state| {
            let properties = self.data.states.get(state);
            if let Some(spans) = spans.as_mut()
                && !properties.is_air()
            {
                if let Some((min, _, previous)) = spans.last_mut()
                    && *min == y + 1
                    && *previous == state
                {
                    *min = y;
                } else {
                    spans.push((y, y + 1, state));
                }
            }
            if surface_complete {
                return false;
            }
            if properties.is_air() || properties.has_fluid() {
                if top.is_some() {
                    surface_complete = true;
                    return spans.is_none();
                }
                if properties.has_fluid() {
                    water.get_or_insert((y + 1, state));
                } else {
                    // Match the material traversal's water reset across air gaps.
                    water = None;
                }
            } else {
                top.get_or_insert((y + 1, state));
                below += 1;
            }
            false
        };
        self.noise_column(x, z, &mut visit);
        let (height, state) = top?;
        // A superflat surface is its top layer, with its one biome.
        if self.flat.is_some() {
            let biome = self
                .source
                .possible_biomes()
                .into_iter()
                .find_map(|name| self.data.biome_id(name))?;
            return Some(SurfaceSample {
                height,
                state,
                water,
                biome,
            });
        }
        let y = height - 1;
        let position = BlockPos::new(x, y, z);
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        // Use a fresh bounded context: far samples must not grow the chunk/structure caches.
        let mut biome_context = SamplerContext::default();
        let biome = resolver
            .sample(&mut biome_context, self.zoom.quart_at_block(position))
            .ok()?;
        let biome = self.data.biome_id(biome)?;
        let volume = DensityVolume::new([1; 3], [x, y, z], [1; 3]).ok()?;
        let queries = SurfaceQueries {
            pipeline: self,
            temperatures: &mut worker.temperatures,
            biome: Biomes::Resolver(&mut resolver, &mut biome_context),
            uniform_biomes: None,
            possible: None,
        };
        let sources = MaterialDensitySources::new(
            queries,
            volume,
            self.preliminary.clone(),
            SamplerContext::default(),
        )
        .ok()?;
        let mut material =
            MaterialConditions::<R, _>::new(self.generation(), self.root.clone(), sources);
        let rule = material.add_rule(self.rule.clone()).ok()?;
        // shortcut: distant surfaces omit slope rules and column extensions; nearby chunks provide full terrain.
        material.update_xz(x, z, 0, 0);
        material.update_y(1, below, water.map_or(i32::MIN, |(height, _)| height), y);
        let state = material.try_apply(rule, position).ok()?.unwrap_or(state);
        if let Some(spans) = spans
            && let Some(index) = spans.iter().position(|&(min, max, _)| min <= y && y < max)
            && spans[index].2 != state
        {
            let (min, max, _) = spans[index];
            if min == y {
                spans[index].2 = state;
            } else {
                spans[index].1 = y;
                spans.insert(index, (y, max, state));
            }
        }
        Some(SurfaceSample {
            height,
            state,
            water,
            biome,
        })
    }
    fn lod_grid(
        &self,
        worker: &mut WorkerState,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError> {
        if width <= 0 || side == 0 || side > 64 {
            return Err(resource("far sample dimensions"));
        }
        // A superflat world's columns are its layers: the surface sample, and its spans.
        if self.flat.is_some() {
            let mut columns = Vec::with_capacity(side * side);
            for dz in 0..side {
                for dx in 0..side {
                    let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
                    let mut spans = Vec::new();
                    let sample =
                        SeededPipeline::surface_sample(self, worker, wx, wz, Some(&mut spans));
                    columns.push((sample, spans));
                }
            }
            return Ok(columns);
        }
        let mut geometry = Vec::with_capacity(side * side);
        // The coarse and band passes below share the 2D (x/z-only) parts.
        let mut context = if approximate {
            SamplerContext::sparse().with_planes()
        } else {
            SamplerContext::sparse()
        };
        let (min, height) = (self.settings.noise_min_y, self.settings.noise_height);
        let step_xz = width.max(4);
        let ox = x.div_euclid(4) * 4;
        let oz = z.div_euclid(4) * 4;
        let nx = ((x - ox + (side as i32 - 1) * width + step_xz - 1) / step_xz) + 1;
        let nz = ((z - oz + (side as i32 - 1) * width + step_xz - 1) / step_xz) + 1;
        // Approximate: cells many blocks wide keep their interpolated shape on
        // a coarser vertical lattice, for a fraction of the density work.
        let step_y = match width {
            _ if !approximate => 8,
            ..32 => 8,
            32..128 => 16,
            _ => 32,
        };
        let final_density = self.router.get(RouterField::FinalDensity);
        // Approximate: far tiles show the surface, so find its band on a
        // coarse lattice (its points are the fine lattice's), then sample the
        // fine lattice only inside it. Most of a tall world's column is sky or
        // deep rock. Parts thinner than the coarse step above an air sample
        // (floating islands, overhang lips) can be missed; below the band, a
        // column is one span of the default block.
        let (band_min, band_max) = if approximate {
            const COARSE: i32 = 64;
            let coarse_layers = (height + COARSE - 1) / COARSE + 1;
            let coarse = DensityVolume::new(
                [nx, coarse_layers, nz],
                [ox, min, oz],
                [step_xz, COARSE, step_xz],
            )
            .map_err(stage("far density"))?;
            let mut coarse_densities = vec![0.; coarse.size()];
            final_density
                .sample_volume(&mut context, &mut coarse_densities, coarse)
                .map_err(stage("far density"))?;
            let (mut lowest, mut highest) = (i32::MAX, i32::MIN);
            for iz in 0..nz as usize {
                for ix in 0..nx as usize {
                    let solid = (0..coarse_layers as usize)
                        .rev()
                        .find(|&iy| coarse_densities[coarse.index([ix, iy, iz]).unwrap()] > 0.);
                    if let Some(iy) = solid {
                        let y = min + iy as i32 * COARSE;
                        lowest = lowest.min(y - COARSE);
                        highest = highest.max(y + 2 * COARSE);
                    }
                }
            }
            if lowest > highest {
                (min, min + height)
            } else {
                let lowest = lowest.max(min);
                (
                    min + (lowest - min) / step_y * step_y,
                    highest
                        .max(self.settings.sea_level + COARSE)
                        .min(min + height),
                )
            }
        } else {
            (min, min + height)
        };
        let ny = (band_max - band_min + step_y - 1) / step_y + 1;
        let volume =
            DensityVolume::new([nx, ny, nz], [ox, band_min, oz], [step_xz, step_y, step_xz])
                .map_err(stage("far density"))?;
        let mut densities = vec![0.; volume.size()];
        final_density
            .sample_volume(&mut context, &mut densities, volume)
            .map_err(stage("far density"))?;
        let aquifer = DisabledAquifer::new(
            self.picker,
            self.data.registry.default_state("minecraft:air").unwrap(),
        );
        // Interpolate the supplied lattice, never the physical bounding volume.
        let value = |wx: i32, iy: usize, wz: i32| {
            let rx = (wx - ox) as f32 / step_xz as f32;
            let rz = (wz - oz) as f32 / step_xz as f32;
            let ix = rx.floor() as usize;
            let iz = rz.floor() as usize;
            let at = |dx: usize, dz: usize| {
                densities[volume
                    .index([
                        (ix + dx).min(nx as usize - 1),
                        iy,
                        (iz + dz).min(nz as usize - 1),
                    ])
                    .unwrap()]
            };
            let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
            mix(
                mix(at(0, 0), at(1, 0), rx - ix as f32),
                mix(at(0, 1), at(1, 1), rx - ix as f32),
                rz - iz as f32,
            )
        };
        let mut profile = vec![0.; ny as usize];
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        let mut biome_context = SamplerContext::cached();
        let mut biome_memo = BiomeMemo {
            biomes: HashMap::new(),
            // Columns `width` blocks apart are width / 4 quarts apart; cells
            // stop at 64 blocks (wider ones moved a third of level 6's biomes).
            cell: if approximate && width >= 4 {
                width.min(16)
            } else {
                0
            },
        };
        for dz in 0..side {
            for dx in 0..side {
                let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
                let mut top = None;
                let mut water = None;
                let mut below = 0;
                let mut complete = false;
                let mut spans: Vec<(i32, i32, BlockStateId)> = Vec::new();
                for (iy, density) in profile.iter_mut().enumerate() {
                    *density = value(wx, iy, wz);
                }
                for y in (band_min..band_max).rev() {
                    let iy = ((y - band_min) / step_y) as usize;
                    let alpha = ((y - band_min) % step_y) as f32 / step_y as f32;
                    let density = profile[iy] + (profile[iy + 1] - profile[iy]) * alpha;
                    let state = aquifer
                        .compute_substance([wx, y, wz], f64::from(density))
                        .unwrap_or(self.settings.default_block);
                    let properties = self.data.states.get(state);
                    if !properties.is_air() {
                        if let Some((min, _, old)) = spans.last_mut()
                            && *min == y + 1
                            && *old == state
                        {
                            *min = y;
                        } else {
                            spans.push((y, y + 1, state));
                        }
                    }
                    if !complete {
                        if properties.is_air() || properties.has_fluid() {
                            if top.is_some() {
                                complete = true;
                            } else if properties.has_fluid() {
                                water.get_or_insert((y + 1, state));
                            } else {
                                water = None;
                            }
                        } else {
                            top.get_or_insert((y + 1, state));
                            below += 1;
                        }
                    }
                }
                if band_min > min {
                    let fill = self.settings.default_block;
                    if top.is_some() && !complete {
                        below += band_min - min;
                    }
                    match spans.last_mut() {
                        Some((low, _, state)) if *low == band_min && *state == fill => *low = min,
                        _ => spans.push((min, band_min, fill)),
                    }
                }
                // Void cells still need a biome: the End's small islands are
                // features over the void, and their biome is 2D there.
                let biome = if let Some((height, _)) = top {
                    memo_biome(
                        &self.data,
                        &mut resolver,
                        &mut biome_context,
                        &mut biome_memo,
                        self.zoom.quart_at_block(BlockPos::new(wx, height - 1, wz)),
                    )
                    .map_err(stage("far biome"))?
                } else {
                    BiomeId(0)
                };
                geometry.push((top, water, below, spans, biome));
            }
        }
        if geometry.iter().all(|column| column.0.is_none()) {
            return Ok(geometry
                .into_iter()
                .map(|(_, _, _, spans, _)| (None, spans))
                .collect());
        }
        let volume =
            DensityVolume::new([1; 3], [x, 0, z], [1; 3]).map_err(stage("far material"))?;
        let queries = SurfaceQueries {
            pipeline: self,
            temperatures: &mut worker.temperatures,
            biome: Biomes::Memo(&mut resolver, &mut biome_context, &mut biome_memo),
            uniform_biomes: None,
            possible: None,
        };
        let sources = MaterialDensitySources::new(
            queries,
            volume,
            self.preliminary.clone(),
            SamplerContext::cached(),
        )
        .map_err(stage("far material"))?;
        let mut material =
            MaterialConditions::<R, _>::new(self.generation(), self.root.clone(), sources);
        let rule = material
            .add_rule(self.rule.clone())
            .map_err(stage("far material"))?;
        let mut result = Vec::with_capacity(side * side);
        for (index, (top, water, below, mut spans, biome)) in geometry.into_iter().enumerate() {
            let (wx, wz) = (
                x + (index % side) as i32 * width,
                z + (index / side) as i32 * width,
            );
            let Some((height, mut state)) = top else {
                result.push((None, spans));
                continue;
            };
            material.update_xz(wx, wz, 0, 0);
            for depth in 1..=below.min(8) {
                let y = height - depth;
                material.update_y(
                    depth,
                    below - depth + 1,
                    water.map_or(i32::MIN, |(height, _)| height),
                    y,
                );
                let Some(index) = spans.iter().position(|&(min, max, _)| min <= y && y < max)
                else {
                    continue;
                };
                let (min, max, old) = spans[index];
                let painted = material
                    .try_apply(rule, BlockPos::new(wx, y, wz))
                    .map_err(stage("far material"))?
                    .unwrap_or(old);
                if depth == 1 {
                    state = painted;
                }
                if old == painted {
                    continue;
                }
                let mut replacement = Vec::with_capacity(3);
                if y + 1 < max {
                    replacement.push((y + 1, max, old));
                }
                replacement.push((y, y + 1, painted));
                if min < y {
                    replacement.push((min, y, old));
                }
                spans.splice(index..=index, replacement);
            }
            result.push((
                Some(SurfaceSample {
                    height,
                    state,
                    water,
                    biome,
                }),
                spans,
            ));
        }
        Ok(result)
    }
    fn noise_biomes(&self, worker: &mut WorkerState, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        let mut resolver = self.source.resolver(
            &self.climate,
            self.router.get(RouterField::Erosion).as_ref(),
        );
        quarts
            .iter()
            .map(|&quart| {
                resolver
                    .sample(&mut worker.carver_context, quart)
                    .ok()
                    .map(str::to_owned)
            })
            .collect()
    }
    fn jigsaw_pieces(
        &self,
        worker: &mut WorkerState,
        query: &JigsawQuery<'_>,
    ) -> Option<JigsawPieces> {
        // JigsawPlacement.generateJigsaw: addPieces with no heightmap
        // projection, MaxDistance(128), no aliases, no padding and
        // waterlogging applied. The pieces draw from the GenerationContext's
        // random, seeded from the level seed and the position's chunk, not
        // from the level random.
        let structures = self.structures.as_ref()?;
        let mut random = horizon_core::random::WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(self.seed, query.position[0] >> 4, query.position[2] >> 4);
        let aliases = BTreeMap::new();
        let config = super::structure::jigsaw::JigsawConfig {
            start_pool: query.pool,
            start_jigsaw: Some(query.target),
            max_depth: query.max_depth,
            use_expansion_hack: false,
            project_to: None,
            max_distance: [128, 128],
            aliases: &aliases,
            padding: [0, 0],
            waterlogging: true,
        };
        let (_, pieces) = super::structure::jigsaw::assemble(
            &config,
            &mut StructureQueries {
                pipeline: self,
                worker,
                valid: BTreeSet::new(),
            },
            &structures.registry.pools,
            &structures.resources,
            &structures.bindings,
            &mut random,
            query.position,
        )?;
        Some(JigsawPieces {
            pieces,
            resources: structures.resources.clone(),
        })
    }
}

/// A `JigsawPlacement.generateJigsaw` request (`/place jigsaw`).
pub struct JigsawQuery<'a> {
    pub pool: &'a str,
    pub target: &'a str,
    pub max_depth: i32,
    pub position: [i32; 3],
}
/// The assembled pieces of a jigsaw and the templates they place.
pub struct JigsawPieces {
    pub pieces: Vec<super::structure::jigsaw::PoolPiece>,
    pub resources: Arc<super::structure::StructureResources>,
}

/// A `findClosestBiome3d` query.
pub struct BiomeSearch<'a> {
    pub allowed: &'a dyn Fn(&str) -> bool,
    pub origin: [i32; 3],
    pub radius: i32,
    pub horizontal: i32,
    pub vertical: i32,
    /// `level.getMinY()` and `level.getMaxY()`.
    pub min_y: i32,
    pub max_y: i32,
}

/// `Mth.outFromOrigin(origin, lower, upper, step)`.
pub fn out_from_origin(origin: i32, lower: i32, upper: i32, step: i32) -> Vec<i32> {
    let center = origin.clamp(lower, upper);
    let mut out = Vec::new();
    let mut cursor = center;
    loop {
        let distance = (center - cursor).abs();
        if !(center - distance >= lower || center + distance <= upper) {
            return out;
        }
        out.push(cursor);
        let negative = cursor <= center;
        let can_move_positive = center + distance + step <= upper;
        let mut next = center + distance + step;
        if !negative || !can_move_positive {
            let attempt = center - distance - if negative { step } else { 0 };
            if attempt >= lower {
                next = attempt;
            }
        }
        cursor = next;
    }
}

/// `BlockPos.spiralAround(ZERO, radius, EAST, SOUTH)` as (x, z) offsets.
pub fn spiral_around(radius: i32) -> impl Iterator<Item = [i32; 2]> {
    const DIRS: [[i32; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
    let legs = 4 * radius;
    let (mut leg, mut leg_size, mut leg_index) = (-1i32, 0i32, 0i32);
    let mut last = [0, 1];
    std::iter::from_fn(move || {
        let d = DIRS[((leg + 4) % 4) as usize];
        let cursor = [last[0] + d[0], last[1] + d[1]];
        last = cursor;
        if leg_index >= leg_size {
            if leg >= legs {
                return None;
            }
            leg += 1;
            leg_index = 0;
            leg_size = leg / 2 + 1;
        }
        leg_index += 1;
        Some(cursor)
    })
}

/// Counters from one FEATURES pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorateStats {
    /// Placed-feature roots executed (including those that placed nothing).
    pub placed: usize,
    /// Roots skipped because their algorithm or bindings are not implemented.
    pub unsupported: usize,
    /// Roots that started but failed on an unavailable world query or error.
    /// Their partial writes are kept, as each root reseeds its own random.
    pub failed: usize,
    /// Identifiers of skipped or failed roots, for diagnostics.
    pub skipped: BTreeSet<String>,
}

/// A seed-bound generator for one dimension, shareable across worker threads.
#[derive(Clone)]
pub struct OverworldGenerator {
    pipeline: Arc<dyn SeededPipeline>,
    /// The noise range the settings asked for and the build height's part
    /// of it that generates, when they differ.
    noise_clamped: Option<(NoiseSettings, NoiseSettings)>,
}
impl OverworldGenerator {
    /// The chunks an older version generated, which new chunks blend with
    /// (`Blender.of`); set once, before chunks are generated.
    pub fn set_old_world(&self, world: Arc<dyn super::blending::OldWorld>) {
        self.pipeline.set_old_world(world);
    }
    /// The `minecraft:overworld` dimension of the `minecraft:normal` world preset.
    pub fn new(data: Arc<GeneratorData>, seed: i64) -> Result<Self, GeneratorError> {
        Self::for_dimension(data, seed, "minecraft:normal", "minecraft:overworld")
    }
    /// The overworld without structures (no starts, terrain adaptation or
    /// structure pieces), as in a world with structure generation disabled.
    pub fn without_structures(data: Arc<GeneratorData>, seed: i64) -> Result<Self, GeneratorError> {
        Self::build(data, seed, "minecraft:normal", "minecraft:overworld", false)
    }
    /// Any noise-generator dimension of a bundled world preset.
    pub fn for_dimension(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
    ) -> Result<Self, GeneratorError> {
        Self::build(data, seed, preset, dimension, true)
    }
    /// A superflat overworld (`FlatLevelSource`): the layers where the terrain would be, a
    /// fixed biome, and the structure sets `structure_overrides` allows (every one when it
    /// names none) generating on the flat ground, with or without structures.
    pub fn flat(
        data: Arc<GeneratorData>,
        seed: i64,
        flat: &FlatSettings,
        structures: bool,
    ) -> Result<Self, GeneratorError> {
        let registry = data.registry;
        let (level_min_y, level_height) = overrides::LevelHeight::of_type(
            &Value::from("minecraft:overworld"),
            &data.dimension_types,
        )
        .map_or((-64, 384), |h| (h.min_y, h.height));
        // The overworld's own noise settings give the level its sea level and the rest of
        // the generator its data; no noise is ever sampled.
        let settings_json: Value = serde_json::from_str(
            data.settings
                .get("minecraft:overworld")
                .ok_or_else(|| resource("missing noise settings minecraft:overworld"))?,
        )?;
        // `Biome.CODEC.lenientOptionalFieldOf("biome")`: an unknown biome is Plains.
        let biome = if data.biome_id(&flat.biome).is_some() {
            flat.biome.as_str()
        } else {
            "minecraft:plains"
        };
        let generator = serde_json::json!({
            "type": "minecraft:noise",
            "settings": "minecraft:overworld",
            "biome_source": { "type": "minecraft:fixed", "biome": biome },
            "structure_overrides": flat.structure_overrides,
        });
        let mut pipeline = Pipeline::new(
            data.clone(),
            seed,
            XoroshiroRandom::new(seed),
            &generator,
            &settings_json,
            structures,
            Some((level_min_y, level_height)),
        )?;
        let mut states = Vec::new();
        let mut null = Vec::new();
        for (block, height) in &flat.layers {
            let id = registry
                .default_state(block)
                .ok_or_else(|| resource(format!("missing block {block}")))?;
            // `adjustGenerationSettings`: a layer that is not opaque to MOTION_BLOCKING is
            // `null` in the layers `FlatLevelSource` reads.
            let hole = !HeightmapType::MotionBlocking.is_opaque(data.states.get(id));
            for _ in 0..(*height).max(0) {
                states.push(id);
                null.push(hole);
            }
        }
        states.truncate(level_height.max(0) as usize);
        null.truncate(states.len());
        // `FlatLevelSource.getSeaLevel`; its minimum Y and depth are in `generation`.
        pipeline.settings.sea_level = -63;
        let top_layers: Vec<(i32, BlockStateId)> = null
            .iter()
            .zip(&states)
            .enumerate()
            .filter(|(_, (hole, _))| **hole)
            .map(|(i, (_, &state))| (level_min_y + i as i32, state))
            .collect();
        pipeline.flat = Some(FlatLayers {
            min_y: level_min_y,
            states,
            null,
        });
        // No placed features here (the layers are the whole terrain), but the structure
        // steps run: the plan keeps the steps and leaves their feature lists empty.
        pipeline.decoration =
            super::generator_decoration::Decoration::new_with(&data, &pipeline.source, false)?.map(
                |mut decoration| {
                    decoration.top_layers = top_layers;
                    Arc::new(decoration)
                },
            );
        if let (Some(state), Some(only)) = (pipeline.structures.as_mut(), &flat.structure_overrides)
        {
            state.restrict_to(only);
        }
        // `ChunkGeneratorStructureState.createForFlat`: the stronghold rings use seed 0.
        if let Some(state) = pipeline.structures.as_mut() {
            state.ring_seed = Some(0);
        }
        Ok(Self {
            pipeline: Arc::new(pipeline),
            noise_clamped: None,
        })
    }
    /// A dimension of a world preset, with or without structures (the
    /// world's `generate_structures`: without, no starts, terrain adaptation,
    /// structure pieces or structure locating).
    pub fn for_world(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> Result<Self, GeneratorError> {
        Self::build(data, seed, preset, dimension, structures)
    }
    fn build(
        data: Arc<GeneratorData>,
        seed: i64,
        preset: &str,
        dimension: &str,
        with_structures: bool,
    ) -> Result<Self, GeneratorError> {
        let preset_json: Value = serde_json::from_str(
            data.world_presets
                .get(preset)
                .ok_or_else(|| resource(format!("missing world preset {preset}")))?,
        )?;
        // A data pack's level stem replaces the preset's, as the world's
        // dimension registry does.
        let stem = match data.dimensions.get(dimension) {
            Some(json) => serde_json::from_str(json)?,
            None => preset_json["dimensions"][dimension].clone(),
        };
        let generator = stem["generator"].clone();
        // The level's build range: its dimension type's, a data pack's
        // over the vanilla one.
        let level = overrides::LevelHeight::of_type(&stem["type"], &data.dimension_types)
            .map(|h| (h.min_y, h.height));
        if generator["type"].as_str() != Some("minecraft:noise") {
            return Err(resource(format!("{dimension} is not a noise generator")));
        }
        let mut settings_json: Value = match &generator["settings"] {
            Value::String(name) => serde_json::from_str(
                data.settings
                    .get(name)
                    .ok_or_else(|| resource(format!("missing noise settings {name}")))?,
            )?,
            inline => inline.clone(),
        };
        // NoiseSettings.clampToHeightAccessor: noise past the build height
        // (a data pack's taller terrain over a dimension type it left as it
        // was) fills only the part inside it.
        let mut noise_clamped = None;
        if let Some((level_min_y, level_height)) = level
            && let (Some(min_y), Some(height)) = (
                settings_json["noise"]["min_y"].as_i64(),
                settings_json["noise"]["height"].as_i64(),
            )
        {
            let noise = NoiseSettings::new(min_y as i32, height as i32);
            let clamped = noise.clamp_to_height(level_min_y, level_height);
            if clamped != noise {
                settings_json["noise"]["min_y"] = clamped.min_y.into();
                settings_json["noise"]["height"] = clamped.height.into();
                noise_clamped = Some((noise, clamped));
            }
        }
        let pipeline: Arc<dyn SeededPipeline> = if settings_json["legacy_random_source"]
            .as_bool()
            .unwrap_or(false)
        {
            Arc::new(Pipeline::new(
                data,
                seed,
                LegacyRandom::new(seed),
                &generator,
                &settings_json,
                with_structures,
                level,
            )?)
        } else {
            Arc::new(Pipeline::new(
                data,
                seed,
                XoroshiroRandom::new(seed),
                &generator,
                &settings_json,
                with_structures,
                level,
            )?)
        };
        Ok(Self {
            pipeline,
            noise_clamped,
        })
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        self.pipeline.data()
    }
    /// The noise range the noise settings asked for and the part of it
    /// inside the dimension's build height, when the settings reach past it.
    pub fn noise_clamped(&self) -> Option<(NoiseSettings, NoiseSettings)> {
        self.noise_clamped
    }
    /// The exact `end_island_decorated` origins in a chunk, without terrain.
    pub fn end_island_origins(&self, chunk: ChunkPos) -> Vec<[i32; 3]> {
        self.pipeline.end_island_origins(chunk)
    }
    pub fn sea_level(&self) -> i32 {
        self.pipeline.sea_level()
    }
    /// The dimension's build range (minimum Y, height): what its chunks span.
    pub fn level(&self) -> (i32, i32) {
        self.pipeline.level()
    }
    /// Placed features of this dimension's biomes that decoration skips because
    /// their algorithm or bindings are not implemented, with the decode error.
    pub fn unsupported_features(&self) -> Vec<(String, String)> {
        self.pipeline.unsupported_features()
    }
    /// NoiseBasedChunkGenerator.getOrigin: the chunk a new world's spawn
    /// search starts from (NoiseSpawnFinder over the `spawn_target` points).
    pub fn origin(&self) -> Result<ChunkPos, GeneratorError> {
        let [x, z] = self.spawn_position()?;
        Ok(ChunkPos::new(x >> 4, z >> 4))
    }
    /// NoiseSpawnFinder.findSpawnPosition: the column `origin` is the chunk of.
    pub fn spawn_position(&self) -> Result<[i32; 2], GeneratorError> {
        self.pipeline.spawn_position()
    }
    /// A new `WorldGenRegion` random centred on the chunk: what its generation
    /// steps draw from (`level.getRandom()`).
    pub fn region_random(
        &self,
        center: ChunkPos,
    ) -> Box<dyn super::structure::legacy::PieceRandom> {
        self.pipeline.region_random(center)
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the `Density` line of
    /// the noise settings' `debug_functions` at the feet (none without them).
    /// It reads the pipeline's samplers only, so it needs no worker.
    pub fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        self.pipeline.debug_density_line(feet)
    }
    /// Create per-thread state. Workers are cheap; keep one per thread.
    pub fn worker(&self) -> GeneratorWorker {
        let data = self.pipeline.data();
        GeneratorWorker {
            pipeline: self.pipeline.clone(),
            state: WorkerState {
                temperatures: data
                    .temperatures
                    .iter()
                    .map(|s| BiomeTemperature::new(*s))
                    .collect(),
                terrain_context: SamplerContext::cached(),
                biome_context: SamplerContext::cached(),
                carver_context: SamplerContext::default(),
                carver_biomes: HashMap::new(),
                structure_biomes: HashMap::new(),
                timings: StageTimings::default(),
                structures_only: false,
                structure_mobs: None,
                heights: HashMap::new(),
            },
        }
    }
}

/// A surface-only noise column for far terrain. Heights are exclusive top faces.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSample {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: BiomeId,
}

pub type LodColumn = (Option<SurfaceSample>, Vec<(i32, i32, BlockStateId)>);

/// Per-thread generation state.
pub struct GeneratorWorker {
    pipeline: Arc<dyn SeededPipeline>,
    state: WorkerState,
}
impl GeneratorWorker {
    /// Biomes, noise fill and aquifers, material surface and carvers: the
    /// vanilla statuses up to TERRAIN. Independent of neighbor chunks' state.
    pub fn generate_proto(&mut self, pos: ChunkPos) -> Result<ProtoChunk, GeneratorError> {
        let result = self.pipeline.generate(&mut self.state, pos);
        self.state.timings.chunks += 1;
        result
    }
    /// Noise/aquifer surface and its material rule, without chunks, structures or features.
    pub fn surface_sample(&mut self, x: i32, z: i32) -> Option<SurfaceSample> {
        self.pipeline.surface_sample(&mut self.state, x, z, None)
    }
    /// Noise/aquifer material runs, excluding air; bounds are half-open.
    /// Features and carvers still require generated chunks.
    pub fn surface_column(
        &mut self,
        x: i32,
        z: i32,
    ) -> (Option<SurfaceSample>, Vec<(i32, i32, BlockStateId)>) {
        let mut spans = Vec::new();
        let surface = self
            .pipeline
            .surface_sample(&mut self.state, x, z, Some(&mut spans));
        (surface, spans)
    }
    /// `/locate structure`: the nearest start of one of `names`.
    pub fn locate(
        &mut self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.locate_skipping(names, from, radius, &|_, _| false)
    }
    /// `ChunkGenerator.findNearestMapStructure`: like `locate`, past the
    /// starts `skip` says can't be referenced again (an explorer map that
    /// skips known structures).
    pub fn locate_skipping(
        &mut self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])> {
        self.pipeline
            .locate(&mut self.state, names, from, radius, skip)
    }
    /// Direct far sampling: work scales with rendered vertical resolution,
    /// without allocating chunks, carving masks or propagated aquifers.
    /// `approximate` (Reduced far detail) trades the exact tile for speed in
    /// wide cells: one biome per 4x4 columns, a coarser vertical lattice, and
    /// density only around the surface.
    pub fn lod_grid(
        &mut self,
        x: i32,
        z: i32,
        width: i32,
        side: usize,
        approximate: bool,
    ) -> Result<Vec<LodColumn>, GeneratorError> {
        self.pipeline
            .lod_grid(&mut self.state, x, z, width, side, approximate)
    }
    pub fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        self.pipeline.noise_biomes(&mut self.state, quarts)
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: the `Density` line of
    /// the noise settings' `debug_functions` at the feet (none without them).
    pub fn debug_density_line(&self, feet: [i32; 3]) -> Option<String> {
        self.pipeline.debug_density_line(feet)
    }
    /// `BiomeSource.findClosestBiome3d`: the first sampled position whose
    /// noise biome passes `allowed`, spiralling out from the origin.
    pub fn find_closest_biome(&mut self, search: &BiomeSearch<'_>) -> Option<(String, [i32; 3])> {
        self.pipeline.find_closest_biome(&mut self.state, search)
    }
    /// `JigsawPlacement.addPieces` for `/place jigsaw`, drawing from the
    /// generation context's random (level seed and the position's chunk).
    pub fn jigsaw_pieces(&mut self, query: &JigsawQuery<'_>) -> Option<JigsawPieces> {
        self.pipeline.jigsaw_pieces(&mut self.state, query)
    }
    /// ConcentricRingsStructurePlacement chunks of a structure set (such as
    /// `minecraft:strongholds`), searched for now if nothing needed them yet.
    pub fn ring_positions(&mut self, set: &str) -> Option<Vec<ChunkPos>> {
        self.pipeline.ring_positions(&mut self.state, set)
    }
    /// STRUCTURE_STARTS: the structure starts owned by a chunk (cached).
    pub fn structure_starts(
        &mut self,
        pos: ChunkPos,
    ) -> Vec<Arc<super::structure::start::StructureStart>> {
        self.pipeline.structure_starts(&mut self.state, pos)
    }
    /// `Structure.generate` for `/place structure`: the start `structure`
    /// builds in `chunk`, whatever its placement set says about that chunk.
    pub fn structure_start(
        &mut self,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<Arc<super::structure::start::StructureStart>> {
        self.pipeline
            .structure_start(&mut self.state, structure, chunk)
    }
    /// Puts a saved start of `structure` (its pieces as a save lists them, in
    /// JSON) in `chunk`, in place of the one generation builds there, until
    /// the starts are forgotten. Harnesses place another game's layout.
    pub fn use_saved_start(
        &mut self,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[serde_json::Value],
    ) -> Result<(), String> {
        (self.pipeline).use_saved_start(&mut self.state, chunk, structure, pieces)
    }
    /// Forget the cached structure starts: chunks generated afterwards get
    /// fresh starts, as in a new level that has not loaded the old chunks.
    pub fn forget_structure_starts(&mut self) {
        self.pipeline.forget_structure_starts();
    }
    pub fn stage_timings(&self) -> StageTimings {
        self.state.timings
    }
    /// FEATURES status for `region[4]`. The region is a 3×3 neighborhood in
    /// row-major order: index `(dz + 1) * 3 + (dx + 1)` around the center.
    /// Writes may reach the neighbors, as in vanilla. Unsupported features are
    /// skipped and counted. On error, chunks keep partial writes.
    pub fn decorate(
        &mut self,
        region: &mut [ProtoChunk; 9],
    ) -> Result<DecorateStats, GeneratorError> {
        self.pipeline.decorate(&mut self.state, region)
    }
    /// Skip placed features when decorating and keep structure pieces only
    /// (for structure placement parity checks).
    pub fn set_structures_only(&mut self, only: bool) {
        self.state.structures_only = only;
    }
    pub fn set_structure_mobs(&mut self, mobs: Arc<dyn StructureMobs>) {
        self.state.structure_mobs = Some(mobs);
    }
    /// Give back the density scratch buffers and the lookup caches, which only
    /// speed up the next chunks (a generating worker holds megabytes of them).
    /// Results do not change: they are made again on demand.
    pub fn release(&mut self) {
        let state = &mut self.state;
        for context in [
            &mut state.terrain_context,
            &mut state.biome_context,
            &mut state.carver_context,
        ] {
            context.clear_scratch();
        }
        state.carver_biomes = HashMap::new();
        state.structure_biomes = HashMap::new();
        state.heights = HashMap::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pack_tags_join_the_prepared_ones_and_broken_ones_are_dropped() {
        let raw = Bundle::from([(
            "minecraft:hill".to_owned(),
            r#"{"values":["minecraft:a"]}"#.to_owned(),
        )]);
        let known = |id: &str| id != "minecraft:gone";
        let mut ov = WorldgenOverrides::default();
        ov.add_tag(
            "biome_tags",
            "minecraft:hill",
            "file/p",
            r#"{"values":["minecraft:b"]}"#.into(),
        );
        ov.add_tag(
            "biome_tags",
            "p:broken",
            "file/p",
            r#"{"values":["minecraft:gone"]}"#.into(),
        );
        let mut dropped = Vec::new();
        let tags = build_tags(&raw, &ov, "biome_tags", &mut dropped, known).unwrap();
        assert_eq!(tags["minecraft:hill"], ["minecraft:a", "minecraft:b"]);
        assert!(!tags.contains_key("p:broken"));
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert!(dropped[0].contains("p:broken"));
        // Without packs the prepared tags must all resolve.
        let broken = Bundle::from([(
            "p:broken".to_owned(),
            r#"{"values":["minecraft:gone"]}"#.to_owned(),
        )]);
        let none = WorldgenOverrides::default();
        assert!(build_tags(&broken, &none, "biome_tags", &mut dropped, known).is_err());
    }
    #[test]
    fn a_pack_level_stem_names_taller_noise_which_clamps_to_the_build_height() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let dir = root.join("worldgen");
        let vanilla = GeneratorData::load(&dir, registry).unwrap();
        let mut settings: Value =
            serde_json::from_str(&vanilla.settings["minecraft:overworld"]).unwrap();
        settings["noise"]["height"] = 2096.into();
        let preset: Value =
            serde_json::from_str(&vanilla.world_presets["minecraft:normal"]).unwrap();
        let mut stem = preset["dimensions"]["minecraft:overworld"].clone();
        stem["generator"]["settings"] = "horizon:tall".into();
        let mut ov = WorldgenOverrides::default();
        ov.set("settings", "horizon:tall", settings.to_string());
        ov.set(
            overrides::DIMENSIONS,
            "minecraft:overworld",
            stem.to_string(),
        );
        let packed = GeneratorData::load_with(&dir, registry, &ov).unwrap();
        assert!(packed.warnings().is_empty(), "{:?}", packed.warnings());
        let (asked, kept) = OverworldGenerator::new(packed, 0)
            .unwrap()
            .noise_clamped()
            .expect("the pack's noise reaches past the build height");
        assert_eq!((asked.min_y, asked.height), (-64, 2096));
        assert_eq!((kept.min_y, kept.height), (-64, 384));
        let plain = OverworldGenerator::new(vanilla, 0).unwrap();
        assert!(plain.noise_clamped().is_none());
    }
    #[test]
    fn end_identity_surface_matches_normal_surface_and_rejects_custom_rules() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
        let settings: Value = serde_json::from_str(&data.settings["minecraft:end"]).unwrap();
        let mut pipeline = Pipeline::new(
            data.clone(),
            12345,
            LegacyRandom::new(12345),
            &preset["dimensions"]["minecraft:the_end"]["generator"],
            &settings,
            false,
            None,
        )
        .unwrap();
        let mut worker = OverworldGenerator::for_world(
            data.clone(),
            12345,
            "minecraft:normal",
            "minecraft:the_end",
            false,
        )
        .unwrap()
        .worker();
        assert!(pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        for pos in [
            ChunkPos::new(0, 0),
            ChunkPos::new(128, 128),
            ChunkPos::new(-128, 192),
            ChunkPos::new(1_875_000, -1_875_000),
        ] {
            let actual = pipeline.generate(&mut worker.state, pos).unwrap();
            let rule = pipeline.rule.clone();
            // A singleton sequence evaluates the same block rule through the full surface path.
            pipeline.rule = MaterialRule::Sequence(vec![rule.clone()]);
            assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
            let expected = pipeline.generate(&mut worker.state, pos).unwrap();
            pipeline.rule = rule;
            assert_eq!(
                actual.terrain.generation_stage(),
                expected.terrain.generation_stage()
            );
            for section in 0..actual.terrain.height() as usize / 16 {
                assert_eq!(
                    actual.terrain.section(section).unwrap().to_nbt().unwrap(),
                    expected.terrain.section(section).unwrap().to_nbt().unwrap()
                );
            }
            for kind in HeightmapType::ALL {
                assert_eq!(
                    actual.terrain.heightmap(kind).map(|m| m.raw_data()),
                    expected.terrain.heightmap(kind).map(|m| m.raw_data())
                );
            }
            assert_eq!(
                actual.terrain.postprocessing(),
                expected.terrain.postprocessing()
            );
            assert_eq!(
                actual.terrain.block_ticks().pack(0),
                expected.terrain.block_ticks().pack(0)
            );
            assert_eq!(
                actual.terrain.fluid_ticks().pack(0),
                expected.terrain.fluid_ticks().pack(0)
            );
            assert_eq!(actual.block_entities, expected.block_entities);
            assert_eq!(actual.entities, expected.entities);
            assert_eq!(actual.structures(), expected.structures());
        }
        assert!(!pipeline.identity_end_surface(ChunkPos::new(i32::MAX, 0)));
        let old_rule = pipeline.rule.clone();
        pipeline.rule = MaterialRule::Block(registry.default_state("minecraft:stone").unwrap());
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.rule = old_rule;
        let old_fluid = pipeline.settings.default_fluid;
        pipeline.settings.default_fluid = registry.default_state("minecraft:stone").unwrap();
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.settings.default_fluid = old_fluid;
        let old_source = std::mem::replace(
            &mut pipeline.source,
            ResolvedBiomeSource::Fixed(super::super::biome_source::FixedBiomeSource {
                biome: "minecraft:the_end".into(),
            }),
        );
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
        pipeline.source = old_source;
        pipeline.aquifer = Some(std::array::from_fn(|_| {
            pipeline.router.get(RouterField::Erosion).clone()
        }));
        assert!(!pipeline.identity_end_surface(ChunkPos::new(0, 0)));
    }

    #[test]
    fn uniform_surface_biomes_preserve_zoom_and_missing_corner_errors() {
        use super::super::biome_chunk::BiomeChunk;
        use std::convert::Infallible;

        for pos in [
            ChunkPos::new(0, 0),
            ChunkPos::new(-17, 23),
            ChunkPos::new(i32::MIN >> 4, i32::MAX >> 4),
        ] {
            for seed in [0, -1, i64::MIN] {
                for mixed in [false, true] {
                    let mut chunks = Vec::new();
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            let mut chunk = BiomeChunk::new(
                                ChunkPos::new(pos.x + dx, pos.z + dz),
                                -64,
                                128,
                                BiomeId(7),
                            )
                            .unwrap();
                            chunk
                                .fill(-64, 63, |[x, y, z]| {
                                    Ok::<_, Infallible>(if mixed {
                                        BiomeId(
                                            (x.div_euclid(3) + y.div_euclid(4) + z.div_euclid(2))
                                                .rem_euclid(5)
                                                as u32,
                                        )
                                    } else {
                                        BiomeId(7)
                                    })
                                })
                                .unwrap();
                            chunks.push(chunk);
                        }
                    }
                    let region = BiomeRegion::new(chunks, seed).unwrap();
                    let mut cache = UniformSurfaceBiomes::new(pos, -64, 128);
                    let mut hits = 0;
                    for x in 0..16 {
                        for z in 0..16 {
                            // Include clamped Y, out-of-cache Y, and revisit each
                            // cached proof in the ordinary descending traversal.
                            for y in (-70..70).rev() {
                                let p = BlockPos::new(pos.x * 16 + x, y, pos.z * 16 + z);
                                let shortcut = cache.get(&region, p);
                                hits += usize::from(shortcut.is_some());
                                let actual = shortcut.map_or_else(|| region.biome(p).copied(), Ok);
                                assert_eq!(actual, region.biome(p).copied());
                            }
                        }
                    }
                    if !mixed && pos.x != i32::MIN >> 4 {
                        assert!(hits > 0);
                    }
                }
            }
        }

        // Some unselected corners are missing. Probing them must neither invent
        // an error nor hide the original selected-corner error.
        let pos = ChunkPos::new(0, 0);
        let region =
            BiomeRegion::new(vec![BiomeChunk::new(pos, 0, 16, BiomeId(7)).unwrap()], -1).unwrap();
        let mut cache = UniformSurfaceBiomes::new(pos, 0, 16);
        let mut successes = 0;
        let mut failures = 0;
        for x in -2..18 {
            for z in -2..18 {
                let p = BlockPos::new(x, 7, z);
                let actual = cache
                    .get(&region, p)
                    .map_or_else(|| region.biome(p).copied(), Ok);
                assert_eq!(actual, region.biome(p).copied());
                successes += usize::from(actual.is_ok());
                failures += usize::from(actual.is_err());
            }
        }
        assert!(successes > 0 && failures > 0);
    }

    #[test]
    fn far_surface_matches_undecorated_land_and_keeps_end_void() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let generator =
            OverworldGenerator::without_structures(data.clone(), 1234).expect("generator");
        let mut worker = generator.worker();
        let sample = worker.surface_sample(0, 0).expect("land");
        let (column_surface, spans) = worker.surface_column(0, 0);
        let column_surface = column_surface.expect("column land");
        assert_eq!(column_surface.height, sample.height);
        assert_eq!(column_surface.state, sample.state);
        assert_eq!(column_surface.water, sample.water);
        assert_eq!(column_surface.biome, sample.biome);
        assert!(spans.iter().all(|&(min, max, state)| {
            min < max && !generator.data().block_states().get(state).is_air()
        }));
        assert!(spans.windows(2).all(|pair| {
            pair[1].1 <= pair[0].0 && (pair[1].1 != pair[0].0 || pair[1].2 != pair[0].2)
        }));
        assert!(spans.iter().any(|&(min, max, state)| {
            min < sample.height && sample.height - 1 < max && state == sample.state
        }));
        assert!(sample.water.is_some_and(|(height, state)| {
            spans
                .iter()
                .any(|&(min, max, block)| min < height && height - 1 < max && block == state)
        }));
        let chunk = worker.generate_proto(ChunkPos::new(0, 0)).expect("terrain");
        assert_eq!(
            sample.water.map_or(sample.height, |(height, _)| height),
            chunk.surface_height(0, 0)
        );
        assert_eq!(sample.state, chunk.block(0, sample.height - 1, 0));
        assert!(
            sample.water.is_some(),
            "fixture includes underwater terrain"
        );
        let started = std::time::Instant::now();
        let mut exact = Vec::new();
        for z in 0..8 {
            for x in 0..8 {
                exact.push(worker.surface_column(-1024 + x * 32, z * 32));
            }
        }
        let dense_time = started.elapsed();
        let started = std::time::Instant::now();
        let direct = worker.lod_grid(-1024, 0, 32, 8, false).unwrap();
        for z in 0..8 {
            for x in 0..8 {
                let wx = -1024 + x * 32;
                let wz = z * 32;
                let (surface, spans) = &direct[(z * 8 + x) as usize];
                let expected = exact[(z * 8 + x) as usize].0;
                if let (Some(surface), Some(expected)) = (surface, expected) {
                    assert!(
                        (surface.height - expected.height).abs() <= 32,
                        "coarse height exceeds its vertical resolution at {wx} {wz}: {} vs {}",
                        surface.height,
                        expected.height
                    );
                    assert!(spans.iter().any(|&(min, max, state)| min < surface.height
                        && surface.height - 1 < max
                        && state == surface.state));
                }
                assert!(spans.iter().all(|&(min, max, _)| min < max));
                assert!(spans.windows(2).all(|pair| pair[1].1 <= pair[0].0));
            }
        }
        eprintln!(
            "64 far columns: dense={dense_time:?}, direct32={:?}",
            started.elapsed()
        );
        let end =
            OverworldGenerator::for_dimension(data, 1234, "minecraft:normal", "minecraft:the_end")
                .expect("End generator");
        let mut worker = end.worker();
        assert!(worker.surface_sample(0, 0).is_some());
        assert!(worker.surface_sample(256, 0).is_none());
        let (surface, spans) = worker.surface_column(256, 0);
        assert!(surface.is_none());
        assert!(spans.is_empty());
        let (surface, spans) = worker
            .lod_grid(256, 0, 32, 1, false)
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            surface.is_none() && spans.is_empty(),
            "direct sampling retains End void"
        );
    }

    #[test]
    fn local_carver_cache_is_worker_owned_and_eviction_preserves_terrain() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let generator = OverworldGenerator::new(data.clone(), 12345).unwrap();
        let mut first = generator.worker();
        let mut second = generator.worker();
        let pos = ChunkPos::new(0, 0);
        let before = first.generate_proto(pos).unwrap();
        assert!(!first.state.carver_biomes.is_empty());
        assert!(second.state.carver_biomes.is_empty());
        second.generate_proto(pos).unwrap();
        assert_eq!(first.state.carver_biomes, second.state.carver_biomes);
        let expected = first.state.carver_biomes.clone();
        first.state.carver_biomes.clear();
        // Fill unrelated origins to force eviction on the next local miss.
        for x in 0..4096 {
            first.state.carver_biomes.insert((x, 100_000), BiomeId(0));
        }
        let after = first.generate_proto(pos).unwrap();
        assert_eq!(first.state.carver_biomes, expected);
        for y in -64..320 {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(before.block(x, y, z), after.block(x, y, z));
                    assert_eq!(before.biome_id_at(x, y, z), after.biome_id_at(x, y, z));
                }
            }
        }
        assert!(
            OverworldGenerator::new(data, -1)
                .unwrap()
                .worker()
                .state
                .carver_biomes
                .is_empty()
        );
        assert_eq!(second.state.carver_biomes, expected);
    }

    /// The prepared worldgen data, if this checkout has it.
    fn prepared_data() -> Option<Arc<GeneratorData>> {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return None;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        Some(GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data"))
    }

    /// Seed 1234's strongholds where vanilla 26.3 puts them
    /// (VanillaStructureStartOracle with HORIZON_RINGS). Each search sampled
    /// its quarts through fresh resolvers, which broke climate ties their own
    /// way: three of the 128 were up to 6 chunks off.
    #[test]
    fn stronghold_rings_match_vanilla() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data, 1234).expect("generator");
        let rings = generator
            .worker()
            .ring_positions("minecraft:strongholds")
            .expect("ring positions");
        assert_eq!(rings.len(), 128);
        for (i, x, z) in [
            (0, -100, -125),
            (1, 131, -12),
            (111, 1190, -560),
            (113, 1305, -112),
        ] {
            assert_eq!(rings[i], ChunkPos::new(x, z), "stronghold {i}");
        }
    }

    /// Seed 8675309's giant ruined portal starts in chunk (-26, 23) and lies
    /// wholly in (-26, 24): its own chunk still holds the start, as vanilla's
    /// getAllStarts does, though no piece reaches it.
    #[test]
    fn a_chunk_holds_its_starts_whose_pieces_lie_elsewhere() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data, 8675309).expect("generator");
        let mut worker = generator.worker();
        let proto = worker
            .generate_proto(ChunkPos::new(-26, 23))
            .expect("proto");
        let own: Vec<_> = proto.own_starts().iter().map(|s| s.name.clone()).collect();
        assert!(
            own.iter().any(|n| n == "minecraft:ruined_portal_mountain"),
            "{own:?}"
        );
    }

    /// Chunks no ring search can end near are decided without the rings,
    /// which stay unsearched, and get the starts they get with them.
    #[test]
    fn chunks_far_from_the_rings_do_not_search_them() {
        let Some(data) = prepared_data() else {
            return;
        };
        let generator = OverworldGenerator::new(data.clone(), 4321).expect("generator");
        let mut worker = generator.worker();
        let summary = |worker: &mut GeneratorWorker, c: ChunkPos| -> Vec<(String, i32, i32)> {
            worker
                .structure_starts(c)
                .iter()
                .map(|s| (s.name.clone(), s.chunk.x, s.chunk.z))
                .collect()
        };
        let chunks: Vec<ChunkPos> = (-24..=24)
            .step_by(3)
            .flat_map(|x| (-24..=24).step_by(3).map(move |z| ChunkPos::new(x, z)))
            .collect();
        let before: Vec<_> = chunks.iter().map(|&c| summary(&mut worker, c)).collect();
        assert!(before.iter().any(|s| !s.is_empty()), "no starts to compare");
        let searched = || data.ring_cache.lock().unwrap()[0].1.get().is_some();
        assert!(!searched());
        let rings = worker
            .ring_positions("minecraft:strongholds")
            .expect("ring positions");
        assert_eq!(rings.len(), 128);
        assert!(searched());
        worker.forget_structure_starts();
        let after: Vec<_> = chunks.iter().map(|&c| summary(&mut worker, c)).collect();
        assert_eq!(before, after);
    }

    /// Generators of one seed and dimension share their ring positions;
    /// another seed or generator gets its own, and old entries go.
    #[test]
    fn ring_positions_are_shared_by_seed_and_dimension() {
        let Some(data) = prepared_data() else {
            return;
        };
        let first = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        let again = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        {
            let cache = data.ring_cache.lock().unwrap();
            assert_eq!(cache.len(), 1);
            // The cache's handle and the two generators'.
            assert_eq!(Arc::strong_count(&cache[0].1), 3);
        }
        drop((first, again));
        let a = data.rings_for(1, &serde_json::json!({ "settings": "a" }));
        assert!(Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "a" }))
        ));
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(2, &serde_json::json!({ "settings": "a" }))
        ));
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "b" }))
        ));
        for seed in 10..10 + RING_CACHE_LIMIT as i64 {
            data.rings_for(seed, &serde_json::json!({ "settings": "a" }));
        }
        assert_eq!(data.ring_cache.lock().unwrap().len(), RING_CACHE_LIMIT);
        assert!(!Arc::ptr_eq(
            &a,
            &data.rings_for(1, &serde_json::json!({ "settings": "a" }))
        ));
    }

    /// Generators of every seed share their biome source's decoration, the
    /// one the world screens' preload built; another dimension gets its own.
    #[test]
    fn decoration_is_shared_by_biome_source() {
        let Some(data) = prepared_data() else {
            return;
        };
        data.prepare_decoration("minecraft:normal", "minecraft:overworld")
            .expect("decoration");
        let built = |i: usize| {
            let cache = data.decorations.lock().unwrap();
            let slot = cache[i].1.get().expect("built").clone();
            slot.expect("decoration").expect("feature resources")
        };
        let prepared = built(0);
        // The cache's handle and ours.
        assert_eq!(Arc::strong_count(&prepared), 2);
        let first = OverworldGenerator::new(data.clone(), 1234).expect("generator");
        let other = OverworldGenerator::new(data.clone(), -7).expect("generator");
        assert_eq!(Arc::strong_count(&prepared), 4);
        assert_eq!(data.decorations.lock().unwrap().len(), 1);
        let nether = OverworldGenerator::for_dimension(
            data.clone(),
            1234,
            "minecraft:normal",
            "minecraft:the_nether",
        )
        .expect("generator");
        assert_eq!(data.decorations.lock().unwrap().len(), 2);
        assert!(!Arc::ptr_eq(&prepared, &built(1)));
        drop((first, other, nether));
        assert_eq!(Arc::strong_count(&prepared), 2);
    }

    #[test]
    fn parameter_reports_quantize_like_vanilla() {
        let report = r#"{"biomes":[{"biome":"minecraft:plains","parameters":{
            "temperature":[-0.45,-0.15],"humidity":[-1.0,-0.35],"continentalness":[-0.11,0.03],
            "erosion":[0.45,0.55],"depth":0.0,"weirdness":[-0.2633,-0.05],"offset":0.0}}]}"#;
        let table: Value =
            serde_json::from_str(&quantize_parameter_report(report).unwrap()).unwrap();
        assert_eq!(
            table,
            serde_json::json!([{"coordinates":[[-4500,-1500],[-10000,-3500],[-1100,300],
                [4500,5500],[0,0],[-2633,-500]],"offset":0,"biome":"minecraft:plains"}])
        );
        assert!(
            quantize_parameter_report(r#"{"biomes":[{"parameters":{"temperature":"x"}}]}"#)
                .is_err()
        );
    }
    #[test]
    fn neighbour_work_cleanup_keeps_active_keys_and_releases_idle_keys() {
        let work = ChunkWork::new();
        let first = work.at((0, 0));
        let held = first.lock().unwrap();
        let same = work.at((0, 0));
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &work.at((1, 0))));
        // Sweep many expired weak entries while a builder and a prospective
        // waiter retain their key. Cleanup must not split their coordination.
        for x in 1..8192 {
            drop(work.at((x, 1)));
        }
        assert!(
            work.0
                .iter()
                .map(|s| s.lock().unwrap().len())
                .sum::<usize>()
                <= 4096
        );
        assert!(Arc::ptr_eq(&same, &work.at((0, 0))));
        let weak = Arc::downgrade(&first);
        drop(held);
        drop((first, same));
        assert!(weak.upgrade().is_none());
        let next = work.at((0, 0));
        assert_eq!(Arc::strong_count(&next), 1);
    }

    /// Equal concurrent misses share one biome volume and the same mutable
    /// structure pieces. Exercise cold ring startup before work coordination,
    /// then cache eviction and explicit start invalidation with old Arcs alive.
    #[test]
    fn concurrent_neighbours_share_results_and_survive_invalidation() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let registry = Box::leak(Box::new(
            crate::level::data::load_registry(&root).expect("registry"),
        ));
        let data = GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
        let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
        let generator_json = &preset["dimensions"]["minecraft:overworld"]["generator"];
        let settings: Value =
            serde_json::from_str(&data.settings[generator_json["settings"].as_str().unwrap()])
                .unwrap();
        let pipeline = Arc::new(
            Pipeline::new(
                data,
                12345,
                XoroshiroRandom::new(12345),
                generator_json,
                &settings,
                true,
                Some((-64, 384)),
            )
            .expect("pipeline"),
        );
        let generator = OverworldGenerator {
            pipeline: pipeline.clone(),
            noise_clamped: None,
        };
        let barrier = std::sync::Barrier::new(6);
        let pos = ChunkPos::new(11, -11); // ancient city for this seed
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        let mut worker = generator.worker();
                        barrier.wait();
                        let biomes = pipeline.biome_chunk(&mut worker.state, pos).unwrap();
                        let starts = pipeline.structure_starts(&mut worker.state, pos);
                        (biomes, starts)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        let (biomes, starts) = &results[0];
        assert!(!starts.is_empty(), "fixture must exercise shared pieces");
        for (b, s) in &results[1..] {
            assert!(Arc::ptr_eq(biomes, b));
            assert_eq!(starts.len(), s.len());
            for (first, other) in starts.iter().zip(s) {
                assert!(Arc::ptr_eq(first, other));
            }
        }
        // Force the existing whole-cache biome eviction threshold.
        {
            let mut cache = pipeline.biome_cache.lock().unwrap();
            cache.clear();
            for x in 0..CACHE_LIMIT as i32 {
                cache.insert((x, 10000), biomes.clone());
            }
        }
        let mut worker = generator.worker();
        let rebuilt = pipeline.biome_chunk(&mut worker.state, pos).unwrap();
        assert!(!Arc::ptr_eq(biomes, &rebuilt));
        assert_eq!(pipeline.biome_cache.lock().unwrap().len(), 1);
        for section in 0..24 {
            assert_eq!(biomes.section(section), rebuilt.section(section));
        }
        worker.forget_structure_starts();
        let fresh = worker.structure_starts(pos);
        assert_eq!(starts.len(), fresh.len());
        for (old, new) in starts.iter().zip(&fresh) {
            assert!(!Arc::ptr_eq(old, new));
            assert_eq!(old.name, new.name);
            assert_eq!(old.bounding_box, new.bounding_box);
        }
        let again = worker.structure_starts(pos);
        for (first, hit) in fresh.iter().zip(&again) {
            assert!(Arc::ptr_eq(first, hit));
        }
    }

    /// The resources the worldgen tests read.
    fn test_root() -> std::path::PathBuf {
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            })
    }
    /// The block registry the worldgen bundle needs. `GeneratorData` keeps it
    /// for as long as the generator lives, so it is shared, not freed.
    static TEST_REGISTRY: std::sync::LazyLock<BlockRegistry> = std::sync::LazyLock::new(|| {
        crate::level::data::load_registry(&test_root()).expect("registry")
    });

    /// The world presets with their own noise settings generate the terrain
    /// those settings describe: the amplified and large-biome presets are not
    /// the overworld's terrain under another name.
    #[test]
    fn world_presets_use_their_own_noise_settings() {
        let root = test_root();
        if !root.join("worldgen/world_presets.json").exists() {
            eprintln!("skipping: no worldgen bundle at {}", root.display());
            return;
        }
        let data =
            GeneratorData::load(&root.join("worldgen"), &TEST_REGISTRY).expect("worldgen data");
        let surface = |preset: &str| {
            let generator = OverworldGenerator::for_world(
                data.clone(),
                1234,
                preset,
                "minecraft:overworld",
                false,
            )
            .expect("generator");
            let mut worker = generator.worker();
            (0..32)
                .map(|i| {
                    worker
                        .surface_sample(i * 137, i * 91)
                        .expect("surface")
                        .height
                })
                .collect::<Vec<_>>()
        };
        let normal = surface("minecraft:normal");
        assert!(normal.iter().all(|height| (0..384).contains(height)));
        assert_ne!(normal, surface("minecraft:amplified"));
        assert_ne!(normal, surface("minecraft:large_biomes"));
    }
}
