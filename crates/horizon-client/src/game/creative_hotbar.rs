//! The creative toolbars, `hotbar.nbt` beside `options.txt`: hold the save or
//! load key and press a hotbar key (`Minecraft.handleKeybinds`).
use super::Game;
use horizon_client::{hotbar, items::ItemStack, keybinds, text::Text};
use horizon_world::nbt::Tag;

impl Game<'_> {
    /// Whether the key bound to `mapping` is held.
    pub(super) fn mapped_key_down(&self, mapping: &str) -> bool {
        keybinds::key_code(self.settings.keys.get(mapping))
            .is_some_and(|k| self.physical_keys.contains(&k))
    }

    /// `CreativeModeInventoryScreen.selectTab` on the HOTBAR tab: each saved
    /// row's nine stacks, or for an empty row a locked paper in that row's own
    /// slot, named after the keys that save it (`inventory.hotbarInfo`).
    pub(super) fn refresh_saved_hotbars(&mut self) {
        if !self.creative_screen.tab().hotbar {
            return;
        }
        let path = Self::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.res.items().get(id).is_some();
        let rows = hotbar::load(&path, &known);
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping)).plain();
        let empty = || ItemStack::new("minecraft:air", 0);
        let mut items = Vec::with_capacity(hotbar::GROUPS * hotbar::SIZE);
        for (i, row) in rows.iter().enumerate().take(hotbar::GROUPS) {
            if row.iter().any(Option::is_some) {
                items.extend(row.iter().map(|s| s.clone().unwrap_or_else(empty)));
                continue;
            }
            for j in 0..hotbar::SIZE {
                if j != i {
                    items.push(empty());
                    continue;
                }
                let with = [
                    key("key.saveToolbarActivator"),
                    key(&format!("key.hotbar.{}", i + 1)),
                ];
                let mut paper = ItemStack::new("minecraft:paper", 1);
                let data = paper.data_mut();
                data.set_other(
                    "minecraft:creative_slot_lock",
                    Some(Tag::compound(Vec::<(&str, Tag)>::new())),
                );
                data.set_other(
                    "minecraft:item_name",
                    Some(Tag::compound([
                        ("translate", Tag::String("inventory.hotbarInfo".into())),
                        (
                            "with",
                            Tag::List(
                                with.iter()
                                    .map(|s| Tag::String(s.as_str().into()))
                                    .collect(),
                            ),
                        ),
                    ])),
                );
                items.push(paper);
            }
        }
        self.creative_screen.set_saved_hotbars(items);
    }
    /// `CreativeModeInventoryScreen.handleHotbarLoadOrSave` for the hotbar key
    /// `index`: load puts the saved row in the hotbar (the creative slot
    /// packets follow from the inventory change), save stores the hotbar and
    /// says so on the action bar.
    pub(super) fn hotbar_load_or_save(&mut self, index: usize, load: bool) {
        let path = Self::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.items().get(id).is_some();
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
        // handleHotbarLoadOrSave: the overlay, said at once.
        self.say_system_now(&message);
        self.titles.set_overlay(message, false);
    }
}
