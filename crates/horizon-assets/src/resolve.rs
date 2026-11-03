//! Cuboid parent inheritance and texture-slot resolution.
use crate::{json::*, model::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MISSING_MODEL: &str = "minecraft:builtin/missing";
/// `ItemModelGenerator.GENERATED_ITEM_MODEL_ID`.
pub const GENERATED_MODEL: &str = "minecraft:builtin/generated";
/// Composite children nest through parents that may name the composite again.
const MAX_LOADER_DEPTH: u8 = 8;

/// Decoded models are immutable and their inherited geometry is shared.
pub struct ModelLibrary {
    models: BTreeMap<String, Arc<CuboidModel>>,
    missing: Arc<CuboidModel>,
}
/// What a model's loader adds to its own elements.
#[derive(Clone, Debug)]
pub enum ResolvedLoader {
    /// The shown children, resolved on their own, in order.
    Composite(Vec<ResolvedCuboidModel>),
}
#[derive(Clone, Debug)]
pub struct ResolvedCuboidModel {
    pub ambient_occlusion: bool,
    pub gui_light: GuiLight,
    pub display: ItemTransforms,
    pub textures: BTreeMap<String, Material>,
    /// Cyclic and dangling aliases are absent from `textures`.
    pub unresolved_textures: BTreeSet<String>,
    /// NeoForge's `render_type`; the leaf-most model that sets one wins.
    pub render_type: Option<String>,
    geometry: Option<Arc<CuboidModel>>,
    generated: bool,
    loader: Option<ResolvedLoader>,
}
impl ResolvedCuboidModel {
    /// Geometry from a custom loader, baked after the elements.
    pub fn loader(&self) -> Option<&ResolvedLoader> {
        self.loader.as_ref()
    }
    /// Whether the top geometry is `builtin/generated`'s layer extrusion
    /// (no model closer to the leaf has elements of its own).
    pub fn uses_item_generator(&self) -> bool {
        self.generated
    }
    pub fn elements(&self) -> &[Option<Element>] {
        self.geometry
            .as_ref()
            .and_then(|m| m.elements.as_deref())
            .unwrap_or(&[])
    }
    /// Native slot lookup strips a single leading '#'. Non-reference names
    /// are still slot names, not direct sprite identifiers.
    pub fn material(&self, slot: &str) -> Result<Option<&Material>> {
        if slot.is_empty() {
            return Err(error("empty texture lookup"));
        }
        Ok(self.textures.get(slot.strip_prefix('#').unwrap_or(slot)))
    }
}
impl ModelLibrary {
    /// A library that also knows `builtin/generated`, the parent of every
    /// flat item model; the native client registers it as a special model.
    pub fn with_item_generator(mut models: BTreeMap<String, CuboidModel>) -> Result<Self> {
        models.insert(GENERATED_MODEL.to_owned(), Self::item_generator_model());
        Self::new(models)
    }
    /// Register the item built-in while reusing already decoded resources.
    pub fn with_shared_item_generator(
        mut models: BTreeMap<String, Arc<CuboidModel>>,
    ) -> Result<Self> {
        models.insert(
            GENERATED_MODEL.to_owned(),
            Arc::new(Self::item_generator_model()),
        );
        Self::from_shared(models)
    }
    fn item_generator_model() -> CuboidModel {
        CuboidModel {
            parent: None,
            loader: None,
            ambient_occlusion: None,
            gui_light: Some(GuiLight::Front),
            textures: BTreeMap::from([(
                "particle".to_owned(),
                TextureSlot::Reference("layer0".to_owned()),
            )]),
            elements: None,
            display: None,
            render_type: None,
            visibility: BTreeMap::new(),
        }
    }
    pub fn new(models: BTreeMap<String, CuboidModel>) -> Result<Self> {
        Self::from_models(
            models
                .into_iter()
                .map(|(name, model)| (name, Arc::new(model))),
        )
    }
    /// Build an immutable resolution view without copying model geometry.
    pub fn from_shared(models: BTreeMap<String, Arc<CuboidModel>>) -> Result<Self> {
        Self::from_models(models)
    }
    fn from_models(models: impl IntoIterator<Item = (String, Arc<CuboidModel>)>) -> Result<Self> {
        let mut decoded = BTreeMap::new();
        for (name, model) in models {
            if decoded.insert(identifier(&name)?, model).is_some() {
                return Err(error("duplicate normalized model identifier"));
            }
        }
        let missing = Arc::new(missing_model());
        // The native discovery registers this built-in before consulting resources.
        decoded.insert(MISSING_MODEL.to_owned(), missing.clone());
        Ok(Self {
            models: decoded,
            missing,
        })
    }
    /// This library plus `extra` models; an `extra` model replaces a
    /// same-named one, but not the built-in missing and generated models.
    /// Every existing model is shared, not copied.
    pub fn overlaid(&self, extra: BTreeMap<String, CuboidModel>) -> Result<Self> {
        let mut models = self.models.clone();
        let mut added = BTreeSet::new();
        for (name, model) in extra {
            let name = identifier(&name)?;
            if !added.insert(name.clone()) {
                return Err(error("duplicate normalized model identifier"));
            }
            // The built-ins win, as `from_models` keeps `missing` and
            // `with_item_generator` keeps `generated`.
            if name != MISSING_MODEL && name != GENERATED_MODEL {
                models.insert(name, Arc::new(model));
            }
        }
        Ok(Self {
            models,
            missing: Arc::clone(&self.missing),
        })
    }
    /// Follow the complete parent chain. A missing resource uses the built-in
    /// model; a cyclic chain is rejected, as native discovery excludes it.
    /// The returned model can outlive this library without copying its elements.
    pub fn resolve(&self, name: &str) -> Result<ResolvedCuboidModel> {
        self.resolve_name(name, None)
    }
    /// Whether `name` and every parent it reaches are models of this library,
    /// with no cycle: `resolve` puts the missing model in for any other.
    pub fn resolves(&self, name: &str) -> bool {
        let Ok(mut name) = identifier(name) else {
            return false;
        };
        let mut visited = BTreeSet::new();
        loop {
            let Some(model) = self.models.get(&name) else {
                return false;
            };
            if !visited.insert(name.clone()) {
                return false;
            }
            match &model.parent {
                Some(parent) => name = parent.clone(),
                None => return true,
            }
        }
    }
    /// `name`'s resolved model with a virtual child in front of it that has
    /// only `textures`: the child's slots win, and `#references` anywhere in
    /// the chain re-bind through them (so `builtin/generated`'s
    /// `particle: #layer0` follows a replaced `layer0`). The child's slots are
    /// taken as given: sprites namespaced, references without their `#`, as
    /// the JSON decoder leaves them. A cyclic chain is an
    /// error, as for `resolve`: a caller wanting the missing model falls
    /// back to it.
    pub fn resolve_with_textures(
        &self,
        name: &str,
        child: &BTreeMap<String, TextureSlot>,
    ) -> Result<ResolvedCuboidModel> {
        self.resolve_name(name, Some(child))
    }
    fn resolve_name(
        &self,
        name: &str,
        child: Option<&BTreeMap<String, TextureSlot>>,
    ) -> Result<ResolvedCuboidModel> {
        let next = Some(identifier(name)?);
        self.resolve_chain(Vec::new(), Vec::new(), next, &BTreeMap::new(), child, 0)
    }
    /// `chain` and `names` start with models that are not in the library (a
    /// composite child); `next` is the parent to follow. `fallback` slots lose
    /// to any slot the chain defines.
    fn resolve_chain(
        &self,
        mut chain: Vec<Arc<CuboidModel>>,
        mut names: Vec<String>,
        mut next: Option<String>,
        fallback: &BTreeMap<String, TextureSlot>,
        child: Option<&BTreeMap<String, TextureSlot>>,
        depth: u8,
    ) -> Result<ResolvedCuboidModel> {
        if depth > MAX_LOADER_DEPTH {
            return Err(error("composite models nest too deeply"));
        }
        let mut visited = BTreeSet::new();
        while let Some(name) = next.take() {
            if !visited.insert(name.clone()) {
                return Err(error("cyclic model parent"));
            }
            let model = self.models.get(&name).unwrap_or(&self.missing);
            chain.push(Arc::clone(model));
            names.push(name);
            next = model.parent.clone();
        }
        let ambient_occlusion = chain
            .iter()
            .find_map(|m| m.ambient_occlusion)
            .unwrap_or(true);
        let gui_light = chain
            .iter()
            .find_map(|m| m.gui_light)
            .unwrap_or(GuiLight::Side);
        let render_type = chain.iter().find_map(|m| m.render_type.clone());
        let geometry = chain.iter().find(|m| m.elements.is_some()).map(Arc::clone);
        let generated = chain
            .iter()
            .zip(&names)
            .find(|(m, name)| m.elements.is_some() || *name == GENERATED_MODEL)
            .is_some_and(|(_, name)| name == GENERATED_MODEL);
        let display = ItemTransforms {
            contexts: std::array::from_fn(|index| {
                chain
                    .iter()
                    .filter_map(|m| m.display.as_ref())
                    .map(|d| &d.contexts[index])
                    .find(|t| **t != DisplayTransform::Inherit)
                    .cloned()
                    .unwrap_or(DisplayTransform::Inherit)
            }),
        };
        // Resolve aliases only after applying every child override. A reference
        // inherited from a parent binds to the child's final slot definition.
        let mut slots: BTreeMap<String, &TextureSlot> =
            fallback.iter().map(|(k, v)| (k.clone(), v)).collect();
        for model in chain.iter().rev() {
            slots.extend(model.textures.iter().map(|(k, v)| (k.clone(), v)));
        }
        if let Some(child) = child {
            slots.extend(child.iter().map(|(k, v)| (k.clone(), v)));
        }
        let loader = match chain.iter().find_map(|m| m.loader.as_ref()) {
            Some(Loader::Composite {
                children,
                visibility,
            }) => {
                let inherited: BTreeMap<String, TextureSlot> = slots
                    .iter()
                    .map(|(k, v)| (k.clone(), (*v).clone()))
                    .collect();
                // A model closer to the leaf overrides the loader's own map.
                let mut visibility = visibility.clone();
                for model in chain.iter().rev() {
                    visibility.extend(model.visibility.iter().map(|(k, v)| (k.clone(), *v)));
                }
                let mut resolved = Vec::new();
                for (child_name, child_model) in children {
                    if visibility.get(child_name) == Some(&false) {
                        continue;
                    }
                    resolved.push(self.resolve_chain(
                        vec![Arc::clone(child_model)],
                        vec![format!("{}#{child_name}", names[0])],
                        child_model.parent.clone(),
                        &inherited,
                        None,
                        depth + 1,
                    )?);
                }
                Some(ResolvedLoader::Composite(resolved))
            }
            Some(Loader::Unknown(_)) | None => None,
        };
        let mut textures = BTreeMap::<String, Material>::new();
        let mut unresolved_textures = BTreeSet::<String>::new();
        for start in slots.keys() {
            let mut current = start.as_str();
            let mut seen = BTreeSet::new();
            let mut path = Vec::new();
            let material = loop {
                if let Some(material) = textures.get(current) {
                    break Some(material.clone());
                }
                if unresolved_textures.contains(current) || !seen.insert(current) {
                    break None;
                }
                let Some(slot) = slots.get(current) else {
                    break None;
                };
                path.push(current);
                match slot {
                    TextureSlot::Reference(target) => current = target,
                    TextureSlot::Material(material) => break Some(material.clone()),
                }
            };
            // Cache the entire walk, including failures, so long alias chains
            // and cycles are traversed once rather than once per referencing slot.
            match material {
                Some(material) => {
                    for name in path {
                        textures.insert(name.to_owned(), material.clone());
                    }
                }
                None => {
                    unresolved_textures.extend(path.into_iter().map(str::to_owned));
                }
            }
        }
        Ok(ResolvedCuboidModel {
            ambient_occlusion,
            gui_light,
            display,
            textures,
            unresolved_textures,
            render_type,
            geometry,
            generated,
            loader,
        })
    }
}
/// The native missing-model cube uses slot names without a leading '#'.
pub fn missing_model() -> CuboidModel {
    let faces = Direction::ALL
        .into_iter()
        .map(|direction| {
            (
                direction,
                Some(Face {
                    cull: Some(direction),
                    tint_index: -1,
                    texture: "missingno".to_owned(),
                    uv: Some([0.0, 0.0, 16.0, 16.0]),
                    rotation: 0,
                    light_emission: None,
                    neoforge_data: None,
                }),
            )
        })
        .collect();
    CuboidModel {
        parent: None,
        loader: None,
        ambient_occlusion: None,
        gui_light: None,
        elements: Some(vec![Some(Element {
            from: [0.0; 3],
            to: [16.0; 3],
            faces,
            rotation: None,
            shade_direction_override: None,
            light_emission: 0,
            neoforge_data: None,
        })]),
        display: Some(ItemTransforms {
            contexts: std::array::from_fn(|_| DisplayTransform::Inherit),
        }),
        render_type: None,
        textures: BTreeMap::from([
            (
                "particle".to_owned(),
                TextureSlot::Reference("missingno".to_owned()),
            ),
            (
                "missingno".to_owned(),
                TextureSlot::Material(Material {
                    sprite: "minecraft:missingno".to_owned(),
                    force_translucent: false,
                }),
            ),
        ]),
        visibility: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_alias_uses_child_material_and_geometry_outlives_library() {
        let parent = CuboidModel::from_json(r##"{"textures":{"side":"#all","all":"block/stone"},"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"#side"}}}]}"##).unwrap();
        let child = CuboidModel::from_json(r#"{"parent":"horizon:parent","textures":{"all":{"sprite":"block/glass","force_translucent":true}}}"#).unwrap();
        let library = ModelLibrary::new(BTreeMap::from([
            ("horizon:parent".into(), parent),
            ("horizon:child".into(), child),
        ]))
        .unwrap();
        let model = library.resolve("horizon:child").unwrap();
        drop(library);
        assert_eq!(model.elements()[0].as_ref().unwrap().to, [16.0, 8.0, 16.0]);
        assert_eq!(
            model.material("#side").unwrap().unwrap(),
            &Material {
                sprite: "minecraft:block/glass".into(),
                force_translucent: true
            }
        );
    }
    fn library(models: &[(&str, &str)]) -> ModelLibrary {
        ModelLibrary::with_item_generator(
            models
                .iter()
                .map(|(name, json)| (name.to_string(), CuboidModel::from_json(json).unwrap()))
                .collect(),
        )
        .unwrap()
    }
    const GENERATED: &str = r#"{"parent":"builtin/generated","display":{"ground":{"translation":[0,2,0],"scale":[0.5,0.5,0.5]}}}"#;
    const CIT: &str = "horizon_cit:minecraft/optifine/cit/x";
    #[test]
    fn child_textures_replace_slots_and_rebind_the_generated_particle() {
        let library = library(&[
            ("item/generated", GENERATED),
            (
                "item/stick",
                r#"{"parent":"item/generated","textures":{"layer0":"item/stick"}}"#,
            ),
        ]);
        let child = BTreeMap::from([(
            "layer0".to_owned(),
            TextureSlot::Material(Material {
                sprite: CIT.to_owned(),
                force_translucent: false,
            }),
        )]);
        let model = library.resolve_with_textures("item/stick", &child).unwrap();
        assert!(model.uses_item_generator());
        assert_eq!(model.material("layer0").unwrap().unwrap().sprite, CIT);
        assert_eq!(model.material("particle").unwrap().unwrap().sprite, CIT);
        assert_eq!(
            model.display.contexts[6],
            DisplayTransform::Explicit(Some(ItemTransform {
                rotation: [0.0; 3],
                translation: [0.0, 0.125, 0.0],
                scale: [0.5; 3],
            }))
        );
        // The library itself is untouched.
        let plain = library.resolve("item/stick").unwrap();
        assert_eq!(
            plain.material("particle").unwrap().unwrap().sprite,
            "minecraft:item/stick"
        );
    }
    #[test]
    fn overlaid_models_inherit_from_the_library_and_share_the_rest() {
        let base = library(&[
            ("item/generated", GENERATED),
            (
                "item/handheld",
                r#"{"parent":"item/generated","display":{"thirdperson_righthand":{"rotation":[0,-90,55]}}}"#,
            ),
            (
                "item/stick",
                r#"{"parent":"item/generated","textures":{"layer0":"item/stick"}}"#,
            ),
            (
                "item/apple",
                r#"{"parent":"item/generated","textures":{"layer0":"item/apple"}}"#,
            ),
        ]);
        let cit = format!(r#"{{"parent":"item/handheld","textures":{{"layer0":"{CIT}"}}}}"#);
        let over = base
            .overlaid(BTreeMap::from([
                (CIT.to_owned(), CuboidModel::from_json(&cit).unwrap()),
                (
                    "item/stick".to_owned(),
                    CuboidModel::from_json(
                        r#"{"parent":"item/generated","textures":{"layer0":"item/bone"}}"#,
                    )
                    .unwrap(),
                ),
            ]))
            .unwrap();
        let model = over.resolve(CIT).unwrap();
        assert!(model.uses_item_generator());
        assert_eq!(model.material("particle").unwrap().unwrap().sprite, CIT);
        let DisplayTransform::Explicit(Some(right)) = &model.display.contexts[1] else {
            panic!("handheld's right-hand transform is inherited");
        };
        assert_eq!(right.rotation, [0.0, -90.0, 55.0]);
        let sprite = |l: &ModelLibrary| {
            l.resolve("item/stick").unwrap().textures["layer0"]
                .sprite
                .clone()
        };
        assert_eq!(sprite(&over), "minecraft:item/bone");
        assert_eq!(sprite(&base), "minecraft:item/stick");
        assert!(!base.resolve(CIT).unwrap().uses_item_generator());
        assert!(Arc::ptr_eq(
            &base.models["minecraft:item/apple"],
            &over.models["minecraft:item/apple"]
        ));
    }
    fn material(sprite: &str) -> TextureSlot {
        TextureSlot::Material(Material {
            sprite: sprite.to_owned(),
            force_translucent: false,
        })
    }
    #[test]
    fn child_slots_take_part_in_reference_chains() {
        let library = library(&[
            (
                "item/two",
                r##"{"textures":{"layer0":"#tex","layer1":"item/b","tex":"item/a"}}"##,
            ),
            ("horizon:loop", r#"{"parent":"horizon:loop2"}"#),
            ("horizon:loop2", r#"{"parent":"horizon:loop"}"#),
        ]);
        let child = BTreeMap::from([
            // A base alias binds through a slot only the child sets...
            ("tex".to_owned(), material(CIT)),
            // ...and a child alias through the base's slots.
            (
                "particle".to_owned(),
                TextureSlot::Reference("layer1".to_owned()),
            ),
            // A child alias to itself resolves to nothing, without looping.
            ("self".to_owned(), TextureSlot::Reference("self".to_owned())),
        ]);
        let model = library.resolve_with_textures("item/two", &child).unwrap();
        assert_eq!(model.material("layer0").unwrap().unwrap().sprite, CIT);
        assert_eq!(
            model.material("particle").unwrap().unwrap().sprite,
            "minecraft:item/b"
        );
        assert_eq!(model.material("self").unwrap(), None);
        assert!(model.unresolved_textures.contains("self"));
        // An absent base is the missing cube under the child; a cyclic one an error.
        let absent = library
            .resolve_with_textures("item/absent", &child)
            .unwrap();
        assert_eq!(absent.elements().len(), 1);
        assert!(
            library
                .resolve_with_textures("horizon:loop", &child)
                .is_err()
        );
    }
    #[test]
    fn an_overlay_replaces_parents_but_not_the_built_ins_and_refuses_twins() {
        let base = library(&[
            ("item/generated", GENERATED),
            ("item/handheld", r#"{"parent":"item/generated"}"#),
            (
                "item/sword",
                r#"{"parent":"item/handheld","textures":{"layer0":"item/sword"}}"#,
            ),
        ]);
        let model = |json: &str| CuboidModel::from_json(json).unwrap();
        let over = base
            .overlaid(BTreeMap::from([
                (
                    "item/handheld".to_owned(),
                    model(
                        r#"{"parent":"item/generated","display":{"head":{"rotation":[0,45,0]}}}"#,
                    ),
                ),
                // Neither built-in can be replaced.
                ("builtin/missing".to_owned(), model(r#"{"elements":[]}"#)),
                ("builtin/generated".to_owned(), model("{}")),
            ]))
            .unwrap();
        // The base's sword sees the overlay's parent; the base library does not.
        let head = |l: &ModelLibrary| l.resolve("item/sword").unwrap().display.contexts[4].clone();
        let DisplayTransform::Explicit(Some(rotated)) = head(&over) else {
            panic!("the replaced parent's head transform");
        };
        assert_eq!(rotated.rotation, [0.0, 45.0, 0.0]);
        assert_ne!(head(&base), head(&over));
        // The generator kept: its `particle: #layer0`, which `{}` would drop.
        let sword = over.resolve("item/sword").unwrap();
        assert_eq!(
            sword.material("particle").unwrap().unwrap().sprite,
            "minecraft:item/sword"
        );
        // The missing cube kept, looked up by its own name, not as a fallback.
        let missing = over.resolve("builtin/missing").unwrap();
        assert_eq!(missing.elements().len(), 1);
        // Two spellings of one id.
        let twins = BTreeMap::from([
            ("item/x".to_owned(), model("{}")),
            ("minecraft:item/x".to_owned(), model("{}")),
        ]);
        assert!(base.overlaid(twins).is_err());
    }
    #[test]
    fn absent_parents_fall_back_but_cycles_invalidate_descendants() {
        let a = CuboidModel::from_json(r#"{"parent":"horizon:b"}"#).unwrap();
        let b = CuboidModel::from_json(r#"{"parent":"horizon:a"}"#).unwrap();
        let child = CuboidModel::from_json(r#"{"parent":"horizon:a","elements":[]}"#).unwrap();
        let library = ModelLibrary::new(BTreeMap::from([
            ("horizon:a".into(), a),
            ("horizon:b".into(), b),
            ("horizon:child".into(), child),
        ]))
        .unwrap();
        assert!(library.resolve("horizon:child").is_err());
        let absent = library.resolve("horizon:absent").unwrap();
        assert_eq!(absent.elements().len(), 1);
        assert_eq!(
            absent.material("particle").unwrap().unwrap().sprite,
            "minecraft:missingno"
        );
    }
    #[test]
    fn a_model_resolves_only_with_its_whole_parent_chain() {
        let library = library(&[
            ("item/generated", GENERATED),
            (
                "item/stick",
                r#"{"parent":"item/generated","textures":{"layer0":"item/stick"}}"#,
            ),
            ("item/orphan", r#"{"parent":"item/gone"}"#),
            ("item/a", r#"{"parent":"item/b"}"#),
            ("item/b", r#"{"parent":"item/a"}"#),
        ]);
        assert!(library.resolves("item/stick"));
        assert!(library.resolves("minecraft:item/stick"));
        assert!(!library.resolves("item/absent"));
        assert!(!library.resolves("item/orphan"));
        assert!(!library.resolves("item/a"));
        assert!(!library.resolves("Item/Stick"));
    }
    const LIGHTS: &str = r##"{"textures":{"a":"block/a"},"render_type":"minecraft:cutout","elements":[
        {"from":[0,0,0],"to":[16,16,16],"light_emission":4,"neoforge_data":{"block_light":9,"color":-65536,"ambient_occlusion":false},
         "faces":{"north":{"texture":"#a","neoforge_data":{"block_light":2,"sky_light":15,"unknown":1}},
                  "south":{"texture":"#a","light_emission":11},
                  "up":{"texture":"#a"}}}]}"##;
    #[test]
    fn neoforge_data_of_a_face_overrides_its_element_key_by_key() {
        use crate::bake::bake_block_model;
        let library = library(&[("block/lights", LIGHTS)]);
        let model = library.resolve("block/lights").unwrap();
        let quads =
            bake_block_model(&model, crate::bake::BlockModelTransform::new([0; 3], false)).unwrap();
        let by = |d| quads.iter().find(|q| q.facing == d).unwrap();
        let north = by(Direction::North);
        assert_eq!(north.light, [4, 15]);
        assert_eq!(north.color, Some(-65536));
        assert_eq!(north.ambient_occlusion, Some(false));
        assert_eq!(by(Direction::South).light, [11, 11]);
        assert_eq!(by(Direction::South).light_emission, 11);
        assert_eq!(by(Direction::Up).light, [9, 4]);
        assert!(
            quads
                .iter()
                .all(|q| q.render_type == Some(crate::bake::RenderType::Cutout))
        );
    }
    #[test]
    fn vanilla_models_bake_without_extra_face_data() {
        use crate::bake::bake_block_model;
        let library = library(&[(
            "block/plain",
            r##"{"textures":{"a":"block/a"},"elements":[{"from":[0,0,0],"to":[16,16,16],"light_emission":6,"faces":{"up":{"texture":"#a"}}}]}"##,
        )]);
        let model = library.resolve("block/plain").unwrap();
        let quad = &bake_block_model(&model, crate::bake::BlockModelTransform::new([0; 3], false))
            .unwrap()[0];
        assert_eq!((quad.light, quad.light_emission), ([6, 6], 6));
        assert_eq!(
            (quad.color, quad.ambient_occlusion, quad.render_type),
            (None, None, None)
        );
    }
    #[test]
    fn render_type_comes_from_the_leaf_most_model_and_loses_its_prefix() {
        let library = library(&[
            ("block/base", r#"{"render_type":"translucent"}"#),
            ("block/mid", r#"{"parent":"block/base"}"#),
            (
                "block/leaf",
                r#"{"parent":"block/mid","render_type":"minecraft:cutout_mipped"}"#,
            ),
            ("block/bare", r#"{"parent":"block/mid"}"#),
        ]);
        let render = |name| library.resolve(name).unwrap().render_type;
        assert_eq!(render("block/leaf").as_deref(), Some("cutout_mipped"));
        assert_eq!(render("block/bare").as_deref(), Some("translucent"));
        assert_eq!(render("block/absent"), None);
        assert_eq!(
            crate::bake::RenderType::by_name("minecraft:tripwire"),
            Some(crate::bake::RenderType::Tripwire)
        );
        assert_eq!(crate::bake::RenderType::by_name("mod:glow"), None);
    }
    #[test]
    fn neoforge_data_colour_and_legacy_emission_spellings() {
        let model = CuboidModel::from_json(
            r##"{"elements":[{"from":[0,0,0],"to":[1,1,1],"faces":{
                "up":{"texture":"#a","neoforge_data":{"color":"#80FF0000","light_emission":7}},
                "down":{"texture":"#a","neoforge_data":{"color":4294967295,"block_light":99}}}}]}"##,
        )
        .unwrap();
        let elements = model.elements.unwrap();
        let faces = &elements[0].as_ref().unwrap().faces;
        let up = faces[&Direction::Up]
            .as_ref()
            .unwrap()
            .neoforge_data
            .unwrap();
        assert_eq!(
            (up.color, up.block_light, up.sky_light),
            (Some(0x80FF0000u32 as i32), Some(7), Some(7))
        );
        let down = faces[&Direction::Down]
            .as_ref()
            .unwrap()
            .neoforge_data
            .unwrap();
        assert_eq!(
            (down.color, down.block_light, down.sky_light),
            (Some(-1), Some(15), None)
        );
    }
}
