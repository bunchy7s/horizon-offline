//! OS modifier reconciliation and modifiers synthesized by agent control.

use crate::{
    keybinds::{KeyBinds, key_code},
    keymapping::{KeyAction, SNEAK, SPRINT, ToggleKeys},
};
use std::collections::HashSet;
use winit::keyboard::{KeyCode, ModifiersState};

const MODIFIERS: [(ModifiersState, [KeyCode; 2]); 4] = [
    (
        ModifiersState::SHIFT,
        [KeyCode::ShiftLeft, KeyCode::ShiftRight],
    ),
    (
        ModifiersState::CONTROL,
        [KeyCode::ControlLeft, KeyCode::ControlRight],
    ),
    (ModifiersState::ALT, [KeyCode::AltLeft, KeyCode::AltRight]),
    (
        ModifiersState::SUPER,
        [KeyCode::SuperLeft, KeyCode::SuperRight],
    ),
];

/// A synthesized key updates the same modifier word as window events.
/// Releasing one side must preserve the other side's hold.
pub fn set_key(
    modifiers: &mut ModifiersState,
    physical: &mut HashSet<KeyCode>,
    key: KeyCode,
    down: bool,
) {
    let Some((bit, pair)) = MODIFIERS.iter().find(|(_, pair)| pair.contains(&key)) else {
        return;
    };
    if down {
        physical.insert(key);
    } else {
        physical.remove(&key);
    }
    modifiers.set(*bit, pair.iter().any(|key| physical.contains(key)));
}

/// A modifier held for one synthesized click (the pad's Y, an agent's
/// shift-click): what [`restore`] puts back once the click is done.
#[derive(Clone, Copy, Debug)]
pub struct Held {
    key: KeyCode,
    was_down: bool,
    bit: bool,
}

/// Holds `key` in both the physical keys and the modifier word, which is
/// what screens read (`InputWithModifiers.hasShiftDown`).
pub fn hold(modifiers: &mut ModifiersState, physical: &mut HashSet<KeyCode>, key: KeyCode) -> Held {
    let bit = MODIFIERS
        .iter()
        .find(|(_, pair)| pair.contains(&key))
        .is_some_and(|(bit, _)| modifiers.contains(*bit));
    let held = Held {
        key,
        was_down: physical.contains(&key),
        bit,
    };
    set_key(modifiers, physical, key, true);
    held
}

/// Undoes [`hold`]: a key or bit that was already down stays down.
pub fn restore(modifiers: &mut ModifiersState, physical: &mut HashSet<KeyCode>, held: Held) {
    if !held.was_down {
        physical.remove(&held.key);
    }
    if let Some((bit, _)) = MODIFIERS.iter().find(|(_, pair)| pair.contains(&held.key)) {
        modifiers.set(*bit, held.bit);
    }
}

