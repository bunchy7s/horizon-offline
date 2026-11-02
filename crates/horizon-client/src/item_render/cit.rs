//! OptiFine Custom Item Textures on item models: the layer of a stack a `type=item` rule matches
//! draws the rule's model, or its own model with the rule's textures in some slots. The rules'
//! JSON models join the model library, their textures the atlas (`cit_assets`).

use super::{ItemRenderer, corners};
use crate::cit::{CitRules, Hand, Kind};
use crate::cit_assets::{self, model_id, sprite_id};
use crate::hud::cit_glint::GlintTextures;
use crate::items::ItemStack;
use glam::Vec3;
use horizon_assets::{
    item_bake::{BakedItemModel, bake_resolved},
    model::{Material, TextureSlot},
    resolve::{MISSING_MODEL, ModelLibrary},
};
use horizon_world::nbt::Tag;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// A baked layer and its quad corners, as `ItemRenderer::bake` and `quad_extents` give them.
pub(super) type Baked = (Arc<BakedItemModel>, Arc<Vec<Vec3>>);

/// What one item rule swaps, as model and sprite ids.
#[derive(Debug, Default)]
struct Plan {
    model: Option<String>,
    /// `model.<name>`, by the leaf model's [`short`] name.
    models: HashMap<String, String>,
    texture: Option<String>,
    /// `texture.<name>`: a leaf, a slot or a sprite's file name, and its sprite.
    textures: Vec<(String, String)>,
}

/// One swapped bake: the model and the sprite each changed slot takes, `None`
/// for a layer cleared; and the generic texture, whose quads are left untinted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CitBake {
    base: String,
    slots: BTreeMap<String, Option<String>>,
    untinted: Option<String>,
}

