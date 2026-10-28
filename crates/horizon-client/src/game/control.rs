//! Commands from the control socket (`horizon-ctl`), run on the game thread.
use super::{Game, Screen};
use horizon_client::{Result, items::ItemStack};
use horizon_core::geometry::Vec3;
use serde_json::{Value, json};
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, event_loop::ActiveEventLoop, keyboard::KeyCode};

const HELP: &[&str] = &[
    "status                         game, player, streaming and renderer state",
    "simulation                     authoritative projectiles in the owner's level",
    "server-hold on|off             hold simulation and player packets at a control-fixture barrier",
    "stall MILLISECONDS             stall the window thread (0..5000), leaving transport alive",
    "key KEY down|up|tap|repeat     press, release or auto-repeat a key (w a s d space shift ctrl e q esc f3 1-9 arrows ...)",
    "release                        release every key and mouse button",
    "mouse left|right|middle down|up|click",
    "look YAW PITCH                 set the view direction in degrees",
    "turn DYAW DPITCH               rotate the view",
    "tp X Y Z                       teleport the player",
    "hotbar 1-9                     select a hotbar slot",
    "give ITEM [COUNT]              add items to the inventory",
    "render-distance N              chunks, 2..64",
    "lod-distance N                 far surface chunks, 0 disables, up to1024",
    "time TICKS                     set the time of day",
    "summon KIND [X Y Z] [baby]     spawn a mob (in front of the player without a position)",
    "mobs [RADIUS]                  list mobs near the player with their AI state",
    "hurtlog                        the hurts taken and melee attacks made since the last call, per player (host and guests)",
    "fill X1 Y1 Z1 X2 Y2 Z2 BLOCK   set a box of loaded blocks",
    "resume                         leave the pause menu",
    "screenshot PATH                write the next frame as PNG",
    "chat TEXT                      send a chat line or /command as if typed",
    "type TEXT                      type into the open chat box (key enter tap sends)",
    "chat-log [N]                   the newest N chat messages (default 10)",
    "options [PAGE]                 open the options (video sound controls keybinds mouse chat accessibility language font packs mods graphics graphics-advanced)",
    "stats [reset]                  frame-time summary since the last reset",
    "bench SECONDS                  reset stats, wait, then reply with the summary",
    "fly X1 Y1 Z1 X2 Y2 Z2 SECONDS [PITCH [raw]]  fly the camera along a line at constant speed, then reply with the frame summary (raw: every frame time too)",
    "open WORLD [SEED] [creative]   open or create a world from the title screen",
    "save                           save the world",
    "save-all                       save the world, rewriting every resident chunk",
    "connect HOST[:PORT] [NAME]     join a multiplayer server (offline mode) from the title screen",
    "chat MESSAGE                   send chat or a /command to the server",
    "disconnect                     leave the server for the server list",
    "respawn                        respawn after death on a server",
    "credits [poem] [SCROLL]        roll the credits (with the end poem), or hold them SCROLL pixels in",
    "quit                           save and exit",
];

/// A straight camera path at constant speed, for repeatable benchmarks: the
/// player is placed on it every frame, facing along it.
pub struct Flight {
    from: Vec3,
    to: Vec3,
    pitch: f32,
    started: Instant,
    seconds: f64,
    /// Reply with every frame time too (`fly ... PITCH raw`).
    raw: bool,
    reply: Sender<String>,
    /// The world's smoothed tick time each frame (the F3 server line): the
    /// server thread's when single player runs on one.
    tick_ms: Vec<f32>,
}

