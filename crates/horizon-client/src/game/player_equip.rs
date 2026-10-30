//! `LivingEntity.onEquipItem` for players: the equip sound and the equip
//! game event when an armour or off-hand slot takes a different item, from
//! every path that calls it (use, the inventory menu, the creative screen,
//! the swap-hands key, dispensers), for the host and for LAN guests.
use super::Game;
use horizon_client::items::ItemStack;

/// The equipment slots `onEquipItem` can act on, as `EquipmentSlot` names:
/// the armour (`Player.doesEmitEquipEvent`: these emit the game event) and
/// the off hand.
const ARMOR: [&str; 4] = ["head", "chest", "legs", "feet"];
const OFFHAND: &str = "offhand";

/// `ItemStack.isSameItemSameComponents` over optional stacks.
fn same(a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_item_same_components(b),
        _ => false,
    }
}

impl Game<'_> {
    /// The armour (head first) and the off hand, in `EquipmentSlot` order of
    /// the inventory menu's slots.
    pub(super) fn player_equipment(&self) -> [Option<ItemStack>; 5] {
        let [head, chest, legs, feet] = self.armor.clone();
        [head, chest, legs, feet, self.offhand.clone()]
    }

    /// `onEquipItem` for every slot that changed since `before`: what the
    /// server does for the creative screen, which sends each changed slot
    /// (`handleSetCreativeModeSlot` sets it by player).
    pub(super) fn player_equipment_changed(&mut self, before: &[Option<ItemStack>; 5]) {
        let after = self.player_equipment();
        for (i, slot) in ARMOR.into_iter().chain([OFFHAND]).enumerate() {
            self.player_on_equip(slot, before[i].as_ref(), after[i].as_ref());
        }
    }

    /// `LivingEntity.onEquipItem(slot, old, new)` for the player in place
    /// (the host, or a guest inside `as_guest`).
    pub(super) fn player_on_equip(
        &mut self,
        slot: &str,
        old: Option<&ItemStack>,
        new: Option<&ItemStack>,
    ) {
        if !same(old, new) {
            self.player_equipped(slot, new);
        }
    }

    /// The body of `onEquipItem` once the item changed: the item's
    /// `Equippable.equipSound` when it is equipped in its own slot, and the
    /// `equip` / `unequip` game event for armour. Nothing on a client (the
    /// server plays it) or for a spectator. The player's `firstTick` never
    /// sees an equip here: loading a save sets the slots directly.
    pub(super) fn player_equipped(&mut self, slot: &str, new: Option<&ItemStack>) {
        if self.remote.is_some() || self.spectator() {
            return;
        }
        let Some(at) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let at = [at.x, at.y, at.z];
        let equippable = new
            .and_then(|s| self.items().get(&s.item))
            .and_then(|info| {
                let own = info.equip_slot.as_deref()?;
                Some((own == slot, info.equip_sound.clone()))
            });
        if let Some((true, sound)) = &equippable {
            let sound = sound
                .as_deref()
                .unwrap_or("minecraft:item.armor.equip_generic");
            // Player.getSoundSource: PLAYERS.
            let players = horizon_client::settings::SOUND_SOURCES
                .iter()
                .position(|s| *s == "player")
                .unwrap_or(0);
            self.audio.play_on(players, sound, Some(at), 1.0, 1.0);
        }
        if ARMOR.contains(&slot) {
            let event = if equippable.is_some() {
                "minecraft:equip"
            } else {
                "minecraft:unequip"
            };
            let ctx = self.player_context(None);
            self.post_game_event(event, at, ctx);
        }
    }
}
