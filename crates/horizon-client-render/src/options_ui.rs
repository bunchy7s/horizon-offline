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
            Opt::Narrator
                | Opt::NarratorHotkey
                | Opt::ChatDelay
                // Stored, but there are no blocked players, signed chat or
                // chat drafts yet.
                | Opt::HideMatchedNames
                | Opt::OnlySecureChat
                | Opt::SaveChatDrafts
                | Opt::MonochromeLogo
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
                // Saved, but the creative tabs, the quit keys and the mouse
                // don't read them yet.
                | Opt::OperatorItemsTab
                | Opt::QuitShortcuts
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
            Opt::Narrator => tr("options.narrator.off"),
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
            | Opt::HideMatchedNames
            | Opt::OnlySecureChat
            | Opt::SaveChatDrafts
            | Opt::ModelPart(_)
            | Opt::AllowServerListing
            | Opt::RealmsNotifications
            | Opt::InGameNotification
            | Opt::DirectionalAudio => {
                let key = self.file_key().unwrap();
                s.file
                    .set(key, crate::options_file::V::Bool(!s.file.bool(key)));
            }
            Opt::InvertX => s.invert_x = !s.invert_x,
            Opt::InvertY => s.invert_y = !s.invert_y,
            Opt::DiscreteScroll => s.discrete_scroll = !s.discrete_scroll,
            Opt::AllowCursorChanges => s.allow_cursor_changes = !s.allow_cursor_changes,
            Opt::ViewBobbing => s.view_bobbing = !s.view_bobbing,
            Opt::Vsync => s.vsync = !s.vsync,
            Opt::Fullscreen => s.fullscreen = !s.fullscreen,
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
/// GraphicsPreset.apply: the options Fast, Fancy and Fabulous set (texture
/// filtering and anisotropy have no option here yet).
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
        }
        _ => {}
    }
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
    /// Horizon's graphics settings (`graphics_ui`).
    Graphics,
    GraphicsAdvanced,
    GraphicsHdr,
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
    pub in_world: bool,
    /// Scroll the selected language into the middle on the next layout.
    center_language: bool,
    pub graphics: crate::graphics_ui::GraphicsUi,
    /// The AI Agent page's status lines (set by the game while it shows).
    pub agent_status: Vec<String>,
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
            in_world: false,
            center_language: false,
            graphics: Default::default(),
            agent_status: Vec::new(),
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
    }
}
fn inactive(mut w: Widget) -> Widget {
    if let WKind::Button { active, .. } = &mut w.kind {
        *active = false;
    }
    w
}
fn option_widget(s: &Settings, opt: Opt, x: f32, y: f32, w: f32) -> Widget {
    if !opt.available() {
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
        Page::ShaderPacks => "horizon.pages.shader_packs.tooltip",
        Page::Mods => "horizon.pages.mods.tooltip",
        _ => return None,
    })
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
    pub fn page(&self) -> Page {
        *self.stack.last().unwrap_or(&Page::Main)
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
    /// Open a page whose Done/Escape returns directly to the calling screen.
    pub fn open_at(&mut self, in_world: bool, page: Page) {
        self.stack.clear();
        self.data_pack_dir = None;
        self.push(page);
        self.in_world = in_world;
        self.selected_key = None;
    }
    fn push(&mut self, page: Page) {
        self.stack.push(page);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
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
            Page::Graphics | Page::GraphicsAdvanced | Page::GraphicsHdr => "options.videoTitle",
            Page::Controller => "horizon.pages.controller",
            Page::PadBinds => "horizon.pages.controller_binds",
            Page::Extras => "horizon.pages.extras",
            Page::Credits => "credits_and_attribution.screen.title",
            Page::Horizon => "horizon.pages.settings",
            Page::Agent => "horizon.pages.agent",
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
                rows.extend(pairs(&[
                    Opt::MaxFps,
                    Opt::Vsync,
                    Opt::InactivityFps,
                    Opt::GuiScale,
                    Opt::Fullscreen,
                    Opt::Gamma,
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
                        if installed.problem.is_some() {
                            label = label.append(tr("horizon.mods.stopped"));
                        } else if installed.dev {
                            label = label.append(tr("horizon.mods.dev"));
                        }
                        let dead = installed.dev || installed.id.is_empty();
                        // A native mod gets its settings page's button beside
                        // the toggle.
                        if installed.native.is_some() {
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
                s.shader_packs
                    .iter()
                    .enumerate()
                    .map(|(i, pack)| {
                        let kind = match pack.kind {
                            crate::shader_pack_list::Kind::Horizon => {
                                lang::get("horizon.shader_packs.kind_horizon")
                            }
                            crate::shader_pack_list::Kind::Classic => String::new(),
                        };
                        let label = Text::literal(format!("{}{kind}: ", pack.title))
                            .append(on_off(s.horizon.shader_pack == pack.file));
                        Row::Buttons(button(0.0, 0.0, 310.0, label, Act::PickShaderPack(i)), None)
                    })
                    .collect()
            }
            _ => {
                let _ = s;
                Vec::new()
            }
        }
    }
    /// Build every widget of the current page.
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
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
                    ("options.language", Some(Page::Language)),
                    ("options.chat", Some(Page::Chat)),
                    ("options.resourcepack", Some(Page::Packs)),
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
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
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
                        });
                        y += 20.0;
                    }
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
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let pad = &s.controller;
                for (action, _) in gamepad::ACTIONS {
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
                    });
                    y += 18.0;
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
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
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
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                let folder = match page {
                    Page::Mods => Some(("horizon.mods.open_folder", Act::OpenModFolder)),
                    Page::ShaderPacks => Some(("pack.openFolder", Act::OpenShaderPackFolder)),
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
        (out, top, bottom, content)
    }
    fn clamp_scroll(&mut self, top: f32, bottom: f32, content: f32) {
        let max = (content - (bottom - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
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
        if let Some(level) = Self::graphics_level(self.page()) {
            self.graphics
                .draw(level, s, batch, atlas, cursor, self.in_world);
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
        let in_world = self.in_world;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let list = bottom > top;
        if list {
            ui::list_background(batch, atlas, top, bottom, in_world);
        }
        let visible = |wd: &Widget| !wd.list || (wd.y < bottom && wd.y + wd.h > top);
        let mut tooltip = None;
        for wd in &widgets {
            if !visible(wd) {
                continue;
            }
            let start = batch.vertices.len();
            let hover = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || cursor.1 >= top && cursor.1 < bottom);
            if hover {
                if let WKind::Button {
                    act: Act::Extra(item),
                    ..
                } = &wd.kind
                {
                    tooltip = Some(atlas.font.split(&Text::literal(item.description()), 170.0));
                }
                if let WKind::Button {
                    act: Act::Push(page),
                    ..
                } = &wd.kind
                    && let Some(text) = horizon_page_tooltip(*page)
                {
                    tooltip = Some(atlas.font.split(&tr(text), 170.0));
                }
                let key = match &wd.kind {
                    WKind::Button {
                        act: Act::Cycle(opt),
                        ..
                    }
                    | WKind::Slider { opt, .. } => Some(opt.tooltip_key(s)),
                    WKind::Button {
                        act: Act::Push(Page::Language),
                        ..
                    } => Some("options.language.tooltip".into()),
                    WKind::Button {
                        act: Act::Push(Page::Accessibility),
                        ..
                    } => Some("options.accessibility.tooltip".into()),
                    _ => None,
                };
                if let Some(key) = key.filter(|key| lang::has(key)) {
                    tooltip = Some(atlas.font.split(&tr(&key), 170.0));
                }
                if let WKind::Button {
                    act: Act::Cycle(opt),
                    ..
                } = &wd.kind
                    && !opt.available()
                {
                    tooltip = Some(atlas.font.split(&tr("horizon.options.unavailable"), 170.0));
                }
                if let WKind::Button {
                    act: Act::ToggleMod(i),
                    ..
                } = &wd.kind
                    && let Some(installed) = s.mods.installed.get(*i)
                {
                    let description = installed
                        .problem
                        .as_deref()
                        .unwrap_or(&installed.description);
                    tooltip = Some(atlas.font.split(
                        &Text::literal(format!("{}\n{description}", installed.title)),
                        170.0,
                    ));
                }
            }
            match &wd.kind {
                WKind::Button { label, active, act } => {
                    ui::button(batch, atlas, wd.x, wd.y, wd.w, wd.h, label, *active, hover);
                    if hover && let Act::Key(name) = act {
                        let conflicts = s.keys.conflicts(name);
                        if !conflicts.is_empty() {
                            let names = conflicts
                                .iter()
                                .map(|c| lang::get(c))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let full = Text::translate(
                                "controls.keybinds.duplicateKeybinds",
                                &[Text::literal(names)],
                            );
                            tooltip = Some(
                                full.plain()
                                    .split('\n')
                                    .map(|l| Text::literal(l.to_owned()))
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                }
                WKind::Slider { label, value, opt } => {
                    let dragging = self.dragging == Some(*opt);
                    let shown = self.slider_shown(*opt, *value);
                    ui::slider(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        shown,
                        hover || dragging,
                        true,
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
                    if *selected {
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
            search.draw(batch, atlas, None);
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
            search.draw(batch, atlas, None);
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
        if let Some(level) = Self::graphics_level(self.page()) {
            use crate::graphics_ui::Action;
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
        let mut events = Vec::new();
        if !drag && matches!(self.page(), Page::Video | Page::Controls | Page::KeyBinds) {
            let search = &mut self.settings_search;
            search.focused = ui::hovered((x, y), search.x, search.y, search.width, search.height);
            if search.focused {
                search.click(atlas, x, false, false);
                self.scroll = 0.0;
                events.push(Event::ButtonClick);
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
            if let Some(opt) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
            {
                let v = ui::slider_value(wd.x, wd.w, x);
                self.slider_value = Some((opt, v, Instant::now()));
                set_slider(opt, s, v);
                events.push(Event::SliderDrag);
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
        // accepts other mouse buttons, but is not a widget activation.
        if right {
            return events;
        }
        if ui::scrollbar_thumb(top, bottom, content, self.scroll).is_some()
            && ui::hovered((x, y), self.scrollbar_x(size.0), top, 6.0, bottom - top)
        {
            self.scrollbar_drag_y = Some(y);
            return events;
        }
        for wd in widgets {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h) {
                continue;
            }
            if wd.list && (y < top || y >= bottom) {
                continue;
            }
            match wd.kind {
                WKind::Button {
                    active: true, act, ..
                } => {
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
                        Act::Credits => events.push(Event::Credits),
                        Act::CopyAgentDetails => events.push(Event::CopyAgentDetails),
                        Act::None => {}
                    }
                }
                WKind::Slider { opt, .. } => {
                    self.dragging = Some(opt);
                    let v = ui::slider_value(wd.x, wd.w, x);
                    self.slider_value = Some((opt, v, Instant::now()));
                    set_slider(opt, s, v);
                    events.push(Event::SliderDrag);
                }
                WKind::LanguageRow { code, .. } => {
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
        let slider = self.dragging.take().is_some();
        let graphics_slider = self.graphics.release();
        if slider || graphics_slider {
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
        events
    }
    /// The mouse wheel at the cursor. Over a cycle button it takes the step
    /// (`CycleButton.mouseScrolled`: up goes back) and the list stays put.
    pub fn wheel(
        &mut self,
        s: &mut Settings,
        size: (f32, f32),
        cursor: (f32, f32),
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Event> {
        let plain = self.resource_error.is_none()
            && Self::graphics_level(self.page()).is_none()
            && !(self.pack_screen.is_some() && self.stack.last() == Some(&Page::Packs));
        if !plain {
            self.scroll_at(s, size, cursor, amount);
            return Vec::new();
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
                Some(*opt)
            }
            _ => None,
        });
        match hovered {
            Some(opt) if amount != 0.0 => self.cycle_option(s, opt, amount > 0.0, framebuffer),
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
                self.pack_screen = Some(crate::pack_screen::PackScreen::new(
                    &m,
                    crate::text::Text::translate(title, &[]),
                    w,
                    h,
                    self.in_world,
                    self.pack_infos.clone(),
                    &self.pack_selection,
                ));
            }
        }
        self.pack_screen.as_mut().expect("made above")
    }
    /// Escape or Done: leave the current page, applying page results.
    pub fn back(&mut self, s: &mut Settings) -> Vec<Event> {
        let mut events = Vec::new();
        let page = self.stack.pop().unwrap_or(Page::Main);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.dragging = None;
        self.slider_value = None;
        self.selected_key = None;
        self.selected_pad_action = None;
        match page {
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
        if !s.extras.set_key(name, value.clone()) {
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
        use winit::keyboard::KeyCode as K;
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
        if key == K::Escape {
            return self.back(s);
        }
        if self.page() == Page::Language {
            if !self.language_search.key(key, false, false)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.language_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        } else if self.settings_search_active() {
            if !self.settings_search.key(key, false, false)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.settings_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
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
        for opt in [
            Opt::TextBackground,
            Opt::HideLightningFlashes,
            Opt::HideSplashTexts,
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
            native: Some(crate::mod_list::NativePage::default()),
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
                vec![Event::SliderDrag]
            );
        }
        assert_eq!(ui.release(), vec![Event::ButtonClick, Event::Changed]);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false)
                .is_empty()
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
            for name in [SORT_KEY, "key.jump"] {
                let mut ui = OptionsUi::default();
                let mut s = Settings::default();
                ui.selected_key = Some(name);
                assert_eq!(ui.bind_mouse(&mut s, button), vec![Event::Changed]);
                assert_eq!(ui.selected_key, None);
                let value = keybinds::mouse_name(button);
                match name {
                    SORT_KEY => assert_eq!(s.extras.sort_key, value),
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
}
