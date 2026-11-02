//! The debug keys (26.3 `KeyboardHandler`): `key.debug.modifier` (F3) held
//! with another key runs a debug action (`handleDebugKeys`), released alone
//! it toggles the debug overlay (`key.debug.overlay`, F3 too by default);
//! with Escape it pauses without the menu; held with `key.debug.crash` (C)
//! for ten seconds it crashes the game (`tick`).
use crate::keybinds::KeyBinds;

/// A debug mapping's action, as `handleDebugKeys` tests them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugAction {
    ReloadChunks,
    ShowHitboxes,
    ClearChat,
    ShowChunkBorders,
    AdvancedTooltips,
    CopyRecreateCommand,
    Spectate,
    SwitchGameMode,
    DebugOptions,
    FocusPause,
    DumpDynamicTextures,
    ReloadResourcePacks,
    Profiling,
    CopyLocation,
    DumpVersion,
    ProfilingChart,
    FpsCharts,
    NetworkCharts,
    LightmapTexture,
    ImprovedTransparency,
    /// F3+R: start a repro recording, or stop it and write its bundle.
    RecordSession,
    /// F3+E: write the bundle now.
    ExportRepro,
}

/// The mappings `handleDebugKeys` tests, in its order.
pub const ACTIONS: [(&str, DebugAction); 20] = [
    ("key.debug.reloadChunk", DebugAction::ReloadChunks),
    ("key.debug.showHitboxes", DebugAction::ShowHitboxes),
    ("key.debug.clearChat", DebugAction::ClearChat),
    ("key.debug.showChunkBorders", DebugAction::ShowChunkBorders),
    (
        "key.debug.showAdvancedTooltips",
        DebugAction::AdvancedTooltips,
    ),
    (
        "key.debug.copyRecreateCommand",
        DebugAction::CopyRecreateCommand,
    ),
    ("key.debug.spectate", DebugAction::Spectate),
    ("key.debug.switchGameMode", DebugAction::SwitchGameMode),
    ("key.debug.debugOptions", DebugAction::DebugOptions),
    ("key.debug.focusPause", DebugAction::FocusPause),
    (
        "key.debug.dumpDynamicTextures",
        DebugAction::DumpDynamicTextures,
    ),
    (
        "key.debug.reloadResourcePacks",
        DebugAction::ReloadResourcePacks,
    ),
    ("key.debug.profiling", DebugAction::Profiling),
    ("key.debug.copyLocation", DebugAction::CopyLocation),
    ("key.debug.dumpVersion", DebugAction::DumpVersion),
    ("key.debug.profilingChart", DebugAction::ProfilingChart),
    ("key.debug.fpsCharts", DebugAction::FpsCharts),
    ("key.debug.networkCharts", DebugAction::NetworkCharts),
    ("key.debug.lightmapTexture", DebugAction::LightmapTexture),
    (
        "key.debug.improvedTransparency",
        DebugAction::ImprovedTransparency,
    ),
];

/// Horizon's own debug keys: fixed, so they never enter the options table that
/// vanilla parity compares (`MAPPINGS`). Off until the AI Agent page enables
/// them; `Game` decides that in `debug_action`.
const HORIZON_ACTIONS: [(&str, DebugAction); 2] = [
    ("key.keyboard.r", DebugAction::RecordSession),
    ("key.keyboard.e", DebugAction::ExportRepro),
];

/// GLFW's key actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    Press,
    Repeat,
}

/// The screen open when a key reaches the game (after the screen itself
/// passed on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenScreen {
    None,
    /// `PauseScreen` without its menu (F3+Escape).
    PauseWithoutMenu,
    GameModeSwitcher,
    Other,
}

/// What the game does for `KeyboardHandler.keyPress`.
pub trait DebugHost {
    fn screen(&self) -> OpenScreen;
    /// `DebugScreenEntries.toggleDebugOverlay`.
    fn toggle_overlay(&mut self);
    /// `Minecraft.pauseGame(suppressPauseMenu)`.
    fn pause_game(&mut self, without_menu: bool);
    /// One matching mapping of `handleDebugKeys`; whether its condition held
    /// (vanilla's `debugAction = true`). `shift`: `event.hasShiftDown()`.
    fn debug_action(&mut self, action: DebugAction, shift: bool) -> bool;
}

