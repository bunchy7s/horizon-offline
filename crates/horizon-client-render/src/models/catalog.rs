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
    /// The Fusion connecting texture rule its sprite joins by ([`crate::fusion`]), plus one:
    /// two bytes, within the padding every face already has.
    pub fusion: Option<std::num::NonZeroU16>,
}
pub struct BakedModel {
    pub ambient_occlusion: bool,
    /// The model's particle slot also exists for models without block faces.
    pub particle: Option<u32>,
    pub faces: Vec<BakedFace>,
    pub cube: Option<[super::cube::CubeStyle; 6]>,
}
type VariantKey = (String, [u8; 3], bool);
/// Empty by default: no state has a model (a dedicated server, which draws
/// nothing).
#[derive(Default)]
pub struct StateModels {
    /// Each definition's choices, a variant standing as its index in `baked`.
    choices: Vec<CompiledChoices<u32>>,
    /// By state id: its definition in `choices` and the models it selects.
    states: Vec<Option<(usize, SelectedModels)>>,
    /// Every transformed model once, with the variant it was baked from.
    baked: Vec<(Variant, BakedModel)>,
    /// Standalone models by id: their index in `baked`.
    standalone: BTreeMap<String, u32>,
}
fn key(variant: &Variant) -> VariantKey {
    (variant.model.clone(), variant.rotations, variant.uv_lock)
}
/// Bakes model `name` of `library`, turned and UV-locked as a variant says.
fn bake_variant(
    root: &Path,
    sprites: &mut Sprites,
    library: &horizon_assets::resolve::ModelLibrary,
    name: &str,
    turns: [u8; 3],
    lock: bool,
) -> Result<BakedModel> {
    bake_resolved(root, sprites, &library.resolve(name)?, turns, lock)
}
/// Bakes a resolved `model`, turned and UV-locked as a variant says.
fn bake_resolved(
    root: &Path,
    sprites: &mut Sprites,
    model: &horizon_assets::resolve::ResolvedCuboidModel,
    turns: [u8; 3],
    lock: bool,
) -> Result<BakedModel> {
    let particle = model
        .material("particle")?
        .map(|material| sprites.insert(root, &material.sprite))
        .transpose()?;
    let faces: Vec<BakedFace> = bake_block_model(model, BlockModelTransform::new(turns, lock))?
        .into_iter()
        .map(|quad| {
            let layer = sprites.insert(root, &quad.material.sprite)?;
            Ok(BakedFace {
                translucent: quad.material.force_translucent || sprites.translucent[layer as usize],
                opaque: sprites.region_opaque(layer, &quad.uv),
                quad,
                layer,
                fusion: None,
            })
        })
        .collect::<Result<_>>()?;
    let cube = super::cube::cube_styles(&faces);
    Ok(BakedModel {
        ambient_occlusion: model.ambient_occlusion,
        particle,
        faces,
        cube,
    })
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
        let resolved: Vec<_> = variants
            .iter()
            .map(|(name, ..)| library.resolve(name))
            .collect();
        // Every texture the models name, decoded side by side before the bake adds them one
        // by one in its own order.
        sprites.prefetch(
            root,
            resolved
                .iter()
                .flatten()
                .flat_map(|model| model.textures.values())
                .map(|material| material.sprite.as_str()),
        );
        let mut baked = Vec::with_capacity(variants.len());
        let mut index = BTreeMap::new();
        for ((name, turns, lock), model) in variants.into_iter().zip(resolved) {
            let model = bake_resolved(root, sprites, &model?, turns, lock)?;
            index.insert((name.clone(), turns, lock), baked.len() as u32);
            let variant = Variant {
                model: name,
                rotations: turns,
                uv_lock: lock,
            };
            baked.push((variant, model));
        }
        // Every variant of every definition was baked above.
        let choices = choices.iter().map(|c| c.map(|v| index[&key(v)])).collect();
        Ok(Self {
            choices,
            states,
            baked,
            standalone: BTreeMap::new(),
        })
    }
    /// Bakes `names`, models no blockstate needs to name (vanilla's
    /// standalone models, NeoForge's `ModelEvent.RegisterStandalone`), as
    /// they are, unturned, for [`Self::standalone`]. One a block state
    /// already bakes unturned is shared. A model that doesn't read (missing,
    /// or a parent missing) is logged and left out, as a missing blockstate
    /// is; the others still bake.
    pub(crate) fn bake_standalone(
        &mut self,
        root: &Path,
        sprites: &mut Sprites,
        models: &crate::model_library::ParsedModels,
        names: &[String],
    ) -> Result<()> {
        for name in names {
            if self.standalone.contains_key(name) {
                continue;
            }
            if let Some(i) = self
                .baked
                .iter()
                .position(|(v, _)| v.model == *name && v.rotations == [0; 3] && !v.uv_lock)
            {
                self.standalone.insert(name.clone(), i as u32);
                continue;
            }
            let model = models
                .block_library(root, vec![name.clone()])
                .and_then(|library| bake_variant(root, sprites, &library, name, [0; 3], false));
            match model {
                Ok(model) => {
                    self.standalone
                        .insert(name.clone(), self.baked.len() as u32);
                    let variant = Variant {
                        model: name.clone(),
                        rotations: [0; 3],
                        uv_lock: false,
                    };
                    self.baked.push((variant, model));
                }
                Err(e) => log::warn!("Horizon: standalone model {name} not loaded: {e}"),
            }
        }
        Ok(())
    }
    /// Every baked model with the variant it was baked from, to change its faces.
    pub fn models_mut(&mut self) -> impl Iterator<Item = (&Variant, &mut BakedModel)> {
        self.baked
            .iter_mut()
            .map(|(variant, model)| (&*variant, model))
    }
    /// A model baked by [`Self::bake_standalone`], by id (`mymod:block/shaft_half`).
    pub fn standalone(&self, name: &str) -> Option<&BakedModel> {
        let &i = self.standalone.get(name)?;
        Some(&self.baked[i as usize].1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_library::ParsedModels;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn write(&self, name: &str, json: &str) {
            let (namespace, path) = name.split_once(':').unwrap();
            let path = self
                .0
                .join(format!("assets/{namespace}/models/{path}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn standalone_models_bake_once_and_skip_what_does_not_read() -> Result<()> {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("horizon-standalone-{}", std::process::id())),
        );
        // A half shaft: one element, two faces; a child with its parent's elements.
        fixture.write(
            "lanef:block/shaft_half",
            r##"{"textures":{"side":"lanef:block/shaft"},"elements":[{"from":[6,6,0],"to":[10,10,8],"faces":{"north":{"texture":"#side"},"up":{"texture":"#side"}}}]}"##,
        );
        fixture.write(
            "lanef:block/cog",
            r#"{"parent":"lanef:block/shaft_half","textures":{"side":"lanef:block/cog"}}"#,
        );
        fixture.write("lanef:block/orphan", r#"{"parent":"lanef:block/absent"}"#);
        let mut models = StateModels::default();
        let mut sprites = Sprites::default();
        let names: Vec<String> = [
            "lanef:block/shaft_half",
            "lanef:block/cog",
            "lanef:block/orphan",
            "lanef:block/absent",
            "lanef:block/shaft_half",
        ]
        .map(str::to_owned)
        .into();
        let parsed = ParsedModels::load(&fixture.0);
        models.bake_standalone(&fixture.0, &mut sprites, &parsed, &names)?;
        let shaft = models.standalone("lanef:block/shaft_half").expect("baked");
        assert_eq!(shaft.faces.len(), 2);
        let cog = models.standalone("lanef:block/cog").expect("baked");
        assert_eq!(cog.faces.len(), 2);
        assert_ne!(cog.faces[0].layer, shaft.faces[0].layer, "its own texture");
        assert!(models.standalone("lanef:block/orphan").is_none());
        assert!(models.standalone("lanef:block/absent").is_none());
        assert!(models.standalone("lanef:block/unnamed").is_none());
        // Named twice, baked once.
        assert_eq!(models.baked.len(), 2);
        // Without a parsed library, the files are read directly.
        let mut direct = StateModels::default();
        direct.bake_standalone(&fixture.0, &mut sprites, &ParsedModels::default(), &names)?;
        assert_eq!(
            direct.standalone("lanef:block/cog").map(|m| m.faces.len()),
            Some(2)
        );
        Ok(())
    }

    #[test]
    fn a_standalone_model_a_state_bakes_unturned_is_shared() -> Result<()> {
        let mut models = StateModels::default();
        let variant = |rotations| Variant {
            model: "lanef:block/shared".into(),
            rotations,
            uv_lock: false,
        };
        let empty = || BakedModel {
            ambient_occlusion: true,
            particle: None,
            faces: Vec::new(),
            cube: None,
        };
        models.baked.push((variant([0, 90, 0]), empty()));
        models.baked.push((variant([0; 3]), empty()));
        let names = vec!["lanef:block/shared".to_owned()];
        let root = std::env::temp_dir().join("horizon-standalone-none");
        models.bake_standalone(
            &root,
            &mut Sprites::default(),
            &ParsedModels::default(),
            &names,
        )?;
        assert_eq!(models.baked.len(), 2);
        assert_eq!(models.standalone.get("lanef:block/shared"), Some(&1));
        Ok(())
    }
}
