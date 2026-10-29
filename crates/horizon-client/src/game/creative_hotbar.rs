//! The creative toolbars, `hotbar.nbt` beside `options.txt`: hold the save or
//! load key and press a hotbar key (`Minecraft.handleKeybinds`).
use super::Game;
use horizon_client::{hotbar, keybinds, text::Text};

impl Game<'_> {
    /// Whether the key bound to `mapping` is held.
    pub(super) fn mapped_key_down(&self, mapping: &str) -> bool {
        keybinds::key_code(self.settings.keys.get(mapping))
            .is_some_and(|k| self.physical_keys.contains(&k))
    }

    /// `CreativeModeInventoryScreen.handleHotbarLoadOrSave` for the hotbar key
    /// `index`: load puts the saved row in the hotbar (the creative slot
    /// packets follow from the inventory change), save stores the hotbar and
    /// says so on the action bar.
    pub(super) fn hotbar_load_or_save(&mut self, index: usize, load: bool) {
        let path = Self::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.res.items().get(id).is_some();
        let mut rows = hotbar::load(&path, &known);
        if load {
            for (slot, stack) in rows[index].iter().enumerate() {
                self.inventory[slot] = stack.clone();
            }
            return;
        }
        rows[index] = self.inventory[..hotbar::SIZE].to_vec();
        if let Err(e) = hotbar::save(&path, &rows) {
            // saveHotbar sets the message inside its try, so a failed save says nothing.
            log::warn!("the creative hotbar was not saved: {e}");
            return;
        }
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping));
        let message = Text::translate(
            "inventory.hotbarSaved",
            &[
                key("key.loadToolbarActivator"),
                key(&format!("key.hotbar.{}", index + 1)),
            ],
        );
        self.titles.set_overlay(message, false);
    }
}
