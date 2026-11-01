//! Server menus for native mods (docs/native-mods.md, "Server menus", API
//! 10): a vanilla chest menu (`generic_9x1` to `generic_9x6`) a mod opens
//! for a player and fills with items to look at. No click moves anything:
//! each reaches the mod (`native_menu_clicked`), as does the menu's closing
//! (`native_menu_closed`), and the client shows the menu as the server sends
//! it again. A stock client shows it; nothing on the player's side.
//! `native_exchange_items` swaps two players' stacks in one step, checked
//! first, so a trade built on these menus never moves an item before both
//! sides are settled.
use super::Game;
use horizon_client::{
    items::ItemStack,
    lan::{data::component_nbt, menu::NativeMenuEvent},
    text::Text,
};

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(crate) fn native_menu_inventory_slot(size: usize, slot: i32) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// Adds every stack of `gets` to the 36 slots of `inventory`, onto matching
/// stacks first, then into empty slots, hotbar first; false when one doesn't
/// fit whole. `max` is an item's stack size.
fn fit(
    inventory: &mut [Option<ItemStack>],
    gets: &[ItemStack],
    max: &impl Fn(&ItemStack) -> u32,
) -> bool {
    for stack in gets {
        let mut left = stack.count;
        let max = max(stack).max(1);
        for slot in inventory.iter_mut().take(36).flatten() {
            if left > 0 && slot.same_item_same_components(stack) && slot.count < max {
                let n = (max - slot.count).min(left);
                slot.count += n;
                left -= n;
            }
        }
        for slot in inventory.iter_mut().take(36) {
            if left > 0 && slot.is_none() {
                let n = max.min(left);
                *slot = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                left -= n;
            }
        }
        if left > 0 {
            return false;
        }
    }
    true
}

/// Two players' offers swapped, or nothing changed: each offer is the
/// inventory slots it takes with the stack each must still hold, exactly.
/// The stacks leave their slots, then go to the other side where they fit
/// whole; an Err (which side, why) leaves both inventories as they were.
fn exchange(
    a: &mut Vec<Option<ItemStack>>,
    a_offer: &[(usize, ItemStack)],
    b: &mut Vec<Option<ItemStack>>,
    b_offer: &[(usize, ItemStack)],
    max: impl Fn(&ItemStack) -> u32,
) -> Result<(), String> {
    fn take(
        inv: &[Option<ItemStack>],
        offer: &[(usize, ItemStack)],
        who: &str,
    ) -> Result<(Vec<Option<ItemStack>>, Vec<ItemStack>), String> {
        let mut left = inv.to_vec();
        let mut taken = Vec::new();
        for (slot, expected) in offer {
            match left.get_mut(*slot).filter(|_| *slot < 36) {
                Some(s) if s.as_ref() == Some(expected) => taken.push(s.take().expect("checked")),
                _ => return Err(format!("{who} no longer holds their offer")),
            }
        }
        Ok((left, taken))
    }
    let (mut a_after, a_gives) = take(a, a_offer, "the first player")?;
    let (mut b_after, b_gives) = take(b, b_offer, "the second player")?;
    if !fit(&mut a_after, &b_gives, &max) {
        return Err("the first player has no room".into());
    }
    if !fit(&mut b_after, &a_gives, &max) {
        return Err("the second player has no room".into());
    }
    *a = a_after;
    *b = b_after;
    Ok(())
}

