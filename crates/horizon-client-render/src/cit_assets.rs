//! The ids Custom Item Textures' files take in Horizon's atlas and model library. A CIT rule
//! names its textures and models by resource path, mostly under `optifine/cit/` where vanilla
//! never looks: those take the reserved namespace [`NAMESPACE`], their path the resource path's
//! (`assets/minecraft/optifine/cit/swords/flame.png` is
//! `horizon_cit:minecraft/optifine/cit/swords/flame`). A file already under `textures/` (or
//! `models/`) keeps its vanilla id, so it is not packed twice.

use horizon_assets::model::{CuboidModel, TextureSlot};
use horizon_optifine::cit::{Kind, Rule};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The namespace of CIT files outside `textures/` and `models/`.
pub const NAMESPACE: &str = "horizon_cit";

/// `<namespace>:<path>`, if both keep to the identifier rules (lower-case letters, digits,
/// `_.-`, and `/` in the path). CIT Resewn refuses the other paths too.
fn identifier(namespace: &str, path: &str) -> Option<String> {
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    horizon_client_core::identifier::parse_identifier(&format!("{namespace}:{path}"))
}

/// The id of a resource path ending in `.<ext>`: `assets/<ns>/<folder>/<p>` is `<ns>:<p>`, any
/// other `assets/<rest>` is `horizon_cit:<rest>`.
fn id(resource: &str, folder: &str, ext: &str) -> Option<String> {
    let rest = resource
        .strip_prefix("assets/")?
        .strip_suffix(ext)?
        .strip_suffix('.')?;
    if let Some((namespace, path)) = rest.split_once('/')
        && let Some(path) = path.strip_prefix(folder).and_then(|p| p.strip_prefix('/'))
    {
        // A pack's own `assets/horizon_cit/` would name another file than its id maps back to.
        return (namespace != NAMESPACE)
            .then(|| identifier(namespace, path))
            .flatten();
    }
    identifier(NAMESPACE, rest)
}

/// The atlas sprite id of a `.png` resource path.
pub fn sprite_id(resource: &str) -> Option<String> {
    id(resource, "textures", "png")
}

/// The model id of a `.json` resource path.
pub fn model_id(resource: &str) -> Option<String> {
    id(resource, "models", "json")
}

/// The resource path of a `horizon_cit` id, with the extension `ext`; `None` for other ids,
/// and for one that would leave the resource root (a pack model may write any id).
pub fn resource(id: &str, ext: &str) -> Option<String> {
    let rest = id.strip_prefix(NAMESPACE)?.strip_prefix(':')?;
    horizon_optifine::paths::valid(rest).then(|| format!("assets/{rest}.{ext}"))
}

/// Sprite ids the rules whose kind passes `keep` draw: their `texture` and `texture.<name>`
/// values. A path no id can name is skipped with a warning.
pub fn rule_sprites(rules: &[Rule], keep: impl Fn(Kind) -> bool) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for rule in rules.iter().filter(|r| keep(r.kind)) {
        let named = rule.textures.iter().map(|(_, path)| path);
        for path in rule.texture.iter().chain(named) {
            match sprite_id(path) {
                Some(id) => {
                    out.insert(id);
                }
                None => log::warn!(
                    "custom item textures: {} names {path}, which is not a valid texture id",
                    rule.path
                ),
            }
        }
    }
    out
}

/// Sprite ids the item rules draw (`Kind::Item`, see [`rule_sprites`]). Textures named by CIT
/// JSON models are [`model_sprites`].
pub fn item_sprites(rules: &[Rule]) -> BTreeSet<String> {
    rule_sprites(rules, |kind| kind == Kind::Item)
}

/// The resource path a model value `./x` (or `./../x`, `../x`) names beside the file at `file`,
/// ending in `.<ext>` (added unless the value has it). `None` for other values and for paths
/// that would leave the root.
fn relative(file: &str, value: &str, ext: &str) -> Option<String> {
    let mut rest = value
        .strip_prefix("./")
        .or_else(|| value.starts_with("../").then_some(value))?;
    let mut dir = file.rsplit_once('/')?.0;
    while let Some(up) = rest.strip_prefix("../") {
        dir = dir.rsplit_once('/')?.0;
        rest = up;
    }
    let rest = rest
        .strip_suffix(ext)
        .and_then(|r| r.strip_suffix('.'))
        .unwrap_or(rest);
    let path = format!("{dir}/{rest}.{ext}");
    horizon_optifine::paths::valid(&path).then_some(path)
}

