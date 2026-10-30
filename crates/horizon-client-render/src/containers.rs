//! Container block entities: chests/barrels (27 slots) and the three furnace
//! kinds, ticking as `AbstractFurnaceBlockEntity.serverTick` with fuel times
//! from the data-driven `context_int_provider`/`context_float_provider` files.
pub use crate::chunk::block_entity::{
    Banner, Beacon, BlockEntity, Brushable, Campfire, ChiseledBookshelf, Crafter, CreakingHeart,
    EndGateway, Furnace, FurnaceKind, Jukebox, Lectern, PotentSulfur, Shelf, Skull, SkullSkin,
};
use crate::items::ItemStack;
use std::collections::HashMap;

impl FurnaceKind {
    pub fn recipe_type(self) -> &'static str {
        match self {
            Self::Furnace => "minecraft:smelting",
            Self::BlastFurnace => "minecraft:blasting",
            Self::Smoker => "minecraft:smoking",
        }
    }
    pub fn fast(self) -> bool {
        self != Self::Furnace
    }
    pub fn from_block(name: &str) -> Option<Self> {
        match name {
            "minecraft:furnace" => Some(Self::Furnace),
            "minecraft:blast_furnace" => Some(Self::BlastFurnace),
            "minecraft:smoker" => Some(Self::Smoker),
            _ => None,
        }
    }
}
/// The key `recipes_used` keeps a recipe under: its full id, as vanilla's `ResourceKey<Recipe>`
/// writes it in `RecipesUsed`. Horizon's own recipe ids are file names without the namespace, so
/// keeping both forms would write a key twice, and vanilla refuses such a furnace
/// ("Duplicate entry for key").
pub fn recipe_used_key(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
/// A hopper's slots: the only plain container with five.
pub const HOPPER_SLOTS: usize = 5;

/// `getDefaultName` of the `BaseContainerBlockEntity` that `block` holds: the
/// translation key of its menu's title when it has no custom name. Copper
/// chests hold a `ChestBlockEntity`, shulker boxes of every colour a
/// `ShulkerBoxBlockEntity`.
pub fn default_name_key(block: &str) -> Option<&'static str> {
    Some(match block {
        "minecraft:chest" | "minecraft:trapped_chest" => "container.chest",
        n if n.ends_with("copper_chest") => "container.chest",
        "minecraft:barrel" => "container.barrel",
        n if n.ends_with("shulker_box") => "container.shulkerBox",
        "minecraft:dispenser" => "container.dispenser",
        "minecraft:dropper" => "container.dropper",
        "minecraft:hopper" => "container.hopper",
        "minecraft:furnace" => "container.furnace",
        "minecraft:smoker" => "container.smoker",
        "minecraft:blast_furnace" => "container.blast_furnace",
        "minecraft:brewing_stand" => "container.brewing",
        "minecraft:crafter" => "container.crafter",
        _ => return None,
    })
}

/// `BaseContainerBlockEntity.getDisplayName` (`getName`), the title of the
/// `ClientboundOpenScreenPacket` that opens it: the block entity's custom
/// name (`CustomName`, a literal), else its default name.
pub fn display_name(custom: Option<&str>, default_key: &str) -> horizon_world::nbt::Tag {
    match custom {
        Some(name) => horizon_protocol::text::literal(name),
        None => horizon_protocol::text::translatable(default_key, &[]),
    }
}

/// `ChestBlock.MENU_PROVIDER_COMBINER`'s `getDisplayName` for a double chest:
/// the first half's custom name (the right half, `DoubleBlockCombiner`'s
/// `FIRST`), else the second's, else `container.chestDouble`.
pub fn double_chest_name(first: Option<&str>, second: Option<&str>) -> horizon_world::nbt::Tag {
    display_name(first.or(second), "container.chestDouble")
}

