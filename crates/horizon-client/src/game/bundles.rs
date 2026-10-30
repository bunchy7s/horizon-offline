//! `BundleMouseActions`: the wheel over a bundle in a container screen selects
//! one of the items it shows, which a click that takes one out of it then takes
//! (`BundleContents.Mutable.removeOne`). Moving off the bundle clears the
//! selection. On a server the selection is sent to it (`ServerboundSelectBundleItemPacket`).
use super::{Game, Screen};
use horizon_client::{
    bundle::{Contents, toggle_selected},
    mouse::ScrollWheelHandler,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

#[derive(Default)]
pub(super) struct BundleMouse {
    wheel: ScrollWheelHandler,
    /// The slot under the cursor at the last tick (`AbstractContainerScreen.hoveredSlot`).
    hovered: Option<usize>,
}

/// Whether `screen` is an `AbstractContainerScreen`, whose `mouseScrolled` first offers
/// the wheel to the hovered slot's `ItemSlotMouseAction`s (the bundle's), and whose
/// `onStopHovering` clears the selection: the inventory and container menus, and
/// `CreativeModeInventoryScreen`, which scrolls its items only when no bundle took
/// the wheel.
pub(super) fn takes_bundle_wheel(screen: Screen) -> bool {
    matches!(screen, Screen::Inventory | Screen::Creative)
}

impl Game<'_> {
    /// A slot a bundle can be selected in: the player's inventory, the off hand and a
    /// container's own slots. A server only knows these in the inventory and chest-like menus.
    fn bundle_slot(&mut self, slot: usize) -> bool {
        (slot < 36 || slot == 504 || (300..500).contains(&slot))
            && (self.remote.is_none() || self.remote_storage_open())
            && (slot < 36 || slot == 504 || self.slots().contains(&slot))
    }

    /// `BundleMouseActions.onMouseScrolled`: the wheel over a bundle moves the selection
    /// through the items its tooltip shows. True when the bundle took the wheel.
    pub(super) fn bundle_scroll(&mut self, x: f64, y: f64) -> bool {
        let Some(slot) = self.inventory_target().0 else {
            return false;
        };
        if !self.bundle_slot(slot) {
            return false;
        }
        let stack = self.slot_mut(slot).clone();
        let items = self.items();
        let Some(bundle) = stack.filter(|s| items.get(&s.item).is_some_and(|i| i.bundle)) else {
            return false;
        };
        let shown = Contents::of(items, &bundle).map_or(0, |c| c.shown_count());
        if shown == 0 {
            return false;
        }
        let (wheel_x, wheel_y) = self.bundle_mouse.wheel.on_mouse_scroll(x, y);
        let wheel = if wheel_y == 0 { -wheel_x } else { wheel_y };
        if wheel != 0 {
            let selected = bundle.data().map_or(-1, |d| d.bundle_selected.0);
            let next = ScrollWheelHandler::next_selection(f64::from(wheel), selected, shown as i32);
            if next != selected {
                self.select_bundle(slot, next);
            }
        }
        true
    }

    /// `BundleMouseActions.toggleSelectedBundleItem`: the stack here and, on a server, there.
    fn select_bundle(&mut self, slot: usize, index: i32) {
        if let Some(stack) = self.slot_mut(slot) {
            toggle_selected(stack, index);
        }
        if self.remote.is_some() {
            let (_, menu) = self.remote_menu();
            if let Some(menu_slot) = Self::menu_slot_index(&menu, slot) {
                self.send(S::SelectBundleItem(play::SelectBundleItem {
                    slot: menu_slot as i32,
                    index,
                }));
            }
        }
    }

    /// `BundleMouseActions.unselectedBundleItem` for the bundle in `slot`, if it has a selection.
    pub(super) fn unselect_bundle(&mut self, slot: usize) {
        if !self.bundle_slot(slot) {
            return;
        }
        let selected = self
            .slot_mut(slot)
            .as_ref()
            .and_then(|s| s.data())
            .is_some_and(|d| d.bundle_selected.0 >= 0);
        if selected {
            self.select_bundle(slot, -1);
        }
    }

    /// `AbstractContainerScreen.onStopHovering` (and `onClose`): leaving a slot, or the
    /// screen, clears the bundle's selection.
    pub(super) fn tick_bundle_hover(&mut self) {
        let now = if takes_bundle_wheel(self.screen) {
            self.inventory_target().0
        } else {
            None
        };
        if let Some(left) = std::mem::replace(&mut self.bundle_mouse.hovered, now)
            && Some(left) != now
        {
            self.unselect_bundle(left);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CreativeModeInventoryScreen.mouseScrolled` calls `AbstractContainerScreen`'s
    /// first: a bundle under the cursor takes the wheel there too (#3211).
    #[test]
    fn container_screens_offer_the_wheel_to_bundles() {
        assert!(takes_bundle_wheel(Screen::Inventory));
        assert!(takes_bundle_wheel(Screen::Creative));
        for screen in [
            Screen::Playing,
            Screen::Chat,
            Screen::Paused,
            Screen::Options,
        ] {
            assert!(!takes_bundle_wheel(screen));
        }
    }
}