/// The id a relative value of the model at `file` names, as `CuboidModel::from_json` read it
/// (`./x` is `minecraft:./x` there), with the resource path of the file it names.
fn rebase(file: &str, id: &str, ext: &str) -> Option<(String, String)> {
    let path = relative(file, id.strip_prefix("minecraft:")?, ext)?;
    let id = if ext == "png" {
        sprite_id(&path)
    } else {
        model_id(&path)
    }?;
    Some((id, path))
}

/// Parents followed from a rule's model before the rest are left to the library.
const MAX_PARENTS: usize = 16;

/// The CIT JSON models the item rules name (`model`, `model.<name>`) under their [`model_id`]s,
/// with the parents they reach by relative path. Their `./x` texture and parent values become
/// the ids of the files beside them. Only `horizon_cit` models: the others are in the model
/// library already. A file that does not read or parse is left out, and a rule's layer that
/// names it (or a model whose parents reach it) keeps its vanilla model.
pub fn load_models(root: &Path, rules: &[Rule]) -> BTreeMap<String, CuboidModel> {
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for rule in rules.iter().filter(|r| r.kind == Kind::Item) {
        let named = rule.models.iter().map(|(_, path)| path);
        for path in rule.model.iter().chain(named) {
            load_model(root, path, 0, &mut seen, &mut out);
        }
    }
    out
}

fn load_model(
    root: &Path,
    resource: &str,
    depth: usize,
    seen: &mut BTreeSet<String>,
    out: &mut BTreeMap<String, CuboidModel>,
) {
    let Some(id) = model_id(resource) else {
        log::warn!("custom item textures: {resource} is not a valid model id");
        return;
    };
    let ours = id
        .strip_prefix(NAMESPACE)
        .is_some_and(|r| r.starts_with(':'));
    if !ours || depth > MAX_PARENTS || !seen.insert(id.clone()) {
        return;
    }
    let model = fs::read_to_string(root.join(resource))
        .map_err(|e| e.to_string())
        .and_then(|text| CuboidModel::from_json(&text).map_err(|e| e.to_string()));
    let mut model = match model {
        Ok(model) => model,
        Err(e) => {
            log::warn!("custom item textures: skipped model {resource}: {e}");
            return;
        }
    };
    for slot in model.textures.values_mut() {
        if let TextureSlot::Material(m) = slot
            && let Some((id, _)) = rebase(resource, &m.sprite, "png")
        {
            m.sprite = id;
        }
    }
    let parent = model
        .parent
        .as_deref()
        .and_then(|p| rebase(resource, p, "json"));
    if let Some((id, path)) = parent {
        model.parent = Some(id);
        load_model(root, &path, depth + 1, seen, out);
    }
    out.insert(id, model);
}