/// Server menus, for server mods (API 10). The players are named by UUID;
/// none of these runs while a player's state is swapped in (inside
/// `native_perform_command`, the menus can be opened and filled, but
/// `native_player_stacks` and `native_exchange_items` refuse: call them from
/// `native_menu_clicked` or `native_server_tick`).
#[allow(dead_code)]
impl Game<'_> {
    fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.uuid == uuid)?;
        Some(g.conn)
    }
    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its window id.
    pub(super) fn native_open_menu(
        &mut self,
        uuid: u128,
        rows: u8,
        title: Text,
        items: Vec<Option<ItemStack>>,
    ) -> Option<i32> {
        let conn = self.native_conn(uuid)?;
        let mut host = self.lan.take()?;
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(&mut host, conn);
        let window =
            host.hub
                .open_native_menu(conn, rows, component_nbt(&title.component()), items);
        self.lan = Some(host);
        window
    }
    /// Replaces what the player's open native menu shows; sent with the
    /// tick's container sync.
    pub(super) fn native_set_menu_items(&mut self, uuid: u128, items: Vec<Option<ItemStack>>) {
        if let Some(conn) = self.native_conn(uuid)
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.set_native_menu(conn, items);
        }
    }
    /// Closes the player's native menu (a `native_menu_closed` follows).
    pub(super) fn native_close_menu(&mut self, uuid: u128) {
        if let Some(conn) = self.native_conn(uuid)
            && let Some(host) = self.lan.as_mut()
            && host
                .hub
                .guest(conn)
                .and_then(|g| g.menu.as_ref())
                .is_some_and(|m| m.slots == horizon_client::lan::menu::Slots::Native)
        {
            host.hub.close_container(conn);
        }
    }
    /// The player's 36 inventory slots (0..9 the hotbar), to show or offer
    /// them; None for a player not here, dead, or whose state is swapped in.
    pub(super) fn native_player_stacks(&self, uuid: u128) -> Option<Vec<Option<ItemStack>>> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.uuid == uuid)?;
        (!g.dead && self.lan_acting != Some(g.conn)).then(|| {
            let mut stacks = g.inventory.clone();
            stacks.resize(36, None);
            stacks
        })
    }
    /// Swaps two players' offers (inventory slots, each with the stack it
    /// must still hold, exactly) in one step: the stacks leave their slots
    /// and go to the other player where they fit whole, or nothing moves and
    /// the Err says why. Both must be here and alive, neither with their
    /// state swapped in. Their screens follow with the tick's sync.
    pub(super) fn native_exchange_items(
        &mut self,
        a: u128,
        a_offer: &[(usize, ItemStack)],
        b: u128,
        b_offer: &[(usize, ItemStack)],
    ) -> Result<(), String> {
        if a == b {
            return Err("a player can't trade with themselves".into());
        }
        let acting = self.lan_acting;
        let items = self.res.items();
        let Some(host) = self.lan.as_mut() else {
            return Err("no server".into());
        };
        let find = |uuid: u128| {
            host.hub
                .guests
                .iter()
                .position(|g| g.placed && !g.dead && g.uuid == uuid)
                .filter(|&i| acting != Some(host.hub.guests[i].conn))
        };
        let (Some(ia), Some(ib)) = (find(a), find(b)) else {
            return Err("both players must be here".into());
        };
        let mut inv_a = std::mem::take(&mut host.hub.guests[ia].inventory);
        let mut inv_b = std::mem::take(&mut host.hub.guests[ib].inventory);
        let result = exchange(&mut inv_a, a_offer, &mut inv_b, b_offer, |s| {
            items.get(&s.item).map_or(64, |i| i.max_stack)
        });
        host.hub.guests[ia].inventory = inv_a;
        host.hub.guests[ib].inventory = inv_b;
        result
    }
}

