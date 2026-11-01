//! The debug keys in the game (26.3 `KeyboardHandler.keyPress`,
//! `handleDebugKeys` and `tick`): F3 combinations, F3+Escape, the held
//! debug-crash keys, and the debug renderers they switch on.
use super::{Game, Screen};
use horizon_client::{
    Result,
    command::args::GameMode,
    debug_keys::{CrashNotice, DebugAction, DebugHost, KeyAction, OpenScreen},
    debug_overlay,
    gizmos::Gizmos,
    hud::HudBatch,
    hud_layout, keybinds,
    text::Text,
};
use horizon_core::geometry::{Axis, Vec3};
use horizon_protocol::packets::play::{BlockEntityTagQuery, EntityTagQuery, Serverbound};
use horizon_world::nbt::Tag;
use winit::keyboard::KeyCode;

/// `ChatFormatting.YELLOW` and `RED`.
const YELLOW: u32 = 0xFF_FF55;
const RED: u32 = 0xFF_5555;

/// The game as `KeyboardHandler` sees it while it runs one key event.
struct Host<'g, 'a> {
    game: &'g mut Game<'a>,
}

impl DebugHost for Host<'_, '_> {
    fn screen(&self) -> OpenScreen {
        self.game.open_screen()
    }
    fn toggle_overlay(&mut self) {
        self.game.debug = !self.game.debug;
    }
    fn pause_game(&mut self, without_menu: bool) {
        // Gui.setPauseScreen: PauseScreen(false) only when the game really
        // pauses (single player, not open to the network). Escape alone opens
        // the menu as it always has (key_pressed).
        let g = &mut *self.game;
        if without_menu && g.screen == Screen::Playing {
            g.pause_menu = !g.has_singleplayer_server() || g.is_published();
            g.pause_focus = None;
            g.screen = Screen::Paused;
            g.capture(false);
            g.mining = None;
        }
    }
    fn debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        self.game.run_debug_action(action, shift)
    }
}

