//! Registry-bound block-writing features and their state codec bindings.
use super::{
    DecodeError, Object,
    density::identifier,
    feature::FeatureWorld,
    int_range,
    placement::{PlacementContext, PlacementQueryError},
    required,
};
use crate::block_properties::{BlockPropertiesRegistry, BlockProperty, BoundBlockProperty};
use crate::registry::{BlockRegistry, BlockStateId, BlockStateJsonCodec};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Block states by block name and by tag name ([`FeatureBlockBindings::bind_tags`]).
pub type BoundBlockTags = (
    Arc<BTreeMap<String, Arc<BTreeSet<BlockStateId>>>>,
    Arc<BoundTags>,
);

/// Block tags' states by tag name, each bound the first time it is read: of
/// vanilla's 300 block tags (406,779 states in all), world generation reads a
/// few dozen. A tag's blocks are checked when it is made, so binding it later
/// cannot fail; once bound, reading it takes no lock.
pub struct BoundTags {
    blocks: Arc<BTreeMap<String, Arc<BTreeSet<BlockStateId>>>>,
    tags: BTreeMap<
        String,
        (
            Vec<String>,
            std::sync::OnceLock<Arc<BTreeSet<BlockStateId>>>,
        ),
    >,
}
impl BoundTags {
    /// The states of the tag `name`, bound on first use.
    pub fn get<Q: Ord + ?Sized>(&self, name: &Q) -> Option<&Arc<BTreeSet<BlockStateId>>>
    where
        String: std::borrow::Borrow<Q>,
    {
        let (blocks, states) = self.tags.get(name)?;
        Some(states.get_or_init(|| {
            Arc::new(
                blocks
                    .iter()
                    .flat_map(|block| {
                        self.blocks
                            .get::<str>(block)
                            .into_iter()
                            .flat_map(|states| states.iter().copied())
                    })
                    .collect(),
            )
        }))
    }
}
pub struct FeatureBlockBindings<'r> {
    registry: &'r BlockRegistry,
    pub(crate) simple_blocks: Option<(
        super::state_provider::StateProviderBindings<'r>,
        super::state_provider::StateProviderCatalog,
        crate::block_placement::BlockPlacementBindings<'r>,
    )>,
    physical_properties: Option<[BoundBlockProperty; 4]>,
    pub(crate) occlusion_faces: Option<crate::block_properties::BoundOcclusionFaces>,
    pub(crate) can_occlude: Option<BoundBlockProperty>,
    biomes: Option<Arc<super::biome_predicate::BiomePredicateBindings>>,
    pub(crate) fluids: super::fluid_predicate::FluidPredicateBindings,
    pub(crate) codec: BlockStateJsonCodec<'r>,
    pub(crate) blocks: Arc<BTreeMap<String, Arc<BTreeSet<BlockStateId>>>>,
    pub(crate) tags: Arc<BoundTags>,
    pub(crate) air: [BlockStateId; 3],
    tree_blocks: std::sync::OnceLock<Arc<super::tree_blocks::TreeBlocks>>,
    pub(crate) structures: Option<Arc<super::structure::StructureResources>>,
}
impl<'r> FeatureBlockBindings<'r> {
    /// Shared tree/vegetation classifications, built on first use. Requires
    /// state-provider and block-property bindings.
    /// Bind structure templates and processor lists for template features.
    pub fn with_structures(
        mut self,
        structures: Arc<super::structure::StructureResources>,
    ) -> Self {
        self.structures = Some(structures);
        self
    }
    pub(crate) fn tree_blocks(&self) -> Result<Arc<super::tree_blocks::TreeBlocks>, DecodeError> {
        if let Some(blocks) = self.tree_blocks.get() {
            return Ok(blocks.clone());
        }
        let blocks = Arc::new(super::tree_blocks::TreeBlocks::build(self)?);
        Ok(self.tree_blocks.get_or_init(|| blocks).clone())
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    /// BlockState.isSolid (legacy solidity), when block properties are bound.
    pub(crate) fn solid(&self) -> Result<BoundBlockProperty, DecodeError> {
        self.physical_properties
            .as_ref()
            .map(|p| p[0].clone())
            .ok_or(DecodeError::Field("block properties"))
    }

    /// Bind provider resources and precompiled placement operations to this registry.
    pub fn with_state_providers(
        mut self,
        providers: super::state_provider::StateProviderBindings<'r>,
        catalog: super::state_provider::StateProviderCatalog,
        placement: crate::block_placement::BlockPlacementBindings<'r>,
    ) -> Result<Self, DecodeError> {
        if !std::ptr::eq(self.registry, providers.registry())
            || !std::ptr::eq(self.registry, placement.registry())
        {
            return Err(DecodeError::Field(
                "feature provider/placement registry identity",
            ));
        }
        self.simple_blocks = Some((providers, catalog, placement));
        Ok(self)
    }

    pub fn with_block_properties(
        mut self,
        properties: &BlockPropertiesRegistry<'r>,
    ) -> Result<Self, DecodeError> {
        if !std::ptr::eq(self.registry, properties.registry()) {
            return Err(DecodeError::Field(
                "block properties use a different registry",
            ));
        }
        self.physical_properties = Some([
            properties.bind(BlockProperty::Solid),
            properties.bind(BlockProperty::Replaceable),
            properties.bind(BlockProperty::Liquid),
            properties.bind(BlockProperty::SolidRender),
        ]);
        self.occlusion_faces = Some(properties.bind_occlusion_faces());
        self.can_occlude = Some(properties.bind_can_occlude());
        Ok(self)
    }
    pub fn with_biomes(
        mut self,
        biomes: Arc<super::biome_predicate::BiomePredicateBindings>,
    ) -> Self {
        self.biomes = Some(biomes);
        self
    }

    pub fn with_fluid_tags(
        mut self,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        self.fluids = self.fluids.with_resolved_tags(tags)?;
        Ok(self)
    }
    pub fn new(
        registry: &'r BlockRegistry,
        integers: BTreeSet<(String, String)>,
    ) -> Result<Self, DecodeError> {
        Self::with_tags(registry, integers, &BTreeMap::new())
    }
    /// Bind an immutable snapshot of already-resolved block tags.
    pub fn with_tags(
        registry: &'r BlockRegistry,
        integers: BTreeSet<(String, String)>,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        let bound = Self::bind_tags(registry, tags)?;
        Self::with_bound_tags(registry, integers, bound)
    }
    /// Every block's states and every tag's, by name: what `with_tags` binds,
    /// to share between the bindings made over the same registry and tags.
    pub fn bind_tags(
        registry: &BlockRegistry,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<BoundBlockTags, DecodeError> {
        let mut blocks = BTreeMap::<String, BTreeSet<BlockStateId>>::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            blocks
                .entry(registry.state(id).unwrap().name.clone())
                .or_default()
                .insert(id);
        }
        let mut bound_tags = BTreeMap::new();
        for (name, entries) in tags {
            let mut tag_blocks = Vec::with_capacity(entries.len());
            for name in entries {
                let block = identifier(name)?;
                if !blocks.contains_key(&block) {
                    return Err(DecodeError::Registry(format!("unknown block {name}")));
                }
                tag_blocks.push(block);
            }
            if bound_tags
                .insert(
                    super::tags::identifier(name)?,
                    (tag_blocks, std::sync::OnceLock::new()),
                )
                .is_some()
            {
                return Err(DecodeError::Field("duplicate block tag"));
            }
        }
        let blocks: Arc<BTreeMap<_, _>> = Arc::new(
            blocks
                .into_iter()
                .map(|(name, states)| (name, Arc::new(states)))
                .collect(),
        );
        Ok((
            blocks.clone(),
            Arc::new(BoundTags {
                blocks,
                tags: bound_tags,
            }),
        ))
    }
    /// [`FeatureBlockBindings::with_tags`] over tags already bound by
    /// [`FeatureBlockBindings::bind_tags`].
    pub fn with_bound_tags(
        registry: &'r BlockRegistry,
        integers: BTreeSet<(String, String)>,
        (blocks, tags): BoundBlockTags,
    ) -> Result<Self, DecodeError> {
        let codec = BlockStateJsonCodec::new(registry, integers)
            .map_err(|e| DecodeError::Registry(e.to_string()))?;
        let get = |name| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown air block {name}")))
        };
        Ok(Self {
            registry,
            simple_blocks: None,
            codec,
            biomes: None,
            physical_properties: None,
            occlusion_faces: None,
            can_occlude: None,
            fluids: super::fluid_predicate::FluidPredicateBindings::new(registry)?,
            blocks,
            tags,
            tree_blocks: std::sync::OnceLock::new(),
            structures: None,
            air: [
                get("minecraft:air")?,
                get("minecraft:cave_air")?,
                get("minecraft:void_air")?,
            ],
        })
    }
}
impl super::block_predicate::PredicateBlockResolver for FeatureBlockBindings<'_> {
    fn predicate_state(&self, json: &str) -> Result<BlockStateId, DecodeError> {
        self.codec
            .decode(json, false)
            .map_err(|e| DecodeError::Registry(e.to_string()))
    }
    fn predicate_property(
        &self,
        property: BlockProperty,
    ) -> Result<BoundBlockProperty, DecodeError> {
        super::block_predicate::bound_property(self.physical_properties.as_ref(), property)
    }
    fn predicate_biome_set(
        &self,
        json: &str,
    ) -> Result<Arc<BTreeSet<super::biome::BiomeId>>, DecodeError> {
        self.biomes
            .as_ref()
            .ok_or(DecodeError::Field("predicate biome registry"))?
            .resolve(json)
    }

    fn predicate_fluid_set(
        &self,
        json: &str,
    ) -> Result<super::fluid_predicate::BoundFluidSet, DecodeError> {
        self.fluids.resolve(json)
    }
    fn predicate_block_set(&self, json: &str) -> Result<Arc<BTreeSet<BlockStateId>>, DecodeError> {
        if let Ok(name) = serde_json::from_str::<String>(json) {
            if let Some(tag) = name.strip_prefix('#') {
                return self
                    .tags
                    .get(&super::tags::identifier(tag)?)
                    .cloned()
                    .ok_or_else(|| {
                        DecodeError::Registry(format!("unknown predicate block tag {tag}"))
                    });
            }
            return self
                .blocks
                .get(&identifier(&name)?)
                .cloned()
                .ok_or_else(|| DecodeError::Registry(format!("unknown predicate block {name}")));
        }
        let names: Vec<String> = serde_json::from_str(json)?;
        if names.len() > 8192 {
            return Err(DecodeError::Field("predicate block list size"));
        }
        let mut states = BTreeSet::new();
        for name in names {
            states.extend(
                self.blocks
                    .get(&identifier(&name)?)
                    .ok_or_else(|| {
                        DecodeError::Registry(format!("unknown predicate block {name}"))
                    })?
                    .iter()
                    .copied(),
            );
        }
        Ok(Arc::new(states))
    }
}
#[derive(Clone, Debug)]
pub struct FillLayerFeature {
    height: i32,
    state: BlockStateId,
    air: [BlockStateId; 3],
}
#[derive(Debug)]
pub enum BlockFeatureError<E> {
    World(E),
    Query(PlacementQueryError),
}
impl<E: std::fmt::Display> std::fmt::Display for BlockFeatureError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockFeatureError<E> {}
impl FillLayerFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:fill_layer" {
            return Err(DecodeError::Field("fill-layer feature type"));
        }
        Ok(Self {
            height: int_range(required(&object, "height")?, "fill-layer height", 0, 4064)?,
            state: bindings
                .codec
                .decode(required(&object, "state")?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))?,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        origin: [i32; 3],
    ) -> Result<bool, BlockFeatureError<W::Error>> {
        for dx in 0..16 {
            for dz in 0..16 {
                // Vanilla re-queries the world minimum for each column.
                let y = world
                    .min_y()
                    .map_err(BlockFeatureError::Query)?
                    .wrapping_add(self.height);
                let position = [origin[0].wrapping_add(dx), y, origin[2].wrapping_add(dz)];
                let state = world.block(position).map_err(BlockFeatureError::World)?;
                if self.air.contains(&state) {
                    world
                        .set_block(position, self.state, 2)
                        .map_err(BlockFeatureError::World)?;
                }
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Debug)]
pub struct ReplaceBlockFeature {
    targets: Vec<(super::rule_test::RuleTest, BlockStateId)>,
}
impl ReplaceBlockFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:replace_single_block" {
            return Err(DecodeError::Field("replace-block feature type"));
        }
        let targets = decode_targets(&object, bindings, depth, remaining)?;
        Ok(Self { targets })
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl horizon_core::random::RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BlockFeatureError<W::Error>> {
        for (rule, state) in &self.targets {
            let current = world.block(origin).map_err(BlockFeatureError::World)?;
            if rule.test(current, origin, random) {
                world
                    .set_block(origin, *state, 2)
                    .map_err(BlockFeatureError::World)?;
                break;
            }
        }
        Ok(true)
    }
}

