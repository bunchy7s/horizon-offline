//! Container menus for guests (AbstractContainerMenu.doClick): the player's
//! inventory menu (window 0) and block containers (chests, barrels, hoppers,
//! dispensers, shulker boxes). The click modes are pickup, quick move, swap,
//! clone, throw, quick craft (drag) and pickup all; items thrown out of a
//! menu are returned for the game to drop.
use super::guest::Guest;
use crate::{
    bundle::{Contents, can_be_in_bundle, toggle_selected},
    containers::BlockEntity,
    items::{ItemStack, Items},
    world::World,
};

/// What the menus need to know about items and the slots' rules.
pub trait Rules {
    fn items(&self) -> &Items;
    fn max_stack(&self, item: &str) -> u32;
    /// The armor slot (0 head .. 3 feet) an item can be worn in.
    fn armor_index(&self, item: &str) -> Option<usize>;
    /// `ItemStack.getEquipmentSlot`: the `minecraft:equippable` component's
    /// slot ("head", "chest", "legs", "feet", "mainhand", "offhand").
    fn equip_slot(&self, item: &str) -> Option<&str> {
        self.items().get(item)?.equip_slot.as_deref()
    }
    /// `Slot.mayPickup`: whether the stack in menu slot `slot` may be taken
    /// out. A block container's slots and the player's own allow it; a mod's
    /// menu can refuse.
    fn may_pickup(&self, _slot: usize, _stack: &ItemStack) -> bool {
        true
    }
    /// `Player.canDropItems`: a client that drops items faster than its
    /// throttle allows stops sending THROW clicks (`LocalPlayer.canDropItems`
    /// is its `dropSpamThrottler`'s answer).
    fn can_drop_items(&self) -> bool {
        true
    }
}

/// An open block container.
pub struct Menu {
    pub window: i32,
    /// The `minecraft:menu` registry id.
    pub kind: i32,
    pub pos: Option<(i32, i32, i32)>,
    /// Where the container's slots live.
    pub slots: Slots,
    /// Slots of the container itself (without the 36 inventory slots).
    pub size: usize,
    pub state: i32,
    /// What the client was last told, for change detection.
    pub sent: Vec<Option<ItemStack>>,
    /// An unfinished drag: (mode, slots).
    pub drag: Option<(u8, Vec<usize>)>,
    /// The data slots the client was last sent (ContainerSetData), a furnace's
    /// progress; None until sent, so any first value goes out.
    pub sent_data: Vec<Option<i16>>,
    /// Container slots the menu does not have: a mod's screen hides them, so
    /// no click or quick move reaches them (`Slot` positions are the screen's).
    pub hidden: Vec<usize>,
    /// A content mod's menu type: its data slot count. Its clicks run on the
    /// vanilla menu engine (`menu::Kind::Mod`), not here.
    pub mod_type: Option<&'static crate::containers::ModMenuType>,
}

/// What a guest did with a native mod's menu (`Slots::Native`), for the game
/// to tell the mod once the hub is back in its hands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMenuEvent {
    /// A click (`ServerboundContainerClickPacket`): the menu's slot (its own
    /// first, then the player's 36; -999 outside), the button and the mode.
    Click {
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    },
    /// The menu closed: by the player, by the server, another opened, or
    /// the player left.
    Closed { uuid: u128, window: i32 },
}

/// The y of the key a chest minecart's or chest boat's items are kept under
/// while a menu shows them.
pub const VEHICLE_KEY_Y: i32 = -100_000;

/// Where the slots of a container menu live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    /// One block entity.
    Block((i32, i32, i32)),
    /// A double chest (`CompoundContainer`): the first half's slots, then
    /// the second's.
    Double((i32, i32, i32), (i32, i32, i32)),
    /// The player's own ender chest inventory (`PlayerEnderChestContainer`).
    Ender,
    /// A native mod's menu (`Guest::native_menu`): items to look at, which no
    /// click moves; the clicks go to the mod.
    Native,
}

impl Slots {
    fn halves(self) -> Vec<(i32, i32, i32)> {
        match self {
            Slots::Block(k) => vec![k],
            Slots::Double(a, b) => vec![a, b],
            Slots::Ender | Slots::Native => Vec::new(),
        }
    }
    /// Whether the block entities behind the slots are still there
    /// (`Container.stillValid`).
    pub fn valid(self, world: &World) -> bool {
        self.halves()
            .iter()
            .all(|k| world.block_entities.contains_key(k))
    }
    /// `Container.stillValid` for the guest: the block entities are there
    /// and the guest is within reach of each (`stillValidBlockEntity`). The
    /// ender chest's block is `at` (`PlayerEnderChestContainer.activeChest`).
    pub fn still_valid(self, world: &World, guest: &Guest, at: Option<(i32, i32, i32)>) -> bool {
        let blocks = match self {
            Slots::Ender => at.into_iter().collect(),
            Slots::Native => Vec::new(),
            _ => self.halves(),
        };
        // A vehicle's container is kept under a key, not a block: the game
        // checks its reach to the vehicle (`isChestVehicleStillValid`).
        self.valid(world)
            && blocks
                .iter()
                .all(|k| k.1 == VEHICLE_KEY_Y || guest.within_block_range([k.0, k.1, k.2], 4.0))
    }
    /// Container slot `n`.
    pub fn get(self, world: &World, guest: &Guest, mut n: usize) -> Option<ItemStack> {
        if self == Slots::Ender {
            return guest.ender_items.get(n).cloned().flatten();
        }
        if self == Slots::Native {
            return guest.native_menu.get(n).cloned().flatten();
        }
        for k in self.halves() {
            let slots = shown(world, k)?;
            if n < slots.len() {
                return slots[n].clone();
            }
            n -= slots.len();
        }
        None
    }
    /// Every container slot, in menu order.
    pub fn read(self, world: &World, guest: &Guest) -> Option<Vec<Option<ItemStack>>> {
        if self == Slots::Ender {
            return Some(guest.ender_items.clone());
        }
        if self == Slots::Native {
            return Some(guest.native_menu.clone());
        }
        let mut items = Vec::new();
        for k in self.halves() {
            items.extend_from_slice(shown(world, k)?);
        }
        Some(items)
    }
    /// Store what `read` gave, changed.
    pub fn write(self, world: &mut World, guest: &mut Guest, items: Vec<Option<ItemStack>>) {
        if self == Slots::Ender {
            guest.ender_items = items;
            return;
        }
        if self == Slots::Native {
            // Only the mod changes what it shows.
            return;
        }
        let mut rest = items.into_iter();
        for k in self.halves() {
            let n = shown(world, k).map_or(0, <[_]>::len);
            if let Some(e) = world.block_entities.get_mut(&k) {
                for slot in e.slots_mut().iter_mut().take(n) {
                    *slot = rest.next().flatten();
                }
            }
        }
    }
}

