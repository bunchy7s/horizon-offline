//! Player settings persisted in `options.txt` with vanilla's keys and value
//! encodings (Options.processOptions); lines Horizon does not know are kept.
//! The option screens live in `options_ui`.
use crate::keybinds::KeyBinds;
use crate::options_file::{OptionsFile, V};
use horizon_protocol::packets::configuration::ClientInformation;
use std::{fs, path::Path};

pub use horizon_client_core::sound_source::SOUND_SOURCES;

/// Per-source volumes for the mixer (soundCategory_* except master).
static SOURCE_VOLUMES: std::sync::RwLock<[f32; 11]> = std::sync::RwLock::new([1.0; 11]);
pub fn set_source_volumes(volumes: [f32; 11]) {
    *SOURCE_VOLUMES.write().unwrap_or_else(|e| e.into_inner()) = volumes;
}
/// The SoundSource an event plays on. Vanilla callers pick the source; this
/// follows their conventions by event family.
pub fn sound_source(event: &str) -> usize {
    let e = event.trim_start_matches("minecraft:");
    let index = |name: &str| SOUND_SOURCES.iter().position(|n| *n == name).unwrap_or(0);
    const HOSTILE: [&str; 25] = [
        "zombie",
        "skeleton",
        "creeper",
        "spider",
        "cave_spider",
        "enderman",
        "witch",
        "slime",
        "magma_cube",
        "blaze",
        "ghast",
        "drowned",
        "husk",
        "stray",
        "phantom",
        "pillager",
        "vindicator",
        "evoker",
        "illusioner",
        "ravager",
        "guardian",
        "elder_guardian",
        "silverfish",
        "zombified_piglin",
        "wither_skeleton",
    ];
    if e == "block.potent_sulfur.noxious_gas" {
        // PotentSulfurBlock.animateTick, its only caller, plays it as AMBIENT.
        index("ambient")
    } else if e.starts_with("music_disc.") {
        index("record")
    } else if e.starts_with("music.") {
        index("music")
    } else if e.starts_with("weather.") || e.starts_with("entity.lightning_bolt") {
        index("weather")
    } else if e.starts_with("block.") {
        index("block")
    } else if e.starts_with("ambient.") {
        index("ambient")
    } else if e.starts_with("ui.") {
        index("ui")
    } else if e.starts_with("entity.player.")
        || e.starts_with("item.")
        || e.starts_with("entity.item.")
        || e.starts_with("entity.experience_orb")
        || e.starts_with("entity.arrow")
    {
        index("player")
    } else if let Some(rest) = e.strip_prefix("entity.") {
        let mob = rest.split('.').next().unwrap_or("");
        if HOSTILE.contains(&mob) {
            index("hostile")
        } else {
            index("neutral")
        }
    } else {
        index("master")
    }
}
/// Volume multiplier of a sound source by index.
pub fn source_volume(i: usize) -> f32 {
    if i == 0 {
        1.0
    } else {
        SOURCE_VOLUMES.read().unwrap_or_else(|e| e.into_inner())[i]
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Degrees, 30..=110 (options.txt stores (fov - 70) / 40).
    pub fov: f32,
    /// Chunks, 2..=32.
    pub render_distance: i32,
    /// Chunks, 5..=32.
    pub simulation_distance: i32,
    /// 0..1, vanilla default 0.5.
    pub sensitivity: f32,
    /// 0..1 (soundCategory_master).
    pub master_volume: f32,
    /// Volumes of every other sound source, indexed like SOUND_SOURCES.
    pub volumes: [f32; 11],
    /// Empty for system default, or a specific audio device name.
    pub sound_device: String,
    /// Requested scale (0 = auto); rendering caps it to the current framebuffer.
    pub gui_scale: u32,
    pub view_bobbing: bool,
    /// Brightness 0..1 (gamma).
    pub gamma: f32,
    /// 10..=260 (260 = unlimited).
    pub max_fps: u32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Smooth lighting (ao).
    pub smooth_lighting: bool,
    /// "false", "fast" or "true" (renderClouds).
    pub clouds: String,
    pub cloud_range: i32,
    /// 0 all, 1 decreased, 2 minimal.
    pub particles: u8,
    pub entity_distance: f32,
    pub entity_shadows: bool,
    pub biome_blend: i32,
    /// "fast", "fancy", "fabulous" or "custom".
    pub graphics_preset: String,
    pub mipmap_levels: i32,
    pub weather_radius: i32,
    pub chunk_fade: f32,
    pub vignette: bool,
    pub autosave_indicator: bool,
    pub cutout_leaves: bool,
    pub improved_transparency: bool,
    /// 0 off, 1 crosshair, 2 hotbar.
    pub attack_indicator: u8,
    pub inactivity_fps_limit: String,
    pub prioritize_chunk_updates: u8,
    pub menu_blur: i32,
    pub invert_y: bool,
    pub mouse_wheel_sensitivity: f32,
    pub discrete_scroll: bool,
    pub toggle_crouch: bool,
    pub toggle_sprint: bool,
    pub auto_jump: bool,
    pub language: String,
    pub force_unicode: bool,
    pub japanese_glyphs: bool,
    pub chat_visibility: u8,
    pub chat_colors: bool,
    pub chat_links: bool,
    pub chat_links_prompt: bool,
    pub chat_opacity: f32,
    /// Chat Delay in seconds (`chatDelay`): player chat shows this far apart.
    pub chat_delay: f64,
    pub text_background_opacity: f32,
    pub chat_scale: f32,
    pub chat_width: f32,
    pub chat_height_focused: f32,
    pub chat_height_unfocused: f32,
    pub chat_line_spacing: f32,
    pub auto_suggestions: bool,
    pub reduced_debug_info: bool,
    pub high_contrast: bool,
    /// `showSubtitles`: the closed-caption overlay.
    pub show_subtitles: bool,
    /// `notificationDisplayTime`: a multiplier for subtitle and toast times.
    pub notification_display_time: f32,
    pub main_hand_left: bool,
    /// Enabled resource packs, bottom first (resourcePacks).
    pub resource_packs: Vec<String>,
    pub keys: KeyBinds,
    /// Every option in its stored form (the typed fields above mirror it).
    pub file: OptionsFile,
    pub invert_x: bool,
    /// Allow widgets to replace the default mouse cursor icon.
    pub allow_cursor_changes: bool,
    pub toggle_attack: bool,
    pub toggle_use: bool,
    /// Taps of the sprint key counted as double taps (sprintWindow).
    pub sprint_window: i32,
    /// Show the operator items tab in a creative inventory.
    pub operator_items_tab: bool,
    /// Allow platform quit shortcuts to close the game window.
    pub quit_shortcuts: bool,
    /// On macOS, turn Control-click into a right click.
    pub ctrl_click_emulates_right_click: bool,
    /// Horizon's own graphics settings (`horizon-graphics.txt`, not options.txt).
    pub horizon: crate::graphics::Graphics,
    /// Controller settings (`horizon-controller.txt`, Horizon's own).
    pub controller: crate::gamepad::PadSettings,
    /// Horizon's optional extras (`horizon-extras.txt`), all off by default.
    pub extras: crate::extras::Extras,
    /// The Horizon mods turned on (`horizon-mods.txt`), all off by default.
    pub mods: crate::mod_list::ModList,
    /// The packs in `shaderpacks/`, listed when the Shader Packs page opens; the pick is
    /// `horizon.shader_pack`.
    pub shader_packs: Vec<crate::shader_pack_list::Entry>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fov: 70.0,
            render_distance: 12,
            simulation_distance: 12,
            sensitivity: 0.5,
            master_volume: 1.0,
            volumes: [1.0; 11],
            sound_device: String::new(),
            gui_scale: 0,
            view_bobbing: true,
            gamma: 0.5,
            // Vanilla's default, as for an options.txt without the key: a
            // capped frame rate leaves the cores idle when VSync is off.
            max_fps: 120,
            vsync: true,
            fullscreen: false,
            smooth_lighting: true,
            clouds: "true".into(),
            cloud_range: 128,
            particles: 0,
            entity_distance: 1.0,
            entity_shadows: true,
            biome_blend: 2,
            graphics_preset: "fancy".into(),
            mipmap_levels: 4,
            weather_radius: 10,
            chunk_fade: 0.75,
            vignette: true,
            autosave_indicator: true,
            cutout_leaves: true,
            improved_transparency: false,
            attack_indicator: 1,
            inactivity_fps_limit: "afk".into(),
            prioritize_chunk_updates: 0,
            menu_blur: 5,
            invert_y: false,
            mouse_wheel_sensitivity: 1.0,
            discrete_scroll: false,
            allow_cursor_changes: true,
            toggle_crouch: false,
            toggle_sprint: false,
            auto_jump: false,
            language: "en_us".into(),
            force_unicode: false,
            japanese_glyphs: false,
            chat_visibility: 0,
            chat_colors: true,
            chat_links: true,
            chat_links_prompt: true,
            chat_opacity: 1.0,
            chat_delay: 0.0,
            text_background_opacity: 0.5,
            chat_scale: 1.0,
            chat_width: 1.0,
            chat_height_focused: 1.0,
            chat_height_unfocused: 70.0 / 160.0,
            chat_line_spacing: 0.0,
            auto_suggestions: true,
            reduced_debug_info: false,
            high_contrast: false,
            show_subtitles: false,
            notification_display_time: 1.0,
            main_hand_left: false,
            resource_packs: Vec::new(),
            keys: KeyBinds::default(),
            file: OptionsFile::default(),
            invert_x: false,
            toggle_attack: false,
            toggle_use: false,
            sprint_window: 7,
            operator_items_tab: false,
            quit_shortcuts: true,
            ctrl_click_emulates_right_click: false,
            horizon: Default::default(),
            controller: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            shader_packs: Vec::new(),
        }
    }
}
fn put_f(f: &mut OptionsFile, key: &str, v: f32) {
    if v != f.double(key) as f32 {
        f.set(key, V::Double(f64::from(v)));
    }
}
fn put_i(f: &mut OptionsFile, key: &str, v: i32) {
    if v != f.int(key) {
        f.set(key, V::Int(v));
    }
}
fn put_b(f: &mut OptionsFile, key: &str, v: bool) {
    if v != f.bool(key) {
        f.set(key, V::Bool(v));
    }
}
fn put_name(f: &mut OptionsFile, key: &str, v: &str) {
    if v != f.name(key) {
        f.set_name(key, v);
    }
}
fn put_s(f: &mut OptionsFile, key: &str, v: &str) {
    if v != f.string(key) {
        f.set(key, V::Str(v.to_owned()));
    }
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        let horizon = path
            .parent()
            .map(crate::graphics::Graphics::load)
            .unwrap_or_default();
        let controller = path
            .parent()
            .map(crate::gamepad::PadSettings::load)
            .unwrap_or_default();
        let extras = path
            .parent()
            .map(crate::extras::Extras::load)
            .unwrap_or_default();
        let mods = path
            .parent()
            .map(crate::mod_list::ModList::load)
            .unwrap_or_default();
        let settings = match fs::read(path) {
            Ok(bytes) => Self {
                horizon,
                controller,
                extras,
                mods,
                ..Self::parse(&String::from_utf8_lossy(&bytes))
            },
            Err(_) => Self {
                horizon,
                controller,
                extras,
                mods,
                ..Self::default()
            },
        };
        settings.describe_for_crashes();
        settings
    }
    /// The render settings and resource packs a crash report lists.
    fn describe_for_crashes(&self) {
        let on = |b: bool| if b { "on" } else { "off" };
        let scale = match self.gui_scale {
            0 => "auto".to_owned(),
            n => n.to_string(),
        };
        crate::crash::set_detail(
            "Render Settings",
            format!(
                "render distance {}, simulation distance {}, graphics {}, FOV {:.0}, max FPS {}, VSync {}, fullscreen {}, GUI scale {scale}, smooth lighting {}, clouds {}, biome blend {}, mipmap levels {}",
                self.render_distance,
                self.simulation_distance,
                self.graphics_preset,
                self.fov,
                if self.max_fps >= 260 {
                    "unlimited".to_owned()
                } else {
                    self.max_fps.to_string()
                },
                on(self.vsync),
                on(self.fullscreen),
                on(self.smooth_lighting),
                self.clouds,
                self.biome_blend,
                self.mipmap_levels,
            ),
        );
        crate::crash::set_detail(
            "Resource Packs",
            if self.resource_packs.is_empty() {
                "vanilla".to_owned()
            } else {
                self.resource_packs.join(", ")
            },
        );
    }
    pub fn parse(text: &str) -> Self {
        Self::from_file(OptionsFile::parse(text))
    }
    /// Typed view of an options file.
    pub fn from_file(f: OptionsFile) -> Self {
        let mut keys = KeyBinds::default();
        for (name, _, _) in crate::keybinds::MAPPINGS {
            keys.set(name, &f.key_binding(name));
        }
        let mut volumes = [1.0; 11];
        for (i, source) in SOUND_SOURCES.iter().enumerate() {
            volumes[i] = f.double(&format!("soundCategory_{source}")) as f32;
        }
        Self {
            fov: f.int("fov") as f32,
            render_distance: f.int("renderDistance"),
            simulation_distance: f.int("simulationDistance"),
            sensitivity: f.double("mouseSensitivity") as f32,
            master_volume: volumes[0],
            volumes,
            sound_device: f.string("soundDevice"),
            gui_scale: f.int("guiScale").max(0) as u32,
            view_bobbing: f.bool("bobView"),
            gamma: f.double("gamma") as f32,
            max_fps: f.int("maxFps") as u32,
            vsync: f.bool("enableVsync"),
            fullscreen: f.bool("fullscreen"),
            smooth_lighting: f.bool("ao"),
            clouds: f.name("renderClouds"),
            cloud_range: f.int("cloudRange"),
            particles: f.int("particles") as u8,
            entity_distance: f.double("entityDistanceScaling") as f32,
            entity_shadows: f.bool("entityShadows"),
            biome_blend: f.int("biomeBlendRadius"),
            graphics_preset: f.name("graphicsPreset"),
            mipmap_levels: f.int("mipmapLevels"),
            weather_radius: f.int("weatherRadius"),
            chunk_fade: f.double("chunkSectionFadeInTime") as f32,
            vignette: f.bool("vignette"),
            autosave_indicator: f.bool("showAutosaveIndicator"),
            cutout_leaves: f.bool("cutoutLeaves"),
            improved_transparency: f.bool("improvedTransparency"),
            attack_indicator: f.int("attackIndicator") as u8,
            inactivity_fps_limit: f.name("inactivityFpsLimit"),
            prioritize_chunk_updates: f.int("prioritizeChunkUpdates") as u8,
            menu_blur: f.int("menuBackgroundBlurriness"),
            invert_x: f.bool("invertXMouse"),
            invert_y: f.bool("invertYMouse"),
            mouse_wheel_sensitivity: f.double("mouseWheelSensitivity") as f32,
            discrete_scroll: f.bool("discrete_mouse_scroll"),
            allow_cursor_changes: f.bool("allowCursorChanges"),
            toggle_crouch: f.bool("toggleCrouch"),
            toggle_sprint: f.bool("toggleSprint"),
            toggle_attack: f.bool("toggleAttack"),
            toggle_use: f.bool("toggleUse"),
            sprint_window: f.int("sprintWindow"),
            operator_items_tab: f.bool("operatorItemsTab"),
            quit_shortcuts: f.bool("quitShortcuts"),
            ctrl_click_emulates_right_click: f.bool("ctrlClickEmulatesRightClick"),
            horizon: Default::default(),
            controller: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            shader_packs: Vec::new(),
            auto_jump: f.bool("autoJump"),
            language: f.string("lang"),
            force_unicode: f.bool("forceUnicodeFont"),
            japanese_glyphs: f.bool("japaneseGlyphVariants"),
            chat_visibility: f.int("chatVisibility") as u8,
            chat_colors: f.bool("chatColors"),
            chat_links: f.bool("chatLinks"),
            chat_links_prompt: f.bool("chatLinksPrompt"),
            chat_opacity: f.double("chatOpacity") as f32,
            chat_delay: f.double("chatDelay"),
            text_background_opacity: f.double("textBackgroundOpacity") as f32,
            chat_scale: f.double("chatScale") as f32,
            chat_width: f.double("chatWidth") as f32,
            chat_height_focused: f.double("chatHeightFocused") as f32,
            chat_height_unfocused: f.double("chatHeightUnfocused") as f32,
            chat_line_spacing: f.double("chatLineSpacing") as f32,
            auto_suggestions: f.bool("autoSuggestions"),
            reduced_debug_info: f.bool("reducedDebugInfo"),
            high_contrast: f.bool("highContrast"),
            show_subtitles: f.bool("showSubtitles"),
            notification_display_time: f.double("notificationDisplayTime") as f32,
            main_hand_left: f.name("mainHand") == "left",
            resource_packs: f.list("resourcePacks"),
            keys,
            file: f,
        }
    }
    /// `Options.createInformation`: what the server learns about this player.
    pub fn client_information(&self, view_distance: i8) -> ClientInformation {
        ClientInformation {
            chat_visibility: self.chat_visibility as i32,
            model_customisation: self.file.model_customisation(),
            main_hand: i32::from(!self.main_hand_left),
            allows_listing: self.file.bool("allowServerListing"),
            ..ClientInformation::new(view_distance)
        }
    }
    /// The options file reflecting the typed fields; values the game did not
    /// change keep their stored form.
    pub fn to_file(&self) -> OptionsFile {
        let mut f = self.file.clone();
        put_i(&mut f, "fov", self.fov as i32);
        put_i(&mut f, "renderDistance", self.render_distance);
        put_i(&mut f, "simulationDistance", self.simulation_distance);
        put_f(&mut f, "mouseSensitivity", self.sensitivity);
        for (i, source) in SOUND_SOURCES.iter().enumerate() {
            let v = if i == 0 {
                self.master_volume
            } else {
                self.volumes[i]
            };
            put_f(&mut f, &format!("soundCategory_{source}"), v);
        }
        put_s(&mut f, "soundDevice", &self.sound_device);
        put_i(&mut f, "guiScale", self.gui_scale as i32);
        put_b(&mut f, "bobView", self.view_bobbing);
        put_f(&mut f, "gamma", self.gamma);
        put_i(&mut f, "maxFps", self.max_fps as i32);
        put_b(&mut f, "enableVsync", self.vsync);
        put_b(&mut f, "fullscreen", self.fullscreen);
        put_b(&mut f, "ao", self.smooth_lighting);
        put_name(&mut f, "renderClouds", &self.clouds);
        put_i(&mut f, "cloudRange", self.cloud_range);
        put_i(&mut f, "particles", i32::from(self.particles));
        put_f(&mut f, "entityDistanceScaling", self.entity_distance);
        put_b(&mut f, "entityShadows", self.entity_shadows);
        put_i(&mut f, "biomeBlendRadius", self.biome_blend);
        put_name(&mut f, "graphicsPreset", &self.graphics_preset);
        put_i(&mut f, "mipmapLevels", self.mipmap_levels);
        put_i(&mut f, "weatherRadius", self.weather_radius);
        put_f(&mut f, "chunkSectionFadeInTime", self.chunk_fade);
        put_b(&mut f, "vignette", self.vignette);
        put_b(&mut f, "showAutosaveIndicator", self.autosave_indicator);
        put_b(&mut f, "cutoutLeaves", self.cutout_leaves);
        put_b(&mut f, "improvedTransparency", self.improved_transparency);
        put_i(&mut f, "attackIndicator", i32::from(self.attack_indicator));
        put_name(&mut f, "inactivityFpsLimit", &self.inactivity_fps_limit);
        put_i(
            &mut f,
            "prioritizeChunkUpdates",
            i32::from(self.prioritize_chunk_updates),
        );
        put_i(&mut f, "menuBackgroundBlurriness", self.menu_blur);
        put_b(&mut f, "invertXMouse", self.invert_x);
        put_b(&mut f, "invertYMouse", self.invert_y);
        put_f(
            &mut f,
            "mouseWheelSensitivity",
            self.mouse_wheel_sensitivity,
        );
        put_b(&mut f, "discrete_mouse_scroll", self.discrete_scroll);
        put_b(&mut f, "allowCursorChanges", self.allow_cursor_changes);
        put_b(&mut f, "toggleCrouch", self.toggle_crouch);
        put_b(&mut f, "toggleSprint", self.toggle_sprint);
        put_b(&mut f, "toggleAttack", self.toggle_attack);
        put_b(&mut f, "toggleUse", self.toggle_use);
        put_i(&mut f, "sprintWindow", self.sprint_window);
        put_b(&mut f, "operatorItemsTab", self.operator_items_tab);
        put_b(&mut f, "quitShortcuts", self.quit_shortcuts);
        put_b(
            &mut f,
            "ctrlClickEmulatesRightClick",
            self.ctrl_click_emulates_right_click,
        );
        put_b(&mut f, "autoJump", self.auto_jump);
        put_s(&mut f, "lang", &self.language);
        put_b(&mut f, "forceUnicodeFont", self.force_unicode);
        put_b(&mut f, "japaneseGlyphVariants", self.japanese_glyphs);
        put_i(&mut f, "chatVisibility", i32::from(self.chat_visibility));
        put_b(&mut f, "chatColors", self.chat_colors);
        put_b(&mut f, "chatLinks", self.chat_links);
        put_b(&mut f, "chatLinksPrompt", self.chat_links_prompt);
        put_f(&mut f, "chatOpacity", self.chat_opacity);
        if self.chat_delay != f.double("chatDelay") {
            f.set("chatDelay", V::Double(self.chat_delay));
        }
        put_f(
            &mut f,
            "textBackgroundOpacity",
            self.text_background_opacity,
        );
        put_f(&mut f, "chatScale", self.chat_scale);
        put_f(&mut f, "chatWidth", self.chat_width);
        put_f(&mut f, "chatHeightFocused", self.chat_height_focused);
        put_f(&mut f, "chatHeightUnfocused", self.chat_height_unfocused);
        put_f(&mut f, "chatLineSpacing", self.chat_line_spacing);
        put_b(&mut f, "autoSuggestions", self.auto_suggestions);
        put_b(&mut f, "reducedDebugInfo", self.reduced_debug_info);
        put_b(&mut f, "highContrast", self.high_contrast);
        put_b(&mut f, "showSubtitles", self.show_subtitles);
        put_f(
            &mut f,
            "notificationDisplayTime",
            self.notification_display_time,
        );
        put_name(
            &mut f,
            "mainHand",
            if self.main_hand_left { "left" } else { "right" },
        );
        if self.resource_packs != f.list("resourcePacks") {
            f.set("resourcePacks", V::List(self.resource_packs.clone()));
        }
        for (name, key) in self.keys.saved() {
            put_s(&mut f, &format!("key_{name}"), &key);
        }
        f
    }
    /// These settings with one `options.txt` value changed (`horizon-ctl
    /// options-set`, as `OptionsFile::set_text` reads it). Horizon's own
    /// files (graphics, controller, extras, the mod list) aren't in
    /// `options.txt`, so they stay as they are.
    pub fn with_option(&self, key: &str, value: &str) -> Result<Self, String> {
        let mut file = self.to_file();
        file.set_text(key, value)?;
        Ok(Self {
            horizon: self.horizon.clone(),
            controller: self.controller.clone(),
            extras: self.extras.clone(),
            mods: self.mods.clone(),
            ..Self::from_file(file)
        })
    }
    /// The `options.txt` text (Options.save).
    pub fn serialize(&self) -> String {
        self.to_file().serialize()
    }
    pub fn save(&self, path: &Path) {
        self.describe_for_crashes();
        let _ = fs::write(path, self.serialize());
        if let Some(dir) = path.parent() {
            self.horizon.save(dir);
            self.controller.save(dir);
            self.extras.save(dir);
            self.mods.save(dir);
        }
    }
    /// Volume of a sound source (master excluded).
    pub fn volume(&self, source: &str) -> f32 {
        SOUND_SOURCES
            .iter()
            .position(|n| *n == source)
            .map_or(1.0, |i| {
                if i == 0 {
                    self.master_volume
                } else {
                    self.volumes[i]
                }
            })
    }
    /// `Options.getBackgroundColor(float)`: the text backdrop colour, `default`
    /// opacity unless `backgroundForChatOnly` is off.
    pub fn text_background(&self, default: f32) -> u32 {
        crate::hud_layout::background_color(
            self.file.bool("backgroundForChatOnly"),
            f64::from(self.text_background_opacity),
            default,
        )
    }
    pub fn chat_options(&self) -> crate::chat::ChatOptions {
        crate::chat::ChatOptions {
            opacity: f64::from(self.chat_opacity),
            background_opacity: f64::from(self.text_background_opacity),
            scale: f64::from(self.chat_scale),
            width: f64::from(self.chat_width),
            height_focused: f64::from(self.chat_height_focused),
            height_unfocused: f64::from(self.chat_height_unfocused),
            line_spacing: f64::from(self.chat_line_spacing),
            colors: self.chat_colors,
        }
    }
}