impl Game<'_> {
    fn open_screen(&self) -> OpenScreen {
        if self.title.is_some() {
            return OpenScreen::Other;
        }
        match self.screen {
            Screen::GameModeSwitcher => OpenScreen::GameModeSwitcher,
            Screen::Playing if self.book.is_none() && self.sign_edit.is_none() => OpenScreen::None,
            Screen::Paused if !self.pause_menu => OpenScreen::PauseWithoutMenu,
            _ => OpenScreen::Other,
        }
    }
    /// A physical key event (already in `physical_keys`) before the game's
    /// own handling. Returns true when it is used up (a debug combination,
    /// F3 itself, F3+Escape).
    pub(super) fn debug_key(&mut self, key: KeyCode, action: KeyAction) -> bool {
        if self.title.is_some() {
            return false;
        }
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let pressed = action != KeyAction::Release;
        let switcher_open = self.screen == Screen::GameModeSwitcher;
        if switcher_open && pressed {
            if name == self.settings.keys.get("key.debug.switchGameMode") {
                if let Some(switcher) = &mut self.gamemode_switcher {
                    switcher.cycle();
                }
            } else if key == KeyCode::Escape {
                self.close_gamemode_switcher(false);
            } else if matches!(key, KeyCode::F2 | KeyCode::F11) {
                return false;
            }
            return true;
        }
        // Vanilla types into a focused EditBox through charTyped, which no
        // debug key can swallow. Here the character comes with the key press,
        // so a screen with a focused text box takes every press first;
        // releases still reach KeyboardHandler.
        let typing = matches!(
            self.screen,
            Screen::Chat
                | Screen::Options
                | Screen::WorldOptions
                | Screen::CommandBlock
                | Screen::BlockEditor
        ) || self.sign_edit.is_some()
            || self.book.is_some()
            || (self.screen == Screen::Creative && self.creative_screen.tab().search)
            || self.recipe_search_focused()
            || self.station_typing()
            || self
                .title
                .as_mut()
                .is_some_and(|t| t.focused_field().is_some());
        // Container screens use Escape, the inventory key and the hotbar and
        // drop keys first; any other screen closes on Escape.
        let screen_key = match self.screen {
            Screen::Inventory | Screen::Creative => matches!(
                key,
                KeyCode::Escape
                    | KeyCode::KeyE
                    | KeyCode::KeyF
                    | KeyCode::KeyQ
                    | KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8
                    | KeyCode::Digit9
            ),
            Screen::Playing => false,
            _ => key == KeyCode::Escape,
        };
        if pressed && (typing || screen_key) {
            return false;
        }
        let crash_down = keybinds::key_code(self.settings.keys.get("key.debug.crash"))
            .is_some_and(|k| self.physical_keys.contains(&k));
        let shift = self.physical_keys.contains(&KeyCode::ShiftLeft)
            || self.physical_keys.contains(&KeyCode::ShiftRight);
        let now = self.debug_millis();
        let binds = self.settings.keys.clone();
        let mut keys = std::mem::take(&mut self.debug_keys);
        let modifier_was_down = keys.modifier_down;
        let to_game = keys.key_press(
            &mut Host { game: self },
            &binds,
            name,
            action,
            shift,
            crash_down,
            now,
        );
        self.debug_keys = keys;
        if switcher_open {
            if name == binds.get("key.debug.modifier") {
                self.close_gamemode_switcher(true);
                return true;
            }
            return false;
        }
        if !pressed {
            return false;
        }
        // The debug modifier does nothing else.
        if name == binds.get("key.debug.modifier") || name == binds.get("key.debug.overlay") {
            return true;
        }
        if self.open_screen() == OpenScreen::None {
            !to_game
        } else {
            modifier_was_down
        }
    }
    /// `Util.getMillis` (positive: KeyboardHandler treats 0 as unset).
    pub(super) fn debug_millis(&self) -> i64 {
        self.started.elapsed().as_millis() as i64 + 1_000_000
    }
    /// `KeyboardHandler.tick`: the debug crash countdown.
    pub(super) fn tick_debug_keys(&mut self) -> Result<()> {
        let now = self.debug_millis();
        match self.debug_keys.tick(now) {
            None => {}
            Some(CrashNotice::Message) => {
                let args = [
                    self.key_text("key.debug.modifier"),
                    self.key_text("key.debug.crash"),
                ];
                self.debug_feedback(Text::translate("debug.crash.message", &args));
            }
            Some(CrashNotice::Warning(seconds)) => self.debug_warning(Text::translate(
                "debug.crash.warning",
                &[Text::literal(seconds.to_string())],
            )),
            Some(CrashNotice::Crash) => {
                // Minecraft.emergencySaveAndCrash: the world is saved, then the
                // game stops; main writes the crash report.
                let crash = horizon_client::crash::DebugCrash::new();
                if let Err(e) = self.save_game() {
                    log::error!("the emergency save before the crash failed: {e}");
                }
                return Err(crash.into());
            }
        }
        Ok(())
    }
    /// The network charts' samples each tick: `PingDebugMonitor.tick` while
    /// they show, `BandwidthDebugMonitor.tick` always.
    pub(super) fn tick_debug_charts(&mut self) {
        let shown = self.debug && self.debug_charts.network;
        let Some(r) = &self.remote else {
            // The integrated server answers within the tick.
            if shown {
                self.debug_charts.ping.log(0);
            }
            return;
        };
        let received = r.session.counters.lock().unwrap().as_ref().map(|c| {
            let order = std::sync::atomic::Ordering::Relaxed;
            [
                c.bytes_in.load(order),
                c.sent.load(order),
                c.received.load(order),
            ]
        });
        if let Some([bytes, sent, received]) = received {
            self.debug_charts.log_bandwidth(bytes);
            self.debug_charts.log_packets([sent, received]);
        }
        if shown {
            use horizon_protocol::packets::status::Ping;
            let time = self.debug_millis();
            self.send(Serverbound::PingRequest(Ping { time }));
        }
    }
    /// The overlay's chart lines (`DebugScreenOverlay`'s "Debug charts").
    pub(super) fn debug_chart_lines(&self) -> [String; 3] {
        let keys = &self.settings.keys;
        let modifier = keys.get("key.debug.modifier");
        let key = |mapping: &str| {
            let prefix = if modifier == "key.keyboard.unknown" {
                String::new()
            } else {
                format!("{}+", keybinds::display_name(modifier).plain())
            };
            format!(
                "[{prefix}{}]",
                keybinds::display_name(keys.get(mapping)).plain()
            )
        };
        let chart = |mapping: &str, name: &str, on: bool| {
            format!(
                "{} {name} {}",
                key(mapping),
                if on { "visible" } else { "hidden" }
            )
        };
        // `DebugScreenOverlay`: the single-player variants for a local server.
        let remote = !self.has_singleplayer_server();
        let c = &self.debug_charts;
        [
            format!(
                "Debug charts: {}; {};",
                chart("key.debug.profilingChart", "Profiler", false),
                chart(
                    "key.debug.fpsCharts",
                    if remote { "fps" } else { "fps + tps" },
                    c.fps
                ),
            ),
            format!(
                "{}; {}",
                chart(
                    "key.debug.networkCharts",
                    if remote { "Bandwidth + Ping" } else { "Ping" },
                    c.network
                ),
                chart("key.debug.lightmapTexture", "Lightmap", c.lightmap),
            ),
            format!("To edit: press {}", key("key.debug.debugOptions")),
        ]
    }
    /// `Gui.extractRenderState`'s debug overlay, over everything the frame
    /// drew (menus too); the debug options screen draws it under itself.
    pub(super) fn draw_debug_overlay(&self, batch: &mut HudBatch, size: [u32; 2]) {
        if self.screen != Screen::DebugOptions {
            self.draw_debug_overlay_under(batch, size);
        }
    }
    /// `DebugScreenOverlay.extractRenderState`: the enabled entries' lines
    /// (`debug-profile.json`), at the debug GUI scale, and the charts while
    /// the overlay is open. F1 hides it unless a screen is open.
    pub(super) fn draw_debug_overlay_under(&self, batch: &mut HudBatch, size: [u32; 2]) {
        let screen_open = self.screen != Screen::Playing;
        let reduced = self.reduced_debug_info();
        if self.title.is_some()
            || !self.debug_entries.any_enabled(self.debug, reduced)
            || (self.ui.hide_gui && !screen_open)
        {
            return;
        }
        let framebuffer = size.map(|v| v as i32);
        let (scale, [width, height]) = debug_overlay::scale(
            framebuffer,
            batch.scale as i32,
            self.settings.file.int("debugGuiScale"),
            self.settings.force_unicode,
            [batch.width as i32, batch.height as i32],
        );
        let mut d = debug_overlay::Displayer::default();
        self.debug_entry_lines(&mut d, framebuffer);
        // The chart keys and the charts only while the overlay is open.
        let footer = if self.debug {
            self.debug_chart_lines().to_vec()
        } else {
            Vec::new()
        };
        let (left, right) = d.columns(&footer);
        let atlas = &self.gui.atlas;
        let mut draws = debug_overlay::draw(&atlas.font, [&left, &right], scale, width);
        if self.debug {
            let charts = self.debug_charts.draw(
                &atlas.font,
                width,
                height,
                self.settings.max_fps,
                1000.0 / self.clock.rate(),
                !self.has_singleplayer_server(),
            );
            draws.push(hud_layout::GuiDraw::Push(vec![hud_layout::PoseOp::Scale(
                scale, scale,
            )]));
            draws.extend(charts);
            if self.debug_charts.lightmap
                && let Some(r) = &self.renderer
            {
                draws.extend(horizon_client::debug_chart::lightmap(
                    &r.lightmap_pixels,
                    width,
                    height,
                ));
            }
            draws.push(hud_layout::GuiDraw::Pop);
        }
        hud_layout::paint(batch, atlas, &draws);
    }
    /// `DebugEntryFps`.
    pub(super) fn debug_fps_line(&self, d: &mut debug_overlay::Displayer) {
        let limit = self.framerate_limit();
        let present = self
            .renderer
            .as_ref()
            .map_or("", |r| match r.config.present_mode {
                wgpu::PresentMode::Immediate => " (immediate)",
                wgpu::PresentMode::Mailbox => " (mailbox)",
                wgpu::PresentMode::Fifo => " (fifo)",
                wgpu::PresentMode::FifoRelaxed => " (fifo relaxed)",
                _ => "",
            });
        let refresh = self
            .window
            .as_ref()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or("0".into(), |mhz| {
                debug_overlay::refresh_rate_label(f64::from(mhz) / 1000.0)
            });
        d.priority(format!(
            "{} fps T: {}{present} @{refresh}Hz",
            self.fps.round() as i32,
            if limit == 260 {
                "inf".into()
            } else {
                limit.to_string()
            },
        ));
    }
    /// `DebugEntrySimplePerformanceImpactors`.
    pub(super) fn debug_performance_lines(&self, d: &mut debug_overlay::Displayer) {
        let s = &self.settings;
        d.line(format!(
            "{}{}B: {}",
            if s.improved_transparency {
                "improved-transparency "
            } else {
                ""
            },
            match s.clouds.as_str() {
                "false" => "",
                "fast" => "fast-clouds ",
                _ => "fancy-clouds ",
            },
            s.biome_blend
        ));
        d.line("Filtering: None");
        d.line(format!(
            "Terrain Rendering: {}",
            if self.renderer.as_ref().is_some_and(|r| r.multi_draw) {
                "multidrawindirect"
            } else {
                "naive"
            }
        ));
    }
    /// `DebugEntrySystemSpecs` (no Java line: Horizon has no JVM).
    pub(super) fn debug_system_lines(
        &self,
        d: &mut debug_overlay::Displayer,
        framebuffer: [i32; 2],
    ) {
        if let Some(r) = self.renderer.as_ref() {
            static CPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            d.group(
                "system",
                [format!(
                    "CPU: {}",
                    CPU.get_or_init(horizon_client::crash::cpu)
                )],
            );
            let info = &r.info;
            d.group(
                "system",
                [format!(
                    "Display: {}x{} ({})",
                    framebuffer[0],
                    framebuffer[1],
                    // Metal adapters report no PCI vendor.
                    if info.vendor == 0 && info.backend == wgpu::Backend::Metal {
                        "Apple".into()
                    } else {
                        debug_overlay::vendor_name(info.vendor)
                    }
                )],
            );
            if let Some(w) = self.window.as_ref() {
                let density = w.scale_factor();
                let logical = w.inner_size().to_logical::<f64>(density);
                d.group(
                    "system",
                    [format!(
                        "Window: {}x{} ({density:.2}x pixel density)",
                        logical.width.round() as i32,
                        logical.height.round() as i32
                    )],
                );
            }
            let kind = match info.device_type {
                wgpu::DeviceType::IntegratedGpu => " (iGPU)",
                wgpu::DeviceType::DiscreteGpu => " (dGPU)",
                wgpu::DeviceType::VirtualGpu => " (vGPU)",
                wgpu::DeviceType::Cpu => " (software)",
                wgpu::DeviceType::Other => "",
            };
            let driver = format!("{} {}", info.driver, info.driver_info);
            d.group(
                "system",
                [
                    format!("{}{kind}", info.name),
                    format!(
                        "{:?} {}",
                        info.backend,
                        driver.trim().lines().next().unwrap_or("")
                    )
                    .trim_end()
                    .to_owned(),
                ],
            );
        }
        // Horizon's own: HDR's output while the setting is on, under the GPU (a group of its
        // own would move others to the other column).
        if self.settings.horizon.hdr
            && let Some(r) = self.renderer.as_ref()
        {
            d.group("system", [r.hdr_line()]);
        }
    }
    /// `DebugEntryTps` for local and remote connections.
    pub(super) fn debug_tps_line(&self, d: &mut debug_overlay::Displayer) {
        if self.player.is_some() {
            let [tx, rx] = self.debug_charts.packet_rates.map(|n| n.round() as i64);
            let (target, status) = if self.clock.sprinting() {
                ("-".to_owned(), " (sprinting)")
            } else if self.clock.stepping() {
                (
                    format!("{:.1}", 1000.0 / self.clock.rate()),
                    " (frozen - stepping)",
                )
            } else if self.clock.frozen() {
                (format!("{:.1}", 1000.0 / self.clock.rate()), " (frozen)")
            } else {
                (format!("{:.1}", 1000.0 / self.clock.rate()), "")
            };
            if self.remote.is_none() || self.integrated.is_some() {
                let tick_ms = self
                    .integrated
                    .as_ref()
                    .map_or(self.debug_charts.smoothed_tick_ms, |server| {
                        server.smoothed_tick_ms()
                    });
                d.line(format!(
                    "Integrated server @ {tick_ms:.1}/{target} ms{status}, {tx} tx, {rx} rx",
                ));
            } else if let Some(remote) = &self.remote {
                d.line(format!(
                    "\"{}\" server{status}, {tx} tx, {rx} rx",
                    remote.brand
                ));
            }
        }
    }
    /// `KeyMapping.getTranslatedKeyMessage`.
    fn key_text(&self, mapping: &str) -> Text {
        keybinds::display_name(self.settings.keys.get(mapping))
    }
    /// `KeyboardHandler.decorateDebugComponent`: `[Debug]:` in bold, a space,
    /// the message, shown with `Minecraft.showDebugChat`.
    fn debug_decorated(&mut self, color: u32, message: Text) {
        let prefix = Text::translate("debug.prefix", &[])
            .with_color(color)
            .bold();
        self.show_debug_chat(
            Text::empty()
                .append(prefix)
                .append(Text::literal(" "))
                .append(message),
        );
    }
    /// `Minecraft.showDebugChat`: a client system message the narrator says.
    fn show_debug_chat(&mut self, message: Text) {
        self.say_system_queued(&message);
        self.system_message(message);
    }
    fn debug_feedback(&mut self, message: Text) {
        self.debug_decorated(YELLOW, message);
    }
    fn debug_warning(&mut self, message: Text) {
        self.debug_decorated(RED, message);
    }
    fn debug_translated(&mut self, key: &str) {
        self.debug_feedback(Text::translate(key, &[]));
    }
    /// `LocalPlayer.isReducedDebugInfo`: the server's rule or the option.
    pub(super) fn reduced_debug_info(&self) -> bool {
        self.settings.reduced_debug_info || self.ui.rules.bool("reduced_debug_info")
    }
    /// `DebugScreenEntryList.isCurrentlyEnabled` for an entry's path.
    pub(super) fn debug_entry(&self, path: &str) -> bool {
        self.debug_entries
            .enabled(path, self.debug, self.reduced_debug_info())
    }
    /// The `3d_crosshair` entry.
    pub(super) fn debug_crosshair(&self) -> bool {
        self.debug_entry("3d_crosshair")
    }
    /// `Permissions.COMMANDS_GAMEMASTER` (`GameModeCommand.PERMISSION_CHECK`).
    pub(super) fn gamemaster(&self) -> bool {
        self.ui.registries.permission >= 2
    }
    fn toggle_option(&mut self, key: &str) -> bool {
        let on = !self.settings.file.bool(key);
        self.settings
            .file
            .set(key, horizon_client::options_file::V::Bool(on));
        self.settings.save(&Self::options_path());
        on
    }
    /// One matching mapping of `handleDebugKeys`; whether it counts.
    fn run_debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        let player = self.player.is_some() && !self.reduced_debug_info();
        match action {
            DebugAction::ReloadChunks => {
                self.all_sections_changed();
                self.debug_translated("debug.reload_chunks.message");
                true
            }
            DebugAction::ShowHitboxes if player => {
                let on = self
                    .debug_entries
                    .toggle_status("entity_hitboxes", self.debug);
                self.debug_translated(if on {
                    "debug.show_hitboxes.on"
                } else {
                    "debug.show_hitboxes.off"
                });
                true
            }
            DebugAction::ClearChat => {
                // ChatComponent.clearMessages(false): sent history stays.
                self.ui.chat.clear();
                true
            }
            DebugAction::ShowChunkBorders if player => {
                let on = self
                    .debug_entries
                    .toggle_status("chunk_borders", self.debug);
                self.debug_translated(if on {
                    "debug.chunk_boundaries.on"
                } else {
                    "debug.chunk_boundaries.off"
                });
                true
            }
            DebugAction::AdvancedTooltips => {
                // The message comes before Options.save here.
                let on = !self.settings.file.bool("advancedItemTooltips");
                self.debug_translated(if on {
                    "debug.advanced_tooltips.on"
                } else {
                    "debug.advanced_tooltips.off"
                });
                self.toggle_option("advancedItemTooltips");
                true
            }
            DebugAction::CopyRecreateCommand => {
                if player {
                    self.copy_recreate_command(self.gamemaster(), !shift);
                }
                true
            }
            DebugAction::Spectate => {
                if self.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.creative_spectator.error");
                } else if self.mode != GameMode::Spectator {
                    self.change_game_mode(GameMode::Spectator);
                } else {
                    let back = self.previous_mode.unwrap_or(GameMode::Creative);
                    self.change_game_mode(back);
                }
                true
            }
            DebugAction::SwitchGameMode if self.open_screen() == OpenScreen::None => {
                if self.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.gamemodes.error");
                } else {
                    self.open_gamemode_switcher();
                }
                true
            }
            DebugAction::FocusPause => {
                let on = self.toggle_option("pauseOnLostFocus");
                self.debug_translated(if on {
                    "debug.pause_focus.on"
                } else {
                    "debug.pause_focus.off"
                });
                true
            }
            DebugAction::ReloadResourcePacks => {
                self.debug_translated("debug.reload_resourcepacks.message");
                self.reload_packs();
                true
            }
            DebugAction::CopyLocation if player => {
                self.debug_translated("debug.copy_location.message");
                if let Some(p) = &self.player {
                    use horizon_client::java_text::format_fixed2 as f2;
                    let command = format!(
                        "/execute in {} run tp @s {} {} {} {} {}",
                        self.dim.id(),
                        f2(p.position.x),
                        f2(p.position.y),
                        f2(p.position.z),
                        f2(f64::from(self.yaw)),
                        f2(f64::from(self.pitch)),
                    );
                    self.set_clipboard(&command);
                }
                true
            }
            DebugAction::DumpVersion => {
                self.debug_translated("debug.version.header");
                for line in self.version_lines() {
                    self.show_debug_chat(line);
                }
                true
            }
            DebugAction::ImprovedTransparency => {
                self.settings.improved_transparency = !self.settings.improved_transparency;
                self.settings.save(&Self::options_path());
                self.apply_settings();
                self.debug_translated(if self.settings.improved_transparency {
                    "debug.improvedTransparency.on"
                } else {
                    "debug.improvedTransparency.off"
                });
                true
            }
            DebugAction::FpsCharts => {
                self.debug_charts.toggle_fps(&mut self.debug);
                true
            }
            DebugAction::NetworkCharts => {
                self.debug_charts.toggle_network(&mut self.debug);
                true
            }
            DebugAction::LightmapTexture => {
                self.debug_charts.toggle_lightmap(&mut self.debug);
                true
            }
            DebugAction::DebugOptions => {
                self.toggle_debug_options();
                true
            }
            // Screens and overlays Horizon does not draw yet (the profiler and
            // its chart, the texture dump): the keys are used up as in vanilla.
            DebugAction::DumpDynamicTextures
            | DebugAction::Profiling
            | DebugAction::ProfilingChart => true,
            DebugAction::ShowHitboxes
            | DebugAction::ShowChunkBorders
            | DebugAction::SwitchGameMode
            | DebugAction::CopyLocation => false,
        }
    }
    /// `ServerboundChangeGameModePacket` handled by the integrated server:
    /// `GameModeCommand.setGameMode` with its feedback.
    pub(super) fn change_game_mode(&mut self, mode: GameMode) {
        if self.remote.is_some() {
            use horizon_protocol::packets::play::ChangeGameMode;
            let mode = match mode {
                GameMode::Survival => 0,
                GameMode::Creative => 1,
                GameMode::Adventure => 2,
                GameMode::Spectator => 3,
            };
            self.send(Serverbound::ChangeGameMode(ChangeGameMode { mode }));
            return;
        }
        if self.set_game_mode(mode) {
            let name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            self.feedback(Text::translate("commands.gamemode.success.self", &[name]));
        }
    }
    /// `KeyboardHandler.copyRecreateCommand`: the targeted block or mob as a
    /// `/setblock` or `/summon` command. The server's data (no Shift) comes
    /// from the server the game is connected to, the integrated one included
    /// (`DebugQueryHandler`); without one, the game's own data is the server's.
    fn copy_recreate_command(&mut self, add_nbt: bool, from_server: bool) {
        let query = add_nbt && from_server && self.remote.is_some();
        let side = if add_nbt && from_server {
            "server"
        } else {
            "client"
        };
        if self.entity_targeted()
            && let Some((index, _)) = self.mob_hit()
        {
            let (kind, position, id) = {
                let m = &self.mobs[index];
                (m.kind.id.clone(), m.position, m.id)
            };
            if query {
                // DebugQueryHandler.queryEntityTag: the answer copies it.
                let reply = QueryReply::Entity { id, kind, position };
                if let Some(transaction) = self.remote.as_mut().map(|r| r.debug_query.start(reply))
                {
                    self.send(Serverbound::EntityTagQuery(EntityTagQuery {
                        transaction,
                        entity: id as i32,
                    }));
                }
                return;
            }
            // Entity.saveWithoutId.
            let tag = if add_nbt {
                self.entity_nbt(super::commands::Target::Mob(id))
                    .map(|mut tag| {
                        if let Tag::Compound(c) = &mut tag {
                            c.remove(&"id".into());
                        }
                        tag
                    })
            } else {
                None
            };
            self.copy_create_entity_command(&kind, position, tag);
            self.debug_translated(&format!("debug.inspect.{side}.entity"));
            return;
        }
        let Some(hit) = self.pick() else {
            return;
        };
        let pos = hit.pos;
        let state = self.state_string(self.world.block(pos[0], pos[1], pos[2]));
        if query {
            // DebugQueryHandler.queryBlockEntityTag: the answer copies it.
            let reply = QueryReply::Block { pos, state };
            if let Some(transaction) = self.remote.as_mut().map(|r| r.debug_query.start(reply)) {
                self.send(Serverbound::BlockEntityTagQuery(BlockEntityTagQuery {
                    transaction,
                    pos,
                }));
            }
            return;
        }
        let tag = if add_nbt {
            self.block_entity_tag(pos)
        } else {
            None
        };
        self.copy_create_block_command(pos, &state, tag.as_ref());
        self.debug_translated(&format!("debug.inspect.{side}.block"));
    }
    /// `BlockEntity.saveWithoutMetadata` of the block entity at `pos`, if
    /// there is one: its data without the id or the position.
    pub(super) fn block_entity_tag(&self, pos: [i32; 3]) -> Option<Tag> {
        let e = self.world.block_entities.get(&(pos[0], pos[1], pos[2]))?;
        let name = self.data_block_name(pos);
        let mut tag = horizon_client::save::encode_block_entity((pos[0], pos[1], pos[2]), e, &name);
        if let Tag::Compound(c) = &mut tag {
            for key in ["id", "x", "y", "z", "keepPacked"] {
                c.remove(&key.into());
            }
        }
        Some(tag)
    }
    /// `ClientPacketListener.handleTagQueryPacket`: the server's answer to
    /// the last debug query (`DebugQueryHandler.handleResponse`) completes
    /// it; an answer to an older query is ignored.
    pub(super) fn answer_tag_query(&mut self, transaction: i32, tag: Tag) {
        let Some(reply) = self
            .remote
            .as_mut()
            .and_then(|r| r.debug_query.answer(transaction))
        else {
            return;
        };
        // FriendlyByteBuf.readNbt: an End tag is no compound.
        let tag = match tag {
            Tag::End => None,
            tag => Some(tag),
        };
        match reply {
            QueryReply::Block { pos, state } => {
                self.copy_create_block_command(pos, &state, tag.as_ref());
                self.debug_translated("debug.inspect.server.block");
            }
            QueryReply::Entity { id, kind, position } => {
                // Entity.position() when the answer comes: where it is now,
                // else where it was last.
                let position = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map_or(position, |m| m.position);
                self.copy_create_entity_command(&kind, position, tag);
                self.debug_translated("debug.inspect.server.entity");
            }
        }
    }
    /// `KeyboardHandler.copyCreateBlockCommand`.
    fn copy_create_block_command(&mut self, pos: [i32; 3], state: &str, tag: Option<&Tag>) {
        let mut description = state.to_owned();
        if let Some(tag) = tag {
            description.push_str(&horizon_client::snbt::to_snbt(tag));
        }
        self.set_clipboard(&format!(
            "/setblock {} {} {} {description}",
            pos[0], pos[1], pos[2]
        ));
    }
    /// `KeyboardHandler.copyCreateEntityCommand`: the data without its UUID
    /// or position.
    fn copy_create_entity_command(&mut self, kind: &str, pos: Vec3, tag: Option<Tag>) {
        use horizon_client::java_text::format_fixed2 as f2;
        let mut command = format!("/summon {kind} {} {} {}", f2(pos.x), f2(pos.y), f2(pos.z));
        if let Some(mut tag) = tag {
            if let Tag::Compound(c) = &mut tag {
                for key in ["UUID", "Pos"] {
                    c.remove(&key.into());
                }
            }
            command.push(' ');
            command.push_str(&horizon_client::snbt::pretty(&tag).plain());
        }
        self.set_clipboard(&command);
    }
    /// `KeyboardHandler.setClipboard`.
    fn set_clipboard(&mut self, text: &str) {
        use horizon_client::clipboard::Clipboard;
        self.clipboard.set(text);
    }
    /// The debug renderers' gizmos for a frame (`DebugRenderer.emitGizmos`).
    pub(super) fn debug_gizmos(&self, alpha: f32, camera: [f64; 3]) -> Gizmos {
        let mut g = Gizmos::default();
        self.bounding_box_gizmos(camera, &mut g);
        self.debug_renderer_gizmos(&mut g);
        if self.debug_entry("chunk_borders")
            && let Some(p) = &self.player
        {
            let block = [
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            ];
            let min_y = self.world.bottom_y();
            g.chunk_borders(block, min_y, min_y + self.world.layout().height() - 1);
        }
        if !self.debug_entry("entity_hitboxes") {
            return g;
        }
        let lerp = |a: f64, b: f64| a + f64::from(alpha) * (b - a);
        let lerpf = |a: f32, b: f32| a + alpha * (b - a);
        let guests = self.lan.iter().flat_map(|h| h.models.values());
        for m in self.mobs.iter().chain(guests) {
            if m.removed || m.invisible || m.has_effect("minecraft:invisibility") {
                continue;
            }
            let pos = [
                lerp(m.previous.x, m.position.x),
                lerp(m.previous.y, m.position.y),
                lerp(m.previous.z, m.position.z),
            ];
            let view = (
                lerpf(m.previous_head_pitch, m.head_pitch),
                lerpf(m.previous_head_yaw, m.head_yaw),
            );
            g.hitbox(
                pos,
                (m.width(), m.height()),
                m.eye_height() as f32,
                true,
                view,
            );
        }
        for item in &self.dropped {
            let pos = [
                lerp(item.previous.x, item.position.x),
                lerp(item.previous.y, item.position.y),
                lerp(item.previous.z, item.position.z),
            ];
            // EntityType.ITEM: 0.25 by 0.25, eyes at 0.2125.
            g.hitbox(pos, (0.25, 0.25), 0.2125, false, (0.0, 0.0));
        }
        // The other entities (EntityTypes sizes; eyes at 0.85 of the height
        // unless the type sets one). `at` is the interpolated position.
        let at = |previous: Vec3, now: Vec3| {
            [
                lerp(previous.x, now.x),
                lerp(previous.y, now.y),
                lerp(previous.z, now.z),
            ]
        };
        for p in &self.paintings {
            let (min, max) = p.bounds();
            let centre = std::array::from_fn(|i| (min[i] + max[i]) / 2.0);
            g.entity_box(min, max, centre, 0.425, false, step_view(p.step()));
        }
        for f in self.item_frames.iter().filter(|f| !f.invisible) {
            let (min, max) = f.bounds();
            let centre = std::array::from_fn(|i| (min[i] + max[i]) / 2.0);
            g.entity_box(min, max, centre, 0.0, false, step_view(f.step()));
        }
        for c in &self.cushions {
            let (min, max) = c.bounds();
            let p = c.position;
            g.entity_box(min, max, [p.x, p.y, p.z], 0.2125, false, (0.0, c.yaw));
        }
        // Interaction: EntityDimensions.scalable(width, height), so its eyes
        // are at 0.85 of the height. Its rotation is not kept here.
        for i in self.interactions.values() {
            let b = i.bounds();
            let p = i.position;
            g.entity_box(
                [b.min(Axis::X), b.min(Axis::Y), b.min(Axis::Z)],
                [b.max(Axis::X), b.max(Axis::Y), b.max(Axis::Z)],
                [p.x, p.y, p.z],
                i.height.max(0.0) * 0.85,
                false,
                (0.0, 0.0),
            );
        }
        for a in self.armor_stands.iter().filter(|a| !a.invisible) {
            // ArmorStand.getDimensions: a marker has none, a small one is
            // half scale with its eyes at 0.9875.
            let (size, eye) = match (a.marker, a.small) {
                (true, _) => ((0.0, 0.0), 0.0),
                (_, true) => ((0.25, 0.9875), 0.9875),
                _ => ((0.5, 1.975), 1.7775),
            };
            g.hitbox(
                at(a.previous, a.position),
                size,
                eye,
                true,
                (0.0, a.render_yaw(alpha)),
            );
        }
        for b in &self.boats {
            let h = super::boats::HEIGHT as f32;
            g.hitbox(
                at(b.previous, b.position),
                (super::boats::WIDTH as f32, h),
                h,
                false,
                (0.0, b.yaw),
            );
        }
        for c in &self.carts {
            let (w, h) = (
                super::minecarts::WIDTH as f32,
                super::minecarts::HEIGHT as f32,
            );
            g.hitbox(
                at(c.previous, c.position),
                (w, h),
                h * 0.85,
                false,
                (0.0, c.yaw),
            );
        }
        for o in &self.orbs {
            g.hitbox(
                at(o.previous, o.position),
                (0.5, 0.5),
                0.425,
                false,
                (0.0, 0.0),
            );
        }
        for f in &self.falling {
            g.hitbox(
                at(f.previous, f.position),
                (0.98, 0.98),
                0.833,
                false,
                (0.0, 0.0),
            );
        }
        for t in &self.tnts {
            g.hitbox(
                at(t.previous, t.position),
                (0.98, 0.98),
                0.15,
                false,
                (0.0, 0.0),
            );
        }
        for c in &self.crystals {
            let p = c.position;
            g.hitbox([p.x, p.y, p.z], (2.0, 2.0), 1.7, false, (0.0, 0.0));
        }
        for p in &self.creatures.projectiles {
            use horizon_client::projectiles::ProjectileKind as K;
            let (size, eye) = match p.kind {
                K::Arrow { .. } | K::Trident => (0.5, 0.13),
                K::Snowball
                | K::Egg
                | K::EnderPearl
                | K::LlamaSpit
                | K::Potion { .. }
                | K::Thrown(_) => (0.25, 0.2125),
                K::SmallFireball | K::ShulkerBullet { .. } | K::WitherSkull { .. } => {
                    (0.3125, 0.265_625)
                }
                K::WindCharge | K::BreezeWindCharge => (0.3125, 0.0),
                K::LargeFireball { .. } => (1.0, 0.85),
            };
            g.hitbox(
                at(p.previous, p.position),
                (size, size),
                eye,
                false,
                (p.pitch, p.yaw),
            );
        }
        for t in &self.tridents {
            g.hitbox(
                at(t.previous, t.position),
                (0.5, 0.5),
                0.13,
                false,
                (t.pitch, t.yaw),
            );
        }
        // EntityType.EYE_OF_ENDER: 0.25 by 0.25, never turned.
        for (previous, now) in self.end.eye_motions() {
            let v = |p: glam::DVec3| Vec3::new(p.x, p.y, p.z);
            g.hitbox(
                at(v(previous), v(now)),
                (0.25, 0.25),
                0.2125,
                false,
                (0.0, 0.0),
            );
        }
        for r in &self.rockets {
            // Projectile.updateRotation: the look follows the motion.
            let v = r.velocity;
            let yaw = v.x.atan2(v.z).to_degrees() as f32;
            let pitch = v.y.atan2(v.x.hypot(v.z)).to_degrees() as f32;
            g.hitbox(
                at(r.previous, r.position),
                (0.25, 0.25),
                0.2125,
                false,
                (pitch, yaw),
            );
        }
        for h in self
            .hook
            .iter()
            .map(|h| (h.previous, h.position))
            .chain(self.remote_hooks.iter().map(|h| (h.previous, h.position)))
        {
            g.hitbox(at(h.0, h.1), (0.25, 0.25), 0.2125, false, (0.0, 0.0));
        }
        for b in &self.bolts {
            g.hitbox(b.at, (0.0, 0.0), 0.0, false, (0.0, 0.0));
        }
        // The dragon: 16 by 8, a living entity; its parts follow.
        if let Some(d) = &self.dragon {
            g.hitbox(
                at(d.previous, d.position),
                (16.0, 8.0),
                6.8,
                true,
                (0.0, d.yaw),
            );
        }
        // The dragon's parts.
        for (id, b) in self.special_targets() {
            if (super::dragon::SPECIAL..super::dragon::SPECIAL + 8).contains(&id) {
                g.cuboid(
                    [b.min(Axis::X), b.min(Axis::Y), b.min(Axis::Z)],
                    [b.max(Axis::X), b.max(Axis::Y), b.max(Axis::Z)],
                    horizon_client::gizmos::DRAGON_PART,
                    horizon_client::gizmos::STROKE_WIDTH,
                );
            }
        }
        // The player's own, unless the camera is in its head.
        if !self.camera_type.first_person()
            && let Some(p) = &self.player
        {
            let d = p.dimensions;
            let pos = [
                lerp(self.previous_position.x, p.position.x),
                lerp(self.previous_position.y, p.position.y),
                lerp(self.previous_position.z, p.position.z),
            ];
            g.hitbox(
                pos,
                (d.width, d.height),
                d.eye_height,
                true,
                (self.pitch, self.yaw),
            );
        }
        g
    }
}