/// The `horizon_cit` sprites `models` name, which the atlas packs beside [`item_sprites`].
pub fn model_sprites(models: &BTreeMap<String, CuboidModel>) -> BTreeSet<String> {
    let prefix = format!("{NAMESPACE}:");
    models
        .values()
        .flat_map(|m| m.textures.values())
        .filter_map(|slot| match slot {
            TextureSlot::Material(m) if m.sprite.starts_with(&prefix) => Some(m.sprite.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas_sources::texture_path;
    use std::path::Path;

    #[test]
    fn cit_paths_take_the_reserved_namespace_and_come_back() {
        let png = "assets/minecraft/optifine/cit/swords/flame.png";
        let id = sprite_id(png).unwrap();
        assert_eq!(id, "horizon_cit:minecraft/optifine/cit/swords/flame");
        assert_eq!(resource(&id, "png").as_deref(), Some(png));
        let json = "assets/minecraft/optifine/cit/swords/flame.json";
        let id = model_id(json).unwrap();
        assert_eq!(id, "horizon_cit:minecraft/optifine/cit/swords/flame");
        assert_eq!(resource(&id, "json").as_deref(), Some(json));
    }

    #[test]
    fn files_under_textures_and_models_keep_their_vanilla_ids() {
        let id = sprite_id("assets/minecraft/textures/item/stick.png");
        assert_eq!(id.as_deref(), Some("minecraft:item/stick"));
        let id = model_id("assets/gems/models/item/ruby.json");
        assert_eq!(id.as_deref(), Some("gems:item/ruby"));
        // A model id is not a resource of this module's own.
        assert_eq!(resource("minecraft:item/stick", "png"), None);
        // `textures` as a folder name elsewhere is still a CIT path.
        let id = sprite_id("assets/minecraft/optifine/cit/textures/a.png");
        assert_eq!(
            id.as_deref(),
            Some("horizon_cit:minecraft/optifine/cit/textures/a")
        );
    }

    #[test]
    fn paths_no_identifier_can_name_are_refused() {
        for path in [
            "assets/minecraft/optifine/cit/Flame.png",
            "assets/minecraft/optifine/cit/fire sword.png",
            "assets/minecraft/textures/item/Stick.png",
            "assets/minecraft/optifine/cit/a.json",
            "optifine/cit/a.png",
        ] {
            assert_eq!(sprite_id(path), None, "{path}");
        }
        assert_eq!(model_id("assets/minecraft/optifine/cit/A.json"), None);
        assert_eq!(
            sprite_id("assets//textures/item/x.png"),
            None,
            "no namespace"
        );
    }

    #[test]
    fn the_reserved_namespace_names_only_files_inside_the_root() {
        // A pack's own `horizon_cit` namespace would map back to another file.
        assert_eq!(sprite_id("assets/horizon_cit/textures/item/x.png"), None);
        // A model may write any id: only ones inside the root name a file.
        for id in [
            "horizon_cit:../../etc/x",
            "horizon_cit:a/../../x",
            "horizon_cit:a\\..\\x",
        ] {
            assert_eq!(resource(id, "png"), None, "{id}");
            // Nor does the texture's file: refused, not read from elsewhere.
            let file = texture_path(Path::new("/r"), id, "textures");
            assert!(file.starts_with("/r/assets") && !file.to_string_lossy().contains(".."));
        }
        let escape = texture_path(Path::new("/r"), "minecraft:../../../x", "textures");
        assert!(!escape.to_string_lossy().contains(".."), "{escape:?}");
    }

    #[test]
    fn both_id_forms_find_their_file() {
        let root = Path::new("/r");
        assert_eq!(
            texture_path(root, "horizon_cit:minecraft/optifine/cit/a", "textures"),
            root.join("assets/minecraft/optifine/cit/a.png")
        );
        assert_eq!(
            texture_path(root, "minecraft:item/stick", "textures"),
            root.join("assets/minecraft/textures/item/stick.png")
        );
    }

    #[test]
    fn rule_models_load_with_their_relative_textures_and_parents() {
        let root = std::env::temp_dir().join(format!("horizon-cit-models-{}", std::process::id()));
        let cit = root.join("assets/minecraft/optifine/cit");
        fs::create_dir_all(cit.join("swords")).unwrap();
        let write = |rel: &str, text: &str| fs::write(cit.join(rel), text).unwrap();
        write(
            "swords/flame.json",
            r#"{"parent":"./../base","textures":{"layer0":"./flame","layer1":"item/stick"}}"#,
        );
        write(
            "base.json",
            r#"{"parent":"item/handheld","textures":{"layer2":"./glow.png"}}"#,
        );
        let props =
            horizon_optifine::Properties::parse_equals_first("items=stick\nmodel=./swords/flame");
        let rule = Rule::parse("assets/minecraft/optifine/cit/a.properties", &props, |p| {
            root.join(p).is_file()
        })
        .unwrap();
        let models = load_models(&root, &[rule]);
        let _ = fs::remove_dir_all(&root);
        let cit = |p: &str| format!("horizon_cit:minecraft/optifine/cit/{p}");
        assert_eq!(
            models.keys().cloned().collect::<Vec<_>>(),
            [cit("base"), cit("swords/flame")]
        );
        let flame = &models[&cit("swords/flame")];
        assert_eq!(flame.parent.as_deref(), Some(cit("base").as_str()));
        assert_eq!(
            models[&cit("base")].parent.as_deref(),
            Some("minecraft:item/handheld")
        );
        assert_eq!(
            model_sprites(&models).into_iter().collect::<Vec<_>>(),
            [cit("glow"), cit("swords/flame")]
        );
    }

    #[test]
    fn relative_values_stay_in_the_root() {
        let file = "assets/minecraft/optifine/cit/a/m.json";
        let at = |v: &str| relative(file, v, "png");
        assert_eq!(
            at("./x").as_deref(),
            Some("assets/minecraft/optifine/cit/a/x.png")
        );
        assert_eq!(
            at("./../x.png").as_deref(),
            Some("assets/minecraft/optifine/cit/x.png")
        );
        assert_eq!(
            at("../../x").as_deref(),
            Some("assets/minecraft/optifine/x.png")
        );
        assert_eq!(at("item/x"), None);
        assert_eq!(at("./../../../../../x"), None);
        assert_eq!(at("./a/../x"), None);
    }
}
