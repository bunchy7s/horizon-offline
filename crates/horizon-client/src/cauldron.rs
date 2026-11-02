//! `CauldronInteractions`: what an item does to an empty, water, lava or
//! powder snow cauldron. `tools/test_cauldron_parity.py` compares every item
//! with vanilla's `useItemOn`.
use crate::items::{ItemStack, Items};

/// `ItemTags.CAULDRON_CAN_REMOVE_DYE` membership.
pub fn removes_dye(items: &Items, item: &str) -> bool {
    items
        .item_tags
        .get("minecraft:cauldron_can_remove_dye")
        .is_some_and(|members| members.contains(item))
}
use horizon_world::chunk::BlockState;

/// The player side of an interaction.
pub trait Player {
    fn creative(&self) -> bool;
    /// The stack in the interacting hand.
    fn hand(&mut self) -> &mut Option<ItemStack>;
    /// `Inventory.add`; the leftover stays in `stack`.
    fn add(&mut self, stack: &mut ItemStack) -> bool;
    /// `Inventory.contains`.
    fn contains(&self, stack: &ItemStack) -> bool;
    fn drop_stack(&mut self, stack: ItemStack);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// `InteractionResult.SUCCESS`
    Success,
    /// `InteractionResult.CONSUME`
    Consume,
    /// `TRY_WITH_EMPTY_HAND`
    Pass,
}

#[derive(Debug)]
pub struct Interaction {
    pub outcome: Outcome,
    /// The cauldron's new state.
    pub block: Option<BlockState>,
    pub sound: Option<&'static str>,
    /// The custom statistic the player earns (`use_cauldron`, ...).
    pub stat: Option<&'static str>,
}

fn state(name: &str, level: Option<u8>) -> BlockState {
    BlockState {
        name: name.to_owned(),
        properties: level
            .map(|l| ("level".to_owned(), l.to_string()))
            .into_iter()
            .collect(),
    }
}

fn pass() -> Interaction {
    Interaction {
        outcome: Outcome::Pass,
        block: None,
        sound: None,
        stat: None,
    }
}

/// `ItemUtils.createFilledResult`.
fn filled(player: &mut dyn Player, new: ItemStack, limit_creative: bool) {
    let creative = player.creative();
    if limit_creative && creative {
        if !player.contains(&new) {
            let mut new = new;
            player.add(&mut new);
        }
        return;
    }
    let hand = player.hand();
    if !creative && let Some(held) = hand.as_mut() {
        held.count -= 1;
        if held.count == 0 {
            *hand = None;
        }
    }
    if hand.is_none() {
        *hand = Some(new);
        return;
    }
    let mut new = new;
    if !player.add(&mut new) {
        player.drop_stack(new);
    }
}

/// `LayeredCauldronBlock.handleEntityOnFireInside`: a burning entity in a
/// water cauldron lowers it a level; a powder snow cauldron becomes a water
/// cauldron of its level first, then lowers.
pub fn extinguished(cauldron: &BlockState) -> BlockState {
    if cauldron.name == "minecraft:powder_snow_cauldron" {
        let mut water = cauldron.clone();
        water.name = "minecraft:water_cauldron".into();
        lowered(&water)
    } else {
        lowered(cauldron)
    }
}

/// `LayeredCauldronBlock.lowerFillLevel`.
fn lowered(cauldron: &BlockState) -> BlockState {
    let level: u8 = cauldron
        .properties
        .get("level")
        .and_then(|l| l.parse().ok())
        .unwrap_or(1);
    if level <= 1 {
        state("minecraft:cauldron", None)
    } else {
        let mut next = cauldron.clone();
        next.properties
            .insert("level".into(), (level - 1).to_string());
        next
    }
}

fn changed(block: BlockState, sound: Option<&'static str>, stat: &'static str) -> Interaction {
    Interaction {
        outcome: Outcome::Success,
        block: Some(block),
        sound,
        stat: Some(stat),
    }
}