impl Campfire {
    /// `placeFood`'s slot: the first empty one, given the item's
    /// `campfire_cooking` time; it takes one item of `held`.
    pub fn place_food(&mut self, held: &mut ItemStack, cooking_time: i32) -> bool {
        let Some(slot) = self.items.iter().position(Option::is_none) else {
            return false;
        };
        self.total[slot] = cooking_time;
        self.progress[slot] = 0;
        self.items[slot] = Some(ItemStack {
            count: 1,
            ..held.clone()
        });
        held.count = held.count.saturating_sub(1);
        true
    }
    /// `cookTick` (lit): each item's progress rises by one; the slots whose
    /// progress reached their cooking time give up their item (the caller
    /// drops its result).
    pub fn cook_tick(&mut self) -> Vec<(usize, ItemStack)> {
        let mut done = Vec::new();
        for i in 0..4 {
            if self.items[i].is_none() {
                continue;
            }
            self.progress[i] += 1;
            if self.progress[i] >= self.total[i]
                && let Some(item) = self.items[i].take()
            {
                done.push((i, item));
            }
        }
        done
    }
    /// `cooldownTick` (unlit): the progress falls by two, no lower than 0.
    /// Whether anything changed (`setChanged`).
    pub fn cooldown_tick(&mut self) -> bool {
        let mut changed = false;
        for i in 0..4 {
            if self.progress[i] > 0 {
                changed = true;
                self.progress[i] = (self.progress[i] - 2).clamp(0, self.total[i]);
            }
        }
        changed
    }
}
impl Skull {
    pub fn new(
        profile: Option<horizon_world::nbt::Tag>,
        note_block_sound: Option<String>,
        custom_name: Option<horizon_world::nbt::Tag>,
    ) -> Self {
        let skin = profile.as_ref().map(SkullSkin::of);
        Self {
            profile,
            note_block_sound,
            custom_name,
            skin,
        }
    }
}
impl SkullSkin {
    pub fn of(profile: &horizon_world::nbt::Tag) -> Self {
        let download = crate::skins::profile_skin_url_nbt(profile).and_then(|url| {
            let location = crate::skins::skin_location(&crate::skins::texture_hash(&url)?);
            Some((url, location))
        });
        // A `Dynamic` profile (a name or id only) resolves through the
        // services when the skull is drawn.
        let dynamic = crate::skins::dynamic_nbt(profile);
        // DefaultPlayerSkin.get(id), or getDefaultSkin (slim steve) without an id.
        let fallback = crate::skins::profile_uuid_nbt(profile)
            .or_else(|| {
                crate::skins::nbt_text(profile.get("name"))
                    .map(|name| horizon_protocol::crypto::offline_uuid(&name))
            })
            .map_or("entity/player/slim/steve", crate::tab_list::default_skin);
        Self {
            download,
            dynamic,
            fallback,
        }
    }
}
/// The skull and head blocks (`SkullBlock`, `WallSkullBlock`): every
/// `*_skull` and `*_head` but the piston's head.
fn is_skull(name: &str) -> bool {
    name.ends_with("_skull") || (name.ends_with("_head") && name != "minecraft:piston_head")
}
impl Crafter {
    /// `slotCanBeDisabled`: only empty slots.
    pub fn slot_can_be_disabled(&self, slot: usize) -> bool {
        slot < 9 && self.items[slot].is_none()
    }
    /// `setSlotState`.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        if self.slot_can_be_disabled(slot) {
            self.disabled[slot] = !enabled;
        }
    }
    /// `getRedstoneSignal`: filled or disabled slots.
    pub fn redstone_signal(&self) -> i32 {
        (0..9)
            .filter(|&i| self.items[i].is_some() || self.disabled[i])
            .count() as i32
    }
    /// `canPlaceItem`: a hopper fills the smallest stacks first.
    pub fn can_place(&self, slot: usize, _stack: &ItemStack, max_stack: u32) -> bool {
        if slot >= 9 || self.disabled[slot] {
            return false;
        }
        let Some(current) = &self.items[slot] else {
            return true;
        };
        if current.count >= max_stack {
            return false;
        }
        !(slot + 1..9).any(|i| {
            !self.disabled[i]
                && match &self.items[i] {
                    None => true,
                    Some(other) => {
                        other.count < current.count && other.same_item_same_components(current)
                    }
                }
        })
    }
}
impl BlockEntity {
    pub fn for_block(name: &str) -> Option<Self> {
        if let Some(hanging) = crate::signs::sign_kind(name) {
            return Some(Self::Sign(crate::signs::SignData {
                hanging,
                ..Default::default()
            }));
        }
        if let Some(kind) = FurnaceKind::from_block(name) {
            return Some(Self::Furnace(Furnace {
                kind,
                items: [None, None, None],
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 0,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }));
        }
        if matches!(
            name,
            "minecraft:suspicious_sand"
                | "minecraft:suspicious_gravel"
                | "minecraft:brushable_block"
        ) {
            return Some(Self::Brushable(Box::default()));
        }
        if name == "minecraft:creaking_heart" {
            return Some(Self::CreakingHeart(Box::default()));
        }
        if name == "minecraft:spawner" {
            return Some(Self::Spawner(Box::default()));
        }
        if name == "minecraft:trial_spawner" {
            return Some(Self::TrialSpawner(Box::default()));
        }
        if name == "minecraft:vault" {
            return Some(Self::Vault(Box::default()));
        }
        if name == "minecraft:beehive" || name == "minecraft:bee_nest" {
            return Some(Self::Beehive(Box::default()));
        }
        if name == "minecraft:lectern" {
            return Some(Self::Lectern(Lectern::default()));
        }
        if crate::command_block::Mode::of_block(name).is_some() {
            // CommandBlock.newBlockEntity: setAutomatic(this.automatic).
            return Some(Self::CommandBlock(Box::new(
                crate::command_block::CommandBlock {
                    auto: crate::command_block::automatic_block(name),
                    ..Default::default()
                },
            )));
        }
        if let Some(sculk) = crate::sculk::SculkEntity::for_block(name) {
            return Some(Self::Sculk(Box::new(sculk)));
        }
        if name == "minecraft:crafter" {
            // CrafterBlock.newBlockEntity copies the placed state's flag: the
            // first tick sees it as a change from untriggered.
            return Some(Self::Crafter(Crafter {
                state_triggered: Some(false),
                ..Crafter::default()
            }));
        }
        if name.ends_with("_banner") {
            return Some(Self::Banner(Banner::default()));
        }
        if is_skull(name) {
            return Some(Self::Skull(Skull::default()));
        }
        if name == "minecraft:end_gateway" {
            return Some(Self::EndGateway(EndGateway::default()));
        }
        if name == "minecraft:daylight_detector" {
            return Some(Self::DaylightDetector);
        }
        if name == "minecraft:beacon" {
            return Some(Self::Beacon(Box::new(Beacon {
                last_check_y: i32::MIN / 2,
                ..Default::default()
            })));
        }
        if name == "minecraft:brewing_stand" {
            return Some(Self::Brewing(Box::new(crate::brewing::BrewingStand::new())));
        }
        if name == "minecraft:campfire" || name == "minecraft:soul_campfire" {
            return Some(Self::Campfire(Box::default()));
        }
        if name == "minecraft:shelf" || name.ends_with("_shelf") {
            return Some(Self::Shelf(Box::default()));
        }
        if name == "minecraft:chiseled_bookshelf" {
            return Some(Self::ChiseledBookshelf(Box::default()));
        }
        if name == "minecraft:potent_sulfur" {
            return Some(Self::PotentSulfur(PotentSulfur::default()));
        }
        let size = match name {
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => 27,
            n if n.ends_with("copper_chest") => 27,
            "minecraft:hopper" => HOPPER_SLOTS,
            "minecraft:dispenser" | "minecraft:dropper" => 9,
            n if n.ends_with("shulker_box") => 27,
            _ => return None,
        };
        Some(Self::Chest(vec![None; size]))
    }
    pub fn id(&self) -> &'static str {
        match self {
            Self::Chest(items) => match items.len() {
                HOPPER_SLOTS => "minecraft:hopper",
                9 => "minecraft:dispenser",
                _ => "minecraft:chest",
            },
            Self::Furnace(f) => match f.kind {
                FurnaceKind::Furnace => "minecraft:furnace",
                FurnaceKind::BlastFurnace => "minecraft:blast_furnace",
                FurnaceKind::Smoker => "minecraft:smoker",
            },
            Self::Brewing(_) => "minecraft:brewing_stand",
            Self::Sign(s) => s.id(),
            Self::Jukebox(_) => "minecraft:jukebox",
            Self::Beacon(_) => "minecraft:beacon",
            Self::Lectern(_) => "minecraft:lectern",
            Self::Sculk(s) => s.id(),
            Self::Banner(_) => "minecraft:banner",
            Self::Crafter(_) => "minecraft:crafter",
            Self::CreakingHeart(_) => "minecraft:creaking_heart",
            Self::Spawner(_) => "minecraft:mob_spawner",
            Self::TrialSpawner(_) => "minecraft:trial_spawner",
            Self::Vault(_) => "minecraft:vault",
            Self::Beehive(_) => "minecraft:beehive",
            Self::CommandBlock(_) => "minecraft:command_block",
            Self::Skull(_) => "minecraft:skull",
            Self::EndGateway(_) => "minecraft:end_gateway",
            Self::DaylightDetector => "minecraft:daylight_detector",
            Self::Brushable(_) => "minecraft:brushable_block",
            Self::Campfire(_) => "minecraft:campfire",
            Self::ChiseledBookshelf(_) => "minecraft:chiseled_bookshelf",
            Self::Shelf(_) => "minecraft:shelf",
            Self::PotentSulfur(_) => "minecraft:potent_sulfur",
        }
    }
    pub fn contents(&self) -> Vec<ItemStack> {
        match self {
            Self::Chest(items) => items.iter().flatten().cloned().collect(),
            Self::Furnace(f) => f.items.iter().flatten().cloned().collect(),
            Self::Brewing(b) => b.items.iter().flatten().cloned().collect(),
            Self::Sign(_) => Vec::new(),
            Self::Jukebox(j) => j.record.iter().cloned().collect(),
            Self::Beacon(_)
            | Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_)
            | Self::EndGateway(_)
            | Self::Skull(_)
            | Self::PotentSulfur(_)
            | Self::DaylightDetector
            | Self::Brushable(_) => Vec::new(),
            Self::Lectern(l) => l.book.iter().cloned().collect(),
            Self::Sculk(_) => Vec::new(),
            Self::Banner(_) => Vec::new(),
            Self::Crafter(c) => c.items.iter().flatten().cloned().collect(),
            Self::Campfire(c) => c.items.iter().flatten().cloned().collect(),
            Self::ChiseledBookshelf(b) => b.items.iter().flatten().cloned().collect(),
            Self::Shelf(b) => b.items.iter().flatten().cloned().collect(),
        }
    }
    pub fn slots(&self) -> &[Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &f.items,
            Self::Brewing(b) => &b.items,
            Self::Sign(_)
            | Self::Jukebox(_)
            | Self::Beacon(_)
            | Self::Lectern(_)
            | Self::Sculk(_)
            | Self::Banner(_)
            | Self::EndGateway(_)
            | Self::Skull(_)
            | Self::PotentSulfur(_)
            | Self::DaylightDetector
            | Self::Brushable(_) => &[],
            Self::Crafter(c) => &c.items,
            Self::ChiseledBookshelf(b) => &b.items,
            Self::Shelf(b) => &b.items,
            Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_)
            // Not a Container (only Clearable): hoppers don't reach in.
            | Self::Campfire(_) => &[],
        }
    }
    pub fn slots_mut(&mut self) -> &mut [Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &mut f.items,
            Self::Brewing(b) => &mut b.items,
            Self::Sign(_)
            | Self::Jukebox(_)
            | Self::Beacon(_)
            | Self::Lectern(_)
            | Self::Sculk(_)
            | Self::Banner(_)
            | Self::EndGateway(_)
            | Self::Skull(_)
            | Self::PotentSulfur(_)
            | Self::DaylightDetector
            | Self::Brushable(_) => &mut [],
            Self::Crafter(c) => &mut c.items,
            Self::ChiseledBookshelf(b) => &mut b.items,
            Self::Shelf(b) => &mut b.items,
            Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_)
            | Self::Campfire(_) => &mut [],
        }
    }
}
pub use crate::providers::Providers;

