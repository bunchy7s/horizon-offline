//! The down/click state of `KeyMapping` and `ToggleKeyMapping` (sneak,
//! sprint, attack and use can be toggled instead of held).

/// One mapping's runtime state. `toggle` is `Some(restore)` for a
/// `ToggleKeyMapping` (with its `shouldRestore` flag), `None` for a plain one.
#[derive(Clone, Debug)]
pub struct KeyState {
    pub is_down: bool,
    click_count: i32,
    released_by_screen_when_down: bool,
    toggle: Option<bool>,
    keyboard: bool,
    /// Held down by a controller's toggle (Horizon's pad toggle-sneak, which
    /// vanilla has no option for): until it lets go, the mapping toggles as
    /// if its vanilla option were on, so a screen releases and restores it.
    pad_toggled: bool,
}

impl KeyState {
    pub fn plain() -> Self {
        Self {
            is_down: false,
            click_count: 0,
            released_by_screen_when_down: false,
            toggle: None,
            keyboard: true,
            pad_toggled: false,
        }
    }
    /// A `ToggleKeyMapping`; `keyboard` is whether the bound key is a keyboard key.
    pub fn toggle(should_restore: bool, keyboard: bool) -> Self {
        Self {
            toggle: Some(should_restore),
            keyboard,
            ..Self::plain()
        }
    }
    /// `KeyMapping.click`: one more pending click.
    pub fn click(&mut self) {
        self.click_count += 1;
    }
    /// `consumeClick`.
    pub fn consume_click(&mut self) -> bool {
        if self.click_count == 0 {
            false
        } else {
            self.click_count -= 1;
            true
        }
    }
    /// The mapping toggles: its option is on, or a pad toggled it down.
    fn needs_toggle(&self, option: bool) -> bool {
        option || self.pad_toggled
    }
    /// `setDown`; `needs_toggle` is the mapping's option value.
    pub fn set_down(&mut self, down: bool, needs_toggle: bool) {
        if self.toggle.is_some() && self.needs_toggle(needs_toggle) {
            if down {
                self.is_down = !self.is_down;
                // Toggled back up: the pad's toggle is over.
                self.pad_toggled &= self.is_down;
            }
        } else {
            self.is_down = down;
        }
    }
    /// A controller's toggle button: flips the mapping like `setDown(true)`
    /// with its option on, whatever the option says.
    pub fn pad_toggle(&mut self) {
        self.is_down = !self.is_down;
        self.pad_toggled = self.is_down;
    }
    /// `shouldSetOnIngameFocus` for a keyboard-bound, bound key.
    pub fn should_set_on_ingame_focus(&self, needs_toggle: bool) -> bool {
        self.keyboard && (self.toggle.is_none() || !needs_toggle)
    }
    /// `release` (a screen opened): drops pending clicks and the held state.
    pub fn release(&mut self, needs_toggle: bool) {
        if self.toggle.is_some() {
            // ToggleKeyMapping.release does not clear pending clicks.
            if self.needs_toggle(needs_toggle) && self.is_down || self.released_by_screen_when_down
            {
                self.released_by_screen_when_down = true;
            }
            self.reset();
        } else {
            self.click_count = 0;
            self.set_down(false, needs_toggle);
        }
    }
    /// `ToggleKeyMapping.reset`. A pad toggle stays on record, so the screen
    /// that released it restores it.
    pub fn reset(&mut self) {
        self.is_down = false;
    }
    /// Clicks not yet consumed.
    pub fn pending_clicks(&self) -> i32 {
        self.click_count
    }
    /// `shouldRestoreStateOnScreenClosed` (clears the remembered release).
    pub fn should_restore_state_on_screen_closed(&mut self, needs_toggle: bool) -> bool {
        let Some(restore) = self.toggle else {
            return false;
        };
        let result = restore
            && self.needs_toggle(needs_toggle)
            && self.keyboard
            && self.released_by_screen_when_down;
        self.released_by_screen_when_down = false;
        result
    }
}

/// A keyboard event (`KeyEvent.Action`): SDL reports a held key's repeats
/// as their own action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    Press,
    Repeat,
}

/// Sneak.
pub const SNEAK: usize = 0;
/// Sprint.
pub const SPRINT: usize = 1;
/// Attack (mouse).
pub const ATTACK: usize = 2;

