//! Horizon's inventory extras (#332, `docs/extras.md`), Mouse Tweaks-style
//! controls and sorting, both off by default: while an extra is off its
//! hooks return false and the screen clicks exactly as vanilla's. Each action is made of ordinary slot clicks
//! (the menu engine's, or `ContainerClick`s on a server), so it works
//! wherever vanilla's clicks do. They run in the player inventory, the
//! chest-like containers (chests, barrels, ender chests, shulker boxes,
//! hoppers, dispensers), furnaces and crafting tables.
//!
//! - Right-drag holding a stack: one item into each slot entered, at once.
//! - Left-drag holding a stack, before vanilla's drag has spread over a
//!   second slot: each slot entered holding the same item is gathered into
//!   the hand (when it all fits); with Shift, those slots are quick-moved.
//! - Shift + left-drag with an empty hand: every slot entered is quick-moved.
//! - The wheel over a slot, with an empty hand: down moves one of its items
//!   where a Shift click would send it (`quickMoveStack`: fuel to the fuel
//!   slot, a smeltable to the input), up brings one of the same item from the
//!   other side. Down over a result slot takes one craft or the whole furnace
//!   output to the inventory; nothing ever goes into a result slot.
//!
//! Sorting draws a small button by the container's title and one by the
//! inventory's (an optional key sorts the part under the cursor); the
//! clicks come from `inv_sort::plan`.
use super::{Creative, Game, Screen};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    inv_sort,
    items::ItemStack,
    keybinds,
    menu::Input,
    stations::hover_name,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// A sort button's side in GUI pixels.
const SORT_BUTTON: f32 = 9.0;

#[derive(Default)]
pub(super) struct Tweaks {
    drag: Option<Drag>,
    /// Wheel steps not yet acted on (a trackpad scrolls in fractions).
    wheel: f32,
}
/// A server's menu the tweaks run in: their clicks are vanilla's in these,
/// and the menu engine knows which slot takes what. Not the menus whose slots
/// cost or trigger something (anvils, enchanting, merchants, beacons, crafters).
pub(super) fn tweak_menu(name: &str) -> bool {
    name == "inventory"
        || name.starts_with("minecraft:generic_9x")
        || matches!(
            name,
            "minecraft:shulker_box"
                | "minecraft:hopper"
                | "minecraft:generic_3x3"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
                | "minecraft:crafting"
        )
}
/// A drag the tweaks follow.
#[derive(Clone, Debug)]
struct Drag {
    right: bool,
    /// The stack in hand at the press and its count since, as the drag's
    /// own clicks change it (a server answers them later).
    held: Option<ItemStack>,
    /// The drag replaced vanilla's: its release clicks nothing.
    active: bool,
    visited: Vec<usize>,
}