const COLORS: [&str; 16] = [
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

/// The cauldron interaction of the held item (`useItemOn`); the hand stack is
/// changed through `player`. `removes_dye` is whether the held item is in the
/// `cauldron_can_remove_dye` tag.
pub fn interact(
    cauldron: &BlockState,
    under_water: bool,
    removes_dye: bool,
    player: &mut dyn Player,
) -> Interaction {
    let Some(held) = player.hand().clone() else {
        return pass();
    };
    let kind = match cauldron.name.as_str() {
        "minecraft:cauldron" => "empty",
        "minecraft:water_cauldron" => "water",
        "minecraft:lava_cauldron" => "lava",
        "minecraft:powder_snow_cauldron" => "powder_snow",
        _ => return pass(),
    };
    let level: u8 = cauldron
        .properties
        .get("level")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let item = &*held.item;
    let water_potion = held.data().and_then(|d| d.potion.as_deref()) == Some("minecraft:water");
    // Dispatcher.get: tags first, then the item map.
    if kind == "water" && removes_dye {
        if held.data().and_then(|d| d.dyed_color).is_none() {
            return pass();
        }
        if let Some(stack) = player.hand().as_mut() {
            stack.data_mut().dyed_color = None;
            stack.tidy();
        }
        return changed(lowered(cauldron), None, "clean_armor");
    }
    let fill_bucket = |player: &mut dyn Player, new: &str, can_fill: bool, sound| {
        if !can_fill {
            return pass();
        }
        filled(player, ItemStack::new(new, 1), true);
        changed(
            state("minecraft:cauldron", None),
            Some(sound),
            "use_cauldron",
        )
    };
    let empty_bucket = |player: &mut dyn Player, block: BlockState, sound| {
        filled(player, ItemStack::new("minecraft:bucket", 1), true);
        changed(block, Some(sound), "fill_cauldron")
    };
    match item {
        "minecraft:lava_bucket" => {
            return if under_water {
                Interaction {
                    outcome: Outcome::Consume,
                    block: None,
                    sound: None,
                    stat: None,
                }
            } else {
                empty_bucket(
                    player,
                    state("minecraft:lava_cauldron", None),
                    "minecraft:item.bucket.empty_lava",
                )
            };
        }
        "minecraft:water_bucket" => {
            return empty_bucket(
                player,
                state("minecraft:water_cauldron", Some(3)),
                "minecraft:item.bucket.empty",
            );
        }
        "minecraft:powder_snow_bucket" => {
            return if under_water {
                Interaction {
                    outcome: Outcome::Consume,
                    block: None,
                    sound: None,
                    stat: None,
                }
            } else {
                empty_bucket(
                    player,
                    state("minecraft:powder_snow_cauldron", Some(3)),
                    "minecraft:item.bucket.empty_powder_snow",
                )
            };
        }
        _ => {}
    }
    match (kind, item) {
        ("empty", "minecraft:potion") => {
            if !water_potion {
                return pass();
            }
            filled(player, ItemStack::new("minecraft:glass_bottle", 1), true);
            changed(
                state("minecraft:water_cauldron", Some(1)),
                Some("minecraft:item.bottle.empty"),
                "use_cauldron",
            )
        }
        ("water", "minecraft:bucket") => fill_bucket(
            player,
            "minecraft:water_bucket",
            level == 3,
            "minecraft:item.bucket.fill",
        ),
        ("water", "minecraft:glass_bottle") => {
            filled(
                player,
                ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"),
                true,
            );
            changed(
                lowered(cauldron),
                Some("minecraft:item.bottle.fill"),
                "use_cauldron",
            )
        }
        ("water", "minecraft:potion") => {
            if level == 3 || !water_potion {
                return pass();
            }
            filled(player, ItemStack::new("minecraft:glass_bottle", 1), true);
            let mut next = cauldron.clone();
            // BlockState.cycle on a 1..=3 property
            next.properties
                .insert("level".into(), (level % 3 + 1).to_string());
            changed(next, Some("minecraft:item.bottle.empty"), "use_cauldron")
        }
        ("water", name)
            if name
                .strip_prefix("minecraft:")
                .and_then(|n| n.strip_suffix("_banner"))
                .is_some_and(|c| COLORS.contains(&c)) =>
        {
            let layers = held.data().map_or(0, |d| d.banner_patterns.len());
            if layers == 0 {
                return pass();
            }
            let mut cleaned = held.clone();
            cleaned.count = 1;
            cleaned.data_mut().banner_patterns.pop();
            cleaned.tidy();
            filled(player, cleaned, false);
            changed(lowered(cauldron), None, "clean_banner")
        }
        ("water", name)
            if name
                .strip_prefix("minecraft:")
                .and_then(|n| n.strip_suffix("_shulker_box"))
                .is_some_and(|c| COLORS.contains(&c)) =>
        {
            filled(player, held.transmute("minecraft:shulker_box", 1), false);
            changed(lowered(cauldron), None, "clean_shulker_box")
        }
        ("lava", "minecraft:bucket") => fill_bucket(
            player,
            "minecraft:lava_bucket",
            true,
            "minecraft:item.bucket.fill_lava",
        ),
        ("powder_snow", "minecraft:bucket") => fill_bucket(
            player,
            "minecraft:powder_snow_bucket",
            level == 3,
            "minecraft:item.bucket.fill_powder_snow",
        ),
        _ => pass(),
    }
}

/// What `interact` would answer for `held`, changing nothing: a client's
/// `useItemOn`, where every interaction checks the same conditions and acts
/// only `if (!level.isClientSide())`.
pub fn outcome(
    cauldron: &BlockState,
    under_water: bool,
    removes_dye: bool,
    held: &ItemStack,
) -> Outcome {
    struct Dry(Option<ItemStack>);
    impl Player for Dry {
        fn creative(&self) -> bool {
            false
        }
        fn hand(&mut self) -> &mut Option<ItemStack> {
            &mut self.0
        }
        fn add(&mut self, _: &mut ItemStack) -> bool {
            true
        }
        fn contains(&self, _: &ItemStack) -> bool {
            false
        }
        fn drop_stack(&mut self, _: ItemStack) {}
    }
    interact(
        cauldron,
        under_water,
        removes_dye,
        &mut Dry(Some(held.clone())),
    )
    .outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_sees_the_bucket_taken_without_acting() {
        let empty = state("minecraft:cauldron", None);
        let water = |l| state("minecraft:water_cauldron", Some(l));
        let stack = |id| ItemStack::new(id, 1);
        let outcome = |c: &BlockState, under_water, id| outcome(c, under_water, false, &stack(id));
        assert_eq!(
            outcome(&empty, false, "minecraft:water_bucket"),
            Outcome::Success
        );
        assert_eq!(
            outcome(&empty, false, "minecraft:lava_bucket"),
            Outcome::Success
        );
        // fillLavaInteraction under water: CONSUME, the bucket kept.
        assert_eq!(
            outcome(&empty, true, "minecraft:lava_bucket"),
            Outcome::Consume
        );
        assert_eq!(
            outcome(&water(3), false, "minecraft:bucket"),
            Outcome::Success
        );
        // fillBucket needs a full cauldron.
        assert_eq!(outcome(&water(2), false, "minecraft:bucket"), Outcome::Pass);
        assert_eq!(outcome(&empty, false, "minecraft:bucket"), Outcome::Pass);
        assert_eq!(outcome(&empty, false, "minecraft:stone"), Outcome::Pass);
    }

    #[test]
    fn a_burning_entity_lowers_water_and_melts_powder_snow() {
        let water = |l| state("minecraft:water_cauldron", Some(l));
        assert_eq!(extinguished(&water(3)), water(2));
        assert_eq!(extinguished(&water(1)), state("minecraft:cauldron", None));
        // Powder snow turns to water of its level, then lowers.
        assert_eq!(
            extinguished(&state("minecraft:powder_snow_cauldron", Some(3))),
            water(2)
        );
        assert_eq!(
            extinguished(&state("minecraft:powder_snow_cauldron", Some(1))),
            state("minecraft:cauldron", None)
        );
    }
}
