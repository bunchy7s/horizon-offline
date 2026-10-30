//! Configured and placed features for a live level: what vanilla block
//! behaviour reaches through `Feature.place(level, generator, random, pos)`
//! on a `ServerLevel` (sapling and fungus growth, bone meal on grass, nylium,
//! moss, huge mushrooms). Definitions are decoded lazily by name from the same
//! worldgen bundle as decoration and cached.
use super::{
    biome::BiomeId,
    biome_predicate::BiomePredicateBindings,
    block_feature::FeatureBlockBindings,
    feature::{FeatureBlocks, FeatureWorld},
    feature_catalog::FeatureCatalog,
    feature_graph::{FeatureDefinition, PlacedFeatureDefinition},
    generator::{GeneratorData, GeneratorError},
    placement::PlacementContext,
    state_provider::{StateProviderBindings, StateProviderCatalog},
    tags::TagDefinitions,
};
use crate::block_placement::BlockPlacementBindings;
use horizon_core::random::RandomSource;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

type Cache<T> = Mutex<HashMap<String, Option<Arc<T>>>>;

pub struct LiveFeatures {
    data: Arc<GeneratorData>,
    catalog: FeatureCatalog,
    bindings: FeatureBlockBindings<'static>,
    blocks: FeatureBlocks,
    configured: Cache<FeatureDefinition>,
    placed: Cache<PlacedFeatureDefinition>,
    /// `BiomeGenerationSettings.getBoneMealFeatures` per biome, in vanilla order.
    bone_meal: Vec<Vec<String>>,
    /// Temperature settings per biome (`Biome.getTemperature`).
    temperatures: Vec<super::biome_temperature::BiomeTemperatureSettings>,
}

fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}

/// `ConfiguredFeature.getFeatures`: nested configured-feature references of a
/// feature JSON, depth first (each reference followed by its own children).
fn nested_features(
    json: &Value,
    features: &std::collections::BTreeMap<String, String>,
    depth: usize,
    out: &mut Vec<String>,
) {
    if depth > 64 {
        return;
    }
    match json {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "feature"
                    && let Some(name) = value.as_str()
                {
                    let name = super::tags::identifier(name).unwrap_or_else(|_| name.to_owned());
                    out.push(name.clone());
                    if let Some(child) = features
                        .get(&name)
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    {
                        nested_features(&child, features, depth + 1, out);
                    }
                } else {
                    nested_features(value, features, depth + 1, out);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                nested_features(value, features, depth + 1, out);
            }
        }
        _ => {}
    }
}