/// What `KeyboardHandler.tick` reports while the debug crash keys are held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashNotice {
    /// `debug.crash.message`, a second after the keys went down.
    Message,
    /// `debug.crash.warning` with the seconds left.
    Warning(i32),
    /// "Manually triggered debug crash".
    Crash,
}

/// `KeyboardHandler`'s state, with the down state of `keyDebugModifier`.
#[derive(Clone, Debug, Default)]
pub struct DebugKeys {
    pub used_debug_key_as_modifier: bool,
    /// `debugCrashKeyTime`: when the crash keys went down (milliseconds),
    /// 0 never, -1 released.
    pub crash_key_time: i64,
    pub crash_key_reported_time: i64,
    pub crash_key_reported_count: i64,
    /// `keyDebugModifier.isDown()`.
    pub modifier_down: bool,
}

impl DebugKeys {
    /// `keyPress` once the open screen (if any) did not take the key. `key`
    /// is the physical key's name (`key.keyboard.f3`); `crash_key_down`
    /// whether the crash mapping's key is held (`InputConstants.isKeyDown`).
    /// Returns whether the key goes on to the game's key mappings
    /// (`KeyMapping.set(key, true)` and `click`): only without a screen and
    /// when it ran no debug action.
    #[allow(clippy::too_many_arguments)]
    pub fn key_press(
        &mut self,
        host: &mut impl DebugHost,
        binds: &KeyBinds,
        key: &str,
        action: KeyAction,
        shift: bool,
        crash_key_down: bool,
        now: i64,
    ) -> bool {
        let modifier = binds.get("key.debug.modifier");
        let overlay = binds.get("key.debug.overlay");
        let same = modifier == overlay;
        let modifier_down = self.modifier_down;
        let crash = binds.get("key.debug.crash") != "key.keyboard.unknown" && crash_key_down;
        if self.crash_key_time > 0 {
            if !crash || !modifier_down {
                self.crash_key_time = -1;
            }
        } else if crash && modifier_down {
            self.used_debug_key_as_modifier = same;
            self.crash_key_time = now;
            self.crash_key_reported_time = now;
            self.crash_key_reported_count = 0;
        }
        let screen = host.screen();
        let handles_game = screen == OpenScreen::None;
        let handles_global = handles_game
            || matches!(
                screen,
                OpenScreen::PauseWithoutMenu | OpenScreen::GameModeSwitcher
            );
        if same && key == modifier && action == KeyAction::Release {
            if self.used_debug_key_as_modifier {
                self.used_debug_key_as_modifier = false;
            } else {
                host.toggle_overlay();
            }
        } else if !same && key == overlay && action == KeyAction::Press {
            host.toggle_overlay();
        }
        if action == KeyAction::Release {
            // KeyMapping.set(key, false).
            if key == modifier {
                self.modifier_down = false;
            }
            return false;
        }
        let mut did = false;
        if handles_global && key == "key.keyboard.escape" {
            host.pause_game(modifier_down);
            did = modifier_down;
        } else if modifier_down {
            did = self.handle_debug_keys(host, binds, key, shift, now);
        }
        if same {
            self.used_debug_key_as_modifier |= did;
        }
        // KeyMapping.set(key, !did) for the game, setDown(!did) behind a screen.
        if key == modifier {
            self.modifier_down = !did;
        }
        handles_game && !did
    }
    /// `handleDebugKeys`: every debug mapping on this key, in order.
    fn handle_debug_keys(
        &mut self,
        host: &mut impl DebugHost,
        binds: &KeyBinds,
        key: &str,
        shift: bool,
        now: i64,
    ) -> bool {
        // Once the crash keys have been down a tenth of a second.
        if self.crash_key_time > 0 && self.crash_key_time < now - 100 {
            return true;
        }
        let mut did = false;
        for (name, action) in ACTIONS {
            if binds.get(name) == key {
                did |= host.debug_action(action, shift);
            }
        }
        for (name, action) in HORIZON_ACTIONS {
            if key == name {
                did |= host.debug_action(action, shift);
            }
        }
        did
    }
    /// `KeyboardHandler.tick`.
    pub fn tick(&mut self, now: i64) -> Option<CrashNotice> {
        if self.crash_key_time <= 0 {
            return None;
        }
        let remaining = 10000 - (now - self.crash_key_time);
        let reported = now - self.crash_key_reported_time;
        if remaining < 0 {
            return Some(CrashNotice::Crash);
        }
        if reported >= 1000 {
            let notice = if self.crash_key_reported_count == 0 {
                CrashNotice::Message
            } else {
                // Mth.ceil(remainingTime / 1000.0F).
                CrashNotice::Warning((remaining as f32 / 1000.0).ceil() as i32)
            };
            self.crash_key_reported_time = now;
            self.crash_key_reported_count += 1;
            return Some(notice);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Host {
        screen: Option<OpenScreen>,
        overlay: u32,
        paused: Vec<bool>,
        actions: Vec<(DebugAction, bool)>,
        /// Actions whose condition fails (no player, reduced debug info...).
        refused: Vec<DebugAction>,
    }
    impl DebugHost for Host {
        fn screen(&self) -> OpenScreen {
            self.screen.unwrap_or(OpenScreen::None)
        }
        fn toggle_overlay(&mut self) {
            self.overlay += 1;
        }
        fn pause_game(&mut self, without_menu: bool) {
            self.paused.push(without_menu);
        }
        fn debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
            self.actions.push((action, shift));
            !self.refused.contains(&action)
        }
    }
    const F3: &str = "key.keyboard.f3";

    fn press(k: &mut DebugKeys, h: &mut Host, b: &KeyBinds, key: &str, now: i64) -> bool {
        k.key_press(h, b, key, KeyAction::Press, false, false, now)
    }
    fn release(k: &mut DebugKeys, h: &mut Host, b: &KeyBinds, key: &str) -> bool {
        k.key_press(h, b, key, KeyAction::Release, false, false, 0)
    }

    #[test]
    fn f3_alone_toggles_the_overlay_on_release() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        // The F3 press goes to the game's mappings (key.debug.* are down now).
        assert!(press(&mut k, &mut h, &b, F3, 0));
        assert!(k.modifier_down);
        assert_eq!(h.overlay, 0);
        assert!(!release(&mut k, &mut h, &b, F3));
        assert_eq!(h.overlay, 1);
        assert!(!k.modifier_down);
    }

