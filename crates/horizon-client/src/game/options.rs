//! The options screens bound to the game: drawing, input, applying settings
//! (video, sound, chat, font, language) and the F11 fullscreen toggle.
use super::{Game, Screen};
use horizon_client::{
    Result,
    hud::HudBatch,
    lang,
    options_ui::{Event, Page},
    window::{FrameContext, InactivityFpsLimit, framerate_limit},
};
use serde_json::{Value, json};
use winit::{event::MouseButton, keyboard::KeyCode, window::Fullscreen};

pub(super) const CONTROL_HELP: &[&str] = &[
    "options-get [KEY...]           options.txt values by key, with their type, range or choices",
    "options-set KEY VALUE          set an option (enums by name, keys as key.keyboard.w, lists as JSON), apply and save it",
];

impl Game<'_> {
    pub(super) fn save_title_options(&mut self) {
        if let Some(title) = &self.title
            && (title.last_server != self.settings.file.string("lastServer")
                || title.skip_multiplayer_warning
                    != self.settings.file.bool("skipMultiplayerWarning"))
        {
            self.settings.file.set(
                "lastServer",
                horizon_client::options_file::V::Str(title.last_server.clone()),
            );
            self.settings.file.set(
                "skipMultiplayerWarning",
                horizon_client::options_file::V::Bool(title.skip_multiplayer_warning),
            );
            self.settings.save(&Self::options_path());
        }
    }
    /// `options-get` and `options-set`; None for other commands.
    pub(super) fn options_control(&mut self, words: &[&str], line: &str) -> Result<Option<Value>> {
        match words[0] {
            "options-get" => {
                let file = self.settings.to_file();
                let wanted: Vec<&str> = if words.len() > 1 {
                    words[1..].to_vec()
                } else {
                    horizon_client::options_file::keys()
                };
                let mut options = serde_json::Map::new();
                for key in wanted {
                    let described = file.describe(key).ok_or_else(|| {
                        format!("unknown option {key}; options-get lists them all")
                    })?;
                    options.insert(key.to_owned(), described);
                }
                Ok(Some(json!({ "options": options })))
            }
            "options-set" => {
                let key = words.get(1).ok_or("options-set KEY VALUE")?;
                // The value runs to the end of the line (lists, server names).
                let value = line
                    .trim_start()
                    .strip_prefix("options-set")
                    .and_then(|rest| rest.trim_start().strip_prefix(key))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or("options-set KEY VALUE: missing the value")?;
                self.settings = self.settings.with_option(key, value)?;
                // As the options screen applies a change.
                self.handle_option_events(vec![Event::Changed])?;
                match *key {
                    "lang" => self.load_language(&self.settings.language.clone()),
                    "resourcePacks" | "incompatibleResourcePacks" => self.reload_packs(),
                    _ => {}
                }
                // Labels change (a language, a value): `ui` reads the redrawn screen.
                self.ui_wait.clicked();
                let now = self.settings.to_file().describe(key);
                Ok(Some(json!({ "ok": true, "key": key, "option": now })))
            }
            _ => Ok(None),
        }
    }
    pub(super) fn options_path() -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap_or_default()
            .join("options.txt")
    }
    /// `shaderpacks/` beside `options.txt`.
    pub(super) fn shader_packs_dir() -> std::path::PathBuf {
        Self::options_path().with_file_name(horizon_client::shader_pack_list::DIR)
    }
    pub(super) fn gui_size(&self) -> (f32, f32) {
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        (b.width, b.height)
    }
    /// Open the options screens (OptionsScreen) from the title or pause menu.
    pub(super) fn open_options(&mut self, from_title: bool) {
        self.open_options_at(from_title, Page::Main);
    }
    pub(super) fn open_options_at(&mut self, from_title: bool, page: Page) {
        // Gameplay keys must not stay held behind the options screen, where
        // bindings can also change before the physical key is released.
        self.keys.clear();
        self.screen = Screen::Options;
        self.options_from_title = from_title;
        let ui = &mut self.ui.options;
        ui.open_at(!from_title && self.title.is_none(), page);
        let root = self.options.resources.clone();
        ui.languages = lang::available(&[&root]);
        self.refresh_pack_lists();
    }
    pub(super) fn draw_options(&mut self, batch: &mut HudBatch) {
        let mut ui = std::mem::take(&mut self.ui.options);
        ui.draw(&self.settings, batch, &self.gui.atlas, self.cursor);
        self.ui.options = ui;
    }
    pub(super) fn options_mouse(&mut self, pressed: bool, right: bool) -> Result<()> {
        if !pressed {
            if right {
                return Ok(());
            }
            let events = self.ui.options.release();
            self.dragging = false;
            return self.handle_option_events(events);
        }
        if !right {
            self.dragging = true;
        }
        let size = self.gui_size();
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.click(
            &mut self.settings,
            &self.gui.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            framebuffer,
            false,
            right,
        );
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_drag(&mut self) -> Result<()> {
        let size = self.gui_size();
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.click(
            &mut self.settings,
            &self.gui.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            framebuffer,
            true,
            false,
        );
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_key(&mut self, key: KeyCode, text: Option<&str>) -> Result<()> {
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.key(&mut self.settings, &self.gui.atlas, key, text);
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    /// Ctrl+A/C/V/X in the options' search boxes; false when none has focus.
    pub(super) fn options_shortcut(
        &mut self,
        shortcut: horizon_client::clipboard::EditShortcut,
    ) -> bool {
        self.ui
            .options
            .shortcut(&self.gui.atlas, shortcut, &mut self.clipboard)
    }
    pub(super) fn options_scroll(&mut self, amount: f32) -> Result<()> {
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        if self.ui.options.page() == Page::Video && self.has_control_down() {
            // VideoSettingsScreen.mouseScrolled: Ctrl with the wheel steps the GUI scale.
            let mut ui = std::mem::take(&mut self.ui.options);
            let events = ui.video_wheel_gui_scale(&mut self.settings, framebuffer, amount);
            self.ui.options = ui;
            return self.handle_option_events(events);
        }
        let size = self.gui_size();
        let events =
            self.ui
                .options
                .wheel(&mut self.settings, size, self.cursor, framebuffer, amount);
        self.handle_option_events(events)
    }
    /// Mouse buttons pressed while a key is being bound.
    pub(super) fn options_waiting_for_key(&self) -> bool {
        self.ui.options.selected_key.is_some()
    }
    pub(super) fn options_bind_mouse(&mut self, button: MouseButton) -> Result<()> {
        let events = self.ui.options.bind_mouse(&mut self.settings, button);
        self.handle_option_events(events)
    }
    pub(super) fn handle_option_events(&mut self, events: Vec<Event>) -> Result<()> {
        for e in events {
            match e {
                Event::ButtonClick => self.audio.play_button_click(),
                Event::Changed => {
                    self.settings.save(&Self::options_path());
                    self.apply_settings();
                    self.sync_agent_control();
                    self.ui.options.agent_status = self.agent_status_lines();
                }
                Event::SliderDrag => {
                    self.apply_settings();
                }
                Event::CopyAgentDetails => {
                    use horizon_client::clipboard::Clipboard;
                    let text = self.agent_details();
                    self.clipboard.set(&text);
                }
                Event::Close if self.ui.options.resource_error.is_some() => {}
                Event::Close => {
                    self.settings.save(&Self::options_path());
                    self.screen = if std::mem::take(&mut self.options_to_world_options) {
                        Screen::WorldOptions
                    } else if self.options_from_title {
                        Screen::Playing
                    } else {
                        Screen::Paused
                    };
                    self.dragging = false;
                }
                Event::Language(code) => {
                    self.load_language(&code);
                }
                Event::Packs(_) => {
                    self.settings.save(&Self::options_path());
                    self.reload_packs();
                }
                Event::OpenPackFolder => {
                    let dir = super::packs::packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::DataPacks(ids) => {
                    if let Some(title) = &mut self.title {
                        title.set_data_packs(ids);
                    }
                }
                Event::OpenDataPackFolder(dir) => {
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::OpenModFolder => {
                    let dir = Self::options_path().with_file_name(horizon_client::mod_list::DIR);
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::ListShaderPacks => {
                    self.settings.shader_packs =
                        horizon_client::shader_pack_list::scan(&Self::shader_packs_dir());
                }
                // Saved; the next frame's `shader_pack_inputs` loads the pick.
                Event::ShaderPack => self.settings.save(&Self::options_path()),
                Event::OpenShaderPackFolder => {
                    let dir = Self::shader_packs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
                Event::NativeSettings(i) => self.native_settings_refresh(i),
                Event::NativeButton(i, b) => self.native_settings_event(i, b),
                Event::Credits => self.open_win_screen(false, super::win::Finish::Options),
            }
        }
        Ok(())
    }
    /// LanguageManager.setSelected + reload: translations, then re-wrap chat.
    pub(super) fn load_language(&mut self, code: &str) {
        let root = self.options.resources.clone();
        lang::set(lang::Language::load(&[&root], code));
        self.refresh_remote_language();
        self.ui.chat.rescale(&self.gui.atlas);
        self.creative_screen.forget_items();
    }
    /// Settings that Horizon applies beyond render distance, volume and GUI scale.
    pub(super) fn apply_ui_settings(&mut self) {
        let s = &self.settings;
        if let Some(r) = self.renderer.as_mut() {
            r.gamma = s.gamma;
            r.fullbright = s.extras.fullbright;
            r.set_present_mode(s.vsync, s.horizon.present_mode);
            r.animated_textures = s.horizon.animated_textures;
            r.set_mipmap_levels(s.mipmap_levels.max(0) as u32);
            r.set_world_post(
                s.horizon.resolution_scale as f32 / 100.0,
                s.horizon.fxaa,
                s.horizon.taa,
                s.horizon.upscaling.ratio(),
            );
            r.set_hdr(
                s.horizon.hdr,
                s.horizon.paper_white_nits() as f32,
                s.horizon.peak_nits() as f32,
                s.horizon.hdr_black_level as f32 / 100.0,
                s.horizon.hdr_ui_brightness as f32 / 100.0,
                &Self::options_path().with_file_name(horizon_client::graphics::HDR_MARKER),
            );
        }
        // Window.updateFullscreenIfChanged: only a changed option asks the
        // window to change, so applying settings never undoes a fullscreen
        // the compositor set up.
        if let Some(w) = &self.window
            && let Some(want) = self.fullscreen.request(s.fullscreen)
        {
            w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
        }
        let mut volumes = s.volumes;
        volumes[0] = 1.0;
        horizon_client::settings::set_source_volumes(volumes);
        let chat = s.chat_options();
        let rescale = {
            let o = &self.ui.chat.options;
            o.width != chat.width || o.scale != chat.scale || o.line_spacing != chat.line_spacing
        };
        self.ui.chat.options = chat;
        if rescale {
            self.ui.chat.rescale(&self.gui.atlas);
        }
        let font = &mut self.gui.atlas.font;
        if font.uniform != s.force_unicode || font.japanese != s.japanese_glyphs {
            font.uniform = s.force_unicode;
            font.japanese = s.japanese_glyphs;
            font.reset_cache();
            self.ui.chat.rescale(&self.gui.atlas);
        }
    }
    /// `Window.updateFullscreenState`: a fullscreen change made by the window
    /// system is adopted by the option (`Minecraft.fullscreenStateChanged`).
    pub(super) fn observe_fullscreen(&mut self) {
        let Some(w) = &self.window else { return };
        if let Some(now) = self.fullscreen.observe(w.fullscreen().is_some())
            && self.settings.fullscreen != now
        {
            self.settings.fullscreen = now;
            self.settings.save(&Self::options_path());
        }
    }
    /// F11 (key.fullscreen): Window.toggleFullScreen, remembered in options.txt.
    pub(super) fn toggle_fullscreen(&mut self) {
        self.settings.fullscreen = !self.settings.fullscreen;
        self.settings.save(&Self::options_path());
        self.apply_ui_settings();
    }
    /// `FramerateLimitTracker.getFramerateLimit`: the Max Framerate option,
    /// throttled when minimised, idle, or in a menu without a world.
    pub(super) fn framerate_limit(&self) -> i32 {
        let context = FrameContext {
            inactivity: if self.settings.inactivity_fps_limit == "minimized" {
                InactivityFpsLimit::Minimized
            } else {
                InactivityFpsLimit::Afk
            },
            iconified: self
                .window
                .as_ref()
                .is_some_and(|w| w.is_minimized().unwrap_or(false)),
            focused: self.focused,
            exclusive_fullscreen: false,
            now_ms: self.started.elapsed().as_millis() as i64,
            latest_input_ms: self.last_input.duration_since(self.started).as_millis() as i64,
            has_level: self.title.is_none(),
            has_screen: self.title.is_some() || self.screen != super::Screen::Playing,
            has_overlay: false,
        };
        let limit = framerate_limit(self.settings.max_fps as i32, context);
        self.settings
            .horizon
            .battery_limit(limit, self.title.is_none(), self.focused)
    }
    /// `Minecraft.runTick`'s frame limiter: `FramerateLimiter.limitDisplayFPS`
    /// below 260 frames per second, sleeping then spin-waiting until the
    /// frame's slot ends (not in scripted frame-count runs).
    pub(super) fn limit_frame(&mut self) {
        if self.options.frames.is_some() {
            return;
        }
        let limit = self.framerate_limit();
        match self.settings.horizon.pacing {
            horizon_client::graphics::Pacing::Precise => {
                horizon_client::window::end_frame(&mut self.limiter, limit, &mut self.frame_clock)
            }
            horizon_client::graphics::Pacing::Efficient => {
                let mut clock = horizon_client::window::SleepingClock(self.frame_clock);
                horizon_client::window::end_frame(&mut self.limiter, limit, &mut clock);
            }
        }
    }
}
