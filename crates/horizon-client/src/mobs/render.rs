//! Mob rendering (`LivingEntityRenderer.submit` and the render layers):
//! variant/baby model and texture selection, setupRotations, per-species
//! scale, and extra layers (sheep wool, spider eyes, drowned/stray outer
//! layers, villager overlays, collars...).
use super::{
    Mob, MobData, ModelNormals, Part, Species,
    anim::{self, Poses},
    living::wrap,
};
use crate::mesher::Vertex;
use horizon_core::geometry::trig::Mth;
use std::f32::consts::PI;

/// A texture lookup into the shared atlas (name without namespace -> uv rect).
pub type Atlas<'a> = &'a dyn Fn(&str) -> Option<[f32; 4]>;

/// The overlay code of an enchanted armour layer's glint, past the ones
/// `gfx/entity_batch.rs` gives materials (13 is the boat water mask, which
/// draws depth only: armour with 13 vanished; 35 is text pulled toward the
/// camera). It marks the `ARMOR_ENTITY_GLINT_TEXTURING` sample: entity.wgsl
/// transforms the model UV and reads `enchanted_glint_armor`.
pub const ARMOR_GLINT: u32 = 36;

/// `RenderTypes.breezeWind`: the texture of the breeze's wind layer scrolls
/// by 0.02 of the texture per tick.
pub const BREEZE_WIND_SCROLL: f32 = 0.02;
/// `WindChargeRenderer.xOffset`: the wind charge's texture scrolls by 0.03 of
/// the texture per tick.
pub const WIND_CHARGE_SCROLL: f32 = 0.03;
/// The offset of a scrolling texture: `RenderTypes.breezeWind` takes the
/// texture's age times the speed, modulo one.
pub fn scroll_offset(age: f32, speed: f32) -> f32 {
    (age * speed) % 1.0
}
/// The vertex's U with the scroll: the texture matrix of
/// `RenderTypes.breezeWind` moves the texture under the model, so the U
/// moves by the offset and wraps around the texture.
fn scroll_u(u: f32, scroll: f32) -> f32 {
    if scroll == 0.0 { u } else { (u + scroll) % 1.0 }
}
/// How brightly the eyes render type's layers (spiders', endermen's and
/// phantoms' eyes, the breeze's, the creaking's, the copper golem's) shine in
/// HDR output, as `mesher::hdr_tier`.
const EYES_TIER: u8 = crate::mesher::BRIGHT;

/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`gfx::OIT_ENTITY`): improved transparency draws them in its stages.
fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        vertex.layer |= crate::gfx::OIT_ENTITY;
    }
}
/// Whether the species' model is built with `RenderTypes::entityTranslucent`
/// (`Model.renderType`, which `LivingEntityRenderer.getRenderType` uses for the
/// body): the allay, the piglins, the breeze, the vex, the player model and the
/// sulfur cube.
fn translucent_body(m: &Mob) -> bool {
    matches!(
        m.species(),
        Species::Allay
            | Species::Piglin
            | Species::PiglinBrute
            | Species::ZombifiedPiglin
            | Species::Breeze
            | Species::Vex
            | Species::SulfurCube
    ) || avatar(m)
}
/// Whether a layer's render type has an `OitPipelineSet`: `RenderTypes.eyes` and
/// `entityTranslucentEmissive` (the full bright layers), `breezeWind`, and
/// `entityTranslucent` (the slime's outer cube, the sulfur cube's inner cube
/// and the horse's markings).
fn translucent_layer(m: &Mob, layer: &Layer) -> bool {
    layer.full_bright
        || layer.model == "breeze#wind"
        || layer.model == "slime#outer"
        || matches!(m.species(), Species::SulfurCube)
        || layer.texture.starts_with("entity/horse/horse_markings")
}

/// One draw of a model with a texture.
pub struct Layer {
    pub model: String,
    pub texture: String,
    pub color: u32,
    pub full_bright: bool,
    /// `RenderTypes.breezeWind`: the texture scrolls with the entity's age.
    pub scroll: f32,
}
/// DyeColor.getTextureDiffuseColor (RGB).
pub const DYE: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];
pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
/// ABGR vertex colour from an RGB value.
pub fn abgr(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    0xFF00_0000 | b << 16 | g << 8 | r
}