impl Game<'_> {
    /// Move the camera along the running flight; reply when it ends.
    pub(super) fn advance_flight(&mut self) {
        let tick_ms = self
            .integrated
            .as_ref()
            .map_or(self.debug_charts.smoothed_tick_ms, |s| s.smoothed_tick_ms());
        let Some(flight) = self.flight.as_mut() else {
            return;
        };
        flight.tick_ms.push(tick_ms);
        let flight = &*flight;
        let t = (flight.started.elapsed().as_secs_f64() / flight.seconds).min(1.0);
        let (a, b) = (flight.from, flight.to);
        let d = [b.x - a.x, b.y - a.y, b.z - a.z];
        let at = Vec3::new(a.x + d[0] * t, a.y + d[1] * t, a.z + d[2] * t);
        // Minecraft yaw: 0 faces +Z, 90 faces -X.
        self.yaw = (-d[0]).atan2(d[2]).to_degrees() as f32;
        self.pitch = flight.pitch;
        if let Some(player) = self.player.as_mut() {
            player.position = at;
            player.velocity = Vec3::new(0.0, 0.0, 0.0);
            player.fall_distance = 0.0;
            // Stay where the flight ends (creative flight).
            player.flying = true;
            self.previous_position = at;
        }
        if t >= 1.0
            && let Some(flight) = self.flight.take()
        {
            let mut summary = self.frame_stats.summary();
            let n = flight.tick_ms.len().max(1) as f32;
            summary["tick_ms"] = (flight.tick_ms.iter().sum::<f32>() / n).into();
            summary["tick_ms_max"] = flight.tick_ms.iter().copied().fold(0.0, f32::max).into();
            if flight.raw {
                summary["frame_ms"] = self.frame_stats.frame_millis().into();
                summary["cpu_ms"] = self.frame_stats.cpu_millis().into();
                summary["gpu_ms"] = self.frame_stats.gpu_millis().into();
            }
            let _ = flight.reply.send(summary.to_string());
        }
    }
    pub(super) fn poll_control(&mut self, event_loop: &ActiveEventLoop) {
        if let Some((deadline, reply)) = self.pending_bench.take() {
            if Instant::now() >= deadline {
                let _ = reply.send(self.frame_stats.summary().to_string());
            } else {
                self.pending_bench = Some((deadline, reply));
            }
        }
        horizon_client::widgets::end_frame(self.frames);
        self.advance_agent();
        for request in self.ui_wait.ready() {
            self.answer(request, event_loop);
        }
        while let Some(request) = self.control.as_ref().and_then(|c| c.try_recv()) {
            self.agent_command_seen();
            self.answer(request, event_loop);
            if self.pending_save.is_some() {
                break;
            }
        }
        if self.control.is_some() {
            self.watch_events();
        }
        if self.screen == Screen::Options
            && self.ui.options.page() == horizon_client::options_ui::Page::Agent
        {
            self.ui.options.agent_status = self.agent_status_lines();
        }
    }

    fn answer(&mut self, request: horizon_client::control::Request, event_loop: &ActiveEventLoop) {
        let reply = match self.control_command(&request.line, &request.reply, event_loop) {
            Ok(Some(value)) => value,
            // Deferred: the command keeps its own reply channel.
            Ok(None) => return,
            Err(e) => json!({ "error": e.to_string() }),
        };
        let _ = request.reply.send(reply.to_string());
    }

    fn control_command(
        &mut self,
        line: &str,
        reply: &Sender<String>,
        event_loop: &ActiveEventLoop,
    ) -> Result<Option<Value>> {
        let words: Vec<&str> = line.split_whitespace().collect();
        if self.pending_save.is_some() && words.first() != Some(&"quit") {
            return Err("a world save is in progress".into());
        }
        let arg = |i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("missing argument {i} for {}", words[0]))
        };
        let ok = || Ok(Some(json!({ "ok": true })));
        if let Some(value) = self.block_command(&words)? {
            return Ok(Some(value));
        }
        match self.ui_command(&words, line, reply, event_loop)? {
            super::agent::Handled::Reply(value) => return Ok(Some(value)),
            super::agent::Handled::Deferred => return Ok(None),
            super::agent::Handled::No => {}
        }
        if let Some(value) = self.events_command(&words, reply)? {
            return Ok(value);
        }
        if let Some(value) = self.saves_control(&words, line, reply)? {
            return Ok(value);
        }
        if let Some(value) = self.options_control(&words, line)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.lan_control(&words)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.friends_control(&words)? {
            return Ok(Some(value));
        }
        match self.agent_command(&words, line, reply, event_loop)? {
            super::agent::Handled::Reply(value) => return Ok(Some(value)),
            super::agent::Handled::Deferred => return Ok(None),
            super::agent::Handled::No => {}
        }
        match words[0] {
            "help" => Ok(Some(json!({ "commands": HELP
                .iter()
                .chain(super::station_control::HELP)
                .chain(super::blockctl::HELP)
                .chain(super::lan::HELP)
                .chain(super::friends::HELP)
                .chain(super::render_control::HELP)
                .chain(super::sculk::HELP)
                .chain(super::agent::HELP)
                .chain(super::ui_control::HELP)
                .chain(super::options::CONTROL_HELP)
                .chain(super::events::HELP)
                .chain(super::saves_control::HELP)
                .chain(super::pad::HELP)
                .collect::<Vec<_>>() }))),
            "setblock" | "fill" | "particle" | "signedit" => self.render_command(&words),
            "section-draws" => {
                let r = self.renderer.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("flat") => r.set_flat_draws(true)?,
                    Some("indirect") => r.set_flat_draws(false)?,
                    Some(other) => return Err(format!("unknown section draw mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "flat": r.flat_draws() })))
            }
            "section-cull" => {
                let r = self.renderer.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("on") => r.set_section_cull(true)?,
                    Some("off") => r.set_section_cull(false)?,
                    Some(other) => return Err(format!("unknown section cull mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "cull": r.section_cull() })))
            }
            "section-sides" => {
                let r = self.renderer.as_mut().ok_or("no renderer")?;
                match words.get(1).copied() {
                    None => {}
                    Some("on") => r.set_section_sides(true),
                    Some("off") => r.set_section_sides(false),
                    Some(other) => return Err(format!("unknown section sides mode {other}").into()),
                }
                Ok(Some(json!({ "ok": true, "sides": r.section_sides() })))
            }
            "sculk" => Ok(Some(self.sculk_command(&words)?)),
            "status" => Ok(Some(self.control_status())),
            "stall" => {
                let millis: u64 = arg(1)?.parse()?;
                if millis > 5000 {
                    return Err("stall is limited to 5000 milliseconds".into());
                }
                std::thread::sleep(Duration::from_millis(millis));
                ok()
            }
            "simulation" => {
                self.require_world()?;
                let value = if let Some(server) = &self.integrated {
                    server
                        .call(|g| {
                            g.in_owner_level(|g| {
                                json!({ "game_time": g.world.game_time, "projectiles": g.projectiles_status() })
                            })
                        })
                        .ok_or("the integrated server stopped")?
                } else {
                    json!({ "game_time": self.world.game_time, "projectiles": self.projectiles_status() })
                };
                Ok(Some(value))
            }
            "server-hold" => {
                self.require_world()?;
                let held = match arg(1)? {
                    "on" => true,
                    "off" => false,
                    _ => return Err("server-hold on|off".into()),
                };
                if let Some(server) = &self.integrated {
                    server
                        .call(move |g| g.agent.hold_server = held)
                        .ok_or("the integrated server stopped")?;
                } else if self.remote.is_some() {
                    return Err("server-hold requires the world's host".into());
                } else {
                    self.agent.hold_server = held;
                }
                ok()
            }
            "pad" => Ok(Some(self.pad_command(&words)?)),
            "key" => {
                let key = key_code(arg(1)?).ok_or_else(|| format!("unknown key {}", words[1]))?;
                // A server's dialog takes the keys, in configuration too.
                if self.dialog_open() {
                    if arg(2)? != "up" {
                        self.dialog_key(key, None, None)?;
                    }
                    return ok();
                }
                self.require_world()?;
                // The sign editor takes raw presses (Enter, arrows, Backspace, Escape).
                if self.sign_edit.is_some() {
                    if arg(2)? != "up" {
                        self.sign_input(Some(key), None);
                    }
                    return ok();
                }
                // A held edit modifier (`key super down`) and A/C/V/X: the
                // open book's Select All, Copy, Paste or Cut.
                if self.book.is_some() && arg(2)? != "up" {
                    let held = |k: [KeyCode; 2]| k.iter().any(|k| self.keys.contains(k));
                    let edit = if cfg!(target_os = "macos") {
                        held([KeyCode::SuperLeft, KeyCode::SuperRight])
                    } else {
                        held([KeyCode::ControlLeft, KeyCode::ControlRight])
                    };
                    let name = arg(1)?;
                    let letter = name.chars().next().filter(|_| name.len() == 1);
                    let shift = held([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
                    if let Some(shortcut) =
                        horizon_client::clipboard::edit_shortcut(letter, edit, shift, false)
                    {
                        self.book_shortcut(shortcut);
                        return ok();
                    }
                }
                // KeyboardHandler.keyPress: the debug keys see the physical key first.
                let physical = key;
                {
                    use horizon_client::debug_keys::KeyAction;
                    let action = arg(2)?;
                    if action != "up" {
                        self.physical_keys.insert(key);
                        if self.debug_key(key, KeyAction::Press) {
                            if action == "tap" {
                                self.physical_keys.remove(&key);
                                self.debug_key(key, KeyAction::Release);
                            }
                            return ok();
                        }
                    }
                    if action == "up" {
                        self.physical_keys.remove(&key);
                        self.debug_key(key, KeyAction::Release);
                    }
                }
                // Mods' keys (capability `input`) see the key as the keyboard
                // hook does.
                let action = arg(2)?;
                self.mod_key(physical, action != "up", action == "repeat");
                self.native_key_event(physical, action != "up", action == "repeat");
                if action == "tap" {
                    self.mod_key(physical, false, false);
                    self.native_key_event(physical, false, false);
                }
                // Text screens consume raw presses; releases follow key binds.
                let typing = matches!(self.screen, Screen::Chat | Screen::Options);
                let key = if typing && arg(2)? != "up" {
                    key
                } else {
                    match self.settings.keys.translate(key) {
                        Some(Ok(k)) => k,
                        Some(Err(button)) => {
                            let action = arg(2)?;
                            if self.screen == Screen::Playing {
                                if action != "up" {
                                    self.play_mouse(button, true)?;
                                }
                                if action != "down" {
                                    self.play_mouse(button, false)?;
                                }
                            }
                            return ok();
                        }
                        None => return ok(),
                    }
                };
                match arg(2)? {
                    "down" => self.control_key_down(key)?,
                    "up" => {
                        self.keys.remove(&key);
                        self.physical_keys.remove(&key);
                    }
                    // The system's auto-repeat while the key is held.
                    "repeat" => {
                        self.physical_keys.insert(physical);
                        self.mapped_key(key, true, true)?;
                    }
                    "tap" => {
                        self.control_key_down(key)?;
                        self.keys.remove(&key);
                        self.physical_keys.remove(&physical);
                        self.debug_key(physical, horizon_client::debug_keys::KeyAction::Release);
                    }
                    other => {
                        return Err(format!(
                            "key action must be down, up, tap or repeat, not {other}"
                        )
                        .into());
                    }
                }
                ok()
            }
            "release" => {
                self.keys.clear();
                self.physical_keys.clear();
                self.left = false;
                self.right = false;
                ok()
            }
            "mouse" => {
                let button = match arg(1)? {
                    "left" => MouseButton::Left,
                    "right" => MouseButton::Right,
                    "middle" => MouseButton::Middle,
                    other => return Err(format!("unknown mouse button {other}").into()),
                };
                self.require_world()?;
                if self.screen != Screen::Playing {
                    return Err(format!(
                        "mouse needs the playing screen, not {}",
                        self.screen_name()
                    )
                    .into());
                }
                match arg(2)? {
                    "down" => {
                        self.extras_mouse(button, true);
                        self.mod_mouse_key(button, true);
                        self.play_mouse(button, true)?
                    }
                    "up" => {
                        self.extras_mouse(button, false);
                        self.mod_mouse_key(button, false);
                        self.play_mouse(button, false)?
                    }
                    "click" => {
                        self.extras_mouse(button, true);
                        self.mod_mouse_key(button, true);
                        self.extras_mouse(button, false);
                        self.mod_mouse_key(button, false);
                        self.play_mouse(button, true)?;
                        self.play_mouse(button, false)?;
                    }
                    other => {
                        return Err(
                            format!("mouse action must be down, up or click, not {other}").into(),
                        );
                    }
                }
                ok()
            }
            "look" => {
                self.yaw = arg(1)?.parse()?;
                self.pitch = arg(2)?.parse::<f32>()?.clamp(-90.0, 90.0);
                ok()
            }
            "turn" => {
                self.yaw += arg(1)?.parse::<f32>()?;
                self.pitch = (self.pitch + arg(2)?.parse::<f32>()?).clamp(-90.0, 90.0);
                ok()
            }
            "tp" => {
                let to = Vec3::new(arg(1)?.parse()?, arg(2)?.parse()?, arg(3)?.parse()?);
                let player = self.player.as_mut().ok_or("no player yet")?;
                player.position = to;
                player.velocity = Vec3::new(0.0, 0.0, 0.0);
                player.fall_distance = 0.0;
                self.previous_position = to;
                ok()
            }
            "hotbar" => {
                let slot: usize = arg(1)?.parse()?;
                if !(1..=9).contains(&slot) {
                    return Err("hotbar slot must be 1..9".into());
                }
                self.selected = slot - 1;
                ok()
            }
            "give" => {
                self.require_world()?;
                let name = arg(1)?;
                let item = if name.contains(':') {
                    name.to_owned()
                } else {
                    format!("minecraft:{name}")
                };
                let info = self
                    .res
                    .items()
                    .get(&item)
                    .ok_or_else(|| format!("unknown item {item}"))?;
                let count: u32 = words.get(2).map_or(Ok(1), |c| c.parse())?;
                // Single player's inventory is its server thread's player's.
                if let Some(server) = &self.integrated {
                    let command = format!("give @s {item} {count}");
                    server
                        .call(move |g| g.owner_console_command(&command))
                        .ok_or("the integrated server stopped")?;
                    return Ok(Some(json!({ "ok": true, "given": count })));
                }
                let max = info.max_stack;
                let mut given = 0;
                // Stop once the inventory is full.
                while given < count {
                    let n = (count - given).min(max);
                    let rest = self
                        .give(ItemStack {
                            item: item.as_str().into(),
                            count: n,
                            damage: 0,
                            extra: None,
                        })
                        .map_or(0, |r| r.count);
                    given += n - rest;
                    if rest > 0 {
                        break;
                    }
                }
                Ok(Some(json!({ "ok": true, "given": given })))
            }
            "render-distance" => {
                let n: i32 = arg(1)?.parse()?;
                if !(2..=64).contains(&n) {
                    return Err("render distance must be 2..64".into());
                }
                self.settings.render_distance = n;
                self.apply_settings();
                ok()
            }
            "lod-distance" => {
                let n: u32 = arg(1)?.parse()?;
                if n > 1024 {
                    return Err("LOD distance must be 0..1024".into());
                }
                self.settings.horizon.lod_distance = n;
                self.apply_settings();
                ok()
            }
            "time" => {
                let time: i64 = arg(1)?.parse()?;
                self.world.day_time = time;
                // Single player's clock is its server thread's.
                if let Some(server) = &self.integrated {
                    server
                        .call(move |g| g.world.day_time = time)
                        .ok_or("the integrated server stopped")?;
                }
                ok()
            }
            "summon" => {
                self.require_world()?;
                let baby = words.contains(&"baby");
                let nums: Vec<f64> = words[2..].iter().filter_map(|w| w.parse().ok()).collect();
                let at = (nums.len() >= 3).then(|| [nums[0], nums[1], nums[2]]);
                let id = self.summon(arg(1)?, at, baby)?;
                Ok(Some(json!({ "ok": true, "id": id })))
            }
            "mobs" => {
                self.require_world()?;
                let radius: f64 = words.get(1).map_or(Ok(64.0), |r| r.parse())?;
                if let Some(server) = &self.integrated {
                    let center = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
                    let value = server
                        .call(move |g| g.in_owner_level(|g| g.control_mobs_at(radius, center)))
                        .ok_or("the integrated server stopped")?;
                    return Ok(Some(value));
                }
                Ok(Some(self.control_mobs(radius)))
            }
            "hurtlog" => {
                self.require_world()?;
                if let Some(server) = &self.integrated {
                    let value = server
                        .call(|g| g.control_hurt_log())
                        .ok_or("the integrated server stopped")?;
                    return Ok(Some(value));
                }
                Ok(Some(self.control_hurt_log()))
            }
            "resume" => {
                if self.screen == Screen::Paused {
                    self.screen = Screen::Playing;
                }
                ok()
            }
            "screenshot" => {
                let renderer = self.renderer.as_mut().ok_or("no renderer yet")?;
                if self.pending_screenshot.is_some() {
                    return Err("a screenshot is already pending".into());
                }
                arg(1)?;
                // The rest of the line: a path may hold spaces.
                let path = line.trim_start().strip_prefix("screenshot").unwrap_or("");
                renderer.screenshot = Some(path.trim().into());
                self.pending_screenshot = Some(reply.clone());
                Ok(None)
            }
            "chat" => {
                let text = line.trim_start().strip_prefix("chat").unwrap_or("").trim();
                if self.remote.is_some() {
                    self.send_chat(text);
                    return ok();
                }
                self.require_world()?;
                self.handle_chat_input(text);
                Ok(Some(
                    json!({ "ok": true, "chat": self.ui.chat.recent_lines(5) }),
                ))
            }
            "type" => {
                let text = line
                    .trim_start()
                    .strip_prefix("type")
                    .unwrap_or("")
                    .strip_prefix(' ')
                    .unwrap_or("");
                // A server's dialog, in configuration too.
                if self.dialog_type(text) {
                    return ok();
                }
                self.require_world()?;
                if self.book.is_some() {
                    self.book_type(text);
                    return ok();
                }
                if self.sign_edit.is_some() {
                    self.sign_input(None, Some(text));
                    return ok();
                }
                match self.screen {
                    Screen::Chat => self.chat_type(text),
                    Screen::CommandBlock => self.command_block_type(text),
                    Screen::Options => self.ui.options.type_text(&self.gui.atlas, text),
                    // The port, or the game rules' search and number boxes.
                    Screen::WorldOptions if text == "\\b" => self.world_options_text(None),
                    Screen::WorldOptions => self.world_options_text(Some(text)),
                    Screen::Inventory if self.mod_browser_typing() => {
                        self.mod_browser_text((text != "\\b").then_some(text), text == "\\b")
                    }
                    Screen::Inventory if self.recipe_search_focused() => {
                        self.recipe_text(Some(text), false)
                    }
                    // The anvil's rename box.
                    Screen::Inventory
                        if self
                            .station
                            .as_ref()
                            .is_some_and(|s| s.kind == super::stations::Kind::Anvil) =>
                    {
                        self.station_text(Some(text), false)
                    }
                    Screen::Creative if self.creative_screen.tab().search => {
                        // As typed: replaces a selected query, keeps the length limit.
                        self.creative_search_edit(Some(text))
                    }
                    _ => {
                        return Err(
                            "type needs the chat screen (key t tap), a sign or book, the language search, the creative search, the anvil or World Options".into(),
                        );
                    }
                }
                ok()
            }
            "options" => {
                use horizon_client::options_ui::Page;
                let page = match words.get(1).copied().unwrap_or("main") {
                    "main" => Page::Main,
                    "video" => Page::Video,
                    "sound" => Page::Sound,
                    "controls" => Page::Controls,
                    "keybinds" => Page::KeyBinds,
                    "controller" => Page::Controller,
                    "controller-binds" => Page::PadBinds,
                    "mouse" => Page::Mouse,
                    "chat" => Page::Chat,
                    "accessibility" => Page::Accessibility,
                    "language" => Page::Language,
                    "font" => Page::Font,
                    "packs" => Page::Packs,
                    "mods" => Page::Mods,
                    "shader-packs" => Page::ShaderPacks,
                    "graphics" => Page::Graphics,
                    "graphics-advanced" => Page::GraphicsAdvanced,
                    "extras" => Page::Extras,
                    "credits" => Page::Credits,
                    "horizon" => Page::Horizon,
                    "agent" => Page::Agent,
                    other => return Err(format!("unknown options page {other}").into()),
                };
                if self.screen == Screen::Playing && self.title.is_none() {
                    self.capture(false);
                }
                self.open_options(self.title.is_some());
                self.ui.options.show(page);
                if page == Page::ShaderPacks {
                    self.settings.shader_packs =
                        horizon_client::shader_pack_list::scan(&Self::shader_packs_dir());
                }
                ok()
            }
            "chat-log" => {
                let n = words.get(1).map_or(Ok(10), |n| n.parse())?;
                let lines = self
                    .remote_chat(n)
                    .unwrap_or_else(|| self.ui.chat.recent_lines(n));
                Ok(Some(json!({ "chat": lines })))
            }
            "stats" => {
                let summary = self.frame_stats.summary();
                if words.get(1) == Some(&"reset") {
                    self.frame_stats.reset();
                }
                Ok(Some(summary))
            }
            "bench" => {
                let seconds: f64 = arg(1)?.parse()?;
                if self.pending_bench.is_some() {
                    return Err("a benchmark is already running".into());
                }
                self.frame_stats.reset();
                self.pending_bench = Some((
                    Instant::now() + Duration::from_secs_f64(seconds),
                    reply.clone(),
                ));
                Ok(None)
            }
            "fly" => {
                self.require_world()?;
                if self.flight.is_some() || self.pending_bench.is_some() {
                    return Err("a benchmark is already running".into());
                }
                let n = |i: usize| -> Result<f64> { Ok(arg(i)?.parse()?) };
                let pitch = words.get(8).map_or(Ok(10.0), |p| p.parse::<f32>())?;
                self.frame_stats.reset();
                self.flight = Some(Flight {
                    from: Vec3::new(n(1)?, n(2)?, n(3)?),
                    to: Vec3::new(n(4)?, n(5)?, n(6)?),
                    pitch,
                    started: Instant::now(),
                    seconds: n(7)?.max(0.1),
                    raw: words.get(9) == Some(&"raw"),
                    reply: reply.clone(),
                    tick_ms: Vec::new(),
                });
                Ok(None)
            }
            "open" => {
                if self.title.is_none() {
                    return Err("a world is already open".into());
                }
                let folder = arg(1)?;
                let seed = words.get(2).map(|s| s.parse::<i64>()).transpose()?;
                let creative = words.get(3) == Some(&"creative");
                self.open_world(folder, Some(folder), seed, creative, None)?;
                ok()
            }
            "connect" => {
                if self.title.is_none() {
                    return Err("leave the current world first".into());
                }
                let name = words
                    .get(2)
                    .map_or_else(|| self.options.player_name(), |n| (*n).to_owned());
                self.connect_server(arg(1)?, &name)?;
                ok()
            }
            "credits" => {
                let poem = words.get(1) == Some(&"poem");
                let hold = words
                    .get(1 + usize::from(poem))
                    .map(|s| s.parse::<f32>())
                    .transpose()?;
                if self.screen != Screen::Win {
                    let finish = if self.screen == Screen::Options {
                        super::win::Finish::Options
                    } else {
                        super::win::Finish::Closed
                    };
                    self.open_win_screen(poem, finish);
                }
                if let Some(scroll) = hold {
                    self.hold_win_screen(scroll);
                }
                ok()
            }
            "respawn" => {
                if self.remote.is_none() || self.screen != Screen::Dead {
                    return Err("respawn needs the death screen on a server".into());
                }
                self.remote_respawn();
                ok()
            }
            "disconnect" => {
                if self.remote.is_none() {
                    return Err("not connected to a server".into());
                }
                self.leave_server(None)?;
                ok()
            }
            "save" | "save-all" => {
                self.require_world()?;
                // `save-all` is like `save-all flush`: it also rewrites chunks
                // that were not modified (the save round-trip harness compares
                // them with vanilla's files).
                let all = words[0] == "save-all";
                // Single player's world is its server thread's.
                if let Some(server) = &self.integrated {
                    server
                        .call(move |g| g.control_save(all).map_err(|e| e.to_string()))
                        .ok_or("the integrated server stopped")??;
                } else if self.remote.is_none() {
                    self.begin_control_save(all, reply.clone())?;
                    return Ok(None);
                } else {
                    self.control_save(all)?;
                }
                ok()
            }
            "dragon-hurt" => {
                // Test hook: damage the dragon's head as the player would.
                let amount: f32 = arg(1)?.parse()?;
                self.hit_special(super::dragon::SPECIAL, amount, true, false);
                ok()
            }
            "quit" => {
                self.begin_closing_save(true)?;
                ok()
            }
            other => match self.station_command(&words)? {
                Some(value) => Ok(Some(value)),
                None => Err(format!("unknown command {other}; try help").into()),
            },
        }
    }

    /// `horizon-ctl save` and `save-all`.
    fn control_save(&mut self, all: bool) -> Result<()> {
        if all {
            let keys: Vec<_> = self.world.chunks.keys().copied().collect();
            self.world.modified.extend(keys);
        }
        self.save_game()
    }

    fn require_world(&self) -> Result<()> {
        if self.title.is_some() {
            return Err("no world open; use: open WORLD [SEED] [creative]".into());
        }
        Ok(())
    }

    fn control_key_down(&mut self, key: KeyCode) -> Result<()> {
        self.physical_keys.insert(key);
        if self.screen == Screen::Chat {
            self.chat_key_code(key, None);
            return Ok(());
        }
        if self.screen == Screen::Options {
            return self.options_key(key, None);
        }
        if self.keys.insert(key) {
            self.key_pressed(key)?;
        }
        Ok(())
    }

    fn screen_name(&self) -> &'static str {
        if self.screen == Screen::Win {
            return "win";
        }
        if self.title.is_some() {
            return "title";
        }
        match self.screen {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "gamemode-switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Inventory => "inventory",
            Screen::Creative => "creative",
            Screen::Dead => "dead",
            Screen::Chat => "chat",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world-options",
            Screen::Friends => "friends",
            Screen::ServerLinks => "server-links",
            Screen::ConfirmLink => "confirm-link",
            Screen::DialogWarning => "dialog-warning",
            Screen::CommandBlock => "command-block",
            Screen::Win => "win",
            Screen::Mod => "mod",
        }
    }

    /// The hurts and melee attacks each player logged since the last call
    /// (`PlayerMotion::hurt_log`, `attack_log`), taken out.
    fn control_hurt_log(&mut self) -> Value {
        fn drain(
            name: &str,
            hurt_log: &mut Vec<horizon_client::health::HurtRecord>,
            attack_log: &mut Vec<horizon_client::health::AttackRecord>,
        ) -> Value {
            let hurts: Vec<Value> = hurt_log
                .drain(..)
                .map(|r| {
                    json!({
                        "t": r.game_time, "source": r.source, "amount": r.amount,
                        "blocked": r.blocked, "taken": r.taken, "hurt": r.hurt,
                        "armor": r.armor, "toughness": r.toughness, "health": r.health,
                        "fire_ticks": r.fire_ticks, "knockback": r.knockback,
                    })
                })
                .collect();
            let attacks: Vec<Value> = attack_log
                .drain(..)
                .map(|r| {
                    json!({
                        "t": r.game_time, "target": r.target, "id": r.id, "killed": r.killed, "amount": r.amount,
                        "scale": r.scale, "crit": r.crit, "landed": r.landed, "dealt": r.dealt,
                    })
                })
                .collect();
            json!({ "name": name, "hurts": hurts, "attacks": attacks })
        }
        let mut players = Vec::new();
        if let Some(p) = self.player.as_mut() {
            players.push(drain("host", &mut p.hurt_log, &mut p.attack_log));
        }
        if let Some(host) = self.lan.as_mut() {
            for g in &mut host.hub.guests {
                let name = g.name.clone();
                if let Some(m) = g.motion.as_mut() {
                    players.push(drain(&name, &mut m.hurt_log, &mut m.attack_log));
                } else if let Some((mut hurts, mut attacks)) = g.logs_kept.take() {
                    // Dead, not yet respawned: what its last body logged.
                    players.push(drain(&name, &mut hurts, &mut attacks));
                }
            }
        }
        json!({ "ok": true, "players": players })
    }

    fn control_status(&self) -> Value {
        let player = self.player.as_ref().map(|p| {
            json!({
                "position": [p.position.x, p.position.y, p.position.z],
                "velocity": [p.velocity.x, p.velocity.y, p.velocity.z],
                "on_ground": p.ground.on_ground,
                "health": p.health.health(),
                // The maximum the HUD's hearts last read (its cached value).
                "max_health": p
                    .attributes
                    .lookup(p.attributes.key("minecraft:max_health").as_ref())
                    .and_then(|h| p.attributes.instance(&h).ok())
                    .map(|i| i.cached_value()),
                "food": p.health.food.food,
            })
        });
        let target = self.pick().map(|hit| {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            json!({ "pos": hit.pos, "block": &*self.res.facts().get(state).name })
        });
        let renderer = self.renderer.as_ref().map(|r| {
            let [arena_bytes, section_vertex_bytes] = r.section_vertex_bytes();
            // Device memory by the backend's allocator (Vulkan; absent elsewhere).
            let memory = r.device.generate_allocator_report();
            json!({
                "gpu": r.info.name,
                "vendor": vendor_name(r.info.vendor),
                "driver": format!("{} {}", r.info.driver, r.info.driver_info).trim(),
                "device_type": match r.info.device_type {
                    wgpu::DeviceType::IntegratedGpu => "integrated",
                    wgpu::DeviceType::DiscreteGpu => "discrete",
                    wgpu::DeviceType::VirtualGpu => "virtual",
                    wgpu::DeviceType::Cpu => "cpu",
                    wgpu::DeviceType::Other => "other",
                },
                "backend": format!("{:?}", r.info.backend),
                "size": [r.config.width, r.config.height],
                "present_mode": format!("{:?}", r.config.present_mode),
                "drawn_sections": r.drawn_sections,
                "section_draws": r.section_draws,
                "section_draw_calls": r.section_draw_calls,
                "culled_sections": r.culled_sections(),
                "uploaded_bytes": r.uploaded_bytes,
                "resident_sections": r.resident_sections(),
                "arena_bytes": arena_bytes,
                "section_vertex_bytes": section_vertex_bytes,
                "gpu_allocated_bytes": memory.as_ref().map(|m| m.total_allocated_bytes),
                "gpu_reserved_bytes": memory.as_ref().map(|m| m.total_reserved_bytes),
            })
        });
        // The keyboard as the OS reports it (screens read Shift and Ctrl here).
        let mut physical_keys: Vec<_> = self
            .physical_keys
            .iter()
            .map(|k| format!("{k:?}"))
            .collect();
        physical_keys.sort();
        let lod = json!({
            "distance": self.settings.horizon.lod_distance,
            "effective_distance": self.lod_distance(),
            "terrain_distance": self.terrain_distance(),
            "source": if self.lod.is_none() { "none" } else if self.integrated.is_some() { "integrated" } else { "local" },
            "source_attempted": self.lod_source_attempted,
            "active_tiles": self.lod.as_ref().map_or(0, |lod| lod.stats().0),
            "cached_tiles": self.lod.as_ref().map_or(0, |lod| lod.stats().1),
            "far_detail": self.settings.horizon.far_detail.name(),
            "resident_quads": self.renderer.as_ref().map_or(0, |r| r.lod_quads()),
            "pending_observations": self.lod_observe.len(),
            "refined_samples": self.lod.as_ref().map_or(0, |lod| lod.refined_samples()),
        });
        // The other players' cracks this client draws (LevelRenderer.destroyingBlocks).
        let destroying: Vec<_> = self
            .destroying
            .iter()
            .map(|(id, (pos, stage, _))| json!({ "id": id, "pos": pos, "stage": stage }))
            .collect();
        let mut status = json!({
            "screen": self.screen_name(),
            "debug": {
                "overlay": self.debug,
                "hitboxes": self.show_hitboxes,
                "chunk_borders": self.show_chunk_borders,
                "pause_menu": self.pause_menu,
                "advanced_tooltips": self.settings.file.bool("advancedItemTooltips"),
                "pause_on_lost_focus": self.settings.file.bool("pauseOnLostFocus"),
                "clipboard": self.clipboard.copied(),
            },
            "dimension": self.dim.id(),
            "creative": self.creative,
            "player": player,
            "yaw": self.yaw,
            "pitch": self.pitch,
            "target": target,
            "selected": self.selected + 1,
            "held": self.held().map(|s| json!({ "item": &*s.item, "count": s.count })),
            "offhand": self.offhand.as_ref().map(|s| json!({ "item": &*s.item, "count": s.count, "damage": s.damage })),
            "keys": self.keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            // The keyboard as the OS reports it (Shift and Ctrl in screens).
            "fps": self.fps,
            "frames": self.frames,
            "ticks": self.ticks,
            "day_time": self.world.day_time,
            "music": self.music.current(&self.audio.engine),
            "sounds": self.audio.census(),
            "fishing": self.hook_status(),
            "tridents": self.tridents.len(),
            "rockets": self.rockets.len(),
            "fight": self.dragon_status(),
            // A client of a server (single player's server thread) knows the
            // carts it was sent.
            "carts": self.remote_carts().unwrap_or_else(|| self.carts.iter().map(|c| serde_json::json!({"kind": c.kind.id(), "position": [c.position.x, c.position.y, c.position.z], "velocity": [c.velocity.x, c.velocity.y, c.velocity.z]})).collect::<Vec<_>>()),
            "riding": self.riding,
            "using": self.using,
            "render_distance": self.streaming.radius,
            "chunks": self.world.chunks.len(),
            "mobs": self.mobs.len(),
            "block_entities": self.world.block_entities.len(),
            "pending_chunks": self.streaming.pending() + self.pending_uploads.len() + self.remote_backlog(),
            "generated_chunks": self.streaming.generated_total,
            "meshed_sections": self.streaming.meshed_total,
            "streaming": self.streaming.stages().into_iter().map(|(k, v)| (k.to_string(), v.into())).collect::<serde_json::Map<_, _>>(),
            "dirty_light": self.world.dirty_light.len(),
            "renderer": renderer,
            "display": self.window.as_deref().and_then(horizon_client::telemetry::monitor).map(|m| json!({
                "width": m.size().width,
                "height": m.size().height,
                "scale": m.scale_factor(),
                "refresh_hz": m.refresh_rate_millihertz().map(|r| r.div_ceil(1000)),
            })),
            "uptime_seconds": self.started.elapsed().as_secs_f64(),
            "server": self.remote_status(),
            // The names the name tags show now, seen from the eyes.
            "name_tags": self
                .eye_position()
                .map(|e| self.shown_name_tags(glam::DVec3::from_array(e))),
        });
        status["lod"] = lod;
        let (target, sampled) = self.lod.as_ref().map_or((0, 0), |lod| lod.progress());
        status["lod"]["target_tiles"] = target.into();
        status["lod"]["sampled_target_tiles"] = sampled.into();
        status["pending_uploads"] = self.pending_uploads.len().into();
        status["destroying"] = json!(destroying);
        status["physical_keys"] = json!(physical_keys);
        status["heard"] = json!(self.audio.heard);
        status["stopped_sounds"] = json!(self.audio.stopped);
        // Boats, minecarts, armor stands, item frames and paintings.
        status["objects"] = json!(self.objects_status());
        status["lightning"] = json!({"bolts": self.bolts.len(), "struck": self.lightning_seen[0],
            "flashes": self.lightning_seen[1], "flash": self.flash});
        // Primed TNT, falling blocks, projectiles, tridents and end crystals.
        status["projectiles"] = json!(self.projectiles_status());
        let mut fangs_vertices = Vec::new();
        self.fangs_geometry(glam::DVec3::ZERO, 0.5, &mut fangs_vertices);
        status["fangs_vertices"] = json!(fangs_vertices.len());
        status["maps"] = json!(self.maps_status());
        // The particles made so far, by type.
        status["particles"] = json!(self.particles.made());
        // The world border this side knows of.
        status["border"] = json!({ "center": self.border.center(), "size": self.border.size() });
        // The scoreboard sidebar the HUD shows.
        status["sidebar"] = self.sidebar_status();
        status
    }
}

/// PCI vendor IDs of the GPU makers players have (Apple's is wgpu's Metal value).
fn vendor_name(id: u32) -> String {
    match id {
        0x10de => "nvidia".into(),
        0x1002 | 0x1022 => "amd".into(),
        0x8086 => "intel".into(),
        0x106b => "apple".into(),
        0x5143 => "qualcomm".into(),
        0x13b5 => "arm".into(),
        0x10005 => "mesa".into(),
        0 => String::new(),
        other => format!("{other:#06x}"),
    }
}

fn key_code(name: &str) -> Option<KeyCode> {
    let name = name.to_ascii_lowercase();
    let letters = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    let digits = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    if let [c] = name.as_bytes() {
        return match c {
            b'a'..=b'z' => Some(letters[usize::from(c - b'a')]),
            b'0'..=b'9' => Some(digits[usize::from(c - b'0')]),
            _ => None,
        };
    }
    Some(match name.as_str() {
        "space" => KeyCode::Space,
        "shift" | "lshift" => KeyCode::ShiftLeft,
        "ctrl" | "control" | "lctrl" => KeyCode::ControlLeft,
        // The edit modifier on macOS (Cmd+A selects all there).
        "super" | "cmd" | "meta" => KeyCode::SuperLeft,
        "esc" | "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "enter" | "return" => KeyCode::Enter,
        "f1" => KeyCode::F1,
        "f2" => KeyCode::F2,
        "f3" => KeyCode::F3,
        "f4" => KeyCode::F4,
        "f5" => KeyCode::F5,
        "f6" => KeyCode::F6,
        "f11" => KeyCode::F11,
        "slash" | "/" => KeyCode::Slash,
        "backspace" => KeyCode::Backspace,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        _ => return None,
    })
}
