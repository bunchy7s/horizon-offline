//! Immutable material-rule definitions with a world-seeded density compiler.
use super::{MaterialCondition, MaterialConditionRegistry, MaterialRule, codec::identifier};
use crate::{
    registry::{BlockStateId, BlockStateJsonCodec},
    worldgen::{
        DecodeError, Object,
        aquifer::{AquiferConfig, AquiferSamplers},
        density::DensityRegistry,
        number_text, required,
        router::{NoiseRouter, RouterSamplers},
    },
};
use horizon_core::density::{
    DensitySampler,
    graph::{DensityCompiler, FunctionId},
};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug)]
enum Definition {
    Block(BlockStateId),
    Bandlands,
    Sequence(Vec<Arc<Self>>),
    Condition(MaterialCondition, Arc<Self>),
    Reference(Arc<Self>),
    Ore {
        blocks: [BlockStateId; 3],
        chance: f32,
        roots: [usize; 3],
    },
}

/// Definitions are decoded atomically. Density samplers are compiled lazily for
/// this world's seed; create workers with the same seed/random mode and a fresh
/// cached SamplerContext per worker. The library does not own mutable chunk data.
#[derive(Debug)]
pub struct MaterialRuleLibrary {
    definitions: BTreeMap<String, Arc<Definition>>,
    roots: Vec<FunctionId>,
    compiler: DensityCompiler,
    compiled: BTreeMap<usize, Arc<MaterialRule>>,
}
impl MaterialRuleLibrary {
    pub fn from_json(
        definitions: &BTreeMap<String, String>,
        conditions: &MaterialConditionRegistry,
        blocks: &BlockStateJsonCodec<'_>,
        mut densities: DensityRegistry,
        seed: i64,
        legacy: bool,
    ) -> Result<Self, DecodeError> {
        if definitions.len() > 8192 {
            return Err(DecodeError::Field("material rule registry size"));
        }
        let mut raw = BTreeMap::new();
        for (name, json) in definitions {
            let value: Box<RawValue> = serde_json::from_str(json)?;
            if raw.insert(identifier(name)?, value).is_some() {
                return Err(DecodeError::Field("duplicate material rule"));
            }
        }
        let mut loader = Loader {
            raw,
            values: BTreeMap::new(),
            active: BTreeSet::new(),
            conditions,
            blocks,
            roots: Vec::new(),
            nodes: 0,
        };
        for name in loader.raw.keys().cloned().collect::<Vec<_>>() {
            loader.resolve(&name, 0)?;
        }
        let roots = densities
            .decode_functions(&loader.roots.iter().map(String::as_str).collect::<Vec<_>>())?;
        Ok(Self {
            definitions: loader.values,
            roots,
            compiler: DensityCompiler::new_seeded(densities.into_graph(), seed, legacy),
            compiled: BTreeMap::new(),
        })
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(String::as_str)
    }
    /// Resolve a named rule without sampling chunk densities yet. Binding the
    /// returned rule to MaterialConditions performs eager per-chunk sampling.
    pub fn rule(&mut self, name: &str) -> Result<MaterialRule, DecodeError> {
        let definition = self
            .definitions
            .get(&identifier(name)?)
            .ok_or(DecodeError::Field("unknown material rule"))?
            .clone();
        Ok(MaterialRule::Reference(self.compile(&definition)?))
    }
    /// Share this compiler's cache namespace with preliminary surface sampling.
    pub fn named_density(&mut self, name: &str) -> Result<Arc<dyn DensitySampler>, DecodeError> {
        let id = self
            .compiler
            .graph()
            .function(&identifier(name)?)
            .ok_or(DecodeError::Field("unknown material density"))?;
        Ok(self.compiler.get_sampler(id)?)
    }
    /// The sampler of a function decoded into the density registry passed to
    /// `from_json`, sharing this compiler's cache namespace.
    pub fn function_sampler(
        &mut self,
        id: FunctionId,
    ) -> Result<Arc<dyn DensitySampler>, DecodeError> {
        Ok(self.compiler.get_sampler(id)?)
    }
    /// Compile a router decoded into the density registry passed to `from_json`.
    /// Its roots share cache IDs with all material and aquifer samplers.
    pub fn compile_router(&mut self, router: NoiseRouter) -> Result<RouterSamplers, DecodeError> {
        Ok(router.compile(&mut self.compiler)?)
    }
    /// Compile an aquifer decoded into the same density registry as this library.
    /// Reuse the returned roots and the router in one worker's sampler context.
    pub fn compile_aquifer(
        &mut self,
        config: AquiferConfig,
    ) -> Result<AquiferSamplers, DecodeError> {
        Ok(config.compile(&mut self.compiler)?)
    }
    fn compile(&mut self, definition: &Arc<Definition>) -> Result<Arc<MaterialRule>, DecodeError> {
        let key = Arc::as_ptr(definition) as usize;
        if let Some(value) = self.compiled.get(&key) {
            return Ok(value.clone());
        }
        let value = Arc::new(match definition.as_ref() {
            Definition::Block(id) => MaterialRule::Block(*id),
            Definition::Bandlands => MaterialRule::Bandlands,
            Definition::Reference(value) => MaterialRule::Reference(self.compile(value)?),
            Definition::Sequence(children) => MaterialRule::Sequence(
                children
                    .iter()
                    .map(|child| self.compile(child).map(MaterialRule::Reference))
                    .collect::<Result<_, _>>()?,
            ),
            Definition::Condition(condition, child) => MaterialRule::Condition {
                if_true: condition.clone(),
                then_run: Box::new(MaterialRule::Reference(self.compile(child)?)),
            },
            Definition::Ore {
                blocks,
                chance,
                roots,
            } => MaterialRule::OreVein {
                ore: blocks[0],
                raw_ore: blocks[1],
                filler: blocks[2],
                raw_ore_chance: *chance,
                density: self.compiler.get_sampler(self.roots[roots[0]])?,
                richness: self.compiler.get_sampler(self.roots[roots[1]])?,
                filler_gap: self.compiler.get_sampler(self.roots[roots[2]])?,
            },
        });
        self.compiled.insert(key, value.clone());
        Ok(value)
    }
}
struct Loader<'a, 'b> {
    raw: BTreeMap<String, Box<RawValue>>,
    values: BTreeMap<String, Arc<Definition>>,
    active: BTreeSet<String>,
    conditions: &'a MaterialConditionRegistry,
    blocks: &'a BlockStateJsonCodec<'b>,
    roots: Vec<String>,
    nodes: usize,
}
impl Loader<'_, '_> {
    fn resolve(&mut self, name: &str, depth: usize) -> Result<Arc<Definition>, DecodeError> {
        if depth >= 256 {
            return Err(DecodeError::Field("material rule depth"));
        }
        if let Some(value) = self.values.get(name) {
            return Ok(value.clone());
        }
        if !self.active.insert(name.to_owned()) {
            return Err(DecodeError::Field("cyclic material rule"));
        }
        let raw = self
            .raw
            .get(name)
            .ok_or(DecodeError::Field("missing material rule"))?
            .clone();
        if !raw.get().starts_with('{') {
            return Err(DecodeError::Field(
                "material rule definition must be direct",
            ));
        }
        let value = self.node(&raw, depth + 1)?;
        self.active.remove(name);
        self.values.insert(name.to_owned(), value.clone());
        Ok(value)
    }
    fn node(&mut self, raw: &RawValue, depth: usize) -> Result<Arc<Definition>, DecodeError> {
        self.nodes += 1;
        if depth >= 256 || self.nodes > 65536 {
            return Err(DecodeError::Field("material rule size"));
        }
        if raw.get().starts_with('"') {
            let name = identifier(&serde_json::from_str::<String>(raw.get())?)?;
            return Ok(Arc::new(Definition::Reference(
                self.resolve(&name, depth + 1)?,
            )));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        let block = |name| {
            self.blocks
                .decode(required(&object, name)?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))
        };
        let value = match kind.as_str() {
            "minecraft:block" => Definition::Block(block("result_state")?),
            "minecraft:bandlands" => Definition::Bandlands,
            "minecraft:sequence" => {
                let children: Vec<Box<RawValue>> =
                    serde_json::from_str(required(&object, "sequence")?.get())?;
                Definition::Sequence(
                    children
                        .iter()
                        .map(|child| self.node(child, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            "minecraft:condition" => Definition::Condition(
                self.conditions
                    .decode(required(&object, "if_true")?.get())?,
                self.node(required(&object, "then_run")?, depth + 1)?,
            ),
            "minecraft:ore_vein" => {
                let blocks = [
                    block("ore_block")?,
                    block("raw_ore_block")?,
                    block("filler_block")?,
                ];
                let chance = number_text(required(&object, "raw_ore_chance")?)?
                    .parse::<f32>()
                    .map_err(|_| DecodeError::Field("raw ore chance"))?;
                if !(0.0..=1.0).contains(&chance) {
                    return Err(DecodeError::Field("raw ore chance"));
                }
                let base = self.roots.len();
                for name in ["density", "richness", "filler_gap"] {
                    self.roots.push(required(&object, name)?.get().to_owned());
                }
                Definition::Ore {
                    blocks,
                    chance,
                    roots: [base, base + 1, base + 2],
                }
            }
            _ => return Err(DecodeError::Field("material rule type")),
        };
        Ok(Arc::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DATA_VERSION,
        nbt::Tag,
        registry::BlockRegistry,
        worldgen::{
            anchor::WorldGenerationContext,
            material_condition::{MaterialConditions, MaterialSurfaceQueries},
        },
    };
    use horizon_core::{
        position::BlockPos,
        random::{LegacyRandom, RandomSource},
    };
    struct Surface;
    impl MaterialSurfaceQueries for Surface {
        fn surface_depth(&mut self, _: i32, _: i32) -> i32 {
            0
        }
        fn surface_secondary(&mut self, _: i32, _: i32) -> f64 {
            0.0
        }
    }
    #[test]
    fn forward_rule_holders_and_invalid_graphs_are_atomic() {
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(vec![Tag::compound([(
                    "id",
                    Tag::String("minecraft:stone".into()),
                )])]),
            ),
            (
                "defaults",
                Tag::compound([("minecraft:stone", Tag::Int(0))]),
            ),
        ]))
        .unwrap();
        let blocks = BlockStateJsonCodec::new(&registry, BTreeSet::new()).unwrap();
        let conditions = MaterialConditionRegistry::default();
        let definitions = BTreeMap::from([
            (
                "horizon:a".to_owned(),
                r#"{"type":"condition","if_true":{"type":"hole"},"then_run":"horizon:z"}"#
                    .to_owned(),
            ),
            (
                "horizon:z".to_owned(),
                r#"{"type":"block","result_state":"stone"}"#.to_owned(),
            ),
        ]);
        let load = |definitions: &BTreeMap<String, String>| {
            MaterialRuleLibrary::from_json(
                definitions,
                &conditions,
                &blocks,
                DensityRegistry::from_json(&BTreeMap::new(), &BTreeMap::new()).unwrap(),
                0,
                false,
            )
        };
        let mut valid = load(&definitions).unwrap();
        for bad in [
            r#"{"type":"condition","if_true":{"type":"hole"},"then_run":"horizon:a"}"#,
            r#"{"type":"condition","if_true":{"type":"hole"},"then_run":"horizon:missing"}"#,
            r#""horizon:z""#,
            r#"{"type":"ore_vein","ore_block":"stone","raw_ore_block":"stone","filler_block":"stone","raw_ore_chance":0.1,"density":"horizon:missing","richness":0,"filler_gap":0}"#,
        ] {
            let mut invalid = definitions.clone();
            invalid.insert("horizon:z".to_owned(), bad.to_owned());
            assert!(load(&invalid).is_err());
        }
        let mut duplicate = definitions;
        duplicate.insert("z".to_owned(), r#"{"type":"bandlands"}"#.to_owned());
        duplicate.insert(
            "minecraft:z".to_owned(),
            r#"{"type":"bandlands"}"#.to_owned(),
        );
        assert!(load(&duplicate).is_err());
        assert_eq!(
            valid.names().collect::<Vec<_>>(),
            vec!["horizon:a", "horizon:z"]
        );
        let mut random = LegacyRandom::new(0);
        let mut worker = MaterialConditions::<LegacyRandom, _>::new(
            WorldGenerationContext::new(-64, 384, 63),
            random.fork_positional(),
            Surface,
        );
        let index = worker.add_rule(valid.rule("horizon:a").unwrap()).unwrap();
        assert_eq!(
            worker.try_apply(index, BlockPos::new(0, 64, 0)).unwrap(),
            Some(BlockStateId(0))
        );
    }
}