/// `EquipmentLayerRenderer.getColorForLayer`: a dyeable layer takes the
/// stack's dye colour (`minecraft:dyed_color`), the colour it has when
/// undyed otherwise; a layer that is not dyeable is not tinted.
pub fn armor_tint(undyed: Option<u32>, stack: Option<&crate::items::ItemStack>) -> u32 {
    let Some(undyed) = undyed else {
        return 0xFFFF_FFFF;
    };
    let dyed = stack
        .and_then(|s| s.extra.as_ref())
        .and_then(|e| e.dyed_color)
        .map(|d| d as u32)
        .unwrap_or(undyed);
    abgr(dyed)
}
/// ColorLerper.Type.SHEEP colour: white is 0xE6E6E6, others 75% of the dye.
pub fn sheep_wool_rgb(color: u8) -> u32 {
    if color == 0 {
        return 0xE6E6E6;
    }
    let src = DYE[color as usize & 15];
    let f = |c: u32| ((c as f32 * 0.75).floor() as u32).min(255);
    f(src >> 16 & 255) << 16 | f(src >> 8 & 255) << 8 | f(src & 255)
}
/// `ColorLerper.getLerpedColor(ColorLerper.Type.SHEEP, ageInTicks)`: the 16
/// wool colours in DyeColor order, one per 25 ticks, lerped in between. The
/// colour a `jeb_` sheep's wool takes.
pub fn jeb_wool_rgb(age: f32) -> u32 {
    // ageInTicks is the tick count plus the partial tick: never negative.
    let i = age.floor();
    let ticks = i as u64;
    let j = ticks / 25;
    let f = ((ticks % 25) as f32 + (age - i)) / 25.0;
    let lo = sheep_wool_rgb((j % 16) as u8);
    let hi = sheep_wool_rgb(((j + 1) % 16) as u8);
    let mix = |shift: u32| {
        let (lo, hi) = ((lo >> shift & 255) as f32, (hi >> shift & 255) as f32);
        ((lo + (hi - lo) * f) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}
pub const VILLAGER_TYPES: [&str; 7] = [
    "plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga",
];
pub const PROFESSIONS: [&str; 15] = [
    "none",
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "nitwit",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];
fn baby_or(m: &Mob, adult: &str, baby: &str) -> String {
    if m.is_baby() && m.kind.layer(baby).is_some() {
        baby.to_owned()
    } else {
        adult.to_owned()
    }
}
fn climate(m: &Mob) -> &'static str {
    match m.extra.variant {
        1 => "warm",
        2 => "cold",
        _ => "temperate",
    }
}
/// `AvatarRenderer` entities: a mannequin draws exactly like a player —
/// the player model, its resolved skin and the same scale.
fn avatar(m: &Mob) -> bool {
    matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin")
}
/// The model layer key and texture for the mob's body.
pub fn body(m: &Mob) -> (String, String) {
    use Species as S;
    let baby = m.is_baby();
    let b = |s: &str| {
        if baby {
            format!("{s}_baby")
        } else {
            s.to_owned()
        }
    };
    if avatar(m) {
        // AvatarRenderer: PlayerModel's slim layer for a slim skin.
        return (
            if m.slim { "player_slim#main" } else { "main" }.into(),
            m.skin.clone().unwrap_or_else(|| m.kind.texture.clone()),
        );
    }
    match m.species() {
        S::Llama | S::TraderLlama => {
            let name = if m.species() == S::Llama {
                "llama"
            } else {
                "trader_llama"
            };
            (
                baby_or(m, &format!("{name}#main"), &format!("{name}_baby#main")),
                super::llama::texture(m),
            )
        }
        s if super::equine::is_equine(s) => {
            let name = super::equine::model_name(s);
            (
                baby_or(m, &format!("{name}#main"), &format!("{name}_baby#main")),
                super::equine::texture(m),
            )
        }
        S::Wolf => (
            baby_or(m, "wolf#main", "wolf_baby#main"),
            super::tame::texture(m),
        ),
        S::Panda => (
            baby_or(m, "panda#main", "panda_baby#main"),
            super::panda::texture(m),
        ),
        S::HappyGhast => (
            baby_or(m, "happy_ghast#main", "happy_ghast_baby#main"),
            if baby {
                "entity/ghast/happy_ghast_baby".into()
            } else {
                "entity/ghast/happy_ghast".into()
            },
        ),
        S::Nautilus | S::ZombieNautilus => (
            super::nautilus::model(m).into(),
            super::nautilus::texture(m),
        ),
        S::SulfurCube => super::sulfur_cube::body(m),
        S::Sniffer => (
            baby_or(m, "sniffer#main", "sniffer_baby#main"),
            if baby {
                "entity/sniffer/snifflet".into()
            } else {
                "entity/sniffer/sniffer".into()
            },
        ),
        S::Shulker => ("shulker#main".into(), super::shulker::texture(m)),
        S::Vex => ("vex#main".into(), super::vex::texture(m).into()),
        S::Wither => ("wither#main".into(), super::wither::texture(m).into()),
        S::Ghast => (
            "ghast#main".into(),
            if m.extra.flag {
                "entity/ghast/ghast_shooting".into()
            } else {
                "entity/ghast/ghast".into()
            },
        ),
        S::Cat => (
            baby_or(m, "cat#main", "cat_baby#main"),
            super::tame::cat_texture(m),
        ),
        S::Frog => (
            "frog#main".into(),
            format!("entity/frog/frog_{}", climate(m)),
        ),
        // RabbitRenderer.getTextureLocation: the Toast name tag, else the
        // variant (the killer bunny's id is 99).
        S::Rabbit => {
            let name = if m.custom_name.as_deref() == Some("Toast") {
                "toast"
            } else {
                match m.extra.variant {
                    1 => "white",
                    2 => "black",
                    3 => "white_splotched",
                    4 => "gold",
                    5 => "salt",
                    99 => "caerbannog",
                    _ => "brown",
                }
            };
            (
                baby_or(m, "rabbit#main", "rabbit_baby#main"),
                b(&format!("entity/rabbit/rabbit_{name}")),
            )
        }
        // ParrotRenderer.getVariantTexture.
        S::Parrot => {
            let name = ["red_blue", "blue", "green", "yellow_blue", "grey"]
                [usize::from(m.extra.variant).min(4)];
            (m.kind.model_key(), format!("entity/parrot/parrot_{name}"))
        }
        // AxolotlRenderer.getTextureLocation.
        S::Axolotl => {
            let name =
                ["lucy", "wild", "gold", "cyan", "blue"][usize::from(m.extra.variant).min(4)];
            (
                baby_or(m, "axolotl#main", "axolotl_baby#main"),
                b(&format!("entity/axolotl/axolotl_{name}")),
            )
        }
        // SalmonRenderer.submit: the model of the salmon's size.
        S::Salmon => {
            let model = match m.extra.variant {
                0 => "salmon_small#main",
                2 => "salmon_large#main",
                _ => "salmon#main",
            };
            (model.into(), "entity/fish/salmon".into())
        }
        // The baby models and textures of AgeableMobRenderer's other
        // subclasses.
        S::Goat => (
            baby_or(m, "goat#main", "goat_baby#main"),
            b(&m.kind.texture),
        ),
        S::PolarBear => (
            baby_or(m, "polar_bear#main", "polar_bear_baby#main"),
            b(&m.kind.texture),
        ),
        S::Ocelot => (
            baby_or(m, "ocelot#main", "ocelot_baby#main"),
            b(&m.kind.texture),
        ),
        S::Turtle => (
            baby_or(m, "turtle#main", "turtle_baby#main"),
            b(&m.kind.texture),
        ),
        S::Armadillo => (
            baby_or(m, "armadillo#main", "armadillo_baby#main"),
            b(&m.kind.texture),
        ),
        // FoxRenderer.getTextureLocation: snow variant, sleeping, baby.
        S::Fox => (
            baby_or(m, "fox#main", "fox_baby#main"),
            format!(
                "entity/fox/fox{}{}{}",
                if m.extra.variant == 0 { "" } else { "_snow" },
                if super::fox::flag(m, super::fox::SLEEPING) {
                    "_sleep"
                } else {
                    ""
                },
                if baby { "_baby" } else { "" },
            ),
        ),
        S::Pig => {
            let model = if baby {
                "pig_baby#main".into()
            } else if m.extra.variant == 2 {
                "cold_pig#main".into()
            } else {
                "pig#main".into()
            };
            (model, b(&format!("entity/pig/pig_{}", climate(m))))
        }
        S::Cow => {
            let model = match (m.extra.variant, baby) {
                (1, false) => "warm_cow#main",
                (1, true) => "warm_cow_baby#main",
                (2, false) => "cold_cow#main",
                (2, true) => "cold_cow_baby#main",
                (_, true) => "cow_baby#main",
                _ => "cow#main",
            };
            (model.into(), b(&format!("entity/cow/cow_{}", climate(m))))
        }
        S::Mooshroom => (
            baby_or(m, "mooshroom#main", "mooshroom_baby#main"),
            b(if m.extra.variant == 1 {
                "entity/cow/mooshroom_brown"
            } else {
                "entity/cow/mooshroom_red"
            }),
        ),
        S::Chicken => {
            let model = if baby {
                "chicken_baby#main"
            } else if m.extra.variant == 2 {
                "cold_chicken#main"
            } else {
                "chicken#main"
            };
            (
                model.into(),
                b(&format!("entity/chicken/chicken_{}", climate(m))),
            )
        }
        S::Sheep => (
            baby_or(m, "sheep#main", "sheep_baby#main"),
            b("entity/sheep/sheep"),
        ),
        S::Zombie => (
            baby_or(m, "zombie#main", "zombie_baby#main"),
            b("entity/zombie/zombie"),
        ),
        S::Husk => (
            baby_or(m, "husk#main", "husk_baby#main"),
            b("entity/zombie/husk"),
        ),
        S::Drowned => (
            baby_or(m, "drowned#main", "drowned_baby#main"),
            b("entity/zombie/drowned"),
        ),
        S::ZombieVillager => (
            baby_or(m, "zombie_villager#main", "zombie_villager_baby#main"),
            b("entity/zombie_villager/zombie_villager"),
        ),
        S::Villager => (
            baby_or(m, "villager#main", "villager_baby#main"),
            b("entity/villager/villager"),
        ),
        S::Piglin => (
            baby_or(m, "piglin#main", "piglin_baby#main"),
            b("entity/piglin/piglin"),
        ),
        S::ZombifiedPiglin => (
            baby_or(m, "zombified_piglin#main", "zombified_piglin_baby#main"),
            b("entity/piglin/zombified_piglin"),
        ),
        S::Hoglin => (
            baby_or(m, "hoglin#main", "hoglin_baby#main"),
            b("entity/hoglin/hoglin"),
        ),
        S::Zoglin => (
            baby_or(m, "zoglin#main", "zoglin_baby#main"),
            b("entity/hoglin/zoglin"),
        ),
        S::Strider => (
            baby_or(m, "strider#main", "strider_baby#main"),
            b(if m.extra.flag {
                "entity/strider/strider_cold"
            } else {
                "entity/strider/strider"
            }),
        ),
        // BeeRenderer.getTextureLocation: angry, carrying nectar, or both.
        S::Bee => {
            let angry = super::bee::get(m).is_some_and(|b| b.anger_end > 0);
            let nectar = super::bee::has_nectar(m);
            let texture = match (angry, nectar) {
                (true, true) => "entity/bee/bee_angry_nectar",
                (true, false) => "entity/bee/bee_angry",
                (false, true) => "entity/bee/bee_nectar",
                (false, false) => "entity/bee/bee",
            };
            let layer = baby_or(m, "bee#main", "bee_baby#main");
            let texture = if layer.starts_with("bee_baby") {
                b(texture)
            } else {
                texture.to_owned()
            };
            (layer, texture)
        }
        S::Squid => (
            baby_or(m, "squid#main", "squid_baby#main"),
            b("entity/squid/squid"),
        ),
        S::GlowSquid => (
            baby_or(m, "glow_squid#main", "glow_squid_baby#main"),
            b("entity/squid/glow_squid"),
        ),
        S::Dolphin => (
            baby_or(m, "dolphin#main", "dolphin_baby#main"),
            b("entity/dolphin/dolphin"),
        ),
        S::Pufferfish => {
            let model = match m.extra.size {
                0 => "pufferfish_small#main",
                1 => "pufferfish_medium#main",
                _ => "pufferfish_big#main",
            };
            (model.into(), "entity/fish/pufferfish".into())
        }
        S::TropicalFish => {
            if m.extra.variant < 6 {
                (
                    "tropical_fish_small#main".into(),
                    "entity/fish/tropical_a".into(),
                )
            } else {
                (
                    "tropical_fish_large#main".into(),
                    "entity/fish/tropical_b".into(),
                )
            }
        }
        // CopperGolemOxidationLevels: the texture of the weathering.
        S::CopperGolem => (m.kind.model_key(), super::copper_golem::texture(m)),
        _ => (m.kind.model_key(), m.kind.texture.clone()),
    }
}
/// LivingEntityRenderer.getModelTint (tropical fish base colour).
pub fn body_color(m: &Mob, alpha: f32) -> u32 {
    match m.species() {
        Species::Wolf => super::tame::tint(m, alpha),
        Species::TropicalFish => abgr(DYE[m.extra.variant2 as usize & 15]),
        // LivingEntityRenderer.submit: a spectator is invisible
        // (Player.updateInvisibilityStatus) but not to another spectator or
        // itself, so its body is forced translucent (654311423, alpha 38).
        _ if avatar(m) && m.extra.spectator => 0x26FF_FFFF,
        _ => 0xFFFF_FFFF,
    }
}
/// Extra render layers drawn over the body with the same pose. `age` is the
/// render state's `ageInTicks` (`Mob.age` plus the partial tick).
pub fn layers(m: &Mob, age: f32) -> Vec<Layer> {
    use Species as S;
    let mut v = Vec::new();
    if avatar(m) {
        return v;
    }
    let white = 0xFFFF_FFFF;
    let push = |v: &mut Vec<Layer>, model: &str, texture: &str, color: u32, full_bright: bool| {
        v.push(Layer {
            model: model.into(),
            texture: texture.into(),
            color,
            full_bright,
            scroll: 0.0,
        })
    };
    match m.species() {
        // WolfArmorLayer, then WolfCollarLayer.
        S::Wolf if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("wolf_baby#main", "entity/wolf/wolf_collar_baby")
            } else {
                ("wolf#main", "entity/wolf/wolf_collar")
            };
            // The armor, then the cracks of its damage (`Crackiness.WOLF_ARMOR`).
            if !m.is_baby() && m.extra.head.as_deref() == Some("minecraft:wolf_armor") {
                let left = (super::WOLF_ARMOR_DURABILITY - m.extra.body_damage) as f32
                    / super::WOLF_ARMOR_DURABILITY as f32;
                push(
                    &mut v,
                    "wolf_armor#main",
                    "entity/equipment/wolf_body/armadillo_scute",
                    white,
                    false,
                );
                let crack = [(0.32, "high"), (0.69, "medium"), (0.95, "low")]
                    .iter()
                    .find(|(t, _)| left < *t);
                if let Some((_, level)) = crack {
                    push(
                        &mut v,
                        "wolf_armor#main",
                        &format!("entity/wolf/wolf_armor_crackiness_{level}"),
                        white,
                        false,
                    );
                }
            }
            push(
                &mut v,
                model,
                texture,
                abgr(DYE[m.extra.color as usize & 15]),
                false,
            );
        }
        // SimpleEquipmentLayer: the harness over the body.
        S::HappyGhast => {
            let harness = m
                .extra
                .head
                .as_deref()
                .and_then(super::happy_ghast::harness_texture);
            if !m.is_baby()
                && let Some(texture) = &harness
            {
                push(&mut v, "happy_ghast_harness#main", texture, white, false);
            }
            // RopesLayer: the ropes over a harnessed body while the ghast
            // holds a quad lead (`isLeashHolder`), the baby's model for a baby.
            if harness.is_some() && super::happy_ghast::is_leash_holder(m) {
                let model = if m.is_baby() {
                    "happy_ghast_baby_ropes#main"
                } else {
                    "happy_ghast_ropes#main"
                };
                push(
                    &mut v,
                    model,
                    "entity/ghast/happy_ghast_ropes",
                    white,
                    false,
                );
            }
        }
        // BreezeWindLayer and BreezeEyesLayer. The wind layer's texture
        // scrolls (`RenderTypes.breezeWind`): the offset is taken inside the
        // sprite, so the atlas does not have to wrap.
        S::Breeze => {
            v.push(Layer {
                model: "breeze#wind".into(),
                texture: "entity/breeze/breeze_wind".into(),
                color: white,
                full_bright: false,
                scroll: scroll_offset(age, BREEZE_WIND_SCROLL),
            });
            push(
                &mut v,
                "breeze#eyes",
                "entity/breeze/breeze_eyes",
                white,
                true,
            );
        }
        // LlamaDecorLayer: the carpet, or the trader's blanket.
        S::Llama | S::TraderLlama => {
            if let Some((model, texture)) = super::llama::decor(m) {
                push(&mut v, &model, &texture, white, false);
            }
        }
        // HorseMarkingLayer, the saddle and the horse armour.
        s if super::equine::is_equine(s) => {
            let name = super::equine::model_name(s);
            let body = if m.is_baby() {
                format!("{name}_baby#main")
            } else {
                format!("{name}#main")
            };
            if let Some(tex) = super::equine::markings(m) {
                push(&mut v, &body, &tex, white, false);
            }
            if m.extra.saddled && !m.is_baby() {
                push(
                    &mut v,
                    &format!("{name}#saddle"),
                    super::equine::saddle_texture(s),
                    white,
                    false,
                );
            }
            if let Some(armor) = m.extra.head.as_deref()
                && let Some(material) = armor
                    .strip_prefix("minecraft:")
                    .and_then(|a| a.strip_suffix("_horse_armor"))
            {
                let tex = format!("entity/equipment/horse_body/{material}");
                let color = if material == "leather" {
                    abgr(0xA06540)
                } else {
                    white
                };
                // UndeadHorseRenderer bakes its own armour layer.
                let layer = if matches!(s, S::ZombieHorse | S::SkeletonHorse) {
                    "undead_horse_armor#main"
                } else {
                    "horse_armor#main"
                };
                push(&mut v, layer, &tex, color, false);
            }
        }
        // SimpleEquipmentLayer: pig and strider saddles.
        S::Pig if m.extra.saddled && !m.is_baby() => push(
            &mut v,
            "pig#saddle",
            "entity/equipment/pig_saddle/saddle",
            white,
            false,
        ),
        S::Strider if m.extra.saddled && !m.is_baby() => push(
            &mut v,
            "strider#saddle",
            "entity/equipment/strider_saddle/saddle",
            white,
            false,
        ),
        // CatCollarLayer.
        S::Cat if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("cat_baby#collar", "entity/cat/cat_collar_baby")
            } else {
                ("cat#collar", "entity/cat/cat_collar")
            };
            push(
                &mut v,
                model,
                texture,
                abgr(DYE[m.extra.color as usize & 15]),
                false,
            );
        }
        // IronGolemCrackinessLayer.
        S::IronGolem => {
            if let Some(texture) = super::golem::crackiness(m) {
                push(&mut v, "iron_golem#main", texture, white, false);
            }
        }
        // NautilusRenderer: the armour and saddle SimpleEquipmentLayers.
        S::Nautilus | S::ZombieNautilus if !m.is_baby() => {
            if let Some(texture) = m
                .extra
                .head
                .as_deref()
                .and_then(super::nautilus::armor_texture)
            {
                push(&mut v, "nautilus_armor#main", &texture, white, false);
            }
            if m.extra.saddled {
                push(
                    &mut v,
                    "nautilus#saddle",
                    "entity/equipment/nautilus_saddle/saddle",
                    white,
                    false,
                );
            }
        }
        // SulfurCubeInnerLayer.
        S::SulfurCube => {
            if let Some((model, texture)) = super::sulfur_cube::inner(m) {
                push(&mut v, model, texture, white, false);
            }
        }
        // SlimeOuterLayer: the translucent outer cube.
        S::Slime => push(&mut v, "slime#outer", "entity/slime/slime", white, false),
        // SheepWoolUndercoatLayer has no isSheared guard: a sheared dyed sheep
        // keeps it, while only SheepWoolLayer stops.
        S::Sheep => {
            // MobRenderer.checkMagicName(entity, "jeb_"): an exact custom name.
            let jeb = m.custom_name.as_deref() == Some("jeb_");
            let color = abgr(if jeb {
                jeb_wool_rgb(age)
            } else {
                sheep_wool_rgb(m.extra.color)
            });
            if !m.is_baby() && (jeb || m.extra.color != 0) {
                push(
                    &mut v,
                    "sheep#wool_undercoat",
                    "entity/sheep/sheep_wool_undercoat",
                    color,
                    false,
                );
            }
            // SheepWoolLayer: none once sheared.
            if !m.extra.sheared {
                let (model, texture) = if m.is_baby() {
                    ("sheep_baby#wool", "entity/sheep/sheep_wool_baby")
                } else {
                    ("sheep#wool", "entity/sheep/sheep_wool")
                };
                push(&mut v, model, texture, color, false);
            }
        }
        S::Creaking if super::creaking::eyes_glowing(m) => push(
            &mut v,
            "creaking#eyes",
            "entity/creaking/creaking_eyes",
            white,
            true,
        ),
        // The copper golem's eyes glow: CopperGolemRenderer's
        // LivingEntityEmissiveLayer (eyes render type) draws on a model baked
        // from ModelLayers.COPPER_GOLEM, the body's, not COPPER_GOLEM_EYES.
        S::CopperGolem => {
            let eyes = super::copper_golem::eye_texture(m);
            push(&mut v, &m.kind.model_key(), &eyes, white, true);
        }
        S::Spider | S::CaveSpider => push(
            &mut v,
            &m.kind.model_key(),
            "entity/spider/spider_eyes",
            white,
            true,
        ),
        S::Enderman => push(
            &mut v,
            "enderman#main",
            "entity/enderman/enderman_eyes",
            white,
            true,
        ),
        S::Phantom => push(
            &mut v,
            "phantom#main",
            "entity/phantom/phantom_eyes",
            white,
            true,
        ),
        S::Drowned => {
            if m.is_baby() {
                push(
                    &mut v,
                    "drowned_baby#outer",
                    "entity/zombie/drowned_outer_layer_baby",
                    white,
                    false,
                );
            } else {
                push(
                    &mut v,
                    "drowned#outer",
                    "entity/zombie/drowned_outer_layer",
                    white,
                    false,
                );
            }
        }
        S::Stray => push(
            &mut v,
            "stray#outer",
            "entity/skeleton/stray_overlay",
            white,
            false,
        ),
        S::Bogged if !m.extra.sheared => push(
            &mut v,
            "bogged#outer",
            "entity/skeleton/bogged_overlay",
            white,
            false,
        ),
        S::TropicalFish => {
            // TropicalFishPatternLayer: the pattern tinted with the pattern colour.
            let (model, letter, n) = if m.extra.variant < 6 {
                ("tropical_fish_small#pattern", "a", m.extra.variant + 1)
            } else {
                ("tropical_fish_large#pattern", "b", m.extra.variant - 5)
            };
            push(
                &mut v,
                model,
                &format!("entity/fish/tropical_{letter}_pattern_{n}"),
                abgr(DYE[m.extra.variant3 as usize & 15]),
                false,
            );
        }
        S::Villager | S::ZombieVillager => {
            let base = if m.species() == S::Villager {
                "villager"
            } else {
                "zombie_villager"
            };
            let model = if m.is_baby() {
                format!("{base}_baby#main")
            } else {
                format!("{base}#main")
            };
            let t = VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()];
            let folder = if m.species() == S::Villager {
                "entity/villager"
            } else {
                "entity/zombie_villager"
            };
            // VillagerProfessionLayer: a baby's type texture is in `baby/`.
            let group = if m.is_baby() { "baby" } else { "type" };
            push(
                &mut v,
                &model,
                &format!("{folder}/{group}/{t}"),
                white,
                false,
            );
            let prof = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
            if !m.is_baby() && prof != "none" {
                push(
                    &mut v,
                    &model,
                    &format!("{folder}/profession/{prof}"),
                    white,
                    false,
                );
                if prof != "nitwit" {
                    // `LEVEL_LOCATIONS` clamps the level to 1..=5.
                    let level = ["stone", "iron", "gold", "emerald", "diamond"]
                        [(m.extra.variant3.max(1) as usize - 1).min(4)];
                    push(
                        &mut v,
                        &model,
                        &format!("{folder}/profession_level/{level}"),
                        white,
                        false,
                    );
                }
            }
        }
        _ => {}
    }
    humanoid_armor(m, &mut v);
    v
}
/// `HumanoidArmorLayer`: the armour models of the mob's main model with the
/// equipment textures (leather tinted, with its overlay).
fn humanoid_armor(m: &Mob, v: &mut Vec<Layer>) {
    // A player's or mannequin's armour is drawn by the game's armour layers.
    if super::equine::is_equine(m.species()) || avatar(m) {
        return;
    }
    let (model, _) = body(m);
    let Some(base) = model.strip_suffix("#main") else {
        return;
    };
    let slots = [
        (&m.extra.head, &m.extra.armor[0], "helmet"),
        (&m.extra.chest, &m.extra.armor[1], "chestplate"),
        (&m.extra.legs, &m.extra.armor[2], "leggings"),
        (&m.extra.feet, &m.extra.armor[3], "boots"),
    ];
    for (item, stack, layer) in slots {
        let Some(name) = item.as_deref().and_then(|i| i.strip_prefix("minecraft:")) else {
            continue;
        };
        let Some(material) = name.strip_suffix(&format!("_{layer}")) else {
            continue;
        };
        let asset = match material {
            "golden" => "gold",
            "turtle" => "turtle_scute",
            other => other,
        };
        let folder = if layer == "leggings" {
            "humanoid_leggings"
        } else {
            "humanoid"
        };
        let model = format!("{base}#{layer}");
        // EquipmentLayerRenderer.getColorForLayer: the leather layer takes
        // the stack's dye colour, the colour it has when undyed otherwise.
        let tint = if asset == "leather" {
            armor_tint(Some(0xA06540), stack.as_ref())
        } else {
            0xFFFF_FFFF
        };
        v.push(Layer {
            model: model.clone(),
            texture: format!("entity/equipment/{folder}/{asset}"),
            color: tint,
            full_bright: false,
            scroll: 0.0,
        });
        if asset == "leather" {
            v.push(Layer {
                model,
                texture: format!("entity/equipment/{folder}/leather_overlay"),
                color: 0xFFFF_FFFF,
                full_bright: false,
                scroll: 0.0,
            });
        }
    }
}
impl super::MobType {
    /// The main model layer key without the namespace.
    pub fn model_key(&self) -> String {
        super::species::def(self.species).model.to_owned()
    }
}
/// The mob's root transform (LivingEntityRenderer.submit + setupRotations).
pub fn base_transform(m: &Mob, alpha: f32, camera: glam::DVec3) -> glam::Mat4 {
    let a = f64::from(alpha);
    let p = glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ) - camera
        + enderman_render_offset(m, alpha);
    base_transform_at(m, alpha, glam::Mat4::from_translation(p.as_vec3()))
}
/// `EndermanRenderer.getRenderOffset`: a creepy (angry) enderman is moved by
/// a gaussian 0.02 blocks sideways in x and z every frame. Vanilla draws from
/// the renderer's own random; this one is seeded by the mob and the frame's
/// age, so the body, its carried block and its eyes agree within a frame.
fn enderman_render_offset(m: &Mob, alpha: f32) -> glam::DVec3 {
    use horizon_core::random::{LegacyRandom, RandomSource};
    if m.species() != Species::Enderman || !m.extra.flag {
        return glam::DVec3::ZERO;
    }
    let frame = (m.age as f32 + alpha).to_bits();
    let mut random = LegacyRandom::new((i64::from(m.id) << 32) ^ i64::from(frame));
    // `0.02 * state.scale`, the enderman's scale being 1.
    let d = 0.02;
    let x = random.next_gaussian() * d;
    let z = random.next_gaussian() * d;
    glam::DVec3::new(x, 0.0, z)
}
/// `base_transform` from a pose of the entity's origin (`EntityRenderDispatcher.submit`
/// under a pose stack, as `SpawnerRenderer` draws the mob inside a spawner).
pub fn base_transform_at(m: &Mob, alpha: f32, origin: glam::Mat4) -> glam::Mat4 {
    let mut body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha;
    if shaking(m) {
        // LivingEntityRenderer.setupRotations: a rapid wobble.
        let ticks = (m.age as f32 + alpha).floor();
        body += (f64::from(ticks * 3.25).cos() * std::f64::consts::PI * 0.4) as f32;
    }
    // A sleeping villager, or another player lying in their bed.
    let sleep = (m.species() == Species::Villager || avatar(m))
        .then(|| super::villager::sleep_pose(m))
        .flatten();
    let rotation = match sleep {
        // setupRotations for Pose.SLEEPING, with the head moved toward the pillow.
        Some((angle, step)) => {
            // The eye height of the standing pose (LivingEntityRenderer
            // extracts `getEyeHeight(Pose.STANDING)` for a bed).
            let eye = if avatar(m) {
                m.kind.eye_height as f32
            } else {
                m.awake_dims().2 as f32
            };
            let head = eye - 0.1;
            glam::Mat4::from_translation(glam::Vec3::new(-step[0] * head, 0.0, -step[1] * head))
                * glam::Mat4::from_rotation_y(angle.to_radians())
                * glam::Mat4::from_rotation_z(90.0_f32.to_radians())
                * glam::Mat4::from_rotation_y(270.0_f32.to_radians())
        }
        None => super::aquatic::rotations(m, alpha, body)
            .unwrap_or_else(|| glam::Mat4::from_rotation_y((180.0 - body).to_radians())),
    };
    // PufferfishRenderer.setupRotations: bobs before LivingEntityRenderer's.
    let mut base = origin * pufferfish_bob(m, alpha) * rotation;
    if m.death_time > 0 && !super::creaking::tearing_down(m) {
        let f = ((m.death_time as f32 + alpha - 1.0) / 20.0 * 1.6)
            .sqrt()
            .min(1.0);
        base *= glam::Mat4::from_rotation_z(f * flip_degrees(m).to_radians());
    } else if m.extra.auto_spin_attack {
        // Riptide: lying along the look direction, spinning 75 degrees a tick.
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        base *= glam::Mat4::from_rotation_x((-90.0 - pitch).to_radians())
            * glam::Mat4::from_rotation_y(((m.age as f32 + alpha) * -75.0).to_radians());
    } else if matches!(m.custom_name.as_deref(), Some("Dinnerbone" | "Grumm")) {
        // isEntityUpsideDown
        base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, m.height() + 0.1, 0.0))
            * glam::Mat4::from_rotation_z(PI);
    }
    // IronGolemRenderer.setupRotations, after LivingEntityRenderer's: the body
    // tilts side to side while it walks.
    if m.species() == Species::IronGolem {
        let s = anim::state(m, alpha);
        if s.speed >= 0.01 {
            let wp = s.pos + 6.0;
            let triangle_wave = ((wp % 13.0 - 6.5).abs() - 3.25) / 3.25;
            base *= glam::Mat4::from_rotation_z((6.5 * triangle_wave).to_radians());
        }
    }
    // AvatarRenderer.setupRotations, after LivingEntityRenderer's.
    if m.extra.fall_flying {
        let ticks = m.extra.fall_fly_ticks as f32 + alpha;
        let scale = (ticks * ticks / 100.0).min(1.0);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        if !m.extra.auto_spin_attack {
            base *= glam::Mat4::from_rotation_x((scale * (-90.0 - pitch)).to_radians());
        }
        let yaw = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
        let (look_x, look_z) = (-(yaw.to_radians()).msin(), (yaw.to_radians()).mcos());
        let v = m.extra.previous_velocity.lerp(m.velocity, f64::from(alpha));
        let view_scale = (pitch.to_radians()).mcos();
        let (look_x, look_z) = (look_x * view_scale, look_z * view_scale);
        let view_length = f64::from(look_x * look_x + look_z * look_z);
        let speed = v.x * v.x + v.z * v.z;
        if speed > f64::from(1.0e-5f32) && view_length > f64::from(1.0e-5f32) {
            let dot =
                (v.x * f64::from(look_x) + v.z * f64::from(look_z)) / (speed * view_length).sqrt();
            let sign = (v.x * f64::from(look_z) - v.z * f64::from(look_x)).signum();
            base *= glam::Mat4::from_rotation_y(
                (sign * horizon_core::jmath::acos(dot.abs().min(1.0))) as f32,
            );
        }
    } else if m.species() == Species::Drowned {
        // DrownedRenderer.setupRotations: tip toward the look pitch about
        // the middle of the box while swimming.
        let amount = m.extra.swim.amount(alpha);
        if amount > 0.0 {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            let middle = glam::Vec3::new(0.0, m.height() / 2.0, 0.0);
            base *= glam::Mat4::from_translation(middle)
                * glam::Mat4::from_rotation_x((amount * (-10.0 - pitch)).to_radians())
                * glam::Mat4::from_translation(-middle);
        }
    } else if m.extra.swim.amount(alpha) > 0.0 {
        // AvatarRenderer.setupRotations: tip forward with the swim amount,
        // following the pitch in water, then lie along the swimming box.
        let amount = m.extra.swim.amount(alpha);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        let target = if m.extra.swim.in_water {
            -90.0 - pitch
        } else {
            -90.0
        };
        base *= glam::Mat4::from_rotation_x((amount * target).to_radians());
        if m.extra.swim.visually {
            base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, 0.3));
        }
    }
    base *= species_rotations(m, alpha);
    base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
    base *= species_scale(m, alpha);
    base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0))
}
/// `PufferfishRenderer.setupRotations`: `translate(0, cos(ageInTicks * 0.05) * 0.08, 0)`
/// before the usual rotations, so a pufferfish bobs up and down.
fn pufferfish_bob(m: &Mob, alpha: f32) -> glam::Mat4 {
    if m.species() != Species::Pufferfish {
        return glam::Mat4::IDENTITY;
    }
    glam::Mat4::from_translation(glam::Vec3::new(
        0.0,
        pufferfish_bob_y(m.age as f32 + alpha),
        0.0,
    ))
}
fn pufferfish_bob_y(age_in_ticks: f32) -> f32 {
    (age_in_ticks * 0.05).mcos() * 0.08
}
/// `LivingEntityRenderer.isShaking`: a fully frozen mob (the skeleton's own
/// `isShaking` is its stray conversion only), converting zombies, piglins
/// and hoglins, a zombie villager being cured and a cold strider.
fn shaking(m: &Mob) -> bool {
    if m.is_fully_frozen() && m.species() != Species::Skeleton {
        return true;
    }
    match m.species() {
        Species::Strider => super::strider::suffocating(m),
        Species::ZombieVillager => super::zombie_villager::is_converting(m),
        Species::Zombie | Species::Husk | Species::Skeleton => m.extra.counter2 > 0,
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => {
            !m.extra.flag2 && m.extra.counter2 > 0
        }
        _ => false,
    }
}
fn flip_degrees(m: &Mob) -> f32 {
    match m.species() {
        Species::Spider | Species::CaveSpider | Species::Silverfish | Species::Endermite => 180.0,
        _ => 90.0,
    }
}
fn species_rotations(m: &Mob, alpha: f32) -> glam::Mat4 {
    match m.species() {
        Species::Cat => super::tame::cat_rotations(m, alpha),
        Species::Panda => super::panda::rotations(m, alpha),
        Species::Shulker => super::shulker::rotations(m),
        // FoxRenderer.setupRotations: the body follows a pounce's pitch.
        Species::Fox if super::fox::flag(m, super::fox::POUNCING | super::fox::FACEPLANTED) => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x((-pitch).to_radians())
        }
        // PhantomRenderer.setupRotations: the body follows its pitch.
        Species::Phantom => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x(pitch.to_radians())
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Creeper.getSwelling: the fuse's progress, past 1 in its last two ticks.
fn creeper_swelling(m: &Mob, alpha: f32) -> f32 {
    (m.previous_swell as f32 + alpha * (m.swell - m.previous_swell) as f32) / 28.0
}
/// LivingEntityRenderer.getOverlayCoords for the body: the hurt red, or a
/// swelling creeper's white flash. Layers take the red only (progress 0).
fn body_overlay(m: &Mob, alpha: f32) -> u32 {
    let white = if m.species() == Species::Creeper {
        crate::overlay::creeper_progress(creeper_swelling(m, alpha))
    } else {
        0.0
    };
    crate::overlay::living(white, red_overlay(m))
}
/// LivingEntityRenderState.hasRedOverlay: hurt or dying (not a creaking
/// tearing down).
fn red_overlay(m: &Mob) -> bool {
    m.hurt_time > 0 || (m.death_time > 0 && !super::creaking::tearing_down(m))
}
fn species_scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    if avatar(m) {
        // AvatarRenderer.scale, shared by the inventory and world renderer, on
        // top of the entity's own `scale` (`LivingEntityRenderer.submit`).
        let own = if m.extra.model_scale > 0.0 {
            m.extra.model_scale
        } else {
            1.0
        };
        return glam::Mat4::from_scale(glam::Vec3::splat(0.9375 * own));
    }
    match m.species() {
        Species::Wither => {
            glam::Mat4::from_scale(glam::Vec3::splat(super::wither::scale(m, alpha)))
        }
        Species::Creeper => {
            // CreeperRenderer.scale: swelling with a wobble.
            let g = creeper_swelling(m, alpha);
            let wobble = 1.0 + (g * 100.0).msin() * g * 0.01;
            let s = g.clamp(0.0, 1.0);
            let s4 = s * s * s * s;
            let xz = (1.0 + s4 * 0.4) * wobble;
            let y = (1.0 + s4 * 0.1) / wobble;
            glam::Mat4::from_scale(glam::Vec3::new(xz, y, xz))
        }
        Species::Slime | Species::MagmaCube => super::cube::scale(m, alpha),
        Species::SulfurCube => super::sulfur_cube::scale(m, alpha),
        Species::Phantom => {
            // PhantomRenderer.scale: 15% bigger per size, and the model moved
            // down to the box (it is modelled 1.3125 blocks above the feet).
            let s = 1.0 + 0.15 * m.extra.size as f32;
            glam::Mat4::from_scale(glam::Vec3::splat(s))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.3125, 0.1875))
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Draw a part tree with a pose map, texture rect, colour and light.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    draw_scrolled(part, base, poses, uv, 0.0, color, light, overlay, out);
}
/// `draw` for a texture that scrolls: the vertex's U is moved by the offset
/// and wraps inside the texture, as the texture matrix of
/// `RenderTypes.breezeWind` does.
#[allow(clippy::too_many_arguments)]
pub fn draw_scrolled(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    draw_normals(
        part,
        base,
        ModelNormals::new(base),
        poses,
        uv,
        scroll,
        color,
        light,
        overlay,
        out,
    );
}
#[allow(clippy::too_many_arguments)]
fn draw_normals(
    part: &Part,
    base: glam::Mat4,
    normals: ModelNormals,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let p = poses.pose(part);
    let m = base
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
        return;
    }
    let normals = normals.part(p);
    // `skips` is a name lookup: ask once per part, not once per polygon. A part
    // hidden this way still draws its children, so only the polygons are gated.
    if !poses.skips(part) {
        for poly in &part.polygons {
            let normal = poly.vertex_normal(normals);
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                // The armour glint scrolls the layer's model UV in the shader;
                // the item path already passed its transformed UV.
                let glint_uv = if overlay == ARMOR_GLINT {
                    [v[3], v[4]]
                } else {
                    [0.0, 0.0]
                };
                out.push(Vertex {
                    normal,
                    pos: pos.to_array(),
                    uv: [
                        uv[0] + (uv[2] - uv[0]) * scroll_u(v[3], scroll),
                        uv[1] + (uv[3] - uv[1]) * v[4],
                    ],
                    layer: overlay,
                    color,
                    light,
                    glint_uv,
                });
            }
        }
    }
    for c in &part.children {
        draw_normals(c, m, normals, poses, uv, scroll, color, light, overlay, out);
    }
}
/// Render a mob with its variant model, texture and layers.
pub fn render(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    render_with(m, alpha, camera, atlas, light, data, &[], out);
}
/// `render` plus extra layers posed like the body (`HumanoidArmorLayer`:
/// the armour model's parts copy the body's): (model, texture rect, colour,
/// overlay: 0 or `ARMOR_GLINT`, as `EquipmentLayerRenderer` never tints them).
#[allow(clippy::too_many_arguments)]
pub fn render_with(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    extra: &[(&Part, [f32; 4], u32, u32)],
    out: &mut Vec<Vertex>,
) {
    // The invisibility effect hides the model (a drinking wandering trader);
    // an illusioner shows its four mirror images instead.
    if super::trader::is_invisible(m) {
        if m.species() == Species::Illusioner {
            let age = m.age as f32 + alpha;
            for (i, off) in super::illusioner::offsets(m, alpha).iter().enumerate() {
                let w = super::illusioner::wobble(i, age);
                let shifted =
                    camera - glam::DVec3::new(off[0] + w[0], off[1] + w[1], off[2] + w[2]);
                let base = base_transform(m, alpha, shifted);
                render_visible(m, alpha, base, atlas, light, data, extra, true, out);
            }
        }
        return;
    }
    render_visible(
        m,
        alpha,
        base_transform(m, alpha, camera),
        atlas,
        light,
        data,
        extra,
        true,
        out,
    );
}
/// `CapeLayer.submit` with `PlayerCapeModel`: the player model's body posed
/// as the player's, its cape turned by `ModelPart.rotateBy` with the cape's
/// flap and leans. `base` is the player's root transform (moved out over a
/// chestplate); `uv` the cape texture.
#[allow(clippy::too_many_arguments)]
pub fn cape(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    let Some(model) = m.kind.layer("player#cape") else {
        return;
    };
    let s = anim::state(m, alpha);
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let a = f64::from(alpha);
    let at = [
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ];
    let body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha;
    // LivingEntityRenderState.fallFlyingScale.
    let ticks = m.extra.fall_fly_ticks as f32 + alpha;
    let gliding = (ticks * ticks / 100.0).clamp(0.0, 1.0);
    let [flap, lean, lean2] = m.extra.avatar.cape(alpha, at, body, gliding);
    let pose = poses.get("cape");
    let by = glam::Quat::from_rotation_y(-PI)
        * glam::Quat::from_rotation_x((6.0 + lean / 2.0 + flap).to_radians())
        * glam::Quat::from_rotation_z((lean2 / 2.0).to_radians())
        * glam::Quat::from_rotation_y((180.0 - lean2 / 2.0).to_radians());
    let now = glam::Quat::from_euler(glam::EulerRot::ZYX, pose[5], pose[4], pose[3]);
    let (z, y, x) = (now * by).to_euler(glam::EulerRot::ZYX);
    (pose[3], pose[4], pose[5]) = (x, y, z);
    // RenderTypes.entitySolid with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `Deadmau5EarsLayer.submit` with `PlayerEarsModel`: the player model's
/// head carrying two 6x6x1 ears (the model's other cubes are cleared),
/// drawn with the player's skin as `RenderTypes.entitySolid`. The caller has
/// checked `showExtraEars`; an invisible avatar draws no ears.
#[allow(clippy::too_many_arguments)]
pub fn ears(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    if super::trader::is_invisible(m) {
        return;
    }
    let Some(model) = m.kind.layer("player#ears") else {
        return;
    };
    let s = anim::state(m, alpha);
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &data.animations);
    // LivingEntityRenderer.getOverlayCoords(state, 0.0F).
    draw(
        model,
        base,
        &poses,
        uv,
        0xFFFF_FFFF,
        light,
        body_overlay(m, alpha),
        out,
    );
}
/// `PlayerModel.bodyParts`: what `getRandomBodyPart` picks from.
const BODY_PARTS: [&str; 6] = [
    "head",
    "body",
    "left_arm",
    "right_arm",
    "left_leg",
    "right_leg",
];