/// Recover missing modifier releases without releasing a rebound key or a pad.
/// ToggleKeyMapping itself preserves keyboard and controller toggles on release.
pub fn reconcile(
    modifiers: ModifiersState,
    physical: &mut HashSet<KeyCode>,
    bindings: &KeyBinds,
    toggles: &mut ToggleKeys,
    options: [bool; 2],
    pad_held: [bool; 2],
) {
    for (bit, pair) in MODIFIERS {
        if modifiers.contains(bit) {
            continue;
        }
        for key in pair {
            if !physical.remove(&key) {
                continue;
            }
            for (slot, name) in [(SNEAK, "key.sneak"), (SPRINT, "key.sprint")] {
                if key_code(bindings.get(name)) == Some(key) && !pad_held[slot] {
                    toggles.key(slot, KeyAction::Release, false, options[slot]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_taps_release_both_bits() {
        let mut modifiers = ModifiersState::empty();
        let mut physical = HashSet::new();
        for (bit, pair) in MODIFIERS {
            for key in pair {
                set_key(&mut modifiers, &mut physical, key, true);
                assert!(modifiers.contains(bit));
                set_key(&mut modifiers, &mut physical, key, false);
                assert!(modifiers.is_empty());
                assert!(physical.is_empty());
            }
        }
    }

    #[test]
    fn releasing_one_side_keeps_the_other_modifier_held() {
        let mut modifiers = ModifiersState::empty();
        let mut physical = HashSet::new();
        for (bit, [left, right]) in MODIFIERS {
            set_key(&mut modifiers, &mut physical, left, true);
            set_key(&mut modifiers, &mut physical, right, true);
            set_key(&mut modifiers, &mut physical, left, false);
            assert!(modifiers.contains(bit));
            set_key(&mut modifiers, &mut physical, right, false);
            assert!(modifiers.is_empty());
        }
    }

    #[test]
    fn a_held_click_restores_the_previous_state() {
        let mut modifiers = ModifiersState::empty();
        let mut physical = HashSet::new();
        let held = hold(&mut modifiers, &mut physical, KeyCode::ShiftLeft);
        assert!(modifiers.shift_key());
        restore(&mut modifiers, &mut physical, held);
        assert!(modifiers.is_empty());
        assert!(physical.is_empty());

        // Right Shift already held, or the OS word set with no physical key.
        for start in [HashSet::from([KeyCode::ShiftRight]), HashSet::new()] {
            let mut modifiers = ModifiersState::SHIFT;
            let mut physical = start.clone();
            let held = hold(&mut modifiers, &mut physical, KeyCode::ShiftLeft);
            restore(&mut modifiers, &mut physical, held);
            assert_eq!(modifiers, ModifiersState::SHIFT);
            assert_eq!(physical, start);
        }
    }

    #[test]
    fn lost_releases_respect_rebindings_and_pad_holds() {
        for slot in [SNEAK, SPRINT] {
            for rebound in [false, true] {
                for pad in [false, true] {
                    for toggle in [false, true] {
                        let mut bindings = KeyBinds::default();
                        let (name, modifier) = if slot == SNEAK {
                            ("key.sneak", KeyCode::ShiftLeft)
                        } else {
                            ("key.sprint", KeyCode::ControlLeft)
                        };
                        if rebound {
                            bindings.set(name, "key.keyboard.c");
                        }
                        let mut physical = HashSet::from([modifier]);
                        if rebound {
                            physical.insert(KeyCode::KeyC);
                        }
                        let mut toggles = ToggleKeys::default();
                        toggles.key(slot, KeyAction::Press, false, toggle);
                        let mut options = [false; 2];
                        options[slot] = toggle;
                        let mut pad_held = [false; 2];
                        pad_held[slot] = pad;
                        reconcile(
                            ModifiersState::ALT,
                            &mut physical,
                            &bindings,
                            &mut toggles,
                            options,
                            pad_held,
                        );
                        assert_eq!(toggles.is_down(slot), rebound || pad || toggle);
                        assert!(!physical.contains(&modifier));
                        assert_eq!(physical.contains(&KeyCode::KeyC), rebound);
                    }
                }
            }
        }
    }

    #[test]
    fn unrelated_modifier_change_does_not_release_an_action() {
        let mut toggles = ToggleKeys::default();
        toggles.key(SNEAK, KeyAction::Press, false, false);
        reconcile(
            ModifiersState::ALT,
            &mut HashSet::new(),
            &KeyBinds::default(),
            &mut toggles,
            [false; 2],
            [false; 2],
        );
        assert!(toggles.is_down(SNEAK));
    }

    #[test]
    fn lost_modifier_release_preserves_a_controller_toggle() {
        let mut toggles = ToggleKeys::default();
        toggles.pad_toggle(SNEAK, false);
        reconcile(
            ModifiersState::empty(),
            &mut HashSet::from([KeyCode::ShiftLeft]),
            &KeyBinds::default(),
            &mut toggles,
            [false; 2],
            [false; 2],
        );
        assert!(toggles.is_down(SNEAK));
    }
}