/// A `ChestBlock`, which `ChestBlock.combine` can join into a double chest:
/// chests, trapped chests and copper chests in every weathering state, waxed
/// or not (`CopperChestBlock` extends `ChestBlock`). Ender chests are
/// `EnderChestBlock`s and always single.
pub fn is_chest_block(name: &str) -> bool {
    matches!(name, "minecraft:chest" | "minecraft:trapped_chest") || name.ends_with("copper_chest")
}

/// A chest block's name with its `type` and `facing` properties.
#[derive(Clone, Copy, Debug)]
pub struct ChestHalf<'a> {
    pub block: &'a str,
    pub kind: &'a str,
    pub facing: &'a str,
}

impl ChestHalf<'_> {
    /// `ChestBlock.getConnectedDirection` as an (x, z) step to the other
    /// half: clockwise from the facing for a left half, counter-clockwise for
    /// a right one. None for a single chest.
    pub fn connected_step(&self) -> Option<(i32, i32)> {
        let step = |facing: &str| match facing {
            "north" => Some((0, -1)),
            "south" => Some((0, 1)),
            "west" => Some((-1, 0)),
            "east" => Some((1, 0)),
            _ => None,
        };
        let (x, z) = step(self.facing)?;
        // Turning clockwise seen from above takes (x, z) to (-z, x).
        match self.kind {
            "left" => Some((-z, x)),
            "right" => Some((z, -x)),
            _ => None,
        }
    }

    /// `DoubleBlockCombiner.combineWithNeigbour` as `ChestBlock.combine` calls
    /// it: `other`, the block in this half's connected direction, is its other
    /// half when it is the same block, a half of the other side and faces the
    /// same way. Copper chests in different weathering states never join
    /// (`CopperChestBlock` keeps both halves in one state instead).
    pub fn pairs_with(&self, other: &ChestHalf) -> bool {
        is_chest_block(self.block)
            && self.kind != "single"
            && other.block == self.block
            && other.kind != "single"
            && other.kind != self.kind
            && other.facing == self.facing
    }
}

