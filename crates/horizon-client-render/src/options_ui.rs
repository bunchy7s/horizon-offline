//! The options screens: OptionsScreen, Video Settings, Music & Sounds,
//! Controls, Key Binds, Mouse Settings, Chat Settings, Accessibility,
//! Language and Resource Packs, laid out like vanilla's HeaderAndFooter /
//! OptionsList screens and drawn with the shared widgets.
use std::time::Instant;

pub use crate::settings::Opt;

use crate::{
    gamepad::{self, GlyphStyle},
    hud::{HudAtlas, HudBatch, rgba},
    keybinds::{self, CATEGORIES, KeyBinds, MAPPINGS},
    lang::{self, LanguageInfo},
    settings::{SOUND_SOURCES, Settings},
    text::{self, Style, Text},
    ui,
};

fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}
fn generic(caption: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[tr(caption), value])
}
fn percent(caption: &str, value: f32) -> Text {
    Text::translate(
        "options.percent_value",
        &[
            tr(caption),
            Text::literal(((f64::from(value) * 100.0) as i32).to_string()),
        ],
    )
}
fn on_off(v: bool) -> Text {
    tr(if v { "options.on" } else { "options.off" })
}
fn chat_px(caption: &str, px: f32) -> Text {
    Text::translate(
        "options.pixel_value",
        &[tr(caption), Text::literal((px as i32).to_string())],
    )
}
pub(crate) enum Kind {
    /// Slider position 0..1.
    Slider(f32),
    Cycle,
}
/// `PlayerModelPart` keys, in vanilla's bit order (`modelPart_*` in options.txt).
const MODEL_PART_KEYS: [&str; 7] = [
    "modelPart_cape",
    "modelPart_jacket",
    "modelPart_left_sleeve",
    "modelPart_right_sleeve",
    "modelPart_left_pants_leg",
    "modelPart_right_pants_leg",
    "modelPart_hat",
];
/// The Accessibility Settings page:
/// `AccessibilityOptionsScreen.options`, with the Controls link inserted separately.
pub(crate) const ACCESSIBILITY_OPTIONS: &[Opt] = &[
    Opt::Narrator,
    Opt::ShowSubtitles,
    Opt::HighContrast,
    Opt::MenuBlur,
    Opt::TextBackgroundOpacity,
    Opt::TextBackground,
    Opt::ChatOpacity,
    Opt::ChatLineSpacing,
    Opt::ChatDelay,
    Opt::NotificationDisplayTime,
    Opt::ViewBobbing,
    Opt::DistortionEffects,
    Opt::FovEffects,
    Opt::DarknessPulsing,
    Opt::DamageTilt,
    Opt::GlintSpeed,
    Opt::GlintStrength,
    Opt::HideLightningFlashes,
    Opt::MonochromeLogo,
    Opt::PanoramaSpeed,
    Opt::HideSplashTexts,
    Opt::NarratorHotkey,
    Opt::RotateWithMinecarts,
    Opt::HighContrastBlockOutlines,
];

/// `ControlsScreen.options`: the controls added by Java Edition 26.3, in
/// their vanilla order. The macOS-only Control-click switch is omitted on
/// other platforms just as `InputQuirks.EMULATE_RIGHT_CLICK_WITH_CTRL_KEY`
/// does in Java.
pub(crate) fn vanilla_control_options() -> Vec<Opt> {
    let mut options = vec![
        Opt::ToggleCrouch,
        Opt::ToggleSprint,
        Opt::ToggleAttack,
        Opt::ToggleUse,
        Opt::AutoJump,
        Opt::SprintWindow,
        Opt::QuitShortcuts,
    ];
    if cfg!(target_os = "macos") {
        options.push(Opt::CtrlClickEmulatesRightClick);
    }
    options.push(Opt::OperatorItemsTab);
    options
}

/// `MouseSettingsScreen.options` in Java Edition 26.3, in vanilla order.
pub(crate) const MOUSE_OPTIONS: &[Opt] = &[
    Opt::Sensitivity,
    Opt::WheelSensitivity,
    Opt::DiscreteScroll,
    Opt::InvertX,
    Opt::InvertY,
    Opt::AllowCursorChanges,
];