/// The slots of the block entity at `k` its menu shows: a mod container's
/// count, though items past it stay saved.
fn shown(world: &World, k: (i32, i32, i32)) -> Option<&[Option<ItemStack>]> {
    let e = world.block_entities.get(&k)?;
    let slots = e.slots();
    if !matches!(e, BlockEntity::Mod(_)) {
        return Some(slots);
    }
    let state = world.block(k.0, k.1, k.2).0;
    let count = world
        .block_entities
        .mod_container(state)
        .map_or(0, |c| c.slots);
    Some(&slots[..count.min(slots.len())])
}

/// Slots of a menu seen as one list.
pub struct View<'a> {
    pub guest: &'a mut Guest,
    /// The container's slots, or None for the inventory menu.
    pub container: Option<&'a mut [Option<ItemStack>]>,
    /// The container is a shulker box (`ShulkerBoxSlot`).
    pub shulker: bool,
    /// Container slots that do not exist in this menu (see [`Menu::hidden`]).
    pub hidden: &'a [usize],
}

impl View<'_> {
    pub fn len(&self) -> usize {
        match &self.container {
            Some(c) => c.len() + 36,
            None => 46,
        }
    }
    pub fn is_empty(&self) -> bool {
        false
    }
    pub fn get(&self, i: usize) -> Option<ItemStack> {
        match &self.container {
            Some(c) if i < c.len() => c[i].clone(),
            Some(c) => self
                .guest
                .inventory
                .get(player_slot(i - c.len()))
                .cloned()
                .flatten(),
            None => self.guest.menu_slot(i),
        }
    }
    pub fn set(&mut self, i: usize, item: Option<ItemStack>) {
        match &mut self.container {
            Some(c) if i < c.len() => c[i] = item,
            Some(c) => {
                let n = c.len();
                self.guest.inventory[player_slot(i - n)] = item;
            }
            None => self.guest.set_menu_slot(i, item),
        }
    }
    /// Whether the slot takes part in clicks at all.
    fn enabled(&self, i: usize) -> bool {
        (self.container.is_some() || i >= 5) && !self.hidden.contains(&i)
    }
    fn may_place(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> bool {
        if self.container.is_none() && (5..=8).contains(&i) {
            return rules.armor_index(&stack.item) == Some(i - 5);
        }
        if self.shulker
            && self.container.as_ref().is_some_and(|c| i < c.len())
            && !crate::menu::fits_inside_container_items(stack)
        {
            return false;
        }
        self.enabled(i)
    }
    fn slot_max(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> u32 {
        if self.container.is_none() && (5..=8).contains(&i) {
            return 1;
        }
        rules.max_stack(&stack.item).max(1)
    }
    /// `Slot.mayPickup`: whether menu slot `i` gives its stack up. The
    /// armour slots refuse while a survival player wears a binding curse
    /// (`ArmorSlot.mayPickup`: `PREVENT_ARMOR_CHANGE` and no creative); the
    /// menu's rules answer for their own slots.
    fn may_pickup(&self, i: usize, rules: &dyn Rules) -> bool {
        let stack = self.get(i);
        if self.container.is_none()
            && (5..=8).contains(&i)
            && !self.guest.creative()
            && stack
                .as_ref()
                .is_some_and(|s| s.enchantment_level("minecraft:binding_curse") > 0)
        {
            return false;
        }
        stack.is_none_or(|s| rules.may_pickup(i, &s))
    }
    /// The ranges quick move uses: (container start..end, main, hotbar).
    fn player_ranges(&self) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        match &self.container {
            Some(c) => (c.len()..c.len() + 27, c.len() + 27..c.len() + 36),
            None => (9..36, 36..45),
        }
    }
}

/// Inventory index of the n-th player slot of a container menu (27 main, then hotbar).
fn player_slot(n: usize) -> usize {
    if n < 27 { 9 + n } else { n - 27 }
}

/// Inventory.add-like merge of `stack` into slots `range` (moveItemStackTo).
fn move_to(
    view: &mut View,
    mut stack: ItemStack,
    range: std::ops::Range<usize>,
    reverse: bool,
    rules: &dyn Rules,
) -> Option<ItemStack> {
    let order: Vec<usize> = if reverse {
        range.rev().collect()
    } else {
        range.collect()
    };
    for &i in &order {
        if stack.count == 0 {
            break;
        }
        if !view.enabled(i) {
            continue;
        }
        if let Some(mut existing) = view.get(i)
            && existing.same_item_same_components(&stack)
        {
            let max = view.slot_max(i, &existing, rules);
            if existing.count < max {
                let moved = (max - existing.count).min(stack.count);
                existing.count += moved;
                stack.count -= moved;
                view.set(i, Some(existing));
            }
        }
    }
    for &i in &order {
        if stack.count == 0 {
            break;
        }
        if view.enabled(i) && view.get(i).is_none() && view.may_place(i, &stack, rules) {
            let moved = stack.count.min(view.slot_max(i, &stack, rules));
            view.set(
                i,
                Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                }),
            );
            stack.count -= moved;
        }
    }
    (stack.count > 0).then_some(stack)
}

fn quick_move(view: &mut View, i: usize, rules: &dyn Rules) {
    let Some(stack) = view.get(i) else {
        return;
    };
    if !view.enabled(i) {
        return;
    }
    let (main, hotbar) = view.player_ranges();
    let rest = if let Some(c) = &view.container {
        let n = c.len();
        if i < n {
            // Container to inventory, filling from the hotbar's end.
            move_to(view, stack, n..n + 36, true, rules)
        } else {
            move_to(view, stack, 0..n, false, rules)
        }
    } else if (5..=8).contains(&i) || i < 5 {
        move_to(view, stack, 9..45, false, rules)
    } else {
        // Wear it, otherwise the stack fills the inventory: from the main
        // slots for the hotbar and the off hand, from the hotbar for the
        // main ones. The off hand is in this arm too, so armour worn there
        // is put on like any other (InventoryMenu.quickMoveStack asks for
        // the item's equipment slot before it looks at the clicked index).
        let armor = rules
            .armor_index(&stack.item)
            .filter(|a| view.get(5 + a).is_none());
        if let Some(a) = armor {
            view.set(
                5 + a,
                Some(ItemStack {
                    count: 1,
                    ..stack.clone()
                }),
            );
            let mut rest = stack.clone();
            rest.count -= 1;
            (rest.count > 0).then_some(rest)
        } else if rules.equip_slot(&stack.item) == Some("offhand") && view.get(45).is_none() {
            // A shield or a totem goes to the empty off hand before the
            // hotbar (InventoryMenu.quickMoveStack's OFFHAND branch).
            let max = view.slot_max(45, &stack, rules);
            let moved = stack.count.min(max);
            view.set(
                45,
                Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                }),
            );
            let rest = stack.count - moved;
            (rest > 0).then_some(ItemStack {
                count: rest,
                ..stack.clone()
            })
        } else if hotbar.contains(&i) {
            move_to(view, stack, main, false, rules)
        } else if main.contains(&i) {
            move_to(view, stack, hotbar, false, rules)
        } else {
            move_to(view, stack, 9..45, false, rules)
        }
    };
    view.set(i, rest);
}