impl Game<'_> {
    /// The open screen is one the tweaks run in, with the extra on.
    pub(super) fn tweaks_on(&self) -> bool {
        if !self.settings.extras.mouse_tweaks
            || self.screen != Screen::Inventory
            || self.station.is_some()
            || self.mount_active()
        {
            return false;
        }
        if self.remote.is_some() {
            self.remote_menu_type().is_none_or(tweak_menu)
        } else {
            matches!(self.container_kind(), None | Some(BlockEntity::Chest(_)))
        }
    }
    /// One ordinary click on a game slot. False when nothing took it.
    pub(super) fn tweak_click(&mut self, slot: usize, button: i32, input: Input) -> bool {
        if self.remote.is_some() {
            return self.remote_menu_ops(&[(Some(slot), button, input)]);
        }
        if !self.menu_click(Some(slot), button, input) {
            return false;
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
        true
    }
    /// Puts the hand's stack (`right`: one of it) in `slot`, as a one-slot
    /// drag (`QUICK_CRAFT`) and not a click: a server that has the hand empty
    /// (the stack it was to come from just burned as fuel) ignores a drag, but
    /// a click would pick up what's in `slot`.
    fn tweak_put(&mut self, slot: usize, right: bool) -> bool {
        let kind = i32::from(right) << 2;
        let ops = [
            (None, kind, Input::QuickCraft),
            (Some(slot), kind | 1, Input::QuickCraft),
            (None, kind | 2, Input::QuickCraft),
        ];
        if self.remote.is_some() {
            return self.remote_menu_ops(&ops);
        }
        if !self.menu_ops(&ops) {
            return false;
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
        true
    }
    /// A press on `slot`: true when the tweaks took it (a right-drag
    /// placing), else vanilla's press runs and the tweaks watch the drag.
    pub(super) fn tweak_press(&mut self, slot: Option<usize>, right: bool, shift: bool) -> bool {
        self.tweaks.drag = None;
        let Some(slot) = slot.filter(|&s| s != 200 && s < Creative::DISPLAY) else {
            return false;
        };
        if !self.tweaks_on() {
            return false;
        }
        let held = self.carried.clone();
        if right {
            // A result slot takes nothing: its press stays vanilla's.
            if shift || held.is_none() || self.tweak_result(slot) {
                return false;
            }
            // RMB: the press places at once, then the drag goes on.
            let mut drag = Drag {
                right,
                held,
                active: true,
                visited: vec![slot],
            };
            self.tweak_place(&mut drag, slot);
            self.inv_drag = None;
            self.inv_clicks.skip_release();
            self.tweaks.drag = Some(drag);
            return true;
        }
        if held.is_some() || shift {
            self.tweaks.drag = Some(Drag {
                right,
                held,
                active: false,
                visited: vec![slot],
            });
        }
        false
    }
    /// Places one held item in `slot` when it takes it.
    fn tweak_place(&mut self, drag: &mut Drag, slot: usize) {
        let Some(held) = drag.held.clone() else {
            return;
        };
        if self.tweak_takes_one(slot, &held) && self.tweak_put(slot, true) {
            drag.held = (held.count > 1).then(|| ItemStack {
                count: held.count - 1,
                ..held
            });
        }
    }
    /// The cursor moved while a button is held: true when the tweaks own
    /// the drag (vanilla's quick craft drag doesn't grow).
    pub(super) fn tweak_drag_move(&mut self, slot: Option<usize>) -> bool {
        let Some(mut drag) = self.tweaks.drag.take() else {
            return false;
        };
        let active = drag.active;
        let entered =
            slot.filter(|s| *s != 200 && *s < Creative::DISPLAY && !drag.visited.contains(s));
        if let Some(slot) = entered {
            drag.visited.push(slot);
            let stack = self.slot_view(slot);
            let shift = self.has_shift_down();
            if drag.right {
                self.tweak_place(&mut drag, slot);
            } else if let Some(held) = drag.held.clone() {
                // Holding a stack: only slots of the same item, and only
                // while vanilla's drag hasn't spread.
                let spread = self.inv_drag.as_ref().is_some_and(|(_, s)| s.len() > 1);
                if let Some(s) = stack.filter(|s| s.same_item_same_components(&held))
                    && (drag.active || !spread)
                {
                    if !drag.active {
                        drag.active = true;
                        self.inv_drag = None;
                        self.inv_clicks.skip_release();
                    }
                    if shift {
                        self.tweak_click(slot, 0, Input::QuickMove);
                    } else if held.count + s.count <= self.max_stack(&s)
                        && self.tweak_click(slot, 0, Input::Pickup)
                        && self.tweak_click(slot, 0, Input::Pickup)
                    {
                        // Put the hand's stack in the slot, then take it all.
                        drag.held = Some(ItemStack {
                            count: held.count + s.count,
                            ..held
                        });
                    }
                }
            } else if shift && stack.is_some() {
                drag.active = true;
                self.tweak_click(slot, 0, Input::QuickMove);
            }
        }
        let owned = drag.active || active;
        self.tweaks.drag = Some(drag);
        owned
    }
    /// The button went up: the drag ends.
    pub(super) fn tweak_release(&mut self) {
        self.tweaks.drag = None;
    }
    /// The wheel over an inventory screen: true when the tweaks used it.
    pub(super) fn tweak_scroll(&mut self, delta: f32, slot: Option<usize>) -> bool {
        if !self.settings.extras.mouse_tweaks_wheel || !self.tweaks_on() || self.carried.is_some() {
            return false;
        }
        let Some(slot) = slot else {
            self.tweaks.wheel = 0.0;
            return false;
        };
        self.tweaks.wheel += delta;
        let steps = self.tweaks.wheel.trunc();
        self.tweaks.wheel -= steps;
        for _ in 0..steps.abs() as u32 {
            if steps < 0.0 {
                self.tweak_push(slot);
            } else {
                self.tweak_pull(slot);
            }
        }
        true
    }
    /// `slot` is a result slot of the open menu.
    fn tweak_result(&mut self, slot: usize) -> bool {
        let mut result = false;
        self.menu_session(|menu, _, to_menu| {
            result = to_menu(slot).is_some_and(|i| menu.is_result(i));
            Some(false)
        });
        result
    }
    /// `slot` takes one more of `stack`, by the open menu's rules.
    fn tweak_takes_one(&mut self, slot: usize, stack: &ItemStack) -> bool {
        let mut takes = false;
        self.menu_session(|menu, env, to_menu| {
            takes = to_menu(slot).is_some_and(|i| menu.takes_one(env, i, stack));
            Some(false)
        });
        takes
    }
    /// Where vanilla's quick move would put `slot`'s items, tried on a copy
    /// of the menu that's dropped (`Some(false)` keeps nothing).
    fn tweak_landing(&mut self, slot: usize) -> Vec<usize> {
        let slots = self.slots();
        let mut landing = Vec::new();
        self.menu_session(|menu, env, to_menu| {
            let from = to_menu(slot)?;
            landing = menu
                .quick_move_landing(env, from)
                .into_iter()
                .filter_map(|i| slots.iter().copied().find(|&g| to_menu(g) == Some(i)))
                .collect();
            Some(false)
        });
        landing
    }
    /// The slots across from `slot`: a container's or crafting grid's face
    /// the inventory (hotbar first, from the right), the inventory's face
    /// them; without one, hotbar and main inventory trade.
    fn other_side(&self, slot: usize) -> Vec<usize> {
        let table = self.craft_size == 3;
        let own: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s) || (table && (100..109).contains(s)))
            .collect();
        let player = || (0..9).rev().chain((9..36).rev());
        match slot {
            100..109 | 300..500 => player().collect(),
            0..36 if !own.is_empty() => own,
            0..9 => (9..36).collect(),
            9..36 => (0..9).collect(),
            _ => Vec::new(),
        }
    }
    /// Wheel down: one item of `slot` to where a quick move sends it. A
    /// result can't be split or put back: one craft (or the furnace's whole
    /// output) goes, when it all fits.
    fn tweak_push(&mut self, slot: usize) {
        let Some(stack) = self.slot_view(slot) else {
            return;
        };
        let landing = self.tweak_landing(slot);
        let Some(&target) = landing.first() else {
            return;
        };
        if self.tweak_result(slot) {
            if self.tweak_click(slot, 0, Input::Pickup) {
                for target in landing {
                    self.tweak_put(target, false);
                }
            }
            return;
        }
        if self.tweak_click(slot, 0, Input::Pickup) {
            self.tweak_put(target, true);
            if stack.count > 1 {
                self.tweak_put(slot, false);
            }
        }
    }
    /// Wheel up: one item of the same kind from the other side into `slot`.
    /// A result slot takes none and gives none.
    fn tweak_pull(&mut self, slot: usize) {
        let Some(stack) = self.slot_view(slot) else {
            return;
        };
        if !self.tweak_takes_one(slot, &stack) {
            return;
        }
        let mut source = None;
        for s in self.other_side(slot) {
            if let Some(v) = self.slot_view(s)
                && v.same_item_same_components(&stack)
                && !self.tweak_result(s)
            {
                source = Some((s, v.count));
                break;
            }
        }
        let Some((source, count)) = source else {
            return;
        };
        if self.tweak_click(source, 0, Input::Pickup) {
            self.tweak_put(slot, true);
            if count > 1 {
                self.tweak_put(source, false);
            }
        }
    }
}