/// One option instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opt {
    Fov,
    RenderDistance,
    SimulationDistance,
    Sensitivity,
    WheelSensitivity,
    DiscreteScroll,
    InvertX,
    InvertY,
    AllowCursorChanges,
    Volume(usize),
    SoundDevice,
    GuiScale,
    ViewBobbing,
    Gamma,
    MaxFps,
    Vsync,
    Fullscreen,
    SmoothLighting,
    Clouds,
    CloudRange,
    Particles,
    EntityDistance,
    EntityShadows,
    BiomeBlend,
    Preset,
    Mipmaps,
    WeatherRadius,
    ChunkFade,
    Vignette,
    Autosave,
    CutoutLeaves,
    ImprovedTransparency,
    AttackIndicator,
    InactivityFps,
    ChunkBuilder,
    MenuBlur,
    ToggleCrouch,
    ToggleSprint,
    ToggleAttack,
    ToggleUse,
    AutoJump,
    SprintWindow,
    QuitShortcuts,
    CtrlClickEmulatesRightClick,
    OperatorItemsTab,
    ChatVisibility,
    ChatColors,
    ChatLinks,
    ChatLinksPrompt,
    ChatOpacity,
    TextBackgroundOpacity,
    ChatScale,
    ChatWidth,
    ChatHeightFocused,
    ChatHeightUnfocused,
    ChatLineSpacing,
    AutoSuggestions,
    HideMatchedNames,
    ReducedDebugInfo,
    OnlySecureChat,
    SaveChatDrafts,
    HighContrast,
    ShowSubtitles,
    MusicFrequency,
    NotificationDisplayTime,
    Narrator,
    TextBackground,
    ChatDelay,
    DistortionEffects,
    FovEffects,
    DarknessPulsing,
    DamageTilt,
    GlintSpeed,
    GlintStrength,
    HideLightningFlashes,
    MonochromeLogo,
    PanoramaSpeed,
    HideSplashTexts,
    NarratorHotkey,
    RotateWithMinecarts,
    HighContrastBlockOutlines,
    ForceUnicode,
    JapaneseGlyphs,
    /// Horizon's controller settings (`horizon-controller.txt`).
    PadEnabled,
    PadVibration,
    PadAdaptiveTriggers,
    PadLightbar,
    PadLookSensitivity,
    PadInvertX,
    PadInvertY,
    PadToggleSneak,
    PadMoveDeadzone,
    PadLookDeadzone,
    PadGlyphs,
    PadMenuNav,
    PadKeyboard,
    /// `options.modelPart.*`: one skin layer (index into `options_file::MODEL_PARTS`).
    ModelPart(usize),
    MainHand,
    AllowServerListing,
    RealmsNotifications,
    InGameNotification,
    SharePresence,
    DirectionalAudio,
    MusicToast,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_option_set_keeps_horizon_files() {
        let mut s = Settings::parse("fov:70\n");
        s.mods.enabled.insert("zoom".into());
        s.extras.minimap = true;
        s.horizon.fog = !s.horizon.fog;
        s.controller.vibration = !s.controller.vibration;
        let set = s.with_option("fov", "90").unwrap();
        assert_eq!(set.fov, 90.0);
        assert_eq!(set.mods, s.mods);
        assert_eq!(set.extras, s.extras);
        assert_eq!(set.horizon, s.horizon);
        assert_eq!(set.controller, s.controller);
        assert!(s.with_option("fov", "200").is_err());
    }
    #[test]
    fn gui_scale_survives_temporary_small_framebuffer() {
        for requested in [0, 1, 2, 6] {
            let settings = Settings::parse(&format!("guiScale:{requested}\n"));
            // Hyprland's initial surface can be smaller than the fullscreen one.
            for (framebuffer, maximum) in [([715, 860], 2), ([3400, 1913], 7)] {
                let effective = crate::window::calculate_scale(
                    framebuffer,
                    settings.gui_scale as i32,
                    settings.force_unicode,
                );
                assert_eq!(
                    effective,
                    if requested == 0 {
                        maximum
                    } else {
                        (requested as i32).min(maximum)
                    }
                );
                let saved = Settings::parse(&settings.serialize());
                assert_eq!(saved.gui_scale, requested);
            }
        }
    }

    #[test]
    fn round_trips_vanilla_options_txt() {
        let text = "version:4000\nfov:0.5\nrenderDistance:16\nmaxFps:260\nrenderClouds:\"fast\"\nkey_key.jump:key.keyboard.j\nsoundCategory_music:0.25\nresourcePacks:[\"vanilla\",\"file/x.zip\"]\nlang:fr_fr\nsomeFutureOption:42\n";
        let s = Settings::parse(text);
        assert_eq!(s.fov, 90.0);
        assert_eq!(s.render_distance, 16);
        assert_eq!(s.max_fps, 260);
        assert_eq!(s.clouds, "fast");
        assert_eq!(s.keys.get("key.jump"), "key.keyboard.j");
        assert_eq!(s.volume("music"), 0.25);
        assert_eq!(s.resource_packs, ["vanilla", "file/x.zip"]);
        assert_eq!(s.language, "fr_fr");
        let out = s.serialize();
        assert!(out.starts_with("version:"));
        assert!(out.contains("\nkey_key.jump:key.keyboard.j\n"));
        // Options.save writes only the options it knows.
        assert!(!out.contains("someFutureOption"));
        assert!(out.contains("\nrenderClouds:\"fast\"\n"));
        let again = Settings::parse(&out);
        assert_eq!(again.fov, 90.0);
        assert_eq!(again.keys.get("key.jump"), "key.keyboard.j");
    }
    #[test]
    fn no_options_txt_paces_frames_like_an_empty_one() {
        let (fresh, empty) = (Settings::default(), Settings::parse(""));
        assert_eq!(fresh.max_fps, 120);
        assert_eq!(fresh.max_fps, empty.max_fps);
        assert_eq!(fresh.vsync, empty.vsync);
        assert_eq!(fresh.inactivity_fps_limit, empty.inactivity_fps_limit);
        assert_eq!(fresh.render_distance, empty.render_distance);
        assert_eq!(fresh.simulation_distance, empty.simulation_distance);
    }
    #[test]
    fn subtitle_options() {
        let s = Settings::parse("showSubtitles:true\nnotificationDisplayTime:2.5\n");
        assert!(s.show_subtitles);
        assert_eq!(s.notification_display_time, 2.5);
        let again = Settings::parse(&s.serialize());
        assert!(again.show_subtitles);
        assert_eq!(again.notification_display_time, 2.5);
        let d = Settings::parse("");
        assert!(!d.show_subtitles);
        assert_eq!(d.notification_display_time, 1.0);
        // Codec.doubleRange(0.5, 10.0) rejects other values; the default stays.
        assert_eq!(
            Settings::parse("notificationDisplayTime:50\n").notification_display_time,
            1.0
        );
    }
    #[test]
    fn sound_device_option() {
        let s = Settings::parse("soundDevice:\"Headphones\"\n");
        assert_eq!(s.sound_device, "Headphones");
        let serialized = s.serialize();
        assert!(serialized.contains("soundDevice:\"Headphones\""));
        let again = Settings::parse(&serialized);
        assert_eq!(again.sound_device, "Headphones");
        let empty = Settings::parse("");
        assert_eq!(empty.sound_device, "");
    }
}
