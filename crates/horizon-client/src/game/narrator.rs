//! The narrator (`Minecraft.narrator`, horizon-narrator's `GameNarrator`):
//! opened with the client, closed with it, and kept in step with the options
//! it reads (`Options.narrator`, `getFinalSoundSourceVolume(VOICE)`); and its
//! `NARRATOR_TOGGLE` toast.
use super::{Game, Screen};
use horizon_client::{
    clipboard,
    hud::{HudAtlas, HudBatch},
    screen_narrator::{InputType, ScreenNarrator},
    settings::Settings,
    text::Text,
    toasts::SystemToast,
};
use horizon_narrator::{Status, screen::Trigger};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// `Minecraft`'s constructor: `new GameNarrator`, then
/// `narrator.checkStatus(options.narrator().get() != OFF)`.
pub(super) fn start(settings: &Settings) {
    horizon_narrator::init();
    sync(settings);
    check_status(settings.narrator() != Status::Off);
}

/// `GameNarrator.checkStatus`: the narrator is wanted but its speech didn't
/// open. Vanilla asks in a message box whether to go on; Horizon has no
/// native message box, so it logs vanilla's message and goes on, as the
/// box's Continue does (the option then reads "Not Available").
fn check_status(required_active: bool) {
    if required_active && !horizon_narrator::is_active() {
        log::error!(
            "Failed to initialize text-to-speech library; continuing without the narrator. \
             If this problem persists, please report it at bugs.mojang.com"
        );
    }
}

/// The options the narrator reads, after they load or change: its status and
/// the voice volume (master times voice).
pub(super) fn sync(settings: &Settings) {
    horizon_narrator::set_status(settings.narrator());
    horizon_narrator::set_volume(settings.final_volume("voice"));
}

/// `GameNarrator.updateNarratorStatus`, once the narrator option holds
/// `status`: what was being said stops and "Narrator : <status>" is said,
/// whatever the status (Off included). Then the `NARRATOR_TOGGLE` toast goes
/// up, or the one still up takes the new text (`SystemToast.addOrUpdate`).
pub(super) fn update_status(status: Status, toast: &mut Option<SystemToast>) {
    let announcement = Text::translate("options.narrator", &[])
        .append(Text::literal(" : "))
        .append(Text::translate(status.key(), &[]))
        .plain();
    horizon_narrator::update_status(status, &announcement);
    let (title, message) = horizon_narrator::toast(status, horizon_narrator::is_active());
    let tr = |key: &str| Text::translate(key, &[]).plain();
    let (title, message) = (tr(title), message.map(tr));
    match toast {
        Some(toast) => toast.reset(title, message),
        None => *toast = Some(SystemToast::new(title, message)),
    }
}

/// `ToastManager.update` and `extractRenderState` for the narrator toast: it
/// takes the first slot `used` leaves free, slides, and is drawn there unless
/// `hidden` (F1). The visibility it switched to, for its sound.
pub(super) fn draw_toast(
    toast: &mut Option<SystemToast>,
    used: u32,
    multiplier: f64,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    hidden: bool,
) -> Option<bool> {
    let shown = toast.as_mut()?;
    if shown.slot.is_none() {
        shown.slot = Some((0..Game::TOAST_SLOTS).find(|s| used & (1 << s) == 0)?);
    }
    let (sound, finished) = shown.update(multiplier);
    if finished {
        *toast = None;
    } else if let (false, Some(slot)) = (hidden, shown.slot) {
        shown.draw(atlas, batch, 32.0 * slot as f32);
    }
    sound
}

/// `KeyboardHandler.keyPress`'s narrator hotkey: a press or repeat of B
/// (`shortcutKey() == 98`, the layout's letter) with the edit modifier
/// (`hasControlDownWithQuirk`: Cmd on macOS, Ctrl elsewhere), unless the
/// screen's focus is an edit box that takes the keys, while the narrator is
/// there and the Narrator Hotkey option is on.
pub(super) fn is_hotkey(
    pressed: bool,
    control_with_quirk: bool,
    letter: Option<char>,
    edit_box_focused: bool,
    active: bool,
    hotkey_option: bool,
) -> bool {
    pressed
        && !edit_box_focused
        && control_with_quirk
        && letter == Some('b')
        && active
        && hotkey_option
}

