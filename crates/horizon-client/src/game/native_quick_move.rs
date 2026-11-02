//! A shift-click in a mod container's menu, routed by the mods
//! (`native_menu_quick_move`, `AbstractContainerMenu.quickMoveStack`) before
//! the game's own routing.
use super::{Game, lan::LanHost};
use horizon_client::{
    block_entities::Pos, containers::BlockEntity, items::ItemStack, lan::menu::Slots,
};
use horizon_protocol::packets::play;

/// `ClickType.QUICK_MOVE`'s id in `ServerboundContainerClickPacket`.
const QUICK_MOVE: i32 = 1;

/// Where a menu slot of a block container's menu lives: the container's own
/// slot `k`, or the player's inventory index (hotbar 0 to 8, then the 27
/// main slots), the menu listing the main slots before the hotbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuSlot {
    Container(usize),
    Inventory(usize),
}

/// The slot a quick move of `slot` on a menu with `size` container slots
/// asks a mod about, or None when the click is not one: another click type,
/// a button other than left or right (`doClick` takes 0 and 1), no slot,
/// a slot past the menu, or a container slot the mod's screen hides (the
/// menu has no such slot, so the click goes nowhere).
pub(super) fn quick_move_slot(
    input: i32,
    button: i32,
    slot: i32,
    size: usize,
    hidden: &[usize],
) -> Option<MenuSlot> {
    if input != QUICK_MOVE || !(0..=1).contains(&button) {
        return None;
    }
    let slot = usize::try_from(slot).ok()?;
    if slot < size {
        return (!hidden.contains(&slot)).then_some(MenuSlot::Container(slot));
    }
    let n = slot - size;
    match n {
        0..27 => Some(MenuSlot::Inventory(9 + n)),
        27..36 => Some(MenuSlot::Inventory(n - 27)),
        _ => None,
    }
}

/// What a quick move the mods were asked about leaves: when one took it
/// (`handled`), the clicked slot holds what is left of `stack`, whatever the
/// mods wrote there, and their other changes to the block's items stay;
/// otherwise the block's items are as they were before the call (`before`),
/// so nothing a mod moved without taking the click is kept twice.
fn settle(
    at: MenuSlot,
    handled: bool,
    stack: ItemStack,
    before: &[Option<ItemStack>],
    items: &mut Vec<Option<ItemStack>>,
    inventory: &mut [Option<ItemStack>],
) {
    if !handled {
        before.clone_into(items);
        return;
    }
    let left = (stack.count > 0).then_some(stack);
    match at {
        MenuSlot::Container(k) => {
            if items.len() <= k {
                items.resize(k + 1, None);
            }
            items[k] = left;
        }
        MenuSlot::Inventory(i) => {
            if let Some(s) = inventory.get_mut(i) {
                *s = left;
            }
        }
    }
}