/// The cubes of a part as `ModelPart.cubes` has them: min and max corners in
/// model units. The part keeps six faces per cube, in cube order.
fn cube_bounds(part: &Part) -> Vec<[f32; 6]> {
    let size = if part.polygons.len().is_multiple_of(6) {
        6
    } else {
        part.polygons.len().max(1)
    };
    part.polygons
        .chunks(size)
        .map(|faces| {
            let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
            for v in faces.iter().flat_map(|f| f.vertices) {
                for k in 0..3 {
                    b[k] = b[k].min(v[k]);
                    b[3 + k] = b[3 + k].max(v[k]);
                }
            }
            b
        })
        .collect()
}

/// `StuckInBodyLayer.snapToFace`.
fn snap_to_face(value: f32) -> f32 {
    if value > 0.5 { 1.0 } else { 0.5 }
}

/// `StuckInBodyLayer.submit` (with `submitStuckItem`) for a player: `count`
/// copies of `item` (`ArrowLayer`'s arrow model, `BeeStingerLayer`'s stinger)
/// stuck in random cubes of the posed body, placed from a random source
/// seeded by the entity id `id`. `on_surface` is `PlacementStyle.ON_SURFACE`.
#[allow(clippy::too_many_arguments)]
pub fn stuck_in_body(
    m: &Mob,
    id: i32,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    count: i32,
    on_surface: bool,
    item: &Part,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    use horizon_core::random::{LegacyRandom, RandomSource};
    if count <= 0 {
        return;
    }
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let mut random = LegacyRandom::new(i64::from(id));
    for _ in 0..count {
        let name = BODY_PARTS[random.next_int_bound(BODY_PARTS.len() as i32).unwrap_or(0) as usize];
        let Some(part) = find_part(&model, name) else {
            return;
        };
        let cubes = cube_bounds(part);
        let Ok(index) = random.next_int_bound(cubes.len() as i32) else {
            return;
        };
        let cube = cubes[index as usize];
        let Some(mut t) = part_matrix(&model, base, &poses, name, 0.0) else {
            return;
        };
        let mut mid = [
            random.next_float(),
            random.next_float(),
            random.next_float(),
        ];
        if on_surface {
            let plane = random.next_int_bound(3).unwrap_or(0) as usize;
            if plane < 3 {
                mid[plane] = snap_to_face(mid[plane]);
            }
        }
        let lerp = |k: usize| cube[k] + mid[k] * (cube[3 + k] - cube[k]);
        t *= glam::Mat4::from_translation(glam::Vec3::new(lerp(0), lerp(1), lerp(2)) / 16.0);
        let (dx, dy, dz) = (
            -(mid[0] * 2.0 - 1.0),
            -(mid[1] * 2.0 - 1.0),
            -(mid[2] * 2.0 - 1.0),
        );
        let dxz = f64::from(dx * dx + dz * dz).sqrt() as f32;
        let degrees = |y: f32, x: f32| {
            (horizon_core::jmath::atan2(f64::from(y), f64::from(x)) * 180.0
                / f64::from(std::f32::consts::PI)) as f32
        };
        let y_rot = degrees(dx, dz);
        let x_rot = degrees(dy, dxz);
        t *= glam::Mat4::from_rotation_y((y_rot - 90.0).to_radians())
            * glam::Mat4::from_rotation_z(x_rot.to_radians());
        super::render_model(item, t, &|p| p.pose, uv, light, 0, out);
    }
}
/// `ParrotOnShoulderLayer.submitOnShoulder`: a parrot of `variant` on the
/// avatar's left or right shoulder, in `ParrotModel.Pose.ON_SHOULDER` with
/// the avatar's walk animation, age and head turn. `base` is the avatar's
/// root transform; `parrot` the parrot's entity type (its model).
#[allow(clippy::too_many_arguments)]
pub fn shoulder_parrot(
    avatar: &Mob,
    parrot: &super::MobType,
    variant: i32,
    left: bool,
    alpha: f32,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let Some(uv) = atlas(&shoulder_parrot_texture(variant)) else {
        return;
    };
    let model = parrot.layer(&parrot.model_key()).unwrap_or(&parrot.model);
    let s = anim::State {
        baby: false,
        ..anim::state(avatar, alpha)
    };
    let mut poses = Poses::new(model);
    // ParrotRenderState.flapAngle stays 0.
    super::body_anim::parrot(
        &mut poses,
        &s,
        super::body_anim::ParrotPose::OnShoulder,
        0.0,
    );
    let y = if avatar.extra.crouching { -1.3 } else { -1.5 };
    let x = if left { 0.4 } else { -0.4 };
    let base = base * glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.0));
    // ParrotModel's cutout render type with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `ParrotRenderer.getVariantTexture` for `Parrot.Variant.byId` (an id out
