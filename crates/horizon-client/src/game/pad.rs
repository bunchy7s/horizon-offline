//! Controllers through gilrs (Horizon's own; docs/controllers.md): the
//! active pad's state each loop iteration, its button edges dispatched as
//! the vanilla key mappings they are bound to, the left stick as an analog
//! move vector and the right stick turning the view. `horizon-ctl pad`
//! injects a virtual pad on top of the real one.
use super::event_loop::EventLoopCtl;
use super::{Game, Screen};
use horizon_client::{
    Result,
    gamepad::{self, Button, Edges, GlyphStyle, HealthWatch, PadState, Rumble},
    keybinds::{KeyBinds, key_code, mouse_button},
    keymapping::SNEAK,
};
use serde_json::{Value, json};
use std::time::Instant;
use winit::keyboard::KeyCode;

pub(super) const HELP: &[&str] = &[
    "pad BUTTON down|up|tap         a virtual controller button (south east west north lb rb lt rt select start ls rs up down left right)",
    "pad stick left|right X Y       tilt a virtual stick (-1..1, up positive); 0 0 centres it",
    "pad trigger left|right VALUE   pull a virtual trigger (0..1)",
    "pad release                    centre and release the virtual controller",
    "pad status                     connected controllers, the active one and its state",
];

/// The controllers and the state the game last acted on.
#[derive(Default)]
pub(super) struct Pads {
    /// Splitscreen: this player's pad, read by `PadRouter` for every game, instead of the pads.
    pub(super) feed: Option<PadState>,
    /// The real controllers and the DualSense's output.
    #[cfg(feature = "controllers")]
    devices: hardware::Devices,
    /// The virtual pad of `horizon-ctl pad`.
    injected: PadState,
    /// The virtual pad was in use at the last poll.
    virtual_input: bool,
    /// Buttons a `pad ... tap` releases at the next poll.
    taps: Vec<Button>,
    edges: Edges,
    /// The game tick each button went down in, and releases held back until
    /// a tick has passed: vanilla samples keys once per tick, so a click
    /// shorter than that (a quick stick click for sprint) would be lost.
    pressed_at: [Option<u64>; 16],
    late_releases: Vec<Button>,
    /// The merged state of the last poll.
    pub(super) state: PadState,
    /// The pad drove the game last (the mouse and keyboard hand control back).
    pub(super) in_use: bool,
    pub(super) style: GlyphStyle,
    /// The cursor the pad drives in menus.
    pub(super) menu: super::pad_menu::MenuCursor,
    /// The last vibration asked for, for `pad status`.
    last_rumble: Option<(&'static str, gamepad::Pulse)>,
    /// The player's health between polls (a drop is damage taken).
    health: HealthWatch,
}

#[cfg(feature = "controllers")]
mod hardware;
pub(super) use hardware::{PadId, PadRouter};

/// What a controller did since the last frame.
#[cfg_attr(not(feature = "controllers"), allow(dead_code))]
pub(in crate::game) enum PadEvent {
    Press(PadId, Button),
    /// Disconnected (out of battery, unplugged, out of range).
    Gone(PadId),
    /// Connected again under the same id (Windows keeps it; macOS gives a new one). On Linux
    /// gilrs gives a new device the slot of a disconnected one of the same model, so there this
    /// can be another controller of that model.
    Back(PadId),
}

/// A build without the `controllers` feature (the server build, which must not need libudev): no
/// hardware pads, only `horizon-ctl pad`'s virtual one.
#[cfg(not(feature = "controllers"))]
mod hardware {
    use super::{super::Game, Pads};
    use horizon_client::gamepad::{PadState, Pulse, Rumble};
    use serde_json::{Value, json};

    impl Pads {
        pub(super) fn read(&mut self) -> PadState {
            PadState::default()
        }
        pub(in crate::game) fn release_hardware(&mut self) {}
    }