/// What the open screen's narration hears of the window's input, before the
/// screen takes it: `MouseHandler.onButton` (`setLastInputType(MOUSE)`,
/// `afterMouseAction` on a press), `onScroll`, `handleAccumulatedMovement`
/// (`afterMouseMove`), and `KeyboardHandler.keyPress` (Tab and the arrows
/// set the input type; a press is `afterKeyboardAction`). The window's own
/// shortcuts (fullscreen, screenshots, debug keys) never get here.
pub(super) fn narrate_input(
    narration: &mut ScreenNarrator,
    event: &WindowEvent,
    cursor: (f32, f32),
) {
    match event {
        WindowEvent::KeyboardInput { event, .. } => {
            if let PhysicalKey::Code(key) = event.physical_key {
                narrate_key(narration, key, event.state == ElementState::Pressed);
            } else if event.state == ElementState::Pressed {
                narration.keyboard_action();
            }
        }
        WindowEvent::MouseInput { state, button, .. } => {
            narration.set_input(InputType::Mouse);
            if *state == ElementState::Pressed {
                narration.mouse_pressed(cursor, *button == MouseButton::Left);
            }
        }
        WindowEvent::MouseWheel { .. } => narration.mouse_action(),
        WindowEvent::CursorMoved { .. } => narration.mouse_moved(),
        _ => {}
    }
}

/// `KeyboardHandler.keyPress` for the narration: Tab and the arrows set the
/// input type, a press is `afterKeyboardAction`. The window's keys and the
/// control socket's `key` both come here.
pub(super) fn narrate_key(narration: &mut ScreenNarrator, key: KeyCode, pressed: bool) {
    match key {
        KeyCode::Tab => narration.set_input(InputType::KeyboardTab),
        KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
            narration.set_input(InputType::KeyboardArrow)
        }
        _ => {}
    }
    if pressed {
        narration.keyboard_action();
    }
}

/// The hotkey's step: `options.narrator().set(byId(getId() + 1))`. Whether
/// the narrator was off before (`wasDisabled`).
pub(super) fn cycle_status(settings: &mut Settings) -> bool {
    let was = settings.narrator();
    settings.set_narrator(was.next());
    was == Status::Off
}

