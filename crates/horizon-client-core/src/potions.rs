//! The `MobEffects` and `Potions` registries (code in vanilla, ported as
//! tables): each effect's colour and category, each potion's effects, and the
//! potion colour mix. horizon-client re-exports them from `brewing`.
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Beneficial,
    Harmful,
    Neutral,
}
/// A `MobEffect`: id (without namespace), colour, category, instantaneous.
#[derive(Clone, Copy, Debug)]
pub struct EffectDef {
    pub id: &'static str,
    pub color: u32,
    pub category: Category,
    pub instant: bool,
}
const fn fx(id: &'static str, color: u32, category: Category) -> EffectDef {
    EffectDef {
        id,
        color,
        category,
        instant: false,
    }
}
use Category::{Beneficial as B, Harmful as H, Neutral as N};
/// `MobEffects` in registration order.
pub const EFFECTS: &[EffectDef] = &[
    fx("speed", 3402751, B),
    fx("slowness", 9154528, H),
    fx("haste", 14270531, B),
    fx("mining_fatigue", 4866583, H),
    fx("strength", 16762624, B),
    EffectDef {
        id: "instant_health",
        color: 16262179,
        category: B,
        instant: true,
    },
    EffectDef {
        id: "instant_damage",
        color: 11101546,
        category: H,
        instant: true,
    },
    fx("jump_boost", 16646020, B),
    fx("nausea", 5578058, H),
    fx("regeneration", 13458603, B),
    fx("resistance", 9520880, B),
    fx("fire_resistance", 16750848, B),
    fx("water_breathing", 10017472, B),
    fx("invisibility", 16185078, B),
    fx("blindness", 2039587, H),
    fx("night_vision", 12779366, B),
    fx("hunger", 5797459, H),
    fx("weakness", 4738376, H),
    fx("poison", 8889187, H),
    fx("wither", 7561558, H),
    fx("health_boost", 16284963, B),
    fx("absorption", 2445989, B),
    EffectDef {
        id: "saturation",
        color: 16262179,
        category: B,
        instant: true,
    },
    fx("glowing", 9740385, N),
    fx("levitation", 13565951, H),
    fx("luck", 5882118, B),
    fx("unluck", 12624973, H),
    fx("slow_falling", 15978425, B),
    fx("conduit_power", 1950417, B),
    fx("dolphins_grace", 8954814, B),
    fx("bad_omen", 745784, N),
    fx("hero_of_the_village", 4521796, B),
    fx("darkness", 2696993, H),
    fx("trial_omen", 1484454, N),
    fx("raid_omen", 14565464, N),
    fx("wind_charged", 12438015, H),
    fx("weaving", 7891290, H),
    fx("oozing", 10092451, H),
    fx("infested", 9214860, H),
    fx("breath_of_the_nautilus", 65518, B),
];
pub fn effect(id: &str) -> Option<&'static EffectDef> {
    let id = id.trim_start_matches("minecraft:");
    EFFECTS.iter().find(|e| e.id == id)
}
/// Namespaced effect ids for mob effect storage.
pub fn namespaced(id: &'static str) -> &'static str {
    static IDS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let key = id.strip_prefix("minecraft:").unwrap_or(id);
    IDS.get_or_init(|| {
        EFFECTS
            .iter()
            .map(|e| {
                (
                    e.id,
                    &*Box::leak(format!("minecraft:{}", e.id).into_boxed_str()),
                )
            })
            .collect()
    })
    .get(key)
    .copied()
    .unwrap_or(id)
}
/// A `Potion`: id, name suffix (`item.minecraft.potion.effect.<name>`) and
/// effects as (effect, duration ticks, amplifier).
#[derive(Clone, Copy, Debug)]
pub struct PotionDef {
    pub id: &'static str,
    pub name: &'static str,
    pub effects: &'static [(&'static str, i32, i32)],
}
const fn p(
    id: &'static str,
    name: &'static str,
    effects: &'static [(&'static str, i32, i32)],
) -> PotionDef {
    PotionDef { id, name, effects }
}
/// `Potions` in registration order.
pub const POTIONS: &[PotionDef] = &[
    p("water", "water", &[]),
    p("mundane", "mundane", &[]),
    p("thick", "thick", &[]),
    p("awkward", "awkward", &[]),
    p("night_vision", "night_vision", &[("night_vision", 3600, 0)]),
    p(
        "long_night_vision",
        "night_vision",
        &[("night_vision", 9600, 0)],
    ),
    p("invisibility", "invisibility", &[("invisibility", 3600, 0)]),
    p(
        "long_invisibility",
        "invisibility",
        &[("invisibility", 9600, 0)],
    ),
    p("leaping", "leaping", &[("jump_boost", 3600, 0)]),
    p("long_leaping", "leaping", &[("jump_boost", 9600, 0)]),
    p("strong_leaping", "leaping", &[("jump_boost", 1800, 1)]),
    p(
        "fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 3600, 0)],
    ),
    p(
        "long_fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 9600, 0)],
    ),
    p("swiftness", "swiftness", &[("speed", 3600, 0)]),
    p("long_swiftness", "swiftness", &[("speed", 9600, 0)]),
    p("strong_swiftness", "swiftness", &[("speed", 1800, 1)]),
    p("slowness", "slowness", &[("slowness", 1800, 0)]),
    p("long_slowness", "slowness", &[("slowness", 4800, 0)]),
    p("strong_slowness", "slowness", &[("slowness", 400, 3)]),
    p(
        "turtle_master",
        "turtle_master",
        &[("slowness", 400, 3), ("resistance", 400, 2)],
    ),
    p(
        "long_turtle_master",
        "turtle_master",
        &[("slowness", 800, 3), ("resistance", 800, 2)],
    ),
    p(
        "strong_turtle_master",
        "turtle_master",
        &[("slowness", 400, 5), ("resistance", 400, 3)],
    ),
    p(
        "water_breathing",
        "water_breathing",
        &[("water_breathing", 3600, 0)],
    ),
    p(
        "long_water_breathing",
        "water_breathing",
        &[("water_breathing", 9600, 0)],
    ),
    p("healing", "healing", &[("instant_health", 1, 0)]),
    p("strong_healing", "healing", &[("instant_health", 1, 1)]),
    p("harming", "harming", &[("instant_damage", 1, 0)]),
    p("strong_harming", "harming", &[("instant_damage", 1, 1)]),
    p("poison", "poison", &[("poison", 900, 0)]),
    p("long_poison", "poison", &[("poison", 1800, 0)]),
    p("strong_poison", "poison", &[("poison", 432, 1)]),
    p("regeneration", "regeneration", &[("regeneration", 900, 0)]),
    p(
        "long_regeneration",
        "regeneration",
        &[("regeneration", 1800, 0)],
    ),
    p(
        "strong_regeneration",
        "regeneration",
        &[("regeneration", 450, 1)],
    ),
    p("strength", "strength", &[("strength", 3600, 0)]),
    p("long_strength", "strength", &[("strength", 9600, 0)]),
    p("strong_strength", "strength", &[("strength", 1800, 1)]),
    p("weakness", "weakness", &[("weakness", 1800, 0)]),
    p("long_weakness", "weakness", &[("weakness", 4800, 0)]),
    p("luck", "luck", &[("luck", 6000, 0)]),
    p("slow_falling", "slow_falling", &[("slow_falling", 1800, 0)]),
    p(
        "long_slow_falling",
        "slow_falling",
        &[("slow_falling", 4800, 0)],
    ),
    p("wind_charged", "wind_charged", &[("wind_charged", 3600, 0)]),
    p("weaving", "weaving", &[("weaving", 3600, 0)]),
    p("oozing", "oozing", &[("oozing", 3600, 0)]),
    p("infested", "infested", &[("infested", 3600, 0)]),
];
pub fn potion(id: &str) -> Option<&'static PotionDef> {
    let id = id.trim_start_matches("minecraft:");
    POTIONS.iter().find(|p| p.id == id)
}
/// `PotionContents.BASE_POTION_COLOR` (ARGB -13083194) as RGB.
pub const BASE_POTION_COLOR: u32 = 0x385DC6;
/// `PotionContents.getColor`: amplifier-weighted mix of visible effect colours.
pub fn potion_color(id: Option<&str>) -> u32 {
    let Some(def) = id.and_then(potion) else {
        return BASE_POTION_COLOR;
    };
    let (mut r, mut g, mut b, mut total) = (0u32, 0u32, 0u32, 0u32);
    for (e, _, amp) in def.effects {
        let Some(e) = effect(e) else { continue };
        let w = *amp as u32 + 1;
        r += w * (e.color >> 16 & 255);
        g += w * (e.color >> 8 & 255);
        b += w * (e.color & 255);
        total += w;
    }
    match (
        r.checked_div(total),
        g.checked_div(total),
        b.checked_div(total),
    ) {
        (Some(r), Some(g), Some(b)) => (r << 16) | (g << 8) | b,
        _ => BASE_POTION_COLOR,
    }
}