/// `AbstractFurnaceBlockEntity.createExperience`: `count * experience`, floored, and one
/// more with the chance of what is left over (a level random draw only when there is some).
pub fn create_experience(
    count: u32,
    experience: f32,
    random: &mut impl horizon_core::random::RandomSource,
) -> u32 {
    let total = count as f32 * experience;
    let whole = total.floor();
    let fraction = total - whole;
    let mut xp = whole.max(0.0) as u32;
    if fraction != 0.0 && random.next_float() < fraction {
        xp += 1;
    }
    xp
}

/// The `MenuType` the block entity of a one-block storage container opens
/// (`createMenu`): `ChestMenu.threeRows` for chests, trapped and copper chests
/// and barrels, `ShulkerBoxMenu` for shulker boxes of any colour (its
/// `ShulkerBoxSlot`s refuse shulker boxes), `DispenserMenu` for dispensers
/// and droppers, `HopperMenu` for hoppers. None for other blocks.
pub fn block_menu_type(block: &str) -> Option<&'static str> {
    Some(match block {
        "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => {
            "minecraft:generic_9x3"
        }
        n if n.ends_with("copper_chest") => "minecraft:generic_9x3",
        n if n.ends_with("shulker_box") => "minecraft:shulker_box",
        "minecraft:dispenser" | "minecraft:dropper" => "minecraft:generic_3x3",
        "minecraft:hopper" => "minecraft:hopper",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::{LegacyRandom, RandomSource};

    #[test]
    fn campfire_slots_take_food_cook_it_and_cool_down() {
        let mut c = Campfire::default();
        let mut beef = ItemStack::new("minecraft:beef", 2);
        // placeFood: the first free slot, one item, its recipe's time.
        assert!(c.place_food(&mut beef, 600));
        assert_eq!(beef.count, 1);
        assert_eq!(c.items[0].as_ref().map(|s| s.count), Some(1));
        assert_eq!((c.progress[0], c.total[0]), (0, 600));
        let mut fish = ItemStack::new("minecraft:cod", 1);
        assert!(c.place_food(&mut fish, 3));
        assert_eq!(c.items[1].as_ref().map(|s| &*s.item), Some("minecraft:cod"));
        // cookTick: the cod is done on its third tick, the beef keeps cooking.
        assert!(c.cook_tick().is_empty());
        assert!(c.cook_tick().is_empty());
        let done = c.cook_tick();
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0, &*done[0].1.item), (1, "minecraft:cod"));
        assert!(c.items[1].is_none());
        assert_eq!(c.progress[0], 3);
        // cooldownTick: two a tick, never below zero.
        assert!(c.cooldown_tick());
        assert_eq!(c.progress[0], 1);
        assert!(c.cooldown_tick());
        assert_eq!(c.progress[0], 0);
        // Full: four slots, no fifth.
        for _ in 0..3 {
            let mut more = ItemStack::new("minecraft:potato", 1);
            assert!(c.place_food(&mut more, 600));
        }
        let mut fifth = ItemStack::new("minecraft:potato", 1);
        assert!(!c.place_food(&mut fifth, 600));
        assert_eq!(fifth.count, 1);
    }

    /// A whole amount pays itself without a draw; a fraction is a chance of one more.
    #[test]
    fn create_experience_rounds_the_fraction_by_chance() {
        let mut random = LegacyRandom::new(7);
        let mut untouched = LegacyRandom::new(7);
        assert_eq!(create_experience(4, 0.5, &mut random), 2);
        assert_eq!(create_experience(3, 0.0, &mut random), 0);
        assert_eq!(random.next_long(), untouched.next_long());
        let paid: u32 = (0..1000)
            .map(|_| create_experience(1, 0.35, &mut random))
            .sum();
        assert!((250..450).contains(&paid), "{paid}");
        assert!((0..100).all(|_| (1..=2).contains(&create_experience(1, 1.7, &mut random))));
    }
}