    pub(in crate::game) type PadId = usize;
    #[derive(Default)]
    pub(in crate::game) struct PadRouter {}
    impl PadRouter {
        pub(in crate::game) fn events(&mut self) -> Vec<super::PadEvent> {
            Vec::new()
        }
        pub(in crate::game) fn lend_output(&mut self, _: &mut Pads, _: PadId) {}
        pub(in crate::game) fn reclaim_output(&mut self, _: &mut Pads) {}
        pub(in crate::game) fn connected(&self, _: PadId) -> bool {
            false
        }
        pub(in crate::game) fn uuid(&self, _: PadId) -> [u8; 16] {
            [0; 16]
        }
        pub(in crate::game) fn state(&self, _: PadId) -> PadState {
            PadState::default()
        }
        pub(in crate::game) fn name(&self, _: PadId) -> String {
            String::new()
        }
        pub(in crate::game) fn style(&self, _: PadId) -> horizon_client::gamepad::GlyphStyle {
            horizon_client::gamepad::GlyphStyle::Auto
        }
    }

    impl Game<'_> {
        pub(super) fn pad_output_off(&mut self) {}
        pub(super) fn pad_output_frame(&mut self) {}
        pub(in crate::game) fn pad_close_output(&mut self) {}
        pub(super) fn pad_vibrate(&mut self, _: Rumble, _: Pulse) {}
        pub(super) fn pad_devices_status(&self) -> Value {
            json!({ "backend": false, "controllers": [] })
        }
    }
}

impl Pads {
    /// Include releases delayed until the next simulation tick.
    pub(super) fn holds_action(&self, settings: &gamepad::PadSettings, action: &str) -> bool {
        settings.binding(action).is_some_and(|button| {
            self.edges.is_held(button) || self.late_releases.contains(&button)
        })
    }

    /// A new player entity took over (spawn, respawn, a server joined, a
    /// dimension change): its health is not compared with the old one's.
    pub(super) fn player_replaced(&mut self) {
        self.health.forget();
    }
}