/// The click modes of ServerboundContainerClickPacket.
pub mod input {
    pub const PICKUP: i32 = 0;
    pub const QUICK_MOVE: i32 = 1;
    pub const SWAP: i32 = 2;
    pub const CLONE: i32 = 3;
    pub const THROW: i32 = 4;
    pub const QUICK_CRAFT: i32 = 5;
    pub const PICKUP_ALL: i32 = 6;
}

/// `AbstractContainerMenu.isContainerClickButton`: a pickup and a quick move
/// are clicks with the left or the right button only.
fn is_container_click_button(button: i32) -> bool {
    button == 0 || button == 1
}

/// Apply one click; returns the stacks thrown out and the slots changed.
pub fn click(
    view: &mut View,
    drag: &mut Option<(u8, Vec<usize>)>,
    slot: i32,
    button: i32,
    mode: i32,
    rules: &dyn Rules,
) -> (Vec<ItemStack>, Vec<usize>) {
    let mut drops = Vec::new();
    let before: Vec<Option<ItemStack>> = (0..view.len()).map(|i| view.get(i)).collect();
    if mode == input::QUICK_CRAFT {
        quick_craft(view, drag, slot, button, rules);
    } else {
        *drag = None;
        match mode {
            // isContainerClickButton: PICKUP and QUICK_MOVE take buttons 0
            // and 1; any other button is no click at all, so neither the
            // slot nor the cursor moves (a modified client sends those).
            input::PICKUP | input::QUICK_MOVE if !is_container_click_button(button) => {}
            input::PICKUP | input::QUICK_MOVE if slot == -999 => {
                if mode == input::PICKUP
                    && let Some(mut carried) = view.guest.carried.take()
                {
                    if button == 1 && carried.count > 1 {
                        let mut one = carried.clone();
                        one.count = 1;
                        carried.count -= 1;
                        view.guest.carried = Some(carried);
                        drops.push(one);
                    } else {
                        drops.push(carried);
                    }
                }
            }
            input::PICKUP => pickup(view, slot, button, rules),
            input::QUICK_MOVE => {
                // doClick calls quickMoveStack again while the clicked slot
                // still holds the item it moved, because moveItemStackTo fills
                // one empty slot per call. move_to here fills every empty slot
                // of its range in one pass and leaves the same remainder, so
                // one call is vanilla's loop: a full inventory keeps its stack,
                // and a stack too big for one slot spreads over several.
                if let Ok(i) = usize::try_from(slot)
                    && i < view.len()
                    && view.may_pickup(i, rules)
                {
                    quick_move(view, i, rules);
                }
            }
            input::SWAP => swap(view, slot, button, rules),
            input::CLONE => {
                if view.guest.carried.is_none()
                    && view.guest.creative()
                    && let Ok(i) = usize::try_from(slot)
                    && let Some(stack) = view.get(i)
                {
                    let max = rules.max_stack(&stack.item).max(1);
                    view.guest.carried = Some(ItemStack {
                        count: max,
                        ..stack
                    });
                }
            }
            input::THROW => {
                if rules.can_drop_items()
                    && view.guest.carried.is_none()
                    && let Ok(i) = usize::try_from(slot)
                    && view.enabled(i)
                    && let Some(mut stack) = view.get(i)
                {
                    // doClick's THROW takes one item for button 0 and the
                    // whole stack for any other button.
                    let amount = if button == 0 { 1 } else { stack.count };
                    if amount >= stack.count {
                        drops.push(stack);
                        view.set(i, None);
                    } else {
                        let mut one = stack.clone();
                        one.count = amount;
                        stack.count -= amount;
                        view.set(i, Some(stack));
                        drops.push(one);
                    }
                }
            }
            input::PICKUP_ALL => pickup_all(view, slot, button, rules),
            _ => {}
        }
    }
    let changed = (0..view.len())
        .filter(|&i| view.get(i) != before[i])
        .collect();
    (drops, changed)
}

fn pickup(view: &mut View, slot: i32, button: i32, rules: &dyn Rules) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() || !view.enabled(i) {
        return;
    }
    let right = button == 1;
    if bundle_click(view, i, right, rules) {
        return;
    }
    let (slot_item, carried) = (view.get(i), view.guest.carried.clone());
    match (slot_item, carried) {
        (None, None) => {}
        (None, Some(mut c)) => {
            if view.may_place(i, &c, rules) {
                let amount = if right { 1 } else { c.count }.min(view.slot_max(i, &c, rules));
                view.set(
                    i,
                    Some(ItemStack {
                        count: amount,
                        ..c.clone()
                    }),
                );
                c.count -= amount;
                view.guest.carried = (c.count > 0).then_some(c);
            }
        }
        (Some(mut s), None) => {
            // doClick's PICKUP arm takes from the slot only while
            // `Slot.mayPickup` allows it.
            if !view.may_pickup(i, rules) {
                return;
            }
            let take = if right { s.count.div_ceil(2) } else { s.count };
            let taken = ItemStack {
                count: take,
                ..s.clone()
            };
            s.count -= take;
            view.set(i, (s.count > 0).then_some(s));
            view.guest.carried = Some(taken);
        }
        (Some(mut s), Some(mut c)) => {
            // Merging into, swapping with and topping the hand up from the
            // slot are all takes (`allowModification`: mayPickup and
            // mayPlace).
            if !view.may_pickup(i, rules) {
                return;
            }
            if !view.may_place(i, &c, rules) {
                // A slot that takes nothing gives onto the same carried item,
                // as far as the hand holds (`tryRemove`).
                if s.same_item_same_components(&c) {
                    let max = rules.max_stack(&c.item).max(1);
                    let moved = s.count.min(max.saturating_sub(c.count));
                    s.count -= moved;
                    c.count += moved;
                    view.set(i, (s.count > 0).then_some(s));
                    view.guest.carried = Some(c);
                }
                return;
            }
            if s.same_item_same_components(&c) {
                let max = view.slot_max(i, &s, rules);
                let moved = if right { 1 } else { c.count }.min(max.saturating_sub(s.count));
                s.count += moved;
                c.count -= moved;
                view.set(i, Some(s));
                view.guest.carried = (c.count > 0).then_some(c);
            } else if c.count <= view.slot_max(i, &c, rules) {
                view.set(i, Some(c));
                view.guest.carried = Some(s);
            }
        }
    }
}