/// The (x, y) rotations whose view vector is the step of a direction.
fn step_view(step: [i32; 3]) -> (f32, f32) {
    let [x, y, z] = step.map(f64::from);
    (
        (-y).asin().to_degrees() as f32,
        (-x.atan2(z)).to_degrees() as f32,
    )
}

/// `DebugQueryHandler`: the last debug NBT query (F3+I) sent to the server
/// and what its answer completes. Only the newest query is answered.
#[derive(Debug)]
pub(super) struct DebugQuery {
    transaction: i32,
    pending: Option<QueryReply>,
}
impl Default for DebugQuery {
    fn default() -> Self {
        Self {
            transaction: -1,
            pending: None,
        }
    }
}
/// What a debug query's answer is copied into: the block command
/// (`copyCreateBlockCommand`) or the entity's (`copyCreateEntityCommand`).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum QueryReply {
    Block {
        pos: [i32; 3],
        state: String,
    },
    Entity {
        id: u32,
        kind: String,
        position: Vec3,
    },
}
impl DebugQuery {
    /// `DebugQueryHandler.startTransaction`: the next transaction id, which
    /// replaces any query still waiting.
    fn start(&mut self, reply: QueryReply) -> i32 {
        self.pending = Some(reply);
        self.transaction = self.transaction.wrapping_add(1);
        self.transaction
    }
    /// `DebugQueryHandler.handleResponse`: what the answer to `transaction`
    /// completes, once, when it answers the newest query.
    fn answer(&mut self, transaction: i32) -> Option<QueryReply> {
        if transaction == self.transaction {
            self.pending.take()
        } else {
            None
        }
    }
}

