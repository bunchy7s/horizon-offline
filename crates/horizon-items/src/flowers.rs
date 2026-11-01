//! The suspicious stew effect of each flower block: `FlowerBlock`'s
//! `suspiciousStewEffects`, built by `FlowerBlock.makeEffectList` as
//! `Entry(effect, Mth.floor(seconds * 20))`. The effect belongs to the block,
//! not to the item's components (`SuspiciousEffectHolder.tryGet`), so a brown
//! mooshroom fed a flower stores the same entry a crafted stew carries.
//!
//! In `Blocks`'s constructor order. `wither_rose` is a `WitherRoseBlock` and
//! the eyeblossoms take their values from `EyeblossomBlock.Type`; all three
//! reach `FlowerBlock`'s constructor, so all are suspicious effect holders.
pub const FLOWER_STEW_EFFECTS: [(&str, &str, i32); 17] = [
    ("minecraft:dandelion", "minecraft:saturation", 7),
    ("minecraft:golden_dandelion", "minecraft:saturation", 7),
    ("minecraft:torchflower", "minecraft:night_vision", 100),
    ("minecraft:poppy", "minecraft:night_vision", 100),
    ("minecraft:blue_orchid", "minecraft:saturation", 7),
    ("minecraft:allium", "minecraft:fire_resistance", 60),
    ("minecraft:azure_bluet", "minecraft:blindness", 220),
    ("minecraft:red_tulip", "minecraft:weakness", 140),
    ("minecraft:orange_tulip", "minecraft:weakness", 140),
    ("minecraft:white_tulip", "minecraft:weakness", 140),
    ("minecraft:pink_tulip", "minecraft:weakness", 140),
    ("minecraft:oxeye_daisy", "minecraft:regeneration", 140),
    ("minecraft:cornflower", "minecraft:jump_boost", 100),
    ("minecraft:wither_rose", "minecraft:wither", 140),
    ("minecraft:lily_of_the_valley", "minecraft:poison", 220),
    ("minecraft:open_eyeblossom", "minecraft:blindness", 220),
    ("minecraft:closed_eyeblossom", "minecraft:nausea", 140),
];

/// `SuspiciousEffectHolder.tryGet(item)` for a flower's item: its one entry,
/// as (effect id, ticks).
pub fn stew_effect(item: &str) -> Option<(&'static str, i32)> {
    FLOWER_STEW_EFFECTS
        .iter()
        .find(|(flower, _, _)| *flower == item)
        .map(|&(_, effect, ticks)| (effect, ticks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flower_maps_to_its_blocks_stew_effect() {
        // Read off `Blocks`'s constructor sites: 15 effect-bearing factories
        // (14 `FlowerBlock` sites plus `WitherRoseBlock`), then the two
        // eyeblossoms' `EyeblossomBlock.Type` values.
        let expected = [
            ("minecraft:dandelion", "minecraft:saturation", 7),
            ("minecraft:golden_dandelion", "minecraft:saturation", 7),
            ("minecraft:torchflower", "minecraft:night_vision", 100),
            ("minecraft:poppy", "minecraft:night_vision", 100),
            ("minecraft:blue_orchid", "minecraft:saturation", 7),
            ("minecraft:allium", "minecraft:fire_resistance", 60),
            ("minecraft:azure_bluet", "minecraft:blindness", 220),
            ("minecraft:red_tulip", "minecraft:weakness", 140),
            ("minecraft:orange_tulip", "minecraft:weakness", 140),
            ("minecraft:white_tulip", "minecraft:weakness", 140),
            ("minecraft:pink_tulip", "minecraft:weakness", 140),
            ("minecraft:oxeye_daisy", "minecraft:regeneration", 140),
            ("minecraft:cornflower", "minecraft:jump_boost", 100),
            ("minecraft:wither_rose", "minecraft:wither", 140),
            ("minecraft:lily_of_the_valley", "minecraft:poison", 220),
            ("minecraft:open_eyeblossom", "minecraft:blindness", 220),
            ("minecraft:closed_eyeblossom", "minecraft:nausea", 140),
        ];
        assert_eq!(FLOWER_STEW_EFFECTS, expected);
        assert_eq!(FLOWER_STEW_EFFECTS.len(), 17);
        for (flower, effect, ticks) in expected {
            assert_eq!(stew_effect(flower), Some((effect, ticks)), "{flower}");
        }
        let mut seen = std::collections::HashSet::new();
        for (flower, _, _) in FLOWER_STEW_EFFECTS {
            assert!(seen.insert(flower), "duplicate flower {flower}");
        }
        assert_eq!(stew_effect("minecraft:mushroom_stew"), None);
    }
}
