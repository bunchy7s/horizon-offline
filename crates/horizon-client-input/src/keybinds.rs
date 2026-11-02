//! Key mappings (Options.keyMappings): vanilla names, categories and
//! defaults, `key_<name>:key.keyboard.x` persistence, display names, and the
//! translation of rebound physical keys back to the default keys the game
//! logic listens for.
use std::collections::BTreeMap;
use winit::{event::MouseButton, keyboard::KeyCode};

/// (name, default key, category) in Options.keyMappings order.
pub const MAPPINGS: &[(&str, &str, &str)] = &[
    ("key.attack", "key.mouse.left", "gameplay"),
    ("key.use", "key.mouse.right", "gameplay"),
    ("key.forward", "key.keyboard.w", "movement"),
    ("key.left", "key.keyboard.a", "movement"),
    ("key.back", "key.keyboard.s", "movement"),
    ("key.right", "key.keyboard.d", "movement"),
    ("key.jump", "key.keyboard.space", "movement"),
    ("key.sneak", "key.keyboard.left.shift", "movement"),
    ("key.sprint", "key.keyboard.left.control", "movement"),
    ("key.drop", "key.keyboard.q", "inventory"),
    ("key.inventory", "key.keyboard.e", "inventory"),
    ("key.chat", "key.keyboard.t", "multiplayer"),
    ("key.playerlist", "key.keyboard.tab", "multiplayer"),
    ("key.pickItem", "key.mouse.middle", "gameplay"),
    ("key.command", "key.keyboard.slash", "multiplayer"),
    ("key.friends", "key.keyboard.o", "multiplayer"),
    ("key.socialInteractions", "key.keyboard.p", "multiplayer"),
    ("key.toggleGui", "key.keyboard.f1", "misc"),
    (
        "key.toggleSpectatorShaderEffects",
        "key.keyboard.f4",
        "misc",
    ),
    ("key.screenshot", "key.keyboard.f2", "misc"),
    ("key.togglePerspective", "key.keyboard.f5", "misc"),
    ("key.smoothCamera", "key.keyboard.unknown", "misc"),
    ("key.fullscreen", "key.keyboard.f11", "misc"),
    ("key.spectatorOutlines", "key.keyboard.unknown", "spectator"),
    ("key.spectatorHotbar", "key.mouse.middle", "spectator"),
    ("key.swapOffhand", "key.keyboard.f", "inventory"),
    ("key.saveToolbarActivator", "key.keyboard.c", "creative"),
    ("key.loadToolbarActivator", "key.keyboard.x", "creative"),
    ("key.advancements", "key.keyboard.l", "misc"),
    ("key.quickActions", "key.keyboard.g", "misc"),
    ("key.debug.overlay", "key.keyboard.f3", "debug"),
    ("key.debug.modifier", "key.keyboard.f3", "debug"),
    ("key.hotbar.1", "key.keyboard.1", "inventory"),
    ("key.hotbar.2", "key.keyboard.2", "inventory"),
    ("key.hotbar.3", "key.keyboard.3", "inventory"),
    ("key.hotbar.4", "key.keyboard.4", "inventory"),
    ("key.hotbar.5", "key.keyboard.5", "inventory"),
    ("key.hotbar.6", "key.keyboard.6", "inventory"),
    ("key.hotbar.7", "key.keyboard.7", "inventory"),
    ("key.hotbar.8", "key.keyboard.8", "inventory"),
    ("key.hotbar.9", "key.keyboard.9", "inventory"),
    ("key.debug.reloadChunk", "key.keyboard.a", "debug"),
    ("key.debug.showHitboxes", "key.keyboard.b", "debug"),
    ("key.debug.clearChat", "key.keyboard.d", "debug"),
    ("key.debug.crash", "key.keyboard.c", "debug"),
    ("key.debug.showChunkBorders", "key.keyboard.g", "debug"),
    ("key.debug.showAdvancedTooltips", "key.keyboard.h", "debug"),
    ("key.debug.copyRecreateCommand", "key.keyboard.i", "debug"),
    ("key.debug.spectate", "key.keyboard.n", "debug"),
    ("key.debug.switchGameMode", "key.keyboard.f4", "debug"),
    ("key.debug.debugOptions", "key.keyboard.f6", "debug"),
    ("key.debug.focusPause", "key.keyboard.p", "debug"),
    ("key.debug.dumpDynamicTextures", "key.keyboard.s", "debug"),
    ("key.debug.reloadResourcePacks", "key.keyboard.t", "debug"),
    ("key.debug.profiling", "key.keyboard.l", "debug"),
    ("key.debug.copyLocation", "key.keyboard.c", "debug"),
    ("key.debug.dumpVersion", "key.keyboard.v", "debug"),
    ("key.debug.profilingChart", "key.keyboard.1", "debug"),
    ("key.debug.fpsCharts", "key.keyboard.2", "debug"),
    ("key.debug.networkCharts", "key.keyboard.3", "debug"),
    ("key.debug.lightmapTexture", "key.keyboard.4", "debug"),
    ("key.debug.improvedTransparency", "key.keyboard.x", "debug"),
];
/// Mappings Options builds as `ToggleKeyMapping` (hold or toggle by option).
pub fn is_toggle(name: &str) -> bool {
    matches!(name, "key.sneak" | "key.sprint" | "key.use" | "key.attack")
}
/// KeyMapping.Category sort order.
pub const CATEGORIES: [&str; 8] = [
    "movement",
    "misc",
    "multiplayer",
    "gameplay",
    "inventory",
    "creative",
    "spectator",
    "debug",
];