/// Where the game calls the menu hook points.
impl Game<'_> {
    /// The clicks on and closings of native menus the hub collected, each
    /// told to the mods with the hub back in `self.lan`, no player's state
    /// swapped in.
    pub(super) fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::Click {
                    uuid,
                    window,
                    slot,
                    button,
                    mode,
                } => self.native_menu_clicked(uuid, window, slot, button, mode),
                NativeMenuEvent::Closed { uuid, window } => self.native_menu_closed(uuid, window),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }
    fn inventory(stacks: &[(usize, ItemStack)]) -> Vec<Option<ItemStack>> {
        let mut inv = vec![None; 36];
        for (i, s) in stacks {
            inv[*i] = Some(s.clone());
        }
        inv
    }
    fn max(s: &ItemStack) -> u32 {
        if &*s.item == "minecraft:diamond_sword" {
            1
        } else {
            64
        }
    }

    #[test]
    fn menu_clicks_map_to_the_inventory_below() {
        // A 6-row chest: 54 slots, then the inventory's 27, then the hotbar.
        assert_eq!(native_menu_inventory_slot(54, 3), None);
        assert_eq!(native_menu_inventory_slot(54, 54), Some(9));
        assert_eq!(native_menu_inventory_slot(54, 80), Some(35));
        assert_eq!(native_menu_inventory_slot(54, 81), Some(0));
        assert_eq!(native_menu_inventory_slot(54, 89), Some(8));
        assert_eq!(native_menu_inventory_slot(54, 90), None);
        assert_eq!(native_menu_inventory_slot(54, -999), None);
        assert_eq!(native_menu_inventory_slot(9, 9), Some(9));
    }

    #[test]
    fn an_exchange_swaps_both_offers() {
        let mut a = inventory(&[
            (0, stack("minecraft:diamond", 5)),
            (1, stack("minecraft:dirt", 64)),
        ]);
        let mut b = inventory(&[
            (4, stack("minecraft:diamond_sword", 1)),
            (5, stack("minecraft:diamond", 60)),
        ]);
        let a_offer = [(0, stack("minecraft:diamond", 5))];
        let b_offer = [(4, stack("minecraft:diamond_sword", 1))];
        exchange(&mut a, &a_offer, &mut b, &b_offer, max).unwrap();
        // A gets the sword in the slot their diamonds left; B's diamonds
        // fill their stack to 64, the rest in the first empty slot.
        assert_eq!(a[0], Some(stack("minecraft:diamond_sword", 1)));
        assert_eq!(a[1], Some(stack("minecraft:dirt", 64)));
        assert_eq!(b[0], Some(stack("minecraft:diamond", 1)));
        assert_eq!(b[4], None);
        assert_eq!(b[5], Some(stack("minecraft:diamond", 64)));
        let count = |inv: &[Option<ItemStack>], item: &str| -> u32 {
            inv.iter()
                .flatten()
                .filter(|s| &*s.item == item)
                .map(|s| s.count)
                .sum()
        };
        // Nothing made, nothing lost.
        assert_eq!(
            count(&a, "minecraft:diamond") + count(&b, "minecraft:diamond"),
            65
        );
        assert_eq!(
            count(&a, "minecraft:diamond_sword") + count(&b, "minecraft:diamond_sword"),
            1
        );
    }

    #[test]
    fn an_exchange_that_cant_complete_moves_nothing() {
        let a0 = inventory(&[(0, stack("minecraft:diamond", 5))]);
        let b0 = inventory(&[(2, stack("minecraft:emerald", 3))]);
        // The offer changed since it was made (moved, dropped, used).
        let (mut a, mut b) = (a0.clone(), b0.clone());
        let stale = [(0, stack("minecraft:diamond", 6))];
        assert!(exchange(&mut a, &stale, &mut b, &[], max).is_err());
        assert_eq!((&a, &b), (&a0, &b0));
        // An empty or out-of-range slot.
        assert!(
            exchange(
                &mut a,
                &[(7, stack("minecraft:diamond", 5))],
                &mut b,
                &[],
                max
            )
            .is_err()
        );
        assert!(
            exchange(
                &mut a,
                &[(36, stack("minecraft:diamond", 5))],
                &mut b,
                &[],
                max
            )
            .is_err()
        );
        assert_eq!((&a, &b), (&a0, &b0));
        // No room: B's inventory is full of other things.
        let full: Vec<_> = (0..36).map(|_| Some(stack("minecraft:dirt", 64))).collect();
        let mut b = full.clone();
        let offer = [(0, stack("minecraft:diamond", 5))];
        assert!(exchange(&mut a, &offer, &mut b, &[], max).is_err());
        assert_eq!((&a, &b), (&a0, &full));
        // The same slot offered twice is the second time empty.
        let twice = [
            (0, stack("minecraft:diamond", 5)),
            (0, stack("minecraft:diamond", 5)),
        ];
        let mut b = b0.clone();
        assert!(exchange(&mut a, &twice, &mut b, &[], max).is_err());
        assert_eq!((&a, &b), (&a0, &b0));
    }
}
