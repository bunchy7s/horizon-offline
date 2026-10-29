//! Bind prepared registry states to model choices; bake each transformed model
//! once and share its geometry across positions. Resource failure recovery and
//! resource-pack reloads remain separate work.
use crate::{Result, sprites::Sprites};
use horizon_assets::{
    bake::{BlockModelTransform, ModelQuad, bake_block_model},
    blockstate::{
        BlockStateDefinition, Variant, VariantSet,
        bind::{Properties, PropertySchema, PropertyValues, SelectedModels},
        choice::CompiledChoices,
    },
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, position_seed},
};
use horizon_world::{
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub struct BakedFace {
    pub quad: ModelQuad,
    pub layer: u32,
    /// The texels under the quad's UVs are all opaque (`SpriteContents.computeTransparency`):
    /// drawn in the solid layer without an alpha test.
    pub opaque: bool,
    /// Fractional alpha: drawn in the sorted translucent pass.
    pub translucent: bool,
}
pub struct BakedModel {
    pub ambient_occlusion: bool,
    /// The model's particle slot also exists for models without block faces.
    pub particle: Option<u32>,
    pub faces: Vec<BakedFace>,
    pub cube: Option<[super::cube::CubeStyle; 6]>,
}
type VariantKey = (String, [u8; 3], bool);
pub struct StateModels {
    /// Each definition's choices, a variant standing as its index in `baked`.
    choices: Vec<CompiledChoices<u32>>,
    /// By state id: its definition in `choices` and the models it selects.
    states: Vec<Option<(usize, SelectedModels)>>,
    /// Every transformed model once, with the variant it was baked from.
    baked: Vec<(Variant, BakedModel)>,
}
fn key(variant: &Variant) -> VariantKey {
    (variant.model.clone(), variant.rotations, variant.uv_lock)
}
impl StateModels {
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
    ) -> Result<Self> {
        let facts: BTreeMap<String, Value> =
            serde_json::from_slice(&fs::read(root.join("model-states.json"))?)?;
        Self::from_facts(root, registry, blocks, sprites, &facts)
    }
    /// Bind and bake models using an already parsed model-state document.
    pub fn from_facts(
        root: &Path,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
        facts: &BTreeMap<String, Value>,
    ) -> Result<Self> {
        Self::from_facts_with_models(
            root,
            registry,
            blocks,
            sprites,
            facts,
            &crate::model_library::ParsedModels::default(),
        )
    }
    pub(crate) fn from_facts_with_models(
        root: &Path,
        registry: &BlockRegistry,
        blocks: &[&str],
        sprites: &mut Sprites,
        facts: &BTreeMap<String, Value>,
        models: &crate::model_library::ParsedModels,
    ) -> Result<Self> {
        let mut choices = Vec::new();
        let mut states = vec![None; registry.len()];
        let mut variants = BTreeSet::new();
        let mut by_name: std::collections::HashMap<&str, Vec<BlockStateId>> =
            std::collections::HashMap::new();
        for i in 0..registry.len() {
            let id = BlockStateId(i as u32);
            if let Some(state) = registry.state(id) {
                by_name.entry(state.name.as_str()).or_default().push(id);
            }
        }
        for block in blocks.iter().copied().collect::<BTreeSet<_>>() {
            // Vanilla blocks come bare; content mods' blocks with their namespace.
            let (namespace, path) = block.split_once(':').unwrap_or(("minecraft", block));
            let name = format!("{namespace}:{path}");
            let facts = facts.get(&name).ok_or("missing block model state schema")?;
            let schema: PropertySchema = facts["schema"]
                .as_object()
                .ok_or("invalid model schema")?
                .iter()
                .map(|(key, value)| {
                    let values = if let Some(range) = value.get("integer") {
                        PropertyValues::Integer {
                            minimum: i32::try_from(
                                range[0]
                                    .as_i64()
                                    .ok_or("invalid integer property minimum")?,
                            )?,
                            maximum: i32::try_from(
                                range[1]
                                    .as_i64()
                                    .ok_or("invalid integer property maximum")?,
                            )?,
                        }
                    } else {
                        PropertyValues::Named(
                            value["named"]
                                .as_array()
                                .ok_or("missing named property values")?
                                .iter()
                                .map(|v| {
                                    v.as_str()
                                        .map(str::to_owned)
                                        .ok_or("invalid named property value")
                                })
                                .collect::<std::result::Result<_, _>>()?,
                        )
                    };
                    Ok((key.clone(), values))
                })
                .collect::<Result<_>>()?;
            let possible: Vec<Properties> = serde_json::from_value(facts["states"].clone())?;
            // Vanilla's BlockStateModelLoader: a definition that does not
            // read or bind is logged and its block gets no model; the other
            // blocks still load.
            let loaded = fs::read_to_string(
                root.join(format!("assets/{namespace}/blockstates/{path}.json")),
            )
            .map_err(|e| e.to_string())
            .and_then(|text| BlockStateDefinition::from_json(&text).map_err(|e| e.to_string()))
            .and_then(|definition| {
                let bound = definition
                    .bind(&schema, &possible)
                    .map_err(|e| e.to_string())?;
                Ok((definition, bound))
            });
            let (definition, bound) = match loaded {
                Ok(loaded) => loaded,
                Err(e) => {
                    log::warn!(
                        "Horizon: blockstate definition of {namespace}:{path} not loaded: {e}"
                    );
                    continue;
                }
            };
            let native_order: Vec<_> = by_name
                .get(name.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|&id| (id, &registry.state(id).expect("indexed state").properties))
                .collect();
            if native_order.len() != possible.len()
                || native_order
                    .iter()
                    .zip(&possible)
                    .any(|((_, a), b)| *a != b)
            {
                return Err(
                    "model state definitions disagree with the imported block registry".into(),
                );
            }
            for ((id, _), selected) in native_order.into_iter().zip(bound) {
                if let Some(selected) = selected {
                    states[id.0 as usize] = Some((choices.len(), selected));
                }
            }
            for choice in definition
                .variants
                .iter()
                .flatten()
                .map(|(_, choice)| choice)
                .chain(
                    definition
                        .multipart
                        .iter()
                        .flatten()
                        .map(|selector| &selector.variant),
                )
            {
                match choice {
                    VariantSet::Single(v) => {
                        variants.insert(key(v));
                    }
                    VariantSet::Weighted(v) => variants.extend(v.iter().map(|(v, _)| key(v))),
                }
            }
            choices.push(CompiledChoices::new(&definition)?);
        }
        let names: Vec<String> = variants.iter().map(|(name, _, _)| name.clone()).collect();
        let library = match models.block_library(root, names.clone()) {
            Ok(library) => library,
            Err(e) => {
                log::warn!("Horizon: block models incomplete ({e}); using the missing model");
                models.block_library_or_missing(root, names)?
            }
        };
        let mut baked = Vec::with_capacity(variants.len());
        let mut index = BTreeMap::new();
        for (name, turns, lock) in variants {
            let model = library.resolve(&name)?;
            let particle = model
                .material("particle")?
                .map(|material| sprites.insert(root, &material.sprite))
                .transpose()?;
            let faces: Vec<BakedFace> =
                bake_block_model(&model, BlockModelTransform::new(turns, lock))?
                    .into_iter()
                    .map(|quad| {
                        let layer = sprites.insert(root, &quad.material.sprite)?;
                        Ok(BakedFace {
                            translucent: quad.material.force_translucent
                                || sprites.translucent[layer as usize],
                            opaque: sprites.region_opaque(layer, &quad.uv),
                            quad,
                            layer,
                        })
                    })
                    .collect::<Result<_>>()?;
            let cube = super::cube::cube_styles(&faces);
            index.insert((name.clone(), turns, lock), baked.len() as u32);
            let variant = Variant {
                model: name,
                rotations: turns,
                uv_lock: lock,
            };
            let model = BakedModel {
                ambient_occlusion: model.ambient_occlusion,
                particle,
                faces,
                cube,
            };
            baked.push((variant, model));
        }
        // Every variant of every definition was baked above.
        let choices = choices.iter().map(|c| c.map(|v| index[&key(v)])).collect();
        Ok(Self {
            choices,
            states,
            baked,
        })
    }
    pub fn collect(
        &self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<Vec<(&Variant, &BakedModel)>> {
        // Blocks use their position seed; contained/displayed blocks can supply one.
        self.collect_seeded(state, position_seed(position))
    }
    /// `BlockStateModel.particleMaterial`: the particle sprite layer of the
    /// state's first variant (see `CompiledChoices::particle_variant`).
    pub fn particle(&self, state: BlockStateId) -> Option<u32> {
        let (choice, selected) = self.state(state)?;
        let model = self.choices[*choice].particle_variant(selected)?;
        self.baked[*model as usize].1.particle
    }
    pub fn collect_seeded(
        &self,
        state: BlockStateId,
        seed: i64,
    ) -> Result<Vec<(&Variant, &BakedModel)>> {
        let mut models = Vec::new();
        self.each_seeded(state, seed, |(variant, model)| {
            models.push((variant, model))
        })?;
        Ok(models)
    }
    /// The models a block draws at `position`, into `out` (cleared first):
    /// `collect` for the mesher, which reuses one list for every block.
    pub fn models_at<'a>(
        &'a self,
        state: BlockStateId,
        position: BlockPos,
        out: &mut Vec<&'a BakedModel>,
    ) -> Result<()> {
        out.clear();
        self.each_seeded(state, position_seed(position), |(_, model)| out.push(model))
    }
    fn each_seeded<'a>(
        &'a self,
        state: BlockStateId,
        seed: i64,
        mut f: impl FnMut(&'a (Variant, BakedModel)),
    ) -> Result<()> {
        let (choice, selected) = self.state(state).ok_or("unbound gallery block state")?;
        let mut random = LegacyRandom::new(seed);
        self.choices[*choice].for_each(selected, &mut random, |&model| {
            f(&self.baked[model as usize]);
        })?;
        Ok(())
    }
    fn state(&self, state: BlockStateId) -> Option<&(usize, SelectedModels)> {
        self.states.get(state.0 as usize)?.as_ref()
    }
}

pub fn descriptor(name: &str, overrides: &[(&str, &str)]) -> BlockState {
    BlockState {
        name: format!("minecraft:{name}"),
        properties: overrides
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}