/// The `ToggleKeyMapping`s `Options` builds: sneak and sprint (keyboard,
/// restored after a screen), attack (mouse, restored) and use (mouse, not
/// restored), and the handlers that drive them. `needs_toggle` is each
/// mapping's toggle option, read at every event as vanilla does.
#[derive(Clone, Debug)]
pub struct ToggleKeys(pub [KeyState; 4]);
impl Default for ToggleKeys {
    fn default() -> Self {
        Self([
            KeyState::toggle(true, true),
            KeyState::toggle(true, true),
            KeyState::toggle(true, false),
            KeyState::toggle(false, false),
        ])
    }
}
impl ToggleKeys {
    pub fn is_down(&self, slot: usize) -> bool {
        self.0[slot].is_down
    }
    /// `KeyboardHandler.keyPress` for a key bound to `slot`: a release sets
    /// the mapping up (`KeyMapping.set(key, false)`); a press or a repeat
    /// with no screen open sets it down and clicks it.
    pub fn key(&mut self, slot: usize, action: KeyAction, screen_open: bool, needs_toggle: bool) {
        match action {
            KeyAction::Release => self.0[slot].set_down(false, needs_toggle),
            KeyAction::Press | KeyAction::Repeat if !screen_open => {
                self.0[slot].set_down(true, needs_toggle);
                self.0[slot].click();
            }
            _ => {}
        }
    }
    /// A controller's toggle button bound to `slot` (the pad's toggle-sneak):
    /// a press with no screen open flips the mapping and clicks it, as a
    /// key press does with the mapping's toggle option on.
    pub fn pad_toggle(&mut self, slot: usize, screen_open: bool) {
        if !screen_open {
            self.0[slot].pad_toggle();
            self.0[slot].click();
        }
    }
    /// `MouseHandler.onButton` for a button bound to `slot`: only without a
    /// screen or overlay.
    pub fn button(&mut self, slot: usize, pressed: bool, screen_open: bool, needs_toggle: bool) {
        if !screen_open {
            self.0[slot].set_down(pressed, needs_toggle);
            if pressed {
                self.0[slot].click();
            }
        }
    }
    /// `Gui.setScreen` with a screen: `KeyMapping.releaseAll`.
    pub fn screen_opened(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            key.release(option);
        }
    }
    /// `Gui.setScreen(null)` in a level:
    /// `KeyMapping.restoreToggleStatesOnScreenClosed`.
    pub fn screen_closed(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            if key.should_restore_state_on_screen_closed(option) {
                key.set_down(true, option);
            }
        }
    }
    /// `LocalPlayer.respawn`: `KeyMapping.resetToggleKeys`.
    pub fn respawn(&mut self) {
        for key in &mut self.0 {
            key.reset();
            // As with the vanilla option, the death screen closing after
            // this still restores a toggle it released.
            key.pad_toggled &= key.released_by_screen_when_down;
        }
    }
}

#[cfg(test)]
mod tests {
    /// `KeyMapping.resetToggleKeys` (a new `LocalPlayer`): a sneak and a sprint toggled on in one
    /// world are off in the next.
    #[test]
    fn resetting_the_toggle_keys_puts_every_mapping_up() {
        let mut keys = ToggleKeys::default();
        keys.key(0, KeyAction::Press, false, true);
        keys.key(1, KeyAction::Press, false, true);
        assert!(keys.is_down(0) && keys.is_down(1));
        for key in &mut keys.0 {
            key.reset();
        }
        assert!(!keys.is_down(0) && !keys.is_down(1));
    }

    use super::*;
    #[test]
    fn toggle_flips_on_press_and_ignores_release() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        assert!(k.is_down);
        k.set_down(false, true);
        assert!(k.is_down);
        k.set_down(true, true);
        assert!(!k.is_down);
    }
    #[test]
    fn screens_block_presses_but_not_releases() {
        let mut keys = ToggleKeys::default();
        keys.key(SNEAK, KeyAction::Press, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        assert!(keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Release, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SPRINT, KeyAction::Press, false, true);
        keys.respawn();
        assert!(!keys.is_down(SPRINT));
    }
    /// #3061: the pad's toggle-sneak (vanilla's toggle crouch off) comes
    /// back after a screen, as the vanilla option's toggle does; a
    /// keyboard shift-click in the inventory does not drop it.
    #[test]
    fn pad_toggle_is_restored_after_a_screen() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        assert!(keys.is_down(SNEAK));
        keys.screen_opened(off);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, true, false);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK), "the pad's toggle is restored");
        // Toggled back up, it stays up through the next screen.
        keys.pad_toggle(SNEAK, false);
        assert!(!keys.is_down(SNEAK));
        keys.screen_opened(off);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
        // A held key is not restored, as before.
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.screen_opened(off);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
    }
    /// After the pad's toggle is over (toggled up, or reset by a respawn
    /// with no screen to restore it), the shift key holds again.
    #[test]
    fn pad_toggle_ends_with_its_release() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        keys.respawn();
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.key(SNEAK, KeyAction::Release, false, false);
        assert!(!keys.is_down(SNEAK), "held, not toggled");
        // The death screen releases it and closes after the respawn: the
        // toggle comes back, as with the vanilla option.
        keys.pad_toggle(SNEAK, false);
        keys.screen_opened(off);
        keys.respawn();
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK));
        // A press with a screen open does nothing.
        keys.pad_toggle(SNEAK, true);
        assert!(keys.is_down(SNEAK));
    }
    #[test]
    fn screen_release_restores_toggled_state() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        k.release(true);
        assert!(!k.is_down);
        assert!(k.should_restore_state_on_screen_closed(true));
        assert!(!k.should_restore_state_on_screen_closed(true));
    }
}