impl Game<'_> {
    /// Whether this `Game` is the window's: a server thread's `Game` shares
    /// the process (and its narrator) but has no `Minecraft` to narrate for.
    fn narrates(&self) -> bool {
        self.options.execution_role.is_client()
    }
    /// `GameNarrator.saySystemNow(message)`: cuts off what is being said.
    pub(super) fn say_system_now(&self, message: &Text) {
        if self.narrates() {
            horizon_narrator::say_system_now(&message.plain());
        }
    }
    /// `GameNarrator.saySystemQueued(message)`.
    pub(super) fn say_system_queued(&self, message: &Text) {
        if self.narrates() {
            horizon_narrator::say_system_queued(&message.plain());
        }
    }
    /// `GameNarrator.saySystemChatQueued(message)`.
    pub(super) fn say_system_chat_queued(&self, message: &Text) {
        if self.narrates() {
            horizon_narrator::say_system_chat_queued(&message.plain());
        }
    }
    /// `GameNarrator.sayChatQueued(message)`, its text already made.
    pub(super) fn say_chat_queued(&self, message: &str) {
        if self.narrates() {
            horizon_narrator::say_chat_queued(message);
        }
    }
    /// `GameNarrator.clear()`: what is being said and queued stops.
    pub(super) fn clear_narrator(&self) {
        if self.narrates() {
            horizon_narrator::clear();
        }
    }
    /// The narrator option changed, on the options screen or by its hotkey.
    pub(super) fn narrator_status_changed(&mut self) {
        update_status(self.settings.narrator(), &mut self.narrator_toast);
    }
    /// The narrator hotkey on a key press or repeat, before the screen or
    /// the game takes the key (it doesn't consume it): the next status,
    /// announced with its toast, saved, then the screen hears of it.
    pub(super) fn narrator_hotkey(&mut self, event: &KeyEvent) {
        let letter = clipboard::shortcut_letter(&event.logical_key, event.physical_key);
        if letter != Some('b') {
            return;
        }
        let hotkey = is_hotkey(
            event.state == ElementState::Pressed,
            self.has_control_down_with_quirk(),
            letter,
            self.edit_box_focused(),
            horizon_narrator::is_active(),
            self.settings.file.bool("narratorHotkey"),
        );
        if !hotkey {
            return;
        }
        let was_disabled = cycle_status(&mut self.settings);
        // OptionInstance.set runs the option's onValueUpdate.
        self.narrator_status_changed();
        self.settings.save(&Self::options_path());
        self.on_narrator_status_changed(was_disabled, Trigger::Keyboard);
    }
    /// Where vanilla calls `screen.updateNarratorStatus(wasDisabled, trigger)`
    /// on the open screen, if any: a screen's narration is scheduled 2 s
    /// after the narrator came on (`NARRATE_DELAY_NARRATOR_ENABLED`) and its
    /// narrator button shows the new value. Horizon's narrator buttons read
    /// the option each frame.
    pub(super) fn on_narrator_status_changed(&mut self, was_disabled: bool, trigger: Trigger) {
        self.narration.status_changed(was_disabled, trigger);
    }
    /// The open screen's narration, once a turn after the frame's widgets
    /// were read: `handleDelayedNarration` and a new screen's narration.
    pub(super) fn narrate_screen(&mut self) {
        if !self.narrates() {
            return;
        }
        let pad_focus = self.pad_focus_rect();
        let gui = self.gui_size();
        self.narration.frame(gui, self.cursor, pad_focus);
    }
    /// `hasNoEditboxFocused`'s opposite: the open screen's focus is an
    /// `EditBox` that `canConsumeInput`. The chat, command block, anvil
    /// name, world options' port and rule boxes, a dialog's text input, a
    /// focused options search and the title screens' fields count; a sign, a
    /// book (no `EditBox`), the recipe book (its component has the focus) and
    /// the creative search (focused as a widget, not as the screen's focus)
    /// do not.
    pub(super) fn edit_box_focused(&mut self) -> bool {
        if self.dialog_open() {
            return self.dialog_typing();
        }
        match self.screen {
            Screen::Chat if self.title.is_none() => true,
            Screen::Options => self.ui.options.typing(),
            Screen::WorldOptions => self.world_options_typing(),
            Screen::CommandBlock | Screen::Social => true,
            Screen::Inventory => self.station_typing() || self.mod_browser_typing(),
            _ => self
                .title
                .as_mut()
                .is_some_and(|t| t.has_caret_field() || t.focused_field().is_some()),
        }
    }
    /// The narrator toast over whatever is on screen, the title's own toast
    /// keeping the first slot.
    pub(super) fn draw_narrator_toast(&mut self, batch: &mut HudBatch) {
        let title = self.title.as_ref().is_some_and(|t| t.has_toast());
        let used = self.toast_slots_used() | u32::from(title);
        let multiplier = f64::from(self.settings.notification_display_time);
        let sound = draw_toast(
            &mut self.narrator_toast,
            used,
            multiplier,
            &self.gui.atlas,
            batch,
            self.ui.hide_gui,
        );
        if let Some(show) = sound {
            let event = if show { "ui.toast.in" } else { "ui.toast.out" };
            self.audio
                .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
        }
    }
}

/// `Minecraft.destroy`: `narrator.destroy()`.
pub(super) fn stop() {
    horizon_narrator::destroy();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hotkey_needs_the_modifier_b_no_edit_box_the_narrator_and_its_option() {
        assert!(is_hotkey(true, true, Some('b'), false, true, true));
        for (pressed, control, letter, edit_box, active, option) in [
            (false, true, Some('b'), false, true, true),
            (true, false, Some('b'), false, true, true),
            (true, true, Some('n'), false, true, true),
            (true, true, Some('b'), true, true, true),
            (true, true, Some('b'), false, false, true),
            (true, true, Some('b'), false, true, false),
        ] {
            assert!(!is_hotkey(
                pressed, control, letter, edit_box, active, option
            ));
        }
    }

    #[test]
    fn the_hotkey_steps_through_the_statuses_and_wraps() {
        let mut s = Settings::default();
        assert!(cycle_status(&mut s), "Off was disabled");
        assert_eq!(s.narrator(), Status::All);
        assert!(!cycle_status(&mut s));
        assert!(!cycle_status(&mut s));
        assert_eq!(s.narrator(), Status::System);
        assert!(!cycle_status(&mut s));
        assert_eq!(s.narrator(), Status::Off);
    }

    /// `SystemToast.addOrUpdate`: one narrator toast, reset where it is.
    #[test]
    fn a_second_change_updates_the_toast_still_up() {
        let mut toast = None;
        update_status(Status::All, &mut toast);
        toast.as_mut().expect("the first change shows a toast").slot = Some(2);
        update_status(Status::Off, &mut toast);
        assert_eq!(toast.expect("still the one toast").slot, Some(2));
    }
}