impl LiveFeatures {
    /// None when the bundle carries no feature resources.
    pub fn new(data: &Arc<GeneratorData>) -> Result<Option<Self>, GeneratorError> {
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
        let mut bindings = data
            .feature_block_bindings()?
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
        // Configured-feature tag membership (feature_tags names configured features).
        let mut bone_meal_tag = BTreeSet::new();
        if let Some(json) = resources
            .feature_tags
            .get("minecraft:can_spawn_from_bone_meal")
        {
            let value: Value = serde_json::from_str(json)?;
            for entry in value["values"].as_array().into_iter().flatten() {
                if let Some(name) = entry.as_str() {
                    bone_meal_tag.insert(super::tags::identifier(name)?);
                }
            }
        }
        let placed_tags = feature_tags
            .build(|name, _| resources.placed.contains_key(name).then(|| name.to_owned()))
            .map_err(|e| resource(format!("feature tags: {e:?}")))?
            .tags;
        let mut bone_meal = Vec::with_capacity(data.biome_names().len());
        for i in 0..data.biome_names().len() {
            let biome: Value = serde_json::from_str(data.biome_json(BiomeId(i as u32)))?;
            let mut list = Vec::new();
            for step in biome["features"].as_array().into_iter().flatten() {
                let entries: Vec<String> = match step {
                    Value::String(tag) if tag.starts_with('#') => placed_tags
                        .get(&super::tags::identifier(&tag[1..])?)
                        .cloned()
                        .unwrap_or_default(),
                    Value::String(name) => vec![name.clone()],
                    Value::Array(values) => values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                };
                for entry in entries {
                    let name = super::tags::identifier(&entry)?;
                    let Some(placed) = resources
                        .placed
                        .get(&name)
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    else {
                        continue;
                    };
                    // PlacedFeature.getFeatures: its feature, then that feature's children.
                    let mut found = Vec::new();
                    match &placed["feature"] {
                        Value::String(feature) => {
                            let feature = super::tags::identifier(feature)?;
                            found.push(feature.clone());
                            if let Some(child) = resources
                                .features
                                .get(&feature)
                                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                            {
                                nested_features(&child, &resources.features, 0, &mut found);
                            }
                        }
                        inline => nested_features(inline, &resources.features, 0, &mut found),
                    }
                    list.extend(found.into_iter().filter(|f| bone_meal_tag.contains(f)));
                }
            }
            bone_meal.push(list);
        }
        let temperatures = (0..data.biome_names().len())
            .map(|i| {
                super::biome_temperature::BiomeTemperatureSettings::from_json(
                    data.biome_json(BiomeId(i as u32)),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Self {
            temperatures,
            data: data.clone(),
            catalog,
            bindings,
            blocks: FeatureBlocks::from_registry(registry)?,
            configured: Mutex::new(HashMap::new()),
            placed: Mutex::new(HashMap::new()),
            bone_meal,
        }))
    }
    pub fn data(&self) -> &Arc<GeneratorData> {
        &self.data
    }
    pub fn biome_id(&self, name: &str) -> Option<BiomeId> {
        self.data.biome_id(name)
    }
    /// Raw JSON of a configured feature (for configuration values such as a
    /// trunk placer's base height).
    pub fn configured_json(&self, name: &str) -> Option<Value> {
        let resources = self.data.feature_resources.as_ref()?;
        serde_json::from_str(resources.features.get(name)?).ok()
    }
    pub fn configured(&self, name: &str) -> Option<Arc<FeatureDefinition>> {
        let mut cache = self.configured.lock().unwrap();
        cache
            .entry(name.to_owned())
            .or_insert_with(|| {
                match self
                    .catalog
                    .feature_with_blocks(&Value::from(name).to_string(), &self.bindings)
                {
                    Ok(f) => Some(Arc::new(f)),
                    Err(e) => {
                        eprintln!("Horizon: feature {name} cannot be placed: {e}");
                        None
                    }
                }
            })
            .clone()
    }
    pub fn placed(&self, name: &str) -> Option<Arc<PlacedFeatureDefinition>> {
        let mut cache = self.placed.lock().unwrap();
        cache
            .entry(name.to_owned())
            .or_insert_with(|| {
                match self
                    .catalog
                    .placed_feature_with_blocks(&Value::from(name).to_string(), &self.bindings)
                {
                    Ok(f) => Some(Arc::new(f)),
                    Err(e) => {
                        eprintln!("Horizon: placed feature {name} cannot be placed: {e}");
                        None
                    }
                }
            })
            .clone()
    }
    /// `ConfiguredFeature.place`. Unknown or unsupported features and
    /// execution errors report failure.
    pub fn place_configured<W: FeatureWorld + PlacementContext>(
        &self,
        name: &str,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> bool
    where
        W::Error: std::fmt::Display,
    {
        let Some(feature) = self.configured(name) else {
            return false;
        };
        match feature.place(world, random, origin, self.blocks) {
            Ok(placed) => placed,
            Err(e) => {
                eprintln!("Horizon: placing {name} failed: {e}");
                false
            }
        }
    }
    /// `PlacedFeature.place` (placement modifiers, then the feature).
    pub fn place_placed<W: FeatureWorld + PlacementContext>(
        &self,
        name: &str,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> bool
    where
        W::Error: std::fmt::Display,
    {
        let Some(feature) = self.placed(name) else {
            return false;
        };
        match feature.place(world, random, origin, self.blocks) {
            Ok(placed) => placed,
            Err(e) => {
                eprintln!("Horizon: placing {name} failed: {e}");
                false
            }
        }
    }
    /// Temperature settings of a biome.
    pub fn biome_temperature(
        &self,
        biome: BiomeId,
    ) -> Option<super::biome_temperature::BiomeTemperatureSettings> {
        self.temperatures.get(biome.0 as usize).copied()
    }
    /// Whether the named biome is in a biome tag (`minecraft:...`).
    pub fn biome_in_tag(&self, biome: &str, tag: &str) -> bool {
        self.data
            .biome_tags
            .get(tag)
            .is_some_and(|members| members.iter().any(|m| m == biome))
    }
    /// A block tag's members in registry tag order (for `getRandomElementOf`).
    pub fn block_tag_entries(&self, tag: &str) -> &[String] {
        self.data.block_tags.get(tag).map_or(&[], Vec::as_slice)
    }
    /// `BiomeGenerationSettings.getBoneMealFeatures` of a biome.
    pub fn bone_meal_features(&self, biome: BiomeId) -> &[String] {
        self.bone_meal
            .get(biome.0 as usize)
            .map_or(&[], Vec::as_slice)
    }
}
