//! A controller in menus, screens and inventories (Horizon's own;
//! docs/controllers.md). Two ways to drive them, picked in the controller
//! options:
//!
//! - **Focus** (console-style): a highlight steps between the widgets, list
//!   rows, fields and slots each screen registers (`widgets::target_as`)
//!   with vanilla's arrow-key navigation; A presses the focused one, the
//!   bumpers switch tabs, left/right move a focused slider, A on a text field
//!   opens the on-screen keyboard (`osk`).
//! - **Pointer**: a virtual cursor the left stick moves and the D-pad steps
//!   between slots and buttons.
//!
//! Both feed the screens the same pointer events a mouse sends (the pointer
//! sits on the focused widget, hidden), so every screen keeps its mouse
//! behaviour (drags, sliders, scrollbars, hover highlights) with no pad code
//! of its own. A screen with nothing registered falls back to the pointer,
//! and a right stick click swaps modes for the open screen.
use super::event_loop::EventLoopCtl;
use super::{
    Game, Screen,
    key_focus::{self, Act},
};
use horizon_client::{
    Result,
    creative::Creative,
    gamepad::{self, Button, MenuNav, Rect},
    hud::{HudBatch, rgba},
    options_ui::{Event, Page},
    osk::{self, Keyboard},
    screen_narrator::InputType,
    widgets::{self, Kind, Target},
};
use std::time::Instant;
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::KeyCode,
    window::WindowId,
};

/// D-pad auto-repeat: the first step at once, the next after a pause.
const REPEAT_DELAY: f32 = 0.35;
const REPEAT_EVERY: f32 = 0.09;
/// Lines per second the right stick scrolls at full tilt.
const SCROLL_SPEED: f32 = 12.0;
/// How far the stick must lean to step focus.
const STICK_STEP: f32 = 0.55;
/// Scrolls a step may try to reveal the next row of a list.
const SCROLL_TRIES: u8 = 4;

/// A focus step that ran off the visible part of a list: the list was
/// scrolled and the step runs again once a new frame is drawn. When the
/// targets didn't move (nothing scrolled), the step takes vanilla's vague
/// pass instead.
#[derive(Clone)]
struct Pending {
    button: Button,
    from: Rect,
    generation: u64,
    tries: u8,
    before: Vec<Rect>,
}

#[derive(Default)]
pub(super) struct MenuCursor {
    /// Where the pad put the pointer (GUI pixels); `None` while the mouse
    /// has it.
    pos: Option<(f32, f32)>,
    last_poll: Option<Instant>,
    /// The stick moved the cursor last poll (it settles when let go).
    moving: bool,
    wheel: [f32; 2],
    /// The held D-pad direction and when it last stepped.
    repeat: Option<(Button, Instant, bool)>,
    /// Pointer buttons the pad holds down.
    held: [bool; 2],
    /// The OS pointer is hidden while the pad's cursor is drawn.
    os_hidden: bool,
    /// Polls since a menu opened (its widgets are drawn after one frame).
    open_polls: u32,
    /// Focus mode: the focused target as last drawn.
    focus: Option<Target>,
    /// The left stick's step direction last poll.
    stick: Option<Button>,
    /// A right stick click swapped modes for this screen.
    swapped: bool,
    pending: Option<Pending>,
    /// The focused slider being nudged and where the pad put its handle.
    nudge: Option<(Rect, f32)>,
    /// Steps since the held direction went down (sliders speed up).
    repeats: u32,
    /// The on-screen keyboard, open over a text field.
    pub(super) osk: Option<Keyboard>,
    /// The keyboard was closed on this screen (it doesn't open by itself again).
    osk_dismissed: bool,
    /// The keyboard drives the focus (`key_focus`: Tab, the arrows, Enter):
    /// focus mode whatever the controller options say, the OS pointer shown.
    keyboard: bool,
    /// The focused slider's arrows change its value (`canChangeValue`).
    slider_edit: bool,
    /// What the last click landed on: vanilla focuses it, so the next Tab
    /// or arrow goes on from there (`ContainerEventHandler.mouseClicked`).
    origin: Option<Rect>,
    /// A pointer event the pad or the keyboard sends is being handled.
    synthetic: bool,
    /// Where the real mouse last moved the pointer (window pixels).
    real_pointer: Option<PhysicalPosition<f64>>,
    /// The screen the keyboard's focus is on (`Game::key_screen`), and the
    /// frame drawn when another one opened: the focus starts again on the
    /// new screen's first drawn frame.
    key_screen: u64,
    key_restart: Option<u64>,
}

