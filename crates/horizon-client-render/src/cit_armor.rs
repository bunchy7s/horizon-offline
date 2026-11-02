//! Custom Item Textures for worn armour and elytra (`type=armor`, `type=elytra`). An armour rule
//! names its textures after the armour texture files of before 1.21.2
//! (`textures/models/armor/diamond_layer_1.png`): `texture.diamond_layer_1`,
//! `texture.leather_layer_2_overlay`..., with `texture` for any layer it does not name. 26.3's
//! equipment assets name the same layers `entity/equipment/humanoid/diamond` (layer 1: helmet,
//! chestplate, boots) and `entity/equipment/humanoid_leggings/diamond` (layer 2), so the layers
//! are mapped back to the old names here. An elytra rule's `texture` replaces the wings'.

use crate::cit_assets::{rule_sprites, sprite_id};
use horizon_optifine::cit::{Kind, Rule};
use std::collections::BTreeSet;

/// The pre-1.21.2 armour texture name of an equipment asset's layer: `asset` (`minecraft:gold`
/// or `gold`) on the leggings layer (`humanoid_leggings`) or the others (`humanoid`), the dye
/// overlay (`leather_overlay`) or the main texture.
pub fn armor_layer_name(asset: &str, leggings: bool, overlay: bool) -> String {
    let path = asset.rsplit_once(':').map_or(asset, |(_, p)| p);
    // The equipment assets renamed from their old textures; the rest kept their names.
    let old = match path {
        "turtle_scute" => "turtle",
        other => other,
    };
    let layer = if leggings { 2 } else { 1 };
    let suffix = if overlay { "_overlay" } else { "" };
    format!("{old}_layer_{layer}{suffix}")
}

/// The texture (a resource path) `rule` draws on the layer named `layer`: its `texture.<layer>`,
/// else its `texture`.
pub fn armor_texture<'a>(rule: &'a Rule, layer: &str) -> Option<&'a str> {
    rule.textures
        .iter()
        .find(|(name, _)| name == layer)
        .map(|(_, path)| path.as_str())
        .or(rule.texture.as_deref())
}

/// The atlas name of a CIT texture: its `cit_assets` id, without `minecraft:` as the atlas packs
/// vanilla's sprites.
fn atlas_name(resource: &str) -> Option<String> {
    let id = sprite_id(resource)?;
    Some(match id.strip_prefix("minecraft:") {
        Some(name) => name.to_owned(),
        None => id,
    })
}

/// The atlas sprite `rule` puts on the layer of equipment asset `asset` (see
/// [`armor_layer_name`]); `None` keeps the vanilla texture.
pub fn layer_sprite(rule: &Rule, asset: &str, leggings: bool, overlay: bool) -> Option<String> {
    atlas_name(armor_texture(
        rule,
        &armor_layer_name(asset, leggings, overlay),
    )?)
}

/// The atlas sprite an elytra rule puts on the wings.
pub fn elytra_sprite(rule: &Rule) -> Option<String> {
    atlas_name(armor_texture(rule, "elytra")?)
}

/// The atlas sprite the one CIT armour rule `rule` puts on a worn armour layer: the layer
/// `vanilla` (an atlas name, `…_overlay` for the dye overlay) of equipment asset `asset` in
/// equipment layer `kind` (`humanoid_leggings` for the leggings). `None` draws `vanilla`, as does
/// a name the atlas lacks; nothing is allocated without a rule.
pub fn worn_layer_sprite(
    rule: Option<&Rule>,
    asset: &str,
    kind: &str,
    vanilla: &str,
) -> Option<String> {
    let leggings = kind == "humanoid_leggings";
    let overlay = vanilla.ends_with("_overlay");
    layer_sprite(rule?, asset, leggings, overlay)
}

/// The CIT elytra rule's wings sprite. On `None`, or a name the atlas lacks, the caller draws the
/// cape, else the equipment asset's own wings.
pub fn wings_sprite(rule: Option<&Rule>) -> Option<String> {
    elytra_sprite(rule?)
}

/// What a worn layer draws, looked up with `get` (the atlas): the rule's sprite
/// ([`worn_layer_sprite`]), else `vanilla` when there is no rule or the atlas lacks its texture.
pub fn worn_layer<R>(
    rule: Option<&Rule>,
    asset: &str,
    kind: &str,
    vanilla: &str,
    get: impl Fn(&str) -> Option<R>,
) -> Option<R> {
    worn_layer_sprite(rule, asset, kind, vanilla)
        .and_then(|name| get(&name))
        .or_else(|| get(vanilla))
}