pub(crate) type BlockReplacements = Vec<(super::rule_test::RuleTest, BlockStateId)>;
pub(crate) fn decode_targets(
    object: &Object,
    bindings: &FeatureBlockBindings<'_>,
    depth: usize,
    remaining: &mut usize,
) -> Result<BlockReplacements, DecodeError> {
    let values: Vec<Box<serde_json::value::RawValue>> =
        serde_json::from_str(required(object, "targets")?.get())?;
    if values.len() > 8192 {
        return Err(DecodeError::Field("replacement targets"));
    }
    values
        .iter()
        .map(|value| {
            let object: Object = serde_json::from_str(value.get())?;
            Ok((
                super::rule_test::RuleTest::decode(
                    required(&object, "target")?.get(),
                    bindings,
                    depth + 1,
                    remaining,
                )?,
                bindings
                    .codec
                    .decode(required(&object, "state")?.get(), false)
                    .map_err(|e| DecodeError::Registry(e.to_string()))?,
            ))
        })
        .collect::<Result<_, DecodeError>>()
}

#[cfg(test)]
mod bound_tag_tests {
    use super::*;
    use crate::nbt::Tag;

    fn registry() -> BlockRegistry {
        let state = |id: &str| Tag::compound([("id", Tag::String(id.into()))]);
        let log = |axis: &str| {
            Tag::compound([
                ("id", Tag::String("minecraft:oak_log".into())),
                (
                    "properties",
                    Tag::compound([("axis", Tag::String(axis.into()))]),
                ),
            ])
        };
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![
                    state("minecraft:air"),
                    state("minecraft:stone"),
                    log("x"),
                    log("y"),
                    state("minecraft:dirt"),
                ]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                    ("minecraft:oak_log", Tag::Int(3)),
                    ("minecraft:dirt", Tag::Int(4)),
                ]),
            ),
        ]))
        .unwrap()
    }

    fn tags(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(name, blocks)| {
                (
                    name.to_string(),
                    blocks.iter().map(|b| b.to_string()).collect(),
                )
            })
            .collect()
    }

    /// A tag reads as every state of its blocks, whichever is read first; a
    /// tag that names an unknown block still fails the binding itself.
    #[test]
    fn tags_read_as_their_blocks_states() {
        let registry = registry();
        let (blocks, bound) = FeatureBlockBindings::bind_tags(
            &registry,
            &tags(&[
                ("minecraft:logs", &["minecraft:oak_log"]),
                ("minecraft:base", &["stone", "minecraft:dirt", "oak_log"]),
                ("minecraft:empty", &[]),
            ]),
        )
        .unwrap();
        let ids = |set: &BTreeSet<BlockStateId>| set.iter().map(|s| s.0).collect::<Vec<_>>();
        assert_eq!(ids(&blocks["minecraft:oak_log"]), [2, 3]);
        assert_eq!(ids(bound.get("minecraft:base").unwrap()), [1, 2, 3, 4]);
        assert_eq!(ids(bound.get("minecraft:logs").unwrap()), [2, 3]);
        assert_eq!(ids(bound.get("minecraft:base").unwrap()), [1, 2, 3, 4]);
        assert!(bound.get("minecraft:empty").unwrap().is_empty());
        assert!(bound.get("minecraft:leaves").is_none());
        let unknown = FeatureBlockBindings::bind_tags(
            &registry,
            &tags(&[("minecraft:logs", &["minecraft:birch_log"])]),
        );
        assert!(matches!(unknown, Err(DecodeError::Registry(_))));
    }
}