impl Game<'_> {
    /// `ServerGamePacketListenerImpl.handleContainerClick` for a quick move
    /// in a guest's open mod container menu (on a vanilla menu type or the
    /// mod's own): the mods' `native_menu_quick_move` first, as the clicking
    /// player. True when a mod took it: the slot keeps what the mod left and
    /// the guest's client gets the whole menu again. False for every other
    /// click, and for one no mod took, so the game's own routing runs.
    pub(super) fn lan_mod_quick_move(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some(g) = host.hub.guest(conn) else {
            return false;
        };
        let Some(menu) = g
            .menu
            .as_ref()
            .filter(|m| p.container != 0 && m.window == p.container)
        else {
            return false;
        };
        let Slots::Block(pos) = menu.slots else {
            return false;
        };
        let Some(at) = quick_move_slot(
            p.input,
            i32::from(p.button),
            i32::from(p.slot),
            menu.size,
            &menu.hidden,
        ) else {
            return false;
        };
        // A spectator's or a dead player's click and a menu no longer valid
        // change nothing, and a shift-click during a drag only ends the drag
        // (`doClick`'s resetQuickCraft): the game's own path answers them.
        if g.spectator()
            || g.dead
            || menu.drag.is_some()
            || !menu.slots.still_valid(&self.world, g, menu.pos)
        {
            return false;
        }
        let size = menu.size;
        let window = menu.window;
        // A mod menu type keeps its drag in the connection's machine menu
        // (`lan_machine_click`), not in `menu.drag`: a shift-click during it
        // goes there too, to only end the drag.
        if host
            .play
            .get(&conn)
            .and_then(|play| play.crafting.machine.as_ref())
            .is_some_and(|(w, m)| *w == window && m.quick_crafting())
        {
            return false;
        }
        if !matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(m)) if !m.id.is_empty()
        ) {
            return false;
        }
        let handled = self
            .as_guest(host, conn, |game| {
                game.mod_menu_quick_move(pos, size, p.slot as usize, at)
            })
            .unwrap_or(false);
        if !handled {
            return false;
        }
        // The mod menu type's engine starts over from the block entity next click.
        if let Some(play) = host.play.get_mut(&conn)
            && play
                .crafting
                .machine
                .as_ref()
                .is_some_and(|(w, _)| *w == window)
        {
            play.crafting.machine = None;
        }
        // sendAllDataToRemote: the client predicted the game's routing. A mod
        // that removed its own block leaves no menu to send: it closes, and
        // the inventory goes out on its own.
        if matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(_))
        ) {
            host.hub.resend_menu(conn, window, &self.world);
        } else if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
            host.hub.close_container(conn);
            host.hub.send_inventory(gi);
        }
        true
    }

    /// The mods' `native_menu_quick_move` on menu slot `slot` (living at
    /// `at`) of the mod container at `pos` (`size` container slots), for the
    /// player in place: true when
    /// one took it, the slot then holding what is left of its stack. The
    /// block entity is out of the world during the call.
    fn mod_menu_quick_move(&mut self, pos: Pos, size: usize, slot: usize, at: MenuSlot) -> bool {
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return false;
        };
        let held = match at {
            MenuSlot::Container(k) => m.items.get(k).cloned().flatten(),
            MenuSlot::Inventory(i) => self.inventory.get(i).cloned().flatten(),
        };
        let Some(mut stack) = held.filter(|s| s.count > 0) else {
            return false;
        };
        let mut taken = std::mem::take(&mut **m);
        let items = taken.items.clone();
        let mut handled = false;
        self.native_menu_quick_move(
            [pos.0, pos.1, pos.2],
            &mut taken,
            size,
            slot,
            &mut stack,
            &mut handled,
        );
        settle(
            at,
            handled,
            stack,
            &items,
            &mut taken.items,
            &mut self.inventory,
        );
        self.put_back_mod_block_entity(pos, taken, &items);
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::{MenuSlot, quick_move_slot, settle};
    use horizon_client::items::ItemStack;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// A quick move asks about the clicked slot only: a `generic_9x1` menu
    /// has the container's 9, then the main inventory, then the hotbar.
    #[test]
    fn a_quick_move_names_the_container_or_inventory_slot() {
        assert_eq!(
            quick_move_slot(1, 0, 0, 9, &[]),
            Some(MenuSlot::Container(0))
        );
        assert_eq!(
            quick_move_slot(1, 1, 8, 9, &[]),
            Some(MenuSlot::Container(8))
        );
        // The first main slot is inventory index 9, the last 35.
        assert_eq!(
            quick_move_slot(1, 0, 9, 9, &[]),
            Some(MenuSlot::Inventory(9))
        );
        assert_eq!(
            quick_move_slot(1, 0, 35, 9, &[]),
            Some(MenuSlot::Inventory(35))
        );
        // The hotbar comes last in the menu and first in the inventory.
        assert_eq!(
            quick_move_slot(1, 0, 36, 9, &[]),
            Some(MenuSlot::Inventory(0))
        );
        assert_eq!(
            quick_move_slot(1, 0, 44, 9, &[]),
            Some(MenuSlot::Inventory(8))
        );
    }

    /// Only a left or right shift-click on a slot the menu has is a quick
    /// move: no other click type, button, outside click or hidden slot.
    #[test]
    fn other_clicks_are_not_quick_moves() {
        // PICKUP, SWAP, CLONE, THROW, QUICK_CRAFT, PICKUP_ALL.
        for input in [0, 2, 3, 4, 5, 6] {
            assert_eq!(quick_move_slot(input, 0, 0, 9, &[]), None);
        }
        assert_eq!(quick_move_slot(1, 2, 0, 9, &[]), None);
        assert_eq!(quick_move_slot(1, -1, 0, 9, &[]), None);
        assert_eq!(quick_move_slot(1, 0, -999, 9, &[]), None);
        assert_eq!(quick_move_slot(1, 0, 45, 9, &[]), None);
        // A slot the mod's screen hides is not in the menu.
        assert_eq!(quick_move_slot(1, 0, 3, 9, &[3]), None);
        assert_eq!(
            quick_move_slot(1, 0, 4, 9, &[3]),
            Some(MenuSlot::Container(4))
        );
        // The inventory's slots are never hidden.
        assert_eq!(
            quick_move_slot(1, 0, 9, 9, &[0, 1, 2, 3, 4, 5, 6, 7, 8]),
            Some(MenuSlot::Inventory(9))
        );
    }

    /// A taken shift-click from the inventory: the clicked slot keeps what the
    /// mod left, and what it put in its block stays there.
    #[test]
    fn a_taken_click_leaves_the_remainder_in_the_clicked_slot() {
        let before = vec![None; 9];
        let mut items = before.clone();
        items[8] = Some(stack("minecraft:cobblestone", 32));
        let mut inventory = vec![None; 36];
        inventory[0] = Some(stack("minecraft:cobblestone", 64));
        let left = stack("minecraft:cobblestone", 32);
        settle(
            MenuSlot::Inventory(0),
            true,
            left.clone(),
            &before,
            &mut items,
            &mut inventory,
        );
        assert_eq!(inventory[0], Some(left));
        assert_eq!(items[8], Some(stack("minecraft:cobblestone", 32)));
        // All of it taken: the slot empties.
        settle(
            MenuSlot::Inventory(0),
            true,
            stack("minecraft:cobblestone", 0),
            &before,
            &mut items,
            &mut inventory,
        );
        assert_eq!(inventory[0], None);
    }

    /// A taken click on the block's own slot: the remainder wins over what the
    /// mod wrote to that slot, so a moved stack is never also left behind.
    #[test]
    fn the_clicked_container_slot_gets_the_remainder() {
        let before = vec![Some(stack("minecraft:dirt", 10))];
        // The mod moved the dirt to a slot past the menu's and left the clicked one as it was.
        let mut items = vec![
            Some(stack("minecraft:dirt", 10)),
            None,
            Some(stack("minecraft:dirt", 10)),
        ];
        let mut inventory = vec![None; 36];
        settle(
            MenuSlot::Container(0),
            true,
            stack("minecraft:dirt", 0),
            &before,
            &mut items,
            &mut inventory,
        );
        assert_eq!(items, [None, None, Some(stack("minecraft:dirt", 10))]);
    }

    /// A click no mod took leaves the block's items as they were before the
    /// call, so a mod that moved items without taking it duplicates nothing.
    #[test]
    fn an_untaken_click_keeps_the_items_as_they_were() {
        let before = vec![Some(stack("minecraft:dirt", 10)), None];
        let mut items = vec![None, Some(stack("minecraft:dirt", 10))];
        let mut inventory = vec![None; 36];
        inventory[3] = Some(stack("minecraft:dirt", 5));
        settle(
            MenuSlot::Inventory(3),
            false,
            stack("minecraft:dirt", 0),
            &before,
            &mut items,
            &mut inventory,
        );
        assert_eq!(items, before);
        assert_eq!(inventory[3], Some(stack("minecraft:dirt", 5)));
    }
}