impl Game<'_> {
    /// The sort buttons of the open screen, with the extra on: their
    /// corner (from the screen's) and the slots each sorts. The container's
    /// (chest-like ones only) sits by its title, the main inventory's by
    /// the "Inventory" label's line.
    fn sort_buttons(&self) -> Vec<((f32, f32), Vec<usize>)> {
        if !self.settings.extras.sort
            || self.screen != Screen::Inventory
            || self.station.is_some()
            || self.mount_active()
        {
            return Vec::new();
        }
        let chest_like = if self.remote.is_some() {
            if !self.remote_storage_open() {
                return Vec::new();
            }
            true
        } else {
            match self.container_kind() {
                None => false,
                Some(BlockEntity::Chest(_)) => {
                    self.chest_rows().is_some() || self.container_block_is_shulker()
                }
                Some(_) => return Vec::new(),
            }
        };
        let mut buttons = Vec::new();
        let container: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if chest_like && !container.is_empty() {
            buttons.push(((160.0, 5.0), container));
        }
        buttons.push(((160.0, self.screen_height() - 95.0), (9..36).collect()));
        buttons
    }
    /// The sort buttons: a bordered square with three bars, lighter under
    /// the cursor.
    pub(super) fn draw_sort_buttons(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let atlas = &self.gui.atlas;
        for ((x, y), _) in self.sort_buttons() {
            let (x, y) = (ox + x, oy + y);
            let (cx, cy) = self.cursor;
            let hovered = cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON;
            let face = if hovered {
                rgba(220, 220, 220, 255)
            } else {
                rgba(160, 160, 160, 255)
            };
            batch.fill(atlas, x, y, SORT_BUTTON, SORT_BUTTON, rgba(55, 55, 55, 255));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                SORT_BUTTON - 2.0,
                SORT_BUTTON - 2.0,
                face,
            );
            for (i, w) in [5.0, 4.0, 3.0].into_iter().enumerate() {
                let by = y + 2.0 + 2.0 * i as f32;
                batch.fill(atlas, x + 2.0, by, w, 1.0, rgba(55, 55, 55, 255));
            }
        }
    }
    /// A left press on a sort button sorts its slots (true).
    pub(super) fn sort_press(&mut self) -> bool {
        let Some(r) = self.renderer.as_ref() else {
            return false;
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        let (ox, oy) = self.inventory_origin(&batch);
        let (cx, cy) = self.cursor;
        let hit = self.sort_buttons().into_iter().find(|((x, y), _)| {
            let (x, y) = (ox + x, oy + y);
            cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON
        });
        let Some((_, slots)) = hit else {
            return false;
        };
        self.sort_slots(&slots);
        true
    }
    /// The sort key: sorts the part under the cursor (the container when
    /// the cursor is over no slot).
    pub(super) fn sort_key_press(&mut self, key: KeyCode) -> bool {
        keybinds::key_name(key).is_some_and(|name| self.sort_input(name))
    }
    pub(super) fn sort_mouse_press(&mut self, button: MouseButton) -> bool {
        self.sort_input(&keybinds::mouse_name(button))
    }
    /// The HDR settings key (`Graphics::hdr_key`): opens Graphics: HDR from the game, where
    /// the display offers HDR; Done goes back to the game.
    pub(super) fn hdr_key_press(&mut self, key: KeyCode) -> bool {
        keybinds::key_name(key).is_some_and(|name| self.hdr_input(name))
    }
    pub(super) fn hdr_mouse_press(&mut self, button: MouseButton) -> bool {
        self.hdr_input(&keybinds::mouse_name(button))
    }
    fn hdr_input(&mut self, name: &str) -> bool {
        if self.screen != Screen::Playing
            || self.title.is_some()
            || self.settings.horizon.hdr_key != name
            || !horizon_client::graphics::hdr_offered()
        {
            return false;
        }
        self.capture(false);
        self.open_options_at(false, horizon_client::options_ui::Page::GraphicsHdr);
        self.options_to_game = true;
        true
    }
    fn sort_input(&mut self, name: &str) -> bool {
        let e = &self.settings.extras;
        if !e.sort || e.sort_key != name || self.recipe_search_focused() {
            return false;
        }
        let buttons = self.sort_buttons();
        let hovered = self.inventory_target().0;
        let slots = buttons
            .iter()
            .find(|(_, slots)| hovered.is_some_and(|h| slots.contains(&h)))
            .or(buttons.first())
            .map(|(_, slots)| slots.clone());
        let Some(slots) = slots else {
            return false;
        };
        self.sort_slots(&slots);
        true
    }
    /// Sorts `slots` with ordinary left clicks (needs an empty hand).
    fn sort_slots(&mut self, slots: &[usize]) {
        if self.carried.is_some() {
            return;
        }
        let views: Vec<Option<ItemStack>> = slots.iter().map(|&s| self.slot_view(s)).collect();
        let clicks = {
            let names = |id: &str| self.pack.names.display_item(id);
            let rank = |s: &ItemStack| {
                let order = self.creative_screen.order_of(s).unwrap_or(usize::MAX);
                (order, hover_name(&names, s))
            };
            let max = |s: &ItemStack| self.max_stack(s);
            inv_sort::plan(&views, &rank, &max)
        };
        for i in clicks {
            self.tweak_click(slots[i], 0, Input::Pickup);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tweak_menu;

    #[test]
    fn the_tweaks_run_in_the_menus_with_plain_click_rules() {
        for menu in [
            "inventory",
            "minecraft:generic_9x1",
            "minecraft:generic_9x6",
            "minecraft:shulker_box",
            "minecraft:hopper",
            "minecraft:generic_3x3",
            "minecraft:furnace",
            "minecraft:blast_furnace",
            "minecraft:smoker",
            "minecraft:crafting",
        ] {
            assert!(tweak_menu(menu), "{menu}");
        }
        for menu in [
            "minecraft:anvil",
            "minecraft:enchantment",
            "minecraft:merchant",
            "minecraft:beacon",
            "minecraft:grindstone",
            "minecraft:stonecutter",
            "minecraft:loom",
            "minecraft:brewing_stand",
            "minecraft:crafter_3x3",
        ] {
            assert!(!tweak_menu(menu), "{menu}");
        }
    }
}