impl Game<'_> {
    /// A menu, a screen, the title, a sign or a book is open: the pad drives
    /// it instead of the player.
    pub(super) fn pad_in_menu(&self) -> bool {
        self.title.is_some()
            || self.screen != Screen::Playing
            || self.sign_edit.is_some()
            || self.book.is_some()
    }

    pub(super) fn gui_scale(&self) -> Option<HudBatch> {
        self.renderer
            .as_ref()
            .map(|r| HudBatch::new([r.config.width, r.config.height]))
    }

    /// Focus mode is on for the open screen (the keyboard always focuses).
    fn pad_focus_mode(&self) -> bool {
        self.pad.menu.keyboard
            || (self.settings.controller.menu_nav == MenuNav::Focus) != self.pad.menu.swapped
    }

    /// Where focus can land: buttons drawn last frame, every registered
    /// target (sliders, fields, rows, keyboard keys), and the slots
    /// and tabs of inventories (GUI pixels), on screen. The keyboard's
    /// focus skips what vanilla's keyboard can't reach (`key_targets`).
    fn pad_targets(&self) -> Vec<Target> {
        if self.pad.menu.keyboard {
            self.key_targets()
        } else {
            self.pad_visible_targets()
        }
    }

    /// The targets the keyboard reaches: no slots, no inventory tabs.
    fn key_targets(&self) -> Vec<Target> {
        let inventory =
            self.title.is_none() && matches!(self.screen, Screen::Inventory | Screen::Creative);
        key_focus::keyboard_targets(self.pad_visible_targets(), inventory)
    }

    fn pad_visible_targets(&self) -> Vec<Target> {
        let targets = self.pad_all_targets();
        match self.gui_scale() {
            Some(batch) => key_focus::on_screen(targets, (batch.width, batch.height)),
            None => targets,
        }
    }

    fn pad_all_targets(&self) -> Vec<Target> {
        let mut targets = key_focus::drawn_targets();
        if self.title.is_some() || self.sign_edit.is_some() || self.book.is_some() {
            return targets;
        }
        let Some(batch) = self.gui_scale() else {
            return targets;
        };
        let slot = |x: f32, y: f32| Target {
            rect: [x, y, 16.0, 16.0],
            kind: Kind::Slot,
        };
        match self.screen {
            Screen::Inventory => {
                let (ox, oy) = self.inventory_origin(&batch);
                targets.extend(self.slots().into_iter().map(|i| {
                    let (x, y) = self.slot_position(i);
                    slot(ox + x, oy + y)
                }));
                targets.extend(
                    self.station_targets()
                        .into_iter()
                        .map(|[x, y, w, h]| Target {
                            rect: [ox + x, oy + y, w, h],
                            kind: Kind::Button,
                        }),
                );
                targets.extend(self.recipe_book_targets());
            }
            Screen::Creative => {
                let (ox, oy) = Self::creative_origin(&batch);
                if self.creative_inventory_open() {
                    targets.extend(self.slots().into_iter().map(|i| {
                        let (x, y) = self.slot_position(i);
                        slot(ox + x, oy + y)
                    }));
                } else {
                    for r in 0..5 {
                        for c in 0..9 {
                            targets.push(slot(
                                ox + 9.0 + c as f32 * 18.0,
                                oy + 18.0 + r as f32 * 18.0,
                            ));
                        }
                    }
                    for c in 0..9 {
                        targets.push(slot(ox + 9.0 + c as f32 * 18.0, oy + 112.0));
                    }
                }
                for (i, t) in self.creative_screen.tabs.iter().enumerate() {
                    // A tab of another page of mods' tabs isn't drawn: its place is another's.
                    if !self.creative_screen.on_page(t) {
                        continue;
                    }
                    let (x, y) = Creative::tab_position(t);
                    targets.push(Target {
                        rect: [ox + x, oy + y, 26.0, 32.0],
                        kind: Kind::Tab(i == self.creative_screen.selected),
                    });
                }
            }
            _ => {}
        }
        targets
    }

    /// Where focus starts when a menu opens: the held hotbar slot in an
    /// inventory, else the first button, else the first target that isn't
    /// a tab.
    fn pad_first_target(&self, targets: &[Target]) -> Option<Target> {
        if self.pad.menu.keyboard {
            // Screen.setInitialFocus: a forward Tab from nothing.
            return key_focus::tab_step(targets, None, true).map(|i| targets[i]);
        }
        let batch = self.gui_scale()?;
        let at = |x: f32, y: f32| {
            targets
                .iter()
                .find(|t| t.rect[0] == x && t.rect[1] == y)
                .copied()
        };
        if self.title.is_none() {
            match self.screen {
                Screen::Inventory => {
                    let (ox, oy) = self.inventory_origin(&batch);
                    let (x, y) = self.slot_position(self.selected);
                    if let Some(t) = at(ox + x, oy + y) {
                        return Some(t);
                    }
                }
                Screen::Creative if !self.creative_inventory_open() => {
                    let (ox, oy) = Self::creative_origin(&batch);
                    if let Some(t) = at(ox + 9.0 + self.selected as f32 * 18.0, oy + 112.0) {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
        targets
            .iter()
            .find(|t| t.kind == Kind::Button)
            .or_else(|| targets.iter().find(|t| !matches!(t.kind, Kind::Tab(_))))
            .or(targets.first())
            .copied()
    }

    /// Send the window a pointer move to `at` (GUI pixels), as the mouse would.
    fn pad_pointer_to(&mut self, event_loop: &dyn EventLoopCtl, at: (f32, f32)) {
        let (Some(window), Some(batch)) = (self.window.as_ref(), self.gui_scale()) else {
            return;
        };
        let id = window.id();
        let at = (
            at.0.clamp(0.0, batch.width - 1.0),
            at.1.clamp(0.0, batch.height - 1.0),
        );
        let position =
            PhysicalPosition::new(f64::from(at.0 * batch.scale), f64::from(at.1 * batch.scale));
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position,
            },
        );
        // Read back what the window made of it, so a real mouse move shows.
        self.pad.menu.pos = Some(self.cursor);
    }

    fn pad_pointer_button(&mut self, event_loop: &dyn EventLoopCtl, right: bool, pressed: bool) {
        let Some(id) = self.window.as_ref().map(|w| w.id()) else {
            return;
        };
        self.pad.menu.held[usize::from(right)] = pressed;
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        let state = if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        };
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button,
            },
        );
    }

    /// A click (press and release) at a point, the pointer staying there.
    /// A pointer event of the pad's or the keyboard's, as the window would
    /// send it (marked, so it isn't taken for the real mouse).
    fn pad_synthetic_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
    ) {
        let was = std::mem::replace(&mut self.pad.menu.synthetic, true);
        self.handle_window_event(event_loop, id, event);
        self.pad.menu.synthetic = was;
    }

    /// The event being handled is one the pad or the keyboard's focus made
    /// up (`pad_synthetic_event`), not the window's. A controller's click or
    /// a focused widget's is not the mouse, so the native mods' mouse hooks
    /// skip it.
    pub(super) fn pad_synthetic_input(&self) -> bool {
        self.pad.menu.synthetic
    }

    fn pad_click_at(&mut self, event_loop: &dyn EventLoopCtl, at: (f32, f32), right: bool) {
        self.pad_pointer_to(event_loop, at);
        self.pad_pointer_button(event_loop, right, true);
        self.pad_pointer_button(event_loop, right, false);
    }

    fn pad_wheel(&mut self, event_loop: &dyn EventLoopCtl, x: f32, y: f32) {
        let Some(id) = self.window.as_ref().map(|w| w.id()) else {
            return;
        };
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
            },
        );
    }

    /// Put the pointer where the pad last left it, or on the first target
    /// (focus mode: focus lands there too).
    fn pad_take_pointer(&mut self, event_loop: &dyn EventLoopCtl) {
        if self.pad.menu.pos.is_some() {
            return;
        }
        let targets = self.pad_targets();
        let first = self.pad_first_target(&targets);
        if self.pad_focus_mode() && first.is_some() {
            self.pad.menu.focus = first;
        }
        let at = first.map_or(self.cursor, |t| t.center());
        self.pad_pointer_to(event_loop, at);
    }

    /// Focus mode with something to focus on this screen.
    fn pad_focusing(&self) -> bool {
        self.pad_focus_mode() && self.pad.menu.focus.is_some()
    }

    /// A button in a menu.
    pub(super) fn pad_menu_button(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if !down {
            match button {
                Button::South if self.pad.menu.held[0] => {
                    self.pad_pointer_button(event_loop, false, false)
                }
                Button::West if self.pad.menu.held[1] => {
                    self.pad_pointer_button(event_loop, true, false)
                }
                b if self.pad.menu.repeat.is_some_and(|r| r.0 == b) => self.pad.menu.repeat = None,
                _ => {}
            }
            return Ok(());
        }
        // The pad takes the focus back from the keyboard.
        self.pad.menu.keyboard = false;
        // Controller Binds waiting for a button: this one (Start cancels).
        if self.screen == Screen::Options
            && self.ui.options.page() == Page::PadBinds
            && let Some(action) = self.ui.options.selected_pad_action.take()
        {
            if button != Button::Start {
                self.settings.controller.bind(action, Some(button));
            }
            return self.handle_option_events(vec![Event::Changed]);
        }
        // The on-screen keyboard takes every button while it is open.
        if let Some(keyboard) = self.pad.menu.osk.as_mut() {
            if let Some(out) = keyboard.button(button) {
                self.pad_osk_output(out);
            }
            if matches!(
                button,
                Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight
            ) {
                self.pad.menu.repeat = Some((button, Instant::now(), false));
            }
            return Ok(());
        }
        match button {
            Button::East | Button::Start => return self.pad_back(),
            Button::RightStick => {
                // Swap focus and pointer for this screen.
                self.pad.menu.swapped = !self.pad.menu.swapped;
                self.pad.menu.focus = None;
                self.pad.menu.pending = None;
                self.pad.menu.nudge = None;
                self.pad.menu.pos = None;
                self.pad_take_pointer(event_loop);
            }
            Button::South if self.pad_focusing() => {
                let focus = self.pad.menu.focus.expect("focusing");
                match focus.kind {
                    // A click would jump the handle; left/right move it.
                    Kind::Slider { .. } => {}
                    Kind::Field => {
                        self.pad_click_at(event_loop, focus.center(), false);
                        self.pad_open_keyboard();
                    }
                    _ => {
                        self.pad_pointer_to(event_loop, focus.center());
                        self.pad_pointer_button(event_loop, false, true);
                    }
                }
            }
            Button::South | Button::West => {
                self.pad_take_pointer(event_loop);
                self.pad_pointer_button(event_loop, button == Button::West, true);
            }
            // Y in an inventory: a shift-click (quick move).
            Button::North if matches!(self.screen, Screen::Inventory | Screen::Creative) => {
                self.pad_take_pointer(event_loop);
                let in_keys = self.keys.contains(&KeyCode::ShiftLeft);
                self.keys.insert(KeyCode::ShiftLeft);
                let held = horizon_client_input::modifiers::hold(
                    &mut self.modifiers,
                    &mut self.physical_keys,
                    KeyCode::ShiftLeft,
                );
                self.pad_pointer_button(event_loop, false, true);
                self.pad_pointer_button(event_loop, false, false);
                if !in_keys {
                    self.keys.remove(&KeyCode::ShiftLeft);
                }
                horizon_client_input::modifiers::restore(
                    &mut self.modifiers,
                    &mut self.physical_keys,
                    held,
                );
            }
            // View / left stick in an inventory: throw one / the stack from
            // the slot under focus (Q / Ctrl+Q).
            Button::Select | Button::LeftStick
                if matches!(self.screen, Screen::Inventory | Screen::Creative)
                    && self.title.is_none() =>
            {
                self.pad_take_pointer(event_loop);
                let all = button == Button::LeftStick;
                let ctrl = self.physical_keys.contains(&KeyCode::ControlLeft);
                if all {
                    self.physical_keys.insert(KeyCode::ControlLeft);
                }
                let result = self.key_pressed(KeyCode::KeyQ);
                if all && !ctrl {
                    self.physical_keys.remove(&KeyCode::ControlLeft);
                }
                result?;
            }
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => {
                self.pad.menu.repeats = 0;
                self.pad_step(event_loop, button);
                self.pad.menu.repeat = Some((button, Instant::now(), false));
            }
            // Bumpers switch tabs (creative, Horizon's options sections, ...).
            Button::LeftBumper | Button::RightBumper => {
                self.pad_tab(event_loop, button == Button::RightBumper);
            }
            _ => {}
        }
        Ok(())
    }

    /// One D-pad (or stick) step in either mode.
    fn pad_step(&mut self, event_loop: &dyn EventLoopCtl, button: Button) {
        if let Some(keyboard) = self.pad.menu.osk.as_mut() {
            keyboard.step(button);
            return;
        }
        if self.pad_focus_mode() {
            let targets = self.pad_targets();
            if !targets.is_empty() {
                self.pad_focus_step(event_loop, button, &targets);
                return;
            }
        }
        self.pad_pointer_step(event_loop, button);
    }

    /// Pointer mode: hop the cursor to the nearest target that way.
    fn pad_pointer_step(&mut self, event_loop: &dyn EventLoopCtl, button: Button) {
        if self.pad.menu.pos.is_none() {
            self.pad_take_pointer(event_loop);
            return;
        }
        // With A held (a slider grabbed, a drag), the D-pad nudges instead.
        if self.pad.menu.held[0] {
            let (dx, dy) = match button {
                Button::DPadLeft => (-2.0, 0.0),
                Button::DPadRight => (2.0, 0.0),
                Button::DPadUp => (0.0, -2.0),
                _ => (0.0, 2.0),
            };
            let at = self.cursor;
            self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
            return;
        }
        let from = self.cursor;
        let centres: Vec<(f32, f32)> = self.pad_targets().iter().map(Target::center).collect();
        if let Some(i) = gamepad::focus_step(from, button, &centres) {
            self.pad_pointer_to(event_loop, centres[i]);
        }
    }

    /// Focus mode: move the highlight (or a focused slider's value).
    fn pad_focus_step(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        targets: &[Target],
    ) {
        let Some(focus) = self.pad.menu.focus else {
            self.pad.menu.pos = None;
            self.pad_take_pointer(event_loop);
            return;
        };
        if matches!(focus.kind, Kind::Slider { .. })
            && matches!(button, Button::DPadLeft | Button::DPadRight)
        {
            let step = gamepad::slider_step(self.pad.menu.repeats);
            let sign = if button == Button::DPadLeft {
                -1.0
            } else {
                1.0
            };
            self.pad_nudge(event_loop, focus, sign * step);
            return;
        }
        self.pad_focus_move(event_loop, button, focus, targets);
    }

    /// Move a focused slider's handle by `by` GUI pixels (one pixel is
    /// vanilla's arrow-key step, `value + 1 / (width - 8)`), as a click there.
    fn pad_nudge(&mut self, event_loop: &dyn EventLoopCtl, focus: Target, by: f32) {
        let Kind::Slider { value, track } = focus.kind else {
            return;
        };
        let from = match self.pad.menu.nudge {
            Some((rect, x)) if rect == focus.rect => x,
            _ => gamepad::slider_x(track, value),
        };
        let x = (from + by).clamp(track[0], track[0] + track[1]);
        self.pad.menu.nudge = Some((focus.rect, x));
        self.pad_click_at(event_loop, (x, focus.center().1), false);
    }

    /// Step the focus that way: vanilla's arrow-key navigation; past a
    /// list's last visible row, the list scrolls and the step goes on once
    /// the next frame is drawn.
    fn pad_focus_move(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        focus: Target,
        targets: &[Target],
    ) {
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        let vertical = matches!(button, Button::DPadUp | Button::DPadDown);
        let step = if vertical {
            gamepad::focus_step_strict(focus.rect, button, &rects)
        } else {
            gamepad::focus_step_rect(focus.rect, button, &rects)
        };
        match step {
            Some(i) => self.pad_focus_on(event_loop, targets[i]),
            None if vertical => {
                // Off the visible rows of a list: scroll it and try again on
                // the next frame.
                let up = button == Button::DPadUp;
                self.pad_wheel(event_loop, 0.0, if up { 1.0 } else { -1.0 });
                self.pad.menu.pending = Some(Pending {
                    button,
                    from: focus.rect,
                    generation: widgets::generation(),
                    tries: 1,
                    before: rects,
                });
            }
            None => {}
        }
    }

    fn pad_focus_on(&mut self, event_loop: &dyn EventLoopCtl, target: Target) {
        let left = self.pad.menu.focus.replace(target);
        self.pad.menu.pending = None;
        self.pad.menu.nudge = None;
        if !self.pad.menu.held[0] {
            self.pad_pointer_to(event_loop, target.center());
        }
        if self.pad.menu.keyboard {
            // The key itself was narrated (`narrate_input`).
            self.key_focus_landed(event_loop, left, target);
            return;
        }
        // The screen's narration hears it as an arrow key moving the focus.
        self.narration.set_input(InputType::KeyboardArrow);
        self.narration.keyboard_action();
    }

    /// The controller's focus ring in focus mode, for the screen's narration:
    /// the focused widget.
    pub(super) fn pad_focus_rect(&self) -> Option<[f32; 4]> {
        let focus = self
            .pad
            .menu
            .focus
            .filter(|_| self.pad_in_menu() && self.pad_focus_mode());
        focus.map(|t| t.rect)
    }

    /// A bumper: the next or previous tab, wrapping (creative keeps its own
    /// order; other screens register their tabs).
    fn pad_tab(&mut self, event_loop: &dyn EventLoopCtl, next: bool) {
        if self.screen == Screen::Creative && self.title.is_none() {
            self.pad_creative_tab(next);
            return;
        }
        let tabs: Vec<(Rect, bool)> = self
            .pad_targets()
            .into_iter()
            .filter_map(|t| match t.kind {
                Kind::Tab(selected) => Some((t.rect, selected)),
                _ => None,
            })
            .collect();
        let Some(i) = gamepad::tab_step(&tabs, next) else {
            return;
        };
        let rect = tabs[i].0;
        let back = self.pad.menu.pos;
        self.pad_click_at(
            event_loop,
            (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0),
            false,
        );
        if self.pad_focus_mode() {
            // The page changed under the old focus: start again on the new one.
            self.pad.menu.focus = Some(Target {
                rect,
                kind: Kind::Tab(true),
            });
        } else if let Some(at) = back {
            self.pad_pointer_to(event_loop, at);
        }
    }

    /// Select the next or previous creative tab (wrapping), as clicking it does. A tab on
    /// another page of mods' tabs shows its page: clicking where it would be would hit the
    /// tab drawn there on this page.
    fn pad_creative_tab(&mut self, next: bool) {
        let count = self.creative_screen.tabs.len();
        if count == 0 {
            return;
        }
        let selected = self.creative_screen.selected;
        let tab = if next {
            (selected + 1) % count
        } else {
            (selected + count - 1) % count
        };
        super::select_creative_tab(&mut self.creative_screen, &mut self.inv_clicks, tab);
        self.refresh_saved_hotbars();
        self.inv_drag = None;
        self.last_inv_click = None;
    }

    /// Once per poll in a menu: the stick moves the cursor (pointer) or
    /// steps focus, the right stick scrolls, a held direction repeats, focus
    /// follows its widget from frame to frame, and the OS pointer hides
    /// while the pad drives.
    pub(super) fn pad_menu_frame(&mut self, event_loop: &dyn EventLoopCtl) {
        let now = Instant::now();
        let seconds = self
            .pad
            .menu
            .last_poll
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        self.pad.menu.last_poll = Some(now);
        self.key_focus_frame();
        let in_menu =
            self.pad_in_menu() && (self.settings.controller.enabled || self.pad.menu.keyboard);
        if !in_menu {
            let menu = &mut self.pad.menu;
            menu.keyboard = false;
            menu.slider_edit = false;
            menu.open_polls = 0;
            menu.pos = None;
            menu.repeat = None;
            menu.held = [false; 2];
            menu.focus = None;
            menu.swapped = false;
            menu.pending = None;
            menu.nudge = None;
            menu.osk = None;
            menu.osk_dismissed = false;
            self.pad_show_os_pointer(true);
            return;
        }
        self.pad.menu.open_polls = self.pad.menu.open_polls.saturating_add(1);
        // A real mouse move hands the pointer back. The keyboard's focus
        // stays where the next Tab or arrow goes on from.
        if let Some(at) = self.pad.menu.pos
            && ((at.0 - self.cursor.0).abs() > 0.5 || (at.1 - self.cursor.1).abs() > 0.5)
        {
            self.pad.menu.pos = None;
            let focus = self.pad.menu.focus.take();
            if std::mem::take(&mut self.pad.menu.keyboard) {
                self.pad.menu.origin = focus.map(|t| t.rect);
            }
            self.pad.in_use = false;
        }
        // Focus lands on the menu as it opens when the pad was in use.
        if self.pad.in_use && self.pad.menu.pos.is_none() && self.pad.menu.open_polls == 3 {
            self.pad_take_pointer(event_loop);
        }
        let focus_mode = self.pad_focus_mode();
        let targets = if focus_mode {
            self.pad_targets()
        } else {
            Vec::new()
        };
        let focusing = focus_mode && !targets.is_empty();
        if focusing && !self.key_focus_restart(event_loop, &targets) {
            self.pad_follow_focus(event_loop, &targets);
        }
        let (move_deadzone, look_deadzone) = (
            self.settings.controller.move_deadzone,
            self.settings.controller.look_deadzone,
        );
        let state = self.pad.state;
        if focusing || self.pad.menu.osk.is_some() {
            // The stick steps like the D-pad.
            let dir =
                gamepad::stick_direction(gamepad::radial(state.left, move_deadzone), STICK_STEP);
            if dir != self.pad.menu.stick {
                if let Some((b, ..)) = self.pad.menu.repeat
                    && Some(b) == self.pad.menu.stick
                {
                    self.pad.menu.repeat = None;
                }
                self.pad.menu.stick = dir;
                if let Some(button) = dir {
                    if self.pad.menu.pos.is_none() && self.pad.menu.osk.is_none() {
                        self.pad_take_pointer(event_loop);
                    } else {
                        self.pad.menu.repeats = 0;
                        self.pad_step(event_loop, button);
                    }
                    self.pad.menu.repeat = Some((button, now, false));
                }
            }
        } else {
            self.pad.menu.stick = None;
            let (dx, dy) = gamepad::cursor_delta(state.left, move_deadzone, seconds);
            if dx != 0.0 || dy != 0.0 {
                self.pad_take_pointer(event_loop);
                let at = self.cursor;
                self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
                self.pad.menu.moving = true;
            } else if self.pad.menu.moving {
                // Let go: settle onto a slot or button close by.
                self.pad.menu.moving = false;
                let centres: Vec<(f32, f32)> =
                    self.pad_targets().iter().map(Target::center).collect();
                if let Some(i) = gamepad::nearest(self.cursor, &centres, 10.0) {
                    self.pad_pointer_to(event_loop, centres[i]);
                }
            }
        }
        let scroll = gamepad::radial(state.right, look_deadzone);
        let wheel = &mut self.pad.menu.wheel;
        wheel[0] += scroll[0] * SCROLL_SPEED * seconds;
        wheel[1] += scroll[1] * SCROLL_SPEED * seconds;
        if scroll == [0.0, 0.0] {
            *wheel = [0.0, 0.0];
        }
        let lines = [wheel[0].trunc(), wheel[1].trunc()];
        if lines != [0.0, 0.0] {
            wheel[0] -= lines[0];
            wheel[1] -= lines[1];
            // Stick right scrolls right: the wheel's x runs the other way.
            self.pad_wheel(event_loop, -lines[0], lines[1]);
        }
        if let Some((button, since, repeating)) = self.pad.menu.repeat {
            let wait = if repeating {
                REPEAT_EVERY
            } else {
                REPEAT_DELAY
            };
            if now.duration_since(since).as_secs_f32() >= wait {
                self.pad.menu.repeat = Some((button, now, true));
                self.pad.menu.repeats = self.pad.menu.repeats.saturating_add(1);
                self.pad_step(event_loop, button);
            }
        }
        self.pad_auto_keyboard();
        // Vanilla's keyboard focus leaves the mouse pointer alone.
        self.pad_show_os_pointer(self.pad.menu.pos.is_none() || self.pad.menu.keyboard);
    }

    /// Focus mode, each poll: find the focused widget in the new frame (a
    /// list scrolled, a page changed), finish a step that scrolled a list,
    /// and keep the hidden pointer on the focused widget.
    fn pad_follow_focus(&mut self, event_loop: &dyn EventLoopCtl, targets: &[Target]) {
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        let ready = self
            .pad
            .menu
            .pending
            .as_ref()
            .map(|p| widgets::generation() > p.generation);
        if ready == Some(true)
            && let Some(p) = self.pad.menu.pending.take()
        {
            // A step scrolled a list: carry on from where the focused row
            // moved, or take the vague pass when nothing scrolled.
            let step = match gamepad::scroll_shift(&p.before, &rects) {
                Some(dy) => {
                    let mut from = p.from;
                    from[1] += dy;
                    self.pad.menu.focus = gamepad::refind(from, &rects, 2.0).map(|i| targets[i]);
                    gamepad::focus_step_strict(from, p.button, &rects).map(|i| (i, Some(from)))
                }
                None => gamepad::focus_step_vague(p.from, p.button, &rects).map(|i| (i, None)),
            };
            match step {
                Some((i, _)) => self.pad_focus_on(event_loop, targets[i]),
                None => {
                    if let Some(dy) = gamepad::scroll_shift(&p.before, &rects)
                        && p.tries < SCROLL_TRIES
                    {
                        let up = p.button == Button::DPadUp;
                        self.pad_wheel(event_loop, 0.0, if up { 1.0 } else { -1.0 });
                        let mut from = p.from;
                        from[1] += dy;
                        self.pad.menu.pending = Some(Pending {
                            from,
                            generation: widgets::generation(),
                            tries: p.tries + 1,
                            before: rects.clone(),
                            ..p
                        });
                    }
                }
            }
        } else if ready.is_none() {
            if let Some(focus) = self.pad.menu.focus {
                self.pad.menu.focus = gamepad::refind(focus.rect, &rects, 12.0).map(|i| targets[i]);
                if self.pad.menu.focus.is_none() && self.pad.menu.pos.is_some() {
                    // The widget went away (a page changed): start again.
                    let first = self.pad_first_target(targets);
                    if self.pad.menu.keyboard
                        && let Some(first) = first
                    {
                        // Screen.setInitialFocus after a keyboard press.
                        self.pad_focus_on(event_loop, first);
                    } else {
                        self.pad.menu.focus = first;
                    }
                }
            } else if self.pad.menu.pos.is_some() {
                // The pad drives but nothing is focused (switched from the pointer).
                let centres: Vec<(f32, f32)> = targets.iter().map(Target::center).collect();
                self.pad.menu.focus =
                    gamepad::nearest(self.cursor, &centres, f32::INFINITY).map(|i| targets[i]);
            }
        }
        let Some(focus) = self.pad.menu.focus else {
            return;
        };
        if self.pad.menu.pos.is_none() || self.pad.menu.held.iter().any(|h| *h) {
            return;
        }
        let at = match self.pad.menu.nudge {
            Some((rect, x)) if rect == focus.rect => (x, focus.center().1),
            _ => focus.center(),
        };
        // A text field the keyboard focused keeps the pointer where its
        // click put the caret.
        let [x, y, w, h] = focus.rect;
        let inside = (x..x + w).contains(&self.cursor.0) && (y..y + h).contains(&self.cursor.1);
        if self.pad.menu.keyboard && focus.kind == Kind::Field && inside {
            return;
        }
        if (at.0 - self.cursor.0).abs() > 0.5 || (at.1 - self.cursor.1).abs() > 0.5 {
            self.pad_pointer_to(event_loop, at);
        }
    }

    fn pad_show_os_pointer(&mut self, show: bool) {
        if self.pad.menu.os_hidden != show {
            return;
        }
        self.pad.menu.os_hidden = !show;
        // Controller menus leave the native pointer to its owning player.
        if !self.keyboard_mouse {
            return;
        }
        if let Some(window) = &self.window {
            window.set_cursor_visible(show && !self.captured);
        }
    }

    /// `pad status`: the menu mode, the focused target and every target.
    pub(super) fn pad_menu_status(&self) -> serde_json::Value {
        let kind = |k: Kind| match k {
            Kind::Button => "button",
            Kind::Slider { .. } => "slider",
            Kind::Field => "field",
            Kind::Row => "row",
            Kind::Slot => "slot",
            Kind::Tab(true) => "tab*",
            Kind::Tab(false) => "tab",
        };
        let target = |t: &Target| serde_json::json!({ "rect": t.rect, "kind": kind(t.kind) });
        serde_json::json!({
            "mode": if self.pad_focus_mode() { "focus" } else { "pointer" },
            "focus": self.pad.menu.focus.as_ref().map(target),
            "keyboard": self.pad.menu.osk.is_some(),
            "targets": self.pad_targets().iter().map(target).collect::<Vec<_>>(),
        })
    }

    // ---- the keyboard: vanilla's focus navigation (`key_focus`) ----

    /// The keyboard drives the open menu's focus.
    pub(super) fn key_focus_driving(&self) -> bool {
        self.pad.menu.keyboard && self.pad_in_menu()
    }

    /// A pointer event the keyboard sends (a press, the hidden pointer on
    /// the focused widget) is being handled: the screen's narration doesn't
    /// take it for the mouse (`InputType` stays the keyboard's).
    pub(super) fn key_focus_quiet(&self) -> bool {
        self.pad.menu.synthetic && self.pad.menu.keyboard
    }

    /// The screen keeps the navigation keys for itself: typing in chat (Tab
    /// completes a command), a sign, a book, the creative, recipe-book and
    /// mods searches, the anvil's name, a command block, a dialog, a key or
    /// button being bound, the credits, the on-screen keyboard.
    fn key_focus_blocked(&self) -> bool {
        let playing_screen = |screen| self.title.is_none() && self.screen == screen;
        self.pad.menu.osk.is_some()
            || playing_screen(Screen::Chat)
            || self.sign_edit.is_some()
            || matches!(self.book, Some(super::books::BookScreen::Edit { .. }))
            || (playing_screen(Screen::Creative) && self.creative_screen.tab().search)
            || self.recipe_search_focused()
            || self.mod_browser_typing()
            || self.station_typing()
            || self.screen == Screen::CommandBlock
            || self.screen == Screen::Win
            || self.dialog_open()
            || (self.screen == Screen::Options
                && (self.options_waiting_for_key()
                    || self.ui.options.selected_pad_action.is_some()))
    }

    /// A key pressed in a menu, before the screen takes it
    /// (`Screen.keyPressed`): Tab, Shift+Tab and the arrows move the focus,
    /// Enter, Space and keypad Enter press the focused widget; the focused
    /// widget's own keys come first (a text field's caret, a slider's
    /// value). Whether the key was used; when not, the screen has it.
    pub(super) fn key_focus_key(&mut self, event_loop: &dyn EventLoopCtl, key: KeyCode) -> bool {
        let Some(nav) = key_focus::nav_of(key, self.has_shift_down()) else {
            return false;
        };
        if !self.pad_in_menu() || self.key_focus_blocked() {
            return false;
        }
        let targets = self.key_targets();
        if targets.is_empty() {
            return false;
        }
        let find = |rect: Rect| {
            targets
                .iter()
                .find(|t| t.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5))
                .copied()
        };
        // The focus: the keyboard's or the controller's, else what a click
        // focused (a slider a click focused changes with the arrows).
        let mut slider_edit = self.pad.menu.slider_edit;
        let mut focus = self
            .pad
            .menu
            .focus
            .filter(|_| self.pad_focus_mode())
            .and_then(|f| find(f.rect));
        if focus.is_none()
            && let Some(clicked) = self.pad.menu.origin.and_then(find)
        {
            focus = Some(clicked);
            slider_edit = true;
        }
        let ctx = key_focus::Context {
            slider_edit,
            typing: self.edit_box_focused(),
            rows_take_enter: self.title.as_ref().is_some_and(|t| t.list_focused()),
            gui: self.gui_size(),
        };
        let act = key_focus::act(nav, focus, &targets, ctx);
        if act == Act::Pass {
            return false;
        }
        if !self.pad.menu.keyboard {
            self.pad.menu.key_screen = self.key_screen();
            self.pad.menu.key_restart = None;
        }
        let menu = &mut self.pad.menu;
        menu.keyboard = true;
        menu.origin = None;
        menu.focus = focus;
        menu.slider_edit = slider_edit;
        if menu.pos.is_none() {
            menu.pos = Some(self.cursor);
        }
        match (act, focus) {
            (Act::Focus(i), _) => {
                self.pad_focus_on(event_loop, targets[i]);
                self.pad.menu.slider_edit = key_focus::slider_edit_on_focus(nav);
            }
            (Act::Step(button), Some(focus)) => {
                self.pad_focus_move(event_loop, button, focus, &targets);
            }
            (Act::Press, Some(focus)) => {
                // AbstractButton.keyPressed: onPress, with its click sound.
                self.pad_click_at(event_loop, focus.center(), false);
            }
            (Act::ToggleSlider, _) => self.pad.menu.slider_edit = !slider_edit,
            (Act::Nudge(sign), Some(focus)) => self.pad_nudge(event_loop, focus, sign),
            _ => {}
        }
        // The narration reads a focused slider's `canChangeValue` from the
        // input type (Tab: it can, an arrow: it can't); keep them in step
        // after Enter flipped it or a click let it.
        if let Some(Target {
            kind: Kind::Slider { .. },
            ..
        }) = self.pad.menu.focus
        {
            self.narration.set_input(if self.pad.menu.slider_edit {
                InputType::KeyboardTab
            } else {
                InputType::KeyboardArrow
            });
        }
        true
    }

    /// Which screen is open, as far as the keyboard's focus cares.
    fn key_screen(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.screen).hash(&mut hasher);
        if self.screen == Screen::Options {
            std::mem::discriminant(&self.ui.options.page()).hash(&mut hasher);
        }
        self.title
            .as_ref()
            .map(|t| t.screen_key())
            .hash(&mut hasher);
        (self.sign_edit.is_some(), self.book.is_some()).hash(&mut hasher);
        (
            self.link_prompt.is_some(),
            self.server_packs.prompt.is_some(),
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    /// Each poll while the keyboard has the focus: another screen opened
    /// (a pressed button opened it), so once it is drawn the focus starts on
    /// its first widget (`Screen.setInitialFocus` after a key). Whether the
    /// focus is waiting for that or just started again.
    fn key_focus_restart(&mut self, event_loop: &dyn EventLoopCtl, targets: &[Target]) -> bool {
        if !self.pad.menu.keyboard {
            return false;
        }
        let screen = self.key_screen();
        if screen != self.pad.menu.key_screen {
            self.pad.menu.key_screen = screen;
            self.pad.menu.key_restart = Some(widgets::generation());
            self.pad.menu.pending = None;
        }
        let Some(changed_at) = self.pad.menu.key_restart else {
            return false;
        };
        // Two frames on: the first may have been drawn before the click.
        if widgets::generation() > changed_at + 1 {
            self.pad.menu.key_restart = None;
            self.pad.menu.focus = None;
            if let Some(first) = self.pad_first_target(targets) {
                self.pad_focus_on(event_loop, first);
            }
        }
        true
    }

    /// The keyboard's focus moved from `left` to `target`
    /// (`ComponentPath.applyFocus`): a slider's arrows don't change it yet,
    /// a text field takes the typing (a click at its end, where Tab leaves
    /// the caret) and the one left behind lets go of it, and a world row is
    /// selected (`AbstractSelectionList.setFocused`).
    fn key_focus_landed(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        left: Option<Target>,
        target: Target,
    ) {
        self.pad.menu.slider_edit = false;
        if left.is_some_and(|l| l.rect == target.rect) {
            return;
        }
        if left.is_some_and(|l| l.kind == Kind::Field)
            && target.kind != Kind::Field
            && let Some(title) = self.title.as_mut()
        {
            title.blur_field();
        }
        match target.kind {
            Kind::Field => {
                self.pad_click_at(event_loop, key_focus::field_click(target.rect), false);
            }
            Kind::Row => {
                let size = self.gui_size();
                if let Some(title) = self.title.as_mut() {
                    title.focus_world_row(target.center(), size);
                }
            }
            _ => {}
        }
    }

    /// The real mouse, before the screen takes it: where it is, and a press
    /// or a scroll hands it the pointer back from the keyboard, at the
    /// place the mouse left it (`setLastInputType(MOUSE)`).
    pub(super) fn key_focus_mouse(&mut self, event_loop: &dyn EventLoopCtl, event: &WindowEvent) {
        if self.pad.menu.synthetic {
            return;
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pad.menu.real_pointer = Some(*position);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => self.key_focus_to_mouse(event_loop),
            WindowEvent::MouseWheel { .. } => self.key_focus_to_mouse(event_loop),
            _ => {}
        }
    }

    /// Only a press accepted by ScreenEvent.Pre may focus the widget under it.
    pub(super) fn key_focus_mouse_press(&mut self, event: &WindowEvent) {
        if !self.pad.menu.synthetic
            && self.pad_in_menu()
            && matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                }
            )
        {
            let (cx, cy) = self.cursor;
            self.pad.menu.origin = self
                .key_targets()
                .into_iter()
                .find(|t| {
                    let [x, y, w, h] = t.rect;
                    cx >= x && cy >= y && cx < x + w && cy < y + h
                })
                .map(|t| t.rect);
        }
    }

    /// The keyboard lets go of the pointer: it goes back to the real mouse.
    fn key_focus_to_mouse(&mut self, event_loop: &dyn EventLoopCtl) {
        if !std::mem::take(&mut self.pad.menu.keyboard) {
            return;
        }
        let menu = &mut self.pad.menu;
        menu.origin = menu.focus.take().map(|t| t.rect);
        menu.pos = None;
        menu.pending = None;
        menu.nudge = None;
        let real = menu.real_pointer;
        if let (Some(position), Some(id)) = (real, self.window.as_ref().map(|w| w.id())) {
            self.pad_synthetic_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position,
                },
            );
        }
    }

    /// Each poll: what a click focused is forgotten once it is gone (the
    /// screen changed), and out of menus.
    fn key_focus_frame(&mut self) {
        let Some(origin) = self.pad.menu.origin else {
            return;
        };
        let there = self.pad_in_menu()
            && self
                .key_targets()
                .iter()
                .any(|t| t.rect.iter().zip(origin).all(|(a, b)| (a - b).abs() < 0.5));
        if !there {
            self.pad.menu.origin = None;
        }
    }

    /// The keyboard focus the title had while resources loaded
    /// (`StartupMenu`), carried into the game's.
    pub(super) fn key_focus_adopt(&mut self, focus: Target, slider_edit: bool) {
        let screen = self.key_screen();
        let menu = &mut self.pad.menu;
        menu.key_screen = screen;
        menu.key_restart = None;
        menu.keyboard = true;
        menu.focus = Some(focus);
        menu.slider_edit = slider_edit;
        menu.pos = Some(self.cursor);
    }

    // ---- text fields ----

    /// A text field has keyboard focus: chat, a sign, a book being written,
    /// a search box, the anvil's name, the title screens' and the options'
    /// fields, the LAN port.
    fn pad_typing(&self) -> bool {
        (self.screen == Screen::Chat && self.title.is_none())
            || self.sign_edit.is_some()
            || matches!(self.book, Some(super::books::BookScreen::Edit { .. }))
            || (self.screen == Screen::Creative && self.creative_screen.tab().search)
            || self.recipe_search_focused()
            || self.station_typing()
    }

    /// Open the on-screen keyboard over the focused field: Horizon's, or
    /// Steam's on a Steam Deck (`steam://open/keyboard`, which types into
    /// the window like a real keyboard).
    fn pad_open_keyboard(&mut self) {
        // Without xdg-open (macOS, Windows), Horizon's keyboard instead.
        if gamepad::steam_keyboard(self.settings.controller.keyboard)
            && std::process::Command::new("xdg-open")
                .arg("steam://open/keyboard")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
        {
            return;
        }
        self.pad.menu.osk = Some(Keyboard::default());
    }

    /// Chat or a sign opened by the pad in focus mode: the keyboard opens
    /// with it (once per screen; B closes it for good).
    fn pad_auto_keyboard(&mut self) {
        let menu = &self.pad.menu;
        if menu.osk.is_none()
            && !menu.osk_dismissed
            && self.pad.in_use
            && menu.open_polls >= 3
            && self.pad_focus_mode()
            && (self.screen == Screen::Chat || self.sign_edit.is_some())
        {
            self.pad.menu.osk_dismissed = true;
            self.pad_open_keyboard();
        }
    }

    /// What a key of the on-screen keyboard does: the same as a typed key in
    /// the field that has focus.
    fn pad_osk_output(&mut self, out: osk::Output) {
        use osk::Output;
        let (key, text) = match &out {
            Output::Text(t) => (None, Some(t.as_str())),
            Output::Backspace => (Some(KeyCode::Backspace), None),
            Output::Enter => (Some(KeyCode::Enter), None),
            Output::Close => {
                self.pad.menu.osk = None;
                self.pad.menu.osk_dismissed = true;
                return;
            }
        };
        let backspace = key == Some(KeyCode::Backspace);
        if self.screen == Screen::Chat && self.title.is_none() {
            match text {
                Some(t) => self.chat_type(t),
                None => self.chat_key_code(key.expect("a key"), None),
            }
        } else if self.sign_edit.is_some() {
            self.sign_input(key, text);
        } else if self.book.is_some() {
            self.book_input(key, text);
        } else if self.screen == Screen::Creative && self.creative_screen.tab().search {
            if backspace {
                self.creative_search_backspace(false);
            } else if key != Some(KeyCode::Enter) {
                self.creative_search_edit(text);
            }
        } else if self.recipe_search_focused() {
            if key != Some(KeyCode::Enter) {
                self.recipe_text(text, backspace);
            }
        } else if self.station_typing() {
            if key != Some(KeyCode::Enter) {
                self.station_text(text, backspace);
            }
        } else if self.screen == Screen::WorldOptions {
            if key != Some(KeyCode::Enter) {
                self.world_options_text(text);
            }
        } else if self.screen == Screen::Options {
            let result = match text {
                Some(t) => {
                    self.ui.options.type_text(&self.gui.atlas, t);
                    Ok(())
                }
                None => self.options_key(key.expect("a key"), None),
            };
            if let Err(e) = result {
                log::warn!("Horizon: on-screen keyboard: {e}");
            }
        } else if self.title.is_some() {
            match key {
                Some(KeyCode::Enter) => {
                    if let Some(t) = self.title.as_mut() {
                        t.confirm_edit();
                    }
                }
                Some(KeyCode::Backspace) => self.title_edit(None, true),
                _ => self.title_edit(text, false),
            }
        }
        // Enter ends the typing; a field that closed with it closes the keyboard.
        if key == Some(KeyCode::Enter)
            || (!self.pad_typing()
                && self.title.is_none()
                && !matches!(self.screen, Screen::Options | Screen::WorldOptions))
        {
            self.pad.menu.osk = None;
        }
    }

    // ---- drawing ----

    /// The pad's cursor (pointer mode) or focus highlight (focus mode), and
    /// the on-screen keyboard, over everything else.
    pub(super) fn draw_pad_cursor(&self, batch: &mut HudBatch) {
        if !self.pad_in_menu() {
            return;
        }
        if let Some(keyboard) = &self.pad.menu.osk {
            let at_top = self
                .pad
                .menu
                .focus
                .is_some_and(|f| f.center().1 > batch.height / 2.0)
                || self.screen == Screen::Chat;
            let mut ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, batch, self.cursor);
            keyboard.draw(&mut ui, at_top, self.ui.options.pad_style);
            ui.finish();
            return;
        }
        let Some((x, y)) = self.pad.menu.pos else {
            return;
        };
        if self.pad_focus_mode()
            && let Some(focus) = self.pad.menu.focus
        {
            self.draw_pad_focus(batch, focus);
            return;
        }
        // An arrow, outlined, pointing up-left at (x, y).
        let atlas = &self.gui.atlas;
        let (x, y) = (x.floor(), y.floor());
        const ROWS: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 4, 2, 1];
        for (i, &w) in ROWS.iter().enumerate() {
            let row = y + i as f32;
            batch.fill(
                atlas,
                x - 1.0,
                row,
                f32::from(w) + 2.0,
                1.0,
                rgba(0, 0, 0, 255),
            );
        }
        batch.fill(atlas, x - 1.0, y + 10.0, 2.0, 1.0, rgba(0, 0, 0, 255));
        for (i, &w) in ROWS.iter().enumerate().skip(1).take(8) {
            let row = y + i as f32;
            batch.fill(
                atlas,
                x,
                row,
                f32::from(w) - 1.0,
                1.0,
                rgba(255, 255, 255, 255),
            );
        }
    }

    /// The focus highlight, as vanilla's keyboard focus looks: buttons,
    /// sliders and slots already draw their highlighted sprite under the
    /// (hidden) pointer, and list rows and fields get vanilla's white
    /// one-pixel outline (`AbstractSelectionList`'s focused selection,
    /// `EditBox`'s highlighted border).
    fn draw_pad_focus(&self, batch: &mut HudBatch, focus: Target) {
        key_focus::draw_outline(&self.gui.atlas, batch, focus);
    }
}