/// The enabled packs' rules and what their item rules bake to.
#[derive(Debug)]
pub struct CitModels {
    pub rules: CitRules,
    /// The enchantment rules' glint textures.
    pub glint_textures: GlintTextures,
    /// Per rule (an index into `rules.rules`); `None` for rules of other kinds.
    plans: Vec<Option<Plan>>,
    /// Per (rule, leaf model): its bake, `None` where the rule leaves the leaf alone.
    memo: Mutex<HashMap<(usize, String), Option<CitBake>>>,
    baked: Mutex<HashMap<CitBake, Baked>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The last segment of an id: `minecraft:item/bow_pulling_1` is `bow_pulling_1`.
fn short(id: &str) -> &str {
    let path = id.rsplit_once(':').map_or(id, |(_, p)| p);
    path.rsplit('/').next().unwrap_or(path)
}

/// The 26.x name of an OptiFine sub-texture or sub-model name: the bow's
/// `bow_standby` frame is model `item/bow`, and the potions' bottle layers are
/// the files `potion`, `splash_potion` and `lingering_potion`.
fn legacy(name: &str) -> &str {
    match name {
        "bow_standby" => "bow",
        "potion_bottle_drinkable" => "potion",
        "potion_bottle_splash" => "splash_potion",
        "potion_bottle_lingering" => "lingering_potion",
        other => other,
    }
}

/// The `max_damage` a CIT rule's `damage` test reads: the stack's own
/// component when it holds a non-negative int, else `item_max`, its item's.
pub fn stack_max_damage(stack: &ItemStack, item_max: u32) -> u32 {
    stack
        .data()
        .and_then(|d| match d.other("minecraft:max_damage") {
            Some(Tag::Int(v)) => u32::try_from(*v).ok(),
            _ => None,
        })
        .unwrap_or(item_max)
}

/// The winning `kind` (Armor or Elytra) rule of a worn `stack` whose item has `item_max` max
/// damage, as an index into `rules.rules`. Worn items count as the main hand: OptiFine's off hand
/// is the off-hand slot alone.
pub fn worn_rule(rules: &CitRules, kind: Kind, stack: &ItemStack, item_max: u32) -> Option<usize> {
    rules.select(kind, stack, Hand::Main, stack_max_damage(stack, item_max))
}

/// The rules at `root` and `library` with their models, or `library` alone without rules.
pub(super) fn load(
    root: &Path,
    library: Arc<ModelLibrary>,
) -> (Arc<ModelLibrary>, Option<CitModels>) {
    let rules = CitRules::load(root);
    // A `cit.properties` alone can still turn vanilla's glint off.
    if rules.is_empty() && rules.global.use_glint {
        return (library, None);
    }
    let extra = cit_assets::load_models(root, &rules.rules);
    let library = if extra.is_empty() {
        library
    } else {
        match library.overlaid(extra) {
            Ok(over) => Arc::new(over),
            Err(e) => {
                log::warn!("custom item textures: models left out: {e}");
                library
            }
        }
    };
    let mut models = CitModels::new(rules);
    models.glint_textures = GlintTextures::load(root, &models.rules.rules);
    // An enchantment rule whose texture has no slot draws nothing, so it must not take its
    // layer (or vanilla's glint) from the stack either.
    let slots = &models.glint_textures.slot_of;
    models
        .rules
        .keep_glints(|i| slots.get(i).copied().flatten().is_some());
    (library, Some(models))
}

impl CitModels {
    fn new(rules: CitRules) -> Self {
        // By 26.x name, in file order. A 26.x name beats OptiFine's old name
        // for the same thing (`texture.bow` over `texture.bow_standby`)
        // wherever either stands; of two alike, the later wins.
        let pairs = |list: &[(String, String)], id: fn(&str) -> Option<String>| {
            let mut out: Vec<(String, String, bool)> = Vec::new();
            for (name, path) in list {
                let Some(asset) = id(path) else { continue };
                let (old, new) = (short(name), legacy(short(name)));
                let modern = old == new;
                match out.iter_mut().find(|(n, ..)| n.as_str() == new) {
                    Some(entry) if modern || !entry.2 => *entry = (new.to_owned(), asset, modern),
                    Some(_) => {}
                    None => out.push((new.to_owned(), asset, modern)),
                }
            }
            out.into_iter()
                .map(|(name, asset, _)| (name, asset))
                .collect::<Vec<_>>()
        };
        let plans = rules
            .rules
            .iter()
            .map(|r| {
                (r.kind == Kind::Item).then(|| Plan {
                    model: r.model.as_deref().and_then(model_id),
                    models: pairs(&r.models, model_id).into_iter().collect(),
                    texture: r.texture.as_deref().and_then(sprite_id),
                    textures: pairs(&r.textures, sprite_id),
                })
            })
            .collect();
        Self {
            rules,
            glint_textures: GlintTextures::default(),
            plans,
            memo: Mutex::default(),
            baked: Mutex::default(),
        }
    }
}

impl ItemRenderer {
    /// The layer whose model is `leaf` as item rule `rule` draws it; `None` draws it as usual.
    pub(super) fn cit_layer(&self, rule: usize, leaf: &str) -> Option<Baked> {
        let cit = self.cit.as_ref()?;
        let key = (rule, leaf.to_owned());
        let known = lock(&cit.memo).get(&key).cloned();
        let bake = match known {
            Some(bake) => bake,
            None => {
                let bake = self.cit_plan(cit, rule, leaf);
                lock(&cit.memo).insert(key, bake.clone());
                bake
            }
        }?;
        if let Some(hit) = lock(&cit.baked).get(&bake) {
            return Some(hit.clone());
        }
        let baked = self.cit_bake(&bake);
        lock(&cit.baked).insert(bake, baked.clone());
        Some(baked)
    }