/// InputConstants key names <-> winit physical keys.
const KEYS: &[(&str, KeyCode)] = &[
    ("key.keyboard.a", KeyCode::KeyA),
    ("key.keyboard.b", KeyCode::KeyB),
    ("key.keyboard.c", KeyCode::KeyC),
    ("key.keyboard.d", KeyCode::KeyD),
    ("key.keyboard.e", KeyCode::KeyE),
    ("key.keyboard.f", KeyCode::KeyF),
    ("key.keyboard.g", KeyCode::KeyG),
    ("key.keyboard.h", KeyCode::KeyH),
    ("key.keyboard.i", KeyCode::KeyI),
    ("key.keyboard.j", KeyCode::KeyJ),
    ("key.keyboard.k", KeyCode::KeyK),
    ("key.keyboard.l", KeyCode::KeyL),
    ("key.keyboard.m", KeyCode::KeyM),
    ("key.keyboard.n", KeyCode::KeyN),
    ("key.keyboard.o", KeyCode::KeyO),
    ("key.keyboard.p", KeyCode::KeyP),
    ("key.keyboard.q", KeyCode::KeyQ),
    ("key.keyboard.r", KeyCode::KeyR),
    ("key.keyboard.s", KeyCode::KeyS),
    ("key.keyboard.t", KeyCode::KeyT),
    ("key.keyboard.u", KeyCode::KeyU),
    ("key.keyboard.v", KeyCode::KeyV),
    ("key.keyboard.w", KeyCode::KeyW),
    ("key.keyboard.x", KeyCode::KeyX),
    ("key.keyboard.y", KeyCode::KeyY),
    ("key.keyboard.z", KeyCode::KeyZ),
    ("key.keyboard.1", KeyCode::Digit1),
    ("key.keyboard.2", KeyCode::Digit2),
    ("key.keyboard.3", KeyCode::Digit3),
    ("key.keyboard.4", KeyCode::Digit4),
    ("key.keyboard.5", KeyCode::Digit5),
    ("key.keyboard.6", KeyCode::Digit6),
    ("key.keyboard.7", KeyCode::Digit7),
    ("key.keyboard.8", KeyCode::Digit8),
    ("key.keyboard.9", KeyCode::Digit9),
    ("key.keyboard.0", KeyCode::Digit0),
    ("key.keyboard.enter", KeyCode::Enter),
    ("key.keyboard.escape", KeyCode::Escape),
    ("key.keyboard.backspace", KeyCode::Backspace),
    ("key.keyboard.tab", KeyCode::Tab),
    ("key.keyboard.space", KeyCode::Space),
    ("key.keyboard.minus", KeyCode::Minus),
    ("key.keyboard.equal", KeyCode::Equal),
    ("key.keyboard.left.bracket", KeyCode::BracketLeft),
    ("key.keyboard.right.bracket", KeyCode::BracketRight),
    ("key.keyboard.backslash", KeyCode::Backslash),
    ("key.keyboard.semicolon", KeyCode::Semicolon),
    ("key.keyboard.apostrophe", KeyCode::Quote),
    ("key.keyboard.grave.accent", KeyCode::Backquote),
    ("key.keyboard.comma", KeyCode::Comma),
    ("key.keyboard.period", KeyCode::Period),
    ("key.keyboard.slash", KeyCode::Slash),
    ("key.keyboard.caps.lock", KeyCode::CapsLock),
    ("key.keyboard.world.1", KeyCode::IntlBackslash),
    ("key.keyboard.international1", KeyCode::IntlRo),
    ("key.keyboard.international3", KeyCode::IntlYen),
    ("key.keyboard.lang1", KeyCode::Lang1),
    ("key.keyboard.lang2", KeyCode::Lang2),
    ("key.keyboard.lang3", KeyCode::Lang3),
    ("key.keyboard.lang4", KeyCode::Lang4),
    ("key.keyboard.lang5", KeyCode::Lang5),
    ("key.keyboard.help", KeyCode::Help),
    ("key.keyboard.select", KeyCode::Select),
    ("key.keyboard.again", KeyCode::Again),
    ("key.keyboard.undo", KeyCode::Undo),
    ("key.keyboard.cut", KeyCode::Cut),
    ("key.keyboard.copy", KeyCode::Copy),
    ("key.keyboard.paste", KeyCode::Paste),
    ("key.keyboard.find", KeyCode::Find),
    ("key.keyboard.f1", KeyCode::F1),
    ("key.keyboard.f2", KeyCode::F2),
    ("key.keyboard.f3", KeyCode::F3),
    ("key.keyboard.f4", KeyCode::F4),
    ("key.keyboard.f5", KeyCode::F5),
    ("key.keyboard.f6", KeyCode::F6),
    ("key.keyboard.f7", KeyCode::F7),
    ("key.keyboard.f8", KeyCode::F8),
    ("key.keyboard.f9", KeyCode::F9),
    ("key.keyboard.f10", KeyCode::F10),
    ("key.keyboard.f11", KeyCode::F11),
    ("key.keyboard.f12", KeyCode::F12),
    ("key.keyboard.f13", KeyCode::F13),
    ("key.keyboard.f14", KeyCode::F14),
    ("key.keyboard.f15", KeyCode::F15),
    ("key.keyboard.f16", KeyCode::F16),
    ("key.keyboard.f17", KeyCode::F17),
    ("key.keyboard.f18", KeyCode::F18),
    ("key.keyboard.f19", KeyCode::F19),
    ("key.keyboard.f20", KeyCode::F20),
    ("key.keyboard.f21", KeyCode::F21),
    ("key.keyboard.f22", KeyCode::F22),
    ("key.keyboard.f23", KeyCode::F23),
    ("key.keyboard.f24", KeyCode::F24),
    ("key.keyboard.print.screen", KeyCode::PrintScreen),
    ("key.keyboard.scroll.lock", KeyCode::ScrollLock),
    ("key.keyboard.pause", KeyCode::Pause),
    ("key.keyboard.insert", KeyCode::Insert),
    ("key.keyboard.home", KeyCode::Home),
    ("key.keyboard.page.up", KeyCode::PageUp),
    ("key.keyboard.delete", KeyCode::Delete),
    ("key.keyboard.end", KeyCode::End),
    ("key.keyboard.page.down", KeyCode::PageDown),
    ("key.keyboard.right", KeyCode::ArrowRight),
    ("key.keyboard.left", KeyCode::ArrowLeft),
    ("key.keyboard.down", KeyCode::ArrowDown),
    ("key.keyboard.up", KeyCode::ArrowUp),
    ("key.keyboard.num.lock", KeyCode::NumLock),
    ("key.keyboard.keypad.divide", KeyCode::NumpadDivide),
    ("key.keyboard.keypad.multiply", KeyCode::NumpadMultiply),
    ("key.keyboard.keypad.subtract", KeyCode::NumpadSubtract),
    ("key.keyboard.keypad.add", KeyCode::NumpadAdd),
    ("key.keyboard.keypad.enter", KeyCode::NumpadEnter),
    ("key.keyboard.keypad.1", KeyCode::Numpad1),
    ("key.keyboard.keypad.2", KeyCode::Numpad2),
    ("key.keyboard.keypad.3", KeyCode::Numpad3),
    ("key.keyboard.keypad.4", KeyCode::Numpad4),
    ("key.keyboard.keypad.5", KeyCode::Numpad5),
    ("key.keyboard.keypad.6", KeyCode::Numpad6),
    ("key.keyboard.keypad.7", KeyCode::Numpad7),
    ("key.keyboard.keypad.8", KeyCode::Numpad8),
    ("key.keyboard.keypad.9", KeyCode::Numpad9),
    ("key.keyboard.keypad.0", KeyCode::Numpad0),
    ("key.keyboard.keypad.period", KeyCode::NumpadDecimal),
    ("key.keyboard.keypad.equal", KeyCode::NumpadEqual),
    ("key.keyboard.keypad.comma", KeyCode::NumpadComma),
    ("key.keyboard.keypad.backspace", KeyCode::NumpadBackspace),
    (
        "key.keyboard.keypad.left.parenthesis",
        KeyCode::NumpadParenLeft,
    ),
    (
        "key.keyboard.keypad.right.parenthesis",
        KeyCode::NumpadParenRight,
    ),
    ("key.keyboard.keypad.hash", KeyCode::NumpadHash),
    (
        "key.keyboard.keypad.memory.store",
        KeyCode::NumpadMemoryStore,
    ),
    (
        "key.keyboard.keypad.memory.recall",
        KeyCode::NumpadMemoryRecall,
    ),
    (
        "key.keyboard.keypad.memory.clear",
        KeyCode::NumpadMemoryClear,
    ),
    ("key.keyboard.keypad.memory.add", KeyCode::NumpadMemoryAdd),
    (
        "key.keyboard.keypad.memory.subtract",
        KeyCode::NumpadMemorySubtract,
    ),
    ("key.keyboard.keypad.clear", KeyCode::NumpadClear),
    ("key.keyboard.keypad.clear.entry", KeyCode::NumpadClearEntry),
    ("key.keyboard.menu", KeyCode::ContextMenu),
    ("key.keyboard.left.control", KeyCode::ControlLeft),
    ("key.keyboard.left.shift", KeyCode::ShiftLeft),
    ("key.keyboard.left.alt", KeyCode::AltLeft),
    ("key.keyboard.left.win", KeyCode::SuperLeft),
    ("key.keyboard.right.control", KeyCode::ControlRight),
    ("key.keyboard.right.shift", KeyCode::ShiftRight),
    ("key.keyboard.right.alt", KeyCode::AltRight),
    ("key.keyboard.right.win", KeyCode::SuperRight),
];
pub fn key_code(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}
pub fn key_name(code: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(_, k)| *k == code).map(|(n, _)| *n)
}
pub fn mouse_name(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "key.mouse.left".into(),
        MouseButton::Right => "key.mouse.right".into(),
        MouseButton::Middle => "key.mouse.middle".into(),
        MouseButton::Back => "key.mouse.4".into(),
        MouseButton::Forward => "key.mouse.5".into(),
        MouseButton::Other(n) => format!("key.mouse.{}", n + 1),
    }
}
pub fn mouse_button(name: &str) -> Option<MouseButton> {
    Some(match name {
        "key.mouse.left" => MouseButton::Left,
        "key.mouse.right" => MouseButton::Right,
        "key.mouse.middle" => MouseButton::Middle,
        "key.mouse.4" => MouseButton::Back,
        "key.mouse.5" => MouseButton::Forward,
        _ => return None,
    })
}
pub use horizon_client_core::key_display::display_name;

