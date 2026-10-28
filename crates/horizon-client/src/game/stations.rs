//! Workstation menus in the game: enchanting table, anvil, grindstone,
//! smithing table, stonecutter and the brewing stand screen. Layouts, sprites,
//! colours and sounds follow the vanilla screens (`EnchantmentScreen`,
//! `AnvilScreen`, `GrindstoneScreen`, `SmithingScreen`, `StonecutterScreen`,
//! `BrewingStandScreen`); item rules live in `horizon_client::stations`.
use super::{Game, Screen};
use horizon_client::{
    block_entity_item, brewing,
    containers::BlockEntity,
    enchant::Enchantments,
    hud::{HudBatch, rgba},
    items::ItemStack,
    stations::{self, AnvilResult, EnchantOffers},
    text::Text,
};
use horizon_core::random::{LegacyRandom, RandomSource};

/// Slot ids of the open station: inputs from 600, the result at 650.
pub(super) const INPUT: usize = 600;
pub(super) const RESULT: usize = 650;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Enchanting,
    Anvil,
    Grindstone,
    Smithing,
    Stonecutter,
    /// Villager trading (game/trading.rs).
    Merchant,
    /// The beacon menu (game/beacon.rs).
    Beacon,
    /// The cartography table (game/cartography.rs).
    Cartography,
    /// The loom (game/loom.rs).
    Loom,
}
impl Kind {
    pub(super) fn from_block(name: &str) -> Option<Self> {
        Some(match name {
            "minecraft:enchanting_table" => Self::Enchanting,
            "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
                Self::Anvil
            }
            "minecraft:grindstone" => Self::Grindstone,
            "minecraft:smithing_table" => Self::Smithing,
            "minecraft:stonecutter" => Self::Stonecutter,
            "minecraft:beacon" => Self::Beacon,
            "minecraft:cartography_table" => Self::Cartography,
            "minecraft:loom" => Self::Loom,
            _ => return None,
        })
    }
    pub(super) fn inputs(self) -> usize {
        match self {
            Self::Smithing | Self::Loom => 3,
            Self::Stonecutter | Self::Beacon => 1,
            _ => 2,
        }
    }
    fn texture(self) -> &'static str {
        match self {
            Self::Enchanting => "gui/container/enchanting_table",
            Self::Anvil => "gui/container/anvil",
            Self::Grindstone => "gui/container/grindstone",
            Self::Smithing => "gui/container/smithing",
            Self::Stonecutter => "gui/container/stonecutter",
            Self::Merchant => "gui/container/villager",
            Self::Beacon => "gui/container/beacon",
            Self::Cartography => "gui/container/cartography_table",
            Self::Loom => "gui/container/loom",
        }
    }
    pub(super) fn title_key(self) -> &'static str {
        match self {
            Self::Enchanting => "container.enchant",
            Self::Anvil => "container.repair",
            Self::Grindstone => "container.grindstone_title",
            Self::Smithing => "container.upgrade",
            Self::Stonecutter => "container.stonecutter",
            Self::Merchant => "merchant.trades",
            Self::Beacon => "container.beacon",
            Self::Cartography => "container.cartography_table",
            Self::Loom => "container.loom",
        }
    }
    /// (titleLabelX, titleLabelY).
    fn title_pos(self) -> (f32, f32) {
        match self {
            Self::Anvil => (60.0, 6.0),
            Self::Smithing => (44.0, 15.0),
            Self::Stonecutter => (8.0, 5.0),
            Self::Cartography | Self::Loom => (8.0, 4.0),
            _ => (8.0, 6.0),
        }
    }
    fn input_pos(self, i: usize) -> (f32, f32) {
        let table: &[(f32, f32)] = match self {
            Self::Enchanting => &[(15.0, 47.0), (35.0, 47.0)],
            Self::Anvil => &[(27.0, 47.0), (76.0, 47.0)],
            Self::Grindstone => &[(49.0, 19.0), (49.0, 40.0)],
            Self::Smithing => &[(8.0, 48.0), (26.0, 48.0), (44.0, 48.0)],
            Self::Stonecutter => &[(20.0, 33.0)],
            Self::Merchant => &[(86.0, 37.0), (112.0, 37.0)],
            Self::Beacon => &[(109.0, 110.0)],
            Self::Cartography => &[(15.0, 15.0), (15.0, 52.0)],
            Self::Loom => &[(13.0, 26.0), (33.0, 26.0), (23.0, 45.0)],
        };
        table[i.min(table.len() - 1)]
    }
    /// Whether the menu has a result slot after its inputs.
    pub(super) fn has_result(self) -> bool {
        self.result_pos().is_some()
    }
    fn result_pos(self) -> Option<(f32, f32)> {
        match self {
            Self::Enchanting => None,
            Self::Anvil => Some((134.0, 47.0)),
            Self::Grindstone => Some((129.0, 34.0)),
            Self::Smithing => Some((98.0, 48.0)),
            Self::Stonecutter => Some((143.0, 33.0)),
            Self::Merchant => Some((170.0, 37.0)),
            Self::Beacon => None,
            Self::Cartography => Some((145.0, 39.0)),
            Self::Loom => Some((143.0, 57.0)),
        }
    }
}

pub(super) struct Station {
    pub(super) kind: Kind,
    pub(super) pos: [i32; 3],
    pub(super) inputs: Vec<Option<ItemStack>>,
    pub(super) result: Option<ItemStack>,
    pub(super) offers: EnchantOffers,
    bookcases: i32,
    pub(super) anvil: AnvilResult,
    /// Anvil rename box text (`AnvilMenu.itemName`).
    pub(super) name: String,
    /// The box itself (`AnvilScreen`'s `EditBox` of 50, unbordered).
    pub(super) rename: horizon_client::ui::EditBox,
    /// Stonecutter: the menu's recipe list (horizon_client::menu), its
    /// `selectedRecipeIndex` and the screen's first shown index.
    pub(super) stone: horizon_client::menu::Stonecutter,
    pub(super) selected: Option<usize>,
    start: usize,
    /// Anvil: the input the rename box was last filled from.
    anvil_last: Option<ItemStack>,
    /// Anvil on a server: the name last sent (the client menu's `itemName`).
    pub(super) anvil_sent: Option<String>,
    pub(super) last_sound: u64,
    /// Villager trading state.
    pub(super) merchant: Option<Box<super::trading::MerchantState>>,
    /// Beacon menu: the selected primary and secondary power (EFFECTS index).
    pub(super) beacon: [Option<usize>; 2],
    /// A server's beacon menu: the pyramid's levels (its data slot 0).
    pub(super) beacon_levels: Option<i32>,
    /// The loom's patterns, selection and scroller (game/loom.rs).
    pub(super) loom: super::loom::LoomState,
}
impl Station {
    fn new(kind: Kind, pos: [i32; 3]) -> Self {
        Self {
            kind,
            pos,
            inputs: vec![None; kind.inputs()],
            result: None,
            offers: EnchantOffers::default(),
            bookcases: 0,
            anvil: AnvilResult::default(),
            name: String::new(),
            rename: anvil_box(),
            stone: Default::default(),
            selected: None,
            start: 0,
            anvil_last: None,
            anvil_sent: None,
            last_sound: 0,
            merchant: None,
            beacon: [None; 2],
            beacon_levels: None,
            loom: Default::default(),
        }
    }
    pub(super) fn new_kind(kind: Kind, pos: [i32; 3]) -> Self {
        Self::new(kind, pos)
    }
    /// `StonecutterScreen.containerChanged`: without recipes to show, the
    /// list scrolls back to the top.
    pub(super) fn stonecutter_changed(&mut self) {
        if self.inputs[0].is_none() || self.stone.recipes.is_empty() {
            self.start = 0;
        }
    }
}

/// The 108x19 enchantment button `i` (relative to the GUI origin).
fn enchant_button(i: usize) -> (f32, f32, f32, f32) {
    (60.0, 14.0 + 19.0 * i as f32, 108.0, 19.0)
}
fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
/// `EnchantmentNames` word list.
const WORDS: &[&str] = &[
    "the",
    "elder",
    "scrolls",
    "klaatu",
    "berata",
    "niktu",
    "xyzzy",
    "bless",
    "curse",
    "light",
    "darkness",
    "fire",
    "air",
    "earth",
    "water",
    "hot",
    "dry",
    "cold",
    "wet",
    "ignite",
    "snuff",
    "embiggen",
    "twist",
    "shorten",
    "stretch",
    "fiddle",
    "destroy",
    "imbue",
    "galvanize",
    "enchant",
    "free",
    "limited",
    "range",
    "of",
    "towards",
    "inside",
    "sphere",
    "cube",
    "self",
    "other",
    "ball",
    "mental",
    "physical",
    "grow",
    "shrink",
    "demon",
    "elemental",
    "spirit",
    "animal",
    "creature",
    "beast",
    "humanoid",
    "undead",
    "fresh",
    "stale",
    "phnglui",
    "mglwnafh",
    "cthulhu",
    "rlyeh",
    "wgahnagl",
    "fhtagn",
    "baguette",
];
const GREY: u32 = 0xFF40_4040;
fn argb(c: i32) -> u32 {
    let c = c as u32;
    rgba(
        (c >> 16 & 255) as u8,
        (c >> 8 & 255) as u8,
        (c & 255) as u8,
        (c >> 24 & 255) as u8,
    )
}