/// What worn wings draw, looked up with `get`: the rule's sprite, else the cape; `None` leaves
/// them to the equipment asset's own wings.
pub fn wings<R>(
    rule: Option<&Rule>,
    cape: Option<&str>,
    get: impl Fn(&str) -> Option<R>,
) -> Option<R> {
    wings_sprite(rule)
        .and_then(|name| get(&name))
        .or_else(|| cape.and_then(&get))
}

/// Sprite ids the armour and elytra rules draw (see [`rule_sprites`]).
pub fn armor_sprites(rules: &[Rule]) -> BTreeSet<String> {
    rule_sprites(rules, |kind| matches!(kind, Kind::Armor | Kind::Elytra))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_optifine::Properties;

    fn rule(path: &str, text: &str) -> Rule {
        Rule::parse(path, &Properties::parse_equals_first(text), |_| true).unwrap()
    }

    #[test]
    fn layers_take_their_old_texture_names() {
        assert_eq!(
            armor_layer_name("minecraft:diamond", false, false),
            "diamond_layer_1"
        );
        assert_eq!(armor_layer_name("diamond", true, false), "diamond_layer_2");
        assert_eq!(
            armor_layer_name("minecraft:leather", false, true),
            "leather_layer_1_overlay"
        );
        assert_eq!(
            armor_layer_name("minecraft:leather", true, true),
            "leather_layer_2_overlay"
        );
        assert_eq!(
            armor_layer_name("minecraft:gold", false, false),
            "gold_layer_1"
        );
        assert_eq!(
            armor_layer_name("minecraft:chainmail", true, false),
            "chainmail_layer_2"
        );
        assert_eq!(
            armor_layer_name("minecraft:turtle_scute", false, false),
            "turtle_layer_1"
        );
        assert_eq!(
            armor_layer_name("minecraft:netherite", true, false),
            "netherite_layer_2"
        );
    }

    #[test]
    fn a_named_layer_wins_over_the_generic_texture() {
        let r = rule(
            "assets/minecraft/optifine/cit/armor/blue.properties",
            "type=armor\nitems=diamond_chestplate diamond_leggings\ntexture=generic\n\
             texture.diamond_layer_2=blue_legs\n",
        );
        let dir = "assets/minecraft/optifine/cit/armor";
        assert_eq!(
            armor_texture(&r, "diamond_layer_2"),
            Some(format!("{dir}/blue_legs.png").as_str())
        );
        assert_eq!(
            armor_texture(&r, "diamond_layer_1"),
            Some(format!("{dir}/generic.png").as_str())
        );
        assert_eq!(
            layer_sprite(&r, "minecraft:diamond", true, false).as_deref(),
            Some("horizon_cit:minecraft/optifine/cit/armor/blue_legs")
        );
        assert_eq!(
            layer_sprite(&r, "minecraft:diamond", false, false).as_deref(),
            Some("horizon_cit:minecraft/optifine/cit/armor/generic")
        );
    }

    #[test]
    fn without_a_generic_texture_other_layers_stay_vanilla() {
        let r = rule(
            "assets/minecraft/optifine/cit/a.properties",
            "type=armor\nitems=leather_helmet\ntexture.leather_layer_1=dyed\n",
        );
        assert_eq!(
            layer_sprite(&r, "minecraft:leather", false, false).as_deref(),
            Some("horizon_cit:minecraft/optifine/cit/dyed")
        );
        assert_eq!(layer_sprite(&r, "minecraft:leather", false, true), None);
        assert_eq!(layer_sprite(&r, "minecraft:leather", true, false), None);
    }

    #[test]
    fn a_vanilla_texture_path_keeps_its_atlas_name() {
        let r = rule(
            "assets/minecraft/optifine/cit/wings.properties",
            "type=elytra\ntexture=minecraft:textures/entity/wings/dragon.png\n",
        );
        assert_eq!(elytra_sprite(&r).as_deref(), Some("entity/wings/dragon"));
    }

    #[test]
    fn worn_layers_pick_the_rules_texture_for_their_layer() {
        let r = rule(
            "assets/minecraft/optifine/cit/dye.properties",
            "type=armor\nitems=leather_chestplate leather_leggings\n\
             texture.leather_layer_1=body\ntexture.leather_layer_1_overlay=body_dye\n\
             texture.leather_layer_2=legs\n",
        );
        let cit = |p: &str| Some(format!("horizon_cit:minecraft/optifine/cit/{p}"));
        let pick = |kind: &str, vanilla: &str| {
            worn_layer_sprite(Some(&r), "minecraft:leather", kind, vanilla)
        };
        let body = "entity/equipment/humanoid/leather";
        assert_eq!(pick("humanoid", body), cit("body"));
        assert_eq!(
            pick("humanoid", "entity/equipment/humanoid/leather_overlay"),
            cit("body_dye")
        );
        assert_eq!(
            pick(
                "humanoid_leggings",
                "entity/equipment/humanoid_leggings/leather"
            ),
            cit("legs")
        );
        // No `texture.leather_layer_2_overlay` and no `texture`: the overlay stays vanilla.
        let legs_dye = "entity/equipment/humanoid_leggings/leather_overlay";
        assert_eq!(pick("humanoid_leggings", legs_dye), None);
        // No rule draws vanilla's.
        assert_eq!(
            worn_layer_sprite(None, "minecraft:leather", "humanoid", body),
            None
        );
    }

    #[test]
    fn wings_take_the_rules_texture() {
        let r = rule(
            "assets/minecraft/optifine/cit/wings.properties",
            "type=elytra\ntexture=wings\n",
        );
        assert_eq!(
            wings_sprite(Some(&r)).as_deref(),
            Some("horizon_cit:minecraft/optifine/cit/wings")
        );
        assert_eq!(wings_sprite(None), None);
    }

    #[test]
    fn only_armor_and_elytra_rules_reach_the_atlas() {
        let rules = [
            rule(
                "assets/minecraft/optifine/cit/sword.properties",
                "items=diamond_sword\ntexture=sword\n",
            ),
            rule(
                "assets/minecraft/optifine/cit/plate.properties",
                "type=armor\nitems=iron_chestplate iron_leggings\ntexture=plate\n\
                 texture.iron_layer_2=legs\n",
            ),
            rule(
                "assets/minecraft/optifine/cit/wings.properties",
                "type=elytra\ntexture=wings\n",
            ),
            rule(
                "assets/minecraft/optifine/cit/glint.properties",
                "type=enchantment\nenchantments=sharpness\ntexture=glint\n",
            ),
        ];
        let got: Vec<String> = armor_sprites(&rules).into_iter().collect();
        assert_eq!(
            got,
            [
                "horizon_cit:minecraft/optifine/cit/legs",
                "horizon_cit:minecraft/optifine/cit/plate",
                "horizon_cit:minecraft/optifine/cit/wings",
            ]
        );
    }

    /// An atlas lookup that knows `names`.
    fn atlas(names: &[&str]) -> impl Fn(&str) -> Option<String> {
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        move |s| names.iter().any(|n| n == s).then(|| s.to_owned())
    }

    #[test]
    fn the_atlas_lookup_falls_back_to_vanilla_and_the_cape() {
        let iron = rule(
            "assets/minecraft/optifine/cit/iron.properties",
            "type=armor\nitems=iron_chestplate\ntexture.iron_layer_1=plate",
        );
        let plate = "horizon_cit:minecraft/optifine/cit/plate";
        let vanilla = "entity/equipment/humanoid/iron";
        let layer = |rule: Option<&Rule>, names: &[&str]| {
            worn_layer(rule, "minecraft:iron", "humanoid", vanilla, atlas(names))
        };
        assert_eq!(
            layer(Some(&iron), &[plate, vanilla]).as_deref(),
            Some(plate)
        );
        // The rule's texture didn't reach the atlas, or there's no rule: vanilla's layer.
        assert_eq!(layer(Some(&iron), &[vanilla]).as_deref(), Some(vanilla));
        assert_eq!(layer(None, &[plate, vanilla]).as_deref(), Some(vanilla));
        let elytra = rule(
            "assets/minecraft/optifine/cit/w.properties",
            "type=elytra\ntexture=wings",
        );
        let w = "horizon_cit:minecraft/optifine/cit/wings";
        let wing = |rule: Option<&Rule>, names: &[&str]| wings(rule, Some("cape/x"), atlas(names));
        assert_eq!(wing(Some(&elytra), &[w, "cape/x"]).as_deref(), Some(w));
        assert_eq!(wing(Some(&elytra), &["cape/x"]).as_deref(), Some("cape/x"));
        assert_eq!(wings(None, None, atlas(&[w])), None);
    }
}
