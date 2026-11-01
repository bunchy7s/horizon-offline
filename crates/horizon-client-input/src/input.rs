//! KeyboardInput direction and LocalPlayer input-speed arithmetic for 26.3.
//! Item-use and sneak multipliers come from the player's current state; this
//! module does not implement sprint eligibility, poses or the input lifecycle.

use std::collections::HashSet;
use winit::keyboard::{KeyCode, ModifiersState};

fn length(v: [f32; 2]) -> f32 {
    f64::from(v[0] * v[0] + v[1] * v[1]).sqrt() as f32
}
fn scale(v: [f32; 2], factor: f32) -> [f32; 2] {
    [v[0] * factor, v[1] * factor]
}
pub fn keyboard_direction(forward: bool, backward: bool, left: bool, right: bool) -> [f32; 2] {
    let impulse = |positive: bool, negative: bool| {
        if positive == negative {
            0.0
        } else if positive {
            1.0
        } else {
            -1.0
        }
    };
    let v = [impulse(left, right), impulse(forward, backward)];
    let magnitude = length(v);
    if magnitude < 1.0e-4 {
        [0.0, 0.0]
    } else {
        [v[0] / magnitude, v[1] / magnitude]
    }
}

/// LocalPlayer.modifyInput with already selected multipliers. `item_use` is
/// Some only when using an item and not riding; `sneaking` only when moving
/// slowly. Zero input returns before either multiplier is applied.
pub fn modify_input(input: [f32; 2], item_use: Option<f32>, sneaking: Option<f32>) -> [f32; 2] {
    if input[0] * input[0] + input[1] * input[1] == 0.0 {
        return input;
    }
    let mut v = scale(input, 0.98);
    if let Some(multiplier) = item_use {
        v = scale(v, multiplier);
    }
    if let Some(multiplier) = sneaking {
        v = scale(v, multiplier);
    }
    square_movement(v)
}

fn square_movement(v: [f32; 2]) -> [f32; 2] {
    let magnitude = length(v);
    if magnitude <= 0.0 {
        return v;
    }
    let unit = scale(v, 1.0 / magnitude);
    let x = unit[0].abs();
    let y = unit[1].abs();
    let ratio = if y > x { x / y } else { y / x };
    let distance = f64::from(1.0 + ratio * ratio).sqrt() as f32;
    let speed = magnitude * distance;
    // Java Math.min propagates NaN, unlike Rust's f32::min.
    scale(
        unit,
        if speed.is_nan() || speed < 1.0 {
            speed
        } else {
            1.0
        },
    )
}

/// `InputQuirks.isQuitShortcutDown`: checks whether the platform's window-closing
/// keyboard shortcut is held.
/// - On macOS: Command (`super_key()`) + `Q`.
/// - On Windows / Linux / other: Alt (`alt_key()`) + `F4`.
pub fn is_quit_shortcut_down(modifiers: ModifiersState, keys: &HashSet<KeyCode>) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.super_key() && keys.contains(&KeyCode::KeyQ)
    } else {
        modifiers.alt_key() && keys.contains(&KeyCode::F4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_shortcut_follows_platform_modifiers_and_keys() {
        let mut keys = HashSet::new();
        let alt = ModifiersState::ALT;
        let super_key = ModifiersState::SUPER;

        assert!(!is_quit_shortcut_down(ModifiersState::empty(), &keys));

        keys.insert(KeyCode::F4);
        if cfg!(target_os = "macos") {
            assert!(!is_quit_shortcut_down(alt, &keys));
            keys.remove(&KeyCode::F4);
            keys.insert(KeyCode::KeyQ);
            assert!(is_quit_shortcut_down(super_key, &keys));
            assert!(!is_quit_shortcut_down(alt, &keys));
        } else {
            assert!(is_quit_shortcut_down(alt, &keys));
            assert!(!is_quit_shortcut_down(super_key, &keys));
            keys.remove(&KeyCode::F4);
            keys.insert(KeyCode::KeyQ);
            assert!(!is_quit_shortcut_down(alt, &keys));
            assert!(!is_quit_shortcut_down(super_key, &keys));
        }
    }
}