impl Game<'_> {
    /// Once per loop iteration: read the pads and act on button changes.
    pub(super) fn poll_pads(&mut self, event_loop: &dyn EventLoopCtl) {
        let settings = &self.settings.controller;
        if !settings.enabled {
            self.pad_output_off();
            if self.pad.state != PadState::default() {
                self.pad.state = PadState::default();
                for (button, down) in self.pad.edges.release_all() {
                    self.pad_edge(event_loop, button, down);
                }
            }
            // The keyboard's menu focus follows its widget all the same.
            self.pad_menu_frame(event_loop);
            return;
        }
        let hardware = match self.pad.feed {
            Some(state) => state,
            None => self.pad.read(),
        };
        self.ui.options.pad_style = self.settings.controller.glyphs.resolve(self.pad.style);
        let state = hardware.merge(&self.pad.injected);
        self.pad.virtual_input = self.pad.injected != PadState::default();
        for button in std::mem::take(&mut self.pad.taps) {
            self.pad.injected.set(button, false);
        }
        let threshold = self.settings.controller.trigger_threshold;
        let edges = self.pad.edges.update(&state, threshold);
        if state.active(&self.settings.controller) {
            self.last_input = Instant::now();
            self.pad.in_use = true;
        }
        self.pad.state = state;
        self.pad_output_frame();
        // Damage from any source, here or on a server; nothing counts on
        // the title screen.
        let health = self
            .player
            .as_ref()
            .filter(|_| self.title.is_none())
            .map(|p| p.health.health());
        if let Some(damage) = self.pad.health.observe(health) {
            self.pad_rumble(Rumble::Hurt(damage));
        }
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        for button in std::mem::take(&mut self.pad.late_releases) {
            if !ticking || self.pad.pressed_at[button as usize] != Some(self.ticks) {
                self.pad_button_logged(event_loop, button, false);
            } else {
                self.pad.late_releases.push(button);
            }
        }
        for (button, down) in edges {
            self.pad_edge(event_loop, button, down);
        }
        self.pad_menu_frame(event_loop);
    }

    fn pad_edge(&mut self, event_loop: &dyn EventLoopCtl, button: Button, down: bool) {
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        if down {
            // Pressed again before its held-back release went out.
            if let Some(i) = self.pad.late_releases.iter().position(|b| *b == button) {
                self.pad.late_releases.remove(i);
                self.pad_button_logged(event_loop, button, false);
            }
            self.pad.pressed_at[button as usize] = Some(self.ticks);
        } else if ticking && self.pad.pressed_at[button as usize] == Some(self.ticks) {
            self.pad.late_releases.push(button);
            return;
        }
        self.pad_button_logged(event_loop, button, down);
    }

    fn pad_button_logged(&mut self, event_loop: &dyn EventLoopCtl, button: Button, down: bool) {
        if let Err(e) = self.pad_button(event_loop, button, down) {
            log::warn!("Horizon: controller {}: {e}", button.name());
        }
    }

    /// Input from the pad counts while the window has focus, or always for
    /// the virtual pad (a script can't focus the window).
    fn pad_focused(&self) -> bool {
        self.focused || self.pad.virtual_input
    }

    fn pad_button(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if down && !self.pad_focused() {
            return Ok(());
        }
        let playing = !self.pad_in_menu();
        if down && playing && !self.captured && self.focused {
            // Like a click into the window: take the pointer.
            self.capture(true);
        }
        if !playing {
            self.pad_menu_button(event_loop, button, down)?;
            if down {
                return Ok(());
            }
        } else if button == Button::Start {
            return if down { self.pad_back() } else { Ok(()) };
        }
        // Releases always reach the mappings, so nothing stays held when a
        // screen opened in between.
        for action in self.settings.controller.actions_of(button) {
            self.pad_action(action, down)?;
        }
        Ok(())
    }

    /// A bound action: a vanilla mapping acts as its default key or mouse
    /// button, like a rebound keyboard key.
    fn pad_action(&mut self, action: &str, down: bool) -> Result<()> {
        match action {
            gamepad::HOTBAR_PREVIOUS | gamepad::HOTBAR_NEXT => {
                if down && self.screen == Screen::Playing {
                    self.selected =
                        gamepad::hotbar_step(self.selected, action == gamepad::HOTBAR_NEXT);
                }
                Ok(())
            }
            "key.sneak"
                if self.settings.controller.toggle_sneak && !self.settings.toggle_crouch =>
            {
                // A stick click flips sneaking instead of holding it, as
                // vanilla's toggle crouch does: a screen opening releases it
                // and closing restores it (ToggleKeyMapping).
                if !down {
                    return Ok(());
                }
                let screen_open = self.screen != Screen::Playing || self.title.is_some();
                self.toggles.pad_toggle(SNEAK, screen_open);
                self.sync_toggle(SNEAK);
                self.key_pressed(KeyCode::ShiftLeft)
            }
            _ => {
                let default = KeyBinds::default_of(action);
                if let Some(button) = mouse_button(default) {
                    if self.title.is_none() && self.screen == Screen::Playing {
                        self.play_mouse(button, down)?;
                    }
                    Ok(())
                } else if let Some(key) = key_code(default) {
                    self.mapped_key(key, down, false)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// B and Start: what Escape does where the pad is.
    pub(super) fn pad_back(&mut self) -> Result<()> {
        // The options open over the title screen too: they come first, as
        // for the keyboard's Escape.
        if self.screen == Screen::Options {
            return self.options_key(KeyCode::Escape, None);
        }
        if let Some(title) = self.title.as_mut() {
            title.escape();
            self.save_title_options();
            return Ok(());
        }
        if self.sign_edit.is_some() {
            self.close_sign_editor();
            return Ok(());
        }
        if self.book.is_some() {
            self.close_book();
            return Ok(());
        }
        self.key_pressed(KeyCode::Escape)
    }

    /// The left stick as the move vector, while playing.
    pub(super) fn pad_move(&self) -> Option<[f32; 2]> {
        let settings = &self.settings.controller;
        if !settings.enabled || self.pad_in_menu() || !self.pad_focused() {
            return None;
        }
        gamepad::move_vector(self.pad.state.left, settings.move_deadzone)
    }

    /// The right stick turns the view, once per frame (`Entity.turn`).
    pub(super) fn pad_look(&mut self, seconds: f32) {
        let settings = &self.settings.controller;
        if !settings.enabled || self.pad_in_menu() || self.player.is_none() || !self.pad_focused() {
            return;
        }
        // A long frame (a hitch) must not spin the view.
        let (yaw, pitch) = gamepad::look_delta(
            self.pad.state.right,
            settings,
            seconds.min(0.1),
            self.scoping(),
        );
        if yaw == 0.0 && pitch == 0.0 {
            return;
        }
        self.turn_player(f64::from(yaw / 0.15), f64::from(pitch / 0.15));
    }

    /// Vibrate the active pad, when vibration is on and the pad is the one
    /// playing (a pad lying idle while the mouse plays stays still).
    pub(super) fn pad_rumble(&mut self, rumble: Rumble) {
        let settings = &self.settings.controller;
        if !settings.enabled || !settings.vibration || !self.pad.in_use {
            return;
        }
        let Some(pulse) = rumble.pulse() else {
            return;
        };
        self.pad.last_rumble = Some((rumble.name(), pulse));
        self.pad_vibrate(rumble, pulse);
    }

    /// `horizon-ctl pad ...`.
    pub(super) fn pad_command(&mut self, words: &[&str]) -> Result<Value> {
        let arg = |i: usize| -> Result<&str> {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("usage: {}", HELP[0]).into())
        };
        let number = |i: usize| -> Result<f32> {
            let v: f32 = arg(i)?.parse()?;
            if v.is_finite() {
                Ok(v)
            } else {
                Err("not a number".into())
            }
        };
        match arg(1)? {
            "status" => {
                let s = &self.pad.state;
                let held: Vec<&str> = Button::ALL
                    .into_iter()
                    .filter(|b| self.pad.edges.is_held(*b))
                    .map(Button::name)
                    .collect();
                let mut status = json!({
                    "enabled": self.settings.controller.enabled,
                    "glyphs": self.pad.style.name(),
                    "last_rumble": self.pad.last_rumble.map(|(why, p)| json!({
                        "reason": why, "strong": p.strong, "weak": p.weak, "ms": p.millis,
                    })),
                    "in_use": self.pad.in_use,
                    "menu": self.pad_menu_status(),
                    "held": held,
                    "left": s.left,
                    "right": s.right,
                    "triggers": s.triggers,
                });
                if let (Some(status), Value::Object(devices)) =
                    (status.as_object_mut(), self.pad_devices_status())
                {
                    status.extend(devices);
                }
                Ok(status)
            }
            "release" => {
                self.pad.injected = PadState::default();
                self.pad.taps.clear();
                Ok(json!({ "ok": true }))
            }
            "stick" => {
                let v = [number(3)?.clamp(-1.0, 1.0), number(4)?.clamp(-1.0, 1.0)];
                match arg(2)? {
                    "left" => self.pad.injected.left = v,
                    "right" => self.pad.injected.right = v,
                    other => return Err(format!("stick must be left or right, not {other}").into()),
                }
                Ok(json!({ "ok": true }))
            }
            "trigger" => {
                let v = number(3)?.clamp(0.0, 1.0);
                match arg(2)? {
                    "left" => self.pad.injected.triggers[0] = v,
                    "right" => self.pad.injected.triggers[1] = v,
                    other => {
                        return Err(format!("trigger must be left or right, not {other}").into());
                    }
                }
                Ok(json!({ "ok": true }))
            }
            name => {
                let button = Button::parse(name)
                    .ok_or_else(|| format!("unknown pad button {name}; {}", HELP[0]))?;
                match arg(2)? {
                    "down" => self.pad.injected.set(button, true),
                    "up" => self.pad.injected.set(button, false),
                    "tap" => {
                        self.pad.injected.set(button, true);
                        self.pad.taps.push(button);
                    }
                    other => {
                        return Err(
                            format!("pad action must be down, up or tap, not {other}").into()
                        );
                    }
                }
                Ok(json!({ "ok": true }))
            }
        }
    }
}
