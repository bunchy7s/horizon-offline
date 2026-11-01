//! `AbstractContainerMenu.clicked` over vanilla's slot layouts: every
//! `ContainerInput` (pickup, quick move, swap, clone, throw, the three quick
//! craft drags, pickup all) for the player inventory, the crafting table, the
//! chest family, hoppers, dispensers, shulker boxes, the furnaces, the brewing
//! stand, the crafter (its client menu and the block entity's server menu with
//! `setSlotState`), the loom (`clickMenuButton`, `slotsChanged`), the
//! stonecutter (its recipe buttons and result) and the merchant (`MerchantMenu` over a `merchant::Session`: payment slots, the
//! result slot's trade, `tryMoveItems`).
//! `tools/test_menu_click_parity.py` compares every click with vanilla's menu
//! classes.
use crate::{
    banner::{self, Banners},
    brewing::Brewing,
    crafting::Recipes,
    items::{ItemStack, Items},
};
use std::sync::Arc;

mod bundle;
pub mod place;

/// What a menu needs to know about items and recipes.
pub struct Env<'a> {
    pub items: &'a Items,
    pub recipes: &'a Recipes,
    pub brewing: &'a Brewing,
    pub banners: &'a Banners,
    /// The stonecutter's recipes.
    pub station_recipes: &'a crate::stations::StationRecipes,
    /// The grindstone's and the anvil's enchantment rules.
    pub enchantments: &'a crate::enchant::Enchantments,
    /// Item display names: the anvil compares the rename box with the
    /// input's hover name.
    pub names: &'a crate::creative::Names,
    /// What a merchant menu needs for offers made while it is open (level-ups).
    pub trade: Option<&'a crate::merchant::TradeEnv<'a>>,
    /// The level's maps (`ContainerLevelAccess`): the cartography table reads
    /// and makes them. None is a client's menu (`ContainerLevelAccess.NULL`):
    /// it makes no result, and a taken result keeps its map id.
    pub maps: Option<&'a dyn Maps>,
}

/// The maps of the level a menu is in.
pub trait Maps {
    /// `MapItem.getSavedData`: whether map `id` is locked, and its scale;
    /// None when the level has no such map.
    fn state(&self, id: i32) -> Option<(bool, u8)>;
    /// `MapItem.scaleMap` (`scale`) or `lockMap`: the copy of map `id` the
    /// level makes under a new id (`getFreeMapId`), None without the map.
    fn derive(&self, id: i32, scale: bool) -> Option<i32>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Pickup,
    QuickMove,
    Swap,
    Clone,
    Throw,
    QuickCraft,
    PickupAll,
}

impl Input {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "PICKUP" => Self::Pickup,
            "QUICK_MOVE" => Self::QuickMove,
            "SWAP" => Self::Swap,
            "CLONE" => Self::Clone,
            "THROW" => Self::Throw,
            "QUICK_CRAFT" => Self::QuickCraft,
            "PICKUP_ALL" => Self::PickupAll,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Inventory,
    Crafting,
    Chest(usize),
    Hopper,
    Dispenser,
    Shulker,
    /// The cooking recipe type: smelting, blasting or smoking.
    Furnace(&'static str),
    Brewing,
    /// `CrafterMenu` over a plain crafting container (the client's menu).
    Crafter,
    /// `CrafterMenu` over the `CrafterBlockEntity` (the server's menu): the
    /// result follows the grid, and filling a disabled slot enables it.
    CrafterBlock,
    Loom,
    /// `StonecutterMenu`: the input, the result, then the inventory.
    Stonecutter,
    /// `GrindstoneMenu`: two inputs, the result, then the inventory.
    Grindstone,
    /// `AnvilMenu`: two inputs, the result, then the inventory.
    Anvil,
    /// `SmithingMenu`: template, base, addition, the result, then the inventory.
    Smithing,
    /// `EnchantmentMenu`: the item, the lapis, then the inventory; its data
    /// slots are the three costs, the seed and the clues.
    Enchanting,
    /// `BeaconMenu`: the payment slot, then the inventory; its data slots are
    /// the pyramid's levels and the two powers (`encodeEffect`: id + 1).
    Beacon,
    /// `CartographyTableMenu`: the map, the paper / map / glass pane, the
    /// result, then the inventory.
    Cartography,
    /// `LecternMenu` over the `LecternBlockEntity`: the book's one slot (no
    /// inventory slots); its data slot is the open page.
    Lectern,
    /// `MerchantMenu`: two payment slots, the result, then the inventory.
    Merchant,
    /// `HorseInventoryMenu`: the saddle and body armour slots (`ArmorSlot`s
    /// of the mount, whose entity type is `Menu::mount`), a chest of three
    /// rows of this many columns (`getInventoryColumns`), then the inventory.
    Mount(usize),
    /// `CreativeModeInventoryScreen.ItemPickerMenu`: an item tab's 45
    /// display slots, then the hotbar.
    ItemPicker,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        if let Some(rows) = name.strip_prefix("chest") {
            return rows.parse().ok().map(Self::Chest);
        }
        if let Some(columns) = name.strip_prefix("mount") {
            return columns.parse().ok().map(Self::Mount);
        }
        Some(match name {
            "inventory" => Self::Inventory,
            "crafting" => Self::Crafting,
            "hopper" => Self::Hopper,
            "dispenser" => Self::Dispenser,
            "shulker" => Self::Shulker,
            "furnace" => Self::Furnace("smelting"),
            "blast_furnace" => Self::Furnace("blasting"),
            "smoker" => Self::Furnace("smoking"),
            "brewing" => Self::Brewing,
            "crafter" => Self::Crafter,
            "crafter_be" => Self::CrafterBlock,
            "loom" => Self::Loom,
            "stonecutter" => Self::Stonecutter,
            "grindstone" => Self::Grindstone,
            "anvil" => Self::Anvil,
            "smithing" => Self::Smithing,
            "enchanting" => Self::Enchanting,
            "beacon" => Self::Beacon,
            "cartography" => Self::Cartography,
            "lectern" => Self::Lectern,
            "merchant" => Self::Merchant,
            "item_picker" => Self::ItemPicker,
            _ => return None,
        })
    }
}

/// `Slot` subclasses that change the rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    /// `ArmorSlot` for the equipment slot with this `EquipmentSlot.getIndex`.
    Armor(usize),
    Shulker,
    Fuel,
    /// `FurnaceResultSlot`.
    Output,
    /// `ResultSlot` of a crafting grid.
    CraftResult,
    /// `CrafterSlot`: refuses items while its disabled flag (data slot) is set.
    CrafterIn,
    /// `NonInteractiveResultSlot`.
    Inert,
    /// `BrewingStandMenu.PotionSlot`.
    Potion,
    /// `BrewingStandMenu.IngredientsSlot`.
    Ingredient,
    /// `BrewingStandMenu.FuelSlot`.
    BrewFuel,
    /// The loom's banner, dye and pattern slots.
    LoomBanner,
    LoomDye,
    LoomPattern,
    /// The loom's result slot (a plain slot whose `onTake` uses the inputs).
    LoomResult,
    /// `MerchantResultSlot`.
    MerchantResult,
    /// The stonecutter's result slot (`onTake` uses up one input).
    StoneResult,
    /// The grindstone's inputs: damageable or enchanted items.
    GrindInput,
    /// The grindstone's result slot (`onTake` clears both inputs).
    GrindResult,
    /// A smithing table input: its recipe property set (template, base, addition).
    SmithInput(usize),
    /// `ItemCombinerMenu`'s result slot (the anvil's and the smithing table's).
    CombineResult,
    /// The enchanting table's item slot (one item).
    EnchantItem,
    /// The enchanting table's lapis slot.
    Lapis,
    /// `BeaconMenu.PaymentSlot`: one beacon payment item.
    BeaconPay,
    /// The cartography table's map slot (anything with a map id).
    CartoMap,
    /// Its paper, empty map or glass pane slot.
    CartoExtra,
    /// Its result (`onTake`: one of each input, the map post-processed).
    CartoResult,
    /// `CustomCreativeSlot`: a creative tab's display slot, which the
    /// screen clicks itself; drags and PICKUP_ALL pass it by.
    CreativeDisplay,
    /// An `ArmorSlot` of a mount for this `EquipmentSlot` ("saddle" or
    /// "body"): `isEquippableInSlot` for `Menu::mount`, one item.
    MountEquip(&'static str),
}

#[derive(Clone, Copy)]
struct Slot {
    container: usize,
    index: usize,
    role: Role,
}

/// `BeaconBlockEntity.BEACON_EFFECTS`: the powers of each pyramid level.
pub const BEACON_EFFECTS: [&[&str]; 4] = [
    &["minecraft:speed", "minecraft:haste"],
    &["minecraft:resistance", "minecraft:jump_boost"],
    &["minecraft:strength"],
    &["minecraft:regeneration"],
];

/// `BeaconBlockEntity.getRequiredLevelsFor`: 0 for no power, `i32::MAX`
/// for one no level gives.
pub fn beacon_required_levels(effect: Option<&str>) -> i32 {
    let Some(effect) = effect else {
        return 0;
    };
    BEACON_EFFECTS
        .iter()
        .position(|tier| tier.contains(&effect))
        .map_or(i32::MAX, |i| i as i32 + 1)
}

/// `Item.canFitInsideContainerItems` (`ShulkerBoxSlot.mayPlace`): shulker
/// boxes cannot nest.
pub fn fits_inside_container_items(stack: &ItemStack) -> bool {
    !stack.item.ends_with("shulker_box")
}

/// The containers a menu spans.
const PLAYER: usize = 0;
const MAIN: usize = 1;
const GRID: usize = 2;
const RESULT: usize = 3;
/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u32 = 99;
const OFFHAND: usize = 40;

pub struct Menu {
    kind: Kind,
    slots: Vec<Slot>,
    containers: [Vec<Option<ItemStack>>; 4],
    grid_size: usize,
    pub carried: Option<ItemStack>,
    pub data: Vec<i32>,
    pub creative: bool,
    /// The selected hotbar slot (`Inventory.selected`).
    pub selected: usize,
    /// Dropped stacks (canonical text), `xp+N` and the like, in order.
    pub drops: Vec<String>,
    /// The dropped stacks themselves, for the caller to throw.
    pub dropped: Vec<ItemStack>,
    quick_status: i32,
    quick_type: i32,
    quick_slots: Vec<usize>,
    /// `LoomMenu.selectablePatterns`.
    pub loom_patterns: Vec<Arc<str>>,
    /// Sounds the menu plays (`ui.loom.take_result`), for the caller.
    pub sounds: Vec<&'static str>,
    /// Sounds the player makes (a bundle's insert and remove), for the
    /// caller to play at the player as `Entity.playSound`.
    pub player_sounds: Vec<&'static str>,
    /// Recipes whose result was taken (`awardUsedRecipes`) and the stacks that
    /// went into each, for the caller.
    pub crafted: Vec<(String, Vec<Option<ItemStack>>)>,
    /// Potions taken from a brewing stand's bottle slots
    /// (`PotionSlot.onTake`), for the caller.
    pub brewed: Vec<ItemStack>,
    /// `LivingEntity.onEquipItem` calls that change the item: the
    /// `EquipmentSlot` name and the new stack, in order, for the caller.
    pub equipped: Vec<(&'static str, Option<ItemStack>)>,
    /// The merchant a merchant menu trades with.
    pub merchant: Option<Box<crate::merchant::Session>>,
    /// The stonecutter's recipe list (`recipesForInput`).
    pub stone: Stonecutter,
    /// The grindstone result's `getExperienceAmount` sum before its roll,
    /// set when the result was taken: the caller (with a level) awards the
    /// orbs and plays level event 1042.
    pub ground: Option<i32>,
    /// `Player.experienceLevel`, which the anvil's result needs.
    pub levels: i32,
    /// The levels the menu gave the player (`giveExperienceLevels`), for the caller.
    pub level_change: i32,
    /// `AnvilMenu`'s state besides its slots and its cost data slot.
    pub anvil: Anvil,
    /// The anvil's or the smithing table's result was taken: the caller (with
    /// a level) runs the rest of its `onTake` (the anvil's wear and level
    /// event 1030 or 1029, the smithing table's 1044).
    pub combined: bool,
    /// The enchanting table's item changed to an enchantable one: the caller
    /// (with a level) fills the costs and clues (`slotsChanged`'s
    /// `access.execute`).
    pub enchant_changed: bool,
    /// An enchanting button the menu took (`clickMenuButton` passed its
    /// checks): the caller (with a level) enchants.
    pub enchant_button: Option<usize>,
    /// `Player.mayBuild` (not adventure mode): the lectern's Take Book.
    pub may_build: bool,
    /// A mount menu's entity type (`minecraft:horse`): what its saddle and
    /// body slots take.
    pub mount: Option<String>,
}

/// `AnvilMenu`'s state besides its slots and `cost`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Anvil {
    /// `itemName`: the rename box's text (`ServerboundRenameItemPacket`).
    pub name: Option<String>,
    /// `repairItemCountCost`.
    pub repair_count: u32,
    /// `onlyRenaming`.
    pub only_renaming: bool,
}

/// `StonecutterMenu`'s state besides its slots and `selectedRecipeIndex`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Stonecutter {
    /// `recipesForInput`: indices into `StationRecipes.stonecutting`.
    pub recipes: Vec<usize>,
    /// The item the list was made for (`input`; None for nothing).
    pub input: Option<Arc<str>>,
    /// `ResultContainer.recipeUsed`.
    pub used: Option<usize>,
}

fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item
}

fn split(stack: &mut ItemStack, amount: u32) -> ItemStack {
    let real = amount.min(stack.count);
    let mut out = stack.clone();
    out.count = real;
    stack.count -= real;
    out
}

/// `LecternBlockEntity.getPageCount`: a written book's pages, else a
/// writable book's.
pub fn book_pages(stack: &ItemStack) -> usize {
    stack.data().map_or(0, |d| match &d.written {
        Some(w) => w.pages.len(),
        None => d.writable_pages.len(),
    })
}

fn tidy(slot: &mut Option<ItemStack>) {
    if slot.as_ref().is_some_and(|s| s.count == 0) {
        *slot = None;
    }
}

/// An index outside the slot list: vanilla's `IndexOutOfBoundsException`.
pub const OUT_OF_BOUNDS: &str = "IndexOutOfBoundsException";

impl Menu {
    pub fn new(kind: Kind) -> Self {
        let mut containers: [Vec<Option<ItemStack>>; 4] = Default::default();
        containers[PLAYER] = vec![None; 41];
        let mut slots = Vec::new();
        let mut grid_size = 0;
        let main = |n: usize, role: Role| -> Vec<Slot> {
            (0..n)
                .map(|index| Slot {
                    container: MAIN,
                    index,
                    role,
                })
                .collect()
        };
        let player = |slots: &mut Vec<Slot>| {
            for i in (9..36).chain(0..9) {
                slots.push(Slot {
                    container: PLAYER,
                    index: i,
                    role: Role::Plain,
                });
            }
        };
        let grid = |slots: &mut Vec<Slot>, n: usize| {
            slots.push(Slot {
                container: RESULT,
                index: 0,
                role: Role::CraftResult,
            });
            for index in 0..n * n {
                slots.push(Slot {
                    container: GRID,
                    index,
                    role: Role::Plain,
                });
            }
        };
        let mut data = 0;
        match kind {
            Kind::Inventory => {
                grid_size = 2;
                grid(&mut slots, 2);
                for (i, eq) in [3usize, 2, 1, 0].into_iter().enumerate() {
                    slots.push(Slot {
                        container: PLAYER,
                        index: 39 - i,
                        role: Role::Armor(eq),
                    });
                }
                player(&mut slots);
                slots.push(Slot {
                    container: PLAYER,
                    index: OFFHAND,
                    role: Role::Plain,
                });
            }
            Kind::Crafting => {
                grid_size = 3;
                grid(&mut slots, 3);
                player(&mut slots);
            }
            Kind::Chest(rows) => {
                containers[MAIN] = vec![None; rows * 9];
                slots.extend(main(rows * 9, Role::Plain));
                player(&mut slots);
            }
            Kind::Hopper => {
                containers[MAIN] = vec![None; 5];
                slots.extend(main(5, Role::Plain));
                player(&mut slots);
            }
            Kind::Dispenser => {
                containers[MAIN] = vec![None; 9];
                slots.extend(main(9, Role::Plain));
                player(&mut slots);
            }
            Kind::Shulker => {
                containers[MAIN] = vec![None; 27];
                slots.extend(main(27, Role::Shulker));
                player(&mut slots);
            }
            Kind::Crafter | Kind::CrafterBlock => {
                containers[MAIN] = vec![None; 9];
                slots.extend(main(9, Role::CrafterIn));
                player(&mut slots);
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::Inert,
                });
                data = 10;
            }
            Kind::Loom => {
                containers[MAIN] = vec![None; 3];
                for (index, role) in [Role::LoomBanner, Role::LoomDye, Role::LoomPattern]
                    .into_iter()
                    .enumerate()
                {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::LoomResult,
                });
                player(&mut slots);
                data = 1;
            }
            Kind::Stonecutter => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::StoneResult,
                });
                player(&mut slots);
                // selectedRecipeIndex: DataSlot.standalone() starts at 0.
                data = 1;
            }
            Kind::Grindstone => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::GrindInput));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::GrindResult,
                });
                player(&mut slots);
            }
            Kind::Anvil => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CombineResult,
                });
                player(&mut slots);
                // cost: DataSlot.standalone() starts at 0.
                data = 1;
            }
            Kind::Smithing => {
                containers[MAIN] = vec![None; 3];
                slots.extend((0..3).map(|index| Slot {
                    container: MAIN,
                    index,
                    role: Role::SmithInput(index),
                }));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CombineResult,
                });
                player(&mut slots);
                // hasRecipeError.
                data = 1;
            }
            Kind::Enchanting => {
                containers[MAIN] = vec![None; 2];
                slots.push(Slot {
                    container: MAIN,
                    index: 0,
                    role: Role::EnchantItem,
                });
                slots.push(Slot {
                    container: MAIN,
                    index: 1,
                    role: Role::Lapis,
                });
                player(&mut slots);
                // costs[3], enchantmentSeed, enchantClue[3], levelClue[3].
                data = 10;
            }
            Kind::Beacon => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::BeaconPay));
                player(&mut slots);
                // levels, primary, secondary.
                data = 3;
            }
            Kind::Cartography => {
                containers[MAIN] = vec![None; 2];
                for (index, role) in [Role::CartoMap, Role::CartoExtra].into_iter().enumerate() {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::CartoResult,
                });
                player(&mut slots);
            }
            Kind::Lectern => {
                containers[MAIN] = vec![None; 1];
                slots.extend(main(1, Role::Plain));
                // The open page (LecternBlockEntity.dataAccess).
                data = 1;
            }
            Kind::Mount(columns) => {
                containers[MAIN] = vec![None; 2 + columns * 3];
                for (index, slot) in ["saddle", "body"].into_iter().enumerate() {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role: Role::MountEquip(slot),
                    });
                }
                slots.extend((2..2 + columns * 3).map(|index| Slot {
                    container: MAIN,
                    index,
                    role: Role::Plain,
                }));
                player(&mut slots);
            }
            Kind::ItemPicker => {
                containers[MAIN] = vec![None; 45];
                slots.extend(main(45, Role::CreativeDisplay));
                // addInventoryHotbarSlots.
                slots.extend((0..9).map(|index| Slot {
                    container: PLAYER,
                    index,
                    role: Role::Plain,
                }));
            }
            Kind::Merchant => {
                containers[MAIN] = vec![None; 2];
                slots.extend(main(2, Role::Plain));
                slots.push(Slot {
                    container: RESULT,
                    index: 0,
                    role: Role::MerchantResult,
                });
                player(&mut slots);
            }
            Kind::Brewing => {
                containers[MAIN] = vec![None; 5];
                for (index, role) in [
                    Role::Potion,
                    Role::Potion,
                    Role::Potion,
                    Role::Ingredient,
                    Role::BrewFuel,
                ]
                .into_iter()
                .enumerate()
                {
                    slots.push(Slot {
                        container: MAIN,
                        index,
                        role,
                    });
                }
                player(&mut slots);
                data = 4;
            }
            Kind::Furnace(_) => {
                containers[MAIN] = vec![None; 3];
                slots.extend([
                    Slot {
                        container: MAIN,
                        index: 0,
                        role: Role::Plain,
                    },
                    Slot {
                        container: MAIN,
                        index: 1,
                        role: Role::Fuel,
                    },
                    Slot {
                        container: MAIN,
                        index: 2,
                        role: Role::Output,
                    },
                ]);
                player(&mut slots);
                data = 4;
            }
        }
        containers[GRID] = vec![None; grid_size * grid_size];
        containers[RESULT] = vec![None; 1];
        Self {
            kind,
            slots,
            containers,
            grid_size,
            carried: None,
            data: vec![0; data],
            creative: false,
            selected: 0,
            drops: Vec::new(),
            dropped: Vec::new(),
            quick_status: 0,
            quick_type: 0,
            quick_slots: Vec::new(),
            loom_patterns: Vec::new(),
            sounds: Vec::new(),
            player_sounds: Vec::new(),
            crafted: Vec::new(),
            brewed: Vec::new(),
            equipped: Vec::new(),
            merchant: None,
            stone: Stonecutter::default(),
            ground: None,
            levels: 0,
            level_change: 0,
            anvil: Anvil::default(),
            combined: false,
            enchant_changed: false,
            enchant_button: None,
            may_build: true,
            mount: None,
        }
        .with_initial_data()
    }

    /// Data slots that do not start at 0: the enchanting table's clues are -1.
    fn with_initial_data(mut self) -> Self {
        if self.kind == Kind::Enchanting {
            self.data[4..].fill(-1);
        }
        self
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// `clickMenuButton`: the loom's pattern buttons and the stonecutter's
    /// recipes. False when refused.
    pub fn button(&mut self, env: &Env, id: i32) -> bool {
        if self.kind == Kind::Lectern {
            return self.lectern_button(env, id);
        }
        if self.kind == Kind::Enchanting {
            return self.enchant_click(id);
        }
        if self.kind == Kind::Stonecutter {
            // StonecutterMenu.clickMenuButton: an invalid index is taken too.
            if self.data[0] == id {
                return false;
            }
            if id >= 0 && (id as usize) < self.stone.recipes.len() {
                self.data[0] = id;
                self.stone_setup_result(env, id);
            }
            return true;
        }
        if self.kind != Kind::Loom || id < 0 || id as usize >= self.loom_patterns.len() {
            return false;
        }
        self.data[0] = id;
        let pattern = self.loom_patterns[id as usize].clone();
        self.loom_setup_result(env, &pattern);
        true
    }

    /// `LecternMenu.clickMenuButton`: the page buttons (1 back, 2 forward,
    /// 100 + n to page n) and Take Book (3), which puts the book in the
    /// inventory or drops it.
    fn lectern_button(&mut self, env: &Env, id: i32) -> bool {
        match id {
            100.. => self.set_data(0, id - 100),
            1 => self.set_data(0, self.data[0] - 1),
            2 => self.set_data(0, self.data[0] + 1),
            3 if self.may_build => {
                // removeItemNoUpdate: the whole book, and onBookItemRemove.
                let book = self.containers[MAIN][0].take();
                self.data[0] = 0;
                if let Some(mut book) = book
                    && !self.inventory_add(env, &mut book)
                {
                    self.drop_stack(&book);
                }
            }
            _ => return false,
        }
        true
    }

    /// `AbstractContainerMenu.setData`: the lectern's is
    /// `LecternBlockEntity.setPage`, kept within the book's pages.
    pub fn set_data(&mut self, id: usize, value: i32) {
        if self.kind == Kind::Lectern && id == 0 {
            // Mth.clamp(page, 0, pageCount - 1).
            let count = self.containers[MAIN][0].as_ref().map_or(0, book_pages) as i32;
            self.data[0] = value.max(0).min(count - 1);
            return;
        }
        self.data[id] = value;
    }

    /// `CrafterBlockEntity.setSlotState` (the slot state packet): only an
    /// empty slot can be disabled or enabled.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        if self.kind == Kind::CrafterBlock && slot < 9 && self.containers[MAIN][slot].is_none() {
            self.data[slot] = i32::from(!enabled);
        }
    }

    /// The menu after a fixture set its slots: the block crafter's slot
    /// listener has refreshed the result (`broadcastChanges`).
    pub fn settle(&mut self, env: &Env) {
        if self.kind == Kind::CrafterBlock {
            self.crafter_result(env);
        }
    }

    /// `CrafterMenu.refreshRecipeResult`.
    fn crafter_result(&mut self, env: &Env) {
        self.containers[RESULT][0] = env
            .recipes
            .find(&self.containers[MAIN], 3)
            .map(|(_, stack)| stack)
            .filter(|s| s.count > 0);
    }

    /// `LoomMenu.setupResultSlot`.
    fn loom_setup_result(&mut self, env: &Env, pattern: &str) {
        let mut result = None;
        if let (Some(banner_stack), Some(dye)) = (
            self.containers[MAIN][0].as_ref(),
            self.containers[MAIN][1].as_ref(),
        ) && let Some(color) = env.banners.loom_dye(env.items, dye)
        {
            result = Some(banner::loom_result(banner_stack, &color, pattern));
        }
        self.containers[RESULT][0] = result;
    }

    /// `LoomMenu.slotsChanged`.
    fn loom_changed(&mut self, env: &Env) {
        let has_inputs = self.containers[MAIN][0].is_some() && self.containers[MAIN][1].is_some();
        if !has_inputs {
            self.containers[RESULT][0] = None;
            self.loom_patterns.clear();
            self.data[0] = -1;
            return;
        }
        let selected = self.data[0];
        let valid = selected >= 0 && (selected as usize) < self.loom_patterns.len();
        let pattern_item = self.containers[MAIN][2].clone();
        let previous = std::mem::replace(
            &mut self.loom_patterns,
            env.banners.selectable(pattern_item.as_ref()),
        );
        let shown = if self.loom_patterns.len() == 1 {
            self.data[0] = 0;
            Some(self.loom_patterns[0].clone())
        } else if !valid {
            self.data[0] = -1;
            None
        } else {
            let value = previous[selected as usize].clone();
            match self.loom_patterns.iter().position(|p| *p == value) {
                Some(i) => {
                    self.data[0] = i as i32;
                    Some(value)
                }
                None => {
                    self.data[0] = -1;
                    None
                }
            }
        };
        match shown {
            Some(pattern) => {
                let full = self.containers[MAIN][0]
                    .as_ref()
                    .is_some_and(banner::has_max_layers);
                if full {
                    self.data[0] = -1;
                    self.containers[RESULT][0] = None;
                } else {
                    self.loom_setup_result(env, &pattern);
                }
            }
            None => self.containers[RESULT][0] = None,
        }
    }

    /// `StonecutterMenu.slotsChanged`: a new input item lists its recipes
    /// (`setupRecipeList`), with none selected and no result.
    fn stone_changed(&mut self, env: &Env) {
        let input = self.containers[MAIN][0].as_ref().map(|s| s.item.clone());
        if input == self.stone.input {
            return;
        }
        self.stone.input = input;
        self.data[0] = -1;
        self.containers[RESULT][0] = None;
        self.stone.recipes = match &self.containers[MAIN][0] {
            Some(stack) => env
                .station_recipes
                .stonecutting
                .iter()
                .enumerate()
                .filter(|(_, r)| r.input.test(stack))
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        };
    }

    /// `StonecutterMenu.setupResultSlot`.
    fn stone_setup_result(&mut self, env: &Env, index: i32) {
        let recipe = usize::try_from(index)
            .ok()
            .and_then(|i| self.stone.recipes.get(i).copied());
        self.stone.used = recipe;
        self.containers[RESULT][0] =
            recipe.map(|r| env.station_recipes.stonecutting[r].result.clone());
    }

    /// `Slot.setChanged` on slot `s`: the loom recomputes from its inputs,
    /// the stonecutter lists its recipes, the block crafter refreshes its
    /// result.
    fn slot_changed(&mut self, env: &Env, s: usize) {
        // MerchantContainer.setChanged: every slot of it, the result too.
        if self.kind == Kind::Merchant && self.slots[s].container != PLAYER {
            self.merchant_update(env);
            return;
        }
        // SmithingMenu.slotsChanged runs for its result container too.
        if self.kind == Kind::Smithing && self.slots[s].container == RESULT {
            self.smith_error();
            return;
        }
        // CartographyTableMenu: its result container calls slotsChanged too.
        if self.kind == Kind::Cartography && self.slots[s].container != PLAYER {
            self.carto_changed(env);
            return;
        }
        if self.slots[s].container != MAIN {
            return;
        }
        match self.kind {
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            // GrindstoneMenu.createResult.
            Kind::Grindstone => {
                self.containers[RESULT][0] = crate::stations::grindstone_result(
                    env.enchantments,
                    env.items,
                    self.containers[MAIN][0].as_ref(),
                    self.containers[MAIN][1].as_ref(),
                );
            }
            Kind::CrafterBlock => self.crafter_result(env),
            Kind::Anvil => self.anvil_result(env),
            Kind::Enchanting => self.enchant_changed(env),
            Kind::Smithing => {
                self.smith_result(env);
                self.smith_error();
            }
            _ => {}
        }
    }

    /// `slotsChanged(player.getInventory())`, which a bundle click calls
    /// (`BundleItem.broadcastChangesOnContainerMenu`): the menus that don't
    /// check which container changed recompute from their inputs.
    fn inventory_slots_changed(&mut self, env: &Env) {
        match self.kind {
            Kind::Inventory | Kind::Crafting => self.recompute(env),
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            Kind::Cartography => self.carto_changed(env),
            Kind::Merchant => self.merchant_update(env),
            // ItemCombinerMenu.slotsChanged only for its inputs; the
            // smithing table's hasRecipeError always.
            Kind::Smithing => self.smith_error(),
            _ => {}
        }
    }

    /// `EnchantmentMenu.clickMenuButton`'s checks: the lapis, the cost and
    /// the levels (creative pays nothing). The enchanting itself needs a
    /// level and is the caller's (`enchant_button`).
    fn enchant_click(&mut self, id: i32) -> bool {
        let Some(i) = usize::try_from(id).ok().filter(|&i| i < 3) else {
            return false;
        };
        let need = id + 1;
        let lapis = self.containers[MAIN][1]
            .as_ref()
            .map_or(0, |l| l.count as i32);
        if lapis < need && !self.creative {
            return false;
        }
        if self.data[i] <= 0
            || self.containers[MAIN][0].is_none()
            || (self.levels < need || self.levels < self.data[i]) && !self.creative
        {
            return false;
        }
        self.enchant_button = Some(i);
        true
    }

    /// `EnchantmentMenu.slotsChanged`: no costs or clues without an
    /// enchantable item; with one, a level fills them (`enchant_changed`).
    fn enchant_changed(&mut self, env: &Env) {
        let enchantable = self.containers[MAIN][0].as_ref().is_some_and(|s| {
            env.enchantments.enchantable(&s.item) > 0 && s.enchantments().is_empty()
        });
        if enchantable {
            self.enchant_changed = true;
        } else {
            self.data[..3].fill(0);
            self.data[4..].fill(-1);
        }
    }

    /// `AnvilMenu.createResult`.
    fn anvil_result(&mut self, env: &Env) {
        let hover = self.containers[MAIN][0]
            .as_ref()
            .map(|i| crate::stations::hover_name(&|id| env.names.display_item(id), i))
            .unwrap_or_default();
        let out = crate::stations::anvil_result(
            env.enchantments,
            env.items,
            self.containers[MAIN][0].as_ref(),
            self.containers[MAIN][1].as_ref(),
            self.anvil.name.as_deref(),
            &hover,
            self.creative,
        );
        // onlyRenaming is reset, repairItemCountCost only where an input is.
        self.anvil.only_renaming = out.only_renaming;
        if self.containers[MAIN][0].is_some() {
            self.anvil.repair_count = out.repair_item_count_cost;
        }
        self.data[0] = out.cost;
        self.containers[RESULT][0] = out.result;
    }

    /// `AnvilMenu.setItemName` (`ServerboundRenameItemPacket`): false when
    /// the name is refused or unchanged.
    /// `BeaconMenu.updateEffects` (`ServerboundSetBeaconPacket`): with a
    /// payment, powers `BeaconBlockEntity.validateEffects` allows for the
    /// pyramid's levels (data 0) become data 1 and 2 and the payment is used.
    /// Each power is its `mob_effect` id and name. False when refused.
    pub fn beacon_update(
        &mut self,
        env: &Env,
        primary: Option<(i32, &str)>,
        secondary: Option<(i32, &str)>,
    ) -> bool {
        if self.kind != Kind::Beacon || self.containers[MAIN][0].is_none() {
            return false;
        }
        let levels = self.data[0];
        let (p, s) = (
            beacon_required_levels(primary.map(|e| e.1)),
            beacon_required_levels(secondary.map(|e| e.1)),
        );
        let valid = !(secondary.is_some() && levels < 4)
            && p <= levels
            && s <= levels
            && p < 4
            && !(s != 0 && s < 4 && primary.map(|e| e.0) != secondary.map(|e| e.0));
        if !valid {
            return false;
        }
        self.data[1] = primary.map_or(0, |e| e.0 + 1);
        self.data[2] = secondary.map_or(0, |e| e.0 + 1);
        self.remove(env, 0, 1);
        true
    }

    pub fn rename(&mut self, env: &Env, name: &str) -> bool {
        // StringUtil.filterText, then validateName's length limit.
        let name: String = name
            .chars()
            .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
            .collect();
        if self.kind != Kind::Anvil
            || name.encode_utf16().count() > 50
            || self.anvil.name.as_deref() == Some(&name)
        {
            return false;
        }
        if let Some(result) = self.containers[RESULT][0].as_mut() {
            result.data_mut().custom_name = (!name.trim().is_empty())
                .then(|| horizon_world::nbt::Tag::String(name.as_str().into()));
            result.tidy();
        }
        self.anvil.name = Some(name);
        self.anvil_result(env);
        true
    }

    /// `SmithingMenu.createResult`.
    fn smith_result(&mut self, env: &Env) {
        let [a, b, c] = [0, 1, 2].map(|i| self.containers[MAIN][i].as_ref());
        self.containers[RESULT][0] = env
            .station_recipes
            .smithing_result(env.enchantments, a, b, c);
    }

    /// `SmithingMenu.slotsChanged`'s `hasRecipeError`: every input filled and no result.
    fn smith_error(&mut self) {
        let error = self.containers[MAIN].iter().all(Option::is_some)
            && self.containers[RESULT][0].is_none();
        self.data[0] = i32::from(error);
    }

    /// `CartographyTableMenu.slotsChanged` and `setupResultSlot`: a result
    /// only where the level has the map (`access.execute`), with the post
    /// processing its take runs (`MAP_POST_PROCESSING`: 0 lock, 1 scale).
    fn carto_changed(&mut self, env: &Env) {
        let [map, extra] = [0, 1].map(|i| self.containers[MAIN][i].clone());
        if self.containers[RESULT][0].is_some() && (map.is_none() || extra.is_none()) {
            self.containers[RESULT][0] = None;
            return;
        }
        let (Some(map), Some(extra), Some(maps)) = (map, extra, env.maps) else {
            return;
        };
        let Some((locked, scale)) = map
            .data()
            .and_then(|d| d.map_id)
            .and_then(|id| maps.state(id))
        else {
            return;
        };
        let extendable = env
            .items
            .item_tags
            .get("minecraft:extendable_maps")
            .is_some_and(|t| t.contains(&*map.item));
        let copy = |count: u32, post: Option<u8>| {
            let mut out = ItemStack {
                count,
                ..map.clone()
            };
            if post.is_some() {
                out.data_mut().map_post_processing = post;
            }
            out
        };
        self.containers[RESULT][0] = match &*extra.item {
            "minecraft:paper" if extendable && !locked && scale < 4 => Some(copy(1, Some(1))),
            "minecraft:glass_pane" if !locked => Some(copy(1, Some(0))),
            "minecraft:map" => Some(copy(2, None)),
            _ => None,
        };
    }

    /// `Item.onCraftedBy` of a taken cartography result
    /// (`MapItem.onCraftedPostProcess`): the post processing goes, and on a
    /// level with maps the stack names the scaled or locked copy.
    fn map_crafted(env: &Env, stack: &mut ItemStack) {
        if &*stack.item != "minecraft:filled_map" {
            return;
        }
        let Some(post) = stack.data().and_then(|d| d.map_post_processing) else {
            return;
        };
        stack.data_mut().map_post_processing = None;
        let id = stack.data().and_then(|d| d.map_id);
        if let (Some(maps), Some(id)) = (env.maps, id)
            && let Some(new) = maps.derive(id, post == 1)
        {
            stack.data_mut().map_id = Some(new);
        }
        stack.tidy();
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn slot(&self, i: usize) -> Option<&ItemStack> {
        let s = self.slots[i];
        self.containers[s.container][s.index].as_ref()
    }

    /// Sets a slot as a test fixture does (`Slot.set`).
    pub fn set_slot(&mut self, env: &Env, i: usize, stack: Option<ItemStack>) {
        if self.kind == Kind::Lectern {
            // LecternBlockEntity.setBook: the book its slot can't set.
            self.containers[MAIN][0] = stack;
            return;
        }
        self.put(env, i, stack);
    }

    pub fn player_item(&self, i: usize) -> Option<&ItemStack> {
        self.containers[PLAYER][i].as_ref()
    }

    pub fn set_player_item(&mut self, i: usize, stack: Option<ItemStack>) {
        self.containers[PLAYER][i] = stack;
    }

    /// `Inventory.add`: the leftover stays in `stack`.
    pub fn give(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        self.inventory_add(env, stack)
    }

    /// `Inventory.contains(stack)`.
    pub fn player_contains(&self, stack: &ItemStack) -> bool {
        self.containers[PLAYER]
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }

    // ---- item helpers ----

    fn max_stack(env: &Env, stack: &ItemStack) -> u32 {
        env.items.get(&stack.item).map_or(64, |i| i.max_stack)
    }

    fn damageable(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.max_damage > 0)
    }

    /// `ItemStack.isStackable`.
    fn stackable(env: &Env, stack: &ItemStack) -> bool {
        Self::max_stack(env, stack) > 1 && !(Self::damageable(env, stack) && stack.damage > 0)
    }

    fn is_bucket(stack: &ItemStack) -> bool {
        &*stack.item == "minecraft:bucket"
    }

    fn is_fuel(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.fuel.is_some())
    }

    // ---- Slot ----

    fn may_place(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        match self.slots[s].role {
            Role::Plain | Role::CreativeDisplay => true,
            Role::Armor(eq) => env
                .items
                .get(&stack.item)
                .and_then(|i| {
                    let slot = match i.equip_slot.as_deref()? {
                        "feet" => 0,
                        "legs" => 1,
                        "chest" => 2,
                        "head" => 3,
                        _ => return None,
                    };
                    let allowed = i
                        .equip_allowed
                        .as_ref()
                        .is_none_or(|a| a.contains("minecraft:player"));
                    Some(slot == eq && allowed)
                })
                .unwrap_or(false),
            // LivingEntity.isEquippableInSlot: the item's equippable slot,
            // and the mount among its allowed entities.
            Role::MountEquip(slot) => env.items.get(&stack.item).is_some_and(|i| {
                i.equip_slot.as_deref() == Some(slot)
                    && self
                        .mount
                        .as_deref()
                        .is_some_and(|m| i.equip_allowed.as_ref().is_none_or(|a| a.contains(m)))
            }),
            Role::Shulker => fits_inside_container_items(stack),
            Role::Fuel => Self::is_fuel(env, stack) || Self::is_bucket(stack),
            Role::Output | Role::CraftResult | Role::Inert => false,
            Role::CrafterIn => self.data.get(self.slots[s].index) != Some(&1),
            Role::Potion => {
                env.brewing.is_bottle(stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
            }
            Role::Ingredient => env.brewing.is_reagent(stack),
            Role::BrewFuel => env
                .items
                .get(&stack.item)
                .is_some_and(|i| i.brewing_fuel.is_some()),
            Role::LoomBanner => Self::is_banner(stack),
            Role::LoomDye => env.banners.loom_dye(env.items, stack).is_some(),
            Role::LoomPattern => env.banners.is_pattern_item(env.items, stack),
            Role::GrindInput => crate::stations::grindstone_accepts(env.items, stack),
            Role::SmithInput(i) => env.station_recipes.smithing_accepts(i, stack),
            Role::EnchantItem => true,
            Role::Lapis => &*stack.item == "minecraft:lapis_lazuli",
            Role::BeaconPay => env
                .items
                .item_tags
                .get("minecraft:beacon_payment_items")
                .is_some_and(|t| t.contains(&*stack.item)),
            Role::CartoMap => stack.data().is_some_and(|d| d.map_id.is_some()),
            Role::CartoExtra => Self::is_carto_extra(stack),
            Role::LoomResult
            | Role::MerchantResult
            | Role::StoneResult
            | Role::GrindResult
            | Role::CombineResult
            | Role::CartoResult => false,
        }
    }

    /// What the cartography table's second slot takes.
    fn is_carto_extra(stack: &ItemStack) -> bool {
        matches!(
            &*stack.item,
            "minecraft:paper" | "minecraft:map" | "minecraft:glass_pane"
        )
    }

    /// `BannerItem`.
    fn is_banner(stack: &ItemStack) -> bool {
        stack.item.ends_with("_banner")
    }

    fn may_pickup(&self, env: &Env, s: usize) -> bool {
        match self.slots[s].role {
            Role::Inert => false,
            // AnvilMenu.mayPickup: the levels to pay, and a price.
            Role::CombineResult if self.kind == Kind::Anvil => {
                (self.creative || self.levels >= self.data[0]) && self.data[0] > 0
            }
            Role::MerchantResult => self
                .merchant
                .as_ref()
                .and_then(|m| m.offers.get(m.active?))
                .is_some_and(|o| {
                    let (a, b) = (
                        self.containers[MAIN][0].as_ref(),
                        self.containers[MAIN][1].as_ref(),
                    );
                    o.satisfied_by(env.items, a, b) || o.satisfied_by(env.items, b, a)
                }),
            Role::Armor(_) | Role::MountEquip(_) => match self.slot(s) {
                Some(stack) => {
                    self.creative || stack.enchantment_level("minecraft:binding_curse") == 0
                }
                None => true,
            },
            _ => true,
        }
    }

    /// A result slot (`FurnaceResultSlot`, `ResultSlot`): nothing goes in, and
    /// what's taken from it can't be put back.
    pub fn is_result(&self, s: usize) -> bool {
        matches!(self.slots[s].role, Role::Output | Role::CraftResult)
    }

    /// Slot `s` takes one more item of `stack` (`Slot.mayPlace`, and room).
    pub fn takes_one(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        self.may_place(env, s, stack)
            && self.slot(s).is_none_or(|have| {
                have.same_item_same_components(stack) && have.count < self.slot_max(env, s, have)
            })
    }

    /// The slots `quickMoveStack` would put slot `index`'s items in: one item
    /// of a plain slot, the whole of a result slot, partial stacks first. Empty
    /// when they don't all fit. This runs the move, so call it on a menu that
    /// is thrown away afterwards.
    pub fn quick_move_landing(&mut self, env: &Env, index: usize) -> Vec<usize> {
        let Some(mut stack) = self.slot(index).cloned() else {
            return Vec::new();
        };
        if !self.is_result(index) {
            stack.count = 1;
            let slot = self.slots[index];
            self.containers[slot.container][slot.index] = Some(stack.clone());
        }
        let before: Vec<Option<ItemStack>> = (0..self.slots.len())
            .map(|i| self.slot(i).cloned())
            .collect();
        self.quick_move(env, index);
        let (mut partial, mut empty) = (Vec::new(), Vec::new());
        let mut moved = 0;
        for i in (0..self.slots.len()).filter(|&i| i != index && !self.is_result(i)) {
            let had = before[i].as_ref().map_or(0, |b| b.count);
            let got = self
                .slot(i)
                .filter(|now| now.same_item_same_components(&stack))
                .map_or(0, |now| now.count.saturating_sub(had));
            if got > 0 {
                moved += got;
                if had > 0 {
                    partial.push(i);
                } else {
                    empty.push(i);
                }
            }
        }
        if moved < stack.count {
            return Vec::new();
        }
        partial.extend(empty);
        partial
    }

    /// `Slot.getMaxStackSize(stack)`.
    fn slot_max(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        let own = match self.slots[s].role {
            Role::Armor(_)
            | Role::MountEquip(_)
            | Role::Potion
            | Role::EnchantItem
            | Role::BeaconPay => 1,
            // The lectern's bookAccess holds one.
            Role::Plain if self.kind == Kind::Lectern && self.slots[s].container == MAIN => 1,
            Role::Fuel if Self::is_bucket(stack) => return 1,
            _ => CONTAINER_MAX,
        };
        own.min(Self::max_stack(env, stack))
    }

    /// `Container.setItem` (a grid change recomputes the result).
    fn put(&mut self, env: &Env, s: usize, mut stack: Option<ItemStack>) {
        tidy(&mut stack);
        let slot = self.slots[s];
        // LecternBlockEntity's bookAccess.setItem does nothing.
        if self.kind == Kind::Lectern && slot.container == MAIN {
            return;
        }
        // SimpleContainer / BaseContainerBlockEntity.setItem: limitSize to
        // getMaxStackSize(stack). The loom's result is a SimpleContainer too.
        if (slot.container == MAIN || slot.role == Role::LoomResult)
            && let Some(st) = stack.as_mut()
        {
            st.count = st.count.min(CONTAINER_MAX.min(Self::max_stack(env, st)));
            // The beacon's container holds one item (getMaxStackSize 1).
            if self.kind == Kind::Beacon {
                st.count = st.count.min(1);
            }
        }
        // CrafterBlockEntity.setItem: filling a disabled (empty) slot enables it.
        if self.kind == Kind::CrafterBlock
            && slot.container == MAIN
            && self.data[slot.index] == 1
            && self.containers[MAIN][slot.index].is_none()
        {
            self.data[slot.index] = 0;
        }
        self.containers[slot.container][slot.index] = stack;
        if slot.container == GRID {
            self.recompute(env);
        }
        self.slot_changed(env, s);
    }

    /// `Slot.setByPlayer(stack)`: the slot's item is the previous one.
    fn set_by_player(&mut self, env: &Env, s: usize, stack: Option<ItemStack>) {
        let previous = self.slot(s).cloned();
        self.set_by_player_from(env, s, stack, previous.as_ref());
    }

    /// `Slot.setByPlayer(stack, previous)`: `ArmorSlot` and the inventory
    /// menu's off-hand slot first call the owner's `onEquipItem`, which acts
    /// only when the item changes (`isSameItemSameComponents`).
    fn set_by_player_from(
        &mut self,
        env: &Env,
        s: usize,
        mut stack: Option<ItemStack>,
        previous: Option<&ItemStack>,
    ) {
        tidy(&mut stack);
        if let Some(slot) = self.equipment_slot(s) {
            let same = match (previous.filter(|p| p.count > 0), stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            if !same {
                self.equipped.push((slot, stack.clone()));
            }
        }
        self.put(env, s, stack);
    }

    /// The `EquipmentSlot` of an `ArmorSlot` or of `InventoryMenu`'s off-hand
    /// slot (the only menu slot over inventory slot 40).
    fn equipment_slot(&self, s: usize) -> Option<&'static str> {
        let slot = self.slots[s];
        match slot.role {
            Role::Armor(eq) => Some(["feet", "legs", "chest", "head"][eq]),
            _ if slot.container == PLAYER && slot.index == OFFHAND => Some("offhand"),
            _ => None,
        }
    }

    /// `Slot.remove`: the container's `removeItem`.
    fn remove(&mut self, env: &Env, s: usize, amount: u32) -> Option<ItemStack> {
        let slot = self.slots[s];
        // ResultContainer.removeItem takes the whole stack; the loom's result
        // is a SimpleContainer slot and splits.
        if slot.container == RESULT && slot.role != Role::LoomResult {
            return self.containers[RESULT][0].take();
        }
        let cell = &mut self.containers[slot.container][slot.index];
        let out = cell
            .as_mut()
            .filter(|c| c.count > 0)
            .map(|c| split(c, amount));
        tidy(cell);
        // LecternBlockEntity: an emptied book slot is onBookItemRemove (page 0).
        if self.kind == Kind::Lectern && slot.container == MAIN && cell.is_none() {
            self.data[0] = 0;
        }
        if out.is_some() && slot.container == GRID {
            self.recompute(env);
        }
        // SimpleContainer.removeItem calls setChanged when it took something;
        // MerchantContainer.removeItem updates the result after a payment slot.
        if out.is_some()
            && matches!(
                self.kind,
                Kind::Loom
                    | Kind::Merchant
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Enchanting
                    | Kind::Cartography
            )
        {
            self.slot_changed(env, s);
        }
        out
    }

    /// `AnvilMenu.onTake`: the levels, the material used up, then the input.
    fn anvil_take(&mut self, env: &Env) {
        let cost = self.data[0];
        if !self.creative {
            // Player.giveExperienceLevels(-cost).
            self.level_change -= cost;
            self.drops.push(format!(
                "levels{}{}",
                if cost <= 0 { "+" } else { "" },
                -cost
            ));
        }
        let repair = self.anvil.repair_count;
        if repair > 0 {
            let addition = self.containers[MAIN][1]
                .clone()
                .filter(|a| a.count > repair)
                .map(|mut a| {
                    a.count -= repair;
                    a
                });
            self.put(env, 1, addition);
        } else if !self.anvil.only_renaming {
            self.put(env, 1, None);
        }
        self.data[0] = 0;
        self.put(env, 0, None);
        self.combined = true;
    }

    /// `CraftingMenu.slotChangedCraftingGrid`.
    fn recompute(&mut self, env: &Env) {
        if self.grid_size == 0 {
            return;
        }
        let result = env
            .recipes
            .find(&self.containers[GRID], self.grid_size)
            .map(|(_, stack)| stack);
        self.containers[RESULT][0] = result;
    }

    /// `ResultSlot.onTake` and the other `Slot.onTake` overrides.
    fn on_take(&mut self, env: &Env, s: usize, taken: &ItemStack) {
        if self.slots[s].role == Role::Potion {
            // BrewingStandMenu.PotionSlot.onTake: a stack with potion contents.
            if matches!(
                &*taken.item,
                "minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion"
            ) {
                self.brewed.push(taken.clone());
            }
            return;
        }
        if self.slots[s].role == Role::MerchantResult {
            self.merchant_take(env);
            return;
        }
        if self.slots[s].role == Role::StoneResult {
            // StonecutterMenu's result onTake: awardUsedRecipes, one input
            // used up, the result again for the selected recipe.
            if let Some(r) = self.stone.used {
                self.crafted.push((
                    env.station_recipes.stonecutting[r].id.clone(),
                    vec![self.containers[MAIN][0].clone()],
                ));
            }
            if self.remove(env, 0, 1).is_some() {
                self.stone_setup_result(env, self.data[0]);
            }
            self.sounds.push("minecraft:ui.stonecutter.take_result");
            return;
        }
        if self.slots[s].role == Role::GrindResult {
            // GrindstoneMenu's result onTake: the experience (from the inputs
            // before they go), then both inputs cleared.
            self.ground = Some(crate::stations::grindstone_xp_base(
                env.enchantments,
                self.containers[MAIN][0].as_ref(),
                self.containers[MAIN][1].as_ref(),
            ));
            self.put(env, 0, None);
            self.put(env, 1, None);
            return;
        }
        if self.slots[s].role == Role::CombineResult && self.kind == Kind::Anvil {
            self.anvil_take(env);
            return;
        }
        if self.slots[s].role == Role::CombineResult {
            // SmithingMenu.onTake: awardUsedRecipes, then one of each input.
            if self.kind == Kind::Smithing {
                let inputs = [0, 1, 2].map(|i| self.containers[MAIN][i].clone());
                if let Some((id, _)) = env.station_recipes.smithing_recipe(
                    env.enchantments,
                    inputs[0].as_ref(),
                    inputs[1].as_ref(),
                    inputs[2].as_ref(),
                ) {
                    self.crafted.push((id.to_owned(), inputs.to_vec()));
                }
            }
            for i in 0..3 {
                if let Some(mut stack) = self.containers[MAIN][i].clone() {
                    stack.count -= 1;
                    self.put(env, i, Some(stack));
                }
            }
            self.combined = true;
            return;
        }
        if self.slots[s].role == Role::CartoResult {
            // CartographyTableMenu's result onTake: one of each input, the
            // take sound, then Slot.onTake's setChanged. The taken stack's
            // onCraftedBy is the caller's (map_crafted).
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            self.sounds
                .push("minecraft:ui.cartography_table.take_result");
            self.slot_changed(env, s);
            return;
        }
        if self.slots[s].role == Role::LoomResult {
            // LoomMenu's result onTake: one banner and one dye are used up.
            self.remove(env, 0, 1);
            self.remove(env, 1, 1);
            if self.containers[MAIN][0].is_none() || self.containers[MAIN][1].is_none() {
                self.data[0] = -1;
            }
            self.sounds.push("minecraft:ui.loom.take_result");
            return;
        }
        if matches!(self.kind, Kind::CrafterBlock) && self.slots[s].container == MAIN {
            // Slot.onTake -> CrafterSlot.setChanged -> the result refresh.
            self.slot_changed(env, s);
            return;
        }
        if self.slots[s].role != Role::CraftResult {
            return;
        }
        let n = self.grid_size;
        let grid = self.containers[GRID].clone();
        if let Some((id, _)) = env.recipes.find(&grid, n) {
            self.crafted.push((format!("minecraft:{id}"), grid.clone()));
        }
        let remaining = env
            .recipes
            .find(&grid, n)
            .map(|(id, _)| env.recipes.remainders(id, &grid, n))
            .unwrap_or_else(|| {
                // CraftingRecipe.defaultCraftingReminder
                grid.iter()
                    .map(|cell| {
                        cell.as_ref().and_then(|stack| {
                            env.items
                                .get(&stack.item)
                                .and_then(|i| i.crafting_remainder.as_deref())
                                .map(|r| ItemStack::new(r, 1))
                        })
                    })
                    .collect()
            });
        for cell in 0..n * n {
            let mut here = self.containers[GRID][cell].take();
            if let Some(stack) = here.as_mut() {
                stack.count -= 1;
            }
            tidy(&mut here);
            self.containers[GRID][cell] = here;
            if let Some(replacement) = remaining.get(cell).cloned().flatten() {
                match self.containers[GRID][cell].as_mut() {
                    None => self.containers[GRID][cell] = Some(replacement),
                    Some(have) if have.same_item_same_components(&replacement) => {
                        let mut merged = replacement;
                        merged.count += have.count;
                        self.containers[GRID][cell] = Some(merged);
                    }
                    Some(_) => {
                        let mut left = replacement;
                        if !self.inventory_add(env, &mut left) {
                            self.drop_stack(&left);
                        }
                    }
                }
            }
        }
        self.recompute(env);
    }

    fn drop_stack(&mut self, stack: &ItemStack) {
        if stack.count > 0 {
            self.drops.push(crate::item_data::canon_stack(Some(stack)));
            self.dropped.push(stack.clone());
        }
    }

    /// `Slot.tryRemove`.
    fn try_remove(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        if !self.may_pickup(env, s) {
            return None;
        }
        if !self.allow_modification(env, s) && max_amount < self.slot(s).map_or(0, |c| c.count) {
            return None;
        }
        let result = self.remove(env, s, amount.min(max_amount))?;
        if self.slot(s).is_none() {
            self.set_by_player_from(env, s, None, Some(&result));
        }
        Some(result)
    }

    fn allow_modification(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(env, s)
            && match self.slot(s) {
                Some(stack) => self.may_place(env, s, stack),
                None => true,
            }
    }

    fn safe_take(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        let mut result = self.try_remove(env, s, amount, max_amount)?;
        self.on_take(env, s, &result);
        if self.slots[s].role == Role::CartoResult {
            Self::map_crafted(env, &mut result);
        }
        Some(result)
    }

    /// `Slot.safeInsert`: puts up to `amount` of `input` into the slot.
    fn safe_insert(&mut self, env: &Env, s: usize, input: &mut Option<ItemStack>, amount: u32) {
        let Some(stack) = input.as_mut() else { return };
        if !self.may_place(env, s, stack) {
            return;
        }
        let have = self.slot(s).map_or(0, |c| c.count);
        let max = self.slot_max(env, s, stack);
        let transferable = amount.min(stack.count).min(max.saturating_sub(have));
        if transferable == 0 {
            return;
        }
        match self.slot(s).cloned() {
            None => {
                let part = split(stack, transferable);
                self.set_by_player(env, s, Some(part));
            }
            Some(mut current) if current.same_item_same_components(stack) => {
                stack.count -= transferable;
                current.count += transferable;
                self.set_by_player(env, s, Some(current));
            }
            Some(_) => {}
        }
        tidy(input);
    }

    // ---- Inventory ----

    /// `Inventory.getSlotWithRemainingSpace` (selected slot 0, offhand, rest).
    fn slot_with_space(&self, env: &Env, stack: &ItemStack) -> Option<usize> {
        let inv = &self.containers[PLAYER];
        [self.selected, OFFHAND]
            .into_iter()
            .chain(0..36)
            .find(|&i| {
                inv[i].as_ref().is_some_and(|have| {
                    have.same_item_same_components(stack)
                        && Self::stackable(env, have)
                        && have.count < CONTAINER_MAX.min(Self::max_stack(env, have))
                })
            })
    }

    /// `Inventory.addResource`: returns what did not fit.
    fn add_resource(&mut self, env: &Env, stack: &ItemStack) -> u32 {
        let slot = self.slot_with_space(env, stack).or_else(|| {
            self.containers[PLAYER][..36]
                .iter()
                .position(Option::is_none)
        });
        let Some(slot) = slot else { return stack.count };
        let cell = self.containers[PLAYER][slot].get_or_insert_with(|| {
            let mut empty = stack.clone();
            empty.count = 0;
            empty
        });
        let room = CONTAINER_MAX.min(Self::max_stack(env, cell)) - cell.count;
        let add = stack.count.min(room);
        cell.count += add;
        tidy(&mut self.containers[PLAYER][slot]);
        stack.count - add
    }

    /// `Inventory.add(stack)`: the leftover stays in `stack`.
    fn inventory_add(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        if stack.count == 0 {
            return false;
        }
        if Self::damageable(env, stack) && stack.damage > 0 {
            if let Some(slot) = self.containers[PLAYER][..36]
                .iter()
                .position(Option::is_none)
            {
                self.containers[PLAYER][slot] = Some(stack.clone());
                stack.count = 0;
                return true;
            }
            if self.creative {
                stack.count = 0;
                return true;
            }
            return false;
        }
        loop {
            let last = stack.count;
            stack.count = self.add_resource(env, stack);
            if stack.count == 0 || stack.count >= last {
                if stack.count == last && self.creative {
                    stack.count = 0;
                    return true;
                }
                return stack.count < last;
            }
        }
    }
}