/// A text style's `0xRRGGBB` colour from a HUD colour (`rgba`).
fn rgb_of(rgba: u32) -> u32 {
    (rgba & 0xFF) << 16 | (rgba >> 8 & 0xFF) << 8 | (rgba >> 16 & 0xFF)
}
impl Game<'_> {
    /// `Component.translatable`: the selected language (which `lang` layers
    /// over `en_us`), the caller's English text last.
    pub(super) fn translate(&self, key: &str, fallback: &str) -> String {
        horizon_client::lang::get_or(key, fallback)
    }
    fn brewing_open(&self) -> bool {
        matches!(self.container_kind(), Some(BlockEntity::Brewing(_)))
    }
    /// Opens a workstation menu for the block; false when it is not one.
    pub(super) fn open_station(&mut self, pos: [i32; 3], name: &str) -> bool {
        let Some(kind) = Kind::from_block(name) else {
            return false;
        };
        if self.level.xp_seed == 0 {
            self.level.xp_seed = self.world.random.next_int();
        }
        self.station = Some(Station::new(kind, pos));
        self.station_changed();
        self.beacon_opened();
        // The block's interaction statistic.
        let stat = match name {
            "minecraft:crafting_table" => "interact_with_crafting_table",
            "minecraft:furnace" => "interact_with_furnace",
            "minecraft:blast_furnace" => "interact_with_blast_furnace",
            "minecraft:smoker" => "interact_with_smoker",
            "minecraft:brewing_stand" => "interact_with_brewingstand",
            "minecraft:beacon" => "interact_with_beacon",
            "minecraft:grindstone" => "interact_with_grindstone",
            "minecraft:smithing_table" => "interact_with_smithing_table",
            "minecraft:stonecutter" => "interact_with_stonecutter",
            "minecraft:loom" => "interact_with_loom",
            "minecraft:cartography_table" => "interact_with_cartography_table",
            n if n.ends_with("anvil") => "interact_with_anvil",
            _ => "",
        };
        if !stat.is_empty() {
            self.stats.custom(stat, 1);
        }
        true
    }
    /// `AbstractContainerMenu.clearContainer`: inputs go back to the player.
    pub(super) fn close_station(&mut self) {
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Merchant)
        {
            // MerchantMenu.removed: the carried stack and the payment go back.
            self.menu_session(|menu, env, _| {
                menu.merchant_close(env);
                Some(true)
            });
        }
        let Some(mut station) = self.station.take() else {
            return;
        };
        if self.remote.is_some() && station.kind != Kind::Merchant {
            // A server's workstation: its removed() gives the inputs back.
            return;
        }
        if let Some(m) = station.merchant.as_mut() {
            self.merchant_closed(m);
        }
        for stack in station.inputs.into_iter().flatten() {
            self.give_or_drop(stack);
        }
    }
    pub(super) fn station_slots(&self) -> Option<Vec<usize>> {
        let mut v: Vec<usize> = (0..36).collect();
        if self.brewing_open() {
            v.extend(300..305);
            return Some(v);
        }
        let s = self.station.as_ref()?;
        v.extend((0..s.inputs.len()).map(|i| INPUT + i));
        if s.kind.result_pos().is_some() {
            v.push(RESULT);
        }
        Some(v)
    }
    pub(super) fn station_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if self.brewing_open() && (300..305).contains(&i) {
            return Some(
                [
                    (56.0, 51.0),
                    (79.0, 58.0),
                    (102.0, 51.0),
                    (79.0, 17.0),
                    (17.0, 17.0),
                ][i - 300],
            );
        }
        let s = self.station.as_ref()?;
        if s.kind == Kind::Merchant && i < 36 {
            return Some(super::trading::inventory_slot(i));
        }
        if s.kind == Kind::Beacon {
            return super::beacon::slot_position(i);
        }
        match i {
            RESULT => s.kind.result_pos(),
            _ if i >= INPUT && i < INPUT + s.inputs.len() => Some(s.kind.input_pos(i - INPUT)),
            _ => None,
        }
    }
    pub(super) fn station_slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        match self.station.as_mut() {
            Some(s) if i == RESULT => &mut s.result,
            Some(s) if i >= INPUT && i < INPUT + s.inputs.len() => &mut s.inputs[i - INPUT],
            _ => &mut self.scratch_slot,
        }
    }
    pub(super) fn count_bookshelves(&self, pos: [i32; 3]) -> i32 {
        let tags = &self.res.items().block_tags;
        let is = |p: [i32; 3], tag: &str| {
            let name = &self
                .res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name;
            tags.get(tag).is_some_and(|t| t.contains(&**name))
        };
        stations::bookshelf_offsets()
            .into_iter()
            .filter(|o| {
                is(
                    [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]],
                    "minecraft:enchantment_power_provider",
                ) && is(
                    [pos[0] + o[0] / 2, pos[1] + o[1], pos[2] + o[2] / 2],
                    "minecraft:enchantment_power_transmitter",
                )
            })
            .count() as i32
    }
    /// The anvil name the menu works with (`AnvilScreen.onNameChanged`).
    pub(super) fn anvil_item_name(&self, s: &Station) -> String {
        match &s.inputs[0] {
            Some(input)
                if input.custom_name().is_none()
                    && s.name == self.pack.names.display_item(&input.item) =>
            {
                String::new()
            }
            Some(_) => s.name.clone(),
            None => String::new(),
        }
    }
    /// `slotsChanged` / `createResult` for the open station.
    pub(super) fn station_changed(&mut self) {
        let Some(mut s) = self.station.take() else {
            return;
        };
        let creative = self.creative;
        let data = self.res.stations();
        let items = self.res.items();
        match s.kind {
            // A server's table: its data slots carry the offers, and its
            // client menu only clears them without an enchantable item.
            Kind::Enchanting if self.remote.is_some() => {
                let enchantable = s.inputs[0].as_ref().is_some_and(|i| {
                    data.enchantments.enchantable(&i.item) > 0 && i.enchantments().is_empty()
                });
                if !enchantable {
                    s.offers = Default::default();
                }
            }
            Kind::Enchanting => {
                s.bookcases = self.count_bookshelves(s.pos);
                s.offers = stations::enchant_offers(
                    &data.enchantments,
                    items,
                    self.level.xp_seed,
                    s.bookcases,
                    s.inputs[0].as_ref(),
                );
            }
            Kind::Anvil => {
                let name = self.anvil_item_name(&s);
                let hover = s.inputs[0]
                    .as_ref()
                    .map(|i| stations::hover_name(&|id| self.pack.names.display_item(id), i))
                    .unwrap_or_default();
                s.anvil = stations::anvil_result(
                    &data.enchantments,
                    items,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                    Some(&name),
                    &hover,
                    creative,
                );
                s.result = s.anvil.result.clone();
            }
            Kind::Grindstone => {
                s.result = stations::grindstone_result(
                    &data.enchantments,
                    items,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                );
            }
            Kind::Smithing => {
                s.result = data.recipes.smithing_result(
                    &data.enchantments,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                    s.inputs[2].as_ref(),
                );
            }
            // The menu engine keeps the merchant's result (Game::menu_session).
            Kind::Merchant => {}
            Kind::Beacon => {}
            // The menu engine keeps the cartography table's, the loom's and
            // the stonecutter's state (Game::menu_session).
            Kind::Cartography | Kind::Loom | Kind::Stonecutter => {}
        }
        self.station = Some(s);
    }
    pub(super) fn max_stack(&self, stack: &ItemStack) -> u32 {
        self.res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
    }
    /// `Slot.mayPlace` and `getMaxStackSize` for station slots.
    fn slot_rules(&self, slot: usize, stack: &ItemStack) -> (bool, u32) {
        if slot >= super::mount_menu::SADDLE {
            return self.mount_slot_rules(slot, stack);
        }
        if let Some(BlockEntity::Crafter(c)) = self.container_kind()
            && (300..309).contains(&slot)
        {
            // CrafterSlot.mayPlace: not a disabled slot.
            return (!c.disabled[slot - 300], 64);
        }
        if (300..400).contains(&slot) && !self.brewing_open() && self.refuses(slot, stack) {
            // ShulkerBoxSlot.mayPlace.
            return (false, 64);
        }
        let Some(s) = self.station.as_ref() else {
            return (true, 64);
        };
        if slot < 36 {
            return (true, 64);
        }
        if slot == RESULT {
            return (false, 64);
        }
        (s.kind != Kind::Loom, 64)
    }
    /// `AbstractContainerMenu.doClick` PICKUP on a filtered slot.
    pub(super) fn click_filtered(&mut self, slot: usize, right: bool) {
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let (new_slot, new_carried) = match (carried, current) {
            (None, None) => (None, None),
            (None, Some(mut s)) => {
                let take = if right { s.count.div_ceil(2) } else { s.count };
                s.count -= take;
                let taken = ItemStack {
                    count: take,
                    ..s.clone()
                };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (Some(mut c), None) => {
                let (ok, cap) = self.slot_rules(slot, &c);
                if ok {
                    let n = if right { 1 } else { c.count }.min(cap.min(self.max_stack(&c)));
                    let placed = ItemStack {
                        count: n,
                        ..c.clone()
                    };
                    c.count -= n;
                    (Some(placed), (c.count > 0).then_some(c))
                } else {
                    (None, Some(c))
                }
            }
            (Some(mut c), Some(mut s)) => {
                let (ok, cap) = self.slot_rules(slot, &c);
                if ok && c.same_item_same_components(&s) {
                    let room = cap.min(self.max_stack(&s)).saturating_sub(s.count);
                    let n = if right { 1 } else { c.count }.min(room);
                    s.count += n;
                    c.count -= n;
                    (Some(s), (c.count > 0).then_some(c))
                } else if ok && c.count <= cap.min(self.max_stack(&c)) {
                    (Some(c), Some(s))
                } else if c.same_item_same_components(&s) {
                    let n = s.count.min(self.max_stack(&c).saturating_sub(c.count));
                    s.count -= n;
                    c.count += n;
                    ((s.count > 0).then_some(s), Some(c))
                } else {
                    (Some(s), Some(c))
                }
            }
        };
        *self.slot_mut(slot) = new_slot;
        self.carried = new_carried;
    }
    /// `moveItemStackTo` over `targets` (menu order), merging first, then empty slots.
    pub(super) fn move_to(
        &mut self,
        stack: &mut ItemStack,
        targets: &[usize],
        reverse: bool,
    ) -> bool {
        let order: Vec<usize> = if reverse {
            targets.iter().rev().copied().collect()
        } else {
            targets.to_vec()
        };
        let start = stack.count;
        if self.max_stack(stack) > 1 {
            for &t in &order {
                if stack.count == 0 {
                    break;
                }
                let (_, cap) = self.slot_rules(t, stack);
                let max = cap.min(self.max_stack(stack));
                if let Some(s) = self.slot_mut(t).as_mut()
                    && s.same_item_same_components(stack)
                {
                    let n = max.saturating_sub(s.count).min(stack.count);
                    s.count += n;
                    stack.count -= n;
                }
            }
        }
        for &t in &order {
            if stack.count == 0 {
                break;
            }
            let (ok, cap) = self.slot_rules(t, stack);
            if self.slot_mut(t).is_none() && (t < 36 || ok) {
                let n = cap.min(self.max_stack(stack)).min(stack.count);
                *self.slot_mut(t) = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                stack.count -= n;
            }
        }
        stack.count != start
    }
    /// After an anvil or smithing click on the menu engine: the anvil's cost
    /// and state back into the station and the rename box following the
    /// input (`AnvilScreen.slotChanged`), then the rest of a taken result's
    /// `onTake` (the levels, the anvil's wear, the use sound) unless a
    /// server does it.
    pub(super) fn combiner_session_end(
        &mut self,
        menu: &horizon_client::menu::Menu,
        predicting: bool,
    ) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let (pos, kind) = (s.pos, s.kind);
        if kind == Kind::Anvil {
            s.anvil.cost = menu.data[0];
            s.anvil.repair_item_count_cost = menu.anvil.repair_count;
            s.anvil.only_renaming = menu.anvil.only_renaming;
            s.anvil.result = s.result.clone();
        }
        self.station_after_input();
        if !menu.combined || predicting {
            return;
        }
        if kind == Kind::Smithing {
            self.level_event_at(1044, pos, 0);
            return;
        }
        self.give_levels(menu.level_change);
        if self.anvil_used(pos, self.creative) {
            self.close_station();
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// `AnvilMenu.onTake`'s level part: out of creative a 12% chance
    /// (`AnvilBlock.damage`) to wear the anvil a stage, or break a damaged one
    /// (level event 1029); otherwise level event 1030. True when it broke.
    pub(super) fn anvil_used(&mut self, pos: [i32; 3], creative: bool) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = self.res.facts().get(state).name.clone();
        let anvil = matches!(
            &*name,
            "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil"
        );
        if !creative && anvil && self.world.random.next_float() < 0.12 {
            match &*name {
                "minecraft:anvil" => {
                    let _ = self.set_state_named(pos, "minecraft:chipped_anvil", &["facing"]);
                }
                "minecraft:chipped_anvil" => {
                    let _ = self.set_state_named(pos, "minecraft:damaged_anvil", &["facing"]);
                }
                _ => {
                    let _ = self.world.set(pos[0], pos[1], pos[2], self.world.air);
                    self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
                    self.level_event_at(1029, pos, 0);
                    return true;
                }
            }
        }
        self.level_event_at(1030, pos, 0);
        false
    }
    /// The grindstone result's `onTake` with a level (`amount`: the menu's
    /// `getExperienceAmount` sum): the orbs at the block's centre
    /// (`ExperienceOrb.award`), then level event 1042, the grindstone's sound
    /// for everyone around.
    pub(super) fn grindstone_taken(&mut self, pos: [i32; 3], amount: i32) {
        let xp = stations::grindstone_xp_roll(amount, &mut self.world.random);
        if xp > 0 {
            self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), xp);
        }
        self.level_event_at(1042, pos, 0);
    }
    fn play_at(&mut self, sound: &str, pos: [i32; 3], volume: f32, pitch: f32) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio.play(sound, Some(at), volume, pitch);
    }
    pub(super) fn give_or_drop(&mut self, stack: ItemStack) {
        if let Some(rest) = self.give(stack)
            && let Some(eye) = self.eye_position()
        {
            // Inventory.placeItemBackInInventory: `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false);
        }
    }
    /// `Player.giveExperienceLevels`.
    pub(super) fn give_levels(&mut self, levels: i32) {
        let xp = &mut self.experience;
        xp.level = xp.level.saturating_add(levels);
        if xp.level < 0 {
            xp.level = 0;
            xp.progress = 0.0;
            xp.total = 0;
        }
    }
    /// Whether the result slot may be taken (`mayPickup`).
    fn may_take_result(&self) -> bool {
        let Some(s) = self.station.as_ref() else {
            return false;
        };
        match s.kind {
            Kind::Anvil => {
                (self.creative || self.experience.level >= s.anvil.cost) && s.anvil.cost > 0
            }
            _ => true,
        }
    }
    /// `EnchantmentMenu.clickMenuButton`.
    fn click_enchant(&mut self, button: usize) {
        let Some(s) = self.station.as_ref() else {
            return;
        };
        let cost = s.offers.costs[button];
        let lapis_needed = button as u32 + 1;
        let Some(item) = s.inputs[0].clone() else {
            return;
        };
        let lapis = s.inputs[1].as_ref().map_or(0, |l| l.count);
        if lapis < lapis_needed && !self.creative {
            return;
        }
        if cost <= 0
            || (self.experience.level < lapis_needed as i32 || self.experience.level < cost)
                && !self.creative
        {
            return;
        }
        if self.remote.is_some() {
            // EnchantmentScreen.mouseClicked: the client menu took it, the server enchants.
            self.remote_station_button(button as i32);
            return;
        }
        let Some(enchanted) = stations::apply_enchantment(
            &self.res.stations().enchantments,
            self.res.items(),
            self.level.xp_seed,
            button,
            cost,
            &item,
        ) else {
            return;
        };
        let pos = s.pos;
        let enchanted_item = enchanted.clone();
        // Player.onEnchantmentPerformed
        self.give_levels(-(lapis_needed as i32));
        self.level.xp_seed = self.world.random.next_int();
        if let Some(s) = self.station.as_mut() {
            s.inputs[0] = Some(enchanted);
            if !self.creative
                && let Some(l) = s.inputs[1].as_mut()
            {
                l.count -= lapis_needed;
                if l.count == 0 {
                    s.inputs[1] = None;
                }
            }
        }
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.stats.custom("enchant_item", 1);
        self.trigger_enchanted_item(&enchanted_item, lapis_needed as i32);
        self.play_at("minecraft:block.enchantment_table.use", pos, 1.0, pitch);
        self.station_changed();
    }
    /// Mouse click while a station or the brewing stand is open. True when handled.
    /// A station's own buttons a controller can focus besides its slots
    /// (GUI origin relative): offers, patterns, effects, enchantments and
    /// stonecutter recipes.
    pub(super) fn station_targets(&self) -> Vec<[f32; 4]> {
        let Some(s) = self.station.as_ref() else {
            return Vec::new();
        };
        match s.kind {
            Kind::Merchant => self.merchant_targets(),
            Kind::Loom => self.loom_targets(),
            Kind::Beacon => self.beacon_targets(),
            Kind::Enchanting => (0..3)
                .map(|i| {
                    let (x, y, w, h) = enchant_button(i);
                    [x, y, w, h]
                })
                .collect(),
            Kind::Stonecutter if s.inputs[0].is_some() => (s.start
                ..(s.start + 12).min(s.stone.recipes.len()))
                .map(|index| {
                    let p = index - s.start;
                    [
                        52.0 + (p % 4) as f32 * 16.0,
                        14.0 + (p / 4) as f32 * 18.0,
                        16.0,
                        18.0,
                    ]
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    pub(super) fn station_click(&mut self, right: bool) -> bool {
        // A server's brewing stand: its slots are the server's.
        if self.station.is_none() && (!self.brewing_open() || self.remote.is_some()) {
            return false;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return true;
        };
        let batch = HudBatch::new(size);
        let (ox, oy) = self.inventory_origin(&batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        let shift = self.has_shift_down();
        if let Some(kind) = self.station.as_ref().map(|s| s.kind) {
            if kind == Kind::Merchant && !right && self.merchant_click(local) {
                return true;
            }
            if kind == Kind::Merchant && self.remote.is_some() {
                // A server's trading window: its slots are the server's.
                return false;
            }
            if kind == Kind::Loom && !right && self.loom_click(local) {
                return true;
            }
            if kind == Kind::Beacon && !right && self.beacon_click(local) {
                return true;
            }
            if kind == Kind::Enchanting && !right {
                for i in 0..3 {
                    let (x, y, w, h) = enchant_button(i);
                    if inside(local, x, y, w, h) {
                        self.click_enchant(i);
                        return true;
                    }
                }
            }
            if kind == Kind::Stonecutter && self.stonecutter_click(local) {
                return true;
            }
            if kind == Kind::Anvil && !right && self.anvil_name_click(ox, oy) {
                return true;
            }
        }
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return true;
        };
        // Every station's clicks (and the brewing stand's) run on the
        // verified menu engine.
        let input = if shift {
            horizon_client::menu::Input::QuickMove
        } else {
            horizon_client::menu::Input::Pickup
        };
        self.menu_click(Some(slot), i32::from(right), input);
        true
    }
    /// Recompute after slot changes; the anvil box follows its input item.
    fn station_after_input(&mut self) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.kind == Kind::Anvil && s.inputs[0] != s.anvil_last {
            // AnvilScreen.slotChanged: the box shows the new input's name.
            s.anvil_last = s.inputs[0].clone();
            s.name = s.inputs[0]
                .as_ref()
                .map(|i| stations::hover_name(&|id| self.pack.names.display_item(id), i))
                .unwrap_or_default();
            let name = s.name.clone();
            s.rename.set_value(&name);
        }
        self.station_changed();
    }
    /// `StonecutterScreen.mouseClicked` on its recipe grid: `clickMenuButton`
    /// with the cell's index (an empty cell's too), and the select sound when
    /// the menu took it.
    fn stonecutter_click(&mut self, local: (f32, f32)) -> bool {
        let Some(start) = self
            .station
            .as_ref()
            .filter(|s| s.inputs[0].is_some() && !s.stone.recipes.is_empty())
            .map(|s| s.start)
        else {
            return false;
        };
        for index in start..start + 12 {
            let p = index - start;
            let (x, y) = (52.0 + (p % 4) as f32 * 16.0, 14.0 + (p / 4) as f32 * 18.0);
            if !inside(local, x, y, 16.0, 18.0) {
                continue;
            }
            let mut taken = false;
            let flag = &mut taken;
            self.menu_session(|menu, env, _| {
                *flag = menu.button(env, index as i32);
                Some(*flag)
            });
            if taken {
                self.audio
                    .play("minecraft:ui.stonecutter.select_recipe", None, 1.0, 1.0);
                self.remote_station_button(index as i32);
                return true;
            }
        }
        false
    }
    pub(super) fn station_scroll(&mut self, d: f32) {
        self.merchant_scroll(d);
        self.loom_scroll(d);
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.kind != Kind::Stonecutter || s.stone.recipes.len() <= 12 {
            return;
        }
        let rows = s.stone.recipes.len().div_ceil(4) - 3;
        let first_row = (s.start / 4) as i32 - d.signum() as i32;
        s.start = first_row.clamp(0, rows as i32) as usize * 4;
    }
    /// Whether keyboard text goes to the anvil rename box.
    pub(super) fn station_typing(&self) -> bool {
        self.screen == Screen::Inventory
            && self
                .station
                .as_ref()
                .is_some_and(|s| s.kind == Kind::Anvil && s.inputs[0].is_some())
    }
    pub(super) fn station_text(&mut self, text: Option<&str>, backspace: bool) {
        // The recipe book's search box takes scripted typing too.
        if self.recipe_search_focused() {
            self.recipe_text(text, backspace);
            return;
        }
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if backspace {
            s.rename.delete_chars(-1);
        } else if let Some(t) = text {
            s.rename.insert_text(t);
        }
        s.name.clone_from(&s.rename.value);
        self.station_changed();
        self.remote_anvil_rename();
    }
    /// `AnvilScreen.keyPressed`: the rename box handles the key.
    pub(super) fn station_key(&mut self, key: winit::keyboard::KeyCode) -> bool {
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let Some(s) = self.station.as_mut() else {
            return false;
        };
        if !s.rename.key(key, ctrl, shift) {
            return false;
        }
        s.name.clone_from(&s.rename.value);
        self.station_changed();
        self.remote_anvil_rename();
        true
    }
    /// Ctrl+A/C/V/X in the anvil's rename box (an `EditBox` of 50) or the
    /// recipe book's search box.
    pub(super) fn station_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        if self.recipe_search_focused() {
            self.recipe_shortcut(shortcut);
            return;
        }
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.rename.shortcut(shortcut, &mut self.clipboard) {
            s.name.clone_from(&s.rename.value);
            self.station_changed();
            self.remote_anvil_rename();
        }
    }
    /// A click in the anvil's name box (`EditBox.onClick`).
    fn anvil_name_click(&mut self, ox: f32, oy: f32) -> bool {
        let open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Anvil && s.inputs[0].is_some());
        if !open {
            return false;
        }
        let (x, y, w, h) = (ox + 62.0, oy + 24.0, 103.0, 12.0);
        let (mx, my) = self.cursor;
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }
        let shift = self.has_shift_down();
        let Some(s) = self.station.as_mut() else {
            return false;
        };
        s.rename.x = x;
        s.rename.y = y;
        s.rename.width = w;
        s.rename.click(&self.gui.atlas, mx, shift, false);
        true
    }
    /// AnvilScreen's name box, drawn after the station picture.
    fn draw_anvil_name(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let edit = &mut s.rename;
        edit.x = ox + 62.0;
        edit.y = oy + 24.0;
        edit.width = 103.0;
        edit.height = 12.0;
        edit.bordered = false;
        edit.focused = true;
        edit.text_color = u32::MAX;
        edit.max_length = ANVIL_NAME_CHARS;
        edit.draw(batch, &self.gui.atlas, None);
    }

    // ---------------------------------------------------------------- drawing

    fn part(
        batch: &mut HudBatch,
        atlas: &horizon_client::hud::HudAtlas,
        name: &str,
        src: (f32, f32),
        size: (f32, f32),
        dst: (f32, f32),
    ) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u0 += src.0 * du;
            r.v0 += src.1 * dv;
            r.u1 = r.u0 + size.0 * du;
            r.v1 = r.v0 + size.1 * dv;
            batch.rect_uv(dst.0, dst.1, size.0, size.1, r, u32::MAX);
        }
    }
    /// Text in the Standard Galactic alphabet (`minecraft:alt`), cut to `max` pixels.
    fn galactic(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32, max: f32, color: u32) {
        let atlas = &self.gui.atlas;
        let Some(sheet) = atlas.sprites.get("font/ascii_sga").copied() else {
            batch.text(atlas, text, x, y, color, false);
            return;
        };
        let cell = sheet.w / 16;
        let (px, py) = (
            (sheet.u0 * atlas.width as f32) as u32,
            (sheet.v0 * atlas.height as f32) as u32,
        );
        let mut cursor = x;
        for ch in text.chars() {
            let advance = if ch == ' ' {
                4.0
            } else {
                let code = ch as u32;
                if !(0x41..=0x5A).contains(&code) && !(0x61..=0x7A).contains(&code) {
                    continue;
                }
                let (col, row) = (code % 16, code / 16);
                let (cx, cy) = (px + col * cell, py + row * cell);
                let mut used = 0;
                for dx in (0..cell).rev() {
                    if (0..cell).any(|dy| {
                        atlas.pixels[(((cy + dy) * atlas.width + cx + dx) * 4 + 3) as usize] != 0
                    }) {
                        used = dx + 1;
                        break;
                    }
                }
                let advance = (used as f32 * 8.0 / cell as f32 + 0.5).floor() + 1.0;
                if cursor + advance - x > max {
                    break;
                }
                let r = horizon_client::hud::Rect {
                    u0: cx as f32 / atlas.width as f32,
                    v0: cy as f32 / atlas.height as f32,
                    u1: (cx + cell) as f32 / atlas.width as f32,
                    v1: (cy + cell) as f32 / atlas.height as f32,
                    w: cell,
                    h: cell,
                };
                batch.rect_uv(cursor, y, 8.0, 8.0, r, color);
                advance
            };
            if cursor + advance - x > max {
                break;
            }
            cursor += advance;
        }
    }
    /// Draws the open station or brewing stand screen; false when none is open.
    pub(super) fn draw_station(&mut self, batch: &mut HudBatch) -> bool {
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Merchant)
        {
            return self.draw_merchant(batch);
        }
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Beacon)
        {
            self.draw_beacon(batch);
            return true;
        }
        let brewing = self.brewing_open();
        if self.station.is_none() && !brewing {
            return false;
        }
        let (ox, oy) = self.inventory_origin(batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        let mut tooltip: Option<Vec<(horizon_client::text::Text, u32)>> = None;
        let mut anvil_name = false;
        // Keep item tooltips as stacks until drawing: text alone loses the
        // bundle contents component (including in the player's inventory).
        let mut tooltip_stack: Option<ItemStack> = None;
        {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }
        let inventory_label = self.translate("container.inventory", "Inventory");
        if brewing {
            let Some(BlockEntity::Brewing(b)) = self.container_kind() else {
                return true;
            };
            let (fuel, total_fuel, brew, total_brew) =
                (b.fuel, b.total_fuel, b.brew_time, b.total_brew_time);
            let atlas = &self.gui.atlas;
            Self::part(
                batch,
                atlas,
                "gui/container/brewing_stand",
                (0.0, 0.0),
                (176.0, 166.0),
                (ox, oy),
            );
            let fuel_len = if total_fuel > 0 {
                ((18 * fuel + total_fuel - 1) / total_fuel).clamp(0, 18)
            } else {
                0
            };
            if fuel_len > 0 {
                Self::part(
                    batch,
                    atlas,
                    "gui/sprites/container/brewing_stand/fuel_length",
                    (0.0, 0.0),
                    (fuel_len as f32, 4.0),
                    (ox + 60.0, oy + 44.0),
                );
            }
            if brew > 0 && total_brew > 0 {
                let len = (28.0 * (1.0 - brew as f32 / total_brew as f32)) as i32;
                if len > 0 {
                    Self::part(
                        batch,
                        atlas,
                        "gui/sprites/container/brewing_stand/brew_progress",
                        (0.0, 0.0),
                        (9.0, len as f32),
                        (ox + 97.0, oy + 16.0),
                    );
                }
                let bubble = [29, 24, 20, 16, 11, 6, 0][(brew / 2 % 7) as usize] as f32;
                if bubble > 0.0 {
                    Self::part(
                        batch,
                        atlas,
                        "gui/sprites/container/brewing_stand/bubbles",
                        (0.0, 29.0 - bubble),
                        (12.0, bubble),
                        (ox + 63.0, oy + 14.0 + 29.0 - bubble),
                    );
                }
            }
            let default = self.translate("container.brewing", "Brewing Stand");
            let title = self.remote_menu_title().unwrap_or_else(|| {
                let named = self
                    .station
                    .as_ref()
                    .and_then(|s| self.world.block_names.get(&(s.pos[0], s.pos[1], s.pos[2])));
                Text::literal(named.cloned().unwrap_or(default)).resolve_legacy()
            });
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ox + (176.0 - tw) / 2.0,
                oy + 6.0,
                GREY,
                false,
            );
            batch.text(atlas, &inventory_label, ox + 8.0, oy + 72.0, GREY, false);
            // Empty-slot icons.
            for (i, icon) in [
                (300, "potion"),
                (301, "potion"),
                (302, "potion"),
                (304, "brewing_fuel"),
            ] {
                if self.slot_mut(i).is_none() {
                    let (sx, sy) = self.station_slot_position(i).unwrap_or_default();
                    batch.sprite(
                        &self.gui.atlas,
                        &format!("gui/sprites/container/slot/{icon}"),
                        ox + sx,
                        oy + sy,
                    );
                }
            }
        } else if let Some(kind) = self.station.as_ref().map(|s| s.kind) {
            // A server's menu has the title it was given.
            let title = self
                .remote_menu_title()
                .unwrap_or_else(|| Text::literal(self.translate(kind.title_key(), "")));
            {
                let atlas = &self.gui.atlas;
                Self::part(
                    batch,
                    atlas,
                    kind.texture(),
                    (0.0, 0.0),
                    (176.0, 166.0),
                    (ox, oy),
                );
                let (tx, ty) = kind.title_pos();
                batch.text_styled(atlas, &title, ox + tx, oy + ty, GREY, false);
                batch.text(atlas, &inventory_label, ox + 8.0, oy + 72.0, GREY, false);
            }
            if kind == Kind::Cartography {
                self.draw_cartography(batch, ox, oy);
            }
            if kind == Kind::Loom {
                tooltip = self.draw_loom(batch, ox, oy, local);
            }
            let atlas = &self.gui.atlas;
            let s = self.station.as_ref().expect("open station");
            let has_input = s.inputs.iter().any(Option::is_some);
            match kind {
                Kind::Enchanting => {
                    if s.inputs[1].is_none() {
                        batch.sprite(
                            atlas,
                            "gui/sprites/container/slot/lapis_lazuli",
                            ox + 35.0,
                            oy + 47.0,
                        );
                    }
                    let mut names = LegacyRandom::new(i64::from(self.level.xp_seed));
                    let gold = s.inputs[1].as_ref().map_or(0, |l| l.count as i32);
                    let level = self.experience.level;
                    let offers = s.offers.clone();
                    for i in 0..3 {
                        let (bx, by, bw, bh) = enchant_button(i);
                        let cost = offers.costs[i];
                        if cost == 0 {
                            batch.sprite_scaled(
                                atlas,
                                "gui/sprites/container/enchanting_table/enchantment_slot_disabled",
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            continue;
                        }
                        let cost_text = cost.to_string();
                        let text_width = 86.0 - HudBatch::text_width(atlas, &cost_text);
                        let count = names.next_int_bound(2).unwrap_or(0) + 3;
                        let words: Vec<&str> = (0..count)
                            .map(|_| {
                                WORDS
                                    [names.next_int_bound(WORDS.len() as i32).unwrap_or(0) as usize]
                            })
                            .collect();
                        let message = words.join(" ");
                        let disabled = (gold < i as i32 + 1 || level < cost) && !self.creative;
                        let col: i32 = if disabled {
                            batch.sprite_scaled(
                                atlas,
                                "gui/sprites/container/enchanting_table/enchantment_slot_disabled",
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            batch.sprite(
                                atlas,
                                &format!(
                                    "gui/sprites/container/enchanting_table/level_{}_disabled",
                                    i + 1
                                ),
                                ox + bx + 1.0,
                                oy + by + 1.0,
                            );
                            let c = ((-9937334i32) & 16711422) >> 1;
                            let msg_color = argb(c | 0xFF00_0000u32 as i32);
                            self.galactic_deferred(
                                batch,
                                &message,
                                ox + bx + 20.0,
                                oy + by + 2.0,
                                text_width,
                                msg_color,
                            );
                            -12550384
                        } else {
                            let hover = inside(local, bx, by, bw, bh);
                            let sprite = if hover {
                                "enchantment_slot_highlighted"
                            } else {
                                "enchantment_slot"
                            };
                            batch.sprite_scaled(
                                atlas,
                                &format!("gui/sprites/container/enchanting_table/{sprite}"),
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            batch.sprite(
                                atlas,
                                &format!("gui/sprites/container/enchanting_table/level_{}", i + 1),
                                ox + bx + 1.0,
                                oy + by + 1.0,
                            );
                            let msg_color = if hover { -128 } else { -9937334 };
                            self.galactic_deferred(
                                batch,
                                &message,
                                ox + bx + 20.0,
                                oy + by + 2.0,
                                text_width,
                                argb(msg_color),
                            );
                            -8323296
                        };
                        let atlas = &self.gui.atlas;
                        let cw = HudBatch::text_width(atlas, &cost_text);
                        batch.text(
                            atlas,
                            &cost_text,
                            ox + bx + 20.0 + 86.0 - cw,
                            oy + by + 9.0,
                            argb(col),
                            true,
                        );
                        // Tooltip with the clue (EnchantmentScreen.extractRenderState).
                        if inside(local, bx, by, bw, 17.0)
                            && let Some((id, l)) = &offers.clues[i]
                        {
                            let e = &self.res.stations().enchantments;
                            let (name, _) = e.full_name(id, *l);
                            let clue = self
                                .translate("container.enchant.clue", "%s . . . ?")
                                .replace("%s", &name);
                            let mut lines =
                                vec![(horizon_client::text::Text::literal(clue), u32::MAX)];
                            if !self.creative {
                                lines.push((horizon_client::text::Text::literal(""), u32::MAX));
                                if level < cost {
                                    lines.push((
                                        horizon_client::text::Text::literal(
                                            self.translate(
                                                "container.enchant.level.requirement",
                                                "Level Requirement: %s",
                                            )
                                            .replace("%s", &cost.to_string()),
                                        ),
                                        rgba(255, 85, 85, 255),
                                    ));
                                } else {
                                    let n = i + 1;
                                    let lapis = if n == 1 {
                                        self.translate(
                                            "container.enchant.lapis.one",
                                            "1 Lapis Lazuli",
                                        )
                                    } else {
                                        self.translate(
                                            "container.enchant.lapis.many",
                                            "%s Lapis Lazuli",
                                        )
                                        .replace("%s", &n.to_string())
                                    };
                                    lines.push((
                                        horizon_client::text::Text::literal(lapis),
                                        if gold >= n as i32 {
                                            rgba(170, 170, 170, 255)
                                        } else {
                                            rgba(255, 85, 85, 255)
                                        },
                                    ));
                                    let levels = if n == 1 {
                                        self.translate(
                                            "container.enchant.level.one",
                                            "1 Enchantment Level",
                                        )
                                    } else {
                                        self.translate(
                                            "container.enchant.level.many",
                                            "%s Enchantment Levels",
                                        )
                                        .replace("%s", &n.to_string())
                                    };
                                    lines.push((
                                        horizon_client::text::Text::literal(levels),
                                        rgba(170, 170, 170, 255),
                                    ));
                                }
                            }
                            tooltip = Some(lines);
                        }
                    }
                }
                Kind::Anvil => {
                    let field = if s.inputs[0].is_some() {
                        "text_field"
                    } else {
                        "text_field_disabled"
                    };
                    batch.sprite_scaled(
                        atlas,
                        &format!("gui/sprites/container/anvil/{field}"),
                        ox + 59.0,
                        oy + 20.0,
                        110.0,
                        16.0,
                        u32::MAX,
                    );
                    if has_input && s.result.is_none() {
                        batch.sprite(
                            atlas,
                            "gui/sprites/container/anvil/error",
                            ox + 99.0,
                            oy + 45.0,
                        );
                    }
                    if s.inputs[0].is_some() {
                        // Drawn once this borrow of the station ends.
                        anvil_name = true;
                    }
                    let cost = s.anvil.cost;
                    if cost > 0 {
                        let mut color = argb(-8323296);
                        let line = if cost >= 40 && !self.creative {
                            color = argb(-40864);
                            Some(self.translate("container.repair.expensive", "Too Expensive!"))
                        } else if s.result.is_none() {
                            None
                        } else {
                            if !self.may_take_result() {
                                color = argb(-40864);
                            }
                            Some(
                                self.translate("container.repair.cost", "Enchantment Cost: %1$s")
                                    .replace("%1$s", &cost.to_string()),
                            )
                        };
                        if let Some(line) = line {
                            let atlas = &self.gui.atlas;
                            let tw = HudBatch::text_width(atlas, &line);
                            let tx = 176.0 - 8.0 - tw - 2.0;
                            batch.fill(
                                atlas,
                                ox + tx - 2.0,
                                oy + 67.0,
                                176.0 - 8.0 - (tx - 2.0),
                                12.0,
                                argb(1325400064),
                            );
                            batch.text(atlas, &line, ox + tx, oy + 69.0, color, true);
                        }
                    }
                }
                Kind::Grindstone => {
                    if has_input && s.result.is_none() {
                        batch.sprite(
                            atlas,
                            "gui/sprites/container/grindstone/error",
                            ox + 92.0,
                            oy + 31.0,
                        );
                    }
                }
                Kind::Smithing => {
                    let icons = ["smithing_template_netherite_upgrade", "", ""];
                    for (i, icon) in icons.iter().enumerate() {
                        if s.inputs[i].is_none() && !icon.is_empty() {
                            let (sx, sy) = kind.input_pos(i);
                            batch.sprite(
                                atlas,
                                &format!("gui/sprites/container/slot/{icon}"),
                                ox + sx,
                                oy + sy,
                            );
                        }
                    }
                    if s.inputs.iter().all(Option::is_some) && s.result.is_none() {
                        batch.sprite(
                            atlas,
                            "gui/sprites/container/smithing/error",
                            ox + 65.0,
                            oy + 46.0,
                        );
                        if inside(local, 65.0, 46.0, 28.0, 21.0) {
                            tooltip = Some(vec![(
                                horizon_client::text::Text::literal(self.translate(
                                    "container.upgrade.error_tooltip",
                                    "Item can't be upgraded this way",
                                )),
                                u32::MAX,
                            )]);
                        }
                    }
                }
                Kind::Merchant | Kind::Beacon | Kind::Cartography | Kind::Loom => {}
                Kind::Stonecutter => {
                    let display = s.inputs[0].is_some() && !s.stone.recipes.is_empty();
                    let active = display && s.stone.recipes.len() > 12;
                    let rows = s.stone.recipes.len().div_ceil(4).saturating_sub(3).max(1);
                    let offs = if active {
                        (s.start / 4) as f32 / rows as f32
                    } else {
                        0.0
                    };
                    let scroller = if active {
                        "scroller"
                    } else {
                        "scroller_disabled"
                    };
                    batch.sprite(
                        atlas,
                        &format!("gui/sprites/container/stonecutter/{scroller}"),
                        ox + 119.0,
                        oy + 15.0 + (41.0 * offs).floor(),
                    );
                    if display {
                        let (start, selected) = (s.start, s.selected);
                        // graphics.item: the icon only, without the count.
                        let shown: Vec<(usize, ItemStack)> = (start
                            ..(start + 12).min(s.stone.recipes.len()))
                            .map(|i| {
                                (
                                    i,
                                    ItemStack {
                                        count: 1,
                                        ..self.res.stations().recipes.stonecutting
                                            [s.stone.recipes[i]]
                                            .result
                                            .clone()
                                    },
                                )
                            })
                            .collect();
                        for (index, result) in shown {
                            let p = index - start;
                            let (x, y) = (
                                52.0 + (p % 4) as f32 * 16.0,
                                14.0 + (p / 4) as f32 * 18.0 + 2.0,
                            );
                            let hover = inside(local, x, y, 16.0, 18.0);
                            let sprite = if Some(index) == selected {
                                "recipe_selected"
                            } else if hover {
                                "recipe_highlighted"
                            } else {
                                "recipe"
                            };
                            batch.sprite_scaled(
                                &self.gui.atlas,
                                &format!("gui/sprites/container/stonecutter/{sprite}"),
                                ox + x,
                                oy + y - 1.0,
                                16.0,
                                18.0,
                                u32::MAX,
                            );
                            self.draw_item(batch, &result, ox + x, oy + y);
                            if hover {
                                tooltip_stack = Some(result);
                            }
                        }
                    }
                }
            }
        }
        if anvil_name {
            self.draw_anvil_name(batch, ox, oy);
        }
        let drag = self.drag_view();
        for i in self.station_slots().unwrap_or_default() {
            let Some((sx, sy)) = self.slot_position_any(i) else {
                continue;
            };
            let stack = self.slot_mut(i).clone();
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(
                &self.gui.atlas,
                ox + sx,
                oy + sy,
                16.0,
                16.0,
                rgba(255, 255, 255, 128),
            );
            if self.carried.is_none()
                && tooltip.is_none()
                && tooltip_stack.is_none()
                && let Some(stack) = self.slot_mut(slot).clone()
            {
                tooltip_stack = Some(stack);
            }
        }
        if self.carried.is_some() {
            self.draw_carried(batch);
        } else if let Some(stack) = tooltip_stack {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        } else if let Some(lines) = tooltip {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
        true
    }
    fn galactic_deferred(
        &self,
        batch: &mut HudBatch,
        text: &str,
        x: f32,
        y: f32,
        max: f32,
        color: u32,
    ) {
        self.galactic(batch, text, x, y, max, color);
    }
    fn slot_position_any(&self, i: usize) -> Option<(f32, f32)> {
        self.station_slot_position(i)
            .or_else(|| (i < 36).then(|| self.slot_position(i)))
    }

    // ---------------------------------------------------------------- tooltips

    /// `ItemStack.getTooltipLines` subset: name in rarity colour, lore,
    /// potion effects, enchantments (gray, curses red), stored enchantments, trims.
    pub(super) fn stack_tooltip(
        &self,
        stack: &ItemStack,
    ) -> Vec<(horizon_client::text::Text, u32)> {
        self.stack_tooltip_lines(stack, false)
    }
    /// `ItemStack.getRarity`'s colour for the name line and a `[name]` link.
    fn rarity_color(&self, stack: &ItemStack) -> u32 {
        let props = self.res.stations().enchantments.props(&stack.item);
        let rarity = horizon_client::item_tooltip::rarity(
            stack,
            props.map_or("common", |p| p.rarity.as_str()),
        );
        match rarity {
            "uncommon" => rgba(255, 255, 85, 255),
            "rare" => rgba(85, 255, 255, 255),
            "epic" => rgba(255, 85, 255, 255),
            _ => u32::MAX,
        }
    }
    /// `ItemStack.getHoverName`: the custom name, a written book's title, the
    /// item name component, else the item's or potion's name.
    fn stack_hover_name(&self, stack: &ItemStack) -> String {
        if let Some(n) = stack.custom_name() {
            n.to_owned()
        } else if let Some(title) = stack
            .data()
            .and_then(|d| d.written.as_ref())
            .map(|w| w.title.as_str())
            .filter(|t| !t.trim().is_empty())
        {
            title.to_owned()
        } else if let Some(n) = stack.item_name() {
            n
        } else if stack.data().is_some_and(|d| d.ominous_banner) {
            self.translate("block.minecraft.ominous_banner", "Ominous Banner")
        } else if stack.data().and_then(|d| d.potion.as_ref()).is_some()
            || stack.item.ends_with("potion")
            || &*stack.item == "minecraft:tipped_arrow"
        {
            let key = brewing::potion_name_key(&stack.item, stack.potion());
            self.pack
                .names
                .display_get(&key)
                .unwrap_or_else(|| self.pack.names.display_item(&stack.item))
        } else {
            self.pack.names.display_item(&stack.item)
        }
    }
    /// `ItemStack.getTooltipLines`. A `nested` stack is a crossbow's
    /// projectile: `ChargedProjectiles.addProjectileTooltip` takes its details
    /// without the name, the player and the advanced flag.
    fn stack_tooltip_lines(
        &self,
        stack: &ItemStack,
        nested: bool,
    ) -> Vec<(horizon_client::text::Text, u32)> {
        let e: &Enchantments = &self.res.stations().enchantments;
        let color = self.rarity_color(stack);
        let advanced = !nested && self.options_advanced_tooltips();
        let display = horizon_client::item_tooltip::TooltipDisplay::for_stack(stack);
        let mut lines = Vec::new();
        if !nested && display.name_visible(self.screen == Screen::Creative) {
            // `getStyledHoverName`: a custom name is italic.
            let name = self.stack_hover_name(stack);
            let name = if stack.custom_name().is_some() {
                horizon_client::text::Text::literal(name).italic()
            } else {
                horizon_client::text::Text::legacy(&name)
            };
            lines.push((name, color));
        }
        let translate = |k: &str| self.pack.names.display_get(k);
        let push = |lines: &mut Vec<(horizon_client::text::Text, u32)>, s: String, c: u32| {
            lines.push((horizon_client::text::Text::legacy(&s), c));
        };
        let gray = rgba(170, 170, 170, 255);
        let item_info = self.res.items().get(&stack.item);
        // SmithingTemplateItem.appendHoverText: `Item.appendHoverText` comes
        // before the components.
        for (text, c) in horizon_client::item_tooltip::smithing_template_lines(stack, &translate) {
            push(&mut lines, text, c);
        }
        // InstrumentComponent.addToTooltip.
        if let Some(text) =
            horizon_client::item_tooltip::instrument_line(stack, &display, &translate)
        {
            push(&mut lines, text, gray);
        }
        // Bees.addToTooltip.
        if let Some(text) = horizon_client::item_tooltip::bees_line(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        // ItemContainerContents.addToTooltip.
        let hover_name = |s: &ItemStack| {
            s.custom_name()
                .map(str::to_owned)
                .or_else(|| s.item_name())
                .unwrap_or_else(|| self.pack.names.display_item(&s.item))
        };
        if display.shows("minecraft:container") {
            for text in
                horizon_client::item_tooltip::container_lines(stack, &hover_name, &translate)
            {
                push(&mut lines, text, u32::MAX);
            }
        }
        // BannerPatternLayers.addToTooltip: up to six layer names.
        if display.shows("minecraft:banner_patterns") {
            for layer in horizon_client::banner::tooltip_layers(stack).iter().take(6) {
                let key = self.res.stations().banners.layer_key(layer);
                push(
                    &mut lines,
                    self.translate(&key, &key),
                    rgba(170, 170, 170, 255),
                );
            }
        }
        // WrittenBookContent.addToTooltip.
        for text in horizon_client::item_tooltip::written_book_lines(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        // ChargedProjectiles.addToTooltip: each run of equal projectiles, then
        // the projectile's own details indented under it.
        if display.shows("minecraft:charged_projectiles") {
            let charged = stack.data().map_or(&[][..], |d| &d.charged[..]);
            for run in charged.chunk_by(|a, b| a.same_item_same_components(b)) {
                let projectile = &run[0];
                let name = self.stack_hover_name(projectile);
                let line = if run.len() == 1 {
                    // `getDisplayName`: `[name]` in the rarity colour.
                    let mut shown = horizon_client::text::Text::literal(format!("[{name}]"))
                        .with_color(rgb_of(self.rarity_color(projectile)));
                    if projectile.custom_name().is_some() {
                        shown = shown.italic();
                    }
                    horizon_client::text::format_translation(
                        &self.translate(
                            "item.minecraft.crossbow.projectile.single",
                            "Projectile: %s",
                        ),
                        &[shown],
                    )
                } else {
                    horizon_client::text::format_translation(
                        &self.translate(
                            "item.minecraft.crossbow.projectile.multiple",
                            "Projectile: %s x %s",
                        ),
                        &[
                            horizon_client::text::Text::literal(run.len().to_string()),
                            horizon_client::text::Text::literal(name),
                        ],
                    )
                };
                lines.push((line, u32::MAX));
                for (text, c) in self.stack_tooltip_lines(projectile, true) {
                    // `literal("  ").append(line).withStyle(GRAY)`: the line's
                    // own colours win over the gray.
                    let rgb = if c == u32::MAX {
                        horizon_client::text::GRAY
                    } else {
                        rgb_of(c)
                    };
                    let indented = horizon_client::text::Text::literal("  ")
                        .append(text)
                        .with_color(rgb);
                    lines.push((indented, u32::MAX));
                }
            }
        }
        // Fireworks / FireworkExplosion.addToTooltip.
        for text in horizon_client::item_tooltip::firework_lines(stack, &translate) {
            push(&mut lines, text, rgba(170, 170, 170, 255));
        }
        if display.shows("minecraft:potion_contents")
            && (stack.item.ends_with("potion") || &*stack.item == "minecraft:tipped_arrow")
        {
            let scale = match &*stack.item {
                "minecraft:lingering_potion" => 0.25,
                "minecraft:tipped_arrow" => 0.125,
                _ => 1.0,
            };
            for (text, cat) in brewing::tooltip(stack.potion(), scale, &|k| {
                e.translate(k).map(str::to_owned)
            }) {
                let c = match cat {
                    brewing::Category::Beneficial => rgba(85, 85, 255, 255),
                    brewing::Category::Harmful => rgba(255, 85, 85, 255),
                    brewing::Category::Neutral => rgba(170, 170, 170, 255),
                };
                push(&mut lines, text, c);
            }
            // The attribute modifiers of the effects (`When Applied:`).
            for (text, c) in horizon_client::item_tooltip::potion_modifier_lines(stack, &translate)
            {
                push(&mut lines, text, c);
            }
        }
        // JukeboxPlayable.addToTooltip.
        if let Some(text) =
            horizon_client::item_tooltip::jukebox_line(stack, item_info, &display, &translate)
        {
            push(&mut lines, text, gray);
        }
        if display.shows("minecraft:trim")
            && let Some((material, pattern)) = stack.data().and_then(|d| d.trim.clone())
        {
            push(
                &mut lines,
                e.translate("item.minecraft.smithing_template.upgrade")
                    .unwrap_or("Upgrade:")
                    .to_owned(),
                rgba(170, 170, 170, 255),
            );
            let short = |s: &str| s.trim_start_matches("minecraft:").to_owned();
            let p = e
                .translate(&format!("trim_pattern.minecraft.{}", short(&pattern)))
                .map_or_else(|| short(&pattern), str::to_owned);
            let m = e
                .translate(&format!("trim_material.minecraft.{}", short(&material)))
                .map_or_else(|| short(&material), str::to_owned);
            push(&mut lines, format!(" {p}"), rgba(170, 170, 170, 255));
            push(&mut lines, format!(" {m}"), rgba(170, 170, 170, 255));
        }
        if display.shows("minecraft:stored_enchantments") {
            for (text, curse) in e.tooltip(stack.stored_enchantments()) {
                push(
                    &mut lines,
                    text,
                    if curse {
                        rgba(255, 85, 85, 255)
                    } else {
                        rgba(170, 170, 170, 255)
                    },
                );
            }
        }
        if display.shows("minecraft:enchantments") {
            for (text, curse) in e.tooltip(stack.enchantments()) {
                push(
                    &mut lines,
                    text,
                    if curse {
                        rgba(255, 85, 85, 255)
                    } else {
                        rgba(170, 170, 170, 255)
                    },
                );
            }
        }
        // DyedItemColor.addToTooltip.
        if let Some((text, italic)) =
            horizon_client::item_tooltip::dyed_line(stack, &display, advanced, &translate)
        {
            let text = horizon_client::text::Text::legacy(&text);
            lines.push((if italic { text.italic() } else { text }, gray));
        }
        // ItemLore.addToTooltip: after the enchantments, before the attributes
        // (ItemStack.addDetailsToTooltip).
        if display.shows("minecraft:lore") {
            for text in horizon_client::item_tooltip::lore_texts(stack) {
                lines.push((text, u32::MAX));
            }
        }
        if display.shows("minecraft:attribute_modifiers") {
            // The item's own modifiers, then its enchantments' for each slot
            // group (`ItemStack.forEachModifier`).
            let mut modifiers = item_info.map(|i| i.modifiers.clone()).unwrap_or_default();
            modifiers.extend(e.modifier_rows(stack));
            let base = |name: &str, default| {
                self.player
                    .as_ref()
                    .and_then(|player| {
                        let key = player.attributes.key(name)?;
                        let handle = player.attributes.lookup(Some(&key))?;
                        player
                            .attributes
                            .instance(&handle)
                            .ok()
                            .map(|value| value.base_value())
                    })
                    .unwrap_or(default)
            };
            let player_base = (!nested).then(|| {
                [
                    base("minecraft:attack_damage", 1.0),
                    base(
                        "minecraft:attack_speed",
                        horizon_client::attack_strength::BASE_ATTACK_SPEED,
                    ),
                ]
            });
            for (text, c) in
                horizon_client::item_tooltip::attribute_lines(&modifiers, player_base, &translate)
            {
                push(&mut lines, text, c);
            }
        }
        // ItemStack.UNBREAKABLE_TOOLTIP.
        if let Some(text) =
            horizon_client::item_tooltip::unbreakable_line(stack, &display, &translate)
        {
            push(&mut lines, text, rgba(85, 85, 255, 255));
        }
        // BlockItemStateProperties.addToTooltip.
        if let Some(text) = horizon_client::item_tooltip::honey_line(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        // Spawner.appendHoverText: what a spawner or trial spawner item spawns.
        if display.shows("minecraft:block_entity_data")
            && matches!(
                &*stack.item,
                "minecraft:spawner" | "minecraft:trial_spawner"
            )
        {
            let gray = rgba(170, 170, 170, 255);
            match block_entity_item::spawner_entity(stack) {
                Some(id) => {
                    let key = format!("entity.{}", id.replacen(':', ".", 1));
                    push(&mut lines, self.translate(&key, &id), gray);
                }
                None => {
                    push(&mut lines, String::new(), u32::MAX);
                    let desc1 =
                        self.translate("block.minecraft.spawner.desc1", "Interact with Spawn Egg:");
                    push(&mut lines, desc1, gray);
                    let desc2 = self.translate("block.minecraft.spawner.desc2", "Sets Mob Type");
                    push(&mut lines, format!(" {desc2}"), rgba(85, 85, 255, 255));
                }
            }
        }
        if display.shows("minecraft:damage")
            && let Some(info) = item_info
            && info.max_damage > 0
            && stack.damage > 0
            && advanced
        {
            // `item.durability`, in the selected language.
            lines.push((
                horizon_client::text::Text::translate(
                    "item.durability",
                    &[
                        horizon_client::text::Text::literal(
                            (info.max_damage - stack.damage).to_string(),
                        ),
                        horizon_client::text::Text::literal(info.max_damage.to_string()),
                    ],
                ),
                u32::MAX,
            ));
        }
        // ItemStack.addDetailsToTooltipTail: with advanced tooltips the item id
        // and how many components the stack has.
        if advanced {
            let dark_gray = rgba(85, 85, 85, 255);
            push(&mut lines, stack.item.to_string(), dark_gray);
            let count = horizon_client::item_tooltip::component_count(stack, item_info);
            if count > 0 {
                let text = self.translate("item.components", "%s component(s)");
                push(
                    &mut lines,
                    text.replacen("%s", &count.to_string(), 1),
                    dark_gray,
                );
            }
        }
        // ItemStack.OP_NBT_WARNING: `BlockItem` and `SpawnEggItem`
        // `.shouldPrintOpWarning` for a player with game master permission;
        // a nested projectile has no player. The first line is bold.
        if !nested
            && self.ui.registries.permission >= 2
            && (item_info.is_some_and(|i| i.block.is_some())
                && block_entity_item::op_warning(stack)
                || block_entity_item::spawn_egg_op_warning(stack))
        {
            for (i, (key, fallback)) in [
                ("item.op_warning.line1", "Warning:"),
                (
                    "item.op_warning.line2",
                    "Use of this item might lead to command execution.",
                ),
                (
                    "item.op_warning.line3",
                    "Do not use unless you know the exact contents!",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let text = horizon_client::text::Text::legacy(&self.translate(key, fallback));
                lines.push((
                    if i == 0 { text.bold() } else { text },
                    rgba(255, 85, 85, 255),
                ));
            }
        }
        lines
    }
    /// Menu state for the control socket.
    pub(super) fn station_info(&self) -> serde_json::Value {
        let Some(s) = self.station.as_ref() else {
            return match self.container_kind() {
                Some(BlockEntity::Brewing(b)) => {
                    serde_json::json!({ "brew_time": b.brew_time, "fuel": b.fuel })
                }
                _ => serde_json::Value::Null,
            };
        };
        let recipes = &self.res.stations().recipes.stonecutting;
        serde_json::json!({
            "costs": s.offers.costs,
            "clues": s.offers.clues.iter().map(|c| c.as_ref().map(|(e, l)| format!("{e} {l}"))).collect::<Vec<_>>(),
            "bookcases": s.bookcases,
            "anvil_cost": s.anvil.cost,
            "result": s.result.as_ref().map(|r| format!("{} x{}", r.item, r.count)),
            "recipes": s.stone.recipes.iter().map(|&i| recipes[i].id.clone()).collect::<Vec<_>>(),
            "selected": s.selected,
        })
    }
    /// Options.advancedItemTooltips (F3+H).
    fn options_advanced_tooltips(&self) -> bool {
        self.settings.file.bool("advancedItemTooltips")
    }
    /// `TooltipRenderUtil`: dark box with the purple border, first line spaced.
    pub(super) fn draw_tooltip_lines<T: Clone + Into<horizon_client::text::Text>>(
        &self,
        batch: &mut HudBatch,
        lines: &[(T, u32)],
        x: f32,
        y: f32,
    ) {
        if lines.is_empty() {
            return;
        }
        let lines = Self::resolved(lines);
        let atlas = &self.gui.atlas;
        let (x, y, w, h) =
            Self::tooltip_geometry(atlas, &lines, (x, y), (batch.width, batch.height), 0.0);
        Self::tooltip_box(batch, atlas, x, y, w, h);
        Self::tooltip_text(batch, atlas, &lines, x, y);
    }
    /// A tooltip's lines with their legacy codes resolved and their colours.
    fn resolved<T: Clone + Into<horizon_client::text::Text>>(
        lines: &[(T, u32)],
    ) -> Vec<(horizon_client::text::Text, u32)> {
        lines
            .iter()
            .map(|(text, color)| {
                let text: horizon_client::text::Text = text.clone().into();
                (text.resolve_legacy(), *color)
            })
            .collect()
    }
    /// The tooltip's text's top-left corner and the box's size: beside the
    /// cursor, kept on screen, `extra` pixels taller than the text.
    fn tooltip_geometry(
        atlas: &horizon_client::hud::HudAtlas,
        lines: &[(horizon_client::text::Text, u32)],
        (x, y): (f32, f32),
        (width, height): (f32, f32),
        extra: f32,
    ) -> (f32, f32, f32, f32) {
        let w = lines
            .iter()
            .map(|(t, _)| atlas.font.width_text(t))
            .fold(0.0, f32::max)
            .max(if extra > 0.0 { 96.0 } else { 0.0 });
        let text_h = if lines.len() == 1 {
            8.0
        } else {
            8.0 + 2.0 + (lines.len() - 1) as f32 * 10.0
        };
        let h = text_h + extra;
        let mut x = x + 12.0;
        let mut y = y - 12.0;
        if x + w > width - 4.0 {
            x = (x - 28.0 - w).max(4.0);
        }
        if y + h + 3.0 > height {
            y = height - h - 3.0;
        }
        y = y.max(4.0);
        (x, y, w, h)
    }
    /// The box and its frame sprites.
    fn tooltip_box(
        batch: &mut HudBatch,
        atlas: &horizon_client::hud::HudAtlas,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        batch.fill(atlas, x - 3.0, y - 4.0, w + 6.0, 1.0, rgba(16, 0, 16, 240));
        batch.fill(
            atlas,
            x - 3.0,
            y + h + 3.0,
            w + 6.0,
            1.0,
            rgba(16, 0, 16, 240),
        );
        batch.fill(
            atlas,
            x - 4.0,
            y - 3.0,
            w + 8.0,
            h + 6.0,
            rgba(16, 0, 16, 240),
        );
        batch.fill(atlas, x - 3.0, y - 3.0, w + 6.0, 1.0, rgba(80, 0, 255, 80));
        batch.fill(
            atlas,
            x - 3.0,
            y + h + 2.0,
            w + 6.0,
            1.0,
            rgba(40, 0, 127, 80),
        );
        batch.fill(atlas, x - 3.0, y - 2.0, 1.0, h + 4.0, rgba(60, 0, 190, 80));
        batch.fill(
            atlas,
            x + w + 2.0,
            y - 2.0,
            1.0,
            h + 4.0,
            rgba(60, 0, 190, 80),
        );
    }
    fn tooltip_text(
        batch: &mut HudBatch,
        atlas: &horizon_client::hud::HudAtlas,
        lines: &[(horizon_client::text::Text, u32)],
        x: f32,
        y: f32,
    ) {
        let mut ty = y;
        for (i, (text, color)) in lines.iter().enumerate() {
            batch.text_styled(atlas, text, x, ty, *color, true);
            ty += if i == 0 { 12.0 } else { 10.0 };
        }
    }
    /// A stack's tooltip: its lines, and under them a bundle's contents
    /// (`ClientBundleTooltip`: the 4 wide grid, the `+N` of what it leaves
    /// out, and the weight bar).
    pub(super) fn draw_stack_tooltip(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
    ) {
        let lines = Self::resolved(&self.stack_tooltip(stack));
        if lines.is_empty() {
            return;
        }
        // The component's height does not depend on where it is drawn, so a
        // probe at the origin gives the box its extra height.
        let probe = horizon_client::bundle::tooltip_layout(self.res.items(), stack, 0.0, 0.0, 96.0);
        let extra = probe.as_ref().map_or(0.0, |l| l.height + 2.0);
        let atlas = &self.gui.atlas;
        let (x, y, w, h) =
            Self::tooltip_geometry(atlas, &lines, (x, y), (batch.width, batch.height), extra);
        Self::tooltip_box(batch, atlas, x, y, w, h);
        let text_h = h - extra;
        Self::tooltip_text(batch, atlas, &lines, x, y);
        let Some(layout) =
            horizon_client::bundle::tooltip_layout(self.res.items(), stack, x, y + text_h + 2.0, w)
        else {
            return;
        };
        self.draw_bundle_tooltip(batch, &layout);
    }
    /// `ClientBundleTooltip.extractImage`: the slots, the count past them and
    /// the progress bar.
    fn draw_bundle_tooltip(
        &mut self,
        batch: &mut HudBatch,
        layout: &horizon_client::bundle::TooltipLayout,
    ) {
        for (i, (at, stack)) in layout.slots.iter().enumerate() {
            let selected = layout.selected == Some(i);
            let sprite = |name: &str| format!("gui/sprites/container/bundle/{name}");
            // Each draw takes the atlas on its own: the item's needs `&mut self`.
            horizon_client::ui::nine_slice(
                batch,
                &self.gui.atlas,
                &sprite(if selected {
                    "slot_highlight_back"
                } else {
                    "slot_background"
                }),
                at[0],
                at[1],
                24.0,
                24.0,
                u32::MAX,
            );
            let stack = stack.clone();
            let [x, y] = *at;
            self.draw_item(batch, &stack, x + 4.0, y + 4.0);
            if selected {
                horizon_client::ui::nine_slice(
                    batch,
                    &self.gui.atlas,
                    &sprite("slot_highlight_front"),
                    x,
                    y,
                    24.0,
                    24.0,
                    u32::MAX,
                );
            }
        }
        let atlas = &self.gui.atlas;
        if let Some((at, hidden)) = layout.overflow {
            let label = horizon_client::text::Text::literal(format!("+{hidden}"));
            let w = atlas.font.width_text(&label);
            batch.text_styled(
                atlas,
                &label,
                at[0] + 12.0 - (w / 2.0).floor(),
                at[1] + 10.0,
                u32::MAX,
                true,
            );
        }
        if let Some((at, description)) = layout
            .empty_description
            .zip(Some("item.minecraft.bundle.empty.description"))
        {
            let text = horizon_client::text::Text::translate(description, &[]);
            let mut ty = at[1];
            for line in atlas.font.split_lines(&text, layout.width) {
                batch.text_styled(atlas, &line.0, at[0], ty, 0xFFAA_AAAA, false);
                ty += 9.0;
            }
        }
        let [bx, by] = layout.bar;
        let fill_sprite = if layout.full {
            "gui/sprites/container/bundle/bundle_progressbar_full"
        } else {
            "gui/sprites/container/bundle/bundle_progressbar_fill"
        };
        horizon_client::ui::nine_slice(
            batch,
            atlas,
            fill_sprite,
            bx + 1.0,
            by,
            layout.fill,
            13.0,
            u32::MAX,
        );
        horizon_client::ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/container/bundle/bundle_progressbar_border",
            bx,
            by,
            96.0,
            13.0,
            u32::MAX,
        );
        if let Some(key) = layout.label {
            let label = horizon_client::text::Text::translate(key, &[]);
            let w = atlas.font.width_text(&label);
            batch.text_styled(
                atlas,
                &label,
                bx + 48.0 - (w / 2.0).floor(),
                by + 3.0,
                u32::MAX,
                true,
            );
        }
    }
    /// Hovered-slot tooltip on the plain inventory, crafting and container screens.
    pub(super) fn draw_slot_tooltip(&mut self, batch: &mut HudBatch) {
        if self.carried.is_some() {
            return;
        }
        let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let stack = self.slot_view(slot);
        if let Some(stack) = stack {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        } else if let Some(text) = self.crafter_tooltip(slot) {
            self.draw_tooltip_lines(
                batch,
                &[(horizon_client::text::Text::literal(text), u32::MAX)],
                self.cursor.0,
                self.cursor.1,
            );
        }
    }

    // ---------------------------------------------------------------- brewing stand

    /// `BrewingStandBlockEntity.serverTick` of the brewing stand at `pos`, in
    /// its place among the level's block entity tickers.
    pub(super) fn tick_brewing_stand(
        &mut self,
        pos: (i32, i32, i32),
    ) -> horizon_client::Result<()> {
        let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get_mut(&pos) else {
            return Ok(());
        };
        let out = b.tick(
            &self.res.stations().brewing,
            self.res.items(),
            self.res.providers(),
        );
        if out != brewing::BrewTick::default() {
            let (x, y, z) = pos;
            self.world.modified.insert((x >> 4, z >> 4));
            if out.brewed {
                self.play_at("minecraft:block.brewing_stand.brew", [x, y, z], 1.0, 1.0);
            }
            for d in out.drops {
                self.pop_resource([x, y, z], d);
            }
            if let Some(bottles) = out.bottles {
                let state = self.world.block(x, y, z);
                if let Some(mut s) = self.pack.assets.registry.state(state).cloned()
                    && AsRef::<str>::as_ref(&s.name) == "minecraft:brewing_stand"
                {
                    for (i, has) in bottles.iter().enumerate() {
                        s.properties
                            .insert(format!("has_bottle_{i}"), has.to_string());
                    }
                    if let Ok(id) = self.pack.assets.registry.resolve(&s) {
                        self.world.set(x, y, z, id)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// The anvil's rename box (`AnvilScreen`: `setMaxLength(50)`).
const ANVIL_NAME_CHARS: usize = 50;

fn anvil_box() -> horizon_client::ui::EditBox {
    let mut edit = horizon_client::ui::EditBox::new(0.0, 0.0, 103.0, 12.0);
    edit.bordered = false;
    edit.max_length = ANVIL_NAME_CHARS;
    edit.text_color = u32::MAX;
    edit.focused = true;
    edit
}