/// of range is clamped).
fn shoulder_parrot_texture(variant: i32) -> String {
    const NAMES: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "grey"];
    let name = usize::try_from(variant)
        .ok()
        .and_then(|i| NAMES.get(i))
        .unwrap_or(if variant < 0 { &NAMES[0] } else { &NAMES[4] });
    format!("entity/parrot/parrot_{name}")
}
/// `render_with` without the body, for an avatar the viewer can't see:
/// `LivingEntityRenderer.submit` gets no render type for an invisible body
/// (`isBodyVisible`, `isInvisibleTo`), but its layers still draw
/// (`HumanoidArmorLayer`: the armour in `extra`).
#[allow(clippy::too_many_arguments)]
pub fn render_layers(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    extra: &[(&Part, [f32; 4], u32, u32)],
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    render_visible(m, alpha, base, atlas, light, data, extra, false, out);
}
/// A mob drawn under `origin` instead of at its position (the spinning mob
/// of `SpawnerRenderer.submitEntityInSpawner`).
pub fn render_posed(
    m: &Mob,
    alpha: f32,
    origin: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform_at(m, alpha, origin);
    render_visible(m, alpha, base, atlas, light, data, &[], true, out);
}
#[allow(clippy::too_many_arguments)]
fn render_visible(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    extra: &[(&Part, [f32; 4], u32, u32)],
    body_visible: bool,
    out: &mut Vec<Vertex>,
) {
    let overlay = u32::from(red_overlay(m));
    let s = anim::state(m, alpha);
    let (model_key, texture) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let draw_body = |out: &mut Vec<Vertex>| {
        // A player or mannequin whose skin has no atlas sprite (an evicted or
        // never stored download) draws with its default skin, not invisibly.
        let uv = atlas(&texture).or_else(|| {
            if avatar(m) {
                atlas(&m.kind.texture)
            } else {
                None
            }
        });
        if body_visible && let Some(uv) = uv {
            let start = out.len();
            draw(
                &model,
                base,
                &poses,
                uv,
                body_color(m, alpha),
                light,
                body_overlay(m, alpha),
                out,
            );
            if translucent_body(m) {
                mark_oit(&mut out[start..]);
            }
        }
    };
    // The translucent shell writes depth: draw the opaque core before it.
    let shell_last = m.species() == Species::SulfurCube;
    if !shell_last {
        draw_body(out);
    }
    for (part, uv, color, layer_overlay) in extra {
        draw(part, base, &poses, *uv, *color, light, *layer_overlay, out);
    }
    let mut all_layers = layers(m, s.age);
    // WardenRenderer: the translucent emissive layers fade with the animations.
    if m.species() == Species::Warden {
        for (model, texture, a) in super::warden::emissive_layers(m, alpha) {
            if a > 0.0 {
                all_layers.push(Layer {
                    model: model.into(),
                    texture: texture.into(),
                    color: ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24 | 0x00FF_FFFF,
                    full_bright: true,
                    scroll: 0.0,
                });
            }
        }
    }
    // One Poses refilled for every layer (armour draws up to seven): it keeps
    // its maps' capacity.
    let mut layer_poses = Poses::default();
    for layer in all_layers {
        let Some(part) = m.kind.layer(&layer.model).cloned() else {
            continue;
        };
        let Some(uv) = atlas(&layer.texture) else {
            continue;
        };
        layer_poses.reset(&part);
        anim::setup(m, &s, &mut layer_poses, &data.animations);
        if matches!(m.species(), Species::Villager | Species::ZombieVillager)
            && let Some((folder, variant)) = layer
                .texture
                .split_once("/type/")
                .or_else(|| layer.texture.split_once("/baby/"))
        {
            let profession = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
            data.villager_hats
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .apply(folder, variant, profession, &mut layer_poses);
        }
        let light = if layer.full_bright {
            240 | 240 << 8
        } else {
            light
        };
        // EquipmentLayerRenderer draws armour with OverlayTexture.NO_OVERLAY: never
        // the hurt tint, the armour glint (`ARMOR_GLINT`) when enchanted.
        let overlay = match armor_slot(&layer.model) {
            Some(slot) if super::gear::enchanted(m, slot) => ARMOR_GLINT,
            Some(_) => 0,
            None => overlay,
        };
        let start = out.len();
        draw_scrolled(
            &part,
            base,
            &layer_poses,
            uv,
            layer.scroll,
            layer.color,
            light,
            overlay,
            out,
        );
        if translucent_layer(m, &layer) {
            mark_oit(&mut out[start..]);
        }
        // The eyes render type's glowing eyes also shine in HDR output, at
        // `EYES_TIER` in the overlay's high bits (entity.wgsl fs_hdr_mark).
        if layer.full_bright && m.species() != Species::Warden {
            for vertex in &mut out[start..] {
                vertex.normal = [0; 4];
                vertex.layer |= u32::from(EYES_TIER) << 16;
            }
        }
    }
    // EnergySwirlLayer: the armour model, additive, scrolling over the sprite.
    if let Some((model, texture, x_offset)) = energy_swirl(m, s.age)
        && let Some(part) = m.kind.layer(model)
        && let Some([u0, v0, u1, v1]) = atlas(texture)
    {
        layer_poses.reset(part);
        anim::setup(m, &s, &mut layer_poses, &data.animations);
        // `RenderTypes.energySwirl`'s texture offset, as the sprite-local UV.
        let (du, dv) = (x_offset % 1.0, s.age * 0.01 % 1.0);
        let start = out.len();
        draw(
            part,
            base,
            &layer_poses,
            [du, dv, 1.0 + du, 1.0 + dv],
            crate::breaking::stage_light([u1 - u0, v1 - v0]),
            crate::breaking::stage_light([u0, v0]),
            crate::gfx::ENERGY_SWIRL_OVERLAY,
            out,
        );
        // No lighting: the colour bytes carry the sprite's size (fs_energy_swirl).
        for vertex in &mut out[start..] {
            vertex.normal = [0; 4];
        }
    }
    if shell_last {
        draw_body(out);
    }
}
/// `EnergySwirlLayer`'s model, texture and `xOffset(t)` for a charged creeper
/// (`CreeperPowerLayer`) or a powered wither (`WitherArmorLayer`); `t` is `ageInTicks`.
fn energy_swirl(m: &Mob, t: f32) -> Option<(&'static str, &'static str, f32)> {
    match m.species() {
        Species::Creeper if m.extra.charged => {
            Some(("creeper#armor", "entity/creeper/creeper_armor", t * 0.01))
        }
        Species::Wither if super::wither::powered(m) => Some((
            "wither#armor",
            "entity/wither/wither_armor",
            (t * 0.02).mcos() * 3.0,
        )),
        _ => None,
    }
}
/// The equipment slot of a `HumanoidArmorLayer` model (`zombie#helmet`).
fn armor_slot(model: &str) -> Option<usize> {
    Some(match model.rsplit('#').next()? {
        "helmet" => super::gear::HEAD,
        "chestplate" => super::gear::CHEST,
        "leggings" => super::gear::LEGS,
        "boots" => super::gear::FEET,
        _ => return None,
    })
}
/// Render with a fixed texture rect (the earlier API).
pub fn render_single(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    let overlay = body_overlay(m, alpha);
    let s = anim::state(m, alpha);
    let model = m.kind.model.clone();
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &std::collections::HashMap::new());
    draw(&model, base, &poses, uv, 0xFFFF_FFFF, light, overlay, out);
}