    #[test]
    fn a_combination_runs_its_action_and_keeps_the_overlay() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        press(&mut k, &mut h, &b, F3, 0);
        // F3+G: the action, and G does not reach the game.
        assert!(!press(&mut k, &mut h, &b, "key.keyboard.g", 10));
        assert_eq!(h.actions, [(DebugAction::ShowChunkBorders, false)]);
        release(&mut k, &mut h, &b, "key.keyboard.g");
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 0);
        // The next F3 alone toggles again.
        press(&mut k, &mut h, &b, F3, 20);
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 1);
    }

    #[test]
    fn c_copies_the_location_and_both_c_mappings_share_the_key() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        press(&mut k, &mut h, &b, F3, 0);
        k.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Press,
            true,
            true,
            5,
        );
        assert_eq!(h.actions, [(DebugAction::CopyLocation, true)]);
        // The crash keys are down: from a tenth of a second on, debug keys
        // are swallowed.
        assert_eq!(k.crash_key_time, 5);
        k.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Repeat,
            false,
            true,
            50,
        );
        assert_eq!(h.actions.len(), 2);
        assert!(!k.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Repeat,
            false,
            true,
            200
        ));
        assert_eq!(h.actions.len(), 2);
    }

    #[test]
    fn held_crash_keys_warn_every_second_then_crash() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        press(&mut k, &mut h, &b, F3, 0);
        k.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Press,
            false,
            true,
            1000,
        );
        assert_eq!(k.tick(1500), None);
        assert_eq!(k.tick(2000), Some(CrashNotice::Message));
        assert_eq!(k.tick(2999), None);
        assert_eq!(k.tick(3000), Some(CrashNotice::Warning(8)));
        assert_eq!(k.tick(4100), Some(CrashNotice::Warning(7)));
        assert_eq!(k.tick(11_000), Some(CrashNotice::Warning(0)));
        assert_eq!(k.tick(11_001), Some(CrashNotice::Crash));
        // Releasing either key stops it on the next key event.
        let mut k2 = DebugKeys::default();
        press(&mut k2, &mut h, &b, F3, 0);
        k2.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Press,
            false,
            true,
            1000,
        );
        k2.key_press(
            &mut h,
            &b,
            "key.keyboard.c",
            KeyAction::Release,
            false,
            false,
            1200,
        );
        assert_eq!(k2.crash_key_time, -1);
        assert_eq!(k2.tick(20_000), None);
        // The crash used F3 as a modifier: its release does not toggle the overlay.
        release(&mut k2, &mut h, &b, F3);
        assert_eq!(h.overlay, 0);
    }

    #[test]
    fn refused_actions_do_not_count() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        h.refused.push(DebugAction::ShowHitboxes);
        press(&mut k, &mut h, &b, F3, 0);
        // F3+B without a player (or with reduced debug info): B reaches the
        // game and F3's release still toggles the overlay.
        assert!(press(&mut k, &mut h, &b, "key.keyboard.b", 1));
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 1);
    }

    #[test]
    fn escape_pauses_with_or_without_the_menu() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        assert!(press(&mut k, &mut h, &b, "key.keyboard.escape", 0));
        press(&mut k, &mut h, &b, F3, 1);
        assert!(!press(&mut k, &mut h, &b, "key.keyboard.escape", 2));
        assert_eq!(h.paused, [false, true]);
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 0);
        // Behind another screen, Escape is the screen's.
        h.screen = Some(OpenScreen::Other);
        press(&mut k, &mut h, &b, "key.keyboard.escape", 3);
        assert_eq!(h.paused.len(), 2);
    }

    #[test]
    fn behind_a_screen_combinations_work_but_keys_stay_with_the_screen() {
        let (mut k, mut h, b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        h.screen = Some(OpenScreen::Other);
        assert!(!press(&mut k, &mut h, &b, F3, 0));
        assert!(k.modifier_down);
        assert!(!press(&mut k, &mut h, &b, "key.keyboard.h", 1));
        assert_eq!(h.actions, [(DebugAction::AdvancedTooltips, false)]);
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 0);
    }

    #[test]
    fn switcher_modifier_release_does_not_toggle_the_debug_overlay() {
        let (mut k, mut h, mut b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        // Screen release handling must also work with rebound debug keys.
        b.set("key.debug.modifier", "key.keyboard.f7");
        b.set("key.debug.overlay", "key.keyboard.f7");
        b.set("key.debug.switchGameMode", "key.keyboard.f8");
        press(&mut k, &mut h, &b, "key.keyboard.f7", 0);
        assert!(!press(&mut k, &mut h, &b, "key.keyboard.f8", 1));
        assert_eq!(h.actions, [(DebugAction::SwitchGameMode, false)]);
        h.screen = Some(OpenScreen::GameModeSwitcher);
        release(&mut k, &mut h, &b, "key.keyboard.f8");
        release(&mut k, &mut h, &b, "key.keyboard.f7");
        assert!(!k.modifier_down);
        assert_eq!(h.overlay, 0);
    }

    #[test]
    fn a_separate_overlay_key_toggles_on_press() {
        let (mut k, mut h, mut b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        b.set("key.debug.overlay", "key.keyboard.f7");
        press(&mut k, &mut h, &b, "key.keyboard.f7", 0);
        assert_eq!(h.overlay, 1);
        press(&mut k, &mut h, &b, F3, 1);
        release(&mut k, &mut h, &b, F3);
        assert_eq!(h.overlay, 1);
    }

    #[test]
    fn rebound_mappings_follow_their_keys() {
        let (mut k, mut h, mut b) = (DebugKeys::default(), Host::default(), KeyBinds::default());
        b.set("key.debug.showChunkBorders", "key.keyboard.k");
        press(&mut k, &mut h, &b, F3, 0);
        assert!(press(&mut k, &mut h, &b, "key.keyboard.g", 1));
        press(&mut k, &mut h, &b, "key.keyboard.k", 2);
        assert_eq!(h.actions, [(DebugAction::ShowChunkBorders, false)]);
    }
}