/// A key event: what the layout writes on its key, without modifiers, names
/// that key from now on (`key_display::learn_layout`), so an AZERTY player's
/// Q key shows A.
pub fn learn_layout(event: &winit::event::KeyEvent) {
    use winit::keyboard::{Key, PhysicalKey};
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    let PhysicalKey::Code(code) = event.physical_key else {
        return;
    };
    let Some(name) = key_name(code) else {
        return;
    };
    let learn = horizon_client_core::key_display::learn_layout;
    match event.key_without_modifiers() {
        Key::Character(text) => learn(name, &text),
        Key::Dead(Some(c)) => learn(name, c.encode_utf8(&mut [0; 4])),
        _ => {}
    }
}

/// Current bindings by mapping name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinds {
    keys: BTreeMap<&'static str, String>,
}
impl Default for KeyBinds {
    fn default() -> Self {
        Self {
            keys: MAPPINGS
                .iter()
                .map(|(n, d, _)| (*n, (*d).to_owned()))
                .collect(),
        }
    }
}
/// Each default key once, in order: two mappings that share a default key
/// (Pick Block and the spectator hotbar both on the middle button) are one
/// press of it, not two.
fn distinct<T: PartialEq>(keys: Vec<T>) -> Vec<T> {
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

impl KeyBinds {
    pub fn get(&self, name: &str) -> &str {
        self.keys
            .get(name)
            .map_or("key.keyboard.unknown", String::as_str)
    }
    pub fn default_of(name: &str) -> &'static str {
        MAPPINGS
            .iter()
            .find(|m| m.0 == name)
            .map_or("key.keyboard.unknown", |m| m.1)
    }
    pub fn is_default(&self, name: &str) -> bool {
        self.get(name) == Self::default_of(name)
    }
    pub fn set(&mut self, name: &str, key: &str) {
        if let Some((n, _, _)) = MAPPINGS.iter().find(|m| m.0 == name) {
            self.keys.insert(n, key.to_owned());
        }
    }
    pub fn saved(&self) -> Vec<(&'static str, String)> {
        MAPPINGS
            .iter()
            .map(|(n, _, _)| (*n, self.get(n).to_owned()))
            .collect()
    }
    pub fn reset_all(&mut self) {
        *self = Self::default();
    }
    /// Mappings sharing `name`'s key (KeyMapping.same), for conflict marks.
    pub fn conflicts(&self, name: &str) -> Vec<&'static str> {
        let key = self.get(name);
        if key == "key.keyboard.unknown" {
            return Vec::new();
        }
        MAPPINGS
            .iter()
            .filter(|(n, _, _)| {
                *n != name && self.get(n) == key && (!self.is_default(n) || !self.is_default(name))
            })
            .map(|(n, _, _)| *n)
            .collect()
    }
    /// The mapping a physical input triggers (first in list order).
    pub fn mapping_for(&self, key: &str) -> Option<&'static str> {
        self.mappings_for(key).next()
    }
    /// Every mapping a physical input triggers, in list order: like
    /// `KeyMapping.set`/`click`, one key drives all the mappings bound to it.
    pub fn mappings_for<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'static str> + 'a {
        MAPPINGS
            .iter()
            .map(|m| m.0)
            .filter(move |n| self.get(n) == key)
    }
    /// The mappings bound to `key` that [`KeyBinds::translate_all`] follows:
    /// all but the debug keys. Those act only with F3 held
    /// (`KeyboardHandler.handleDebugKeys`, `debug_keys.rs` reads the physical
    /// key and the binding itself); their defaults (A, S, D, T, L, 1 to 4...)
    /// would otherwise turn the key back into its old gameplay meaning.
    fn translated_mappings<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'static str> + 'a {
        MAPPINGS
            .iter()
            .filter(|m| m.2 != "debug")
            .map(|m| m.0)
            .filter(move |n| self.get(n) == key)
    }
    /// Whether a mapping (other than a debug key) whose default is `key` was
    /// bound elsewhere.
    fn default_moved(&self, key: &str) -> bool {
        MAPPINGS
            .iter()
            .any(|(n, d, c)| *d == key && *c != "debug" && !self.is_default(n))
    }
    /// Translate a physical key into the default key of the mapping it is
    /// bound to, so game code written against vanilla defaults follows the
    /// player's bindings. `None` means the key now does nothing; mouse
    /// mappings bound to keys come back as `Err(button)`. The first of
    /// [`KeyBinds::translate_all`].
    pub fn translate(&self, key: KeyCode) -> Option<Result<KeyCode, MouseButton>> {
        self.translate_all(key).into_iter().next()
    }
    /// The default keys of every mapping `key` is bound to (empty when it
    /// now does nothing), so a key bound to two mappings (forward and
    /// sprint) drives both, as `KeyMapping.set` does.
    pub fn translate_all(&self, key: KeyCode) -> Vec<Result<KeyCode, MouseButton>> {
        let Some(name) = key_name(key) else {
            return vec![Ok(key)];
        };
        let bound: Vec<_> = self
            .translated_mappings(name)
            .filter_map(|mapping| {
                let default = Self::default_of(mapping);
                match key_code(default) {
                    Some(code) => Some(Ok(code)),
                    None => mouse_button(default).map(Err),
                }
            })
            .collect();
        if !bound.is_empty() {
            return distinct(bound);
        }
        // An unbound key whose default meaning was moved elsewhere is inert.
        if self.default_moved(name) {
            return Vec::new();
        }
        vec![Ok(key)]
    }
    /// Translate a mouse button the same way (keys bound to mouse buttons).
    pub fn translate_mouse(&self, button: MouseButton) -> Option<Result<KeyCode, MouseButton>> {
        self.translate_mouse_all(button).into_iter().next()
    }
    /// [`KeyBinds::translate_all`] for a mouse button.
    pub fn translate_mouse_all(&self, button: MouseButton) -> Vec<Result<KeyCode, MouseButton>> {
        let name = mouse_name(button);
        let bound: Vec<_> = self
            .translated_mappings(&name)
            .filter_map(|mapping| {
                let default = Self::default_of(mapping);
                match mouse_button(default) {
                    Some(b) => Some(Err(b)),
                    None => key_code(default).map(Ok),
                }
            })
            .collect();
        if !bound.is_empty() {
            return distinct(bound);
        }
        if self.default_moved(&name) {
            return Vec::new();
        }
        vec![Err(button)]
    }
    /// `AbstractContainerScreen.checkHotbarMouseClicked`: off hand, then hotbar.
    pub fn container_mouse_key(&self, button: MouseButton, creative: bool) -> Option<KeyCode> {
        let name = mouse_name(button);
        if matches!(button, MouseButton::Left | MouseButton::Right)
            || (creative && self.get("key.pickItem") == name)
        {
            return None;
        }
        std::iter::once("key.swapOffhand")
            .chain(
                MAPPINGS
                    .iter()
                    .map(|m| m.0)
                    .filter(|n| n.starts_with("key.hotbar.")),
            )
            .find(|mapping| self.get(mapping) == name)
            .and_then(|mapping| key_code(Self::default_of(mapping)))
    }
    /// `AbstractContainerScreen.checkHotbarKeyPressed`: the off hand and the
    /// hotbar slots, as the controls bind them now. Answers the key the menu
    /// acts on (`KeyF` for the off hand, `DigitN` for a slot), so a rebound key
    /// swaps instead of reaching the screen.
    pub fn container_hotbar_key(&self, key: KeyCode) -> Option<KeyCode> {
        let name = key_name(key)?;
        if self.get("key.swapOffhand") == name {
            return Some(KeyCode::KeyF);
        }
        MAPPINGS
            .iter()
            .filter(|m| m.0.starts_with("key.hotbar."))
            .find(|m| self.get(m.0) == name)
            .and_then(|m| key_code(Self::default_of(m.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn container_mouse_hotkeys_follow_slot_bindings_and_pick_item_priority() {
        for button in [MouseButton::Back, MouseButton::Forward, MouseButton::Middle] {
            for (mapping, default, _) in MAPPINGS.iter().filter(|m| m.0.starts_with("key.hotbar."))
            {
                let mut keys = KeyBinds::default();
                keys.set(mapping, &mouse_name(button));
                assert_eq!(keys.container_mouse_key(button, false), key_code(default));
                assert_eq!(
                    keys.container_mouse_key(button, true),
                    if button == MouseButton::Middle {
                        None
                    } else {
                        key_code(default)
                    }
                );
            }
        }
        let mut keys = KeyBinds::default();
        keys.set("key.hotbar.1", "key.mouse.4");
        keys.set("key.swapOffhand", "key.mouse.4");
        assert_eq!(
            keys.container_mouse_key(MouseButton::Back, false),
            Some(KeyCode::KeyF)
        );
        keys.set("key.hotbar.2", "key.mouse.left");
        assert_eq!(keys.container_mouse_key(MouseButton::Left, false), None);
        assert_eq!(keys.container_mouse_key(MouseButton::Right, false), None);
        assert_eq!(keys.container_mouse_key(MouseButton::Forward, false), None);
    }

    #[test]
    fn container_hotkeys_follow_the_bound_keys() {
        let mut keys = KeyBinds::default();
        assert_eq!(
            keys.container_hotbar_key(KeyCode::KeyF),
            Some(KeyCode::KeyF)
        );
        assert_eq!(
            keys.container_hotbar_key(KeyCode::Digit3),
            Some(KeyCode::Digit3)
        );
        assert_eq!(keys.container_hotbar_key(KeyCode::Tab), None);
        // The report's key: bound to the off hand, the menu takes it first.
        keys.set("key.swapOffhand", "key.keyboard.tab");
        assert_eq!(keys.container_hotbar_key(KeyCode::Tab), Some(KeyCode::KeyF));
        assert_eq!(keys.container_hotbar_key(KeyCode::KeyF), None);
        keys.set("key.hotbar.3", "key.keyboard.t");
        assert_eq!(
            keys.container_hotbar_key(KeyCode::KeyT),
            Some(KeyCode::Digit3)
        );
    }

    #[test]
    fn rebinding_translates_to_defaults() {
        let mut k = KeyBinds::default();
        assert_eq!(k.translate(KeyCode::KeyW), Some(Ok(KeyCode::KeyW)));
        // Forward on the up arrow, inventory on W.
        k.set("key.forward", "key.keyboard.up");
        k.set("key.inventory", "key.keyboard.w");
        assert_eq!(k.translate(KeyCode::ArrowUp), Some(Ok(KeyCode::KeyW)));
        assert_eq!(k.translate(KeyCode::KeyW), Some(Ok(KeyCode::KeyE)));
        // E's default meaning moved and nothing else uses it.
        assert_eq!(k.translate(KeyCode::KeyE), None);
        // Attack on a key.
        k.set("key.attack", "key.keyboard.r");
        assert_eq!(k.translate(KeyCode::KeyR), Some(Err(MouseButton::Left)));
        assert_eq!(k.translate_mouse(MouseButton::Left), None);
        assert_eq!(k.translate(KeyCode::Escape), Some(Ok(KeyCode::Escape)));
        assert!(k.conflicts("key.forward").is_empty());
        k.set("key.jump", "key.keyboard.up");
        assert_eq!(k.conflicts("key.forward"), ["key.jump"]);
    }
    #[test]
    fn a_moved_key_keeps_no_default_action() {
        // #3158: the debug keys (F3+A, F3+1...) share these keys' defaults,
        // and must not hand them back their gameplay meaning.
        let mut k = KeyBinds::default();
        k.set("key.hotbar.1", "key.keyboard.q");
        assert_eq!(k.translate_all(KeyCode::Digit1), []);
        assert_eq!(
            k.translate_all(KeyCode::KeyQ),
            [Ok(KeyCode::KeyQ), Ok(KeyCode::Digit1)]
        );
        k.set("key.inventory", "key.keyboard.a");
        k.set("key.left", "key.keyboard.e");
        assert_eq!(k.translate_all(KeyCode::KeyA), [Ok(KeyCode::KeyE)]);
        assert_eq!(k.translate_all(KeyCode::KeyE), [Ok(KeyCode::KeyA)]);
        k.set("key.back", "key.keyboard.down");
        assert_eq!(k.translate_all(KeyCode::KeyS), []);
        // Keys left alone still act, and a moved debug key changes nothing.
        k.set("key.debug.fpsCharts", "key.keyboard.9");
        assert_eq!(k.translate_all(KeyCode::Digit2), [Ok(KeyCode::Digit2)]);
        assert_eq!(k.translate_all(KeyCode::Digit9), [Ok(KeyCode::Digit9)]);
        assert_eq!(k.translate_all(KeyCode::F3), [Ok(KeyCode::F3)]);
    }
    #[test]
    fn display_names() {
        assert_eq!(display_name("key.keyboard.w").plain(), "W");
        assert_eq!(display_name("key.keyboard.slash").plain(), "/");
    }
}