/// `tryItemClickBehaviourOverride`: the carried stack's
/// `BundleItem.overrideStackedOnOther`, then the clicked one's
/// `overrideOtherStackedOnMe`. True when a bundle took the click.
fn bundle_click(view: &mut View, i: usize, right: bool, rules: &dyn Rules) -> bool {
    let items = rules.items();
    let other = view.get(i);
    if let Some(mut carried) = view.guest.carried.clone()
        && let Some(mut contents) = Contents::of(items, &carried)
        && other.is_some() != right
    {
        match other {
            // tryTransfer: as much as fits, through `safeTake`.
            Some(mut other) => {
                if can_be_in_bundle(&other) {
                    let amount = other.count.min(contents.room_for(items, &other));
                    if amount > 0 {
                        let mut taken = other.clone();
                        taken.count = amount;
                        other.count -= amount;
                        view.set(i, (other.count > 0).then_some(other));
                        contents.try_insert(items, &mut taken);
                    }
                }
            }
            // removeOne into the empty slot (`safeInsert`); the rest goes back.
            None => {
                if let Some(mut stack) = contents.remove_one(items) {
                    if view.may_place(i, &stack, rules) {
                        let amount = stack.count.min(view.slot_max(i, &stack, rules));
                        view.set(
                            i,
                            Some(ItemStack {
                                count: amount,
                                ..stack.clone()
                            }),
                        );
                        stack.count -= amount;
                    }
                    if stack.count > 0 {
                        contents.try_insert(items, &mut stack);
                    }
                }
            }
        }
        contents.store(&mut carried);
        view.guest.carried = Some(carried);
        return true;
    }
    let Some(mut bundle) = other else {
        return false;
    };
    let Some(mut contents) = Contents::of(items, &bundle) else {
        return false;
    };
    // Slot.allowModification: mayPickup and mayPlace of its item.
    let allowed = view.may_place(i, &bundle, rules);
    match (view.guest.carried.take(), right) {
        (Some(mut carried), false) => {
            if allowed {
                contents.try_insert(items, &mut carried);
            }
            view.guest.carried = (carried.count > 0).then_some(carried);
        }
        (None, true) => {
            if allowed {
                view.guest.carried = contents.remove_one(items);
            }
        }
        (carried, _) => {
            // toggleSelectedItem(-1), then the click goes on.
            view.guest.carried = carried;
            toggle_selected(&mut bundle, -1);
            view.set(i, Some(bundle));
            return false;
        }
    }
    contents.store(&mut bundle);
    view.set(i, Some(bundle));
    true
}

fn swap(view: &mut View, slot: i32, button: i32, rules: &dyn Rules) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() || !view.enabled(i) {
        return;
    }
    // Hotbar slot `button` (0..9) or the off hand (40). A chest has no
    // off-hand slot, so F swaps with `Guest.offhand` (Inventory slot 40).
    if button == 40 && view.container.is_some() {
        let (clicked, off) = (view.get(i), view.guest.offhand.clone());
        if off
            .as_ref()
            .is_some_and(|off| !view.may_place(i, off, rules))
        {
            return;
        }
        view.set(i, off);
        view.guest.offhand = clicked;
        return;
    }
    let other = match button {
        0..=8 => match &view.container {
            Some(c) => c.len() + 27 + button as usize,
            None => 36 + button as usize,
        },
        40 => 45,
        _ => return,
    };
    if other == i {
        return;
    }
    let (a, b) = (view.get(i), view.get(other));
    if a.as_ref().is_some_and(|a| !view.may_place(other, a, rules))
        || b.as_ref().is_some_and(|b| !view.may_place(i, b, rules))
    {
        return;
    }
    view.set(i, b);
    view.set(other, a);
}

fn pickup_all(view: &mut View, slot: i32, button: i32, rules: &dyn Rules) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() {
        return;
    }
    let Some(mut carried) = view.guest.carried.clone() else {
        return;
    };
    // doClick returns when the clicked slot holds a stack it may pick up, so
    // a double-click collects from an empty slot or one that refuses.
    if view.get(i).is_some() && view.may_pickup(i, rules) {
        return;
    }
    let max = rules.max_stack(&carried.item).max(1);
    // Button 0 walks the slots from the first, button 1 from the last.
    let order: Vec<usize> = if button == 0 {
        (0..view.len()).collect()
    } else {
        (0..view.len()).rev().collect()
    };
    // Partial stacks first, then full ones.
    for full in [false, true] {
        for &i in &order {
            if carried.count >= max {
                break;
            }
            if !view.enabled(i) || (view.container.is_none() && i == 0) {
                continue;
            }
            // Each swept slot must give its stack up too.
            if !view.may_pickup(i, rules) {
                continue;
            }
            let Some(mut s) = view.get(i) else {
                continue;
            };
            if !s.same_item_same_components(&carried) || (s.count == max) != full {
                continue;
            }
            let moved = (max - carried.count).min(s.count);
            carried.count += moved;
            s.count -= moved;
            view.set(i, (s.count > 0).then_some(s));
        }
    }
    view.guest.carried = Some(carried);
}