/// `EquipmentSlot` of an item as the inventory menu's quick move sees it.
enum Equip {
    Armor(usize),
    Offhand,
    Other,
}

impl Menu {
    fn equip(env: &Env, stack: &ItemStack) -> Equip {
        match env
            .items
            .get(&stack.item)
            .and_then(|i| i.equip_slot.as_deref())
        {
            Some("feet") => Equip::Armor(0),
            Some("legs") => Equip::Armor(1),
            Some("chest") => Equip::Armor(2),
            Some("head") => Equip::Armor(3),
            Some("offhand") => Equip::Offhand,
            _ => Equip::Other,
        }
    }

    fn has_item(&self, s: usize) -> bool {
        self.slot(s).is_some()
    }

    /// `AbstractContainerMenu.moveItemStackTo`.
    fn move_to(
        &mut self,
        env: &Env,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        let mut changed = false;
        let order = |_: ()| -> Vec<usize> {
            if backwards {
                (start..end).rev().collect()
            } else {
                (start..end).collect()
            }
        };
        if Self::stackable(env, stack) {
            for dest in order(()) {
                if stack.count == 0 {
                    break;
                }
                let max = match self.slot(dest) {
                    Some(target) if target.same_item_same_components(stack) => {
                        self.slot_max(env, dest, target)
                    }
                    _ => continue,
                };
                let slot = self.slots[dest];
                let target = self.containers[slot.container][slot.index]
                    .as_mut()
                    .expect("checked");
                let total = target.count + stack.count;
                if total <= max {
                    stack.count = 0;
                    target.count = total;
                    changed = true;
                    self.slot_changed(env, dest);
                } else if target.count < max {
                    stack.count -= max - target.count;
                    target.count = max;
                    changed = true;
                    self.slot_changed(env, dest);
                }
            }
        }
        if stack.count > 0 {
            for dest in order(()) {
                if !self.has_item(dest) && self.may_place(env, dest, stack) {
                    let max = self.slot_max(env, dest, stack);
                    let part = split(stack, stack.count.min(max));
                    self.set_by_player(env, dest, Some(part));
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// `quickMoveStack` of the menu's kind; the moved stack's original copy
    /// (None when nothing moved).
    fn quick_move(&mut self, env: &Env, index: usize) -> Option<ItemStack> {
        let mut stack = self.slot(index)?.clone();
        let clicked = stack.clone();
        let total = self.slots.len();
        let mut check_count = true;
        let mut after_take = false;
        match self.kind {
            Kind::ItemPicker => {
                // ItemPickerMenu.quickMoveStack: a hotbar stack is deleted.
                if index >= total - 9 {
                    self.put(env, index, None);
                }
                return None;
            }
            Kind::Chest(rows) => {
                check_count = false;
                let n = rows * 9;
                if index < n {
                    if !self.move_to(env, &mut stack, n, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, n, false) {
                    return None;
                }
            }
            Kind::Hopper => {
                check_count = false;
                if index < 5 {
                    if !self.move_to(env, &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 5, false) {
                    return None;
                }
            }
            Kind::Shulker => {
                check_count = false;
                if index < 27 {
                    if !self.move_to(env, &mut stack, 27, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 27, false) {
                    return None;
                }
            }
            Kind::Dispenser => {
                after_take = true;
                if index < 9 {
                    if !self.move_to(env, &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 9, false) {
                    return None;
                }
            }
            Kind::Brewing => {
                after_take = true;
                let slot = self.slots[index];
                let commit = |m: &mut Self, stack: &ItemStack| {
                    m.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                };
                if index <= 4 {
                    if !self.move_to(env, &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if env
                    .items
                    .get(&clicked.item)
                    .is_some_and(|i| i.brewing_fuel.is_some())
                {
                    let moved = self.move_to(env, &mut stack, 4, 5, false);
                    if moved
                        || (env.brewing.is_reagent(&stack)
                            && !self.move_to(env, &mut stack, 3, 4, false))
                    {
                        commit(self, &stack);
                        return None;
                    }
                } else if env.brewing.is_reagent(&stack) {
                    if !self.move_to(env, &mut stack, 3, 4, false) {
                        return None;
                    }
                } else if env.brewing.is_bottle(&stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
                {
                    if !self.move_to(env, &mut stack, 0, 3, false) {
                        return None;
                    }
                } else if (5..32).contains(&index) {
                    if !self.move_to(env, &mut stack, 32, 41, false) {
                        return None;
                    }
                } else if (32..41).contains(&index) {
                    if !self.move_to(env, &mut stack, 5, 32, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 5, 41, false) {
                    return None;
                }
            }
            Kind::Loom => {
                after_take = true;
                if index == 3 {
                    if !self.move_to(env, &mut stack, 4, 40, true) {
                        return None;
                    }
                } else if index > 3 {
                    let target = if Self::is_banner(&stack) {
                        Some(0)
                    } else if env.banners.loom_dye(env.items, &stack).is_some() {
                        Some(1)
                    } else if env.banners.is_pattern_item(env.items, &stack) {
                        Some(2)
                    } else {
                        None
                    };
                    match target {
                        Some(t) => {
                            if !self.move_to(env, &mut stack, t, t + 1, false) {
                                return None;
                            }
                        }
                        None if index < 31 => {
                            if !self.move_to(env, &mut stack, 31, 40, false) {
                                return None;
                            }
                        }
                        None => {
                            if index < 40 && !self.move_to(env, &mut stack, 4, 31, false) {
                                return None;
                            }
                        }
                    }
                } else if !self.move_to(env, &mut stack, 4, 40, false) {
                    return None;
                }
            }
            Kind::Stonecutter => {
                after_take = true;
                if index == 1 {
                    if !self.move_to(env, &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if index == 0 {
                    if !self.move_to(env, &mut stack, 2, 38, false) {
                        return None;
                    }
                } else if env
                    .station_recipes
                    .stonecutting
                    .iter()
                    .any(|r| r.input.test(&stack))
                {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (2..29).contains(&index) {
                    if !self.move_to(env, &mut stack, 29, 38, false) {
                        return None;
                    }
                } else if (29..38).contains(&index) && !self.move_to(env, &mut stack, 2, 29, false)
                {
                    return None;
                }
            }
            Kind::Grindstone => {
                after_take = true;
                let full = self.containers[MAIN].iter().all(Option::is_some);
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if full {
                    if (3..30).contains(&index) {
                        if !self.move_to(env, &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 2, false) {
                    return None;
                }
            }
            Kind::Cartography => {
                after_take = true;
                if index == 2 {
                    // onCraftedBy edits the result in its slot first.
                    Self::map_crafted(env, &mut stack);
                    self.containers[RESULT][0] = Some(stack.clone());
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if stack.data().is_some_and(|d| d.map_id.is_some()) {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if Self::is_carto_extra(&stack) {
                    if !self.move_to(env, &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if (3..30).contains(&index) {
                    if !self.move_to(env, &mut stack, 30, 39, false) {
                        return None;
                    }
                } else if (30..39).contains(&index) && !self.move_to(env, &mut stack, 3, 30, false)
                {
                    return None;
                }
            }
            // LecternMenu.quickMoveStack moves nothing.
            Kind::Lectern => return None,
            Kind::Mount(columns) => {
                // AbstractMountInventoryMenu.quickMoveStack: no count check
                // and no onTake.
                check_count = false;
                let start = 2 + columns * 3;
                if index < start {
                    if !self.move_to(env, &mut stack, start, total, true) {
                        return None;
                    }
                } else if self.may_place(env, 1, &stack) && self.containers[MAIN][1].is_none() {
                    if !self.move_to(env, &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if self.may_place(env, 0, &stack) && self.containers[MAIN][0].is_none() {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if columns == 0 || !self.move_to(env, &mut stack, 2, start, false) {
                    // Between the inventory and the hotbar: the stack shrinks
                    // in its slot and EMPTY is returned either way.
                    let (main_end, hotbar_end) = (start + 27, start + 36);
                    if (main_end..hotbar_end).contains(&index) {
                        self.move_to(env, &mut stack, start, main_end, false);
                    } else if (start..main_end).contains(&index) {
                        self.move_to(env, &mut stack, main_end, hotbar_end, false);
                    }
                    let slot = self.slots[index];
                    self.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    return None;
                }
            }
            Kind::Beacon => {
                // BeaconMenu.quickMoveStack.
                after_take = true;
                if index == 0 {
                    if !self.move_to(env, &mut stack, 1, 37, true) {
                        return None;
                    }
                } else if self.containers[MAIN][0].is_none()
                    && self.may_place(env, 0, &stack)
                    && stack.count == 1
                {
                    if !self.move_to(env, &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (1..28).contains(&index) {
                    if !self.move_to(env, &mut stack, 28, 37, false) {
                        return None;
                    }
                } else if (28..37).contains(&index) {
                    if !self.move_to(env, &mut stack, 1, 28, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 1, 37, false) {
                    return None;
                }
            }
            Kind::Enchanting => {
                after_take = true;
                if index < 2 {
                    if !self.move_to(env, &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if &*stack.item == "minecraft:lapis_lazuli" {
                    if !self.move_to(env, &mut stack, 1, 2, true) {
                        return None;
                    }
                } else {
                    // One item into the empty item slot (setByPlayer).
                    if self.containers[MAIN][0].is_some() {
                        return None;
                    }
                    let single = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    stack.count -= 1;
                    // The source slot keeps the rest while the item slot changes.
                    let slot = self.slots[index];
                    self.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    self.set_by_player(env, 0, Some(single));
                }
            }
            Kind::Anvil | Kind::Smithing => {
                // ItemCombinerMenu.quickMoveStack.
                after_take = true;
                let result = if self.kind == Kind::Anvil { 2 } else { 3 };
                let (start, use_row) = (result + 1, result + 28);
                let end = result + 37;
                let into_inputs = self.kind == Kind::Anvil
                    || (0..3).any(|i| {
                        self.containers[MAIN][i].is_none()
                            && env.station_recipes.smithing_accepts(i, &stack)
                    });
                if index == result {
                    if !self.move_to(env, &mut stack, start, end, true) {
                        return None;
                    }
                } else if index < result {
                    if !self.move_to(env, &mut stack, start, end, false) {
                        return None;
                    }
                } else if into_inputs && (start..end).contains(&index) {
                    if !self.move_to(env, &mut stack, 0, result, false) {
                        return None;
                    }
                } else if (start..use_row).contains(&index) {
                    if !self.move_to(env, &mut stack, use_row, end, false) {
                        return None;
                    }
                } else if (use_row..end).contains(&index)
                    && !self.move_to(env, &mut stack, start, use_row, false)
                {
                    return None;
                }
            }
            Kind::Merchant => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                    if let Some(m) = self.merchant.as_mut() {
                        m.events.push(crate::merchant::Event::TradeSound);
                    }
                } else if index != 0 && index != 1 {
                    if (3..30).contains(&index) {
                        if !self.move_to(env, &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 3, 39, false) {
                    return None;
                }
            }
            Kind::Crafter | Kind::CrafterBlock => {
                after_take = true;
                if index < 9 {
                    if !self.move_to(env, &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 0, 9, false) {
                    return None;
                }
            }
            Kind::Furnace(recipe) => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index != 1 && index != 0 {
                    let smelts = env
                        .recipes
                        .cook(&format!("minecraft:{recipe}"), &stack)
                        .is_some();
                    if smelts {
                        if !self.move_to(env, &mut stack, 0, 1, false) {
                            return None;
                        }
                    } else if Self::is_fuel(env, &stack) {
                        if !self.move_to(env, &mut stack, 1, 2, false) {
                            return None;
                        }
                    } else if (3..30).contains(&index) {
                        if !self.move_to(env, &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 3, 39, false) {
                    return None;
                }
            }
            Kind::Crafting => {
                after_take = true;
                if index == 0 {
                    if !self.move_to(env, &mut stack, 10, 46, true) {
                        return None;
                    }
                } else if (10..46).contains(&index) {
                    if !self.move_to(env, &mut stack, 1, 10, false) {
                        if index < 37 {
                            if !self.move_to(env, &mut stack, 37, 46, false) {
                                return None;
                            }
                        } else if !self.move_to(env, &mut stack, 10, 37, false) {
                            return None;
                        }
                    }
                } else if !self.move_to(env, &mut stack, 10, 46, false) {
                    return None;
                }
            }
            Kind::Inventory => {
                after_take = true;
                let equip = Self::equip(env, &clicked);
                let armor_free = |m: &Self, eq: usize| !m.has_item(8 - eq);
                if index == 0 {
                    if !self.move_to(env, &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if (1..9).contains(&index) {
                    if !self.move_to(env, &mut stack, 9, 45, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Armor(eq) if armor_free(self, eq)) {
                    let Equip::Armor(eq) = equip else {
                        unreachable!()
                    };
                    let pos = 8 - eq;
                    if !self.move_to(env, &mut stack, pos, pos + 1, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Offhand) && !self.has_item(45) {
                    if !self.move_to(env, &mut stack, 45, 46, false) {
                        return None;
                    }
                } else if (9..36).contains(&index) {
                    if !self.move_to(env, &mut stack, 36, 45, false) {
                        return None;
                    }
                } else if (36..45).contains(&index) {
                    if !self.move_to(env, &mut stack, 9, 36, false) {
                        return None;
                    }
                } else if !self.move_to(env, &mut stack, 9, 45, false) {
                    return None;
                }
            }
        }
        // The stack was edited in place in its slot.
        let slot = self.slots[index];
        if stack.count == 0 {
            self.containers[slot.container][slot.index] = None;
            // InventoryMenu: setByPlayer(EMPTY, clicked).
            self.set_by_player_from(env, index, None, Some(&clicked));
        } else {
            self.containers[slot.container][slot.index] = Some(stack.clone());
            self.slot_changed(env, index);
        }
        if check_count && stack.count == clicked.count {
            return None;
        }
        if after_take || matches!(self.kind, Kind::Inventory | Kind::Crafting) {
            self.on_take(env, index, &clicked);
        }
        // What the result slot's move left over is dropped.
        if (index == 0 && matches!(self.kind, Kind::Inventory | Kind::Crafting))
            || (index == 1 && self.kind == Kind::Stonecutter)
        {
            self.drop_stack(&stack);
        }
        Some(clicked)
    }

    fn quick_replace(&self, s: usize, stack: &ItemStack, env: &Env) -> bool {
        match self.slot(s) {
            None => true,
            Some(have) if have.same_item_same_components(stack) => {
                have.count <= Self::max_stack(env, stack)
            }
            Some(_) => false,
        }
    }

    fn reset_quick_craft(&mut self) {
        self.quick_status = 0;
        self.quick_slots.clear();
    }

    /// `AbstractContainerMenu.clicked`: Err carries a vanilla exception name.
    pub fn click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Result<(), &'static str> {
        let size = self.slots.len() as i32;
        let in_range = |i: i32| (0..size).contains(&i);
        if input == Input::QuickCraft {
            let expected = self.quick_status;
            self.quick_status = button & 3;
            if ((expected != 1 || self.quick_status != 2) && expected != self.quick_status)
                || self.carried.is_none()
            {
                self.reset_quick_craft();
            } else if self.quick_status == 0 {
                self.quick_type = (button >> 2) & 3;
                if self.quick_type == 0
                    || self.quick_type == 1
                    || (self.quick_type == 2 && self.creative)
                {
                    self.quick_status = 1;
                    self.quick_slots.clear();
                } else {
                    self.reset_quick_craft();
                }
            } else if self.quick_status == 1 {
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                let carried = self.carried.clone().expect("checked");
                if self.quick_replace(s, &carried, env)
                    && self.may_place(env, s, &carried)
                    && (self.quick_type == 2 || carried.count as usize > self.quick_slots.len())
                    && self.can_drag_to(s)
                    && !self.quick_slots.contains(&s)
                {
                    self.quick_slots.push(s);
                }
            } else if self.quick_status == 2 {
                if !self.quick_slots.is_empty() {
                    if self.quick_slots.len() == 1 {
                        let s = self.quick_slots[0] as i32;
                        let ty = self.quick_type;
                        self.reset_quick_craft();
                        return self.click(env, s, ty, Input::Pickup);
                    }
                    let mut source = self.carried.clone().expect("checked");
                    let mut remaining = source.count as i64;
                    let carried = self.carried.clone().expect("checked");
                    for s in self.quick_slots.clone() {
                        if self.quick_replace(s, &carried, env)
                            && self.may_place(env, s, &carried)
                            && (self.quick_type == 2
                                || carried.count as usize >= self.quick_slots.len())
                            && self.can_drag_to(s)
                        {
                            let have = self.slot(s).map_or(0, |c| c.count);
                            let max =
                                Self::max_stack(env, &source).min(self.slot_max(env, s, &source));
                            let each = match self.quick_type {
                                0 => (source.count as f32 / self.quick_slots.len() as f32).floor()
                                    as u32,
                                1 => 1,
                                2 => Self::max_stack(env, &source),
                                _ => source.count,
                            };
                            let new_count = (each + have).min(max);
                            remaining -= i64::from(new_count) - i64::from(have);
                            let mut placed = source.clone();
                            placed.count = new_count;
                            self.set_by_player(env, s, Some(placed));
                        }
                    }
                    source.count = remaining.max(0) as u32;
                    self.carried = Some(source);
                    tidy(&mut self.carried);
                }
                self.reset_quick_craft();
            } else {
                self.reset_quick_craft();
            }
            return Ok(());
        }
        if self.quick_status != 0 {
            self.reset_quick_craft();
            return Ok(());
        }
        if matches!(input, Input::Pickup | Input::QuickMove) && (button == 0 || button == 1) {
            let primary = button == 0;
            if slot_index == -999 {
                if let Some(carried) = self.carried.as_mut() {
                    if primary {
                        let dropped = self.carried.take().expect("some");
                        self.drop_stack(&dropped);
                    } else {
                        let one = split(carried, 1);
                        self.drop_stack(&one);
                        tidy(&mut self.carried);
                    }
                }
            } else if input == Input::QuickMove {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                if !self.may_pickup(env, s) {
                    return Ok(());
                }
                let mut clicked = self.quick_move(env, s);
                while let Some(c) = &clicked {
                    if !self.slot(s).is_some_and(|now| same_item(now, c)) {
                        break;
                    }
                    clicked = self.quick_move(env, s);
                }
            } else {
                if slot_index < 0 {
                    return Ok(());
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                self.pickup(env, slot_index as usize, primary);
            }
        } else if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.swap(env, slot_index as usize, button as usize);
        } else if input == Input::Clone
            && self.creative
            && self.carried.is_none()
            && slot_index >= 0
        {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            if let Some(stack) = self.slot(slot_index as usize) {
                let mut copy = stack.clone();
                copy.count = Self::max_stack(env, stack);
                // The cartography result's safeClone runs onCraftedBy.
                if self.slots[slot_index as usize].role == Role::CartoResult {
                    Self::map_crafted(env, &mut copy);
                }
                self.carried = Some(copy);
            }
        } else if input == Input::Throw && self.carried.is_none() && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            let s = slot_index as usize;
            let amount = if button == 0 {
                1
            } else {
                self.slot(s).map_or(0, |c| c.count)
            };
            let mut taken = self.safe_take(env, s, amount, u32::MAX);
            if let Some(t) = &taken {
                self.drop_stack(t);
            }
            if button == 1 {
                while let Some(t) = &taken {
                    if !self.slot(s).is_some_and(|now| same_item(now, t)) {
                        break;
                    }
                    taken = self.safe_take(env, s, amount, u32::MAX);
                    if let Some(t) = &taken {
                        self.drop_stack(t);
                    }
                }
            }
        } else if input == Input::PickupAll && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            self.pickup_all(env, slot_index as usize, button);
        }
        Ok(())
    }

    /// The PICKUP branch of `doClick`: a bundle's override, else the
    /// plain click, ending with `slot.setChanged()`.
    fn pickup(&mut self, env: &Env, s: usize, primary: bool) {
        if !self.bundle_click(env, s, primary) {
            self.pickup_inner(env, s, primary);
        }
        self.slot_changed(env, s);
    }

    fn pickup_inner(&mut self, env: &Env, s: usize, primary: bool) {
        let clicked = self.slot(s).cloned();
        match clicked {
            None => {
                if let Some(mut carried) = self.carried.take() {
                    let amount = if primary { carried.count } else { 1 };
                    let mut input = Some(carried.clone());
                    self.safe_insert(env, s, &mut input, amount);
                    carried = input.unwrap_or_else(|| {
                        let mut e = carried.clone();
                        e.count = 0;
                        e
                    });
                    self.carried = Some(carried);
                    tidy(&mut self.carried);
                }
            }
            Some(clicked) if self.may_pickup(env, s) => match self.carried.take() {
                None => {
                    let amount = if primary {
                        clicked.count
                    } else {
                        clicked.count.div_ceil(2)
                    };
                    if let Some(mut taken) = self.try_remove(env, s, amount, u32::MAX) {
                        self.carried = Some(taken.clone());
                        self.on_take(env, s, &taken);
                        if self.slots[s].role == Role::CartoResult {
                            // onTake's onCraftedBy edits the carried stack.
                            Self::map_crafted(env, &mut taken);
                            self.carried = Some(taken);
                        }
                    }
                }
                Some(mut carried) => {
                    if self.may_place(env, s, &carried) {
                        if clicked.same_item_same_components(&carried) {
                            let amount = if primary { carried.count } else { 1 };
                            let mut input = Some(carried.clone());
                            self.safe_insert(env, s, &mut input, amount);
                            carried = input.unwrap_or_else(|| {
                                let mut e = carried.clone();
                                e.count = 0;
                                e
                            });
                            self.carried = Some(carried);
                            tidy(&mut self.carried);
                        } else if carried.count <= self.slot_max(env, s, &carried) {
                            self.carried = Some(clicked);
                            self.set_by_player(env, s, Some(carried));
                        } else {
                            self.carried = Some(carried);
                        }
                    } else if clicked.same_item_same_components(&carried) {
                        let room = Self::max_stack(env, &carried).saturating_sub(carried.count);
                        self.carried = Some(carried.clone());
                        if let Some(mut taken) = self.try_remove(env, s, clicked.count, room) {
                            if let Some(c) = self.carried.as_mut() {
                                c.count += taken.count;
                            }
                            self.on_take(env, s, &taken);
                            if self.slots[s].role == Role::CartoResult {
                                // onCraftedBy edits the taken stack, not the carried one.
                                Self::map_crafted(env, &mut taken);
                            }
                        }
                    } else {
                        self.carried = Some(carried);
                    }
                }
            },
            Some(_) => {}
        }
    }

    /// The SWAP branch of `doClick`.
    fn swap(&mut self, env: &Env, target: usize, button: usize) {
        let inv_slot = button;
        let source = self.containers[PLAYER][inv_slot].clone();
        let target_stack = self.slot(target).cloned();
        if source.is_none() && target_stack.is_none() {
            return;
        }
        match (source, target_stack) {
            (None, Some(taken)) => {
                if self.may_pickup(env, target) {
                    self.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.set_by_player(env, target, None);
                    self.on_take(env, target, &taken);
                    if self.slots[target].role == Role::CartoResult
                        && let Some(stack) = self.containers[PLAYER][inv_slot].as_mut()
                    {
                        Self::map_crafted(env, stack);
                    }
                }
            }
            (Some(mut source), None) => {
                if self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                    } else {
                        self.containers[PLAYER][inv_slot] = None;
                        self.set_by_player(env, target, Some(source));
                    }
                }
            }
            (Some(mut source), Some(mut taken)) => {
                if self.may_pickup(env, target) && self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                        self.on_take(env, target, &taken);
                        if !self.inventory_add(env, &mut taken) {
                            self.drop_stack(&taken);
                        }
                    } else {
                        self.containers[PLAYER][inv_slot] = Some(taken.clone());
                        self.set_by_player(env, target, Some(source));
                        self.on_take(env, target, &taken);
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// The PICKUP_ALL branch (double click).
    /// `canTakeItemForPickAll`: never the result slots of the menus that
    /// override it (crafting, smithing, stonecutter, cartography), MerchantMenu's
    /// or an item tab's display slots.
    fn can_take_for_pick_all(&self, t: usize) -> bool {
        self.kind != Kind::Merchant
            && self.slots[t].role != Role::CreativeDisplay
            && (self.slots[t].container != RESULT
                || matches!(self.kind, Kind::Loom | Kind::Grindstone | Kind::Anvil))
    }

    /// `AbstractContainerScreen.mouseReleased` on a Shift double click:
    /// QUICK_MOVE on every slot of `clicked`'s container that holds
    /// `moved`'s item (`lastQuickMoved`, the stack the first click moved).
    pub fn quick_move_all(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
    ) -> Result<(), &'static str> {
        self.quick_move_all_each(env, clicked, moved, &mut |_, _| {})
    }

    /// `quick_move_all`, calling `each` with the menu and the slot after each
    /// QUICK_MOVE (a server's client sends every one of them).
    pub fn quick_move_all_each(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
        each: &mut dyn FnMut(&Self, usize),
    ) -> Result<(), &'static str> {
        if clicked >= self.slots.len() || !self.can_take_for_pick_all(clicked) {
            return Ok(());
        }
        let container = self.slots[clicked].container;
        for t in 0..self.slots.len() {
            if self.may_pickup(env, t)
                && self.has_item(t)
                && self.slots[t].container == container
                && self.quick_replace(t, moved, env)
            {
                self.click(env, t as i32, 0, Input::QuickMove)?;
                each(self, t);
            }
        }
        Ok(())
    }

    /// `AbstractContainerScreen.shouldAddSlotToQuickCraft`: whether a drag
    /// of type `ty` over `dragged` slots so far takes slot `s` too.
    pub fn quick_craft_accepts(&self, env: &Env, s: usize, ty: i32, dragged: usize) -> bool {
        let Some(carried) = self.carried.as_ref() else {
            return false;
        };
        s < self.slots.len()
            && (carried.count as usize > dragged || ty == 2)
            && self.quick_replace(s, carried, env)
            && self.may_place(env, s, carried)
            && self.can_drag_to(s)
    }

    /// What a quick craft drag of type `ty` over `slots` shows
    /// (`AbstractContainerScreen.extractSlot` and
    /// `recalculateQuickCraftRemaining`): each slot's would-be count and
    /// whether the slot caps it (a yellow count), then the count left in
    /// hand. Nothing while the drag spans a single slot.
    pub fn quick_craft_preview(
        &self,
        env: &Env,
        ty: i32,
        slots: &[usize],
    ) -> Option<(Vec<Option<(u32, bool)>>, u32)> {
        let carried = self.carried.as_ref()?;
        if slots.len() < 2 {
            return None;
        }
        let carried_max = Self::max_stack(env, carried);
        // AbstractContainerMenu.getQuickCraftPlaceCount.
        let place = match ty {
            0 => carried.count / slots.len() as u32,
            1 => 1,
            2 => carried_max,
            _ => carried.count,
        };
        let mut left = i64::from(if ty == 2 { carried_max } else { carried.count });
        let mut shown = Vec::with_capacity(slots.len());
        for &s in slots {
            let carry = self.slot(s).map_or(0, |i| i.count);
            let max = carried_max.min(self.slot_max(env, s, carried));
            if ty != 2 {
                left -= i64::from((place + carry).min(max)) - i64::from(carry);
            }
            shown.push(
                (s < self.slots.len()
                    && self.quick_replace(s, carried, env)
                    && self.can_drag_to(s))
                .then(|| ((place + carry).min(max), place + carry > max)),
            );
        }
        Some((shown, left.max(0) as u32))
    }

    /// `canDragTo`: ItemPickerMenu keeps drags off its display slots.
    fn can_drag_to(&self, s: usize) -> bool {
        self.slots[s].role != Role::CreativeDisplay
    }

    fn pickup_all(&mut self, env: &Env, s: usize, button: i32) {
        let Some(carried) = self.carried.clone() else {
            return;
        };
        if self.has_item(s) && self.may_pickup(env, s) {
            return;
        }
        let total = self.slots.len() as i32;
        let (start, step) = if button == 0 { (0, 1) } else { (total - 1, -1) };
        let carried_max = Self::max_stack(env, &carried);
        for pass in 0..2 {
            let mut i = start;
            while i >= 0 && i < total && self.carried.as_ref().map_or(0, |c| c.count) < carried_max
            {
                let t = i as usize;
                i += step;
                let have = self.carried.clone().expect("some");
                if self.has_item(t)
                    && self.quick_replace(t, &have, env)
                    && self.may_pickup(env, t)
                    && self.can_take_for_pick_all(t)
                {
                    let item = self.slot(t).expect("has item").clone();
                    if pass != 0 || item.count != Self::max_stack(env, &item) {
                        let room = carried_max - have.count;
                        if let Some(removed) = self.safe_take(env, t, item.count, room)
                            && let Some(c) = self.carried.as_mut()
                        {
                            c.count += removed.count;
                        }
                    }
                }
            }
        }
    }
}

/// `MerchantMenu` and `MerchantContainer` over the menu's `merchant` session.
impl Menu {
    /// `MerchantContainer.updateSellItem`.
    pub fn merchant_update(&mut self, env: &Env) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        let (a, b) = (
            self.containers[MAIN][0].as_ref(),
            self.containers[MAIN][1].as_ref(),
        );
        let (a, b) = if a.is_none() { (b, None) } else { (a, b) };
        m.active = None;
        if a.is_none() {
            self.containers[RESULT][0] = None;
            m.future_xp = 0;
            return;
        }
        if !m.offers.is_empty() {
            let (active, result) =
                crate::merchant::sell_item(&m.offers, env.items, a, b, Some(m.selection_hint));
            m.active = active;
            m.future_xp = if result.is_some() {
                active.map_or(0, |i| m.offers[i].xp)
            } else {
                0
            };
            self.containers[RESULT][0] = result;
        }
        let has = self.containers[RESULT][0].is_some();
        m.notify_trade_updated(has);
    }
    /// `MerchantResultSlot.onTake`: the payment is taken and the merchant
    /// told (`notifyTrade`).
    fn merchant_take(&mut self, env: &Env) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        let Some(i) = m.active else { return };
        let Some(offer) = m.offers.get(i).cloned() else {
            return;
        };
        let (mut a, mut b) = (
            self.containers[MAIN][0].take(),
            self.containers[MAIN][1].take(),
        );
        let took = offer.take(env.items, &mut a, &mut b) || offer.take(env.items, &mut b, &mut a);
        // The payment stacks shrink in place, before the merchant hears of it.
        self.containers[MAIN][0] = a;
        self.containers[MAIN][1] = b;
        if took {
            let prices = m.notify_trade(i, env.trade, env.items);
            if prices {
                self.merchant_update(env);
            }
            // setItem(0, buyA), setItem(1, buyB).
            self.merchant_update(env);
            self.merchant_update(env);
        }
    }
    /// `MerchantMenu.tryMoveItems` (the select trade packet, with the
    /// selection hint set first).
    pub fn merchant_select(&mut self, env: &Env, index: usize) {
        let Some(m) = self.merchant.as_mut() else {
            return;
        };
        // MerchantContainer.setSelectionHint updates the result.
        m.selection_hint = index;
        let count = m.offers.len();
        self.merchant_update(env);
        if index >= count {
            return;
        }
        for slot in [0usize, 1] {
            if let Some(mut old) = self.containers[MAIN][slot].clone() {
                if !self.move_to(env, &mut old, 3, 39, true) {
                    return;
                }
                self.containers[MAIN][slot] = (old.count > 0).then_some(old);
                self.put(env, slot, self.containers[MAIN][slot].clone());
            }
        }
        if self.containers[MAIN][0].is_some() || self.containers[MAIN][1].is_some() {
            return;
        }
        let Some(offer) = self
            .merchant
            .as_ref()
            .and_then(|m| m.offers.get(index))
            .cloned()
        else {
            return;
        };
        self.merchant_fill(env, 0, &offer.base_cost_a);
        if let Some(b) = &offer.cost_b {
            self.merchant_fill(env, 1, b);
        }
    }
    /// `MerchantMenu.moveFromInventoryToPaymentSlot`.
    fn merchant_fill(&mut self, env: &Env, payment: usize, cost: &crate::merchant::ItemCost) {
        for i in 3..39 {
            let Some(have) = self.slot(i).cloned() else {
                continue;
            };
            if !cost.test(&have) {
                continue;
            }
            let current = self.containers[MAIN][payment].clone();
            if current
                .as_ref()
                .is_some_and(|c| !c.same_item_same_components(&have))
            {
                continue;
            }
            let max = Self::max_stack(env, &have);
            let current_count = current.as_ref().map_or(0, |c| c.count);
            let moved = max.saturating_sub(current_count).min(have.count);
            let payment_stack = ItemStack {
                count: current_count + moved,
                ..have.clone()
            };
            let slot = self.slots[i];
            let mut left = have;
            left.count -= moved;
            self.containers[slot.container][slot.index] = (left.count > 0).then_some(left);
            self.put(env, payment, Some(payment_stack.clone()));
            if payment_stack.count >= max {
                break;
            }
        }
    }
    /// `MerchantMenu.removed` for a living player: the payment goes back to
    /// the inventory (`placeItemBackInInventory`), or is dropped.
    pub fn merchant_close(&mut self, env: &Env) {
        // AbstractContainerMenu.removed: the carried stack first.
        if let Some(mut carried) = self.carried.take() {
            self.place_back(env, &mut carried);
        }
        for slot in [0usize, 1] {
            if let Some(mut stack) = self.containers[MAIN][slot].take() {
                self.place_back(env, &mut stack);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{banner::Banners, brewing::Brewing, crafting::Recipes};
    use horizon_world::loot::PackOverlay;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    /// An item tab's ItemPickerMenu: the 45 display slots show `shown`.
    fn with_picker(shown: &str, test: impl FnOnce(&Env, &mut Menu)) {
        with_menu(Kind::ItemPicker, |env, menu| {
            assert_eq!(menu.slot_count(), 54);
            for i in 0..45 {
                menu.set_slot(env, i, stack(shown, 1));
            }
            test(env, menu);
        });
    }

    /// A menu over the game's data (`.local/horizon-client`); skipped without it.
    pub(crate) fn with_menu(kind: Kind, test: impl FnOnce(&Env, &mut Menu)) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let items = Items::catalog(&root, &PackOverlay::NONE).unwrap();
        let recipes = Recipes::load(&root, &items, &PackOverlay::NONE).unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let banners = Banners::load(&root);
        let station_recipes =
            crate::stations::StationRecipes::load(&root, &items, &PackOverlay::NONE);
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::creative::Names::default();
        let env = Env {
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
        };
        let mut menu = Menu::new(kind);
        menu.creative = true;
        test(&env, &mut menu);
    }

    /// The Mouse Tweaks wheel's targets: vanilla's routing of one item.
    #[test]
    fn quick_move_landing_routes_a_furnace_like_vanilla() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // 0 input, 1 fuel, 2 output, 3..30 inventory, 30..39 hotbar.
            let mut landing = |item, count, from| {
                menu.set_slot(env, from, stack(item, count));
                let to = menu.quick_move_landing(env, from);
                menu.set_slot(env, from, None);
                for &t in &to {
                    menu.set_slot(env, t, None);
                }
                to
            };
            assert_eq!(landing("raw_iron", 5, 30), [0]);
            assert_eq!(landing("coal", 5, 30), [1]);
            // Neither: between the hotbar and the inventory, as a shift-click.
            assert_eq!(landing("dirt", 5, 30), [3]);
            assert_eq!(landing("dirt", 5, 3), [30]);
            // Out of the fuel slot: moveItemStackTo(3, 39, false).
            assert_eq!(landing("coal", 5, 1), [3]);
        });
    }

    #[test]
    fn quick_move_landing_never_fills_a_result() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 30, stack("iron_ingot", 5));
            // An ingot with room in the output still goes elsewhere.
            assert_eq!(menu.quick_move_landing(env, 30), [3]);
            assert_eq!(menu.slot(2).map(|s| s.count), Some(5));
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // The whole output moves: into the partial stack, then an empty slot.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 20, stack("iron_ingot", 62));
            assert_eq!(menu.quick_move_landing(env, 2), [20, 38]);
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // A full inventory: the output stays, nothing to land on.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            for i in 3..39 {
                menu.set_slot(env, i, stack("stone", 64));
            }
            assert!(menu.quick_move_landing(env, 2).is_empty());
        });
    }

    #[test]
    fn quick_move_landing_routes_a_crafting_table_like_vanilla() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            // 0 result, 1..10 grid, 10..37 inventory, 37..46 hotbar.
            menu.set_slot(env, 37, stack("oak_log", 2));
            assert_eq!(menu.quick_move_landing(env, 37), [1]);
        });
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
            assert_eq!(menu.quick_move_landing(env, 0), [45]);
        });
    }

    #[test]
    fn quick_move_landing_routes_plain_containers() {
        for (kind, size) in [
            (Kind::Hopper, 5),
            (Kind::Dispenser, 9),
            (Kind::Chest(3), 27),
        ] {
            with_menu(kind, |env, menu| {
                menu.creative = false;
                let hotbar = size + 27;
                menu.set_slot(env, hotbar, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, hotbar), [0]);
            });
            with_menu(kind, |env, menu| {
                menu.creative = false;
                menu.set_slot(env, 0, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, 0), [size + 35]);
            });
        }
    }

    #[test]
    fn takes_one_follows_the_slots_rules() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            let (coal, cobble) = (stack("coal", 1).unwrap(), stack("cobblestone", 1).unwrap());
            let bucket = stack("bucket", 1).unwrap();
            assert!(menu.takes_one(env, 0, &cobble), "the input takes anything");
            assert!(menu.takes_one(env, 1, &coal));
            assert!(!menu.takes_one(env, 1, &cobble), "not fuel");
            assert!(!menu.takes_one(env, 2, &coal), "the output");
            menu.set_slot(env, 1, Some(bucket.clone()));
            assert!(
                !menu.takes_one(env, 1, &bucket),
                "one bucket in the fuel slot"
            );
            menu.set_slot(env, 1, stack("coal", 1));
            assert!(!menu.takes_one(env, 1, &bucket), "another item");
            assert!(menu.takes_one(env, 1, &coal));
        });
    }

    /// `HorseInventoryMenu`: a shift-click wears horse armour, then saddles,
    /// then fills the chest; the body slot takes only what the mount may wear.
    #[test]
    fn mount_menu_slots_take_what_the_mount_wears() {
        with_menu(Kind::Mount(5), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:donkey".into());
            // 2 saddle and body, 15 chest, then the inventory from 17.
            assert_eq!(menu.slot_count(), 2 + 15 + 36);
            let hotbar = 17 + 27;
            menu.set_slot(env, hotbar, stack("iron_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            // A donkey wears no horse armour: it goes in the chest.
            assert_eq!(menu.slot(1), None);
            assert_eq!(
                menu.slot(2).map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            menu.set_slot(env, hotbar, stack("saddle", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
            // Back out of the mount: to the inventory, from the hotbar's end.
            menu.click(env, 0, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(
                menu.slot(17 + 35).map(|s| &*s.item),
                Some("minecraft:saddle")
            );
        });
        with_menu(Kind::Mount(0), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:horse".into());
            let hotbar = 2 + 27;
            menu.set_slot(env, hotbar, stack("diamond_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            // No chest: dirt goes between the hotbar and the inventory.
            menu.set_slot(env, hotbar, stack("dirt", 3));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(2).map(|s| (&*s.item, s.count)),
                Some(("minecraft:dirt", 3))
            );
            assert_eq!(menu.slot(hotbar), None);
            // Placing by hand: a saddle doesn't go in the body slot.
            menu.carried = stack("saddle", 1);
            menu.click(env, 1, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
        });
    }

    /// `onEquipItem` runs from the armour and off-hand slots' `setByPlayer`
    /// when the item changes: never from `Inventory.setItem` (a swap's source).
    #[test]
    fn armour_and_off_hand_slots_report_what_is_equipped() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            let name = |e: &[(&str, Option<ItemStack>)]| {
                e.iter()
                    .map(|(slot, s)| format!("{slot}:{}", s.as_ref().map_or("-", |s| &*s.item)))
                    .collect::<Vec<_>>()
            };
            // A shift-click puts the helmet on (InventoryMenu.quickMoveStack).
            menu.set_slot(env, 9, stack("iron_helmet", 1));
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(name(&menu.equipped), ["head:minecraft:iron_helmet"]);
            // Taking it off; the same helmet back on is a change both ways.
            menu.equipped.clear();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            assert_eq!(
                name(&menu.equipped),
                ["head:-", "head:minecraft:iron_helmet"]
            );
            // F over a hotbar slot: the shield reaches the off hand by
            // Inventory.setItem, so nothing; a number key onto the off-hand
            // slot sets it by player.
            menu.equipped.clear();
            menu.set_slot(env, 36, stack("shield", 1));
            menu.click(env, 36, 40, Input::Swap).unwrap();
            assert_eq!(menu.slot(45).map(|s| &*s.item), Some("minecraft:shield"));
            assert!(menu.equipped.is_empty());
            menu.click(env, 45, 1, Input::Swap).unwrap();
            assert_eq!(name(&menu.equipped), ["offhand:-"]);
            // Other menus have no equipment slots.
            let mut chest = Menu::new(Kind::Chest(1));
            chest.set_slot(env, 0, stack("iron_helmet", 1));
            chest.click(env, 0, 0, Input::QuickMove).unwrap();
            assert!(chest.equipped.is_empty());
        });
    }

    #[test]
    fn a_quick_craft_drag_previews_each_slot_and_the_rest_in_hand() {
        with_menu(Kind::Inventory, |env, menu| {
            // 9 + 36 player slots: 9 is the first main inventory slot.
            menu.set_slot(env, 10, stack("stone", 60));
            menu.set_slot(env, 11, stack("dirt", 1));
            menu.carried = stack("stone", 20);
            // shouldAddSlotToQuickCraft: same item or empty, placeable, not the result.
            assert!(menu.quick_craft_accepts(env, 9, 0, 0));
            assert!(menu.quick_craft_accepts(env, 10, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 11, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 0, 0, 1));
            // Stone has no place in an armor slot (5).
            assert!(!menu.quick_craft_accepts(env, 5, 0, 1));
            // A right drag stops once every carried item has a slot.
            assert!(!menu.quick_craft_accepts(env, 12, 1, 20));
            // A left drag over three slots: 6 each, the 60 capped at 64 (yellow), 2 + 4 left.
            assert_eq!(menu.quick_craft_preview(env, 0, &[9]), None);
            let (shown, left) = menu.quick_craft_preview(env, 0, &[9, 10, 12]).unwrap();
            assert_eq!(
                shown,
                vec![Some((6, false)), Some((64, true)), Some((6, false))]
            );
            assert_eq!(left, 20 - 6 - 4 - 6);
            // A right drag: one each.
            let (shown, left) = menu.quick_craft_preview(env, 1, &[9, 10]).unwrap();
            assert_eq!(shown, vec![Some((1, false)), Some((61, false))]);
            assert_eq!(left, 18);
        });
    }

    #[test]
    fn item_picker_quick_move_deletes_a_hotbar_stack() {
        with_picker("stone", |env, menu| {
            menu.set_slot(env, 45, stack("dirt", 12));
            menu.set_slot(env, 46, stack("dirt", 5));
            menu.click(env, 45, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.player_item(0), None);
            // Nothing moves anywhere: the other hotbar stack and the tab stay.
            assert_eq!(menu.player_item(1), stack("dirt", 5).as_ref());
            assert_eq!(menu.slot(0), stack("stone", 1).as_ref());
            assert_eq!(menu.carried, None);
        });
    }

    #[test]
    fn item_picker_drags_and_pickup_all_skip_the_display_slots() {
        with_picker("dirt", |env, menu| {
            // A right-drag over a display slot and two hotbar slots.
            menu.carried = stack("dirt", 10);
            for (slot, header) in [(-999, 0), (3, 1), (45, 1), (47, 1), (-999, 2)] {
                menu.click(env, slot, header | (1 << 2), Input::QuickCraft)
                    .unwrap();
            }
            assert_eq!(menu.player_item(0), stack("dirt", 1).as_ref());
            assert_eq!(menu.player_item(2), stack("dirt", 1).as_ref());
            assert_eq!(menu.slot(3), stack("dirt", 1).as_ref());
            assert_eq!(menu.carried, stack("dirt", 8));
            // A double click gathers from the hotbar only.
            menu.click(env, 48, 0, Input::PickupAll).unwrap();
            assert_eq!(menu.carried, stack("dirt", 10));
            assert_eq!(menu.player_item(0), None);
            assert_eq!(menu.slot(0), stack("dirt", 1).as_ref());
        });
    }

    /// #1737: a Shift+Right-Click quick move in a chest (a double chest's
    /// 54 slots) moves the stack and leaves the menu's state clean, so the
    /// next quick move still moves its stack and the inventory still
    /// answers.
    #[test]
    fn a_chest_quick_move_keeps_the_next_one_working() {
        with_menu(Kind::Chest(6), |env, menu| {
            menu.creative = false;
            // The hotbar holds two stacks; the chest is empty.
            menu.set_slot(env, 81, stack("cobblestone", 64));
            menu.set_slot(env, 82, stack("dirt", 32));
            // Shift+Right-Click on the hotbar: the stack goes into the chest.
            menu.click(env, 81, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(81), None);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(64));
            // Nothing is left on the cursor and no quick craft is running.
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
            // The next quick move still works.
            menu.click(env, 82, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(82), None);
            assert_eq!(menu.slot(1).map(|s| s.count), Some(32));
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
        });
    }

    /// #1832: a potion taken from the creative inventory and put in the
    /// hotbar keeps its `PotionContents` (the brew), as vanilla's
    /// `PotionContents.createItemStack` builds it: the item tab's stack is
    /// the real one, and the hotbar slot holds it as it is.
    #[test]
    fn a_creative_potion_keeps_its_contents_in_the_hotbar() {
        let snbt = r#"{components:{"minecraft:potion_contents":{potion:"minecraft:swiftness"}},count:1,id:"minecraft:potion"}"#;
        let potion = crate::item_data::parse_stack(snbt).unwrap().unwrap();
        assert_eq!(potion.potion(), Some("minecraft:swiftness"));
        // The creative screen hands the stack over...
        let mut carried = None;
        crate::creative::click_display(
            &mut carried,
            Some(&potion),
            |_: &ItemStack| 1,
            0,
            Input::Pickup,
        );
        assert_eq!(
            carried.as_ref().and_then(|s| s.potion()),
            Some("minecraft:swiftness")
        );
        // ... and the hotbar slot keeps it: it is not the uncraftable
        // potion, which is a `minecraft:potion` with no contents at all.
        with_menu(Kind::ItemPicker, |env, menu| {
            menu.carried = carried;
            menu.click(env, 45, 0, Input::Pickup).unwrap();
            let in_hand = menu.slot(45).expect("the hotbar slot");
            assert_eq!(in_hand.potion(), Some("minecraft:swiftness"));
            assert_eq!(&*in_hand.item, "minecraft:potion");
        });
    }
}