/// The transform of a named part (ModelPart.translateAndRotate down the tree).
pub fn part_matrix(
    part: &Part,
    parent: glam::Mat4,
    poses: &Poses,
    name: &str,
    x_offset: f32,
) -> Option<glam::Mat4> {
    let mut p = poses.pose(part);
    if part.name == name {
        p[0] += x_offset;
    }
    let m = parent
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if part.name == name {
        return Some(m);
    }
    part.children
        .iter()
        .find_map(|c| part_matrix(c, m, poses, name, x_offset))
}
fn find_part<'a>(part: &'a Part, name: &str) -> Option<&'a Part> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find_part(c, name))
}
/// `AllayModel.translateToHand`: the root and body, then the right arm's
/// pitch, a 0.7 scale and a shift. Both hands use the right arm's pitch.
fn allay_hand(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let body = part_matrix(model, base, poses, "body", 0.0)?;
    let x_rot = poses.pose(find_part(model, "right_arm")?)[3];
    Some(
        body * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0625, 0.1875))
            * glam::Mat4::from_rotation_x(x_rot)
            * glam::Mat4::from_scale(glam::Vec3::splat(0.7))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.0, 0.0)),
    )
}
/// The hand of a mob in model space, up to the item's own transform
/// (`ItemInHandLayer.submitArmWithItem`).
fn hand_base(
    m: &Mob,
    model: &Part,
    base: glam::Mat4,
    poses: &Poses,
    left: bool,
) -> Option<glam::Mat4> {
    let skeleton = matches!(
        m.species(),
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    );
    let arm = if left { "left_arm" } else { "right_arm" };
    // `SkeletonModel.translateToHand` shifts the arm by 1, `PlayerModel.translateToHand`
    // by 0.5 for slim arms: the hand is the middle of the arm.
    let offset = if skeleton {
        if left { -1.0 } else { 1.0 }
    } else if avatar(m) && m.slim {
        if left { -0.5 } else { 0.5 }
    } else {
        0.0
    };
    let mut t = if m.species() == Species::Allay {
        allay_hand(model, base, poses)?
    } else {
        part_matrix(model, base, poses, arm, offset)?
    };
    if m.species() == Species::Vex {
        // `VexModel.translateToHand`: the held item is drawn at 0.55 scale.
        t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55));
        t *= glam::Mat4::from_translation(glam::Vec3::new(
            if left { -0.046_875 } else { 0.046_875 },
            -0.156_25,
            0.078_125,
        ));
    }
    if m.species() == Species::CopperGolem {
        // `CopperGolemModel.translateToHand`: a still golem holds the item
        // across the arm, a working one scaled down in front of it.
        if m.extra.variant3 == super::copper_golem::IDLE {
            t *= glam::Mat4::from_rotation_y((if left { 90.0f32 } else { -90.0 }).to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.125));
        } else {
            t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55))
                * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.3125, -0.1875));
        }
    }
    let (ox, oy, oz) = if m.is_baby() {
        (0.0, 1.0, -4.5)
    } else {
        (1.0, 2.0, -10.0)
    };
    t *= glam::Mat4::from_rotation_x((-90.0f32).to_radians()) * glam::Mat4::from_rotation_y(PI);
    t *= glam::Mat4::from_translation(glam::Vec3::new(
        if left { -ox } else { ox } / 16.0,
        oy / 16.0,
        oz / 16.0,
    ));
    Some(t)
}
/// The transform of one of a mob's model parts, in camera space
/// (`ModelPart.translateAndRotate` down the tree).
pub fn part_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    part: &str,
) -> Option<glam::Mat4> {
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    part_matrix(&model, base, &poses, part, 0.0)
}
/// `PlayerItemInHandLayer.renderItemHeldToEye` up to the item: the root and
/// the head with its pitch clamped to -30 to 90 degrees, `translateToHead`
/// (`CustomHeadLayer.translateToHead`), then the hand's side of the eye.
pub fn held_to_eye_matrix(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    left: bool,
) -> Option<glam::Mat4> {
    use glam::{Mat4, Vec3};
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let pitch =
        poses.get("head")[3].clamp(-std::f32::consts::FRAC_PI_6, std::f32::consts::FRAC_PI_2);
    poses.xrot("head", pitch);
    let head = part_matrix(&model, base, &poses, "head", 0.0)?;
    let side = if left { -2.5 / 16.0 } else { 2.5 / 16.0 };
    Some(
        head * Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_rotation_y(std::f32::consts::PI)
            * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
            * Mat4::from_translation(Vec3::new(side, -0.0625, 0.0)),
    )
}
/// `FoxHeldItemLayer`: the item sits at the head's position under the render
/// state's head roll, yaw and pitch (not the posed head's, which differ while
/// it sleeps), pushed out to the mouth.
fn fox_item_transform(m: &Mob, alpha: f32, camera: glam::DVec3, data: &MobData) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let head = *poses.get("head");
    let fox = super::fox::pose(m, alpha);
    let baby = m.is_baby();
    let mut t = base_transform(m, alpha, camera)
        * Mat4::from_translation(Vec3::new(head[0], head[1], head[2]) / 16.0);
    if baby {
        t *= Mat4::from_scale(Vec3::splat(0.75));
    }
    t *= Mat4::from_rotation_z(fox.head_roll_angle)
        * Mat4::from_rotation_y(s.y_rot.to_radians())
        * Mat4::from_rotation_x(s.x_rot.to_radians());
    let (x, y, z) = match (baby, fox.sleeping) {
        (true, true) => (0.4, 0.26, 0.15),
        (true, false) => (0.06, 0.26, -0.5),
        (false, true) => (0.46, 0.26, 0.22),
        (false, false) => (0.06, 0.27, -0.5),
    };
    t *= Mat4::from_translation(Vec3::new(x, y, z)) * Mat4::from_rotation_x(90.0f32.to_radians());
    if fox.sleeping {
        t *= Mat4::from_rotation_z(90.0f32.to_radians());
    }
    t
}
/// `CrossedArmsItemLayer.applyTranslation`: `translateToArms`, then the item's
/// placement over the folded arms.
fn crossed_arms_matrix(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let arms = part_matrix(model, base, poses, "arms", 0.0)?;
    Some(
        arms * glam::Mat4::from_rotation_x(0.75)
            * glam::Mat4::from_scale(glam::Vec3::splat(1.07))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.13, -0.34))
            * glam::Mat4::from_rotation_x(std::f32::consts::PI),
    )
}
/// The pose a mob's held item is submitted under (`ItemInHandLayer`:
/// `translateToHand`, then the hand offset; the item model adds its own
/// display transform), or `None` when the mob shows no held items.
pub fn held_item_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    left: bool,
) -> Option<glam::Mat4> {
    // PandaHoldsItemLayer: the main hand's item in its mouth.
    if m.species() == Species::Panda {
        return super::panda::item_offset(m, alpha)
            .filter(|_| !left)
            .map(|offset| base_transform(m, alpha, camera) * offset);
    }
    // FoxHeldItemLayer: the main hand's item in its mouth.
    if m.species() == Species::Fox {
        return (!left).then(|| fox_item_transform(m, alpha, camera, data));
    }
    // DolphinCarryingItemLayer: the main hand's item at the snout, lifted and
    // pulled in as the dolphin pitches (up or down) toward 60 degrees.
    if m.species() == Species::Dolphin {
        return (!left).then(|| {
            let pitch = anim::state(m, alpha).x_rot;
            let percent = pitch.abs() / 60.0;
            let offset = if pitch < 0.0 {
                glam::Vec3::new(0.0, 1.0 - percent * 0.5, -1.0 + percent * 0.5)
            } else {
                glam::Vec3::new(0.0, 1.0 + percent * 0.8, -1.0 + percent * 0.2)
            };
            base_transform(m, alpha, camera) * glam::Mat4::from_translation(offset)
        });
    }
    if !super::illager::shows_item(m) {
        return None;
    }
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    // WitchItemLayer.applyTranslation: a potion is held at the nose, any other
    // item as CrossedArmsItemLayer holds it; only the main hand's item is drawn.
    if m.species() == Species::Witch {
        let item = m.extra.main_hand.as_deref()?;
        if left != m.main_arm_left() {
            return None;
        }
        if super::witch::holds_potion(item) {
            let nose = part_matrix(&model, base, &poses, "nose", 0.0)?;
            return Some(
                nose * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.25, 0.0))
                    * glam::Mat4::from_rotation_z(180.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(140.0f32.to_radians())
                    * glam::Mat4::from_rotation_z(10.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(180.0f32.to_radians()),
            );
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    // CrossedArmsItemLayer (villagers and wandering traders): the main hand's
    // item over the folded arms.
    if matches!(m.species(), Species::Villager | Species::WanderingTrader) {
        m.extra.main_hand.as_deref()?;
        if left != m.main_arm_left() {
            return None;
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    let mut t = hand_base(m, &model, base, &poses, left)?;
    // ItemInHandLayer: a STAB swing of this arm thrusts its item out
    // (SpearAnimations.thirdPersonAttackItem, the main hand's forward
    // movement), and a spear in use moves with it (ArmPose.SPEAR's
    // animateUseItem).
    let swing_left = m.extra.swing_right_arm.map_or(m.main_arm_left(), |r| !r);
    if m.extra.swing_stab && swing_left == left {
        let forward = m
            .extra
            .main_hand
            .as_deref()
            .and_then(crate::spear::Kinetic::of)
            .map_or(0.0, |k| k.forward_movement);
        t *= crate::spear::third_person_attack_item(s.swing, forward);
    }
    let off_hand = left != m.main_arm_left();
    if super::spear_use::is_using_item(m)
        && m.extra.using_off_hand == off_hand
        && let Some(kinetic) = (if off_hand {
            &m.extra.off_hand
        } else {
            &m.extra.main_hand
        })
        .as_deref()
        .and_then(crate::spear::Kinetic::of)
    {
        let time_held = m.extra.using_item as f32 + s.partial;
        t *= crate::spear::third_person_use_item(&kinetic, time_held, s.swing, 0.0, !left);
    }
    Some(t)
}

/// `ArmedEntityRenderState.extractArmedEntityRenderState`: each arm's item
/// model reads the stack held by that arm. Only the main hand currently
/// retains a full stack; the off hand must not inherit its item or components.
pub fn held_item_stack(extra: &super::Extra, off_hand: bool) -> Option<crate::items::ItemStack> {
    let item = if off_hand {
        extra.off_hand.as_ref()?
    } else {
        if let Some(stack) = extra.main_hand_stack() {
            return Some(stack.clone());
        }
        extra.main_hand.as_ref()?
    };
    Some(crate::items::ItemStack::new(item, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offhand_does_not_inherit_mainhand_components() {
        use crate::items::ItemStack;
        let mut extra = super::super::Extra::default();
        let mut main = ItemStack::new("minecraft:crossbow", 1);
        main.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("Main hand".into()));
        extra.set_main_hand_stack(Some(main));
        extra.off_hand = Some("minecraft:crossbow".into());
        assert_eq!(
            held_item_stack(&extra, false)
                .unwrap()
                .custom_name()
                .as_deref(),
            Some("Main hand")
        );
        assert!(
            held_item_stack(&extra, true)
                .unwrap()
                .custom_name()
                .is_none()
        );
        extra.off_hand = None;
        assert!(held_item_stack(&extra, true).is_none());
    }

    /// The elytra's flight yaw (`AvatarRenderer.extractFlightData`) lerps
    /// the velocity of the previous tick towards this tick's, so the model
    /// does not step once per tick while the view turns (#4104).
    #[test]
    fn the_elytra_flight_yaw_lerps_the_velocity_between_ticks() {
        use crate::mobs::{Anim, Brain, Flags, MobType, Part};
        use horizon_core::geometry::Vec3;
        use serde_json::json;
        use std::sync::Arc;

        let model = Arc::new(Part::parse("root", &json!({"children": {"head": {}}})));
        let kind = Arc::new(MobType {
            id: "minecraft:player".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Flags::default(),
            model,
            layers: std::collections::HashMap::new(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Camel,
            sounds: [""; 3],
            step_sound: None,
            food: Arc::default(),
            tempt: Arc::default(),
        });
        let gliding = |previous: Vec3, current: Vec3| {
            let mut m = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
            m.extra.fall_flying = true;
            m.extra.fall_fly_ticks = 20;
            m.extra.previous_velocity = previous;
            m.velocity = current;
            m
        };
        let from = Vec3::new(0.4, 0.0, 0.4);
        let to = Vec3::new(0.0, 0.0, 0.5);
        let at = |m: &Mob, alpha: f32| base_transform_at(m, alpha, glam::Mat4::IDENTITY);
        let differs = |a: glam::Mat4, b: glam::Mat4| {
            a.to_cols_array()
                .iter()
                .zip(b.to_cols_array())
                .any(|(x, y)| (x - y).abs() > 1.0e-4)
        };
        let m = gliding(from, to);
        // The ends are the two ticks' velocities.
        assert!(!differs(at(&m, 0.0), at(&gliding(from, from), 0.0)));
        assert!(!differs(at(&m, 1.0), at(&gliding(to, to), 1.0)));
        // Between them the yaw is that of the lerped velocity, which is
        // neither end: the current tick's alone made it snap.
        let mid = Vec3::new(0.2, 0.0, 0.45);
        let half = at(&m, 0.5);
        assert!(!differs(half, at(&gliding(mid, mid), 0.5)));
        assert!(differs(half, at(&gliding(to, to), 0.5)));
        assert!(differs(half, at(&gliding(from, from), 0.5)));
    }

    /// Six faces whose corners span `min` to `max`.
    fn cube(min: [f32; 3], max: [f32; 3]) -> Vec<super::super::Polygon> {
        let corner = |p: [f32; 3]| [p[0], p[1], p[2], 0.0, 0.0];
        (0..6)
            .map(|_| super::super::Polygon {
                normal: [0.0; 3],
                vertices: [corner(min), corner(max), corner(min), corner(max)],
            })
            .collect()
    }

    /// `ModelPart.cubes`: the corners of each cube of a part, six faces apiece.
    #[test]
    fn cube_bounds_follow_the_faces_of_each_cube() {
        let mut polygons = cube([-4.0, -8.0, -2.0], [4.0, 0.0, 2.0]);
        polygons.extend(cube([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]));
        let part = Part {
            name: "body".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons,
            children: Vec::new(),
        };
        assert_eq!(
            cube_bounds(&part),
            vec![
                [-4.0, -8.0, -2.0, 4.0, 0.0, 2.0],
                [0.0, 0.0, 0.0, 1.0, 2.0, 3.0]
            ]
        );
    }

    /// `StuckInBodyLayer.snapToFace`.
    #[test]
    fn a_stuck_item_snaps_to_the_far_or_the_middle_face() {
        assert_eq!(snap_to_face(0.75), 1.0);
        assert_eq!(snap_to_face(0.5), 0.5);
        assert_eq!(snap_to_face(0.1), 0.5);
    }

    #[test]
    fn shoulder_parrot_textures_follow_the_variant() {
        assert_eq!(shoulder_parrot_texture(0), "entity/parrot/parrot_red_blue");
        assert_eq!(shoulder_parrot_texture(4), "entity/parrot/parrot_grey");
        // Parrot.Variant.byId clamps an id out of range.
        assert_eq!(shoulder_parrot_texture(5), "entity/parrot/parrot_grey");
        assert_eq!(shoulder_parrot_texture(-1), "entity/parrot/parrot_red_blue");
    }

    /// VillagerProfessionLayer's textures (type, or baby for a baby;
    /// profession; level) of every villager and zombie villager are in the
    /// prepared resources: a missing one draws nothing.
    #[test]
    fn villager_layers_name_textures_that_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        for kind in ["minecraft:villager", "minecraft:zombie_villager"] {
            let kind = std::sync::Arc::new(types.types[kind].clone());
            for baby in [false, true] {
                for t in 0..VILLAGER_TYPES.len() {
                    for p in 0..PROFESSIONS.len() {
                        let mut m =
                            Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
                        m.extra.age = if baby { -24000 } else { 0 };
                        (m.extra.variant, m.extra.variant2, m.extra.variant3) =
                            (t as u8, p as u8, 5);
                        let layers = layers(&m, m.age as f32);
                        let group = if baby { "/baby/" } else { "/type/" };
                        assert!(
                            layers.iter().any(|l| l.texture.contains(group)),
                            "{kind:?} baby={baby}: no {group} layer"
                        );
                        for layer in layers {
                            let png = root
                                .join(format!("assets/minecraft/textures/{}.png", layer.texture));
                            assert!(png.is_file(), "missing {}", png.display());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sulfur_cube_opaque_core_is_submitted_before_its_translucent_shell() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let mut types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types.remove("minecraft:sulfur_cube").unwrap());
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        };
        let atlas = |name: &str| {
            Some(if name.contains("_inner") {
                [0.0, 0.0, 0.25, 0.25]
            } else {
                [0.5, 0.5, 0.75, 0.75]
            })
        };
        for baby in [false, true] {
            let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.age = if baby { -24000 } else { 0 };
            super::super::sulfur_cube::finalize(&mut mob);
            assert_eq!(mob.is_baby(), baby);
            for posed in [false, true] {
                let mut vertices = Vec::new();
                if posed {
                    render_posed(
                        &mob,
                        0.5,
                        glam::Mat4::IDENTITY,
                        &atlas,
                        0xf0f0,
                        &data,
                        &mut vertices,
                    );
                } else {
                    render(
                        &mob,
                        0.5,
                        glam::DVec3::ZERO,
                        &atlas,
                        0xf0f0,
                        &data,
                        &mut vertices,
                    );
                }
                assert_eq!(
                    vertices.len(),
                    48,
                    "both real six-face models must be drawn"
                );
                assert!(
                    vertices[..24].iter().all(|v| v.uv[0] <= 0.25),
                    "opaque core must precede shell"
                );
                assert!(vertices[24..].iter().all(|v| v.uv[0] >= 0.5));
            }
            mob.extra.head = Some("minecraft:stone".into());
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            assert_eq!(
                vertices.len(),
                24,
                "a swallowed block still hides the default core"
            );
            assert!(vertices.iter().all(|v| v.uv[0] >= 0.5));
        }
    }
    /// CopperGolemRenderer: the emissive eye layer draws the eye texture on
    /// the body's model, every polygon of it, at each oxidation level. The
    /// prepared `copper_golem#eyes` model has no polygons, so the eyes
    /// showed the body texture's black sockets.
    #[test]
    fn copper_golem_eyes_are_drawn_on_the_body_model() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:copper_golem"].clone());
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        };
        let atlas = |name: &str| {
            Some(if name.contains("copper_golem_eyes") {
                [0.5, 0.5, 0.75, 0.75]
            } else {
                [0.0, 0.0, 0.25, 0.25]
            })
        };
        for variant in 0..4 {
            let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.variant = variant;
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            let body = vertices.iter().filter(|v| v.uv[0] <= 0.25).count();
            let eyes = vertices.iter().filter(|v| v.uv[0] >= 0.5).count();
            assert!(body > 0, "variant {variant}: no body drawn");
            assert_eq!(
                eyes, body,
                "variant {variant}: the eye layer covers the body"
            );
        }
    }
    /// `VexModel.translateToHand`: the held item is drawn at 0.55 scale and
    /// moved by the vex's own offset, so the vex's sword is the tiny one
    /// vanilla shows. It was drawn at the full size, like a zombie's.
    #[test]
    fn the_vex_holds_its_item_at_the_small_scale() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:vex"].clone());
        let mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        };
        for left in [false, true] {
            let t = held_item_transform(&mob, 0.5, glam::DVec3::ZERO, &data, left).unwrap();
            // The model's own part scale is 1, so the item's scale is the
            // renderer's: `poseStack.scale(0.55F, 0.55F, 0.55F)`.
            for axis in [t.x_axis, t.y_axis, t.z_axis] {
                let scale = axis.truncate().length();
                assert!(
                    (scale - 0.55).abs() < 1.0e-5,
                    "the vex's item is drawn at 0.55 scale, got {scale}"
                );
            }
        }
    }

    /// `CrossedArmsItemLayer`: a villager and a wandering trader show their
    /// main hand's item over the folded arms, and nothing in the off hand.
    #[test]
    fn villagers_and_traders_hold_items_in_crossed_arms() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let data = MobData::load(&root, &facts);
        for id in ["minecraft:villager", "minecraft:wandering_trader"] {
            let kind = std::sync::Arc::new(types.types[id].clone());
            let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
            let main = mob.main_arm_left();
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_none());
            mob.extra.set_main_hand(Some("minecraft:emerald".into()));
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_some());
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, !main).is_none());
        }
    }
    /// PhantomRenderer.scale moves the model down into its 0.5-high box:
    /// the model's origin ends 1.501 - 1.3125 blocks above the feet (times
    /// the size's scale), not 1.501 like other mobs (#3008).
    #[test]
    fn the_phantom_model_sits_in_its_box() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:phantom"].clone());
        let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        for (size, scale) in [(0, 1.0f32), (5, 1.75)] {
            mob.extra.size = size;
            let origin =
                base_transform(&mob, 1.0, glam::DVec3::ZERO).transform_point3(glam::Vec3::ZERO);
            let y = (1.501 - 1.3125) * scale;
            assert!(
                (origin.y - y).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
            assert!(
                (origin.z.abs() - 0.1875 * scale).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
        }
    }
    /// `EquipmentLayerRenderer.renderLayers`: the extra armour layers (a
    /// player's) take their own overlay, `NO_OVERLAY` or the armour glint,
    /// never the hurt tint of the body, and the glint samples the model UV.
    #[test]
    fn extra_armour_layers_carry_their_glint_not_the_hurt_tint() {
        // The batch gives these overlays their own materials (13, the water
        // mask, draws depth only): the glint must not be one of them.
        use crate::gfx::{
            ENERGY_SWIRL_OVERLAY, SHADOW_OVERLAY, TEXT_POLYGON_OFFSET_OVERLAY,
            TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_PARTICLE_OVERLAY,
            WATER_MASK_OVERLAY,
        };
        assert!(
            ![
                WATER_MASK_OVERLAY,
                SHADOW_OVERLAY,
                TRANSLUCENT_PARTICLE_OVERLAY,
                TRANSLUCENT_ITEM_OVERLAY,
                TRANSLUCENT_GLINT_ITEM_OVERLAY,
                ENERGY_SWIRL_OVERLAY,
                TEXT_POLYGON_OFFSET_OVERLAY,
            ]
            .contains(&ARMOR_GLINT)
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
        let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
        mob.hurt_time = 5;
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        };
        let atlas = |_: &str| Some([0.0, 0.0, 0.25, 0.25]);
        let part = &kind.model;
        for overlay in [ARMOR_GLINT, 0] {
            let mut vertices = Vec::new();
            render_with(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &[(part, [0.5, 0.5, 0.75, 0.75], 0xFFFF_FFFF, overlay)],
                &mut vertices,
            );
            let (armour, body): (Vec<&Vertex>, Vec<&Vertex>) =
                vertices.iter().partition(|v| v.uv[0] >= 0.5);
            assert!(!armour.is_empty() && !body.is_empty());
            // The hurt body keeps its red overlay.
            let hurt = body_overlay(&mob, 0.5);
            assert_ne!(hurt, 0);
            assert!(body.iter().all(|v| v.layer == hurt));
            assert!(armour.iter().all(|v| v.layer == overlay));
            // The glint scrolls the layer's model UV (0..1 over the texture).
            for v in &armour {
                if overlay == ARMOR_GLINT {
                    assert!((v.glint_uv[0] - (v.uv[0] - 0.5) * 4.0).abs() < 1e-4);
                    assert!((v.glint_uv[1] - (v.uv[1] - 0.5) * 4.0).abs() < 1e-4);
                } else {
                    assert_eq!(v.glint_uv, [0.0, 0.0]);
                }
            }
        }
    }
    #[test]
    fn a_dyed_leather_armour_keeps_its_colour() {
        // EquipmentLayerRenderer.getColorForLayer: a dyeable layer takes
        // the stack's dye colour, the colour it has when undyed otherwise.
        let mut stack = crate::items::ItemStack {
            item: "minecraft:leather_helmet".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        // Undyed leather: the colour the leather layer has when undyed.
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0xA06540));
        // Dyed: the stack's own colour.
        stack.extra = Some(Box::new(crate::item_data::StackData {
            dyed_color: Some(0x0000FF),
            ..Default::default()
        }));
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0x0000FF));
        // A layer that is not dyeable is not tinted.
        assert_eq!(armor_tint(None, Some(&stack)), 0xFFFF_FFFF);
    }

    /// `ColorLerper.getLerpedColor(Type.SHEEP, ageInTicks)`: the points the
    /// 26.3 jar gave — 0, half a step, one whole step, a step boundary, the
    /// last step back into white, and the 400-tick (16-step) wrap.
    #[test]
    fn jeb_wool_lerps_through_the_sixteen_dye_colours() {
        assert_eq!(jeb_wool_rgb(0.0), 0xE6E6E6);
        assert_eq!(jeb_wool_rgb(12.5), 0xD0A37D);
        assert_eq!(jeb_wool_rgb(25.0), 0xBA6015);
        assert_eq!(jeb_wool_rgb(25.0), sheep_wool_rgb(1));
        assert_eq!(jeb_wool_rgb(50.0), 0x953A8D);
        assert_eq!(jeb_wool_rgb(399.0), 0xDDDDDD);
        assert_eq!(jeb_wool_rgb(400.0), 0xE6E6E6);
    }

    fn sheep_layer(m: &Mob, model: &str, age: f32) -> Option<Layer> {
        layers(m, age).into_iter().find(|l| l.model == model)
    }

    /// SheepRenderer.extractRenderState's `jeb_` magic name: the wool layer
    /// takes the lerped colour and cycles with the age, a white `jeb_` sheep
    /// keeps its undercoat (SheepWoolUndercoatLayer draws for it too), and
    /// the dye colour, the baby model and the shears still win.
    #[test]
    fn a_jeb_sheep_cycles_its_wool_colour_and_keeps_the_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        mob.custom_name = Some("jeb_".into());
        let young = sheep_layer(&mob, "sheep#wool", 0.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_eq!(young, abgr(0xE6E6E6));
        let older = sheep_layer(&mob, "sheep#wool", 100.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_ne!(young, older, "the colour cycles with the age");
        let undercoat = sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
            .expect("a jeb_ sheep keeps its undercoat");
        assert_eq!(undercoat.color, abgr(0xE6E6E6));
        // A name that is not exactly "jeb_" leaves a dyed sheep's colour, and
        // a non-white sheep has its undercoat too.
        mob.custom_name = Some("jeb".into());
        mob.extra.color = 14;
        let dyed = abgr(sheep_wool_rgb(14));
        assert_eq!(
            sheep_layer(&mob, "sheep#wool", 0.0)
                .expect("an unsheared sheep draws its wool")
                .color,
            dyed
        );
        assert_eq!(
            sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
                .expect("a dyed sheep keeps its undercoat")
                .color,
            dyed
        );
        // A baby `jeb_` sheep gets the lerped colour on the baby wool and
        // never an undercoat.
        mob.custom_name = Some("jeb_".into());
        mob.extra.color = 0;
        mob.extra.age = -24000;
        assert!(mob.is_baby());
        assert_eq!(
            sheep_layer(&mob, "sheep_baby#wool", 50.0)
                .expect("a baby sheep draws its wool")
                .color,
            abgr(0x953A8D)
        );
        assert!(sheep_layer(&mob, "sheep#wool_undercoat", 50.0).is_none());
        // Sheared: no wool layer at all.
        mob.extra.age = 0;
        mob.extra.sheared = true;
        assert!(sheep_layer(&mob, "sheep#wool", 0.0).is_none());
    }
}