/// Dragging a stack over slots (QUICK_CRAFT): start, add slots, end.
fn quick_craft(
    view: &mut View,
    drag: &mut Option<(u8, Vec<usize>)>,
    slot: i32,
    button: i32,
    rules: &dyn Rules,
) {
    let (stage, kind) = (button & 3, ((button >> 2) & 3) as u8);
    match stage {
        0 => {
            // `getQuickcraftType` and `isValidQuickcraftType`: a charitable
            // (0) or greedy (1) drag is anyone's, a clone drag (2) needs
            // infinite materials, and 3 is no type at all. A start that is
            // not legal, or that arrives while a drag is open, leaves the
            // fields reset instead of collecting slots.
            let usable = match kind {
                0 | 1 => true,
                2 => view.guest.creative(),
                _ => false,
            };
            *drag = (usable && drag.is_none() && view.guest.carried.is_some())
                .then_some((kind, Vec::new()));
        }
        1 => {
            if let (Some((_, slots)), Ok(i)) = (drag.as_mut(), usize::try_from(slot))
                && i < view.len()
                && !slots.contains(&i)
                && let Some(c) = &view.guest.carried
                && view.enabled(i)
                && view.may_place(i, c, rules)
                && view.get(i).is_none_or(|s| s.same_item_same_components(c))
            {
                slots.push(i);
            }
        }
        _ => {
            let Some((kind, slots)) = drag.take() else {
                return;
            };
            let Some(mut carried) = view.guest.carried.clone() else {
                return;
            };
            if slots.is_empty() {
                return;
            }
            if slots.len() == 1 {
                // A single slot behaves like a plain click.
                let button = if kind == 1 { 1 } else { 0 };
                pickup(view, slots[0] as i32, button, rules);
                return;
            }
            let each = match kind {
                0 => carried.count / slots.len() as u32,
                1 => 1,
                _ => rules.max_stack(&carried.item).max(1),
            };
            for i in slots {
                if carried.count == 0 {
                    break;
                }
                let existing = view.get(i).map_or(0, |s| s.count);
                let max = view.slot_max(i, &carried, rules);
                let put = each.min(max.saturating_sub(existing)).min(carried.count);
                if put == 0 {
                    continue;
                }
                view.set(
                    i,
                    Some(ItemStack {
                        count: existing + put,
                        ..carried.clone()
                    }),
                );
                if kind != 2 || !view.guest.creative() {
                    carried.count -= put;
                }
            }
            view.guest.carried = (carried.count > 0).then_some(carried);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::{Joined, PlayerSender};
    use horizon_protocol::packets::configuration::ClientInformation;
    use horizon_world::loot::PackOverlay;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    struct ItemRules(Items);
    impl Rules for ItemRules {
        fn items(&self) -> &Items {
            &self.0
        }
        fn max_stack(&self, item: &str) -> u32 {
            self.0.get(item).map_or(64, |i| i.max_stack)
        }
        fn armor_index(&self, _: &str) -> Option<usize> {
            None
        }
    }

    fn guest() -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "Alex".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
            },
            2,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    /// The prepared resources' items, or None (the test is skipped) without them.
    fn rules() -> Option<ItemRules> {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("items.json").exists() {
            eprintln!("skipping resource-backed menu check: {}", root.display());
            return None;
        }
        let registry = horizon_world::level::data::load_registry(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
        Some(ItemRules(
            Items::load(&root, &registry, &facts, &PackOverlay::NONE).unwrap(),
        ))
    }

    fn contents(stack: &Option<ItemStack>) -> Vec<(String, u32)> {
        let inside = stack.as_ref().and_then(|s| s.data());
        inside.map_or(Vec::new(), |d| {
            d.bundle_contents
                .iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect()
        })
    }

    /// A double-click's collect walks the slots in the direction the button
    /// picks: `doClick`'s PICKUP_ALL arm starts at slot 0 with a step of +1 for
    /// button 0, and at the last slot with -1 for button 1, which is the
    /// button the screen sends for a double-click (`mouseReleased`).
    #[test]
    fn pickup_all_walks_from_the_buttons_side() {
        let rules = SwapRules;
        let mut g = guest();
        let mut chest = vec![None; 5];
        chest[0] = Some(ItemStack::new("minecraft:stone", 5));
        chest[1] = Some(ItemStack::new("minecraft:stone", 5));
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        // `doClick` only starts the collect when the carried stack is not
        // empty and the clicked slot has no item or may not be taken
        // (`!carried.isEmpty() && (!slot.hasItem() || !slot.mayPickup(player))`),
        // so the click lands on an empty slot and the sweep picks the side.
        // One stone short of a full stack: the right double-click takes it
        // from the last matching slot.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 2, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        // The left one takes it from the first.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 2, 0, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
    }

    /// A guest's chest clicks run `BundleItem`'s overrides: a carried
    /// bundle takes a slot's stack and drops its first one into an empty
    /// slot; a stack clicked onto a bundle goes in, as far as it fits.
    #[test]
    fn guest_chest_clicks_fill_and_empty_bundles() {
        let Some(rules) = rules() else {
            return;
        };
        let mut g = guest();
        let mut chest = vec![
            Some(ItemStack::new("minecraft:stone", 48)),
            None,
            Some(ItemStack::new("minecraft:bundle", 1)),
        ];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        view.guest.carried = Some(ItemStack::new("minecraft:bundle", 1));
        // Left click with the bundle: the stone goes in.
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0), None);
        assert_eq!(
            contents(&view.guest.carried),
            [("minecraft:stone".into(), 48)]
        );
        // Right click on an empty slot: the stone comes back out.
        click(&mut view, &mut drag, 1, 1, input::PICKUP, &rules);
        assert_eq!(view.get(1).map(|s| s.count), Some(48));
        assert_eq!(contents(&view.guest.carried), []);
        // Ender pearls (1/16 each) onto the bundle in the chest: 16 fit.
        view.guest.carried = Some(ItemStack::new("minecraft:ender_pearl", 16));
        click(&mut view, &mut drag, 2, 0, input::PICKUP, &rules);
        assert_eq!(view.guest.carried, None);
        let bundle = view.get(2);
        assert_eq!(contents(&bundle), [("minecraft:ender_pearl".into(), 16)]);
        // Full: more stone stays in hand.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 5));
        click(&mut view, &mut drag, 2, 0, input::PICKUP, &rules);
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(5));
        // Right click on it with an empty hand takes the pearls out.
        view.guest.carried = None;
        click(&mut view, &mut drag, 2, 1, input::PICKUP, &rules);
        assert_eq!(
            view.guest.carried.as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:ender_pearl", 16))
        );
        assert_eq!(contents(&view.get(2)), []);
    }

    /// A mod container saved with more items than its menu has slots: the
    /// menu reads and writes only its count, the rest stay saved.
    #[test]
    fn mod_menu_shows_only_its_container_slots() {
        use crate::containers::{ModBlockEntityFields, ModContainer};
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let container = ModContainer {
            slots: 5,
            ..Default::default()
        };
        world
            .block_entities
            .set_mod_containers([(0, Arc::new(container))].into_iter().collect());
        let mut items = vec![None; 9];
        items[1] = Some(ItemStack::new("minecraft:stone", 3));
        items[7] = Some(ItemStack::new("minecraft:iron_ore", 2));
        let k = (0, 64, 0);
        world.block_entities.insert(
            k,
            BlockEntity::Mod(Box::new(
                ModBlockEntityFields {
                    id: "gtmod:machine".into(),
                    items,
                    ..Default::default()
                }
                .into(),
            )),
        );
        let mut g = guest();
        let slots = Slots::Block(k);
        let mut read = slots.read(&world, &g).unwrap();
        assert_eq!(read.len(), 5);
        assert_eq!(slots.get(&world, &g, 7), None);
        // A shift-click from the hotbar (menu slot 5 + 27) lands in the
        // container's first empty slot, not past its count.
        if let Some(rules) = rules() {
            g.inventory[0] = Some(ItemStack::new("minecraft:dirt", 1));
            let mut drag = None;
            let mut view = View {
                guest: &mut g,
                container: Some(&mut read),
                shulker: false,
                hidden: &[],
            };
            click(&mut view, &mut drag, 5 + 27, 0, input::QUICK_MOVE, &rules);
            assert_eq!(view.get(0).map(|s| s.item), Some("minecraft:dirt".into()));
        } else {
            read[0] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        slots.write(&mut world, &mut g, read);
        let Some(BlockEntity::Mod(m)) = world.block_entities.get(&k) else {
            panic!("not a mod's");
        };
        assert_eq!(m.items.len(), 9);
        assert_eq!(
            m.items[0].as_ref().map(|s| &*s.item),
            Some("minecraft:dirt")
        );
        assert_eq!(m.items[1].as_ref().map(|s| s.count), Some(3));
        assert_eq!(m.items[7].as_ref().map(|s| s.count), Some(2));
    }

    /// A slot a mod's screen hides is not in the menu: a shift-click from the
    /// inventory skips it, and a shift-click on it moves nothing.
    #[test]
    fn a_hidden_slot_takes_no_quick_move() {
        let Some(rules) = rules() else {
            return;
        };
        let mut g = guest();
        let mut container = vec![None; 3];
        container[2] = Some(ItemStack::new("minecraft:stone", 1));
        g.inventory[0] = Some(ItemStack::new("minecraft:dirt", 1));
        let hidden = [0, 2];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut container),
            shulker: false,
            hidden: &hidden,
        };
        // The hotbar's first slot (menu slot 3 + 27) lands in the one slot shown.
        click(&mut view, &mut drag, 3 + 27, 0, input::QUICK_MOVE, &rules);
        assert!(view.get(0).is_none());
        assert_eq!(view.get(1).map(|s| s.item), Some("minecraft:dirt".into()));
        // Nothing leaves a hidden slot either.
        click(&mut view, &mut drag, 2, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(2).map(|s| s.item), Some("minecraft:stone".into()));
    }

    /// `AbstractContainerMenu.isValidQuickcraftType`: quick-craft type 2 (a
    /// clone drag) is for players with infinite materials only, and type 3 is
    /// no type at all. A survival guest that sends either moves nothing; a
    /// creative guest's clone drag fills the slots and keeps its stack.
    #[test]
    fn a_clone_drag_needs_infinite_materials() {
        let rules = SwapRules;
        let mut g = guest();
        let mut chest = vec![None; 3];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 64));
        // Start, two slots, end, for the two types a survival guest may not
        // use: 3 is no type, 2 is a clone drag.
        for kind in [3, 2] {
            click(
                &mut view,
                &mut drag,
                0,
                kind << 2,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                0,
                kind << 2 | 1,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                1,
                kind << 2 | 1,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                -999,
                kind << 2 | 2,
                input::QUICK_CRAFT,
                &rules,
            );
            assert_eq!(view.get(0), None, "type {kind}");
            assert_eq!(view.get(1), None, "type {kind}");
            assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        }
        // A charitable drag over the same two slots still shares the stack.
        click(&mut view, &mut drag, 0, 0, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, 0, 1, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, 1, 1, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, -999, 2, input::QUICK_CRAFT, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(32));
        assert_eq!(view.get(1).map(|s| s.count), Some(32));
        assert_eq!(view.guest.carried, None);
        // With infinite materials a clone drag goes through: both slots take
        // the whole stack and the cursor keeps it.
        view.guest.game_mode = crate::lan::guest::CREATIVE;
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 64));
        click(&mut view, &mut drag, 0, 2 << 2, input::QUICK_CRAFT, &rules);
        click(
            &mut view,
            &mut drag,
            0,
            2 << 2 | 1,
            input::QUICK_CRAFT,
            &rules,
        );
        click(
            &mut view,
            &mut drag,
            2,
            2 << 2 | 1,
            input::QUICK_CRAFT,
            &rules,
        );
        click(
            &mut view,
            &mut drag,
            -999,
            2 << 2 | 2,
            input::QUICK_CRAFT,
            &rules,
        );
        assert_eq!(view.get(0).map(|s| s.count), Some(64));
        assert_eq!(view.get(2).map(|s| s.count), Some(64));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
    }

    struct SwapRules;
    impl Rules for SwapRules {
        fn items(&self) -> &Items {
            unreachable!("swap does not read the item catalog");
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn armor_index(&self, _: &str) -> Option<usize> {
            None
        }
    }

    /// F (SWAP button 40) over a chest or the inventory below it swaps with
    /// the off hand, even though the menu has no off-hand slot.
    #[test]
    fn guest_chest_f_swaps_with_the_off_hand() {
        let rules = SwapRules;
        let mut g = guest();
        g.offhand = Some(ItemStack::new("minecraft:shield", 1));
        let mut chest = vec![Some(ItemStack::new("minecraft:diamond", 1)), None];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        click(&mut view, &mut drag, 0, 40, input::SWAP, &rules);
        assert_eq!(
            view.get(0).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| &*s.item),
            Some("minecraft:diamond")
        );
        // Empty chest slot: the off hand goes in.
        click(&mut view, &mut drag, 1, 40, input::SWAP, &rules);
        assert_eq!(
            view.get(1).as_ref().map(|s| &*s.item),
            Some("minecraft:diamond")
        );
        assert!(view.guest.offhand.is_none());
        // F over the first hotbar slot (menu slot 2 + 27).
        view.guest.offhand = Some(ItemStack::new("minecraft:oak_boat", 1));
        view.guest.inventory[0] = Some(ItemStack::new("minecraft:apple", 1));
        click(&mut view, &mut drag, 2 + 27, 40, input::SWAP, &rules);
        assert_eq!(
            view.guest.inventory[0].as_ref().map(|s| &*s.item),
            Some("minecraft:oak_boat")
        );
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| &*s.item),
            Some("minecraft:apple")
        );
    }

    #[test]
    fn guest_shulker_box_menu_refuses_shulker_boxes() {
        let Some(rules) = rules() else {
            return;
        };
        for shulker in [true, false] {
            let mut g = guest();
            let mut container = vec![None; 27];
            let mut drag = None;
            let mut view = View {
                guest: &mut g,
                container: Some(&mut container),
                shulker,
                hidden: &[],
            };
            // Carried onto an empty slot, shift-clicked from the hotbar
            // (menu slot 27 + 27) and swapped in with the number key 1.
            view.guest.carried = Some(ItemStack::new("minecraft:red_shulker_box", 1));
            click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
            assert_eq!(view.get(0).is_none(), shulker);
            view.guest.carried = None;
            view.guest.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
            click(&mut view, &mut drag, 54, 0, input::QUICK_MOVE, &rules);
            assert_eq!(view.guest.inventory[0].is_some(), shulker);
            view.guest.inventory[0] = Some(ItemStack::new("minecraft:blue_shulker_box", 1));
            click(&mut view, &mut drag, 5, 0, input::SWAP, &rules);
            assert_eq!(view.get(5).is_none(), shulker);
            // Anything else goes in.
            view.guest.carried = Some(ItemStack::new("minecraft:diamond", 1));
            click(&mut view, &mut drag, 9, 0, input::PICKUP, &rules);
            assert!(view.get(9).is_some());
        }
    }

    struct EquipRules;
    impl Rules for EquipRules {
        fn items(&self) -> &Items {
            unreachable!("a quick move reads no item catalog");
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn armor_index(&self, _: &str) -> Option<usize> {
            None
        }
        fn equip_slot(&self, item: &str) -> Option<&str> {
            (item == "minecraft:shield").then_some("offhand")
        }
    }

    /// A shift-click in the inventory menu puts a shield in the empty off-hand
    /// slot: `InventoryMenu.quickMoveStack` asks `getEquipmentSlotForItem` for
    /// `EquipmentSlot.OFFHAND` and moves it to slot 45 before it tries the
    /// hotbar. A full off hand, or an item with no `equippable` slot, still
    /// goes to the hotbar.
    #[test]
    fn inventory_quick_move_equips_an_off_hand_item() {
        let rules = EquipRules;
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            hidden: &[],
        };
        // Menu slot 9 is the first main-inventory slot, 45 the off hand.
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:shield", 1));
        click(&mut view, &mut drag, 9, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(45).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert!(view.get(9).is_none());
        assert!(view.get(36).is_none());
        // The off hand is taken: the next shield goes to the hotbar instead.
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:shield", 1));
        click(&mut view, &mut drag, 9, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(36).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert!(view.get(9).is_none());
        // An item that equips nowhere is untouched by the off-hand branch.
        view.guest.inventory[10] = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 10, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(37).as_ref().map(|s| s.count), Some(3));
        assert_eq!(
            view.get(45).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
    }

    /// Rules that refuse a pickup in some menu slots and can stop drops.
    struct StrictRules {
        items: Items,
        refuse: &'static [usize],
        drops: bool,
    }
    impl StrictRules {
        fn new(refuse: &'static [usize]) -> Self {
            Self {
                items: Items::for_tests(Default::default()),
                refuse,
                drops: true,
            }
        }
    }
    impl Rules for StrictRules {
        fn items(&self) -> &Items {
            &self.items
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn armor_index(&self, item: &str) -> Option<usize> {
            (item == "minecraft:diamond_chestplate").then_some(1)
        }
        fn may_pickup(&self, slot: usize, _: &ItemStack) -> bool {
            !self.refuse.contains(&slot)
        }
        fn can_drop_items(&self) -> bool {
            self.drops
        }
    }

    /// `Slot.mayPickup`: a pickup and a quick move leave a slot's stack alone
    /// when the menu's rules refuse it there, while the slot next to it still
    /// gives its own stack up.
    #[test]
    fn guest_pickup_and_quick_move_need_the_slots_consent() {
        let rules = StrictRules::new(&[0]);
        let mut g = guest();
        let mut chest = vec![
            Some(ItemStack::new("minecraft:stone", 5)),
            Some(ItemStack::new("minecraft:dirt", 2)),
        ];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        // The refused slot takes no click and gives nothing to the cursor.
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.guest.carried, None);
        // A quick move out of it takes nothing with it either.
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert!(view.get(2).is_none());
        // The slot next to it takes both clicks.
        click(&mut view, &mut drag, 1, 0, input::PICKUP, &rules);
        assert!(view.get(1).is_none());
        assert_eq!(
            view.guest.carried.as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:dirt", 2))
        );
        let back = view.guest.carried.take();
        view.set(1, back);
        click(&mut view, &mut drag, 1, 0, input::QUICK_MOVE, &rules);
        // A container's quick move fills the inventory from its end.
        assert_eq!(
            view.get(37).as_ref().map(|s| &*s.item),
            Some("minecraft:dirt")
        );
    }

    /// `doClick`'s PICKUP_ALL arm returns when the clicked slot holds a stack
    /// it may pick up, so a double-click collects from an empty slot, or from
    /// one that refuses, only.
    #[test]
    fn guest_pickup_all_starts_from_an_empty_or_refusing_slot() {
        let rules = StrictRules::new(&[]);
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 5)), None];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        // One stone short of a full stack, over a slot that holds stone.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 0, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(63));
        // The empty slot next to it does collect that one stone.
        click(&mut view, &mut drag, 1, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        // A refusing slot is where a collect may start: the stone comes from
        // the slot that allows it, the refused one is left alone.
        let refusing = StrictRules::new(&[0]);
        view.set(0, Some(ItemStack::new("minecraft:stone", 5)));
        view.set(1, Some(ItemStack::new("minecraft:stone", 5)));
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 0, 1, input::PICKUP_ALL, &refusing);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
    }

    /// `Player.canDropItems`: a client that has dropped too fast sends THROW
    /// clicks the menu does not act on.
    #[test]
    fn guest_throw_needs_can_drop_items() {
        let mut rules = StrictRules::new(&[]);
        rules.drops = false;
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 5))];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        let (drops, _) = click(&mut view, &mut drag, 0, 0, input::THROW, &rules);
        assert!(drops.is_empty());
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        rules.drops = true;
        let (drops, _) = click(&mut view, &mut drag, 0, 0, input::THROW, &rules);
        assert_eq!(drops.iter().fold(0, |n, s| n + s.count), 1);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
    }

    /// `ArmorSlot.mayPickup`: a survival guest cannot take cursed armour off;
    /// a creative one can, by a click and by a shift-click.
    #[test]
    fn guest_survival_cannot_remove_cursed_armour() {
        let rules = StrictRules::new(&[]);
        for (game_mode, taken) in [
            (crate::lan::guest::SURVIVAL, false),
            (crate::lan::guest::CREATIVE, true),
        ] {
            for mode in [input::PICKUP, input::QUICK_MOVE] {
                let mut g = guest();
                let mut cursed = ItemStack::new("minecraft:diamond_chestplate", 1);
                cursed.set_crafting_enchantments(vec![("minecraft:binding_curse".into(), 1)]);
                g.armor[1] = Some(cursed);
                g.game_mode = game_mode;
                let mut drag = None;
                let mut view = View {
                    guest: &mut g,
                    container: None,
                    shulker: false,
                    hidden: &[],
                };
                // Menu slot 6 is the chest slot.
                click(&mut view, &mut drag, 6, 0, mode, &rules);
                assert_eq!(view.get(6).is_some(), !taken);
                assert_eq!(view.guest.carried.is_some() || view.get(9).is_some(), taken);
            }
        }
    }

    struct ArmorRules;
    impl Rules for ArmorRules {
        fn items(&self) -> &Items {
            unreachable!("a quick move reads no item catalog");
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn armor_index(&self, item: &str) -> Option<usize> {
            (item == "minecraft:iron_helmet").then_some(0)
        }
        fn equip_slot(&self, _: &str) -> Option<&str> {
            None
        }
    }

    /// A shift-click from the off-hand slot wears the armour it holds:
    /// `InventoryMenu.quickMoveStack` asks for the item's equipment slot
    /// before it looks at the clicked index, so the helmet goes to the empty
    /// helmet slot. With that slot taken, and for anything else in the off
    /// hand, the stack fills the main inventory before the hotbar.
    #[test]
    fn inventory_quick_move_wears_armor_from_the_off_hand() {
        let rules = ArmorRules;
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            hidden: &[],
        };
        // Menu slot 45 is the off hand; 5 to 8 are the armour slots, head first.
        view.guest.offhand = Some(ItemStack::new("minecraft:iron_helmet", 1));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(5).as_ref().map(|s| &*s.item),
            Some("minecraft:iron_helmet")
        );
        assert!(view.get(45).is_none());
        assert!(view.get(9).is_none());
        // The helmet slot is taken: the next helmet goes to the inventory.
        view.guest.offhand = Some(ItemStack::new("minecraft:iron_helmet", 1));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(9).as_ref().map(|s| &*s.item),
            Some("minecraft:iron_helmet")
        );
        assert!(view.get(45).is_none());
        // Anything else fills the main inventory before the hotbar.
        view.guest.offhand = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(10).as_ref().map(|s| s.count), Some(3));
        assert!(view.get(36).is_none());
        assert!(view.get(45).is_none());
    }

    /// `doClick` repeats `quickMoveStack` while the clicked slot keeps the
    /// item, because `moveItemStackTo` fills one empty slot per call. This
    /// engine's `move_to` fills every empty slot of its range in one pass, so a
    /// single quick move must spread a stack over several slots with the same
    /// remainder vanilla's loop leaves, and a full inventory must leave the
    /// stack where it is.
    #[test]
    fn one_quick_move_spreads_a_stack_like_vanillas_repeat() {
        struct SmallRules;
        impl Rules for SmallRules {
            fn items(&self) -> &Items {
                unreachable!("a quick move reads no item catalog");
            }
            fn max_stack(&self, item: &str) -> u32 {
                if item == "minecraft:snowball" { 16 } else { 64 }
            }
            fn armor_index(&self, _: &str) -> Option<usize> {
                None
            }
        }
        let rules = SmallRules;
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:snowball", 40))];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            hidden: &[],
        };
        // A container's quick move fills the inventory from its end: 16
        // snowballs in the last hotbar slot (menu slot 36), 16 in the one
        // before it, 8 in the third, and nothing left in the chest.
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert!(view.get(0).is_none());
        assert_eq!(view.get(36).map(|s| s.count), Some(16));
        assert_eq!(view.get(35).map(|s| s.count), Some(16));
        assert_eq!(view.get(34).map(|s| s.count), Some(8));
        // Two slots' worth of room only: the loop vanilla runs would stop
        // after the second call, so 8 stay in the chest.
        for i in 0..36 {
            view.guest.inventory[i] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        view.set(0, Some(ItemStack::new("minecraft:snowball", 40)));
        // The fill starts at the hotbar's end: menu slots 36 and 35.
        view.guest.inventory[8] = None;
        view.guest.inventory[7] = None;
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(8));
        assert_eq!(view.get(36).map(|s| s.count), Some(16));
        assert_eq!(view.get(35).map(|s| s.count), Some(16));
        // A full inventory moves nothing at all.
        for i in 0..36 {
            view.guest.inventory[i] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        view.set(0, Some(ItemStack::new("minecraft:snowball", 5)));
        let (drops, changed) = click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert!(drops.is_empty() && changed.is_empty());
    }

    /// `AbstractContainerMenu.isContainerClickButton`: a pickup and a quick
    /// move exist for buttons 0 and 1 only, so a modified client's other
    /// buttons take nothing out of a slot, move nothing to the hotbar and
    /// drop nothing from the cursor. The other modes are not buttons' business.
    #[test]
    fn guest_clicks_with_a_button_other_than_0_or_1_move_nothing() {
        let rules = ItemRules(Items::for_tests(Default::default()));
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            hidden: &[],
        };
        // Menu slot 9 stands for inventory index 9, 36 for the hotbar's first.
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 9, 2, input::PICKUP, &rules);
        assert_eq!(view.guest.carried, None);
        assert_eq!(view.get(9).map(|s| s.count), Some(3));
        click(&mut view, &mut drag, 9, 2, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(9).map(|s| s.count), Some(3));
        assert!(view.get(36).is_none());
        // A click outside the window drops nothing either.
        view.guest.carried = Some(ItemStack::new("minecraft:dirt", 5));
        let (drops, changed) = click(&mut view, &mut drag, -999, 2, input::PICKUP, &rules);
        assert!(drops.is_empty() && changed.is_empty());
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(5));
        // Button 1 still takes half of the stack.
        view.guest.carried = None;
        click(&mut view, &mut drag, 9, 1, input::PICKUP, &rules);
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(2));
        assert_eq!(view.get(9).map(|s| s.count), Some(1));
        // THROW reads its button as "one or the whole stack", not as a
        // primary or secondary action.
        view.guest.carried = None;
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:diamond", 4));
        let (drops, _) = click(&mut view, &mut drag, 9, 2, input::THROW, &rules);
        assert_eq!(drops.iter().fold(0, |n, s| n + s.count), 4);
        assert!(view.get(9).is_none());
    }
}