    /// The model choice for one leaf, as CIT Resewn's wiki (CIT Item) gives it:
    /// `model.<leaf>` draws that model. `texture.<leaf>` replaces the leaf
    /// with the item generated from it (on a model with elements, its every
    /// face); on a leaf of several layers, one of
    /// which is a slot or a sprite file named `<leaf>` (the 26.x potion's
    /// bottle is `layer1`, file `potion`), it is that layer's texture instead.
    /// Else `model` (or the leaf) with `texture` in every slot of a model, or
    /// the item generated from `texture`. Each `texture.<name>` goes to the
    /// slot `name` and to every slot whose sprite's file is `name`
    /// (`potion_overlay`); the generic `texture` takes the layers none of
    /// them claims. A model that did not load, or whose parents do not,
    /// leaves the leaf as it is.
    fn cit_plan(&self, cit: &CitModels, rule: usize, leaf: &str) -> Option<CitBake> {
        let plan = cit.plans.get(rule)?.as_ref()?;
        let sub = short(leaf);
        if let Some(model) = plan.models.get(sub) {
            return self.library.resolves(model).then(|| CitBake {
                base: model.clone(),
                slots: BTreeMap::new(),
                untinted: None,
            });
        }
        let resolved = |name: &str| self.library.resolve(name).ok();
        let matches = |name: &str, slot: &str, material: &Material| {
            slot == name || short(&material.sprite) == name
        };
        let layer = |slot: &str| {
            slot.strip_prefix("layer")
                .and_then(|n| n.parse::<u32>().ok())
        };
        let leaf_model = resolved(leaf);
        let leaf_textures = leaf_model
            .as_ref()
            .map(|m| m.textures.clone())
            .unwrap_or_default();
        // `texture.<leaf>` swaps the leaf (`texture.bow_pulling_1`). On a leaf
        // of several layers, one of which answers to the name (`potion`'s
        // bottle), it is that layer's swap instead, done below, and leaves
        // `model` and `texture` to the rule.
        let layers = leaf_textures.keys().filter(|s| layer(s).is_some()).count();
        let named = plan
            .textures
            .iter()
            .find(|(n, _)| n == sub)
            .map(|(_, s)| s)
            .filter(|_| {
                layers < 2
                    || !leaf_textures
                        .iter()
                        .any(|(slot, m)| matches(sub, slot.as_str(), m))
            });
        let base = match (named, &plan.model) {
            (None, Some(model)) => model.clone(),
            _ => leaf.to_owned(),
        };
        if base != leaf && !self.library.resolves(&base) {
            return None;
        }
        let base_model = if base == leaf {
            leaf_model
        } else {
            resolved(&base)
        };
        let generated = base_model.as_ref().is_some_and(|m| m.uses_item_generator());
        let textures = base_model.map(|m| m.textures).unwrap_or_default();
        // The slots each `texture.<name>` claims, by name or by sprite file.
        let mut claimed = BTreeMap::new();
        for (name, sprite) in &plan.textures {
            for (slot, material) in &textures {
                if matches(name.as_str(), slot.as_str(), material) {
                    claimed.insert(slot.clone(), sprite.clone());
                }
            }
        }
        let mut slots = BTreeMap::new();
        let mut untinted = None;
        match named.or(plan.texture.as_ref()) {
            // A model's elements (a block item, a 3D model) draw the sprite
            // on every face, as does the rule's `model` with `texture`.
            Some(sprite) if (named.is_none() && plan.model.is_some()) || !generated => {
                for slot in textures.keys() {
                    slots.insert(slot.clone(), Some(sprite.clone()));
                }
            }
            // The item generated from the sprite, drawn once and untinted: a
            // pack's art holds its own colours (OptiFine's doc: a potion's
            // replacement includes "the colored liquid"), and the tinted layer
            // is the bottom one on potions but the top one on maps, firework
            // stars and dyed wolf armour. It is `layer0`, or the lowest layer no
            // `texture.<name>` claims (`texture.potion_overlay` with `texture`:
            // the bottle), and fills the unclaimed layers up to the highest
            // claimed one, so they stay contiguous; the leaf's layers above are
            // cleared (`bake_resolved` stops at the first layer without one).
            Some(sprite) => {
                untinted = Some(sprite.clone());
                let first_free = (0..)
                    .find(|n| !claimed.contains_key(&format!("layer{n}")))
                    .unwrap_or(0);
                let top = claimed
                    .keys()
                    .filter_map(|s| layer(s))
                    .max()
                    .unwrap_or(0)
                    .max(first_free);
                slots.insert("layer0".to_owned(), Some(sprite.clone()));
                for slot in textures.keys() {
                    match layer(slot) {
                        Some(n) if n <= top => {
                            slots.insert(slot.clone(), Some(sprite.clone()));
                        }
                        Some(_) => {
                            slots.insert(slot.clone(), None);
                        }
                        None => {}
                    }
                }
            }
            None => {}
        }
        slots.extend(
            claimed
                .into_iter()
                .map(|(slot, sprite)| (slot, Some(sprite))),
        );
        (base != leaf || !slots.is_empty()).then_some(CitBake {
            base,
            slots,
            untinted,
        })
    }