#[cfg(test)]
mod scroll_tests {
    #[test]
    fn a_pufferfish_bobs_by_eight_hundredths() {
        // PufferfishRenderer.setupRotations: cos(ageInTicks * 0.05) * 0.08.
        assert!((super::pufferfish_bob_y(0.0) - 0.08).abs() < 1e-6);
        let quarter = std::f32::consts::FRAC_PI_2 / 0.05;
        assert!(super::pufferfish_bob_y(quarter).abs() < 1e-3);
        assert!(super::pufferfish_bob_y(2.0 * quarter) < -0.0799);
    }

    use super::*;

    #[test]
    fn the_scrolling_texture_wraps_around() {
        // RenderTypes.breezeWind: the breeze's wind layer's texture scrolls
        // by 0.02 of the texture per tick, the wind charge's by 0.03.
        assert_eq!(scroll_offset(0.0, BREEZE_WIND_SCROLL), 0.0);
        assert!((scroll_offset(25.0, BREEZE_WIND_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(50.0, BREEZE_WIND_SCROLL) - 0.0).abs() < 1e-6);
        assert!((scroll_offset(50.0, WIND_CHARGE_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(100.0, WIND_CHARGE_SCROLL) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn the_scrolled_u_wraps_inside_the_texture() {
        // Without a scroll the U is the vertex's own.
        assert_eq!(scroll_u(0.25, 0.0), 0.25);
        assert_eq!(scroll_u(1.0, 0.0), 1.0);
        // With one, it moves and wraps around the texture.
        assert!((scroll_u(0.25, 0.5) - 0.75).abs() < 1e-6);
        assert!((scroll_u(0.75, 0.5) - 0.25).abs() < 1e-6);
        assert!((scroll_u(1.0, 0.5) - 0.5).abs() < 1e-6);
    }

    /// `SheepWoolLayer.submit` returns early on a sheared sheep, but
    /// `SheepWoolUndercoatLayer.submit` has no isSheared guard: a sheared
    /// dyed sheep keeps its undercoat and loses only its wool.
    #[test]
    fn a_sheared_dyed_sheep_keeps_its_wool_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let sheep = |color: u8, sheared: bool, baby: bool| {
            let mut m = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            m.extra.color = color;
            m.extra.sheared = sheared;
            m.extra.age = if baby { -24000 } else { 0 };
            m
        };
        // The sheep's wool layers, model, texture and colour, in submission order.
        let wool = |m: &Mob| -> Vec<(String, String, u32)> {
            layers(m, 0.0)
                .into_iter()
                .filter(|l| l.model.contains("#wool"))
                .map(|l| (l.model, l.texture, l.color))
                .collect()
        };
        let dyed = abgr(sheep_wool_rgb(1));
        let layer = |model: &str, texture: &str| (model.to_owned(), texture.to_owned(), dyed);

        // The regression: the pre-fix single `!sheared` guard skipped both
        // layers, so a sheared dyed sheep drew nothing. It must keep the
        // undercoat alone, in its own colour and texture.
        assert_eq!(
            wool(&sheep(1, true, false)),
            [layer(
                "sheep#wool_undercoat",
                "entity/sheep/sheep_wool_undercoat"
            )]
        );
        // Guards, already green before the fix: a sheared white sheep has no
        // undercoat to keep, a baby has no undercoat at all, and an unsheared
        // dyed sheep draws both layers as before.
        assert!(wool(&sheep(0, true, false)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, true)),
            [layer("sheep_baby#wool", "entity/sheep/sheep_wool_baby")]
        );
        assert!(wool(&sheep(1, true, true)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, false)),
            [
                layer("sheep#wool_undercoat", "entity/sheep/sheep_wool_undercoat"),
                layer("sheep#wool", "entity/sheep/sheep_wool")
            ]
        );
    }
}