/// `DebugEntryChunkGeneration`'s `lastPos` and `result`: F3's `Density` line is sampled again
/// only when the feet block or the dimension changes. Vanilla adds the group only inside its
/// server-level branch, so with no generator (a remote server, a LAN world joined, the server
/// thread not there yet) nothing is drawn and nothing of the last world is kept.
#[derive(Default)]
pub(super) struct DensityCache(Option<(&'static str, [i32; 3], Option<String>)>);
impl DensityCache {
    /// The line to draw; `sample` is `None` without a generator.
    pub(super) fn line(
        &mut self,
        dim: &'static str,
        feet: [i32; 3],
        sample: Option<impl FnOnce() -> Option<String>>,
    ) -> Option<&str> {
        let Some(sample) = sample else {
            self.0 = None;
            return None;
        };
        if !self
            .0
            .as_ref()
            .is_some_and(|(d, f, _)| *d == dim && *f == feet)
        {
            self.0 = Some((dim, feet, sample()));
        }
        self.0.as_ref().and_then(|(_, _, line)| line.as_deref())
    }
    /// Another world: its generator, not the last one's line.
    pub(super) fn clear(&mut self) {
        self.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(x: i32) -> QueryReply {
        QueryReply::Block {
            pos: [x, 64, 0],
            state: "minecraft:chest[facing=north,type=single,waterlogged=false]".into(),
        }
    }

    #[test]
    fn a_debug_query_is_answered_once_and_only_the_newest() {
        let mut q = DebugQuery::default();
        // The first transaction is 0 (the handler starts at -1).
        assert_eq!(q.start(block(1)), 0);
        assert_eq!(q.answer(0), Some(block(1)));
        assert_eq!(q.answer(0), None);
        // A second press before the first answer: the older answer is ignored
        // and the newer one copies the newer target.
        assert_eq!(q.start(block(2)), 1);
        assert_eq!(q.start(block(3)), 2);
        assert_eq!(q.answer(1), None);
        assert_eq!(q.answer(2), Some(block(3)));
    }

    #[test]
    fn the_density_line_is_sampled_when_the_block_or_the_dimension_changes() {
        let mut cache = DensityCache::default();
        let samples = std::cell::Cell::new(0);
        let at = |cache: &mut DensityCache, dim, feet, present: bool| {
            let sample = || {
                samples.set(samples.get() + 1);
                Some(format!("Density {}", samples.get()))
            };
            cache
                .line(dim, feet, if present { Some(sample) } else { None })
                .map(str::to_owned)
        };
        // Standing still draws the first sample again and again.
        assert_eq!(
            at(&mut cache, "o", [0, 70, 0], true).as_deref(),
            Some("Density 1")
        );
        assert_eq!(
            at(&mut cache, "o", [0, 70, 0], true).as_deref(),
            Some("Density 1")
        );
        // A new block, then a new dimension at the same block, sample again.
        assert_eq!(
            at(&mut cache, "o", [1, 70, 0], true).as_deref(),
            Some("Density 2")
        );
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 3")
        );
        // No generator: nothing is drawn, nothing is kept, even on the same block.
        assert_eq!(at(&mut cache, "n", [1, 70, 0], false), None);
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 4")
        );
        // Another world clears it too.
        cache.clear();
        assert_eq!(
            at(&mut cache, "n", [1, 70, 0], true).as_deref(),
            Some("Density 5")
        );
    }
}