    fn cit_bake(&self, bake: &CitBake) -> Baked {
        let child = bake
            .slots
            .iter()
            .map(|(slot, sprite)| {
                let texture = match sprite {
                    Some(sprite) => TextureSlot::Material(Material {
                        sprite: sprite.clone(),
                        force_translucent: false,
                    }),
                    // A slot referring to itself resolves to nothing
                    // (`ModelLibrary::resolve_with_textures` stops at the loop).
                    None => TextureSlot::Reference(slot.clone()),
                };
                (slot.clone(), texture)
            })
            .collect();
        let baked = self
            .library
            .resolve_with_textures(&bake.base, &child)
            .and_then(|model| bake_resolved(&model, &self.sprites))
            .map(|mut baked| {
                if let Some(sprite) = &bake.untinted {
                    for quad in baked
                        .quads
                        .iter_mut()
                        .filter(|q| &q.material.sprite == sprite)
                    {
                        quad.tint_index = -1;
                    }
                }
                Arc::new(baked)
            })
            .unwrap_or_else(|_| self.bake(MISSING_MODEL));
        let extents = Arc::new(corners(&baked));
        (baked, extents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::item_model::{DisplayContext, NoEnv, StackView};
    use std::{collections::BTreeSet, fs, path::PathBuf};

    const CIT: &str = "assets/minecraft/optifine/cit";

    fn png(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
        let mut encoder = png::Encoder::new(file, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255; 16]).unwrap();
        writer.finish().unwrap();
    }

    /// A stick, a bow's frames and a potion; with `cit`, rules over them.
    fn root(name: &str, cit: bool) -> PathBuf {
        let root = std::env::temp_dir().join(format!("horizon-cit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "items.json",
            r#"{"minecraft:stick":{"minecraft:item_model":"minecraft:stick"}}"#,
        );
        write(
            "assets/minecraft/items/stick.json",
            r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick"}}"#,
        );
        let models = "assets/minecraft/models/item";
        write(
            &format!("{models}/generated.json"),
            r#"{"parent":"builtin/generated"}"#,
        );
        let flat = |name: &str, layers: &[&str]| {
            let textures: Vec<String> = layers
                .iter()
                .enumerate()
                .map(|(i, l)| format!(r#""layer{i}":"item/{l}""#))
                .collect();
            write(
                &format!("{models}/{name}.json"),
                &format!(
                    r#"{{"parent":"item/generated","textures":{{{}}}}}"#,
                    textures.join(",")
                ),
            );
            for l in layers {
                png(&root.join(format!("assets/minecraft/textures/item/{l}.png")));
            }
        };
        flat("stick", &["stick"]);
        flat("bow", &["bow"]);
        flat("bow_pulling_0", &["bow_pulling_0"]);
        flat("bow_pulling_1", &["bow_pulling_1"]);
        flat("potion", &["potion_overlay", "potion"]);
        // A block item: one cube whose faces all draw `#all`.
        let faces: Vec<String> = ["down", "up", "north", "south", "west", "east"]
            .iter()
            .map(|f| format!(r##""{f}":{{"texture":"#all"}}"##))
            .collect();
        write(
            "assets/minecraft/models/block/cube_fixture.json",
            &format!(
                r##"{{"textures":{{"particle":"#all","all":"block/cube_fixture"}},"elements":[{{"from":[0,0,0],"to":[16,16,16],"faces":{{{}}}}}]}}"##,
                faces.join(",")
            ),
        );
        png(&root.join("assets/minecraft/textures/block/cube_fixture.png"));
        if cit {
            for t in ["a", "b", "blade", "hilt"] {
                png(&root.join(format!("{CIT}/{t}.png")));
            }
            write(
                &format!("{CIT}/sword.json"),
                r#"{"parent":"item/generated","textures":{"layer0":"./blade","layer1":"./hilt"}}"#,
            );
            write(
                &format!("{CIT}/other.json"),
                r#"{"parent":"item/generated","textures":{"layer0":"./hilt"}}"#,
            );
            write(&format!("{CIT}/bad.json"), "{not json");
            let rules = [
                ("plain", "items=bow\ntexture.bow_pulling_1=a"),
                (
                    "mix",
                    "items=bow\nmodel=sword\ntexture=a\ntexture.bow_pulling_1=b\nmodel.bow_pulling_0=other",
                ),
                (
                    "potion",
                    "items=potion\ntexture.potion_overlay=a\ntexture.layer1=b",
                ),
                ("stick", "items=stick\nmodel=sword"),
                ("bottle", "items=potion\ntexture.potion=a"),
                (
                    "drinkable",
                    "items=potion\ntexture.potion_bottle_drinkable=b",
                ),
                ("whole", "items=potion\ntexture=a"),
                ("standby", "items=bow\ntexture.bow_standby=b"),
                ("broken", "items=bow\nmodel=bad\nmodel.bow_pulling_0=sword"),
                ("bad_sub", "items=bow\ntexture=a\nmodel.bow_pulling_1=bad"),
                (
                    "overlay_and_generic",
                    "items=potion\ntexture=a\ntexture.potion_overlay=b",
                ),
                ("block", "items=stone\ntexture=a"),
                (
                    "overlay_and_bottle",
                    "items=potion\ntexture=a\ntexture.potion=b",
                ),
                (
                    "bottle_on_model",
                    "items=potion\nmodel=sword\ntexture.potion=b",
                ),
                (
                    "old_first",
                    "items=bow\ntexture.bow_standby=b\ntexture.bow=a",
                ),
                (
                    "new_first",
                    "items=bow\ntexture.bow=a\ntexture.bow_standby=b",
                ),
                (
                    "old_model_first",
                    "items=bow\nmodel.bow_standby=other\nmodel.bow=sword",
                ),
                (
                    "new_model_first",
                    "items=bow\nmodel.bow=sword\nmodel.bow_standby=other",
                ),
            ];
            for (name, text) in rules {
                write(&format!("{CIT}/{name}.properties"), text);
            }
        }
        root
    }

    fn renderer(root: &Path) -> ItemRenderer {
        let library = crate::model_library::ParsedModels::load(root).into_item_library();
        ItemRenderer::load_with_library(root, library).unwrap()
    }

    fn rule(r: &ItemRenderer, name: &str) -> usize {
        let path = format!("{CIT}/{name}.properties");
        let rules = &r.cit.as_ref().unwrap().rules.rules;
        rules.iter().position(|r| r.path == path).unwrap()
    }

    fn sprites(baked: &BakedItemModel) -> BTreeSet<String> {
        baked
            .quads
            .iter()
            .map(|q| q.material.sprite.clone())
            .collect()
    }

    /// Each quad's layer (its tint index) with the sprite it draws.
    fn layers(baked: &BakedItemModel) -> BTreeSet<(i32, String)> {
        baked
            .quads
            .iter()
            .map(|q| (q.tint_index, q.material.sprite.clone()))
            .collect()
    }

    fn cit(name: &str) -> String {
        format!("horizon_cit:minecraft/optifine/cit/{name}")
    }

    #[test]
    fn rules_pick_a_model_or_textures_per_leaf() {
        let root = root("plans", true);
        let r = renderer(&root);
        let layer = |rule: &str, leaf: &str| {
            r.cit_layer(self::rule(&r, rule), &format!("minecraft:item/{leaf}"))
                .map(|(baked, _)| sprites(&baked).into_iter().collect::<Vec<_>>())
        };
        // `texture.<leaf>` swaps that leaf only.
        assert_eq!(layer("plain", "bow_pulling_1"), Some(vec![cit("a")]));
        assert_eq!(layer("plain", "bow_pulling_0"), None);
        assert_eq!(layer("plain", "bow"), None);
        // `model.<leaf>` first, then `texture.<leaf>` (a one-layer frame: the whole leaf), then
        // `model` with `texture` in every slot.
        assert_eq!(layer("mix", "bow_pulling_0"), Some(vec![cit("hilt")]));
        assert_eq!(layer("mix", "bow_pulling_1"), Some(vec![cit("b")]));
        assert_eq!(layer("mix", "bow"), Some(vec![cit("a")]));
        // A model alone keeps its own (relative) textures.
        assert_eq!(
            layer("stick", "stick"),
            Some(vec![cit("blade"), cit("hilt")])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn potion_textures_go_to_the_layer_they_name() {
        let root = root("potion", true);
        let r = renderer(&root);
        let layer = |rule: &str| {
            let (baked, _) = r.cit_layer(self::rule(&r, rule), "minecraft:item/potion")?;
            Some(layers(&baked))
        };
        let vanilla = |i: i32, name: &str| (i, format!("minecraft:item/{name}"));
        // By sprite file name (the overlay) and by slot name (the bottle).
        assert_eq!(
            layer("potion"),
            Some(BTreeSet::from([(0, cit("a")), (1, cit("b"))]))
        );
        // `texture.potion` names the bottle's file, not the leaf's `layer0`.
        assert_eq!(
            layer("bottle"),
            Some(BTreeSet::from([
                vanilla(0, "potion_overlay"),
                (1, cit("a"))
            ]))
        );
        // OptiFine's name for the same bottle.
        assert_eq!(
            layer("drinkable"),
            Some(BTreeSet::from([
                vanilla(0, "potion_overlay"),
                (1, cit("b"))
            ]))
        );
        // A plain `texture` is drawn once, untinted: the pack's whole picture, no vanilla
        // bottle over it and no potion colour on it.
        assert_eq!(layer("whole"), Some(BTreeSet::from([(-1, cit("a"))])));
        // ... unless a `texture.<name>` fills the bottle, even one named like the leaf.
        assert_eq!(
            layer("overlay_and_bottle"),
            Some(BTreeSet::from([(-1, cit("a")), (1, cit("b"))]))
        );
        // A slot named like the leaf leaves the rule's `model` the base.
        let (baked, _) = r
            .cit_layer(rule(&r, "bottle_on_model"), "minecraft:item/potion")
            .unwrap();
        assert_eq!(sprites(&baked), BTreeSet::from([cit("blade"), cit("hilt")]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_generic_texture_takes_the_layers_no_named_texture_claims() {
        let root = root("generic", true);
        let r = renderer(&root);
        let (baked, _) = r
            .cit_layer(rule(&r, "overlay_and_generic"), "minecraft:item/potion")
            .unwrap();
        // `texture.potion_overlay` claims `layer0`; `texture` is the bottle.
        assert_eq!(
            layers(&baked),
            BTreeSet::from([(0, cit("b")), (-1, cit("a"))])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plain_texture_covers_every_face_of_a_block_item() {
        let root = root("block", true);
        let r = renderer(&root);
        let (baked, _) = r
            .cit_layer(rule(&r, "block"), "minecraft:block/cube_fixture")
            .unwrap();
        assert_eq!(baked.quads.len(), 6);
        assert_eq!(sprites(&baked), BTreeSet::from([cit("a")]));
        assert_eq!(baked.particle.sprite, cit("a"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_26x_name_beats_optifines_old_one_in_any_order() {
        let root = root("aliases", true);
        let r = renderer(&root);
        let bow = |rule: &str| {
            let (baked, _) = r
                .cit_layer(self::rule(&r, rule), "minecraft:item/bow")
                .unwrap();
            sprites(&baked)
        };
        for name in ["old_first", "new_first"] {
            assert_eq!(bow(name), BTreeSet::from([cit("a")]), "{name}");
        }
        for name in ["old_model_first", "new_model_first"] {
            assert_eq!(
                bow(name),
                BTreeSet::from([cit("blade"), cit("hilt")]),
                "{name}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bow_standby_is_the_resting_bow() {
        let root = root("standby", true);
        let r = renderer(&root);
        let standby = rule(&r, "standby");
        let (baked, _) = r.cit_layer(standby, "minecraft:item/bow").unwrap();
        assert_eq!(layers(&baked), BTreeSet::from([(-1, cit("b"))]));
        assert!(
            r.cit_layer(standby, "minecraft:item/bow_pulling_0")
                .is_none()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_that_did_not_load_leaves_the_leaf_vanilla() {
        let root = root("broken", true);
        let r = renderer(&root);
        let broken = rule(&r, "broken");
        assert!(r.cit_layer(broken, "minecraft:item/bow").is_none());
        // The rule's other model still loads and draws.
        let (baked, _) = r.cit_layer(broken, "minecraft:item/bow_pulling_0").unwrap();
        assert_eq!(sprites(&baked), BTreeSet::from([cit("blade"), cit("hilt")]));
        // A broken `model.<leaf>` leaves that leaf vanilla, not drawn from the rule's `texture`.
        let bad_sub = rule(&r, "bad_sub");
        assert!(
            r.cit_layer(bad_sub, "minecraft:item/bow_pulling_1")
                .is_none()
        );
        let (baked, _) = r.cit_layer(bad_sub, "minecraft:item/bow").unwrap();
        assert_eq!(sprites(&baked), BTreeSet::from([cit("a")]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn max_damage_is_the_stacks_component_else_the_items() {
        let with = |tag: Option<Tag>| {
            let mut stack = ItemStack::new("minecraft:bow", 1);
            if let Some(tag) = tag {
                stack
                    .data_mut()
                    .set_other("minecraft:max_damage", Some(tag));
            }
            stack_max_damage(&stack, 384)
        };
        assert_eq!(with(Some(Tag::Int(20))), 20);
        assert_eq!(with(Some(Tag::Int(0))), 0);
        assert_eq!(with(None), 384);
        assert_eq!(with(Some(Tag::Int(-1))), 384);
        assert_eq!(with(Some(Tag::String("20".into()))), 384);
    }

    fn armor_rules(text: &str) -> CitRules {
        let props = horizon_optifine::Properties::parse_equals_first(text);
        let path = "assets/minecraft/optifine/cit/worn.properties";
        CitRules::new(vec![
            crate::cit::Rule::parse(path, &props, |_| true).unwrap(),
        ])
    }

    #[test]
    fn worn_armor_counts_as_the_main_hand() {
        let rules = armor_rules("type=armor\nitems=diamond_chestplate\ntexture=x\nhand=main\n");
        let stack = ItemStack::new("minecraft:diamond_chestplate", 1);
        assert_eq!(worn_rule(&rules, Kind::Armor, &stack, 528), Some(0));
        let off = armor_rules("type=armor\nitems=diamond_chestplate\ntexture=x\nhand=off\n");
        assert_eq!(worn_rule(&off, Kind::Armor, &stack, 528), None);
    }

    #[test]
    fn worn_damage_reads_the_stacks_max_damage() {
        let rules = armor_rules("type=armor\nitems=diamond_chestplate\ntexture=x\ndamage=50%\n");
        let mut stack = ItemStack::new("minecraft:diamond_chestplate", 1);
        stack.damage = 5;
        // 5 of the item's 528 is under 1%.
        assert_eq!(worn_rule(&rules, Kind::Armor, &stack, 528), None);
        stack
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(10)));
        assert_eq!(worn_rule(&rules, Kind::Armor, &stack, 528), Some(0));
    }

    #[test]
    fn a_bake_is_shared_by_its_rule_and_leaf_only() {
        let root = root("cache", true);
        let r = renderer(&root);
        let leaf = "minecraft:item/bow_pulling_1";
        let (plain, mix) = (rule(&r, "plain"), rule(&r, "mix"));
        let first = r.cit_layer(plain, leaf).unwrap();
        let again = r.cit_layer(plain, leaf).unwrap();
        assert!(Arc::ptr_eq(&first.0, &again.0));
        assert!(Arc::ptr_eq(&first.1, &again.1));
        assert!(!Arc::ptr_eq(&first.0, &r.cit_layer(mix, leaf).unwrap().0));
        assert!(!Arc::ptr_eq(&first.0, &r.bake(leaf)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_matched_stack_resolves_to_the_rule_model() {
        let root = root("resolve", true);
        let r = renderer(&root);
        let view = StackView::new("minecraft:stick", 1);
        let resolve = |rule| {
            let state =
                r.resolve_with_rule(&view, DisplayContext::Gui, None, None, 0, &NoEnv, rule);
            match &state.layers[0].geometry {
                super::super::LayerGeometry::Quads(baked) => sprites(baked),
                other => panic!("{other:?}"),
            }
        };
        let stick = rule(&r, "stick");
        assert_eq!(
            resolve(Some(stick)),
            BTreeSet::from([cit("blade"), cit("hilt")])
        );
        assert_eq!(
            resolve(None),
            BTreeSet::from(["minecraft:item/stick".to_owned()])
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn without_rules_nothing_changes() {
        let root = root("none", false);
        let r = renderer(&root);
        assert!(r.cit.is_none());
        let view = StackView::new("minecraft:stick", 1);
        let state = |rule| {
            let s = r.resolve_with_rule(&view, DisplayContext::Gui, None, None, 0, &NoEnv, rule);
            format!("{s:?}")
        };
        let plain = format!(
            "{:?}",
            r.resolve(&view, DisplayContext::Gui, None, None, 0, &NoEnv)
        );
        assert_eq!(state(Some(0)), plain);
        assert_eq!(state(None), plain);
        assert!(plain.contains("minecraft:item/stick"));
        let _ = fs::remove_dir_all(&root);
    }
}