impl Opt {
    /// Keep controls visible while their underlying systems are still missing.
    pub(crate) fn available(self) -> bool {
        !matches!(
            self,
            Opt::ChatDelay
                // Stored, but there are no blocked players, signed chat or
                // chat drafts yet.
                | Opt::HideMatchedNames
                | Opt::OnlySecureChat
                | Opt::SaveChatDrafts
                | Opt::RotateWithMinecarts
                // Saved, but no Horizon system reads them yet: no Realms, no
                // friends list, no directional audio, no now-playing toast.
                | Opt::RealmsNotifications
                | Opt::InGameNotification
                | Opt::SharePresence
                | Opt::DirectionalAudio
                | Opt::MusicToast
                // Saved, but nothing in rendering reads them yet.
                | Opt::ChunkFade
                | Opt::ImprovedTransparency
                | Opt::ChunkBuilder
                // Saved, but the mouse doesn't read it yet.
                | Opt::CtrlClickEmulatesRightClick
        )
    }
    fn file_key(self) -> Option<&'static str> {
        Some(match self {
            Opt::Narrator => "narrator",
            Opt::TextBackground => "backgroundForChatOnly",
            Opt::ChatDelay => "chatDelay",
            Opt::HideMatchedNames => "hideMatchedNames",
            Opt::OnlySecureChat => "onlyShowSecureChat",
            Opt::SaveChatDrafts => "saveChatDrafts",
            Opt::DistortionEffects => "screenEffectScale",
            Opt::FovEffects => "fovEffectScale",
            Opt::DarknessPulsing => "darknessEffectScale",
            Opt::DamageTilt => "damageTiltStrength",
            Opt::GlintSpeed => "glintSpeed",
            Opt::GlintStrength => "glintStrength",
            Opt::HideLightningFlashes => "hideLightningFlashes",
            Opt::MonochromeLogo => "darkMojangStudiosBackground",
            Opt::PanoramaSpeed => "panoramaScrollSpeed",
            Opt::HideSplashTexts => "hideSplashTexts",
            Opt::NarratorHotkey => "narratorHotkey",
            Opt::RotateWithMinecarts => "rotateWithMinecart",
            Opt::HighContrastBlockOutlines => "highContrastBlockOutline",
            Opt::MusicFrequency => "musicFrequency",
            Opt::SoundDevice => "soundDevice",
            Opt::OperatorItemsTab => "operatorItemsTab",
            Opt::QuitShortcuts => "quitShortcuts",
            Opt::CtrlClickEmulatesRightClick => "ctrlClickEmulatesRightClick",
            Opt::ModelPart(i) => MODEL_PART_KEYS[i],
            Opt::AllowServerListing => "allowServerListing",
            Opt::RealmsNotifications => "realmsNotifications",
            Opt::InGameNotification => "inGameNotification",
            Opt::SharePresence => "sharePresence",
            Opt::DirectionalAudio => "directionalAudio",
            Opt::MusicToast => "musicToast",
            _ => return None,
        })
    }

    pub(crate) fn caption(self) -> &'static str {
        match self {
            Opt::Fov => "options.fov",
            Opt::RenderDistance => "options.renderDistance",
            Opt::SimulationDistance => "options.simulationDistance",
            Opt::Sensitivity => "options.sensitivity",
            Opt::WheelSensitivity => "options.mouseWheelSensitivity",
            Opt::InvertX => "options.invertMouseX",
            Opt::InvertY => "options.invertMouseY",
            Opt::DiscreteScroll => "options.discrete_mouse_scroll",
            Opt::AllowCursorChanges => "options.allowCursorChanges",
            Opt::Volume(_) => "soundCategory.master",
            Opt::SoundDevice => "options.audioDevice",
            Opt::GuiScale => "options.guiScale",
            Opt::ViewBobbing => "options.viewBobbing",
            Opt::Gamma => "options.gamma",
            Opt::MaxFps => "options.framerateLimit",
            Opt::Vsync => "options.vsync",
            Opt::Fullscreen => "options.fullscreen",
            Opt::ExclusiveFullscreen => "options.exclusiveFullscreen",
            Opt::ExclusiveMode => "options.fullscreen.exclusive.mode",
            Opt::GraphicsBackend => "options.graphicsApi",
            Opt::TextureFiltering => "options.textureFiltering",
            Opt::MaxAnisotropy => "options.maxAnisotropy",
            Opt::SmoothLighting => "options.ao",
            Opt::Clouds => "options.renderClouds",
            Opt::CloudRange => "options.renderCloudsDistance",
            Opt::Particles => "options.particles",
            Opt::EntityDistance => "options.entityDistanceScaling",
            Opt::EntityShadows => "options.entityShadows",
            Opt::BiomeBlend => "options.biomeBlendRadius",
            Opt::Preset => "options.graphics.preset",
            Opt::Mipmaps => "options.mipmapLevels",
            Opt::WeatherRadius => "options.weatherRadius",
            Opt::ChunkFade => "options.chunkFade",
            Opt::Vignette => "options.vignette",
            Opt::Autosave => "options.autosaveIndicator",
            Opt::CutoutLeaves => "options.cutoutLeaves",
            Opt::ImprovedTransparency => "options.improvedTransparency",
            Opt::AttackIndicator => "options.attackIndicator",
            Opt::InactivityFps => "options.inactivityFpsLimit",
            Opt::ChunkBuilder => "options.prioritizeChunkUpdates",
            Opt::MenuBlur => "options.accessibility.menu_background_blurriness",
            Opt::ToggleCrouch => "key.sneak",
            Opt::ToggleSprint => "key.sprint",
            Opt::ToggleAttack => "key.attack",
            Opt::ToggleUse => "key.use",
            Opt::AutoJump => "options.autoJump",
            Opt::SprintWindow => "options.sprintWindow",
            Opt::QuitShortcuts => "options.quitShortcuts",
            Opt::CtrlClickEmulatesRightClick => "options.ctrlClickEmulatesRightClick",
            Opt::OperatorItemsTab => "options.operatorItemsTab",
            Opt::ChatVisibility => "options.chat.visibility",
            Opt::ChatColors => "options.chat.color",
            Opt::ChatLinks => "options.chat.links",
            Opt::ChatLinksPrompt => "options.chat.links.prompt",
            Opt::ChatOpacity => "options.chat.opacity",
            Opt::TextBackgroundOpacity => "options.accessibility.text_background_opacity",
            Opt::ChatScale => "options.chat.scale",
            Opt::ChatWidth => "options.chat.width",
            Opt::ChatHeightFocused => "options.chat.height.focused",
            Opt::ChatHeightUnfocused => "options.chat.height.unfocused",
            Opt::ChatLineSpacing => "options.chat.line_spacing",
            Opt::AutoSuggestions => "options.autoSuggestCommands",
            Opt::HideMatchedNames => "options.hideMatchedNames",
            Opt::OnlySecureChat => "options.onlyShowSecureChat",
            Opt::SaveChatDrafts => "options.chat.drafts",
            Opt::ReducedDebugInfo => "options.reducedDebugInfo",
            Opt::HighContrast => "options.accessibility.high_contrast",
            Opt::ShowSubtitles => "options.showSubtitles",
            Opt::MusicFrequency => "options.music_frequency",
            Opt::NotificationDisplayTime => "options.notifications.display_time",
            Opt::Narrator => "options.narrator",
            Opt::TextBackground => "options.accessibility.text_background",
            Opt::ChatDelay => "horizon.options.chat_delay",
            Opt::DistortionEffects => "options.screenEffectScale",
            Opt::FovEffects => "options.fovEffectScale",
            Opt::DarknessPulsing => "options.darknessEffectScale",
            Opt::DamageTilt => "options.damageTiltStrength",
            Opt::GlintSpeed => "options.glintSpeed",
            Opt::GlintStrength => "options.glintStrength",
            Opt::HideLightningFlashes => "options.hideLightningFlashes",
            Opt::MonochromeLogo => "options.darkMojangStudiosBackgroundColor",
            Opt::PanoramaSpeed => "options.accessibility.panorama_speed",
            Opt::HideSplashTexts => "options.hideSplashTexts",
            Opt::NarratorHotkey => "options.accessibility.narrator_hotkey",
            Opt::RotateWithMinecarts => "options.rotateWithMinecart",
            Opt::HighContrastBlockOutlines => "options.accessibility.high_contrast_block_outline",
            Opt::ForceUnicode => "options.forceUnicodeFont",
            Opt::JapaneseGlyphs => "options.japaneseGlyphVariants",
            // Horizon's own options: vanilla has no keys for them, so they read
            // the namespace the client bundles (assets/horizon/lang).
            Opt::PadEnabled => "horizon.options.controller",
            Opt::PadVibration => "horizon.options.vibration",
            Opt::PadAdaptiveTriggers => "horizon.options.adaptive_triggers",
            Opt::PadLightbar => "horizon.options.lightbar",
            Opt::PadLookSensitivity => "horizon.options.look_sensitivity",
            Opt::PadInvertX => "horizon.options.invert_x",
            Opt::PadInvertY => "horizon.options.invert_y",
            Opt::PadToggleSneak => "horizon.options.sneak_button",
            Opt::PadMoveDeadzone => "horizon.options.move_deadzone",
            Opt::PadLookDeadzone => "horizon.options.look_deadzone",
            Opt::PadGlyphs => "horizon.options.button_glyphs",
            Opt::PadMenuNav => "horizon.options.menu_navigation",
            Opt::PadKeyboard => "horizon.options.on_screen_keyboard",
            Opt::ModelPart(i) => [
                "options.modelPart.cape",
                "options.modelPart.jacket",
                "options.modelPart.left_sleeve",
                "options.modelPart.right_sleeve",
                "options.modelPart.left_pants_leg",
                "options.modelPart.right_pants_leg",
                "options.modelPart.hat",
            ][i],
            Opt::MainHand => "options.mainHand",
            Opt::AllowServerListing => "options.allowServerListing",
            Opt::RealmsNotifications => "options.realmsNotifications.button",
            Opt::InGameNotification => "options.inGameNotification",
            Opt::SharePresence => "options.sharePresence",
            Opt::DirectionalAudio => "options.directionalAudio",
            Opt::MusicToast => "options.musicToast",
        }
    }
    pub(crate) fn kind(self, s: &Settings) -> Kind {
        let unit = |v: f32, lo: f32, hi: f32| Kind::Slider(((v - lo) / (hi - lo)).clamp(0.0, 1.0));
        match self {
            Opt::FovEffects | Opt::DarknessPulsing => {
                Kind::Slider(s.file.double(self.file_key().unwrap()).sqrt() as f32)
            }
            Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::GlintSpeed
            | Opt::GlintStrength
            | Opt::PanoramaSpeed => Kind::Slider(s.file.double(self.file_key().unwrap()) as f32),
            Opt::Fov => unit(s.fov, 30.0, 110.0),
            Opt::RenderDistance => unit(
                s.render_distance as f32,
                2.0,
                crate::options_file::MAX_RENDER_DISTANCE as f32,
            ),
            Opt::SimulationDistance => unit(s.simulation_distance as f32, 5.0, 32.0),
            Opt::Sensitivity => Kind::Slider(s.sensitivity),
            Opt::WheelSensitivity => unit(s.mouse_wheel_sensitivity, 0.01, 10.0),
            Opt::Volume(i) => Kind::Slider(if i == 0 {
                s.master_volume
            } else {
                s.volumes[i]
            }),
            Opt::Gamma => Kind::Slider(s.gamma),
            Opt::MaxFps => unit(s.max_fps as f32, 10.0, 260.0),
            Opt::CloudRange => unit(s.cloud_range as f32, 2.0, 128.0),
            Opt::EntityDistance => unit(s.entity_distance, 0.5, 5.0),
            Opt::BiomeBlend => unit(s.biome_blend as f32, 0.0, 7.0),
            Opt::Preset => unit(preset_index(&s.graphics_preset) as f32, 0.0, 3.0),
            Opt::Mipmaps => unit(s.mipmap_levels as f32, 0.0, 4.0),
            Opt::MaxAnisotropy => unit(s.file.int("maxAnisotropyBit") as f32, 1.0, 3.0),
            Opt::ExclusiveMode => {
                let last = s.video_modes.len() as f32 - 1.0;
                if last < 0.0 {
                    Kind::Slider(0.0)
                } else {
                    unit(s.video_mode_index() as f32, -1.0, last)
                }
            }
            Opt::WeatherRadius => unit(s.weather_radius as f32, 3.0, 10.0),
            Opt::ChunkFade => unit(s.chunk_fade, 0.0, 2.0),
            Opt::MenuBlur => unit(s.menu_blur as f32, 0.0, 10.0),
            Opt::NotificationDisplayTime => unit(s.notification_display_time, 0.5, 10.0),
            Opt::ChatOpacity => Kind::Slider(s.chat_opacity),
            Opt::TextBackgroundOpacity => Kind::Slider(s.text_background_opacity),
            Opt::ChatScale => Kind::Slider(s.chat_scale),
            Opt::ChatWidth => Kind::Slider(s.chat_width),
            Opt::ChatHeightFocused => Kind::Slider(s.chat_height_focused),
            Opt::ChatHeightUnfocused => Kind::Slider(s.chat_height_unfocused),
            Opt::ChatLineSpacing => Kind::Slider(s.chat_line_spacing),
            Opt::PadLookSensitivity => Kind::Slider(s.controller.look_sensitivity),
            Opt::PadMoveDeadzone => unit(s.controller.move_deadzone, 0.0, 0.5),
            Opt::PadLookDeadzone => unit(s.controller.look_deadzone, 0.0, 0.5),
            Opt::SprintWindow => unit(s.sprint_window as f32, 0.0, 10.0),
            _ => Kind::Cycle,
        }
    }
    /// OptionInstance's static or value-dependent tooltip translation.
    fn tooltip_key(self, s: &Settings) -> String {
        let key = match self {
            Opt::InactivityFps => {
                if s.inactivity_fps_limit == "minimized" {
                    "options.inactivityFpsLimit.minimized"
                } else {
                    "options.inactivityFpsLimit.afk"
                }
            }
            Opt::ChunkBuilder => [
                "options.prioritizeChunkUpdates.none",
                "options.prioritizeChunkUpdates.byPlayer",
                "options.prioritizeChunkUpdates.nearby",
            ][usize::from(s.prioritize_chunk_updates.min(2))],
            // Options.exclusiveFullscreen: a tooltip per value.
            Opt::ExclusiveFullscreen => {
                if s.file.bool("exclusiveFullscreen") {
                    "options.exclusiveFullscreen.on"
                } else {
                    "options.exclusiveFullscreen.off"
                }
            }
            Opt::TextureFiltering => [
                "options.textureFiltering.none",
                "options.textureFiltering.rgss",
                "options.textureFiltering.anisotropic",
            ][s.file.int("textureFiltering").clamp(0, 2) as usize],
            // InputQuirks.REPLACE_CTRL_KEY_WITH_CMD_KEY: Cmd+B on macOS.
            Opt::NarratorHotkey if cfg!(target_os = "macos") => {
                "options.accessibility.narrator_hotkey.mac"
            }
            _ => self.caption(),
        };
        format!("{key}.tooltip")
    }
    /// A slider's position (0..1), or `None` for a cycle button.
    pub fn slider_position(self, s: &Settings) -> Option<f32> {
        match self.kind(s) {
            Kind::Slider(v) => Some(v),
            Kind::Cycle => None,
        }
    }
    /// The discrete step count for stepped sliders, or `None` if continuous.
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            Opt::Fov => Some(80),
            Opt::RenderDistance => Some((crate::options_file::MAX_RENDER_DISTANCE - 2) as u32),
            Opt::SimulationDistance => Some(27),
            Opt::MaxFps => Some(25),
            Opt::CloudRange => Some(126),
            Opt::EntityDistance => Some(18),
            Opt::BiomeBlend => Some(7),
            Opt::Preset => Some(3),
            Opt::Mipmaps => Some(4),
            Opt::WeatherRadius => Some(7),
            Opt::ChunkFade => Some(40),
            Opt::MenuBlur => Some(10),
            Opt::NotificationDisplayTime => Some(95),
            Opt::PadMoveDeadzone | Opt::PadLookDeadzone => Some(50),
            Opt::SprintWindow => Some(10),
            _ => None,
        }
    }
    /// Step a slider forward or backward, derived from its own slider definition and step count.
    pub fn step_slider(self, s: &mut Settings, forward: bool, width: f32) {
        if !self.available() {
            return;
        }
        let Some(pos) = self.slider_position(s) else {
            return;
        };
        let next_pos = if let Some(steps) = self.slider_steps() {
            let steps = steps as f32;
            let current = (pos * steps).round();
            let next = if forward {
                current + 1.0
            } else {
                current - 1.0
            };
            (next / steps).clamp(0.0, 1.0)
        } else {
            let delta = 1.0 / (width - 8.0).max(1.0);
            if forward {
                (pos + delta).min(1.0)
            } else {
                (pos - delta).max(0.0)
            }
        };
        self.set_slider(s, next_pos);
    }
    /// The widget message (OptionInstance.toString / CycleButton full name).
    pub fn label(self, s: &Settings) -> Text {
        let c = self.caption();
        match self {
            Opt::ChatDelay => tr("options.chat.delay_none"),
            Opt::FovEffects
            | Opt::DarknessPulsing
            | Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::GlintSpeed
            | Opt::GlintStrength => {
                let value = s.file.double(self.file_key().unwrap()) as f32;
                if value == 0.0 {
                    generic(c, tr("options.off"))
                } else {
                    percent(c, value)
                }
            }
            Opt::PanoramaSpeed => percent(c, s.file.double("panoramaScrollSpeed") as f32),
            Opt::SprintWindow => {
                let value = if s.sprint_window == 0 {
                    tr("options.off")
                } else {
                    Text::tr("options.value", &[&s.sprint_window])
                };
                generic(c, value)
            }
            Opt::Fov => match s.fov as i32 {
                70 => generic(c, tr("options.fov.min")),
                110 => generic(c, tr("options.fov.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::RenderDistance => generic(c, Text::tr("options.chunks", &[&s.render_distance])),
            Opt::SimulationDistance => {
                generic(c, Text::tr("options.chunks", &[&s.simulation_distance]))
            }
            Opt::Sensitivity => {
                if s.sensitivity == 0.0 {
                    generic(c, tr("options.sensitivity.min"))
                } else if s.sensitivity == 1.0 {
                    generic(c, tr("options.sensitivity.max"))
                } else {
                    percent(c, 2.0 * s.sensitivity)
                }
            }
            Opt::WheelSensitivity => generic(
                c,
                Text::literal(format!("{:.2}", s.mouse_wheel_sensitivity)),
            ),
            Opt::Volume(i) => {
                let v = if i == 0 {
                    s.master_volume
                } else {
                    s.volumes[i]
                };
                let caption = format!("soundCategory.{}", SOUND_SOURCES[i]);
                if v == 0.0 {
                    generic(&caption, tr("options.off"))
                } else {
                    percent(&caption, v)
                }
            }
            Opt::GuiScale => {
                let value = if s.gui_scale == 0 {
                    tr("options.guiScale.auto")
                } else {
                    Text::literal(s.gui_scale.to_string())
                };
                generic(c, value)
            }
            Opt::Gamma => match (f64::from(s.gamma) * 100.0) as i32 {
                0 => generic(c, tr("options.gamma.min")),
                50 => generic(c, tr("options.gamma.default")),
                100 => generic(c, tr("options.gamma.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::MaxFps => {
                if s.max_fps >= 260 {
                    generic(c, tr("options.framerateLimit.max"))
                } else {
                    generic(c, Text::tr("options.framerate", &[&s.max_fps]))
                }
            }
            Opt::CloudRange => generic(c, Text::tr("options.chunks", &[&s.cloud_range])),
            Opt::EntityDistance => percent(c, s.entity_distance),
            Opt::BiomeBlend => generic(
                c,
                tr(&format!(
                    "options.biomeBlendRadius.{}",
                    s.biome_blend * 2 + 1
                )),
            ),
            Opt::Preset => generic(c, tr(&format!("options.graphics.{}", s.graphics_preset))),
            Opt::MaxAnisotropy => match s.file.int("maxAnisotropyBit") {
                0 => generic(c, tr("options.off")),
                bit => generic(c, Text::tr("options.multiplier", &[&(1 << bit)])),
            },
            Opt::ExclusiveMode => match s.video_modes.get(s.video_mode_index().max(0) as usize) {
                None if s.video_modes.is_empty() => tr("options.fullscreen.unavailable"),
                Some(mode) if s.video_mode_index() >= 0 => generic(
                    c,
                    Text::tr(
                        "options.fullscreen.entry",
                        &[
                            &mode.width,
                            &mode.height,
                            &mode.refresh_rate_label(),
                            &mode.bits(),
                        ],
                    ),
                ),
                _ => generic(c, tr("options.fullscreen.current")),
            },
            Opt::Mipmaps => {
                if s.mipmap_levels == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.mipmap_levels.to_string()))
                }
            }
            Opt::WeatherRadius => generic(c, Text::tr("options.blocks", &[&s.weather_radius])),
            Opt::ChunkFade => {
                if s.chunk_fade <= 0.0 {
                    tr("options.chunkFade.none")
                } else {
                    Text::tr(
                        "options.chunkFade.seconds",
                        &[&format!("{:.2}", s.chunk_fade)],
                    )
                }
            }
            Opt::MenuBlur => {
                if s.menu_blur == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.menu_blur.to_string()))
                }
            }
            Opt::ChatOpacity => percent(c, s.chat_opacity * 0.9 + 0.1),
            Opt::TextBackgroundOpacity => percent(c, s.text_background_opacity),
            Opt::ChatScale => {
                if s.chat_scale == 0.0 {
                    generic(c, tr("options.off"))
                } else {
                    percent(c, s.chat_scale)
                }
            }
            Opt::ChatWidth => chat_px(c, (s.chat_width * 280.0 + 40.0).floor()),
            Opt::ChatHeightFocused => chat_px(c, (s.chat_height_focused * 160.0 + 20.0).floor()),
            Opt::ChatHeightUnfocused => {
                chat_px(c, (s.chat_height_unfocused * 160.0 + 20.0).floor())
            }
            Opt::ChatLineSpacing => percent(c, s.chat_line_spacing),
            Opt::NotificationDisplayTime => generic(
                c,
                Text::tr(
                    "options.multiplier",
                    &[&format!("{:.1}", s.notification_display_time)],
                ),
            ),
            Opt::PadLookSensitivity => percent(c, 2.0 * s.controller.look_sensitivity),
            Opt::PadMoveDeadzone | Opt::PadLookDeadzone => {
                let v = if self == Opt::PadMoveDeadzone {
                    s.controller.move_deadzone
                } else {
                    s.controller.look_deadzone
                };
                Text::translate(
                    "options.percent_value",
                    &[
                        tr(c),
                        Text::literal(((v * 100.0).round() as i32).to_string()),
                    ],
                )
            }
            _ => generic(c, self.value_text(s)),
        }
    }
    fn value_text(self, s: &Settings) -> Text {
        match self {
            Opt::TextBackground => tr(if s.file.bool("backgroundForChatOnly") {
                "options.accessibility.text_background.chat"
            } else {
                "options.accessibility.text_background.everywhere"
            }),
            // Options.narrator's caption: the status while the narrator is
            // there, "Not Available" otherwise.
            Opt::Narrator => tr(if horizon_narrator::is_active() {
                s.narrator().key()
            } else {
                "options.narrator.notavailable"
            }),
            Opt::ChatDelay => tr("options.chat.delay_none"),
            Opt::HideLightningFlashes => on_off(s.file.bool("hideLightningFlashes")),
            Opt::MonochromeLogo => on_off(s.file.bool("darkMojangStudiosBackground")),
            Opt::HideSplashTexts => on_off(s.file.bool("hideSplashTexts")),
            Opt::HideMatchedNames
            | Opt::OnlySecureChat
            | Opt::SaveChatDrafts
            | Opt::ModelPart(_)
            | Opt::AllowServerListing
            | Opt::RealmsNotifications
            | Opt::InGameNotification
            | Opt::DirectionalAudio => on_off(s.file.bool(self.file_key().unwrap())),
            Opt::MainHand => tr(if s.main_hand_left {
                "options.mainHand.left"
            } else {
                "options.mainHand.right"
            }),
            Opt::SharePresence => tr([
                "options.sharePresence.none",
                "options.sharePresence.limited",
                "options.sharePresence.all",
            ][s.file.int("sharePresence").clamp(0, 2) as usize]),
            Opt::MusicToast => tr([
                "options.musicToast.never",
                "options.musicToast.pauseMenu",
                "options.musicToast.pauseMenuAndToast",
            ][s.file.int("musicToast").clamp(0, 2) as usize]),
            Opt::NarratorHotkey => on_off(s.file.bool("narratorHotkey")),
            Opt::RotateWithMinecarts => on_off(s.file.bool("rotateWithMinecart")),
            Opt::HighContrastBlockOutlines => on_off(s.file.bool("highContrastBlockOutline")),
            Opt::InvertX => on_off(s.invert_x),
            Opt::InvertY => on_off(s.invert_y),
            Opt::DiscreteScroll => on_off(s.discrete_scroll),
            Opt::AllowCursorChanges => on_off(s.allow_cursor_changes),
            Opt::ViewBobbing => on_off(s.view_bobbing),
            Opt::Vsync => on_off(s.vsync),
            Opt::Fullscreen => on_off(s.fullscreen),
            Opt::ExclusiveFullscreen => on_off(s.file.bool("exclusiveFullscreen")),
            Opt::TextureFiltering => tr([
                "options.textureFiltering.none",
                "options.textureFiltering.rgss",
                "options.textureFiltering.anisotropic",
            ][s.file.int("textureFiltering").clamp(0, 2) as usize]),
            Opt::GraphicsBackend => tr(match s.horizon.api {
                crate::graphics::GraphicsApi::OpenGl => "options.graphicsApi.opengl",
                crate::graphics::GraphicsApi::Vulkan => "options.graphicsApi.vulkan",
                _ => "options.graphicsApi.default",
            }),
            Opt::SmoothLighting => on_off(s.smooth_lighting),
            Opt::Clouds => tr(match s.clouds.as_str() {
                "false" => "options.off",
                "fast" => "options.clouds.fast",
                _ => "options.clouds.fancy",
            }),
            Opt::Particles => tr([
                "options.particles.all",
                "options.particles.decreased",
                "options.particles.minimal",
            ][usize::from(s.particles.min(2))]),
            Opt::EntityShadows => on_off(s.entity_shadows),
            Opt::Vignette => on_off(s.vignette),
            Opt::Autosave => on_off(s.autosave_indicator),
            Opt::CutoutLeaves => on_off(s.cutout_leaves),
            Opt::ImprovedTransparency => on_off(s.improved_transparency),
            Opt::AttackIndicator => tr([
                "options.off",
                "options.attack.crosshair",
                "options.attack.hotbar",
            ][usize::from(s.attack_indicator.min(2))]),
            Opt::InactivityFps => tr(if s.inactivity_fps_limit == "minimized" {
                "options.inactivityFpsLimit.minimized"
            } else {
                "options.inactivityFpsLimit.afk"
            }),
            Opt::ChunkBuilder => tr([
                "options.prioritizeChunkUpdates.none",
                "options.prioritizeChunkUpdates.byPlayer",
                "options.prioritizeChunkUpdates.nearby",
            ][usize::from(s.prioritize_chunk_updates.min(2))]),
            Opt::ToggleCrouch => tr(if s.toggle_crouch {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleSprint => tr(if s.toggle_sprint {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleAttack => tr(if s.toggle_attack {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleUse => tr(if s.toggle_use {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::AutoJump => on_off(s.auto_jump),
            Opt::QuitShortcuts => on_off(s.quit_shortcuts),
            Opt::CtrlClickEmulatesRightClick => on_off(s.ctrl_click_emulates_right_click),
            Opt::OperatorItemsTab => on_off(s.operator_items_tab),
            Opt::ChatVisibility => tr([
                "options.chat.visibility.full",
                "options.chat.visibility.system",
                "options.chat.visibility.hidden",
            ][usize::from(s.chat_visibility.min(2))]),
            Opt::ChatColors => on_off(s.chat_colors),
            Opt::ChatLinks => on_off(s.chat_links),
            Opt::ChatLinksPrompt => on_off(s.chat_links_prompt),
            Opt::AutoSuggestions => on_off(s.auto_suggestions),
            Opt::ReducedDebugInfo => on_off(s.reduced_debug_info),
            Opt::HighContrast => on_off(s.high_contrast),
            Opt::ShowSubtitles => on_off(s.show_subtitles),
            Opt::SoundDevice => {
                if s.sound_device.is_empty() {
                    tr("options.audioDevice.default")
                } else {
                    Text::literal(&s.sound_device)
                }
            }
            Opt::MusicFrequency => tr([
                "options.music_frequency.default",
                "options.music_frequency.frequent",
                "options.music_frequency.constant",
            ][s.file.int("musicFrequency").clamp(0, 2) as usize]),
            Opt::ForceUnicode => on_off(s.force_unicode),
            Opt::JapaneseGlyphs => on_off(s.japanese_glyphs),
            Opt::PadEnabled => on_off(s.controller.enabled),
            Opt::PadVibration => on_off(s.controller.vibration),
            Opt::PadAdaptiveTriggers => on_off(s.controller.adaptive_triggers),
            Opt::PadInvertX => on_off(s.controller.invert_x),
            Opt::PadInvertY => on_off(s.controller.invert_y),
            Opt::PadToggleSneak => tr(if s.controller.toggle_sneak {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::PadLightbar => tr(match s.controller.lightbar {
                gamepad::Lightbar::Health => "horizon.options.lightbar.health",
                gamepad::Lightbar::Dimension => "horizon.options.lightbar.dimension",
                gamepad::Lightbar::Off => "options.off",
            }),
            Opt::PadGlyphs => tr(match s.controller.glyphs {
                GlyphStyle::Auto => "horizon.options.glyphs.auto",
                GlyphStyle::Xbox => "horizon.options.glyphs.xbox",
                GlyphStyle::PlayStation => "horizon.options.glyphs.playstation",
                GlyphStyle::Nintendo => "horizon.options.glyphs.nintendo",
            }),
            Opt::PadMenuNav => tr(match s.controller.menu_nav {
                gamepad::MenuNav::Focus => "horizon.options.menu_focus",
                gamepad::MenuNav::Pointer => "horizon.options.menu_pointer",
            }),
            Opt::PadKeyboard => tr(match s.controller.keyboard {
                gamepad::TextKeyboard::Auto => "horizon.options.keyboard.auto",
                gamepad::TextKeyboard::Horizon => "horizon.options.keyboard.horizon",
                gamepad::TextKeyboard::Steam => "horizon.options.keyboard.steam",
            }),
            _ => Text::empty(),
        }
    }
    /// Set a slider from its 0..1 position (OptionInstance value sets).
    pub fn set_slider(self, s: &mut Settings, v: f32) {
        if !self.available() {
            return;
        }
        let v = v.clamp(0.0, 1.0);
        let step = |lo: f32, hi: f32| (lo + v * (hi - lo)).round();
        match self {
            Opt::FovEffects | Opt::DarknessPulsing => s.file.set(
                self.file_key().unwrap(),
                crate::options_file::V::Double(f64::from(v).powi(2)),
            ),
            Opt::DistortionEffects
            | Opt::DamageTilt
            | Opt::PanoramaSpeed
            | Opt::GlintSpeed
            | Opt::GlintStrength => s.file.set(
                self.file_key().unwrap(),
                crate::options_file::V::Double(f64::from(v)),
            ),
            Opt::Fov => s.fov = step(30.0, 110.0),
            Opt::RenderDistance => {
                s.render_distance =
                    step(2.0, crate::options_file::MAX_RENDER_DISTANCE as f32) as i32
            }
            Opt::SimulationDistance => s.simulation_distance = step(5.0, 32.0) as i32,
            Opt::Sensitivity => s.sensitivity = v,
            Opt::WheelSensitivity => s.mouse_wheel_sensitivity = 0.01 + v * 9.99,
            Opt::Volume(0) => s.master_volume = v,
            Opt::Volume(i) => s.volumes[i] = v,
            Opt::Gamma => s.gamma = v,
            Opt::MaxFps => s.max_fps = (step(1.0, 26.0) as u32) * 10,
            Opt::CloudRange => s.cloud_range = step(2.0, 128.0) as i32,
            Opt::EntityDistance => s.entity_distance = step(2.0, 20.0) / 4.0,
            Opt::BiomeBlend => s.biome_blend = step(0.0, 7.0) as i32,
            Opt::Preset => s.graphics_preset = PRESETS[step(0.0, 3.0) as usize].into(),
            Opt::Mipmaps => s.mipmap_levels = step(0.0, 4.0) as i32,
            Opt::MaxAnisotropy => s.file.set(
                "maxAnisotropyBit",
                crate::options_file::V::Int(step(1.0, 3.0) as i32),
            ),
            Opt::ExclusiveMode => {
                // Window.setPreferredFullscreenVideoMode: -1 is the desktop's own mode.
                let Some(last) = s.video_modes.len().checked_sub(1) else {
                    return;
                };
                let index = step(-1.0, last as f32) as i32;
                s.file.fullscreen_resolution = usize::try_from(index)
                    .ok()
                    .map(|i| s.video_modes[i].write());
            }
            Opt::WeatherRadius => s.weather_radius = step(3.0, 10.0) as i32,
            Opt::ChunkFade => s.chunk_fade = step(0.0, 40.0) / 20.0,
            Opt::MenuBlur => s.menu_blur = step(0.0, 10.0) as i32,
            Opt::NotificationDisplayTime => s.notification_display_time = step(5.0, 100.0) / 10.0,
            Opt::ChatOpacity => s.chat_opacity = v,
            Opt::TextBackgroundOpacity => s.text_background_opacity = v,
            Opt::ChatScale => s.chat_scale = v,
            Opt::ChatWidth => s.chat_width = v,
            Opt::ChatHeightFocused => s.chat_height_focused = v,
            Opt::ChatHeightUnfocused => s.chat_height_unfocused = v,
            Opt::ChatLineSpacing => s.chat_line_spacing = v,
            Opt::PadLookSensitivity => s.controller.look_sensitivity = v,
            Opt::PadMoveDeadzone => s.controller.move_deadzone = step(0.0, 50.0) / 100.0,
            Opt::PadLookDeadzone => s.controller.look_deadzone = step(0.0, 50.0) / 100.0,
            Opt::SprintWindow => s.sprint_window = step(0.0, 10.0) as i32,
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
    fn affects_preset(self) -> bool {
        matches!(
            self,
            Opt::RenderDistance
                | Opt::SimulationDistance
                | Opt::SmoothLighting
                | Opt::Clouds
                | Opt::CloudRange
                | Opt::Particles
                | Opt::EntityDistance
                | Opt::EntityShadows
                | Opt::BiomeBlend
                | Opt::Mipmaps
                | Opt::WeatherRadius
                | Opt::CutoutLeaves
                | Opt::ImprovedTransparency
                | Opt::MenuBlur
                | Opt::ChunkBuilder
                | Opt::TextureFiltering
                | Opt::MaxAnisotropy
        )
    }
    /// Advance a cycle button (CycleButton.cycleValue).
    pub fn cycle(self, s: &mut Settings, back: bool, framebuffer: [i32; 2]) {
        if !self.available() {
            return;
        }
        let next = |n: u8, count: u8| {
            if back {
                (n + count - 1) % count
            } else {
                (n + 1) % count
            }
        };
        match self {
            Opt::GuiScale => {
                let max = crate::window::calculate_scale(framebuffer, 0, s.force_unicode) as u32;
                let value = s.gui_scale.min(max);
                s.gui_scale = if back {
                    if value == 0 { max } else { value - 1 }
                } else if value == max {
                    0
                } else {
                    value + 1
                };
            }
            Opt::TextBackground
            | Opt::HideLightningFlashes
            | Opt::HideSplashTexts
            | Opt::HighContrastBlockOutlines
            | Opt::MonochromeLogo
            | Opt::HideMatchedNames
            | Opt::OnlySecureChat
            | Opt::SaveChatDrafts
            | Opt::ModelPart(_)
            | Opt::AllowServerListing
            | Opt::RealmsNotifications
            | Opt::InGameNotification
            | Opt::DirectionalAudio
            | Opt::NarratorHotkey => {
                let key = self.file_key().unwrap();
                s.file
                    .set(key, crate::options_file::V::Bool(!s.file.bool(key)));
            }
            // OptionInstance.Enum over NarratorStatus.values().
            Opt::Narrator => {
                let n = next(s.narrator().id() as u8, 4);
                s.set_narrator(horizon_narrator::Status::by_id(i32::from(n)));
            }
            Opt::InvertX => s.invert_x = !s.invert_x,
            Opt::InvertY => s.invert_y = !s.invert_y,
            Opt::DiscreteScroll => s.discrete_scroll = !s.discrete_scroll,
            Opt::AllowCursorChanges => s.allow_cursor_changes = !s.allow_cursor_changes,
            Opt::ViewBobbing => s.view_bobbing = !s.view_bobbing,
            Opt::Vsync => s.vsync = !s.vsync,
            Opt::Fullscreen => s.fullscreen = !s.fullscreen,
            Opt::ExclusiveFullscreen => {
                let on = s.file.bool("exclusiveFullscreen");
                s.file
                    .set("exclusiveFullscreen", crate::options_file::V::Bool(!on));
            }
            Opt::TextureFiltering => {
                let n = next(s.file.int("textureFiltering").clamp(0, 2) as u8, 3);
                s.file.set(
                    "textureFiltering",
                    crate::options_file::V::Int(i32::from(n)),
                );
            }
            Opt::GraphicsBackend => {
                // PreferredGraphicsApi: Default, OpenGL, Vulkan; a platform lacking one skips it.
                use crate::graphics::GraphicsApi::{Auto, OpenGl, Vulkan};
                let offered: Vec<_> = [Auto, OpenGl, Vulkan]
                    .into_iter()
                    .filter(|a| crate::graphics::GraphicsApi::choices().contains(a))
                    .collect();
                let at = offered
                    .iter()
                    .position(|a| *a == s.horizon.api)
                    .unwrap_or(0);
                let count = offered.len();
                let api = offered[if back {
                    (at + count - 1) % count
                } else {
                    (at + 1) % count
                }];
                s.horizon.api = api;
                s.file.set_name(
                    "preferredGraphicsBackend",
                    match api {
                        OpenGl => "opengl",
                        Vulkan => "vulkan",
                        _ => "default",
                    },
                );
            }
            Opt::SmoothLighting => s.smooth_lighting = !s.smooth_lighting,
            Opt::Clouds => {
                let order = ["false", "fast", "true"];
                let i = order.iter().position(|c| *c == s.clouds).unwrap_or(2) as u8;
                s.clouds = order[usize::from(next(i, 3))].into();
            }
            Opt::Particles => s.particles = next(s.particles, 3),
            Opt::EntityShadows => s.entity_shadows = !s.entity_shadows,
            Opt::Vignette => s.vignette = !s.vignette,
            Opt::Autosave => s.autosave_indicator = !s.autosave_indicator,
            Opt::CutoutLeaves => s.cutout_leaves = !s.cutout_leaves,
            Opt::ImprovedTransparency => s.improved_transparency = !s.improved_transparency,
            Opt::AttackIndicator => s.attack_indicator = next(s.attack_indicator, 3),
            Opt::InactivityFps => {
                s.inactivity_fps_limit = if s.inactivity_fps_limit == "afk" {
                    "minimized".into()
                } else {
                    "afk".into()
                }
            }
            Opt::ChunkBuilder => s.prioritize_chunk_updates = next(s.prioritize_chunk_updates, 3),
            Opt::ToggleCrouch => s.toggle_crouch = !s.toggle_crouch,
            Opt::ToggleSprint => s.toggle_sprint = !s.toggle_sprint,
            Opt::ToggleAttack => s.toggle_attack = !s.toggle_attack,
            Opt::ToggleUse => s.toggle_use = !s.toggle_use,
            Opt::AutoJump => s.auto_jump = !s.auto_jump,
            Opt::QuitShortcuts => s.quit_shortcuts = !s.quit_shortcuts,
            Opt::CtrlClickEmulatesRightClick => {
                s.ctrl_click_emulates_right_click = !s.ctrl_click_emulates_right_click
            }
            Opt::OperatorItemsTab => s.operator_items_tab = !s.operator_items_tab,
            Opt::ChatVisibility => s.chat_visibility = next(s.chat_visibility, 3),
            Opt::ChatColors => s.chat_colors = !s.chat_colors,
            Opt::ChatLinks => s.chat_links = !s.chat_links,
            Opt::ChatLinksPrompt => s.chat_links_prompt = !s.chat_links_prompt,
            Opt::AutoSuggestions => s.auto_suggestions = !s.auto_suggestions,
            Opt::ReducedDebugInfo => s.reduced_debug_info = !s.reduced_debug_info,
            Opt::HighContrast => s.high_contrast = !s.high_contrast,
            Opt::ShowSubtitles => s.show_subtitles = !s.show_subtitles,
            Opt::SoundDevice => {
                let devices = crate::audio::output_devices();
                let mut all = vec![String::new()];
                for d in devices {
                    if !all.contains(&d) {
                        all.push(d);
                    }
                }
                let current_idx = all.iter().position(|d| d == &s.sound_device).unwrap_or(0);
                let count = all.len();
                let next_idx = if back {
                    (current_idx + count - 1) % count
                } else {
                    (current_idx + 1) % count
                };
                s.sound_device = all[next_idx].clone();
            }
            Opt::MusicFrequency => {
                let n = next(s.file.int("musicFrequency").clamp(0, 2) as u8, 3);
                s.file
                    .set("musicFrequency", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::MainHand => s.main_hand_left = !s.main_hand_left,
            Opt::SharePresence => {
                let n = next(s.file.int("sharePresence").clamp(0, 2) as u8, 3);
                s.file
                    .set("sharePresence", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::MusicToast => {
                let n = next(s.file.int("musicToast").clamp(0, 2) as u8, 3);
                s.file
                    .set("musicToast", crate::options_file::V::Int(i32::from(n)));
            }
            Opt::ForceUnicode => s.force_unicode = !s.force_unicode,
            Opt::JapaneseGlyphs => s.japanese_glyphs = !s.japanese_glyphs,
            Opt::PadEnabled => s.controller.enabled = !s.controller.enabled,
            Opt::PadVibration => s.controller.vibration = !s.controller.vibration,
            Opt::PadAdaptiveTriggers => {
                s.controller.adaptive_triggers = !s.controller.adaptive_triggers
            }
            Opt::PadInvertX => s.controller.invert_x = !s.controller.invert_x,
            Opt::PadInvertY => s.controller.invert_y = !s.controller.invert_y,
            Opt::PadToggleSneak => s.controller.toggle_sneak = !s.controller.toggle_sneak,
            Opt::PadLightbar => {
                let all = gamepad::Lightbar::ALL;
                let i = all
                    .iter()
                    .position(|l| *l == s.controller.lightbar)
                    .unwrap_or(0) as u8;
                s.controller.lightbar = all[usize::from(next(i, 3))];
            }
            Opt::PadGlyphs => {
                let i = GlyphStyle::ALL
                    .iter()
                    .position(|g| *g == s.controller.glyphs)
                    .unwrap_or(0) as u8;
                s.controller.glyphs = GlyphStyle::ALL[usize::from(next(i, 4))];
            }
            Opt::PadMenuNav => {
                s.controller.menu_nav = match s.controller.menu_nav {
                    gamepad::MenuNav::Focus => gamepad::MenuNav::Pointer,
                    gamepad::MenuNav::Pointer => gamepad::MenuNav::Focus,
                }
            }
            Opt::PadKeyboard => {
                let all = gamepad::TextKeyboard::ALL;
                let i = all
                    .iter()
                    .position(|k| *k == s.controller.keyboard)
                    .unwrap_or(0) as u8;
                s.controller.keyboard = all[usize::from(next(i, 3))];
            }
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
}
const PRESETS: [&str; 4] = ["fast", "fancy", "fabulous", "custom"];
fn preset_index(p: &str) -> usize {
    PRESETS.iter().position(|x| *x == p).unwrap_or(1)
}
/// A slider moved to `v` (OptionInstance.set): the graphics preset applies
/// its options when its value changes (`Options.applyGraphicsPreset`).
pub fn set_slider(opt: Opt, s: &mut Settings, v: f32) {
    let preset = preset_index(&s.graphics_preset);
    opt.set_slider(s, v);
    if opt == Opt::Preset && preset_index(&s.graphics_preset) != preset {
        apply_preset(s);
    }
}
/// A slider pressed or dragged to `v`: apply it live, and tell the game when
/// a category volume changed (`Options.createSoundSliderOptionInstance`'s
/// callback, which `OptionInstance.set` runs only for a new value).
fn drag_slider(opt: Opt, s: &mut Settings, v: f32, events: &mut Vec<Event>) {
    let volume = |s: &Settings| match opt {
        Opt::Volume(0) => Some(s.master_volume),
        Opt::Volume(i) => Some(s.volumes[i]),
        _ => None,
    };
    let before = volume(s);
    set_slider(opt, s, v);
    events.push(Event::SliderDrag);
    if let (Opt::Volume(source), Some(after)) = (opt, volume(s))
        && before != Some(after)
    {
        events.push(Event::Volume(source, after));
    }
}
/// GraphicsPreset.apply: the options Fast, Fancy and Fabulous set.
pub fn apply_preset(s: &mut Settings) {
    let fabulous = s.graphics_preset == "fabulous";
    match s.graphics_preset.as_str() {
        "fast" => {
            s.biome_blend = 1;
            s.render_distance = 8;
            s.prioritize_chunk_updates = 0;
            s.simulation_distance = 6;
            s.smooth_lighting = false;
            s.clouds = "fast".into();
            s.particles = 1;
            s.mipmap_levels = 2;
            s.entity_shadows = false;
            s.entity_distance = 0.75;
            s.menu_blur = 2;
            s.cloud_range = 32;
            s.cutout_leaves = false;
            s.improved_transparency = false;
            s.weather_radius = 5;
            set_filtering(s, 1, 0);
        }
        "fancy" | "fabulous" => {
            s.biome_blend = 2;
            s.render_distance = if fabulous { 32 } else { 16 };
            s.prioritize_chunk_updates = 1;
            s.simulation_distance = 12;
            s.smooth_lighting = true;
            s.clouds = "true".into();
            s.particles = 0;
            s.mipmap_levels = 4;
            s.entity_shadows = true;
            s.entity_distance = if fabulous { 1.25 } else { 1.0 };
            s.menu_blur = 5;
            s.cloud_range = if fabulous { 128 } else { 64 };
            s.cutout_leaves = true;
            s.improved_transparency = fabulous;
            s.weather_radius = 10;
            // Fabulous takes anisotropic filtering (RGSS on a device with known issues, which
            // wgpu does not report).
            set_filtering(
                s,
                if fabulous { 2 } else { 1 },
                if fabulous { 2 } else { 1 },
            );
        }
        _ => {}
    }
}
/// The preset's `maxAnisotropyBit` and `textureFiltering` (0 none, 1 RGSS, 2 anisotropic).
fn set_filtering(s: &mut Settings, anisotropy_bit: i32, filtering: i32) {
    use crate::options_file::V;
    s.file.set("maxAnisotropyBit", V::Int(anisotropy_bit));
    s.file.set("textureFiltering", V::Int(filtering));
}

/// `Options.preferredGraphicsBackend`'s tooltip: the restart note once the choice differs from
/// the startup one, the description, and Vulkan's warning.
fn graphics_api_tooltip(s: &Settings) -> Text {
    let line = |key: &str| lang::get_or(key, key);
    let mut lines = Vec::new();
    if s.horizon.api != s.startup_api {
        lines.push(line("options.needsRestart"));
        lines.push(String::new());
    }
    lines.push(line("options.graphicsApi.tooltip"));
    if s.horizon.api == crate::graphics::GraphicsApi::Vulkan {
        lines.push(String::new());
        lines.push(line("options.graphicsApi.tooltip.vulkan"));
    }
    Text::literal(lines.join("\n"))
}

/// `selected_key` while a native mod's key waits (`selected_native_key`).
const NATIVE_KEY: &str = "horizon.nativeModKey";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Video,
    Sound,
    Controls,
    KeyBinds,
    Mouse,
    Chat,
    Accessibility,
    Language,
    Font,
    Packs,
    Mods,
    /// A native mod's settings page: its index in `mods.installed`.
    NativeMod(usize),
    /// Horizon's shader packs, either kind (`shader_pack_list`).
    ShaderPacks,
    /// The options of `s.shader_packs[.0]`, a Horizon pack: its `[[pages]]` entry `.1`, the
    /// first the one the button opens.
    NativePackOptions(usize, u8),
    /// The picked `shaders/` pack's options (`shader_options`), one of its screens at a time.
    ShaderOptions,
    /// Horizon's graphics settings (`graphics_ui`).
    Graphics,
    GraphicsAdvanced,
    GraphicsHdr,
    /// HDR calibration, full screen (`calibration`).
    HdrCalibration,
    /// Horizon's controller settings and button binds.
    Controller,
    PadBinds,
    Extras,
    /// CreditsAndAttributionScreen.
    Credits,
    /// Horizon Settings: every page Horizon adds, as a mod's settings
    /// screen hangs off vanilla's Options.
    Horizon,
    /// AI Agent: agent control on or off, whether an agent is connected, the connection details.
    Agent,
    /// SkinCustomizationScreen: the skin layers and Main Hand.
    Skin,
    /// OnlineOptionsScreen: server listing, Realms and friends notifications.
    Online,
    /// Splitscreen: the profile another player joins as (`split_profiles`).
    SplitProfiles,
}
#[derive(Clone, Debug, PartialEq)]
enum Act {
    None,
    Push(Page),
    Done,
    Cycle(Opt),
    Extra(crate::extras::Item),
    Key(&'static str),
    ResetKey(&'static str),
    ResetKeys,
    /// Controller binds: wait for a button, reset one, reset all.
    PadKey(&'static str),
    ResetPadKey(&'static str),
    ResetPadKeys,
    ToggleMod(usize),
    /// Pick `s.shader_packs[i]`, or turn it off when it is the pick.
    PickShaderPack(usize),
    OpenShaderPackFolder,
    /// Open the picked pack's Shader Options page.
    ShaderOptions,
    /// Open the options page of the Horizon pack `s.shader_packs[.0]`.
    NativePackOptions(usize),
    /// Option `.1` of that pack: its next value.
    NativePackOption(usize, usize),
    /// Every option of that pack back to its default.
    NativePackReset(usize),
    /// Shader Options: the next value of an option, a screen of the pack, the next profile,
    /// every option back to its default.
    ShaderOption(String),
    ShaderScreen(String),
    ShaderProfile,
    ShaderReset,
    /// The installed native mod's settings page.
    ModSettings(usize),
    /// A native mod's key (mod, key) waits for a key; a button of its page.
    NativeKey(usize, usize),
    NativeButton(usize, usize),
    OpenModFolder,
    /// Credits & Attribution's Credits button (WinScreen without the poem).
    Credits,
    CopyAgentDetails,
}
enum WKind {
    Button {
        label: Text,
        active: bool,
        act: Act,
    },
    Slider {
        label: Text,
        value: f32,
        opt: Opt,
    },
    /// An option of a shader pack the pack shows as a slider: its value's place in its list.
    ShaderSlider {
        label: Text,
        value: f32,
        name: String,
    },
    Text {
        text: Text,
        color: u32,
        centered: bool,
    },
    LanguageRow {
        label: Text,
        selected: bool,
        code: String,
    },
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: WKind,
    /// Scrolls with the list (clipped to the list area).
    list: bool,
    /// The list row it sits in (`ContainerObjectSelectionList`'s entry).
    row: usize,
}

/// The keyboard-focused UI target on an options page.
#[derive(Clone, Debug, PartialEq)]
enum FocusId {
    Search,
    Slider(Opt),
    Cycle(Opt),
    Act(Act),
    Lang(String),
    Index(usize),
}

fn focus_id(i: usize, wd: &Widget) -> FocusId {
    match &wd.kind {
        WKind::Slider { opt, .. } => FocusId::Slider(*opt),
        WKind::Button {
            act: Act::Cycle(opt),
            ..
        } => FocusId::Cycle(*opt),
        WKind::Button { act, .. } if *act != Act::None => FocusId::Act(act.clone()),
        WKind::LanguageRow { code, .. } => FocusId::Lang(code.clone()),
        _ => FocusId::Index(i),
    }
}

fn matches_focus(id: &FocusId, i: usize, wd: &Widget) -> bool {
    match (id, &wd.kind) {
        (FocusId::Slider(a), WKind::Slider { opt, .. }) => a == opt,
        (
            FocusId::Cycle(a),
            WKind::Button {
                act: Act::Cycle(opt),
                ..
            },
        ) => a == opt,
        (FocusId::Act(a), WKind::Button { act, .. }) => a == act,
        (FocusId::Lang(a), WKind::LanguageRow { code, .. }) => a == code,
        (FocusId::Index(idx), _) => *idx == i,
        _ => false,
    }
}
/// What the game must do after an interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An accepted button press or the release of a slider.
    ButtonClick,
    /// Close the options screens (back to the pause menu or title).
    Close,
    /// Settings changed: apply and save them.
    Changed,
    /// A slider is being dragged: apply settings live but defer saving.
    SliderDrag,
    /// A category volume slider (`SoundSource` index) moved to a new value,
    /// after its `SliderDrag` (the sound slider's `onValueUpdate`).
    Volume(usize, f32),
    /// The language changed (reload translations).
    Language(String),
    /// The pack selection changed (reload resources).
    Packs(Vec<String>),
    /// Create World's Data Packs screen closed with a new selection.
    DataPacks(Vec<String>),
    OpenPackFolder,
    /// Data Packs' Open Pack Folder: create the folder if needed and open it.
    OpenDataPackFolder(std::path::PathBuf),
    OpenModFolder,
    /// The Shader Packs page opened: list `shaderpacks/` into `shader_packs`.
    ListShaderPacks,
    /// A shader pack was picked or turned off: save and apply it.
    ShaderPack,
    OpenShaderPackFolder,
    /// The Shader Options page opened: read the picked pack's options into `shader_options`.
    OpenShaderOptions,
    /// The Shader Options page closed with changes: save them and load the pack again.
    ShaderOptions,
    /// The options of `shader_packs[.0]`, a Horizon pack, changed: save them (the pack reloads
    /// with them).
    NativePackOptions(usize),
    /// Native mod `.0`'s page opened: the game asks the mod for its labels
    /// again (they were made with the list, maybe in another language).
    NativeSettings(usize),
    /// Button `.1` of native mod `.0`'s page was pressed: the game tells the
    /// mod and refreshes the page.
    NativeButton(usize, usize),
    /// Roll the credits (`CreditsAndAttributionScreen.openCreditsScreen`).
    Credits,
    /// Copy how an agent connects (the AI Agent page).
    CopyAgentDetails,
    /// The narrator option changed (after its `Changed`): the option's
    /// `onValueUpdate`, `GameNarrator.updateNarratorStatus`.
    Narrator,
}
/// `SliderableValueSet.applyValueImmediately`: these apply 600 ms after
/// the last change (`OptionInstanceSliderButton.delayedApplyAt`).
fn delays_apply(opt: Opt) -> bool {
    matches!(
        opt,
        Opt::RenderDistance | Opt::SimulationDistance | Opt::BiomeBlend | Opt::WheelSensitivity
    )
}
/// Whether a slider still shows its own position: while held, or for a
/// delayed one until its value applies (both then `resetValue`).
fn slider_unsnapped(opt: Opt, held: bool, moved: Instant) -> bool {
    if delays_apply(opt) {
        moved.elapsed().as_millis() < 600
    } else {
        held
    }
}
pub struct OptionsUi {
    pub stack: Vec<Page>,
    scroll: f32,
    pub(crate) dragging: Option<Opt>,
    /// The shader pack option whose slider is held, and the handle's own position: it follows
    /// the pointer while the option takes the nearest of its values.
    shader_dragging: Option<(String, f32)>,
    /// Shift is held during the click (it resets a shader pack option to its default).
    pub shift: bool,
    /// The pressed slider's own position (`AbstractSliderButton.value`) and
    /// when it last moved: the handle follows the pointer while the option
    /// keeps its stepped value, then snaps to it (`resetValue`).
    slider_value: Option<(Opt, f32, Instant)>,
    /// The pointer's last y while a scrollbar is held.
    pub(crate) scrollbar_drag_y: Option<f32>,
    /// Key Binds: the mapping waiting for a key ([`NATIVE_KEY`]: a native
    /// mod's, `selected_native_key`).
    pub selected_key: Option<&'static str>,
    /// The native mod's key waiting: (mod, key) in `mods.installed`.
    selected_native_key: Option<(usize, usize)>,
    /// Controller Binds: the action waiting for a pad button.
    pub selected_pad_action: Option<&'static str>,
    /// The glyphs the binds list shows (the setting, else the pad's family).
    pub pad_style: GlyphStyle,
    pub languages: Vec<LanguageInfo>,
    language_choice: Option<String>,
    language_search: ui::EditBox,
    pub(crate) settings_search: ui::EditBox,
    /// Packs: the packs found and the selection the screen opens with.
    pub pack_infos: Vec<crate::pack_screen::PackInfo>,
    pub pack_selection: Vec<String>,
    /// The open pack screen, made at its first frame.
    pub pack_screen: Option<crate::pack_screen::PackScreen>,
    /// The Packs page is Create World's Data Packs screen on this folder.
    data_pack_dir: Option<std::path::PathBuf>,
    /// The game's resources, for the built-in data pack's icon.
    data_pack_resources: std::path::PathBuf,
    /// The folder's entries when its packs were listed, and when they were
    /// last compared with it.
    data_pack_stamp: (Vec<(String, Option<std::time::SystemTime>)>, Instant),
    /// A reload failure, dismissed before returning to the current page.
    pub resource_error: Option<String>,
    /// When the pack screen's search box took the focus (its cursor blink).
    pack_focus: Instant,
    /// The selection carried over from a pack screen made again over a changed pack list
    /// (`update_pack_infos`) differs from the settings'.
    pack_reselected: bool,
    pub in_world: bool,
    /// Scroll the selected language into the middle on the next layout.
    center_language: bool,
    pub graphics: crate::graphics_ui::GraphicsUi,
    pub split_profiles: crate::split_profiles::SplitProfiles,
    /// HDR calibration while its page is open.
    pub calibration: Option<crate::hdr_calibration::Calibration>,
    /// Whether the window was fullscreen before calibration made it so (the game keeps it).
    pub calibration_fullscreen: Option<bool>,
    /// The AI Agent page's status lines (set by the game while it shows).
    pub agent_status: Vec<String>,
    /// Keyboard focus: the interactive target currently focused.
    focus: Option<FocusId>,
    /// Whether a focused slider is in value-editing mode (`AbstractSliderButton.canChangeValue`).
    pub(crate) can_change_value: bool,
    /// The current GUI size (w, h).
    pub size: (f32, f32),
    /// The current window framebuffer dimensions [w, h].
    pub framebuffer: [i32; 2],
    /// Splitscreen: another player's options. Language and resource packs are the whole
    /// process's (every player's text and textures), so they're P1's to change.
    pub split_guest: bool,
    /// The list's rows as last laid out: each row's top and bottom (GUI
    /// pixels), for the narration's row under the mouse.
    row_bands: Vec<[f32; 2]>,
}
impl Default for OptionsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 32;
        let mut settings_search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        settings_search.focused = false;
        Self {
            stack: vec![Page::Main],
            scroll: 0.0,
            dragging: None,
            shader_dragging: None,
            shift: false,
            slider_value: None,
            scrollbar_drag_y: None,
            selected_key: None,
            selected_native_key: None,
            selected_pad_action: None,
            pad_style: GlyphStyle::Xbox,
            languages: Vec::new(),
            language_choice: None,
            language_search: search,
            settings_search,
            pack_infos: Vec::new(),
            pack_selection: Vec::new(),
            pack_screen: None,
            data_pack_dir: None,
            data_pack_resources: Default::default(),
            data_pack_stamp: (Vec::new(), Instant::now()),
            resource_error: None,
            pack_focus: Instant::now(),
            pack_reselected: false,
            in_world: false,
            center_language: false,
            graphics: Default::default(),
            split_profiles: Default::default(),
            calibration: None,
            calibration_fullscreen: None,
            agent_status: Vec::new(),
            focus: None,
            can_change_value: false,
            size: (427.0, 240.0),
            framebuffer: [1280, 720],
            split_guest: false,
            row_bands: Vec::new(),
        }
    }
}
fn button(x: f32, y: f32, w: f32, label: Text, act: Act) -> Widget {
    Widget {
        x,
        y,
        w,
        h: 20.0,
        kind: WKind::Button {
            label,
            active: true,
            act,
        },
        list: false,
        row: 0,
    }
}
fn inactive(mut w: Widget) -> Widget {
    if let WKind::Button { active, .. } = &mut w.kind {
        *active = false;
    }
    w
}
fn option_widget(s: &Settings, opt: Opt, x: f32, y: f32, w: f32) -> Widget {
    // OptionsSubScreen.addContents: the narrator button is active only while
    // the narrator is (`narratorButton.active = narrator.isActive()`).
    if !opt.available() || opt == Opt::Narrator && !horizon_narrator::is_active() {
        return inactive(button(x, y, w, opt.label(s), Act::Cycle(opt)));
    }
    // VideoSettingsScreen.tick: Max Anisotropy only works with Anisotropic filtering.
    if opt == Opt::MaxAnisotropy && s.file.int("textureFiltering") != 2 {
        return inactive(button(x, y, w, opt.label(s), Act::Cycle(opt)));
    }
    match opt.kind(s) {
        Kind::Slider(v) => Widget {
            x,
            y,
            w,
            h: 20.0,
            kind: WKind::Slider {
                label: opt.label(s),
                value: v,
                opt,
            },
            list: false,
            row: 0,
        },
        Kind::Cycle => button(x, y, w, opt.label(s), Act::Cycle(opt)),
    }
}
/// What a button opening one of Horizon's pages says on hover.
fn horizon_page_tooltip(page: Page) -> Option<&'static str> {
    Some(match page {
        Page::Horizon => "horizon.pages.settings.tooltip",
        Page::Graphics => "horizon.pages.graphics.tooltip",
        Page::GraphicsAdvanced => "horizon.pages.graphics_advanced.tooltip",
        Page::Controller => "horizon.pages.controller.tooltip",
        Page::Extras => "horizon.pages.extras.tooltip",
        Page::Agent => "horizon.pages.agent.tooltip",
        Page::SplitProfiles => "horizon.pages.split_profiles.tooltip",
        Page::ShaderPacks => "horizon.pages.shader_packs.tooltip",
        Page::Mods => "horizon.pages.mods.tooltip",
        _ => return None,
    })
}
/// The open screen of the picked pack: its title, then its items two to a row (one when the
/// pack asks for a single column).
/// A list's slot with nothing in it: text takes no clicks.
fn blank_slot(w: f32) -> Widget {
    Widget {
        x: 0.0,
        y: 0.0,
        w,
        h: 20.0,
        kind: WKind::Text {
            text: Text::literal(""),
            color: 0,
            centered: false,
        },
        list: false,
        row: 0,
    }
}
/// Page `at` of the options of the Horizon pack `s.shader_packs[i]`: the pack's own layout,
/// else every option two a row.
fn native_pack_option_rows(s: &Settings, i: usize, at: u8) -> Vec<Row> {
    use horizon_shader_api::PageItem;
    let Some(pack) = s.shader_packs.get(i) else {
        return Vec::new();
    };
    let count = pack.options.len().min(pack.values.len());
    let (columns, items) = match pack.pages.get(usize::from(at)) {
        Some(page) => (usize::from(page.columns), page.items.clone()),
        None => (2, (0..count).map(PageItem::Option).collect()),
    };
    let width = if columns == 1 { 310.0 } else { 150.0 };
    let widget = |item: &PageItem| match *item {
        PageItem::Option(o) if o < count => {
            let option = &pack.options[o];
            let value = crate::shader_pack_list::show_value(&option.kind, pack.values[o]);
            let label = Text::literal(format!("{}: {value}", option.name));
            button(0.0, 0.0, width, label, Act::NativePackOption(i, o))
        }
        PageItem::Page(p) if p < pack.pages.len() => {
            let label = Text::literal(format!("{}...", pack.pages[p].name));
            let page = Page::NativePackOptions(i, p as u8);
            button(0.0, 0.0, width, label, Act::Push(page))
        }
        _ => blank_slot(width),
    };
    let mut rows = Vec::new();
    if at == 0 {
        rows.push(Row::Note(pack.title.clone()));
    }
    rows.extend(
        items
            .chunks(columns)
            .map(|row| Row::Buttons(widget(&row[0]), row.get(1).map(widget))),
    );
    rows
}
fn shader_option_rows(s: &Settings) -> Vec<Row> {
    let Some(options) = &s.shader_options else {
        return vec![Row::Header("horizon.shader_options.loading")];
    };
    let columns = options
        .screens
        .screen(options.screen())
        .map_or(2, |(_, c)| c);
    let blank = || blank_slot(150.0);
    let width = if columns == 1 { 310.0 } else { 150.0 };
    let widget = |item: &horizon_shaders::ScreenItem| -> Option<Widget> {
        use horizon_shaders::ScreenItem;
        Some(match item {
            ScreenItem::Empty if columns > 2 => return None,
            ScreenItem::Empty => blank(),
            ScreenItem::Profile => {
                button(0.0, 0.0, width, options.profile_label(), Act::ShaderProfile)
            }
            ScreenItem::Screen(name) => button(
                0.0,
                0.0,
                width,
                options.screen_label(name),
                Act::ShaderScreen(name.clone()),
            ),
            ScreenItem::Option(name) => match options.slider_position(name) {
                Some(value) => Widget {
                    x: 0.0,
                    y: 0.0,
                    w: width,
                    h: 20.0,
                    kind: WKind::ShaderSlider {
                        label: options.option_label(name),
                        value,
                        name: name.clone(),
                    },
                    list: false,
                    row: 0,
                },
                None => button(
                    0.0,
                    0.0,
                    width,
                    options.option_label(name),
                    Act::ShaderOption(name.clone()),
                ),
            },
        })
    };
    let widgets: Vec<Widget> = options.items().iter().filter_map(widget).collect();
    let mut rows = vec![Row::Note(options.title().strip_legacy().plain())];
    if columns == 1 {
        rows.extend(widgets.into_iter().map(|w| Row::Buttons(w, None)));
        return rows;
    }
    let mut widgets = widgets.into_iter();
    while let Some(a) = widgets.next() {
        rows.push(Row::Buttons(a, widgets.next()));
    }
    rows
}
/// Rows of an OptionsList: (big?, options) or a header.
enum Row {
    Big(Opt),
    Pair(Opt, Option<Opt>),
    Header(&'static str),
    /// A line of grey text (a path).
    Note(String),
    Buttons(Widget, Option<Widget>),
}
/// A widget's tooltip (`setTooltip`), unwrapped: drawn while hovered, and
/// its narration's hint.
fn tooltip_text(s: &Settings, wd: &Widget) -> Option<Text> {
    let pack_text = match &wd.kind {
        WKind::Button {
            act: Act::NativePackOption(pack, option),
            ..
        } => Some(&s.shader_packs.get(*pack)?.options.get(*option)?.description),
        WKind::Button {
            act: Act::Push(Page::NativePackOptions(pack, at)),
            ..
        } => Some(
            &s.shader_packs
                .get(*pack)?
                .pages
                .get(usize::from(*at))?
                .description,
        ),
        _ => None,
    };
    if let Some(text) = pack_text {
        return (!text.is_empty()).then(|| Text::literal(text));
    }
    let WKind::Button { act, .. } = &wd.kind else {
        let WKind::Slider { opt, .. } = &wd.kind else {
            return None;
        };
        let key = opt.tooltip_key(s);
        return lang::has(&key).then(|| tr(&key));
    };
    let mut tip = None;
    if let Act::Extra(item) = act {
        tip = Some(Text::literal(item.description()));
    }
    if let Act::Push(page) = act
        && let Some(text) = horizon_page_tooltip(*page)
    {
        tip = Some(tr(text));
    }
    let key = match act {
        Act::Cycle(opt) => Some(opt.tooltip_key(s)),
        Act::Push(Page::Language) => Some("options.language.tooltip".into()),
        Act::Push(Page::Accessibility) => Some("options.accessibility.tooltip".into()),
        _ => None,
    };
    if let Some(key) = key.filter(|key| lang::has(key)) {
        tip = Some(tr(&key));
    }
    if let Act::Cycle(Opt::GraphicsBackend) = act {
        tip = Some(graphics_api_tooltip(s));
    }
    match act {
        Act::Cycle(opt) if !opt.available() => Some(tr("horizon.options.unavailable")),
        Act::ToggleMod(i) => match s.mods.installed.get(*i) {
            Some(installed) => {
                let description = installed
                    .problem
                    .as_deref()
                    .unwrap_or(&installed.description);
                Some(Text::literal(format!("{}\n{description}", installed.title)))
            }
            None => tip,
        },
        Act::Key(name) => {
            let conflicts = s.keys.conflicts(name);
            if conflicts.is_empty() {
                return tip;
            }
            let names = conflicts
                .iter()
                .map(|c| lang::get(c))
                .collect::<Vec<_>>()
                .join(", ");
            Some(Text::translate(
                "controls.keybinds.duplicateKeybinds",
                &[Text::literal(names)],
            ))
        }
        _ => tip,
    }
}
/// The value part of an option's message (`CycleButton.createLabelForValue`):
/// its `options.generic_value` message without the caption.
fn option_value(opt: Opt, s: &Settings) -> String {
    let full = opt.label(s).plain();
    let marker = "\u{1}";
    let template = generic(opt.caption(), Text::literal(marker)).plain();
    match template.split_once(marker) {
        Some((before, after))
            if full.len() >= before.len() + after.len()
                && full.starts_with(before)
                && full.ends_with(after) =>
        {
            full[before.len()..full.len() - after.len()].to_owned()
        }
        _ => full,
    }
}
/// What widget `wd` narrates, over what `ui::button` or `ui::slider`
/// recorded for it (`n`): a cycle button the value a click moves to (worked
/// out only while it may be narrated: under the mouse or focused), a key
/// bind's buttons `narrator.controls.*`, its tooltip as a hint; a list row's
/// widget belongs to the list keyed `list`.
fn narrate_widget(
    n: &mut crate::widgets::Narratable,
    s: &Settings,
    wd: &Widget,
    hover: bool,
    list: u64,
    framebuffer: [i32; 2],
) {
    use crate::widgets::What;
    if let WKind::Button { label, active, act } = &wd.kind {
        let t = |key: &str, args: &[Text]| Text::translate(key, args).plain();
        let bind = |name: Text, bound: bool| {
            if bound {
                let button = Text::translate("gui.narrate.button", std::slice::from_ref(label));
                t("narrator.controls.bound", &[name, button])
            } else {
                t("narrator.controls.unbound", &[name])
            }
        };
        let active = *active;
        match act {
            Act::Cycle(opt) if active => {
                let next = if hover || crate::widgets::narration_focused(n.rect) {
                    let mut next = s.clone();
                    opt.cycle(&mut next, false, framebuffer);
                    option_value(*opt, &next)
                } else {
                    String::new()
                };
                n.what = What::Cycle {
                    message: label.plain(),
                    next,
                    active,
                };
            }
            Act::Key(name) => {
                let bound = s.keys.get(name) != "key.keyboard.unknown";
                n.what = What::Named {
                    title: bind(tr(name), bound),
                    active,
                };
            }
            Act::PadKey(action) => {
                let bound = s.controller.binding(action).is_some();
                n.what = What::Named {
                    title: bind(tr(gamepad::action_caption(action)), bound),
                    active,
                };
            }
            Act::ResetKey(name) => {
                n.what = What::Named {
                    title: t("narrator.controls.reset", &[tr(name)]),
                    active,
                };
            }
            Act::ResetPadKey(action) => {
                n.what = What::Named {
                    title: t(
                        "narrator.controls.reset",
                        &[tr(gamepad::action_caption(action))],
                    ),
                    active,
                };
            }
            _ => {}
        }
    }
    n.hint = tooltip_text(s, wd).map(|tip| tip.plain());
    if wd.list {
        // Stable while the list scrolls: its row and column.
        n.key = crate::widgets::key_of(&(list, wd.row, wd.x.round() as i32));
        n.parent = Some((list, wd.row));
    }
}
/// A list's rows as they are laid out: the row of each widget pushed and
/// each row's band, for the narration (`ContainerObjectSelectionList`'s
/// entries and the one under the mouse).
#[derive(Default)]
struct ListRows {
    start: usize,
    top: f32,
    bands: Vec<[f32; 2]>,
}
impl ListRows {
    /// A row starts at `y` with the next widget pushed.
    fn begin(&mut self, out: &[Widget], y: f32) {
        self.start = out.len();
        self.top = y;
    }
    /// The row ends at `y`: the widgets pushed since `begin` are in it.
    fn end(&mut self, out: &mut [Widget], y: f32) {
        let row = self.bands.len();
        out[self.start..].iter_mut().for_each(|w| w.row = row);
        self.bands.push([self.top, y]);
    }
}
fn pairs(opts: &[Opt]) -> Vec<Row> {
    opts.chunks(2)
        .map(|c| Row::Pair(c[0], c.get(1).copied()))
        .collect()
}
impl OptionsUi {
    /// A switch of `horizon-extras.txt` as the Extras page draws it: "Caption: On".
    fn extra_button(&self, s: &Settings, item: crate::extras::Item) -> Widget {
        let label = format!("{}: {}", item.caption(), item.value(&s.extras));
        button(0.0, 0.0, 150.0, Text::literal(label), Act::Extra(item))
    }
    fn settings_search_active(&self) -> bool {
        self.settings_search.focused
            && matches!(self.page(), Page::Video | Page::Controls | Page::KeyBinds)
    }
    /// Whether the resource pack screen is open (not Create World's data packs on the same
    /// page).
    pub fn resource_packs_open(&self) -> bool {
        self.page() == Page::Packs && self.data_pack_dir.is_none()
    }
    /// The packs in `resourcepacks/` changed while the pack screen is open
    /// (`PackSelectionScreen`'s directory watcher): the screen is made again over
    /// `infos`, keeping the selection made on it so far.
    pub fn update_pack_infos(&mut self, infos: Vec<crate::pack_screen::PackInfo>) {
        if let Some(p) = self.pack_screen.take()
            && p.changed
        {
            self.pack_selection = p.commit();
            self.pack_reselected = true;
        }
        self.pack_infos = infos;
    }
    pub fn page(&self) -> Page {
        *self.stack.last().unwrap_or(&Page::Main)
    }
    /// Whether the screen's focus is a search box that takes the keys (an
    /// `EditBox` that `canConsumeInput`): the language list's, the settings
    /// search, or the pack screen's.
    pub fn typing(&self) -> bool {
        self.resource_error.is_none()
            && match self.page() {
                Page::Language => true,
                Page::Packs => self
                    .pack_screen
                    .as_ref()
                    .is_some_and(|p| p.search_focused()),
                _ => self.settings_search_active(),
            }
    }
    /// Jump straight to a page (control socket / tests).
    pub fn show(&mut self, page: Page) {
        if page != Page::Main {
            self.push(page);
        }
    }
    pub fn open(&mut self, in_world: bool) {
        self.open_at(in_world, Page::Main);
    }
    /// Create World's Data Packs (`openDataPackSelectionScreen`): the Packs
    /// page on the new world's pack folder `dir`, `selected` chosen;
    /// `resources` holds the built-in pack's icon.
    pub fn open_data_packs(
        &mut self,
        dir: std::path::PathBuf,
        selected: Vec<String>,
        resources: std::path::PathBuf,
    ) {
        self.open_at(false, Page::Packs);
        self.pack_infos = crate::datapack::screen_packs(&dir, &resources);
        self.data_pack_resources = resources;
        self.pack_selection = selected;
        self.data_pack_stamp = (crate::datapack::folder_stamp(&dir), Instant::now());
        self.data_pack_dir = Some(dir);
    }
    /// `PackSelectionScreen.tick`'s watcher: once a second, the Data Packs
    /// folder's packs are listed again if it changed, the selection kept
    /// (the search text and scroll start over).
    fn watch_data_packs(&mut self, atlas: &HudAtlas, size: (f32, f32)) {
        let Some(dir) = &self.data_pack_dir else {
            return;
        };
        if self.data_pack_stamp.1.elapsed().as_secs() < 1 {
            return;
        }
        let stamp = crate::datapack::folder_stamp(dir);
        let changed = stamp != self.data_pack_stamp.0;
        self.data_pack_stamp = (stamp, Instant::now());
        if !changed {
            return;
        }
        self.pack_infos = crate::datapack::screen_packs(dir, &self.data_pack_resources);
        if let Some(p) = self.pack_screen.take() {
            self.pack_selection = p.commit();
            self.packs(atlas, size).changed = p.changed;
        }
    }
    /// Steps a slider option forward or backward, derived from its own slider definition.
    pub fn step_slider_option(
        &mut self,
        s: &mut Settings,
        opt: Opt,
        forward: bool,
        width: f32,
    ) -> Vec<Event> {
        let volume = |s: &Settings| match opt {
            Opt::Volume(0) => Some(s.master_volume),
            Opt::Volume(i) => Some(s.volumes[i]),
            _ => None,
        };
        let before_vol = volume(s);
        opt.step_slider(s, forward, width);
        self.slider_value = None;
        let mut events = vec![Event::Changed, Event::SliderDrag];
        if let (Opt::Volume(source), Some(after)) = (opt, volume(s))
            && before_vol != Some(after)
        {
            events.push(Event::Volume(source, after));
        }
        events
    }
    /// Open a page whose Done/Escape returns directly to the calling screen.
    pub fn open_at(&mut self, in_world: bool, page: Page) {
        self.stack.clear();
        self.data_pack_dir = None;
        self.focus = None;
        self.can_change_value = false;
        self.push(page);
        self.in_world = in_world;
        self.selected_key = None;
    }
    fn push(&mut self, page: Page) {
        self.stack.push(page);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.focus = None;
        self.can_change_value = false;
        if matches!(page, Page::Video | Page::Controls | Page::KeyBinds) {
            self.settings_search.set_value("");
            self.settings_search.focused = false;
        }
        if page == Page::Language {
            self.language_choice = None;
            self.language_search.set_value("");
            self.center_language = true;
        }
        if page == Page::Packs {
            self.pack_screen = None;
        }
        if matches!(
            page,
            Page::Graphics | Page::GraphicsAdvanced | Page::GraphicsHdr
        ) {
            self.graphics.reset();
        }
        if page == Page::SplitProfiles {
            self.split_profiles.reset();
        }
    }
    /// The graphics screen level of `page`, if it is one.
    fn graphics_level(page: Page) -> Option<crate::graphics_ui::Level> {
        match page {
            Page::Graphics => Some(crate::graphics_ui::Level::Simple),
            Page::GraphicsAdvanced => Some(crate::graphics_ui::Level::Advanced),
            Page::GraphicsHdr => Some(crate::graphics_ui::Level::Hdr),
            _ => None,
        }
    }
    /// The name of a Horizon pack's options page past its first, as the page's title.
    fn pack_page_title(s: &Settings, page: Page) -> Option<Text> {
        let Page::NativePackOptions(pack, at @ 1..) = page else {
            return None;
        };
        let page = s.shader_packs.get(pack)?.pages.get(usize::from(at))?;
        Some(Text::literal(&page.name))
    }
    fn title(page: Page) -> &'static str {
        match page {
            Page::Main => "options.title",
            Page::Video => "options.videoTitle",
            Page::Sound => "options.sounds.title",
            Page::Skin => "options.skinCustomisation.title",
            Page::Online => "options.online.title",
            Page::Controls => "controls.title",
            Page::KeyBinds => "controls.keybinds.title",
            Page::Mouse => "options.mouse_settings.title",
            Page::Chat => "options.chat.title",
            Page::Accessibility => "options.accessibility.title",
            Page::Language => "options.language.title",
            Page::Font => "options.font.title",
            Page::Packs => "resourcePack.title",
            Page::Mods => "horizon.pages.mods",
            Page::NativeMod(_) => "Mod Settings",
            Page::ShaderPacks => "horizon.pages.shader_packs",
            Page::ShaderOptions | Page::NativePackOptions(..) => "horizon.pages.shader_options",
            Page::Graphics | Page::GraphicsAdvanced | Page::GraphicsHdr | Page::HdrCalibration => {
                "options.videoTitle"
            }
            Page::Controller => "horizon.pages.controller",
            Page::PadBinds => "horizon.pages.controller_binds",
            Page::Extras => "horizon.pages.extras",
            Page::Credits => "credits_and_attribution.screen.title",
            Page::Horizon => "horizon.pages.settings",
            Page::Agent => "horizon.pages.agent",
            Page::SplitProfiles => "horizon.pages.split_profiles",
        }
    }
    fn scrollbar_x(&self, width: f32) -> f32 {
        (width / 2.0).floor()
            + match self.page() {
                Page::KeyBinds | Page::PadBinds => 178.0,
                Page::Language => 118.0,
                _ => 163.0,
            }
    }
    fn filter_option_rows(&self, rows: Vec<Row>) -> Vec<Row> {
        let query = self.settings_search.value.to_lowercase();
        if query.is_empty() {
            return rows;
        }
        let mut filtered = Vec::new();
        let mut pending_header = None;
        for row in rows {
            let matches = |opt: Opt| lang::get(opt.caption()).to_lowercase().contains(&query);
            let visible = match row {
                Row::Header(key) => {
                    pending_header = Some(key);
                    continue;
                }
                Row::Big(opt) if matches(opt) => vec![Row::Big(opt)],
                Row::Pair(a, b) => pairs(
                    &[Some(a), b]
                        .into_iter()
                        .flatten()
                        .filter(|opt| matches(*opt))
                        .collect::<Vec<_>>(),
                ),
                // Keep navigation available regardless of the query.
                Row::Buttons(a, b) => {
                    filtered.push(Row::Buttons(a, b));
                    continue;
                }
                _ => Vec::new(),
            };
            if !visible.is_empty() {
                // Emit a section heading only when its first matching option is found.
                if let Some(key) = pending_header.take() {
                    filtered.push(Row::Header(key));
                }
                filtered.extend(visible);
            }
        }
        filtered
    }

    /// A native mod's page: its name, its keys as Key Binds rows (the
    /// player's choice, else the default), then its buttons.
    fn native_rows(&self, s: &Settings, i: usize) -> Vec<Row> {
        let Some(m) = s.mods.installed.get(i) else {
            return Vec::new();
        };
        let mut rows = vec![Row::Note(m.title.clone())];
        let Some(page) = &m.native else {
            return rows;
        };
        let chosen = s.mods.keys.get(&m.id);
        for (k, [name, label, default]) in page.keys.iter().enumerate() {
            let key = chosen.and_then(|c| c.get(name)).unwrap_or(default);
            let mut message = keybinds::display_name(key);
            if self.selected_native_key == Some((i, k)) {
                message = Text::literal("> ")
                    .append(message.with_color(text::WHITE))
                    .append(" <")
                    .with_color(text::YELLOW);
            }
            let caption = button(0.0, 0.0, 150.0, Text::literal(label.clone()), Act::None);
            let change = button(0.0, 0.0, 150.0, message, Act::NativeKey(i, k));
            rows.push(Row::Buttons(inactive(caption), Some(change)));
        }
        for (b, label) in page.buttons.iter().enumerate() {
            let widget = button(
                0.0,
                0.0,
                310.0,
                Text::literal(label.clone()),
                Act::NativeButton(i, b),
            );
            rows.push(Row::Buttons(widget, None));
        }
        rows
    }

    fn list_rows(&self, s: &Settings) -> Vec<Row> {
        match self.page() {
            Page::Extras => {
                // Each section's switches two to a row, as vanilla's
                // OptionsList.addSmall lays out an options screen.
                let extra = |item: crate::extras::Item| {
                    let value = format!("{}: {}", item.caption(), item.value(&s.extras));
                    let label = if item.key_name().is_some() && self.selected_key == item.key_name()
                    {
                        Text::literal(format!("> {value} <")).with_color(text::YELLOW)
                    } else {
                        Text::literal(value)
                    };
                    let widget = button(0.0, 0.0, 150.0, label, Act::Extra(item));
                    // A setting of a switch that is off does nothing: inactive,
                    // as vanilla greys out an option that can't apply.
                    if item
                        .parent()
                        .is_some_and(|parent| parent.switch(&s.extras) == Some(false))
                    {
                        inactive(widget)
                    } else {
                        widget
                    }
                };
                let mut rows = Vec::new();
                let mut section = Vec::new();
                let flush = |rows: &mut Vec<Row>, section: &mut Vec<crate::extras::Item>| {
                    rows.extend(
                        section
                            .chunks(2)
                            .map(|c| Row::Buttons(extra(c[0]), c.get(1).map(|&b| extra(b)))),
                    );
                    section.clear();
                };
                for row in crate::extras::rows() {
                    match row {
                        Err(header) => {
                            flush(&mut rows, &mut section);
                            rows.push(Row::Header(header));
                        }
                        Ok(item) => section.push(item),
                    }
                }
                flush(&mut rows, &mut section);
                rows
            }
            Page::Horizon => {
                // Two to a row, as vanilla's OptionsList.addSmall lays out
                // buttons; each opens a page built from vanilla's widgets.
                let open =
                    |key: &str, page: Page| button(0.0, 0.0, 150.0, tr(key), Act::Push(page));
                vec![
                    Row::Buttons(
                        open("horizon.pages.open.graphics", Page::Graphics),
                        Some(open(
                            "horizon.pages.open.graphics_advanced",
                            Page::GraphicsAdvanced,
                        )),
                    ),
                    Row::Buttons(
                        open("horizon.pages.open.shader_packs", Page::ShaderPacks),
                        Some(open("horizon.pages.open.controller", Page::Controller)),
                    ),
                    Row::Buttons(
                        open("horizon.pages.open.extras", Page::Extras),
                        Some(open("horizon.pages.open.mods", Page::Mods)),
                    ),
                    // No AI hides the AI Agent page and everything AI.
                    Row::Buttons(
                        self.extra_button(s, crate::extras::Item::DiscordPresence),
                        Some(if s.extras.no_ai {
                            self.extra_button(s, crate::extras::Item::NoAi)
                        } else {
                            open("horizon.pages.open.agent", Page::Agent)
                        }),
                    ),
                ]
                .into_iter()
                .chain(
                    (!s.extras.no_ai).then(|| {
                        Row::Buttons(self.extra_button(s, crate::extras::Item::NoAi), None)
                    }),
                )
                .collect()
            }
            Page::Agent => {
                let mut rows = vec![Row::Buttons(
                    self.extra_button(s, crate::extras::Item::AgentControl),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("horizon.agent.copy_details"),
                        Act::CopyAgentDetails,
                    )),
                )];
                rows.extend(self.agent_status.iter().cloned().map(Row::Note));
                rows
            }
            Page::Video => {
                // Horizon's graphics screens, Simple and Advanced.
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        150.0,
                        tr("horizon.pages.open.horizon_graphics"),
                        Act::Push(Page::Graphics),
                    ),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("horizon.pages.open.graphics_advanced"),
                        Act::Push(Page::GraphicsAdvanced),
                    )),
                )];
                rows.push(Row::Header("options.video.display.header"));
                rows.push(Row::Big(Opt::ExclusiveMode));
                rows.extend(pairs(&[
                    Opt::MaxFps,
                    Opt::Vsync,
                    Opt::InactivityFps,
                    Opt::GuiScale,
                    Opt::Fullscreen,
                    Opt::ExclusiveFullscreen,
                    Opt::Gamma,
                    Opt::GraphicsBackend,
                ]));
                rows.push(Row::Header("options.video.quality.header"));
                rows.push(Row::Big(Opt::Preset));
                rows.extend(pairs(&[
                    Opt::BiomeBlend,
                    Opt::RenderDistance,
                    Opt::ChunkBuilder,
                    Opt::SimulationDistance,
                    Opt::SmoothLighting,
                    Opt::Clouds,
                    Opt::Particles,
                    Opt::Mipmaps,
                    Opt::EntityShadows,
                    Opt::EntityDistance,
                    Opt::MenuBlur,
                    Opt::CloudRange,
                    Opt::CutoutLeaves,
                    Opt::ImprovedTransparency,
                    Opt::TextureFiltering,
                    Opt::MaxAnisotropy,
                    Opt::WeatherRadius,
                ]));
                rows.push(Row::Header("options.video.preferences.header"));
                rows.extend(pairs(&[
                    Opt::Autosave,
                    Opt::Vignette,
                    Opt::AttackIndicator,
                    Opt::ChunkFade,
                ]));
                self.filter_option_rows(rows)
            }
            Page::Sound => {
                // SoundOptionsScreen, without Audio Device: Horizon opens the
                // output device itself and cannot switch it.
                let mut rows = vec![Row::Big(Opt::Volume(0))];
                let others: Vec<Opt> = (1..SOUND_SOURCES.len()).map(Opt::Volume).collect();
                rows.extend(pairs(&others));
                rows.extend(pairs(&[Opt::ShowSubtitles, Opt::DirectionalAudio]));
                rows.extend(pairs(&[Opt::MusicFrequency, Opt::MusicToast]));
                rows.push(Row::Big(Opt::SoundDevice));
                rows
            }
            Page::Skin => {
                // SkinCustomizationScreen: the seven layers, then Main Hand.
                let mut opts: Vec<Opt> = (0..MODEL_PART_KEYS.len()).map(Opt::ModelPart).collect();
                opts.push(Opt::MainHand);
                pairs(&opts)
            }
            Page::Online => {
                // OnlineOptionsScreen without the Xbox and friends-list parts.
                let mut rows = vec![Row::Header("options.online.friends.header")];
                rows.extend(pairs(&[Opt::InGameNotification, Opt::SharePresence]));
                rows.push(Row::Header("options.online.servers.header"));
                rows.push(Row::Big(Opt::AllowServerListing));
                rows.push(Row::Header("options.online.realms.header"));
                rows.push(Row::Big(Opt::RealmsNotifications));
                rows
            }
            Page::Controls => {
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        150.0,
                        tr("options.mouse_settings"),
                        Act::Push(Page::Mouse),
                    ),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("controls.keybinds"),
                        Act::Push(Page::KeyBinds),
                    )),
                )];
                let controls = vanilla_control_options();
                rows.extend(pairs(&controls));
                // Horizon's controller page, after vanilla's rows.
                rows.push(Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        tr("horizon.pages.open.controller"),
                        Act::Push(Page::Controller),
                    ),
                    None,
                ));
                self.filter_option_rows(rows)
            }
            Page::Controller => {
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        tr("horizon.pages.open.controller_binds"),
                        Act::Push(Page::PadBinds),
                    ),
                    None,
                )];
                rows.extend(pairs(&[
                    Opt::PadEnabled,
                    Opt::PadVibration,
                    Opt::PadAdaptiveTriggers,
                    Opt::PadLightbar,
                    Opt::PadLookSensitivity,
                    Opt::PadToggleSneak,
                    Opt::PadInvertX,
                    Opt::PadInvertY,
                    Opt::PadMoveDeadzone,
                    Opt::PadLookDeadzone,
                    Opt::PadGlyphs,
                    Opt::PadMenuNav,
                    Opt::PadKeyboard,
                ]));
                rows
            }
            Page::Mouse => pairs(MOUSE_OPTIONS),
            Page::Chat => pairs(&[
                Opt::ChatVisibility,
                Opt::ChatColors,
                Opt::ChatLinks,
                Opt::ChatLinksPrompt,
                Opt::ChatOpacity,
                Opt::TextBackgroundOpacity,
                Opt::ChatScale,
                Opt::ChatLineSpacing,
                Opt::ChatWidth,
                Opt::ChatHeightFocused,
                Opt::ChatHeightUnfocused,
                Opt::Narrator,
                Opt::AutoSuggestions,
                Opt::HideMatchedNames,
                Opt::ReducedDebugInfo,
                Opt::OnlySecureChat,
                Opt::SaveChatDrafts,
            ]),
            Page::Accessibility => {
                let mut rows = vec![Row::Buttons(
                    option_widget(s, Opt::Narrator, 0.0, 0.0, 150.0),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("options.controls"),
                        Act::Push(Page::Controls),
                    )),
                )];
                rows.extend(pairs(&ACCESSIBILITY_OPTIONS[1..]));
                rows
            }
            Page::Font => pairs(&[Opt::ForceUnicode, Opt::JapaneseGlyphs]),
            Page::Mods => {
                if s.mods.installed.is_empty() {
                    return vec![Row::Header("horizon.mods.empty")];
                }
                s.mods
                    .installed
                    .iter()
                    .enumerate()
                    .map(|(i, installed)| {
                        let mut label = Text::literal(format!("{}: ", installed.title))
                            .append(on_off(s.mods.is_on(i)));
                        if installed.problem.as_deref() == Some(crate::mod_list::NOT_BUILT) {
                            label = label.append(tr("horizon.mods.not_built"));
                        } else if installed.problem.is_some() {
                            label = label.append(tr("horizon.mods.stopped"));
                        } else if installed.dev {
                            label = label.append(tr("horizon.mods.dev"));
                        }
                        let dead = installed.dev || installed.id.is_empty();
                        // Only native mods with actual settings need a second button.
                        if installed
                            .native
                            .as_ref()
                            .is_some_and(|page| !page.keys.is_empty() || !page.buttons.is_empty())
                        {
                            let toggle = button(0.0, 0.0, 150.0, label, Act::ToggleMod(i));
                            let settings = button(
                                0.0,
                                0.0,
                                150.0,
                                tr("horizon.mods.settings"),
                                Act::ModSettings(i),
                            );
                            Row::Buttons(
                                if dead { inactive(toggle) } else { toggle },
                                Some(settings),
                            )
                        } else {
                            let widget = button(0.0, 0.0, 310.0, label, Act::ToggleMod(i));
                            Row::Buttons(if dead { inactive(widget) } else { widget }, None)
                        }
                    })
                    .collect()
            }
            Page::NativeMod(i) => self.native_rows(s, i),
            Page::ShaderPacks => {
                if s.shader_packs.is_empty() {
                    return vec![Row::Header("horizon.shader_packs.empty")];
                }
                let picked = s
                    .shader_packs
                    .iter()
                    .position(|p| p.file == s.horizon.shader_pack);
                let options = picked.and_then(|i| {
                    let pack = &s.shader_packs[i];
                    let act = match pack.kind {
                        crate::shader_pack_list::Kind::Classic => Act::ShaderOptions,
                        _ if pack.options.is_empty() => return None,
                        crate::shader_pack_list::Kind::Horizon => Act::NativePackOptions(i),
                    };
                    let label = tr("horizon.shader_options.open");
                    Some(Row::Buttons(button(0.0, 0.0, 310.0, label, act), None))
                });
                options
                    .into_iter()
                    .chain(s.shader_packs.iter().enumerate().map(|(i, pack)| {
                        let kind = match pack.kind {
                            crate::shader_pack_list::Kind::Horizon => {
                                lang::get("horizon.shader_packs.kind_horizon")
                            }
                            crate::shader_pack_list::Kind::Classic => String::new(),
                        };
                        let label = Text::literal(format!("{}{kind}: ", pack.title))
                            .append(on_off(s.horizon.shader_pack == pack.file));
                        Row::Buttons(button(0.0, 0.0, 310.0, label, Act::PickShaderPack(i)), None)
                    }))
                    .collect()
            }
            Page::ShaderOptions => shader_option_rows(s),
            Page::NativePackOptions(i, at) => native_pack_option_rows(s, i, at),
            _ => {
                let _ = s;
                Vec::new()
            }
        }
    }
    /// Build every widget of the current page.
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
        let mut rows = ListRows::default();
        let page = self.page();
        let cx = (w / 2.0).floor();
        if matches!(page, Page::Video | Page::Controls | Page::KeyBinds) {
            self.settings_search.x = cx - 100.0;
            self.settings_search.y = 17.0;
        }
        let done = |y: f32| button(cx - 100.0, y, 200.0, tr("gui.done"), Act::Done);
        // (list top, list bottom, content height)
        let (mut top, mut bottom, mut content) = (33.0, h - 33.0, 0.0);
        match page {
            Page::Main => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(option_widget(s, Opt::Fov, cx - 154.0, 29.0, 150.0));
                out.push(button(
                    cx + 4.0,
                    29.0,
                    150.0,
                    tr("options.online"),
                    Act::Push(Page::Online),
                ));
                let entries = [
                    ("options.skinCustomisation", Some(Page::Skin)),
                    ("options.sounds", Some(Page::Sound)),
                    ("options.video", Some(Page::Video)),
                    ("options.controls", Some(Page::Controls)),
                    (
                        "options.language",
                        (!self.split_guest).then_some(Page::Language),
                    ),
                    ("options.chat", Some(Page::Chat)),
                    (
                        "options.resourcepack",
                        (!self.split_guest).then_some(Page::Packs),
                    ),
                    ("options.accessibility", Some(Page::Accessibility)),
                    // Vanilla's Telemetry Data and Credits slots: Horizon's
                    // settings and its mods, as a mod adds its button here.
                    // Credits & Attribution (WinScreen's credits) follows them.
                    ("horizon.pages.open.horizon_settings", Some(Page::Horizon)),
                    ("horizon.pages.open.mods", Some(Page::Mods)),
                    ("options.credits_and_attribution", Some(Page::Credits)),
                ];
                let rows = entries.len().div_ceil(2) as f32;
                let gy = (61.0 + 30.0f32).min(h - 33.0 - rows * 24.0);
                for (i, (key, page)) in entries.iter().enumerate() {
                    let x = cx - 158.0 + 4.0 + (i % 2) as f32 * 158.0;
                    let y = gy + (i / 2) as f32 * 24.0;
                    let b = button(x, y, 150.0, tr(key), page.map_or(Act::None, Act::Push));
                    out.push(if page.is_some() { b } else { inactive(b) });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::Credits => {
                // CreditsAndAttributionScreen: the title header, three 210-wide
                // buttons 8 apart (HeaderAndFooterLayout: 30 below the header
                // unless the footer is closer), Done.
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let y = (33.0 + 30.0f32).min(h - 33.0 - 76.0);
                let x = cx - 105.0;
                out.push(button(
                    x,
                    y,
                    210.0,
                    tr("credits_and_attribution.button.credits"),
                    Act::Credits,
                ));
                // ConfirmLinkScreen to minecraft.net: Horizon opens no links.
                for (i, key) in [
                    "credits_and_attribution.button.attribution",
                    "credits_and_attribution.button.licenses",
                ]
                .into_iter()
                .enumerate()
                {
                    let y = y + 28.0 * (i + 1) as f32;
                    out.push(inactive(button(x, y, 210.0, tr(key), Act::None)));
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::KeyBinds => {
                top = 38.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let mut mappings: Vec<(&str, &str)> =
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect();
                let query = self.settings_search.value.to_lowercase();
                mappings.retain(|(name, _)| {
                    query.is_empty() || lang::get(name).to_lowercase().contains(&query)
                });
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb)
                        .then_with(|| lang::get(a.0).cmp(&lang::get(b.0)))
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                // VideoSettingsScreen.addTitle: a changed graphics API needs a restart.
                if page == Page::Video && s.horizon.api != s.startup_api {
                    out.push(Widget {
                        x: 0.0,
                        y: 15.0,
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("options.restartRequired"),
                            color: rgba(223, 80, 80, 255),
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
                        rows.begin(&out, y);
                        let label =
                            Text::translate(&format!("key.category.minecraft.{category}"), &[])
                                .with_style(&Style {
                                    bold: Some(true),
                                    underlined: Some(true),
                                    ..Style::default()
                                });
                        out.push(Widget {
                            x: 0.0,
                            y: y + 7.0,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: label,
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr(name),
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                        row: 0,
                    });
                    let key = s.keys.get(name).to_owned();
                    let mut message = keybinds::display_name(&key);
                    let conflicts = s.keys.conflicts(name);
                    if !conflicts.is_empty() {
                        message = Text::literal("[ ")
                            .append(message.with_color(text::WHITE))
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_key == Some(name) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                color: Some(text::WHITE),
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    let name_static = MAPPINGS.iter().find(|m| m.0 == name).map_or("", |m| m.0);
                    out.push(button(change_x, y, 75.0, message, Act::Key(name_static)));
                    if !conflicts.is_empty() {
                        out.push(Widget {
                            x: change_x - 6.0,
                            y: y + 1.0,
                            w: 3.0,
                            h: 18.0,
                            kind: WKind::Text {
                                text: Text::empty(),
                                color: rgba(255, 255, 0, 255),
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetKey(name_static),
                    );
                    out.push(if s.keys.is_default(name) {
                        inactive(reset)
                    } else {
                        reset
                    });
                    y += 20.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::PadBinds => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let pad = &s.controller;
                for (action, _) in gamepad::ACTIONS {
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr(gamepad::action_caption(action)),
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                        row: 0,
                    });
                    let mut message = match pad.binding(action) {
                        Some(b) => {
                            let [r, g, bl] = b.color(self.pad_style);
                            Text::literal(b.label(self.pad_style))
                                .with_color(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(bl))
                        }
                        None => tr("key.keyboard.unknown"),
                    };
                    if pad.conflicts(action) {
                        message = Text::literal("[ ")
                            .append(message)
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_pad_action == Some(*action) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    out.push(button(change_x, y, 75.0, message, Act::PadKey(action)));
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetPadKey(action),
                    );
                    out.push(if pad.is_default(action) {
                        inactive(reset)
                    } else {
                        reset
                    });
                    y += 20.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetPadKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Language => {
                top = 36.0;
                bottom = h - 53.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                self.language_search.x = cx - 100.0;
                self.language_search.y = 17.0;
                let query = self.language_search.value.to_lowercase();
                let current = self
                    .language_choice
                    .clone()
                    .unwrap_or_else(|| s.language.clone());
                if std::mem::take(&mut self.center_language)
                    && let Some(i) = self.languages.iter().position(|l| l.code == current)
                {
                    // AbstractSelectionList.centerScrollOn.
                    self.scroll = (i as f32 * 18.0 + 9.0 - (bottom - top) / 2.0).max(0.0);
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for l in &self.languages {
                    if !query.is_empty()
                        && !l.label().to_lowercase().contains(&query)
                        && !l.code.contains(&query)
                    {
                        continue;
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: cx - 110.0,
                        y,
                        w: 220.0,
                        h: 18.0,
                        kind: WKind::LanguageRow {
                            label: Text::literal(l.label()),
                            selected: l.code == current,
                            code: l.code.clone(),
                        },
                        list: false,
                        row: 0,
                    });
                    y += 18.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(Widget {
                    x: 0.0,
                    y: h - 45.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.languageAccuracyWarning"),
                        color: rgba(0xBA, 0xBA, 0xBA, 255),
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(button(
                    cx - 154.0,
                    h - 28.0,
                    150.0,
                    tr("options.font"),
                    Act::Push(Page::Font),
                ));
                out.push(button(cx + 4.0, h - 28.0, 150.0, tr("gui.done"), Act::Done));
            }
            // Drawn by `pack_screen`.
            Page::Packs => {}
            _ => {
                if matches!(page, Page::Video | Page::Controls) {
                    top = 38.0;
                }
                out.push(Widget {
                    x: 0.0,
                    y: if matches!(page, Page::Video | Page::Controls) {
                        4.0
                    } else {
                        12.0
                    },
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: Self::pack_page_title(s, page)
                            .unwrap_or_else(|| tr(Self::title(page))),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
                    rows.begin(&out, y);
                    match row {
                        Row::Header(key) => {
                            let padding = if y + self.scroll <= top + 2.0 {
                                0.0
                            } else {
                                18.0
                            };
                            let label = tr(key).with_style(&Style {
                                bold: Some(true),
                                underlined: Some(true),
                                ..Style::default()
                            });
                            out.push(Widget {
                                x: 0.0,
                                y: y + 2.0 + padding + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: label,
                                    color: u32::MAX,
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += padding + 18.0;
                        }
                        Row::Note(text) => {
                            out.push(Widget {
                                x: 0.0,
                                y: y + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal(text),
                                    color: 0xFFA0A0A0,
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += 16.0;
                        }
                        Row::Big(opt) => {
                            out.push(option_widget(s, opt, cx - 155.0, y + 2.0, 310.0));
                            y += 25.0;
                        }
                        Row::Pair(a, b) => {
                            out.push(option_widget(s, a, cx - 155.0, y + 2.0, 150.0));
                            if let Some(b) = b {
                                out.push(option_widget(s, b, cx + 5.0, y + 2.0, 150.0));
                            }
                            y += 25.0;
                        }
                        Row::Buttons(mut a, b) => {
                            a.x = cx - 155.0;
                            a.y = y + 2.0;
                            out.push(a);
                            if let Some(mut b) = b {
                                b.x = cx + 5.0;
                                b.y = y + 2.0;
                                out.push(b);
                            }
                            y += 25.0;
                        }
                    }
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                let folder = match page {
                    Page::Mods => Some(("horizon.mods.open_folder", Act::OpenModFolder)),
                    Page::ShaderPacks => Some(("pack.openFolder", Act::OpenShaderPackFolder)),
                    Page::ShaderOptions => Some(("controls.reset", Act::ShaderReset)),
                    Page::NativePackOptions(i, _) => {
                        Some(("controls.reset", Act::NativePackReset(i)))
                    }
                    _ => None,
                };
                if let Some((label, act)) = folder {
                    out.push(button(cx - 154.0, h - 27.0, 150.0, tr(label), act));
                    out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
                } else {
                    out.push(done(h - 27.0));
                }
            }
        }
        // Layout may change the offset (language centering), or the list may
        // shrink after a resize/search. Correct the rows in this same frame.
        let previous_scroll = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous_scroll - self.scroll;
            }
        }
        self.row_bands = rows
            .bands
            .into_iter()
            .map(|band| band.map(|y| y + previous_scroll - self.scroll))
            .collect();
        (out, top, bottom, content)
    }
    fn clamp_scroll(&mut self, top: f32, bottom: f32, content: f32) {
        let max = (content - (bottom - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }
    /// The page's list as the narrator reads it: Language's
    /// `ObjectSelectionList` ("Selected: <language>"), or the
    /// `ContainerObjectSelectionList` the rows' widgets belong to, with the row
    /// under the mouse (`getEntryAtPosition`: within the row's width).
    fn narrate_list(
        &self,
        widgets: &[Widget],
        key: u64,
        (top, bottom): (f32, f32),
        w: f32,
        cursor: (f32, f32),
    ) {
        use crate::widgets::{ListKind, Narratable, Row as Narrated};
        let rect = [0.0, top, w, bottom - top];
        let in_list = cursor.1 >= top && cursor.1 < bottom;
        if self.page() == Page::Language {
            crate::widgets::narratable(|| {
                let rows: Vec<(&Widget, &Text, bool)> = widgets
                    .iter()
                    .filter_map(|wd| match &wd.kind {
                        WKind::LanguageRow {
                            label, selected, ..
                        } => Some((wd, label, *selected)),
                        _ => None,
                    })
                    .collect();
                let narrated = |(wd, label, _): &(&Widget, &Text, bool)| {
                    Narrated::new(
                        wd.row,
                        Text::translate("narrator.select", &[(*label).clone()]).plain(),
                    )
                };
                let hovered = rows
                    .iter()
                    .find(|(wd, ..)| in_list && ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h));
                let selected = rows.iter().find(|(_, _, selected)| *selected);
                Narratable::list(
                    ListKind::Object,
                    rows.len(),
                    hovered.map(narrated),
                    selected.map(narrated),
                    rect,
                )
                .key(key)
            });
            return;
        }
        // OptionsList rows are 310 wide, KeyBindsList's 340.
        let half = if matches!(self.page(), Page::KeyBinds | Page::PadBinds) {
            170.0
        } else {
            155.0
        };
        let hovered_row = (in_list && (cursor.0 - (w / 2.0).floor()).abs() < half)
            .then(|| {
                self.row_bands
                    .iter()
                    .position(|[a, b]| cursor.1 >= *a && cursor.1 < *b)
            })
            .flatten();
        crate::widgets::narratable(|| {
            Narratable::container_list(self.row_bands.len(), hovered_row, rect).key(key)
        });
    }
    pub fn draw(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let (w, h) = (batch.width, batch.height);
        if let Some(error) = &self.resource_error {
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let title = tr("resourcePack.load_fail");
            // An alert: its title and its message.
            crate::widgets::begin_screen(|| {
                crate::widgets::ScreenInfo::new(
                    "options/resource_error",
                    crate::widgets::join_for_narration(&[&title.plain(), error]),
                )
            });
            batch.text_styled(
                atlas,
                &title,
                ((w - atlas.font.width_text(&title)) / 2.0).floor(),
                30.0,
                u32::MAX,
                true,
            );
            let lines = atlas.font.split(&Text::literal(error), (w - 50.0).max(1.0));
            let limit = ((h - 110.0) / 9.0).max(1.0) as usize;
            for (i, line) in lines.iter().take(limit).enumerate() {
                batch.text_styled(
                    atlas,
                    line,
                    ((w - atlas.font.width_text(line)) / 2.0).floor(),
                    60.0 + i as f32 * 9.0,
                    u32::MAX,
                    true,
                );
            }
            ui::button(
                batch,
                atlas,
                w / 2.0 - 100.0,
                h - 28.0,
                200.0,
                20.0,
                &tr("gui.done"),
                true,
                ui::hovered(cursor, w / 2.0 - 100.0, h - 28.0, 200.0, 20.0),
            );
            return;
        }
        // The page is its own screen (OptionsScreen, VideoSettingsScreen...).
        let page = self.page();
        crate::widgets::begin_screen(|| {
            crate::widgets::ScreenInfo::new(
                format!("options/{page:?}"),
                tr(Self::title(page)).plain(),
            )
        });
        if self.page() == Page::HdrCalibration
            && let Some(c) = &self.calibration
        {
            c.draw(batch, atlas, cursor);
            return;
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            self.graphics.binding = self.selected_key;
            self.graphics
                .draw(level, s, batch, atlas, cursor, self.in_world);
            return;
        }
        if self.page() == Page::SplitProfiles {
            self.split_profiles
                .draw(batch, atlas, cursor, self.in_world, self.pad_style);
            return;
        }
        if self.page() == Page::Packs {
            self.watch_data_packs(atlas, (w, h));
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let blink = self.pack_focus.elapsed().as_millis() as u64;
            let m = crate::screen_draw::Measure(&atlas.font);
            let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
            let packs = self.packs(atlas, (w, h));
            let mut draws = packs.draw(&m, mouse, blink);
            packs.bind_icons(&mut draws, atlas, &mut batch.patches);
            crate::screen_draw::paint(batch, atlas, &draws);
            return;
        }
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        self.size = (w, h);
        let in_world = self.in_world;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let list = bottom > top;
        if list {
            ui::list_background(batch, atlas, top, bottom, in_world);
        }
        let visible = |wd: &Widget| !wd.list || (wd.y < bottom && wd.y + wd.h > top);
        // The list's narration: the language list is an ObjectSelectionList,
        // the others ContainerObjectSelectionLists whose rows' widgets are
        // recorded as they are drawn.
        let list_key = crate::widgets::key_of(&format!("options/{page:?}/list"));
        let framebuffer = [(w * batch.scale) as i32, (h * batch.scale) as i32];
        if list {
            self.narrate_list(&widgets, list_key, (top, bottom), w, cursor);
        }
        let mut tooltip = None;
        for (wd_idx, wd) in widgets.iter().enumerate() {
            if !visible(wd) {
                continue;
            }
            let is_focused = self
                .focus
                .as_ref()
                .is_some_and(|fid| matches_focus(fid, wd_idx, wd));
            let start = batch.vertices.len();
            let hover = (ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || cursor.1 >= top && cursor.1 < bottom))
                || is_focused;
            // A pack's option: its label within the widget (the name shortened), and on hover
            // the whole label when it was, then what the pack says of it.
            let shader = s.shader_options.as_ref();
            let option = match &wd.kind {
                WKind::Button {
                    act: Act::ShaderOption(name),
                    ..
                }
                | WKind::ShaderSlider { name, .. } => Some(name),
                _ => None,
            };
            let fitted = shader.zip(option).map(|(o, name)| {
                o.option_label_within(name, wd.w - 8.0, |t| atlas.font.width_text(t))
            });
            if hover && let Some(o) = shader {
                let comment = match (&wd.kind, option) {
                    (_, Some(name)) => o.comment(&format!("option.{name}")),
                    (
                        WKind::Button {
                            act: Act::ShaderScreen(screen),
                            ..
                        },
                        _,
                    ) => o.comment(&format!("screen.{screen}")),
                    (
                        WKind::Button {
                            act: Act::ShaderProfile,
                            ..
                        },
                        _,
                    ) => o.comment("profile"),
                    _ => None,
                };
                let whole = option
                    .filter(|_| fitted.as_ref().is_some_and(|f| f.1))
                    .map(|name| o.option_label(name));
                let lines: Vec<Text> = whole
                    .into_iter()
                    .chain(comment)
                    .flat_map(|t| atlas.font.split(&t, 200.0))
                    .collect();
                if !lines.is_empty() {
                    tooltip = Some(lines);
                }
            }
            if hover && let Some(tip) = tooltip_text(s, wd) {
                tooltip = Some(match &wd.kind {
                    // The duplicate keybinds tooltip keeps its own lines.
                    WKind::Button {
                        act: Act::Key(_), ..
                    } => tip
                        .plain()
                        .split('\n')
                        .map(|l| Text::literal(l.to_owned()))
                        .collect::<Vec<_>>(),
                    _ => atlas.font.split(&tip, 170.0),
                });
            }
            match &wd.kind {
                WKind::Button { label, active, .. } => {
                    let label = fitted.as_ref().map_or(label, |f| &f.0);
                    ui::button(batch, atlas, wd.x, wd.y, wd.w, wd.h, label, *active, hover);
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::Slider { label, value, opt } => {
                    let dragging = self.dragging == Some(*opt);
                    let shown = self.slider_shown(*opt, *value);
                    ui::slider_ext(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        shown,
                        hover || dragging,
                        is_focused,
                        self.can_change_value && is_focused,
                        true,
                        dragging,
                    );
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::ShaderSlider { label, value, name } => {
                    let label = fitted.as_ref().map_or(label, |f| &f.0);
                    let held = self.shader_dragging.as_ref().filter(|(n, _)| n == name);
                    let dragging = held.is_some();
                    ui::slider(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        held.map_or(*value, |h| h.1),
                        hover || dragging,
                        true,
                        dragging,
                    );
                }
                WKind::Text {
                    text,
                    color,
                    centered,
                } => {
                    if text.is_empty() {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, *color);
                    } else if *centered {
                        let tw = atlas.font.width_text(text);
                        // A list's header row (OptionsList.HeaderEntry,
                        // KeyBindsList.CategoryEntry): a FocusableTextWidget
                        // narrating its text, 4 pixels of padding round it.
                        if wd.list && *color == u32::MAX {
                            crate::widgets::narratable(|| {
                                let x = wd.x + ((wd.w - tw) / 2.0).floor();
                                crate::widgets::Narratable::text(
                                    text.plain(),
                                    [x - 4.0, wd.y - 4.0, tw + 8.0, 17.0],
                                )
                                .key(crate::widgets::key_of(&(list_key, wd.row)))
                                .child_of(list_key, wd.row)
                            });
                        }
                        batch.text_styled(
                            atlas,
                            text,
                            wd.x + ((wd.w - tw) / 2.0).floor(),
                            wd.y,
                            *color,
                            true,
                        );
                    } else {
                        batch.text_styled(atlas, text, wd.x, wd.y, *color, true);
                    }
                }
                WKind::LanguageRow {
                    label, selected, ..
                } => {
                    let rect = [wd.x, wd.y, wd.w, wd.h];
                    crate::widgets::record_as(
                        &label.plain(),
                        rect,
                        true,
                        crate::widgets::Kind::Row,
                    );
                    crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    if *selected || is_focused {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, u32::MAX);
                        batch.fill(
                            atlas,
                            wd.x + 1.0,
                            wd.y + 1.0,
                            wd.w - 2.0,
                            wd.h - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    let tw = atlas.font.width_text(label);
                    batch.text_styled(
                        atlas,
                        label,
                        wd.x + ((wd.w - tw) / 2.0).floor(),
                        wd.y + 2.0 + 7.0 - 4.0,
                        u32::MAX,
                        true,
                    );
                }
            }
            if wd.list && (wd.y < top || wd.y + wd.h > bottom) {
                batch.clip_since(start, 0.0, top, w, bottom);
            }
        }
        if list {
            ui::separators(batch, atlas, top, bottom, in_world);
            let sx = self.scrollbar_x(w);
            ui::scrollbar(batch, atlas, sx, top, bottom, content, self.scroll);
        }
        if self.page() == Page::Language {
            let mut search = self.language_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            // LanguageSelectScreen's header search box (no message), added
            // before the list.
            crate::widgets::last_narratable(|n| n.order = -1);
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr("gui.language.search").with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.language_search = search;
        }
        if matches!(self.page(), Page::Video | Page::Controls | Page::KeyBinds) {
            let mut search = self.settings_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            // Horizon's search box sits in the header, before the list.
            crate::widgets::last_narratable(|n| {
                n.order = -1;
                if let crate::widgets::What::EditBox { message, .. } = &mut n.what {
                    *message = tr("horizon.options.search").plain();
                }
            });
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr("horizon.options.search").with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.settings_search = search;
        }
        if let Some(lines) = tooltip {
            ui::tooltip(batch, atlas, &lines, cursor);
        }
    }
    /// Mouse press/drag. Returns events for the game.
    pub fn click(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        size: (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Event> {
        if self.resource_error.is_some() {
            if !drag
                && !right
                && ui::hovered((x, y), size.0 / 2.0 - 100.0, size.1 - 28.0, 200.0, 20.0)
            {
                self.resource_error = None;
                return vec![Event::ButtonClick];
            }
            return Vec::new();
        }
        if self.page() == Page::SplitProfiles {
            use crate::split_profiles::Click;
            if right || drag {
                return Vec::new();
            }
            return match self.split_profiles.click(atlas, (x, y), size) {
                Click::Nothing => Vec::new(),
                Click::Pressed => vec![Event::ButtonClick],
                Click::Done => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.back(s));
                    events
                }
            };
        }
        if self.page() == Page::HdrCalibration
            && let Some(c) = &mut self.calibration
        {
            if right {
                return Vec::new();
            }
            let outcome = c.click((x, y), size, drag);
            return self.calibration_outcome(s, outcome);
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            use crate::graphics_ui::Action;
            // Waiting on a key: a mouse button binds, as on Key Binds.
            if self.selected_key.is_some() && !drag {
                let button = if right {
                    winit::event::MouseButton::Right
                } else {
                    winit::event::MouseButton::Left
                };
                return self.bind_mouse(s, button);
            }
            let mut events = Vec::new();
            for action in self
                .graphics
                .click(level, s, (x, y), size, framebuffer, drag, right)
            {
                match action {
                    Action::ButtonClick => events.push(Event::ButtonClick),
                    Action::Changed => events.push(Event::Changed),
                    Action::SliderDrag => events.push(Event::SliderDrag),
                    Action::OpenAdvanced => self.push(Page::GraphicsAdvanced),
                    Action::OpenHdr => self.push(Page::GraphicsHdr),
                    Action::BindKey(name) => self.selected_key = Some(name),
                    Action::Calibrate => {
                        self.calibration = Some(crate::hdr_calibration::Calibration::new(s));
                        self.push(Page::HdrCalibration);
                    }
                    Action::OpenSimple => {
                        if self.stack.len() >= 2
                            && self.stack[self.stack.len() - 2] == Page::Graphics
                        {
                            events.extend(self.back(s));
                        } else {
                            self.stack.pop();
                            self.push(Page::Graphics);
                        }
                    }
                    Action::Done => events.extend(self.back(s)),
                }
            }
            return events;
        }
        if self.page() == Page::Packs {
            let mut events = Vec::new();
            if drag {
                self.packs(atlas, size).drag(y as f64);
                return events;
            }
            if right {
                return events;
            }
            let m = crate::screen_draw::Measure(&atlas.font);
            let p = self.packs(atlas, size);
            if p.button_at(&m, x as f64, y as f64) {
                events.push(Event::ButtonClick);
            }
            let before = p.search_focused();
            let action = p.click(&m, x as f64, y as f64);
            if !before && p.search_focused() {
                self.pack_focus = Instant::now();
            }
            match action {
                crate::pack_screen::Action::OpenFolder => events.push(match &self.data_pack_dir {
                    Some(dir) => Event::OpenDataPackFolder(dir.clone()),
                    None => Event::OpenPackFolder,
                }),
                crate::pack_screen::Action::Done => events.extend(self.back(s)),
                crate::pack_screen::Action::None => {}
            }
            return events;
        }
        let (widgets, top, bottom, content) = self.layout(s, size.0, size.1);
        self.size = size;
        self.framebuffer = framebuffer;
        let mut events = Vec::new();
        if !drag && matches!(self.page(), Page::Video | Page::Controls | Page::KeyBinds) {
            let search = &mut self.settings_search;
            search.focused = ui::hovered((x, y), search.x, search.y, search.width, search.height);
            if search.focused {
                self.focus = Some(FocusId::Search);
                self.can_change_value = false;
                search.click(atlas, x, false, false);
                self.scroll = 0.0;
                events.push(Event::ButtonClick);
                return events;
            }
        }
        if !drag && self.page() == Page::Language {
            let search = &mut self.language_search;
            search.focused = ui::hovered((x, y), search.x, search.y, search.width, search.height);
            if search.focused {
                self.focus = Some(FocusId::Search);
                self.can_change_value = false;
                search.click(atlas, x, false, false);
                self.scroll = 0.0;
                return events;
            }
        }
        if drag {
            if let Some(previous_y) = self.scrollbar_drag_y {
                if let Some((_, thumb)) = ui::scrollbar_thumb(top, bottom, content, self.scroll) {
                    let max = content - (bottom - top);
                    self.scroll = if y < top {
                        0.0
                    } else if y > bottom {
                        max
                    } else {
                        self.scroll + (y - previous_y) * max / (bottom - top - thumb)
                    };
                    self.clamp_scroll(top, bottom, content);
                    self.scrollbar_drag_y = Some(y);
                } else {
                    self.scrollbar_drag_y = None;
                }
                return events;
            }
            if let Some((name, at)) = &mut self.shader_dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(&wd.kind, WKind::ShaderSlider { name: n, .. } if n == name))
                && let Some(options) = &mut s.shader_options
            {
                *at = ui::slider_value(wd.x, wd.w, x);
                options.set_position(name, *at);
            }
            if let Some(opt) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
            {
                let v = ui::slider_value(wd.x, wd.w, x);
                self.slider_value = Some((opt, v, Instant::now()));
                drag_slider(opt, s, v, &mut events);
            }
            return events;
        }
        // Controller Binds: a click cancels the wait.
        if self.selected_pad_action.take().is_some() {
            return events;
        }
        // Key Binds: a mouse button pressed while waiting binds it.
        if self.selected_key.is_some() {
            let button = if right {
                winit::event::MouseButton::Right
            } else {
                winit::event::MouseButton::Left
            };
            return self.bind_mouse(s, button);
        }
        // AbstractWidget accepts only the primary button. Rebinding above
        // accepts other mouse buttons, but is not a widget activation. A pack's option steps
        // back with the other one.
        if right {
            let name = widgets.iter().find_map(|wd| match &wd.kind {
                WKind::Button {
                    active: true,
                    act: Act::ShaderOption(name),
                    ..
                } if ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h)
                    && !(wd.list && (y < top || y >= bottom)) =>
                {
                    Some(name.clone())
                }
                _ => None,
            });
            if let Some(name) = name
                && let Some(options) = &mut s.shader_options
            {
                options.cycle(&name, true);
                events.push(Event::ButtonClick);
            }
            let native = widgets.iter().find_map(|wd| match wd.kind {
                WKind::Button {
                    active: true,
                    act: Act::NativePackOption(pack, option),
                    ..
                } if ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h)
                    && !(wd.list && (y < top || y >= bottom)) =>
                {
                    Some((pack, option))
                }
                _ => None,
            });
            if let Some((pack, option)) = native
                && let Some(entry) = s.shader_packs.get_mut(pack)
                && entry.step_option(option, true, false)
            {
                events.push(Event::ButtonClick);
                events.push(Event::NativePackOptions(pack));
            }
            return events;
        }
        if ui::scrollbar_thumb(top, bottom, content, self.scroll).is_some()
            && ui::hovered((x, y), self.scrollbar_x(size.0), top, 6.0, bottom - top)
        {
            self.scrollbar_drag_y = Some(y);
            return events;
        }
        for (wd_idx, wd) in widgets.into_iter().enumerate() {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h) {
                continue;
            }
            if wd.list && (y < top || y >= bottom) {
                continue;
            }
            self.focus = Some(focus_id(wd_idx, &wd));
            match wd.kind {
                WKind::Button {
                    active: true, act, ..
                } => {
                    self.can_change_value = false;
                    events.push(Event::ButtonClick);
                    match act {
                        Act::Push(p) => {
                            if p == Page::ShaderPacks {
                                events.push(Event::ListShaderPacks);
                            }
                            self.push(p)
                        }
                        Act::Done => events.extend(self.back(s)),
                        Act::Cycle(opt) => {
                            events.extend(self.cycle_option(s, opt, right, framebuffer));
                        }
                        Act::Extra(item) => {
                            if let Some(name) = item.key_name() {
                                self.selected_key = Some(name);
                            } else {
                                item.cycle(&mut s.extras, right);
                                events.push(Event::Changed);
                            }
                        }
                        Act::Key(name) => self.selected_key = Some(name),
                        Act::ResetKey(name) => {
                            s.keys.set(name, KeyBinds::default_of(name));
                            events.push(Event::Changed);
                        }
                        Act::ResetKeys => {
                            s.keys.reset_all();
                            events.push(Event::Changed);
                        }
                        Act::PadKey(action) => self.selected_pad_action = Some(action),
                        Act::ResetPadKey(action) => {
                            s.controller
                                .bind(action, gamepad::PadSettings::default_of(action));
                            events.push(Event::Changed);
                        }
                        Act::ResetPadKeys => {
                            s.controller.reset_bindings();
                            events.push(Event::Changed);
                        }
                        Act::ToggleMod(i) => {
                            s.mods.toggle(i);
                            events.push(Event::Changed);
                        }
                        Act::ModSettings(i) => {
                            events.push(Event::ButtonClick);
                            self.push(Page::NativeMod(i));
                            events.push(Event::NativeSettings(i));
                        }
                        Act::NativeKey(i, k) => {
                            self.selected_key = Some(NATIVE_KEY);
                            self.selected_native_key = Some((i, k));
                        }
                        Act::NativeButton(i, b) => events.push(Event::NativeButton(i, b)),
                        Act::OpenModFolder => events.push(Event::OpenModFolder),
                        Act::PickShaderPack(i) => {
                            if let Some(pack) = s.shader_packs.get(i) {
                                s.horizon.shader_pack = if s.horizon.shader_pack == pack.file {
                                    String::new()
                                } else {
                                    pack.file.clone()
                                };
                                events.push(Event::ShaderPack);
                            }
                        }
                        Act::OpenShaderPackFolder => events.push(Event::OpenShaderPackFolder),
                        Act::ShaderOptions => {
                            s.shader_options = None;
                            self.push(Page::ShaderOptions);
                            events.push(Event::OpenShaderOptions);
                        }
                        Act::ShaderOption(name) => {
                            if let Some(options) = &mut s.shader_options {
                                if self.shift {
                                    options.reset_option(&name);
                                } else {
                                    options.cycle(&name, false);
                                }
                            }
                        }
                        Act::ShaderScreen(screen) => {
                            if let Some(options) = &mut s.shader_options {
                                options.path.push(screen);
                                self.scroll = 0.0;
                            }
                        }
                        Act::ShaderProfile => {
                            if let Some(options) = &mut s.shader_options {
                                options.next_profile();
                            }
                        }
                        Act::ShaderReset => {
                            if let Some(options) = &mut s.shader_options {
                                options.reset();
                            }
                        }
                        Act::NativePackOptions(i) => {
                            self.push(Page::NativePackOptions(i, 0));
                        }
                        Act::NativePackOption(pack, option) => {
                            if let Some(entry) = s.shader_packs.get_mut(pack)
                                && entry.step_option(option, false, self.shift)
                            {
                                events.push(Event::NativePackOptions(pack));
                            }
                        }
                        Act::NativePackReset(pack) => {
                            if let Some(entry) = s.shader_packs.get_mut(pack) {
                                entry.reset_options();
                                events.push(Event::NativePackOptions(pack));
                            }
                        }
                        Act::Credits => events.push(Event::Credits),
                        Act::CopyAgentDetails => events.push(Event::CopyAgentDetails),
                        Act::None => {}
                    }
                }
                WKind::ShaderSlider { name, .. } => {
                    let at = ui::slider_value(wd.x, wd.w, x);
                    if let Some(options) = &mut s.shader_options {
                        if self.shift {
                            // Sounds as a released slider does: no drag follows.
                            options.reset_option(&name);
                            events.extend([Event::ButtonClick, Event::Changed]);
                            break;
                        }
                        options.set_position(&name, at);
                    }
                    self.shader_dragging = Some((name, at));
                }
                WKind::Slider { opt, .. } => {
                    self.can_change_value = true;
                    self.dragging = Some(opt);
                    let v = ui::slider_value(wd.x, wd.w, x);
                    self.slider_value = Some((opt, v, Instant::now()));
                    drag_slider(opt, s, v, &mut events);
                }
                WKind::LanguageRow { code, .. } => {
                    self.can_change_value = false;
                    if self.language_choice.as_deref() == Some(code.as_str()) {
                        events.extend(self.back(s));
                    } else {
                        self.language_choice = Some(code);
                    }
                }
                _ => {}
            }
            break;
        }
        events
    }
    /// Where a slider's handle sits: its own position while held (or until
    /// a delayed one applies), else the option's stepped value.
    fn slider_shown(&self, opt: Opt, value: f32) -> f32 {
        match self.slider_value {
            Some((o, v, at)) if o == opt && slider_unsnapped(o, self.dragging == Some(o), at) => v,
            _ => value,
        }
    }
    pub fn release(&mut self) -> Vec<Event> {
        self.scrollbar_drag_y = None;
        if let Some(p) = &mut self.pack_screen {
            p.release();
        }
        // AbstractSliderButton suppresses the press sound and plays once on
        // release, even if the pointer has left the widget or its value stayed.
        let slider = self.dragging.take().is_some() | self.shader_dragging.take().is_some();
        let graphics_slider = self.graphics.release();
        let calibration_slider = self.calibration.as_mut().is_some_and(|c| c.release());
        if slider || graphics_slider || calibration_slider {
            vec![Event::ButtonClick, Event::Changed]
        } else {
            Vec::new()
        }
    }
    pub fn scroll(&mut self, s: &Settings, size: (f32, f32), amount: f32) {
        let (_, top, bottom, content) = self.layout(s, size.0, size.1);
        self.scroll -= amount * 18.0;
        self.clamp_scroll(top, bottom, content);
    }
    /// `CycleButton.cycleValue` on an option, then what the option changes:
    /// the preset's values, High Contrast's built-in pack.
    fn cycle_option(
        &mut self,
        s: &mut Settings,
        opt: Opt,
        back: bool,
        framebuffer: [i32; 2],
    ) -> Vec<Event> {
        let mut events = Vec::new();
        opt.cycle(s, back, framebuffer);
        if opt == Opt::Preset {
            apply_preset(s);
        }
        if opt == Opt::HighContrast {
            // Options.highContrast: the built-in pack follows the option.
            let on = s.resource_packs.iter().any(|id| id == "high_contrast");
            if on != s.high_contrast {
                s.resource_packs.retain(|id| id != "high_contrast");
                if s.high_contrast {
                    s.resource_packs.push("high_contrast".into());
                }
                // PackSelectionScreen reads the repository each time it
                // opens: the next one starts from this selection.
                self.pack_selection = s.resource_packs.clone();
                self.pack_screen = None;
                events.push(Event::Packs(s.resource_packs.clone()));
            }
        }
        events.push(Event::Changed);
        if opt == Opt::Narrator {
            events.push(Event::Narrator);
        }
        events
    }
    /// The mouse wheel at the cursor. Over a cycle button it takes the step
    /// (`CycleButton.mouseScrolled`: up goes back) and the list stays put;
    /// over a slider it nudges the slider up or down.
    ///
    /// **Horizon improvement (diverges from Vanilla 26.3):** when a slider or
    /// cycle button is keyboard-focused (highlighted), the wheel steps or
    /// cycles that widget regardless of where the cursor is. Vanilla 26.3
    /// does not intercept the wheel for sliders at all; this makes keyboard
    /// navigation far more usable.
    pub fn wheel(
        &mut self,
        s: &mut Settings,
        size: (f32, f32),
        cursor: (f32, f32),
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Event> {
        self.size = size;
        self.framebuffer = framebuffer;
        if let Some(level) = Self::graphics_level(self.page()) {
            use crate::graphics_ui::Action;
            let actions = self
                .graphics
                .wheel(level, s, cursor, size, framebuffer, amount);
            let mut events = Vec::new();
            for action in actions {
                match action {
                    Action::ButtonClick => events.push(Event::ButtonClick),
                    Action::Changed => events.push(Event::Changed),
                    Action::SliderDrag => events.push(Event::SliderDrag),
                    Action::OpenAdvanced => self.push(Page::GraphicsAdvanced),
                    Action::OpenHdr => self.push(Page::GraphicsHdr),
                    Action::OpenSimple => {
                        if self.stack.len() >= 2
                            && self.stack[self.stack.len() - 2] == Page::Graphics
                        {
                            events.extend(self.back(s));
                        } else {
                            self.stack.pop();
                            self.push(Page::Graphics);
                        }
                    }
                    Action::BindKey(name) => self.selected_key = Some(name),
                    Action::Calibrate => {
                        self.calibration = Some(crate::hdr_calibration::Calibration::new(s));
                        self.push(Page::HdrCalibration);
                    }
                    Action::Done => events.extend(self.back(s)),
                }
            }
            return events;
        }
        let plain = self.resource_error.is_none()
            && !(self.pack_screen.is_some() && self.stack.last() == Some(&Page::Packs));
        if !plain {
            self.scroll_at(s, size, cursor, amount);
            return Vec::new();
        }
        // Horizon improvement: a keyboard-focused slider or cycle button
        // intercepts the wheel before hover detection (see doc-comment above).
        if amount != 0.0 {
            let (widgets, _, _, _) = self.layout(s, size.0, size.1);
            match &self.focus {
                Some(FocusId::Slider(opt)) => {
                    let opt = *opt;
                    let width = widgets
                        .iter()
                        .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
                        .map_or(150.0, |wd| wd.w);
                    return self.step_slider_option(s, opt, amount > 0.0, width);
                }
                Some(FocusId::Cycle(opt)) => {
                    let opt = *opt;
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.cycle_option(s, opt, amount > 0.0, framebuffer));
                    return events;
                }
                _ => {}
            }
        }
        enum HoveredControl {
            Cycle(Opt),
            Extra(crate::extras::Item),
        }
        let (widgets, top, bottom, _) = self.layout(s, size.0, size.1);
        let hovered = widgets.iter().find_map(|wd| match &wd.kind {
            WKind::Button {
                active: true,
                act: Act::Cycle(opt),
                ..
            } if ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && !(wd.list && (cursor.1 < top || cursor.1 >= bottom)) =>
            {
                Some(HoveredControl::Cycle(*opt))
            }
            WKind::Button {
                active: true,
                act: Act::Extra(item),
                ..
            } if ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && !(wd.list && (cursor.1 < top || cursor.1 >= bottom)) =>
            {
                Some(HoveredControl::Extra(*item))
            }
            _ => None,
        });
        match hovered {
            Some(HoveredControl::Cycle(opt)) if amount != 0.0 => {
                self.cycle_option(s, opt, amount > 0.0, framebuffer)
            }
            Some(HoveredControl::Extra(item)) if amount != 0.0 => {
                item.cycle(&mut s.extras, amount > 0.0);
                vec![Event::Changed]
            }
            Some(_) => Vec::new(),
            None => {
                self.scroll_at(s, size, cursor, amount);
                Vec::new()
            }
        }
    }
    /// The mouse wheel at the cursor.
    pub fn scroll_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32), amount: f32) {
        if self.resource_error.is_some() {
            return;
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            self.graphics.scroll(level, s, size.1, amount);
            return;
        }
        match &mut self.pack_screen {
            Some(p) if self.stack.last() == Some(&Page::Packs) => {
                p.scroll(cursor.0 as f64, cursor.1 as f64, amount as f64)
            }
            _ => self.scroll(s, size, amount),
        }
    }
    /// `VideoSettingsScreen.mouseScrolled`: with Ctrl held the wheel steps the GUI scale
    /// (auto counts as one past the maximum) and the list goes back to the top.
    pub fn video_wheel_gui_scale(
        &mut self,
        s: &mut Settings,
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Event> {
        let max = i64::from(crate::window::calculate_scale(
            framebuffer,
            0,
            s.force_unicode,
        ));
        let old = if s.gui_scale == 0 {
            max + 1
        } else {
            i64::from(s.gui_scale)
        };
        let step = i64::from(amount > 0.0) - i64::from(amount < 0.0);
        let new = old + step;
        if new == 0 || new > max {
            return Vec::new();
        }
        s.gui_scale = new as u32;
        self.scroll = 0.0;
        vec![Event::Changed]
    }
    /// The pack screen at this GUI size, made on first use.
    fn packs(&mut self, atlas: &HudAtlas, size: (f32, f32)) -> &mut crate::pack_screen::PackScreen {
        let m = crate::screen_draw::Measure(&atlas.font);
        let (w, h) = (size.0 as i32, size.1 as i32);
        match &mut self.pack_screen {
            Some(p) => {
                if p.width != w || p.height != h {
                    p.resize(&m, w, h);
                }
            }
            None => {
                self.pack_focus = Instant::now();
                let title = if self.data_pack_dir.is_some() {
                    "dataPack.title"
                } else {
                    "resourcePack.title"
                };
                let mut screen = crate::pack_screen::PackScreen::new(
                    &m,
                    crate::text::Text::translate(title, &[]),
                    w,
                    h,
                    self.in_world,
                    self.pack_infos.clone(),
                    &self.pack_selection,
                );
                screen.changed = std::mem::take(&mut self.pack_reselected);
                self.pack_screen = Some(screen);
            }
        }
        self.pack_screen.as_mut().expect("made above")
    }
    /// Escape or Done: leave the current page, applying page results.
    /// After an input on HDR calibration: Save keeps what it landed on, Cancel what was
    /// set before; both go back to Graphics: HDR.
    fn calibration_outcome(
        &mut self,
        s: &mut Settings,
        outcome: crate::hdr_calibration::Outcome,
    ) -> Vec<Event> {
        use crate::hdr_calibration::Outcome;
        match outcome {
            Outcome::Nothing | Outcome::Changed => Vec::new(),
            Outcome::Click => vec![Event::ButtonClick],
            Outcome::Save | Outcome::Cancel => {
                if let Some(c) = self.calibration.take()
                    && outcome == Outcome::Save
                {
                    c.save(s);
                }
                let mut events = vec![Event::ButtonClick];
                events.extend(self.back(s));
                events
            }
        }
    }
    /// HDR calibration's test picture, while its page is open: the step, its nits and
    /// where the controls start (a share of the height `h`).
    pub fn calibration_picture(&self, h: f32) -> Option<(crate::hdr_calibration::Step, f32, f32)> {
        self.calibration
            .as_ref()
            .filter(|_| self.page() == Page::HdrCalibration)
            .map(|c| c.picture(h))
    }
    pub fn back(&mut self, s: &mut Settings) -> Vec<Event> {
        let mut events = Vec::new();
        // A pack's screen goes back to the one that opened it.
        if self.page() == Page::ShaderOptions
            && let Some(options) = &mut s.shader_options
            && options.path.pop().is_some()
        {
            self.scroll = 0.0;
            return events;
        }
        let page = self.stack.pop().unwrap_or(Page::Main);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.dragging = None;
        self.slider_value = None;
        self.selected_key = None;
        self.selected_pad_action = None;
        self.focus = None;
        self.settings_search.focused = false;
        self.language_search.focused = false;
        self.can_change_value = false;
        match page {
            Page::ShaderOptions => {
                if s.shader_options.as_ref().is_some_and(|o| o.changed) {
                    events.push(Event::ShaderOptions);
                } else {
                    s.shader_options = None;
                }
            }
            Page::Language => {
                if let Some(code) = self.language_choice.take()
                    && code != s.language
                {
                    s.language = code.clone();
                    events.push(Event::Language(code));
                }
            }
            Page::Packs => {
                // PackSelectionScreen.onClose: PackSelectionModel.commit.
                let data = self.data_pack_dir.take();
                if let Some(p) = self.pack_screen.take()
                    && p.changed
                {
                    let ids = p.commit();
                    if data.is_some() {
                        events.push(Event::DataPacks(ids));
                    } else {
                        // `PackSelectionModel.commit` already drops `server/…`
                        // ids; keep the same filter if the screen is rebuilt.
                        let ids = crate::packs::player_pack_ids(ids);
                        s.resource_packs = ids.clone();
                        self.pack_selection = ids.clone();
                        events.push(Event::Packs(ids));
                    }
                }
            }
            _ => {}
        }
        events.push(Event::Changed);
        if self.stack.is_empty() {
            events.push(Event::Close);
        }
        events
    }
    /// Typed text (the language search box).
    pub fn type_text(&mut self, atlas: &HudAtlas, text: &str) {
        if self.resource_error.is_some() {
            return;
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            p.type_text(&crate::screen_draw::Measure(&atlas.font), text);
            return;
        }
        if self.page() == Page::Language {
            self.language_search.insert_text(text);
            self.scroll = 0.0;
        } else if self.page() == Page::SplitProfiles {
            self.split_profiles.type_text(text);
        } else if self.settings_search_active() {
            self.settings_search.insert_text(text);
            self.scroll = 0.0;
        }
    }
    /// Ctrl+A/C/V/X in the language or pack search box; false when neither
    /// is shown with the focus.
    pub fn shortcut(
        &mut self,
        atlas: &HudAtlas,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if self.resource_error.is_some() || self.selected_key.is_some() {
            return false;
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            return p.shortcut(&m, shortcut, clipboard);
        }
        if self.page() == Page::Language {
            if self.language_search.shortcut(shortcut, clipboard) {
                self.scroll = 0.0;
            }
            return true;
        }
        if self.page() == Page::SplitProfiles {
            return self.split_profiles.shortcut(shortcut, clipboard);
        }
        if self.settings_search_active() {
            if self.settings_search.shortcut(shortcut, clipboard) {
                self.scroll = 0.0;
            }
            return true;
        }
        false
    }
    fn bind_key(&mut self, s: &mut Settings, value: String) -> Vec<Event> {
        let Some(name) = self.selected_key.take() else {
            return Vec::new();
        };
        if name == NATIVE_KEY {
            let (i, k) = self.selected_native_key.take().unwrap_or_default();
            let Some(m) = s.mods.installed.get(i) else {
                return Vec::new();
            };
            let Some([key, ..]) = m.native.as_ref().and_then(|p| p.keys.get(k)) else {
                return Vec::new();
            };
            let keys = s.mods.keys.entry(m.id.clone()).or_default();
            keys.insert(key.clone(), value);
            return vec![Event::Changed];
        }
        if !s.extras.set_key(name, value.clone()) && !s.horizon.set_key(name, value.clone()) {
            s.keys.set(name, &value);
        }
        vec![Event::Changed]
    }
    /// Mouse bindings use the same settings and persistence as keyboard bindings.
    pub fn bind_mouse(
        &mut self,
        s: &mut Settings,
        button: winit::event::MouseButton,
    ) -> Vec<Event> {
        self.bind_key(s, keybinds::mouse_name(button))
    }
    /// Keys while the options are open. Returns events; `handled` false lets
    /// Escape fall through.
    pub fn key(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
    ) -> Vec<Event> {
        self.key_modifiers(s, atlas, key, text, false)
    }

    /// Keys while the options are open, including Shift modifier state.
    pub fn key_modifiers(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
        shift: bool,
    ) -> Vec<Event> {
        use winit::keyboard::KeyCode as K;
        if self.page() == Page::HdrCalibration
            && let Some(c) = &mut self.calibration
        {
            let outcome = c.key(key);
            return self.calibration_outcome(s, outcome);
        }
        if self.resource_error.is_some() {
            if matches!(key, K::Escape | K::Enter | K::NumpadEnter) {
                self.resource_error = None;
            }
            return Vec::new();
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            match key {
                // ConfirmScreen.keyPressed: Escape answers no.
                K::Escape if p.confirming() => {
                    p.cancel_confirm(&m);
                    return Vec::new();
                }
                K::Backspace => {
                    p.erase(&m, 1);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if let Some(action) = self.selected_pad_action.take() {
            // As on Key Binds, Escape unbinds; other keys cancel.
            if key == K::Escape {
                s.controller.bind(action, None);
            }
            return vec![Event::Changed];
        }
        if self.selected_key.is_some() {
            // KeyBindsScreen.keyPressed: Escape unbinds.
            let value = if key == K::Escape {
                "key.keyboard.unknown".to_owned()
            } else {
                keybinds::key_name(key)
                    .unwrap_or("key.keyboard.unknown")
                    .to_owned()
            };
            return self.bind_key(s, value);
        }
        if self.page() == Page::SplitProfiles && self.split_profiles.key(key, text) {
            return Vec::new();
        }

        if let Some(level) = Self::graphics_level(self.page()) {
            use crate::graphics_ui::Action;
            let size = (self.size.0.max(480.0), self.size.1.max(240.0));
            let fb = [self.framebuffer[0].max(480), self.framebuffer[1].max(240)];
            let (actions, handled) = self.graphics.key_modifiers(level, s, key, shift, size, fb);
            let mut events = Vec::new();
            for action in actions {
                match action {
                    Action::ButtonClick => events.push(Event::ButtonClick),
                    Action::Changed => events.push(Event::Changed),
                    Action::SliderDrag => events.push(Event::SliderDrag),
                    Action::OpenAdvanced => self.push(Page::GraphicsAdvanced),
                    Action::OpenHdr => self.push(Page::GraphicsHdr),
                    Action::OpenSimple => {
                        if self.stack.len() >= 2
                            && self.stack[self.stack.len() - 2] == Page::Graphics
                        {
                            events.extend(self.back(s));
                        } else {
                            self.stack.pop();
                            self.push(Page::Graphics);
                        }
                    }
                    Action::BindKey(name) => self.selected_key = Some(name),
                    Action::Calibrate => {
                        self.calibration = Some(crate::hdr_calibration::Calibration::new(s));
                        self.push(Page::HdrCalibration);
                    }
                    Action::Done => events.extend(self.back(s)),
                }
            }
            if key == K::Escape {
                events.extend(self.back(s));
                return events;
            }
            if handled {
                return events;
            }
            return events;
        }

        if key == K::Escape {
            return self.back(s);
        }

        // Search field text entry when search box is focused
        if self.focus == Some(FocusId::Search) {
            if key != K::Tab && key != K::ArrowDown {
                let search = if self.page() == Page::Language {
                    &mut self.language_search
                } else {
                    &mut self.settings_search
                };
                if !search.key(key, false, shift)
                    && let Some(t) = text
                {
                    let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !typed.is_empty() {
                        search.insert_text(&typed);
                    }
                }
                self.scroll = 0.0;
                return Vec::new();
            }
        } else if self.page() == Page::Language && self.focus.is_none() {
            if !self.language_search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.language_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        } else if self.settings_search_active() && self.focus.is_none() {
            if !self.settings_search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.settings_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        }

        // Gather focusables on the current page
        let (widgets, top, bottom, content) = self.layout(s, self.size.0, self.size.1);
        let has_search = (self.page() == Page::Language) || self.settings_search_active();
        let mut focusables: Vec<(FocusId, [f32; 4], bool)> = Vec::new();
        if has_search {
            let search = if self.page() == Page::Language {
                &self.language_search
            } else {
                &self.settings_search
            };
            focusables.push((
                FocusId::Search,
                [search.x, search.y, search.width, search.height],
                false,
            ));
        }
        for (i, wd) in widgets.iter().enumerate() {
            match wd.kind {
                WKind::Button { active: true, .. }
                | WKind::Slider { .. }
                | WKind::LanguageRow { .. } => {
                    focusables.push((focus_id(i, wd), [wd.x, wd.y, wd.w, wd.h], wd.list));
                }
                _ => {}
            }
        }
        if focusables.is_empty() {
            return Vec::new();
        }

        // 1. Tab / Shift+Tab navigation
        if key == K::Tab {
            let at = self
                .focus
                .as_ref()
                .and_then(|fid| focusables.iter().position(|(id, ..)| id == fid));
            let next_idx = match at {
                None => {
                    if shift {
                        focusables.len() - 1
                    } else {
                        0
                    }
                }
                Some(p) => {
                    if shift {
                        (p + focusables.len() - 1) % focusables.len()
                    } else {
                        (p + 1) % focusables.len()
                    }
                }
            };
            let (next_id, rect, in_list) = &focusables[next_idx];
            self.focus = Some(next_id.clone());
            self.can_change_value = matches!(next_id, FocusId::Slider(_));
            if *in_list {
                if rect[1] < top {
                    self.scroll -= (top - rect[1]) + 4.0;
                } else if rect[1] + rect[3] > bottom {
                    self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                }
                self.clamp_scroll(top, bottom, content);
            }
            return Vec::new();
        }

        // 2. Left / Right Arrow keys
        if matches!(key, K::ArrowLeft | K::ArrowRight) {
            let forward = key == K::ArrowRight;
            // If focused on a slider and in can_change_value mode: adjust the slider value!
            if let Some(FocusId::Slider(opt)) = self.focus
                && self.can_change_value
            {
                let width = widgets
                    .iter()
                    .find(|wd| matches!(wd.kind, WKind::Slider { opt: o, .. } if o == opt))
                    .map_or(150.0, |wd| wd.w);
                return self.step_slider_option(s, opt, forward, width);
            }
            // Otherwise, navigate horizontally using 2D focus stepping
            if let Some(fid) = &self.focus {
                if let Some(cur_pos) = focusables.iter().position(|(id, ..)| id == fid) {
                    let from = focusables[cur_pos].1;
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    let dir = if forward {
                        gamepad::Button::DPadRight
                    } else {
                        gamepad::Button::DPadLeft
                    };
                    if let Some(next_idx) = gamepad::focus_step_rect(from, dir, &rects) {
                        let (next_id, rect, in_list) = &focusables[next_idx];
                        self.focus = Some(next_id.clone());
                        self.can_change_value = false;
                        if *in_list {
                            if rect[1] < top {
                                self.scroll -= (top - rect[1]) + 4.0;
                            } else if rect[1] + rect[3] > bottom {
                                self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                            }
                            self.clamp_scroll(top, bottom, content);
                        }
                    }
                }
            } else {
                self.focus = Some(focusables[0].0.clone());
                self.can_change_value = false;
            }
            return Vec::new();
        }

        // 3. Up / Down Arrow keys
        if matches!(key, K::ArrowUp | K::ArrowDown) {
            let down = key == K::ArrowDown;
            if let Some(fid) = &self.focus {
                if let Some(cur_pos) = focusables.iter().position(|(id, ..)| id == fid) {
                    let from = focusables[cur_pos].1;
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    let dir = if down {
                        gamepad::Button::DPadDown
                    } else {
                        gamepad::Button::DPadUp
                    };
                    let step = gamepad::focus_step_strict(from, dir, &rects)
                        .or_else(|| gamepad::focus_step_vague(from, dir, &rects));
                    if let Some(next_idx) = step {
                        let (next_id, rect, in_list) = &focusables[next_idx];
                        self.focus = Some(next_id.clone());
                        self.can_change_value = false;
                        if *in_list {
                            if rect[1] < top {
                                self.scroll -= (top - rect[1]) + 4.0;
                            } else if rect[1] + rect[3] > bottom {
                                self.scroll += (rect[1] + rect[3] - bottom) + 4.0;
                            }
                            self.clamp_scroll(top, bottom, content);
                        }
                    } else {
                        // Off the edge of visible rows: scroll the list
                        self.scroll += if down { 18.0 } else { -18.0 };
                        self.clamp_scroll(top, bottom, content);
                    }
                }
            } else {
                self.focus = Some(focusables[0].0.clone());
                self.can_change_value = false;
            }
            return Vec::new();
        }

        // 4. Enter / Space selection keys
        if matches!(key, K::Enter | K::NumpadEnter | K::Space) {
            match &self.focus {
                Some(FocusId::Slider(_)) => {
                    self.can_change_value = !self.can_change_value;
                    return Vec::new();
                }
                Some(FocusId::Cycle(opt)) => {
                    let mut events = vec![Event::ButtonClick];
                    events.extend(self.cycle_option(s, *opt, shift, self.framebuffer));
                    return events;
                }
                Some(FocusId::Act(act)) => {
                    let mut events = vec![Event::ButtonClick];
                    let act = act.clone();
                    match act {
                        Act::Push(p) => {
                            self.focus = None;
                            self.can_change_value = false;
                            if p == Page::ShaderPacks {
                                events.push(Event::ListShaderPacks);
                            }
                            self.push(p);
                        }
                        Act::Done => {
                            self.focus = None;
                            self.can_change_value = false;
                            events.extend(self.back(s));
                        }
                        Act::Cycle(opt) => {
                            events.extend(self.cycle_option(s, opt, shift, self.framebuffer));
                        }
                        Act::Extra(item) => {
                            if let Some(name) = item.key_name() {
                                self.selected_key = Some(name);
                            } else {
                                item.cycle(&mut s.extras, shift);
                                events.push(Event::Changed);
                            }
                        }
                        Act::Key(name) => self.selected_key = Some(name),
                        Act::ResetKey(name) => {
                            s.keys.set(name, KeyBinds::default_of(name));
                            events.push(Event::Changed);
                        }
                        Act::ResetKeys => {
                            s.keys.reset_all();
                            events.push(Event::Changed);
                        }
                        Act::PadKey(action) => self.selected_pad_action = Some(action),
                        Act::ResetPadKey(action) => {
                            s.controller
                                .bind(action, gamepad::PadSettings::default_of(action));
                            events.push(Event::Changed);
                        }
                        Act::ResetPadKeys => {
                            s.controller.reset_bindings();
                            events.push(Event::Changed);
                        }
                        Act::ToggleMod(i) => {
                            s.mods.toggle(i);
                            events.push(Event::Changed);
                        }
                        Act::ModSettings(i) => {
                            self.focus = None;
                            self.can_change_value = false;
                            self.push(Page::NativeMod(i));
                            events.push(Event::NativeSettings(i));
                        }
                        Act::NativeKey(i, k) => {
                            self.selected_key = Some(NATIVE_KEY);
                            self.selected_native_key = Some((i, k));
                        }
                        Act::NativeButton(i, b) => events.push(Event::NativeButton(i, b)),
                        Act::OpenModFolder => events.push(Event::OpenModFolder),
                        Act::PickShaderPack(i) => {
                            if let Some(pack) = s.shader_packs.get(i) {
                                s.horizon.shader_pack = if s.horizon.shader_pack == pack.file {
                                    String::new()
                                } else {
                                    pack.file.clone()
                                };
                                events.push(Event::ShaderPack);
                            }
                        }
                        Act::OpenShaderPackFolder => events.push(Event::OpenShaderPackFolder),
                        // The shader options page's buttons, as a click does them.
                        Act::ShaderOptions => {
                            self.focus = None;
                            self.can_change_value = false;
                            s.shader_options = None;
                            self.push(Page::ShaderOptions);
                            events.push(Event::OpenShaderOptions);
                        }
                        Act::ShaderOption(name) => {
                            if let Some(options) = &mut s.shader_options {
                                if shift {
                                    options.reset_option(&name);
                                } else {
                                    options.cycle(&name, false);
                                }
                            }
                        }
                        Act::ShaderScreen(screen) => {
                            if let Some(options) = &mut s.shader_options {
                                self.focus = None;
                                options.path.push(screen);
                                self.scroll = 0.0;
                            }
                        }
                        Act::ShaderProfile => {
                            if let Some(options) = &mut s.shader_options {
                                options.next_profile();
                            }
                        }
                        Act::ShaderReset => {
                            if let Some(options) = &mut s.shader_options {
                                options.reset();
                            }
                        }
                        Act::NativePackOptions(i) => {
                            self.push(Page::NativePackOptions(i, 0));
                        }
                        Act::NativePackOption(pack, option) => {
                            if let Some(entry) = s.shader_packs.get_mut(pack)
                                && entry.step_option(option, false, shift)
                            {
                                events.push(Event::NativePackOptions(pack));
                            }
                        }
                        Act::NativePackReset(pack) => {
                            if let Some(entry) = s.shader_packs.get_mut(pack) {
                                entry.reset_options();
                                events.push(Event::NativePackOptions(pack));
                            }
                        }
                        Act::Credits => events.push(Event::Credits),
                        Act::CopyAgentDetails => events.push(Event::CopyAgentDetails),
                        Act::None => {}
                    }
                    return events;
                }
                Some(FocusId::Lang(code)) => {
                    let mut events = vec![Event::ButtonClick];
                    if self.language_choice.as_deref() == Some(code.as_str()) {
                        events.extend(self.back(s));
                    } else {
                        self.language_choice = Some(code.clone());
                    }
                    return events;
                }
                _ => {}
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_sliders_and_toggles_survive_options_round_trip() {
        let mut s = Settings::default();
        for opt in [
            Opt::FovEffects,
            Opt::DarknessPulsing,
            Opt::DamageTilt,
            Opt::PanoramaSpeed,
        ] {
            for position in [0.0, 0.5, 1.0] {
                opt.set_slider(&mut s, position);
                let loaded = Settings::parse(&s.serialize());
                assert_eq!(opt.slider_position(&loaded), Some(position));
                let expected = if matches!(opt, Opt::FovEffects | Opt::DarknessPulsing) {
                    f64::from(position).powi(2)
                } else {
                    f64::from(position)
                };
                assert_eq!(loaded.file.double(opt.file_key().unwrap()), expected);
            }
        }
        assert!(Opt::MonochromeLogo.available());
        for opt in [
            Opt::TextBackground,
            Opt::HideLightningFlashes,
            Opt::HideSplashTexts,
            Opt::MonochromeLogo,
        ] {
            let key = opt.file_key().unwrap();
            let before = s.file.bool(key);
            opt.cycle(&mut s, false, [854, 480]);
            assert_eq!(Settings::parse(&s.serialize()).file.bool(key), !before);
        }
        Opt::TextBackground.cycle(&mut s, false, [854, 480]);
        let chat_only = s.text_background(0.0);
        Opt::TextBackground.cycle(&mut s, false, [854, 480]);
        assert_ne!(s.text_background(0.0), chat_only);
    }

    #[test]
    fn mouse_settings_match_java_26_3_option_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Mouse);
        let mut options = Vec::new();
        for row in ui.list_rows(&Settings::default()) {
            if let Row::Pair(a, b) = row {
                options.push(a);
                if let Some(b) = b {
                    options.push(b);
                }
            }
        }
        assert_eq!(
            options,
            vec![
                Opt::Sensitivity,
                Opt::WheelSensitivity,
                Opt::DiscreteScroll,
                Opt::InvertX,
                Opt::InvertY,
                Opt::AllowCursorChanges,
            ]
        );
    }

    #[test]
    fn mouse_settings_round_trip_with_java_defaults() {
        let mut settings = Settings::default();
        assert!(!settings.invert_x);
        assert!(settings.allow_cursor_changes);
        Opt::InvertX.cycle(&mut settings, false, [854, 480]);
        Opt::AllowCursorChanges.cycle(&mut settings, false, [854, 480]);
        let loaded = Settings::parse(&settings.serialize());
        assert!(loaded.invert_x);
        assert!(!loaded.allow_cursor_changes);
        assert!(!loaded.file.bool("allowCursorChanges"));
    }

    #[test]
    fn accessibility_has_all_options_and_disabled_placeholders() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Accessibility);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 1200.0);
        let options: Vec<_> = widgets
            .iter()
            .filter_map(|w| match w.kind {
                WKind::Button {
                    act: Act::Cycle(opt),
                    ..
                }
                | WKind::Slider { opt, .. } => Some(opt),
                _ => None,
            })
            .collect();
        assert_eq!(options, ACCESSIBILITY_OPTIONS);
        for &opt in ACCESSIBILITY_OPTIONS {
            let widget = widgets
                .iter()
                .find(|w| match w.kind {
                    WKind::Button {
                        act: Act::Cycle(o), ..
                    }
                    | WKind::Slider { opt: o, .. } => o == opt,
                    _ => false,
                })
                .expect("every accessibility option has a control");
            if !opt.available() {
                assert!(matches!(widget.kind, WKind::Button { active: false, .. }));
                let before = s.serialize();
                opt.cycle(&mut s, false, [854, 480]);
                opt.set_slider(&mut s, 0.0);
                assert_eq!(s.serialize(), before);
            }
        }
        assert!(widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::Push(Page::Controls),
                active: true,
                ..
            }
        )));
    }

    #[test]
    fn the_narrator_option_cycles_its_four_statuses_and_announces_them() {
        use horizon_narrator::Status;
        let mut s = Settings::default();
        for want in [Status::All, Status::Chat, Status::System, Status::Off] {
            Opt::Narrator.cycle(&mut s, false, [854, 480]);
            assert_eq!(s.narrator(), want);
        }
        Opt::Narrator.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.narrator(), Status::System);
        let mut ui = OptionsUi::default();
        assert_eq!(
            ui.cycle_option(&mut s, Opt::Narrator, false, [854, 480]),
            [Event::Changed, Event::Narrator]
        );
        assert_eq!(
            ui.cycle_option(&mut s, Opt::NarratorHotkey, false, [854, 480]),
            [Event::Changed]
        );
        assert!(!s.file.bool("narratorHotkey"));
    }

    #[test]
    fn without_speech_the_narrator_button_is_inactive_and_not_available() {
        // Tests never open the narrator: text2speech's Narrator.EMPTY.
        let s = Settings::default();
        let widget = option_widget(&s, Opt::Narrator, 0.0, 0.0, 150.0);
        assert!(matches!(widget.kind, WKind::Button { active: false, .. }));
        assert_eq!(
            Opt::Narrator.value_text(&s).plain(),
            tr("options.narrator.notavailable").plain()
        );
        let hotkey = option_widget(&s, Opt::NarratorHotkey, 0.0, 0.0, 150.0);
        assert!(matches!(hotkey.kind, WKind::Button { active: true, .. }));
    }

    #[test]
    fn chat_settings_show_the_narrator_where_vanilla_does() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Chat);
        let options: Vec<Opt> = ui
            .list_rows(&Settings::default())
            .into_iter()
            .flat_map(|row| match row {
                Row::Big(opt) => vec![opt],
                Row::Pair(a, b) => std::iter::once(a).chain(b).collect(),
                _ => Vec::new(),
            })
            .collect();
        let at = options.iter().position(|o| *o == Opt::Narrator).unwrap();
        assert_eq!(options[at - 1], Opt::ChatHeightUnfocused);
        assert_eq!(options[at + 1], Opt::AutoSuggestions);
    }

    #[test]
    fn controls_match_java_26_3_option_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Controls);
        let options =
            ui.list_rows(&Settings::default())
                .into_iter()
                .fold(Vec::new(), |mut options, row| {
                    match row {
                        Row::Big(opt) => options.push(opt),
                        Row::Pair(a, b) => {
                            options.push(a);
                            options.extend(b);
                        }
                        _ => {}
                    }
                    options
                });
        assert_eq!(options, vanilla_control_options());
    }

    #[test]
    fn sprint_window_and_new_control_switches_round_trip() {
        let mut settings = Settings::default();
        set_slider(Opt::SprintWindow, &mut settings, 0.5);
        settings.operator_items_tab = true;
        settings.quit_shortcuts = false;
        settings.ctrl_click_emulates_right_click = true;
        settings.toggle_attack = true;
        settings.toggle_use = true;
        let loaded = Settings::parse(&settings.serialize());
        assert_eq!(loaded.sprint_window, 5);
        assert!(loaded.operator_items_tab);
        assert!(!loaded.quit_shortcuts);
        assert!(loaded.ctrl_click_emulates_right_click);
        assert!(loaded.toggle_attack && loaded.toggle_use);
    }

    #[test]
    fn quit_shortcuts_option_can_be_switched() {
        assert!(Opt::QuitShortcuts.available());
        let s = Settings::default();
        assert_eq!(Opt::QuitShortcuts.caption(), "options.quitShortcuts");
        assert_eq!(
            Opt::QuitShortcuts.tooltip_key(&s),
            "options.quitShortcuts.tooltip"
        );
    }

    #[test]
    fn operator_items_tab_option_can_be_switched() {
        // The creative screen reads it (CreativeModeInventoryScreen.hasPermissions).
        assert!(Opt::OperatorItemsTab.available());
        let mut s = Settings::default();
        assert!(!s.operator_items_tab);
        Opt::OperatorItemsTab.cycle(&mut s, false, [854, 480]);
        assert!(s.operator_items_tab);
    }

    #[test]
    fn render_distance_slider_round_trips_through_extended_options_range() {
        let mut s = Settings::default();
        for (position, distance) in [(0.0, 2), (0.5, 65), (1.0, 128)] {
            set_slider(Opt::RenderDistance, &mut s, position);
            assert_eq!(s.render_distance, distance);
            let reloaded = Settings::parse(&s.serialize());
            assert_eq!(reloaded.render_distance, distance);
            assert_eq!(
                Opt::RenderDistance.slider_position(&reloaded),
                Some(position)
            );
        }
        let mut file = crate::options_file::OptionsFile::default();
        file.set_text("renderDistance", "128").unwrap();
        assert!(file.set_text("renderDistance", "129").is_err());
        assert!(file.set_text("renderDistance", "1").is_err());
        assert_eq!(file.int("renderDistance"), 128);
    }
    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    #[test]
    fn a_horizon_packs_options_step_both_ways_from_the_picked_packs_button() {
        let atlas = atlas();
        let size = (960.0, 540.0);
        let mut s = Settings::default();
        s.horizon.shader_pack = "p".into();
        s.shader_packs = vec![crate::shader_pack_list::Entry {
            file: "p".into(),
            title: "P".into(),
            kind: crate::shader_pack_list::Kind::Horizon,
            options: vec![horizon_shader_api::PackOption {
                id: "a".into(),
                name: "A".into(),
                description: String::new(),
                kind: horizon_shader_api::OptionKind::Float {
                    default: 1.0,
                    min: 0.0,
                    max: 2.0,
                    step: 0.5,
                },
            }],
            values: vec![1.0],
            pages: vec![
                horizon_shader_api::OptionPage {
                    id: "main".into(),
                    name: "P".into(),
                    description: String::new(),
                    columns: 2,
                    items: vec![horizon_shader_api::PageItem::Page(1)],
                },
                horizon_shader_api::OptionPage {
                    id: "more".into(),
                    name: "More".into(),
                    description: String::new(),
                    columns: 1,
                    items: vec![horizon_shader_api::PageItem::Option(0)],
                },
            ],
        }];
        let mut ui = OptionsUi::default();
        ui.show(Page::ShaderPacks);
        let find = |ui: &mut OptionsUi, s: &Settings, want: &Act| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            let w = widgets
                .into_iter()
                .find(|w| matches!(&w.kind, WKind::Button { act, .. } if act == want))
                .expect("button");
            (w.x + w.w / 2.0, w.y + w.h / 2.0)
        };
        let (x, y) = find(&mut ui, &s, &Act::NativePackOptions(0));
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert_eq!(ui.page(), Page::NativePackOptions(0, 0));
        // The option is on the pack's second page, behind a button on its first.
        let (x, y) = find(&mut ui, &s, &Act::Push(Page::NativePackOptions(0, 1)));
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert_eq!(ui.page(), Page::NativePackOptions(0, 1));
        let (x, y) = find(&mut ui, &s, &Act::NativePackOption(0, 0));
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert!(events.contains(&Event::NativePackOptions(0)));
        assert_eq!(s.shader_packs[0].values, [1.5]);
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true);
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true);
        assert_eq!(s.shader_packs[0].values, [0.5]);
        // Shift-click: the option's default.
        ui.shift = true;
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        ui.shift = false;
        assert_eq!(s.shader_packs[0].values, [1.0]);
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        let (x, y) = find(&mut ui, &s, &Act::NativePackReset(0));
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        assert_eq!(s.shader_packs[0].values, [1.0]);
        ui.back(&mut s);
        assert_eq!(ui.page(), Page::NativePackOptions(0, 0));
    }

    #[test]
    fn resource_reload_error_blocks_options_until_dismissed() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut settings = Settings::default();
        let mut ui = OptionsUi {
            resource_error: Some("broken pack".into()),
            ..Default::default()
        };
        let stack = ui.stack.clone();
        assert!(
            ui.click(
                &mut settings,
                &atlas,
                10.0,
                10.0,
                (480.0, 270.0),
                [960, 540],
                false,
                false
            )
            .is_empty()
        );
        assert!(ui.resource_error.is_some());
        assert!(
            ui.key(&mut settings, &atlas, KeyCode::KeyW, None)
                .is_empty()
        );
        assert!(ui.resource_error.is_some());
        ui.key(&mut settings, &atlas, KeyCode::Escape, None);
        assert!(ui.resource_error.is_none());
        assert_eq!(ui.stack, stack);
        ui.resource_error = Some("broken pack".into());
        assert_eq!(
            ui.click(
                &mut settings,
                &atlas,
                240.0,
                250.0,
                (480.0, 270.0),
                [960, 540],
                false,
                false
            ),
            vec![Event::ButtonClick]
        );
        assert!(ui.resource_error.is_none());
    }

    #[test]
    fn classic_mods_page_toggles_mods_and_keeps_dev_and_invalid_entries_inactive() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        assert!(ui.layout(&s, size.0, size.1).0.iter().any(|w| {
            matches!(
                w.kind,
                WKind::Button {
                    active: true,
                    act: Act::Push(Page::Mods),
                    ..
                }
            )
        }));
        ui.show(Page::Mods);
        let empty = ui.layout(&s, size.0, size.1).0;
        assert!(empty.iter().any(|w| {
            matches!(&w.kind, WKind::Text { text, .. } if text.plain() == "horizon.mods.empty")
        }));
        // The empty page points at the bundled key; the English line behind it
        // is what a player reads, so both halves are checked here.
        let en = crate::lang::Language::load(&[], "en_us");
        assert!(
            en.get("horizon.mods.empty")
                .is_some_and(|line| line.contains(".hmod"))
        );
        let open_folder = empty
            .iter()
            .find(|w| {
                matches!(
                    &w.kind,
                    WKind::Button {
                        act: Act::OpenModFolder,
                        ..
                    }
                )
            })
            .expect("Mods page has an Open Mod Folder button");
        assert_eq!(
            ui.click(
                &mut s,
                &atlas,
                open_folder.x + 1.0,
                open_folder.y + 1.0,
                size,
                [480, 240],
                false,
                false,
            ),
            vec![Event::ButtonClick, Event::OpenModFolder]
        );
        for (id, dev) in [("normal", false), ("dev", true), ("", false)] {
            s.mods.installed.push(crate::mod_list::Installed {
                id: id.into(),
                title: id.into(),
                description: "May show HUD panels.".into(),
                problem: id.is_empty().then(|| "Invalid package".into()),
                dev,
                native: None,
            });
        }
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let rows: Vec<_> = widgets
            .iter()
            .filter(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::ToggleMod(_),
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            let events = ui.click(
                &mut s,
                &atlas,
                row.x + 1.0,
                row.y + 1.0,
                size,
                [480, 240],
                false,
                false,
            );
            if i == 0 {
                assert!(events.contains(&Event::Changed));
                assert!(events.contains(&Event::ButtonClick));
            } else {
                assert!(events.is_empty());
            }
        }
        assert!(s.mods.is_on(0));
        assert!(s.mods.is_on(1));
        assert!(!s.mods.is_on(2));
        assert_eq!(s.mods.serialize(), "mod.normal:true\n");
        let row = rows[0];
        ui.click(
            &mut s,
            &atlas,
            row.x + 1.0,
            row.y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        assert!(!s.mods.is_on(0));
        assert!(!ui.back(&mut s).contains(&Event::Close));
        assert_eq!(ui.page(), Page::Main);
    }

    #[test]
    fn opening_a_native_mods_page_asks_the_game_for_its_labels_again() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        s.mods.installed.push(crate::mod_list::Installed {
            id: "native".into(),
            title: "Native 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                buttons: vec!["Setting".into()],
                ..Default::default()
            }),
        });
        ui.show(Page::Mods);
        let (widgets, ..) = ui.layout(&s, size.0, size.1);
        let settings = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::ModSettings(0),
                        ..
                    }
                )
            })
            .expect("a native mod's row has a Settings button");
        let events = ui.click(
            &mut s,
            &atlas,
            settings.x + 1.0,
            settings.y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        // Its labels were made with the list, maybe in another language.
        assert_eq!(events.last(), Some(&Event::NativeSettings(0)));
        assert_eq!(ui.page(), Page::NativeMod(0));
    }

    #[test]
    fn native_mod_without_settings_has_only_a_full_width_toggle() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        s.mods.installed.push(crate::mod_list::Installed {
            id: "no_settings".into(),
            title: "Native 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage::default()),
        });
        ui.show(Page::Mods);
        let (widgets, ..) = ui.layout(&s, 480.0, 240.0);
        assert!(!widgets.iter().any(|w| matches!(
            w.kind,
            WKind::Button {
                act: Act::ModSettings(_),
                ..
            }
        )));
        let toggle = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::ToggleMod(0),
                        ..
                    }
                )
            })
            .unwrap();
        assert_eq!(toggle.w, 310.0);
        let events = ui.click(
            &mut s,
            &atlas,
            toggle.x + 1.0,
            toggle.y + 1.0,
            (480.0, 240.0),
            [480, 240],
            false,
            false,
        );
        assert!(events.contains(&Event::Changed));
        assert!(!s.mods.is_on(0));
        assert!(s.mods.disabled.contains("no_settings"));
    }

    #[test]
    fn a_native_mods_key_shows_and_keeps_the_key_pressed() {
        // #3539: Zoom's key set to R showed another key.
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 240.0);
        s.mods.installed.push(crate::mod_list::Installed {
            id: "zoom".into(),
            title: "Zoom 2.0.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(crate::mod_list::NativePage {
                keys: vec![["zoom".into(), "Zoom key".into(), "key.keyboard.z".into()]],
                buttons: Vec::new(),
            }),
        });
        ui.show(Page::NativeMod(0));
        let key_label = |ui: &mut OptionsUi, s: &Settings| {
            let (widgets, ..) = ui.layout(s, size.0, size.1);
            widgets.into_iter().find_map(|w| match w.kind {
                WKind::Button {
                    label,
                    act: Act::NativeKey(0, 0),
                    ..
                } => Some((label.plain(), w.x, w.y)),
                _ => None,
            })
        };
        let (label, x, y) = key_label(&mut ui, &s).expect("the key's row");
        assert_eq!(label, "Z");
        ui.click(
            &mut s,
            &atlas,
            x + 1.0,
            y + 1.0,
            size,
            [480, 240],
            false,
            false,
        );
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::KeyR, Some("r")),
            vec![Event::Changed]
        );
        assert_eq!(key_label(&mut ui, &s).unwrap().0, "R");
        let saved = crate::mod_list::ModList::parse(&s.mods.serialize());
        assert_eq!(saved.keys_of("zoom"), vec![("zoom", "key.keyboard.r")]);
    }

    fn mode(width: i32, rate: f32) -> crate::window::VideoMode {
        crate::window::VideoMode {
            width,
            height: width * 9 / 16,
            red_bits: 8,
            green_bits: 8,
            blue_bits: 8,
            refresh_rate: rate,
        }
    }

    #[test]
    fn video_page_lists_vanilla_display_and_quality_options_in_order() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let rows = ui.list_rows(&Settings::default());
        let opts: Vec<Opt> = rows
            .iter()
            .flat_map(|r| match r {
                Row::Big(o) => vec![*o],
                Row::Pair(a, b) => std::iter::once(*a).chain(*b).collect(),
                _ => Vec::new(),
            })
            .collect();
        // VideoSettingsScreen.addOptions: the exclusive mode row, displayOptions, the preset,
        // qualityOptions, then the preferences.
        assert_eq!(
            opts[..9],
            [
                Opt::ExclusiveMode,
                Opt::MaxFps,
                Opt::Vsync,
                Opt::InactivityFps,
                Opt::GuiScale,
                Opt::Fullscreen,
                Opt::ExclusiveFullscreen,
                Opt::Gamma,
                Opt::GraphicsBackend,
            ]
        );
        let at = |o| opts.iter().position(|x| *x == o).unwrap();
        assert_eq!(at(Opt::ImprovedTransparency) + 1, at(Opt::TextureFiltering));
        assert_eq!(at(Opt::TextureFiltering) + 1, at(Opt::MaxAnisotropy));
        assert_eq!(at(Opt::MaxAnisotropy) + 1, at(Opt::WeatherRadius));
    }

    #[test]
    fn exclusive_fullscreen_mode_picks_a_monitor_mode_or_the_current_one() {
        let mut s = Settings::default();
        // No monitor mode: the slider changes nothing.
        Opt::ExclusiveMode.set_slider(&mut s, 1.0);
        assert_eq!(s.file.fullscreen_resolution, None);
        s.video_modes = vec![mode(1280, 60.0), mode(1920, 144.0), mode(2560, 59.94)];
        assert_eq!(s.video_mode_index(), -1);
        // IntRange(-1, 2): the slider's thirds are -1, 0, 1, 2.
        Opt::ExclusiveMode.set_slider(&mut s, 2.0 / 3.0);
        assert_eq!(s.video_mode_index(), 1);
        assert_eq!(Opt::ExclusiveMode.slider_position(&s), Some(2.0 / 3.0));
        Opt::ExclusiveMode.set_slider(&mut s, 1.0);
        assert_eq!(s.video_mode_index(), 2);
        Opt::ExclusiveMode.set_slider(&mut s, 0.0);
        assert_eq!(s.file.fullscreen_resolution, None);
    }

    #[test]
    fn exclusive_fullscreen_and_its_tooltips_follow_the_option() {
        let mut s = Settings::default();
        assert!(!s.file.bool("exclusiveFullscreen"));
        assert_eq!(
            Opt::ExclusiveFullscreen.tooltip_key(&s),
            "options.exclusiveFullscreen.off.tooltip"
        );
        Opt::ExclusiveFullscreen.cycle(&mut s, false, [854, 480]);
        assert!(s.file.bool("exclusiveFullscreen"));
        assert_eq!(
            Opt::ExclusiveFullscreen.tooltip_key(&s),
            "options.exclusiveFullscreen.on.tooltip"
        );
        // The mode row's own tooltip says it only matters with Exclusive Fullscreen.
        assert_eq!(
            Opt::ExclusiveMode.tooltip_key(&s),
            "options.fullscreen.exclusive.mode.tooltip"
        );
    }

    #[test]
    fn the_graphics_api_option_cycles_default_opengl_vulkan_and_asks_for_a_restart() {
        use crate::graphics::GraphicsApi;
        let mut s = Settings::default();
        s.startup_api = s.horizon.api;
        let offered = GraphicsApi::choices();
        let mut seen = vec![s.horizon.api];
        for _ in 1..offered.len() {
            Opt::GraphicsBackend.cycle(&mut s, false, [854, 480]);
            seen.push(s.horizon.api);
        }
        // Dx12 is Horizon's own Graphics page's; this option has the three of vanilla.
        assert!(!seen.contains(&GraphicsApi::Dx12));
        assert_eq!(seen[0], GraphicsApi::Auto);
        if offered.len() == 3 {
            assert_eq!(seen[1..], [GraphicsApi::OpenGl, GraphicsApi::Vulkan]);
            assert_eq!(s.file.name("preferredGraphicsBackend"), "vulkan");
            Opt::GraphicsBackend.cycle(&mut s, false, [854, 480]);
            assert_eq!(s.horizon.api, GraphicsApi::Auto);
            assert_eq!(s.file.name("preferredGraphicsBackend"), "default");
        }
    }

    #[test]
    fn texture_filtering_rows_exist_and_the_presets_set_them() {
        let mut s = Settings::default();
        for (preset, bit, filtering) in [("fast", 1, 0), ("fancy", 1, 1), ("fabulous", 2, 2)] {
            s.graphics_preset = preset.into();
            apply_preset(&mut s);
            assert_eq!(s.file.int("maxAnisotropyBit"), bit, "{preset}");
            assert_eq!(s.file.int("textureFiltering"), filtering, "{preset}");
        }
        assert!(Opt::TextureFiltering.available());
        assert!(Opt::MaxAnisotropy.available());
        // The anisotropy slider only works with Anisotropic filtering (VideoSettingsScreen.tick).
        let grey = |s: &Settings| {
            matches!(
                option_widget(s, Opt::MaxAnisotropy, 0.0, 0.0, 150.0).kind,
                WKind::Button { active: false, .. }
            )
        };
        s.file
            .set("textureFiltering", crate::options_file::V::Int(1));
        assert!(grey(&s));
        s.file
            .set("textureFiltering", crate::options_file::V::Int(2));
        assert!(!grey(&s));
    }

    #[test]
    #[ignore = "requires prepared language data in HORIZON_TEST_RESOURCES; run alone"]
    fn the_new_video_options_read_as_vanilla_words_them() {
        use crate::graphics::GraphicsApi;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        lang::set(lang::Language::load(&[&root], "en_us"));
        let mut s = Settings::default();
        assert_eq!(Opt::ExclusiveMode.label(&s).plain(), "Setting unavailable");
        s.video_modes = vec![mode(1280, 60.0), mode(1920, 144.0), mode(2560, 59.94)];
        assert_eq!(
            Opt::ExclusiveMode.label(&s).plain(),
            "Exclusive Fullscreen Mode: Current"
        );
        Opt::ExclusiveMode.set_slider(&mut s, 2.0 / 3.0);
        assert_eq!(
            Opt::ExclusiveMode.label(&s).plain(),
            "Exclusive Fullscreen Mode: 1920x1080@144 (24bit)"
        );
        Opt::ExclusiveMode.set_slider(&mut s, 1.0);
        assert_eq!(
            Opt::ExclusiveMode.label(&s).plain(),
            "Exclusive Fullscreen Mode: 2560x1440@59.94 (24bit)"
        );
        s.horizon.api = GraphicsApi::Vulkan;
        assert_eq!(
            Opt::GraphicsBackend.label(&s).plain(),
            "Graphics API: Prefer Vulkan (Experimental)"
        );
        assert!(
            graphics_api_tooltip(&s)
                .plain()
                .starts_with("Requires a game restart to take effect.\n\n")
        );
        s.startup_api = GraphicsApi::Vulkan;
        assert!(
            graphics_api_tooltip(&s)
                .plain()
                .starts_with("Which graphics API")
        );
        s.file
            .set("maxAnisotropyBit", crate::options_file::V::Int(2));
        s.file
            .set("textureFiltering", crate::options_file::V::Int(2));
        assert_eq!(
            Opt::MaxAnisotropy.label(&s).plain(),
            "Anisotropic Filtering: 4x"
        );
        assert_eq!(
            Opt::TextureFiltering.label(&s).plain(),
            "Texture Filtering: Anisotropic"
        );
    }

    #[test]
    fn option_tooltip_keys_follow_value_dependent_descriptions() {
        let mut s = Settings::default();
        assert_eq!(
            Opt::InactivityFps.tooltip_key(&s),
            "options.inactivityFpsLimit.afk.tooltip"
        );
        s.inactivity_fps_limit = "minimized".into();
        assert_eq!(
            Opt::InactivityFps.tooltip_key(&s),
            "options.inactivityFpsLimit.minimized.tooltip"
        );
        for (value, name) in ["none", "byPlayer", "nearby"].iter().enumerate() {
            s.prioritize_chunk_updates = value as u8;
            assert_eq!(
                Opt::ChunkBuilder.tooltip_key(&s),
                format!("options.prioritizeChunkUpdates.{name}.tooltip")
            );
        }
        assert_eq!(
            Opt::MenuBlur.tooltip_key(&s),
            "options.accessibility.menu_background_blurriness.tooltip"
        );
    }

    #[test]
    #[ignore = "requires prepared language data in HORIZON_TEST_RESOURCES; run alone"]
    fn classic_hover_draws_the_selected_options_wrapped_tooltip() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        lang::set(lang::Language::load(&[&root], "en_us"));
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        for mode in ["afk", "minimized"] {
            s.inactivity_fps_limit = mode.into();
            let (widgets, _, _, _) = ui.layout(&s, 480.0, 480.0);
            let option = widgets
                .iter()
                .find(|w| {
                    matches!(
                        w.kind,
                        WKind::Button {
                            act: Act::Cycle(Opt::InactivityFps),
                            ..
                        }
                    )
                })
                .unwrap();
            let cursor = (option.x + 1.0, option.y + 1.0);
            let mut actual = HudBatch::new([480, 480]);
            ui.draw(&s, &mut actual, &atlas, cursor);
            let key = format!("options.inactivityFpsLimit.{mode}.tooltip");
            assert!(lang::has(&key));
            let mut expected = HudBatch::new([480, 480]);
            ui::tooltip(
                &mut expected,
                &atlas,
                &atlas.font.split(&tr(&key), 170.0),
                cursor,
            );
            assert!(!expected.vertices.is_empty());
            let tail = &actual.vertices[actual.vertices.len() - expected.vertices.len()..];
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(tail),
                bytemuck::cast_slice::<_, u8>(&expected.vertices)
            );
        }
    }

    #[test]
    fn classic_video_search_keeps_only_headings_with_matching_options() {
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let settings = Settings::default();
        let headers = |rows: Vec<Row>| {
            rows.into_iter()
                .filter_map(|row| match row {
                    Row::Header(key) => Some(key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let all_headers = vec![
            "options.video.display.header",
            "options.video.quality.header",
            "options.video.preferences.header",
        ];
        assert_eq!(headers(ui.list_rows(&settings)), all_headers);
        for (opt, header) in [
            (Opt::Fullscreen, all_headers[0]),
            (Opt::RenderDistance, all_headers[1]),
            (Opt::Preset, all_headers[1]),
            (Opt::Autosave, all_headers[2]),
        ] {
            ui.settings_search
                .set_value(&lang::get(opt.caption()).to_uppercase());
            let rows = ui.list_rows(&settings);
            assert_eq!(headers(ui.list_rows(&settings)), vec![header]);
            assert!(rows.iter().any(|row| match row {
                Row::Big(found) => *found == opt,
                Row::Pair(a, b) => *a == opt || *b == Some(opt),
                _ => false,
            }));
            assert!(rows.iter().any(|row| matches!(row, Row::Buttons(..))));
        }
        ui.settings_search.set_value("no matching video setting");
        let rows = ui.list_rows(&settings);
        assert!(headers(ui.list_rows(&settings)).is_empty());
        assert!(rows.iter().all(|row| matches!(row, Row::Buttons(..))));
        ui.settings_search.set_value("");
        assert_eq!(headers(ui.list_rows(&settings)), all_headers);
    }

    #[test]
    fn classic_search_emits_each_matching_heading_once_in_section_order() {
        let mut ui = OptionsUi::default();
        ui.settings_search
            .set_value(&lang::get(Opt::Fullscreen.caption()));
        let rows = ui.filter_option_rows(vec![
            Row::Header("empty"),
            Row::Header("display"),
            Row::Pair(Opt::Fullscreen, Some(Opt::Fullscreen)),
            Row::Big(Opt::Fullscreen),
            Row::Header("quality"),
            Row::Big(Opt::Fullscreen),
            Row::Header("trailing empty"),
        ]);
        assert!(matches!(
            rows.as_slice(),
            [
                Row::Header("display"),
                Row::Pair(Opt::Fullscreen, Some(Opt::Fullscreen)),
                Row::Big(Opt::Fullscreen),
                Row::Header("quality"),
                Row::Big(Opt::Fullscreen),
            ]
        ));
    }

    #[test]
    fn classic_video_gui_scale_cycles_on_press_without_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(Opt::GuiScale),
                        ..
                    }
                )
            })
            .expect("GUI Scale must be a cycle button in classic Video Settings");
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(Opt::GuiScale.slider_position(&s).is_none());
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(s.gui_scale, 0);
        for expected in [1, 2, 0] {
            assert_eq!(
                ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
                vec![Event::ButtonClick, Event::Changed]
            );
            assert_eq!(s.gui_scale, expected);
            assert!(
                ui.click(&mut s, &atlas, x + 100.0, y, size, [854, 480], true, false)
                    .is_empty()
            );
            assert_eq!(s.gui_scale, expected);
            assert!(ui.release().is_empty());
        }
    }

    #[test]
    fn extras_from_horizon_settings_edit_shared_settings_and_return() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        let size = (480.0, 480.0);
        let click = |ui: &mut OptionsUi, s: &mut Settings, act: Act| {
            let (widgets, _, _, _) = ui.layout(s, size.0, size.1);
            let widget = widgets
                .iter()
                .find(
                    |w| matches!(&w.kind, WKind::Button { act: a, active: true, .. } if *a == act),
                )
                .unwrap();
            ui.click(
                s,
                &atlas,
                widget.x + 1.0,
                widget.y + 1.0,
                size,
                [960, 960],
                false,
                false,
            )
        };
        for page in [Page::Horizon, Page::Extras] {
            assert_eq!(
                click(&mut ui, &mut s, Act::Push(page)),
                vec![Event::ButtonClick]
            );
            assert_eq!(ui.page(), page);
        }
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        assert_eq!(
            widgets
                .iter()
                .filter(|w| matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::Extra(_),
                        ..
                    }
                ))
                .count(),
            crate::extras::rows().iter().filter(|r| r.is_ok()).count()
        );
        assert_eq!(
            click(&mut ui, &mut s, Act::Extra(crate::extras::Item::Sort)),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(s.extras.sort);
        assert_eq!(
            click(&mut ui, &mut s, Act::Extra(crate::extras::Item::SortKey)),
            vec![Event::ButtonClick]
        );
        assert_eq!(ui.selected_key, Some(crate::extras::SORT_KEY));
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::KeyC, None),
            vec![Event::Changed]
        );
        assert_eq!(s.extras.sort_key, "key.keyboard.c");
        assert_eq!(
            crate::extras::Extras::parse(&s.extras.serialize()),
            s.extras
        );
        // In a window too short for the page, the wheel scrolls to its end.
        let short = (480.0, 180.0);
        ui.scroll(&s, short, -1000.0);
        let (_, top, bottom, content) = ui.layout(&s, short.0, short.1);
        assert!(content > bottom - top);
        assert_eq!(ui.scroll, content - (bottom - top));
        assert_eq!(
            ui.key(&mut s, &atlas, winit::keyboard::KeyCode::Escape, None),
            vec![Event::Changed]
        );
        assert_eq!(ui.page(), Page::Horizon);
    }

    #[test]
    fn classic_graphics_presets_set_the_distances() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let preset = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Preset,
                        ..
                    }
                )
            })
            .expect("the Graphics preset slider is in classic Video Settings");
        // Dragging the slider across its range (GraphicsPreset.apply on each change).
        let y = preset.y + 1.0;
        let mut seen = Vec::new();
        ui.click(
            &mut s,
            &atlas,
            preset.x + 1.0,
            y,
            size,
            [854, 480],
            false,
            false,
        );
        for step in 0..=20 {
            let x = preset.x + preset.w * step as f32 / 20.0;
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false);
            seen.push((
                s.graphics_preset.clone(),
                s.render_distance,
                s.simulation_distance,
                s.cloud_range,
            ));
        }
        ui.release();
        for expected in [
            ("fast", 8, 6, 32),
            ("fancy", 16, 12, 64),
            ("fabulous", 32, 12, 128),
        ] {
            assert!(
                seen.iter()
                    .any(|(p, r, d, c)| (p.as_str(), *r, *d, *c) == expected),
                "{expected:?} in {seen:?}"
            );
        }
    }

    #[test]
    fn gui_scale_cycle_handles_reverse_and_out_of_range_saved_values() {
        for (framebuffer, force_unicode, max) in [
            ([854, 480], false, 2),
            ([1920, 1080], false, 4),
            ([1280, 720], true, 4),
            ([3840, 2160], false, 9),
            ([320, 240], false, 1),
        ] {
            let mut s = Settings {
                force_unicode,
                ..Settings::default()
            };
            for expected in (0..=max).rev() {
                Opt::GuiScale.cycle(&mut s, true, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            for expected in (1..=max).chain([0]) {
                Opt::GuiScale.cycle(&mut s, false, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, false, framebuffer);
            assert_eq!(s.gui_scale, 0);
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, true, framebuffer);
            assert_eq!(s.gui_scale, max - 1);
        }
        let mut s = Settings {
            gui_scale: 4,
            ..Settings::default()
        };
        // A resized window supplies a new maximum on the next interaction.
        Opt::GuiScale.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.gui_scale, 1);
    }

    #[test]
    fn classic_buttons_sound_but_disabled_and_nonprimary_clicks_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Mouse);
        let size = (480.0, 320.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(_),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(ui.release().is_empty());
        assert!(
            ui.click(&mut s, &atlas, 0.0, 0.0, size, [854, 480], false, false)
                .is_empty()
        );
        ui.show(Page::KeyBinds);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let disabled = widgets
            .iter()
            .find(|wd| matches!(wd.kind, WKind::Button { active: false, .. }))
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                disabled.x + 1.0,
                disabled.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn classic_sliders_sound_once_on_release_including_unchanged_values() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (slider.x + slider.w - 1.0, slider.y + 1.0);
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::SliderDrag]
        );
        assert_eq!(s.master_volume, 1.0);
        assert_eq!(ui.release(), vec![Event::ButtonClick, Event::Changed]);
        assert!(ui.release().is_empty());
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::SliderDrag]
        );
        for cx in [-100.0, 250.0, 1000.0] {
            assert_eq!(
                ui.click(&mut s, &atlas, cx, -100.0, size, [854, 480], true, false),
                vec![Event::SliderDrag, Event::Volume(0, s.master_volume)]
            );
        }
        assert_eq!(ui.release(), vec![Event::ButtonClick, Event::Changed]);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false)
                .is_empty()
        );
    }

    #[test]
    fn a_moved_volume_slider_reports_its_category_and_value() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let hostile = SOUND_SOURCES.iter().position(|n| *n == "hostile").unwrap();
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| matches!(wd.kind, WKind::Slider { opt: Opt::Volume(i), .. } if i == hostile))
            .unwrap();
        let y = slider.y + 1.0;
        let x = slider.x + 4.0 + (slider.w - 8.0) * 0.5;
        let events = ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        let volume = s.volumes[hostile];
        assert!((volume - 0.5).abs() < 1e-3, "{volume}");
        assert_eq!(
            events,
            vec![Event::SliderDrag, Event::Volume(hostile, volume)]
        );
        // OptionInstance.set: an unchanged value runs no callback.
        assert_eq!(
            ui.click(&mut s, &atlas, x, -50.0, size, [854, 480], true, false),
            vec![Event::SliderDrag]
        );
    }

    #[test]
    fn classic_sliders_follow_the_pointer_and_snap_on_release() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (slider.x + 4.0 + (slider.w - 8.0) * 0.3, slider.y + 1.0);
        ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false);
        let raw = ui::slider_value(slider.x, slider.w, x);
        // Held: the handle stays under the pointer whatever the stepped value.
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), raw);
        assert_eq!(ui.slider_shown(Opt::Volume(1), 0.25), 0.25);
        ui.click(
            &mut s,
            &atlas,
            x + 10.0,
            -50.0,
            size,
            [854, 480],
            true,
            false,
        );
        let raw = ui::slider_value(slider.x, slider.w, x + 10.0);
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), raw);
        ui.release();
        assert_eq!(ui.slider_shown(Opt::Volume(0), 0.25), 0.25);
        // Render distance applies late (applyValueImmediately = false) and
        // keeps its position until then, even once released.
        ui.slider_value = Some((Opt::RenderDistance, 0.51, Instant::now()));
        assert_eq!(ui.slider_shown(Opt::RenderDistance, 0.5), 0.51);
        ui.slider_value = Some((
            Opt::RenderDistance,
            0.51,
            Instant::now() - std::time::Duration::from_millis(600),
        ));
        assert_eq!(ui.slider_shown(Opt::RenderDistance, 0.5), 0.5);
    }

    #[test]
    fn classic_language_rows_and_text_fields_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let size = (480.0, 320.0);
        let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
        let row = widgets
            .iter()
            .find(|wd| {
                matches!(wd.kind, WKind::LanguageRow { .. }) && wd.y >= top && wd.y + wd.h <= bottom
            })
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                row.x + 1.0,
                row.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
        assert!(ui.release().is_empty());
        let search = ui.language_search.clone();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                search.x + 1.0,
                search.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn language_search_hint_does_not_overlap_the_focused_caret() {
        let atlas = atlas();
        let settings = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Language);
        let draw_hint = |ui: &mut OptionsUi| {
            let mut batch = HudBatch::new([960, 640]);
            ui.draw(&settings, &mut batch, &atlas, (-1.0, -1.0));
            let search = &ui.language_search;
            batch.vertices.iter().any(|v| {
                v.color == rgba(170, 170, 170, 255)
                    && v.pos[0] >= search.text_x()
                    && v.pos[0] < search.x + search.width
                    && v.pos[1] >= search.text_y()
                    && v.pos[1] < search.y + search.height
            })
        };
        ui.language_search.focused = false;
        assert!(draw_hint(&mut ui));
        ui.language_search.focused = true;
        assert!(!draw_hint(&mut ui));
        ui.language_search.focused = false;
        ui.language_search.set_value("english");
        assert!(!draw_hint(&mut ui));
        ui.language_search.set_value("");
        assert!(draw_hint(&mut ui));
    }

    fn languages() -> Vec<LanguageInfo> {
        (0..30)
            .map(|i| LanguageInfo {
                code: format!("language_{i}"),
                name: format!("Language {i}"),
                region: String::new(),
                bidirectional: false,
            })
            .collect()
    }

    #[test]
    fn classic_scroll_stops_at_both_limits_before_layout() {
        let s = Settings::default();
        let size = (480.0, 120.0);
        for page in [Page::KeyBinds, Page::Chat, Page::Sound, Page::Language] {
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 4.5, "{page:?}");
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), -1000.0);
                assert_eq!(ui.scroll, max, "{page:?}");
            }
            ui.scroll_at(&s, size, (240.0, 60.0), 0.25);
            assert_eq!(ui.scroll, max - 4.5, "{page:?}");
            ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
            ui.scroll_at(&s, size, (240.0, 60.0), -0.25);
            assert_eq!(ui.scroll, 4.5, "{page:?}");
        }
    }

    #[test]
    fn partially_visible_keybind_rows_are_clipped_and_clickable() {
        let mut atlas = atlas();
        let button = crate::hud::Rect {
            u0: 0.25,
            v0: 0.25,
            u1: 0.5,
            v1: 0.5,
            w: 20,
            h: 20,
        };
        for name in ["button", "button_disabled", "button_highlighted"] {
            atlas
                .sprites
                .insert(format!("gui/sprites/widget/{name}"), button);
        }
        for above in [true, false] {
            let mut s = Settings::default();
            let mut ui = OptionsUi::default();
            ui.show(Page::KeyBinds);
            let mut batch = HudBatch::new([480, 240]);
            let size = (batch.width, batch.height);
            let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
            let row = widgets
                .iter()
                .filter(|w| {
                    matches!(
                        w.kind,
                        WKind::Button {
                            act: Act::Key(_),
                            ..
                        }
                    )
                })
                .nth(10)
                .unwrap();
            let WKind::Button {
                act: Act::Key(key), ..
            } = row.kind
            else {
                unreachable!()
            };
            let row_y = if above { top - 5.0 } else { bottom - 16.0 };
            ui.scroll = row.y - row_y;
            let (x, width, height) = (row.x, row.w, row.h);
            ui.draw(&s, &mut batch, &atlas, (-1.0, -1.0));
            let vertices: Vec<_> = batch
                .vertices
                .iter()
                .filter(|v| {
                    (0.25..=0.5).contains(&v.uv[0])
                        && (0.25..=0.5).contains(&v.uv[1])
                        && v.pos[0] >= x
                        && v.pos[0] <= x + width
                        && v.pos[1] >= row_y
                        && v.pos[1] <= row_y + height
                })
                .collect();
            assert!(!vertices.is_empty());
            let lo = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::INFINITY, f32::min);
            let hi = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(lo, row_y.max(top));
            assert_eq!(hi, (row_y + height).min(bottom));
            let hidden_y = if above {
                row_y + 1.0
            } else {
                row_y + height - 1.0
            };
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    hidden_y,
                    size,
                    [480, 240],
                    false,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.selected_key, None);
            assert_eq!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    (lo + hi) / 2.0,
                    size,
                    [480, 240],
                    false,
                    false
                ),
                vec![Event::ButtonClick]
            );
            assert_eq!(ui.selected_key, Some(key));
        }
    }

    #[test]
    fn classic_scrollbar_drag_is_bounded_silent_and_stops_on_release() {
        let atlas = atlas();
        let size = (480.0, 160.0);
        for page in [Page::KeyBinds, Page::PadBinds, Page::Language, Page::Chat] {
            let mut s = Settings::default();
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 0.0, "{page:?}");
            let x = ui.scrollbar_x(size.0) + 2.0;
            let (_, thumb) = ui::scrollbar_thumb(top, bottom, content, ui.scroll).unwrap();
            let y = top + thumb / 2.0;
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, true)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, false)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, Some(y));
            let delta = 5.0;
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x - 100.0,
                    y + delta,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert!((ui.scroll - delta * max / (bottom - top - thumb)).abs() < 0.001);
            for (y, expected) in [(bottom + 100.0, max), (top - 100.0, 0.0)] {
                assert!(
                    ui.click(&mut s, &atlas, x, y, size, [960, 480], true, false)
                        .is_empty()
                );
                assert!((ui.scroll - expected).abs() < 0.001, "{page:?}");
            }
            assert!(ui.release().is_empty());
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x,
                    bottom + 100.0,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.scroll, 0.0);
            assert_eq!(s.keys, Settings::default().keys);
            ui.click(&mut s, &atlas, x, top + 1.0, size, [960, 480], false, false);
            assert!(ui.scrollbar_drag_y.is_some());
            ui.show(Page::Mouse);
            assert_eq!(ui.scrollbar_drag_y, None);
        }
    }

    #[test]
    fn extras_and_vanilla_bindings_accept_mouse_buttons() {
        use crate::extras::{Extras, SORT_KEY};
        use winit::event::MouseButton;
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ] {
            for name in [SORT_KEY, crate::graphics::HDR_KEY, "key.jump"] {
                let mut ui = OptionsUi::default();
                let mut s = Settings::default();
                ui.selected_key = Some(name);
                assert_eq!(ui.bind_mouse(&mut s, button), vec![Event::Changed]);
                assert_eq!(ui.selected_key, None);
                let value = keybinds::mouse_name(button);
                match name {
                    SORT_KEY => assert_eq!(s.extras.sort_key, value),
                    crate::graphics::HDR_KEY => assert_eq!(s.horizon.hdr_key, value),
                    _ => assert_eq!(s.keys.get(name), value),
                }
                assert_eq!(Extras::parse(&s.extras.serialize()), s.extras);
                if name != "key.jump" {
                    assert_eq!(s.keys, Settings::default().keys);
                }
                assert!(ui.bind_mouse(&mut s, button).is_empty());
            }
        }
    }

    #[test]
    fn horizon_extra_mouse_binding_consumes_the_click() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Extras);
        for (right, expected) in [(false, "key.mouse.left"), (true, "key.mouse.right")] {
            ui.selected_key = Some(crate::extras::SORT_KEY);
            assert_eq!(
                ui.click(
                    &mut s,
                    &atlas,
                    2.0,
                    2.0,
                    (480.0, 240.0),
                    [960, 480],
                    false,
                    right
                ),
                vec![Event::Changed]
            );
            assert_eq!(s.extras.sort_key, expected);
            assert_eq!(ui.selected_key, None);
            assert_eq!(ui.page(), Page::Extras);
        }
    }

    #[test]
    fn key_rebinding_takes_precedence_over_scrollbar_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.selected_key = Some("key.jump");
        let x = ui.scrollbar_x(480.0) + 2.0;
        assert_eq!(
            ui.click(
                &mut s,
                &atlas,
                x,
                50.0,
                (480.0, 240.0),
                [960, 480],
                false,
                false
            ),
            vec![Event::Changed]
        );
        assert_eq!(s.keys.get("key.jump"), "key.mouse.left");
        assert_eq!(ui.scrollbar_drag_y, None);
        assert!(ui.release().is_empty());
    }

    #[test]
    fn classic_pages_without_overflow_do_not_scroll() {
        let s = Settings::default();
        for page in [Page::Main, Page::Font, Page::Mouse] {
            let mut ui = OptionsUi::default();
            ui.show(page);
            for amount in [-1000.0, 1000.0, -0.25] {
                ui.scroll(&s, (480.0, 720.0), amount);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
        }
    }

    #[test]
    fn classic_resize_positions_rows_at_the_clamped_offset() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.scroll(&s, (480.0, 120.0), -1000.0);
        assert!(ui.scroll > 0.0);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(ui.scroll, 0.0);
        let (expected, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
    }

    #[test]
    fn language_centering_and_filtering_use_bounded_row_positions() {
        let s = Settings {
            language: "language_29".into(),
            ..Default::default()
        };
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let (widgets, top, bottom, content) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, content - (bottom - top));
        let (expected, _, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
        ui.language_search.set_value("language_29");
        let (widgets, top, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, 0.0);
        assert_eq!(widgets.iter().find(|w| w.list).unwrap().y, top + 2.0);
    }

    #[test]
    fn title_shortcuts_initialize_pages_and_return_directly_to_the_title() {
        let mut ui = OptionsUi::default();
        let mut s = Settings::default();
        for page in [Page::Accessibility, Page::Language] {
            ui.language_choice = Some("de_de".into());
            ui.language_search.set_value("old query");
            ui.open_at(false, page);
            assert_eq!(ui.page(), page);
            assert!(!ui.in_world);
            if page == Page::Language {
                assert_eq!(ui.language_choice, None);
                assert!(ui.language_search.value.is_empty());
                assert!(ui.center_language);
            }
            assert!(ui.back(&mut s).contains(&Event::Close));
            assert_eq!(s.language, "en_us");
            ui.open(false);
            ui.show(page);
            assert!(!ui.back(&mut s).contains(&Event::Close));
            assert_eq!(ui.page(), Page::Main);
        }
        ui.open_at(false, Page::Language);
        ui.language_choice = Some("de_de".into());
        let events = ui.back(&mut s);
        assert!(events.contains(&Event::Language("de_de".into())));
        assert!(events.contains(&Event::Close));
        assert_eq!(s.language, "de_de");
    }

    #[test]
    fn horizon_settings_hang_off_options_and_reach_every_horizon_page() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        let pushes = |ui: &mut OptionsUi| -> Vec<Page> {
            let (widgets, _, _, _) = ui.layout(&s, 427.0, 240.0);
            widgets
                .iter()
                .filter_map(|w| match &w.kind {
                    WKind::Button {
                        act: Act::Push(page),
                        active: true,
                        ..
                    } => Some(*page),
                    _ => None,
                })
                .collect()
        };
        // Vanilla's Options buttons, Horizon's two in the Telemetry and Credits slots, then
        // Credits & Attribution (WinScreen's credits).
        let main = pushes(&mut ui);
        assert_eq!(
            &main[main.len() - 3..],
            [Page::Horizon, Page::Mods, Page::Credits]
        );
        ui.push(Page::Horizon);
        assert_eq!(
            pushes(&mut ui),
            [
                Page::Graphics,
                Page::GraphicsAdvanced,
                Page::ShaderPacks,
                Page::Controller,
                Page::Extras,
                Page::Mods,
                Page::Agent,
            ]
        );
        // No AI hides the AI Agent page.
        let mut no_ai = Settings::default();
        no_ai.extras.no_ai = true;
        let (widgets, _, _, _) = ui.layout(&no_ai, 427.0, 240.0);
        assert!(!widgets.iter().any(|w| matches!(
            &w.kind,
            WKind::Button {
                act: Act::Push(Page::Agent),
                ..
            }
        )));
        // The AI Agent page: the switch, Copy and the status the game sets.
        ui.push(Page::Agent);
        ui.agent_status = vec!["Waiting for an agent.".into()];
        let (widgets, _, _, _) = ui.layout(&s, 427.0, 240.0);
        let acts: Vec<&Act> = widgets
            .iter()
            .filter_map(|w| match &w.kind {
                WKind::Button { act, .. } => Some(act),
                _ => None,
            })
            .collect();
        assert!(acts.contains(&&Act::Extra(crate::extras::Item::AgentControl)));
        assert!(acts.contains(&&Act::CopyAgentDetails));
        // Vanilla's own pages keep the Horizon pages they had.
        ui.stack = vec![Page::Main, Page::Video];
        let video = pushes(&mut ui);
        for page in [Page::Graphics, Page::GraphicsAdvanced] {
            assert!(video.contains(&page), "{page:?}");
        }
        ui.stack = vec![Page::Main, Page::Controls];
        assert!(pushes(&mut ui).contains(&Page::Controller));
    }

    #[test]
    fn sound_device_button_in_sound_page_and_cycles() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let rows = ui.list_rows(&s);
        assert!(
            rows.iter().any(|r| matches!(r, Row::Big(Opt::SoundDevice))),
            "Sound page should contain Row::Big(Opt::SoundDevice)"
        );
        assert_eq!(s.sound_device, "");
        let devices = crate::audio::output_devices();
        let count = 1 + devices.len();
        // Cycling forward 'count' times wraps around back to the initial selection ("")
        for _ in 0..count {
            Opt::SoundDevice.cycle(&mut s, false, [854, 480]);
        }
        assert_eq!(s.sound_device, "");
        // Cycling backward 'count' times also wraps around back to ""
        for _ in 0..count {
            Opt::SoundDevice.cycle(&mut s, true, [854, 480]);
        }
        assert_eq!(s.sound_device, "");
    }

    #[test]
    fn slider_keyboard_navigation_and_selection_toggle_parity() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Main);
        s.fov = 70.0;

        // 1. Tab focuses the first interactive widget: FOV slider
        ui.key(&mut s, &atlas, KeyCode::Tab, None);
        assert_eq!(ui.focus, Some(FocusId::Slider(Opt::Fov)));
        assert!(
            ui.can_change_value,
            "Tab onto slider enables can_change_value (Vanilla behavior)"
        );

        // 2. Right Arrow steps FOV up to 71
        let events = ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(s.fov, 71.0);
        assert!(events.contains(&Event::Changed));

        // 3. Left Arrow steps FOV down back to 70, then 69
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(s.fov, 70.0);
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(s.fov, 69.0);

        // 4. Space / Enter toggles can_change_value to false
        ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            !ui.can_change_value,
            "Space toggles can_change_value to false"
        );

        // 5. With can_change_value false, Left/Right arrow keys navigate focus instead of changing value!
        ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(
            s.fov, 69.0,
            "FOV must not change when can_change_value is false"
        );
        assert_ne!(
            ui.focus,
            Some(FocusId::Slider(Opt::Fov)),
            "Focus moved to next widget horizontally"
        );

        // 6. Navigate back to FOV slider with Left Arrow
        ui.key(&mut s, &atlas, KeyCode::ArrowLeft, None);
        assert_eq!(ui.focus, Some(FocusId::Slider(Opt::Fov)));
        assert!(
            !ui.can_change_value,
            "Arrow navigation onto slider keeps can_change_value false"
        );

        // 7. Space toggles can_change_value back to true
        ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(ui.can_change_value);
        ui.key(&mut s, &atlas, KeyCode::ArrowRight, None);
        assert_eq!(s.fov, 70.0);

        // 8. Escape closes the options page immediately, even while a slider
        // is focused for value changes.
        let events = ui.key(&mut s, &atlas, KeyCode::Escape, None);
        assert!(!ui.can_change_value);
        assert_eq!(
            ui.page(),
            Page::Main,
            "Escape must close the options page from value edit"
        );
        assert!(events.contains(&Event::Close));
    }

    #[test]
    fn escape_closes_options_with_search_focused() {
        use winit::keyboard::KeyCode;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        ui.focus = Some(FocusId::Search);
        ui.settings_search.focused = true;

        let events = ui.key(&mut s, &atlas, KeyCode::Escape, None);
        assert!(events.contains(&Event::Changed));
        assert_eq!(ui.page(), Page::Main);
        assert_eq!(ui.focus, None);
        assert!(!ui.settings_search.focused);
    }

    #[test]
    fn slider_mouse_wheel_scrolls_list_parity() {
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Main);
        s.fov = 70.0;
        let size: (f32, f32) = (427.0, 240.0);
        let cx = (size.0 / 2.0_f32).floor();

        // Cursor over FOV slider (cx - 154.0, 29.0, 150.0, 20.0)
        let cursor = (cx - 100.0, 35.0);
        // In vanilla parity, mouse wheel over a slider does not change the slider value.
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert_eq!(s.fov, 70.0);

        // Sound page: Master Volume slider
        ui.show(Page::Sound);
        s.master_volume = 0.5;
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let master = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let cursor = (master.x + 10.0, master.y + 10.0);
        let events = ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert_eq!(s.master_volume, 0.5);
        assert!(!events.iter().any(|e| matches!(e, Event::Volume(0, _))));
    }

    #[test]
    fn horizon_pages_slider_and_wheel_parity() {
        use winit::keyboard::KeyCode as K;
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();

        // 1. Extras page: scrolling wheel on Sort switch cycles it
        ui.show(Page::Extras);
        let size = (480.0, 600.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let sort_wd = widgets
            .iter()
            .find(|w| {
                matches!(
                    w.kind,
                    WKind::Button {
                        act: Act::Extra(crate::extras::Item::Sort),
                        ..
                    }
                )
            })
            .unwrap();
        let cursor = (sort_wd.x + 5.0, sort_wd.y + 5.0);
        assert!(!s.extras.sort);
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert!(s.extras.sort);
        ui.wheel(&mut s, size, cursor, [854, 480], 1.0);
        assert!(!s.extras.sort);

        // 2. Horizon Graphics page through OptionsUi:
        // Tab onto ChunkUpdates slider, ArrowRight adjusts value, Space toggles can_change_value
        ui.show(Page::GraphicsAdvanced);
        s.horizon.chunk_updates = 10;
        ui.size = size;
        ui.framebuffer = [854, 480];

        // Tab into controls
        for _ in 0..15 {
            ui.key(&mut s, &atlas, K::Tab, None);
            if ui.graphics.focus
                == Some(crate::graphics_ui::Target::Item(
                    crate::graphics_ui::Item::Horizon(crate::graphics_ui::H::ChunkUpdates),
                ))
            {
                break;
            }
        }
        assert_eq!(
            ui.graphics.focus,
            Some(crate::graphics_ui::Target::Item(
                crate::graphics_ui::Item::Horizon(crate::graphics_ui::H::ChunkUpdates)
            ))
        );
        assert!(ui.graphics.can_change_value);

        // Right arrow steps ChunkUpdates to 11
        let events = ui.key(&mut s, &atlas, K::ArrowRight, None);
        assert_eq!(s.horizon.chunk_updates, 11);
        assert!(events.contains(&Event::SliderDrag));

        // Space toggles can_change_value to false
        ui.key(&mut s, &atlas, K::Space, None);
        assert!(!ui.graphics.can_change_value);

        // ArrowRight does not change ChunkUpdates when can_change_value is false
        ui.key(&mut s, &atlas, K::ArrowRight, None);
        assert_eq!(s.horizon.chunk_updates, 11);
    }

    #[test]
    fn options_step_slider_matches_set_slider_for_all_stepped_options() {
        let stepped_opts = [
            Opt::Fov,
            Opt::RenderDistance,
            Opt::SimulationDistance,
            Opt::MaxFps,
            Opt::CloudRange,
            Opt::EntityDistance,
            Opt::BiomeBlend,
            Opt::Preset,
            Opt::Mipmaps,
            Opt::WeatherRadius,
            Opt::ChunkFade,
            Opt::MenuBlur,
            Opt::NotificationDisplayTime,
            Opt::PadMoveDeadzone,
            Opt::PadLookDeadzone,
            Opt::SprintWindow,
        ];
        for opt in stepped_opts {
            let steps = opt.slider_steps().expect("slider steps");
            for i in 0..steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                opt.set_slider(&mut s1, pos);

                opt.step_slider(&mut s1, true, 200.0);
                let next_pos = (i + 1) as f32 / steps as f32;
                opt.set_slider(&mut s2, next_pos);
                assert_eq!(
                    opt.slider_position(&s1),
                    opt.slider_position(&s2),
                    "{opt:?} forward from {i}"
                );
            }
            for i in 1..=steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                opt.set_slider(&mut s1, pos);

                opt.step_slider(&mut s1, false, 200.0);
                let prev_pos = (i - 1) as f32 / steps as f32;
                opt.set_slider(&mut s2, prev_pos);
                assert_eq!(
                    opt.slider_position(&s1),
                    opt.slider_position(&s2),
                    "{opt:?} backward from {i}"
                );
            }
        }
    }

    #[test]
    fn options_keyboard_space_enter_emits_button_click_for_cycle_and_default_buttons() {
        use winit::keyboard::KeyCode;
        let mut ui = OptionsUi::default();
        let mut s = Settings::default();
        let atlas = atlas();

        // 1. Focus a cycle button (e.g. Opt::ViewBobbing)
        ui.focus = Some(FocusId::Cycle(Opt::ViewBobbing));
        let events = ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Space on FocusId::Cycle must emit Event::ButtonClick"
        );
        assert!(events.contains(&Event::Changed));

        let events = ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Enter on FocusId::Cycle must emit Event::ButtonClick"
        );
        assert!(events.contains(&Event::Changed));

        // 2. Focus a default/action button (e.g. Act::Done)
        ui.focus = Some(FocusId::Act(Act::Done));
        let events = ui.key(&mut s, &atlas, KeyCode::Space, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Space on FocusId::Act must emit Event::ButtonClick"
        );

        ui.focus = Some(FocusId::Act(Act::Done));
        let events = ui.key(&mut s, &atlas, KeyCode::Enter, None);
        assert!(
            events.contains(&Event::ButtonClick),
            "Enter on FocusId::Act must emit Event::ButtonClick"
        );
    }
}
