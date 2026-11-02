//! Horizon's graphics settings screens, Simple, Advanced and HDR: which settings
//! they show, in what groups, with what description and cost, and their
//! input. How they look comes from `graphics_style`.
use crate::{
    graphics::{
        EntityCullingMode, FarDetail, FrameGeneration, GraphicsApi, LeafCullingMode, Overlay,
        Pacing, PresentMode, Preset, Upscaling,
    },
    graphics_style::{Cost, STYLE},
    hud::{HudAtlas, HudBatch},
    lang,
    settings::{Opt, Settings},
    text::Text,
    ui,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Simple,
    Advanced,
    /// HDR output's own settings, opened from Advanced while HDR is on.
    Hdr,
}

/// Horizon's own settings (`graphics::Graphics`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H {
    Preset,
    LodDistance,
    FarDetail,
    CaveCulling,
    EntityCulling,
    LeafCulling,
    ChunkUpdates,
    UploadBudget,
    Sky,
    Fog,
    Weather,
    AnimatedTextures,
    DitherCutoutMips,
    PresentMode,
    Pacing,
    Workers,
    Overlay,
    ResolutionScale,
    Antialiasing,
    Upscaling,
    ScreenshotQuality,
    FrameGeneration,
    Hdr,
    /// Opens `Level::Hdr`.
    HdrSettings,
    HdrBlackLevel,
    /// Binds `Graphics::hdr_key`.
    HdrKey,
    /// Starts HDR calibration (`crate::hdr_calibration`).
    HdrCalibrate,
    HdrPeak,
    HdrPaperWhite,
    HdrUiBrightness,
    BatteryMode,
    ShadowCache,
    GraphicsApi,
    TranslucentBatching,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Vanilla(Opt),
    Horizon(H),
}

pub struct Setting {
    pub item: Item,
    pub cost: Cost,
    pub text: &'static str,
}
pub enum Row {
    Header(&'static str),
    Setting(Setting),
}
fn v(opt: Opt, cost: Cost, text: &'static str) -> Row {
    Row::Setting(Setting {
        item: Item::Vanilla(opt),
        cost,
        text,
    })
}
fn h(opt: H, cost: Cost, text: &'static str) -> Row {
    Row::Setting(Setting {
        item: Item::Horizon(opt),
        cost,
        text,
    })
}

/// The settings of a level, grouped under headers (Simple has no headers).
pub fn rows(level: Level) -> Vec<Row> {
    use Cost::*;
    let render = v(
        Opt::RenderDistance,
        High,
        "Full-detail terrain range. LOD Distance can extend local worlds farther.",
    );
    let lod = h(
        H::LodDistance,
        Medium,
        "Coarse distant terrain in local worlds. Integrated GPUs stop at 128 chunks.",
    );
    // Suggested, never switched on, where far terrain costs the GPU most.
    let far = h(
        H::FarDetail,
        Medium,
        if crate::graphics::reduced_far_detail_suggested() {
            "Reduced draws far terrain coarser: faster on this integrated GPU."
        } else {
            "Reduced draws far terrain coarser, for GPUs it slows down."
        },
    );
    let fov = v(
        Opt::Fov,
        Low,
        "Field of view. Wider shows more terrain at once.",
    );
    let fps = v(
        Opt::MaxFps,
        None,
        "Caps frames per second; a cap saves power and heat.",
    );
    let vsync = v(
        Opt::Vsync,
        None,
        "Waits for the display's refresh: no tearing, a little latency.",
    );
    let gui = v(Opt::GuiScale, None, "Size of menus and the HUD.");
    let preset = h(
        H::Preset,
        None,
        "Sets the options below together. Changing one makes it Custom.",
    );
    let mut rows = match level {
        Level::Simple => vec![preset, render, lod, far, fov, fps, vsync, gui],
        Level::Hdr => vec![
            Row::Header("HDR"),
            h(
                H::Hdr,
                None,
                "Lava, fire and sunlight shine brighter than white. Needs an HDR display.",
            ),
            h(
                H::HdrCalibrate,
                None,
                "Sets the three below full screen, one test picture at a time.",
            ),
            h(
                H::HdrBlackLevel,
                None,
                "Your display's darkest black. 0 for OLED; LCDs glow a little.",
            ),
            h(
                H::HdrPeak,
                None,
                "How bright highlights get. Auto: the peak your system reports.",
            ),
            h(
                H::HdrPaperWhite,
                None,
                "How bright white paper and menus look. Auto: your desktop's white.",
            ),
            h(
                H::HdrUiBrightness,
                None,
                "How bright the HUD and menus are, against paper white.",
            ),
            h(
                H::HdrKey,
                None,
                "A key that opens this screen from the game. Not bound at first.",
            ),
        ],
        Level::Advanced => vec![
            Row::Header("Rendering"),
            render,
            lod,
            far,
            v(
                Opt::SimulationDistance,
                Medium,
                "How far mobs, crops and machines keep working. CPU time.",
            ),
            v(
                Opt::EntityDistance,
                Medium,
                "How far away mobs and items are drawn, as a share of normal.",
            ),
            h(
                H::ChunkUpdates,
                Low,
                "Meshed sections uploaded per frame. Lower is smoother while loading.",
            ),
            Row::Header("Culling"),
            h(
                H::CaveCulling,
                Saves,
                "Skips terrain hidden behind closed sections. Off costs a lot.",
            ),
            h(
                H::EntityCulling,
                None,
                "Hides models behind opaque blocks. Async checks off-thread; entities tick.",
            ),
            h(
                H::LeafCulling,
                Saves,
                "Draws or drops leaf faces inside canopies. Off keeps vanilla's fancy leaves.",
            ),
            Row::Header("Quality"),
            v(
                Opt::SmoothLighting,
                Low,
                "Soft shadows in corners. Costs chunk building time only.",
            ),
            v(
                Opt::Mipmaps,
                None,
                "Smaller copies of block textures for distant terrain: less shimmer.",
            ),
            v(
                Opt::BiomeBlend,
                Low,
                "Blends grass and water colours across biomes when building chunks.",
            ),
            v(
                Opt::Clouds,
                Low,
                "Cloud layer: off, flat (fast) or 3D (fancy).",
            ),
            v(Opt::CloudRange, Low, "How far clouds reach, in chunks."),
            h(H::Sky, Low, "Draws the sky, sun, moon and stars."),
            h(
                H::Fog,
                None,
                "Distance fog. Off shows the edge of the loaded world.",
            ),
            h(H::Weather, High, "Draws rain and snow."),
            v(
                Opt::WeatherRadius,
                Medium,
                "How far rain and snow fall around you.",
            ),
            v(Opt::Particles, Medium, "How many particles effects spawn."),
            v(
                Opt::EntityShadows,
                Low,
                "Round shadows under mobs and items.",
            ),
            v(
                Opt::CutoutLeaves,
                Medium,
                "Leaves with see-through gaps. Off draws them solid.",
            ),
            v(
                Opt::ImprovedTransparency,
                Medium,
                "Sorts water and glass more exactly behind each other.",
            ),
            v(
                Opt::ChunkBuilder,
                Low,
                "Which chunk rebuilds wait for the frame: none, nearby or yours.",
            ),
            h(
                H::AnimatedTextures,
                Low,
                "Animates water, lava, fire and other moving textures.",
            ),
            h(
                H::DitherCutoutMips,
                Low,
                "Keeps fine plants in high-res packs from fading far away.",
            ),
            Row::Header("Display"),
            fov,
            fps,
            vsync,
            h(
                H::ResolutionScale,
                High,
                "World pixels per screen pixel. Below 100% is faster on the GPU.",
            ),
            h(
                H::Antialiasing,
                Medium,
                "Smooths jagged edges. FXAA: softer. TAA: smoothest, moving things may trail.",
            ),
            h(
                H::Upscaling,
                High,
                "Renders the world smaller and rebuilds it with TAA. Faster, softer.",
            ),
            h(
                H::ScreenshotQuality,
                None,
                "F2 screenshots at this many times the game's resolution. Higher takes longer.",
            ),
            h(
                H::FrameGeneration,
                Medium,
                "Shows a made-up frame between each two real ones: smoother, a little more lag.",
            ),
            h(
                H::HdrSettings,
                None,
                "HDR output on an HDR display: lava, fire and sunlight shine brighter.",
            ),
            h(
                H::PresentMode,
                None,
                "How frames reach the screen while VSync is off.",
            ),
            h(
                H::Pacing,
                None,
                "Efficient sleeps until the next frame; Precise spins, like vanilla.",
            ),
            v(Opt::Fullscreen, None, "Fills the screen (borderless)."),
            gui,
            v(Opt::Gamma, None, "Brightens dark places."),
            v(
                Opt::InactivityFps,
                None,
                "Lowers the frame rate when minimised, or idle too.",
            ),
            Row::Header("Preferences"),
            v(Opt::ViewBobbing, None, "Sways the view while walking."),
            v(Opt::ChunkFade, None, "Fades newly loaded chunks in."),
            v(Opt::Vignette, None, "Darkens the screen's edges."),
            v(Opt::Autosave, None, "Shows when the world saves."),
            v(Opt::AttackIndicator, None, "Shows attack cooldown."),
            Row::Header("Performance"),
            h(
                H::BatteryMode,
                None,
                "60 FPS cap (30 in menus or unfocused), half the workers: less power.",
            ),
            h(
                H::ShadowCache,
                Saves,
                "Reuses the pack shadow map when sun, view and world match. Off by default.",
            ),
            h(
                H::Workers,
                None,
                "Threads that generate, light and mesh chunks. Next start.",
            ),
            h(
                H::GraphicsApi,
                None,
                "Vulkan, then DirectX 12 (Windows) or OpenGL (Linux) where it fails. Next start.",
            ),
            h(
                H::TranslucentBatching,
                Saves,
                "Water and glass in one buffer: fewer draw calls, a bit more VRAM. Next start.",
            ),
            h(
                H::UploadBudget,
                Low,
                "Mesh data sent to the GPU per frame. Lower is smoother while loading.",
            ),
            Row::Header("Overlay"),
            h(
                H::Overlay,
                None,
                "Frame rate and frame times over the game, with a graph.",
            ),
        ],
    };
    // Only Windows and Linux have more than one graphics API, and translucent batching
    // (Metal draws flat).
    if cfg!(target_os = "macos") {
        rows.retain(|row| {
            !matches!(row, Row::Setting(s)
                if matches!(s.item, Item::Horizon(H::GraphicsApi | H::TranslucentBatching)))
        });
    }
    rows
}

const UPLOAD_STEPS: [u32; 8] = [0, 256, 512, 1024, 2048, 4096, 8192, 16384];
/// What the LOD Distance slider stops at, in chunks: off, then each doubling
/// up to the whole world.
const LOD_STEPS: [u32; 18] = [
    0,
    64,
    128,
    256,
    512,
    1024,
    2048,
    4096,
    8192,
    16384,
    32768,
    65536,
    131072,
    262144,
    524288,
    1048576,
    2097152,
    crate::lod::MAX_CHUNKS,
];

/// Horizon's graphics screens supplement the vanilla options on Video Settings.
/// Upscaling is only there while TAA is, which does it.
fn extra_rows(level: Level, settings: &Settings) -> Vec<Row> {
    let mut rows = rows(level);
    rows.retain(|row| !matches!(row, Row::Setting(s) if matches!(s.item, Item::Vanilla(_))));
    if !settings.horizon.taa {
        rows.retain(|row| !matches!(row, Row::Setting(s) if s.item == Item::Horizon(H::Upscaling)));
    }
    // Drop headings whose settings were all on the parent screens.
    let mut has_setting = false;
    rows.reverse();
    rows.retain(|row| match row {
        Row::Setting(_) => {
            has_setting = true;
            true
        }
        Row::Header(_) => std::mem::take(&mut has_setting),
    });
    rows.reverse();
    rows
}

/// Whether the system set HDR up for the display, and whether the player kept it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setup {
    /// The system says nothing: the player calibrates.
    Unknown,
    /// Peak Brightness and Paper White follow the system.
    Automatic,
    /// The system said, and the player changed them anyway.
    Changed,
}
fn setup(s: &Settings) -> Setup {
    setup_of(&s.horizon, crate::graphics::reported().is_some())
}
fn setup_of(g: &crate::graphics::Graphics, reported: bool) -> Setup {
    if !reported {
        Setup::Unknown
    } else if g.hdr_peak == 0 && g.hdr_paper_white == 0 {
        Setup::Automatic
    } else {
        Setup::Changed
    }
}
/// The description beside `item` where it depends on [`setup`]; `text` otherwise.
fn description(item: Item, s: &Settings, text: &'static str) -> &'static str {
    match (item, setup(s)) {
        (Item::Horizon(H::HdrCalibrate), Setup::Unknown) => {
            "Your display didn't say how bright it gets: calibrate to set it up."
        }
        (Item::Horizon(H::HdrCalibrate), Setup::Automatic) => {
            "Already set up for your display. Only calibrate if HDR looks wrong."
        }
        (Item::Horizon(H::HdrCalibrate), Setup::Changed) => {
            "Puts Peak Brightness and Paper White back to your display's setup."
        }
        (Item::Horizon(H::HdrPeak | H::HdrPaperWhite), Setup::Automatic | Setup::Changed) => {
            "Set up for your display. Leave on Auto unless it looks wrong."
        }
        (Item::Horizon(H::LeafCulling), _) => match s.horizon.leaf_culling {
            LeafCullingMode::Off => "Every leaf face. Vanilla's fancy leaves.",
            LeafCullingMode::Solid => {
                "Leaves fully inside a canopy draw solid: fewer see-through layers."
            }
            LeafCullingMode::SolidAggressive => {
                "Leaves with one open side draw solid too: faster, chunkier edges."
            }
            LeafCullingMode::Hollow => "Drops faces between leaves: faster, but gaps look through.",
        },
        _ => text,
    }
}
fn on_off(on: bool) -> String {
    lang::get(if on { "On" } else { "Off" })
}
/// A control's caption and its value, both through the language table: Horizon's captions are
/// English sentences, and some of the values ("Off", "12 chunks") are words too.
fn labelled(caption: &str, value: impl std::fmt::Display) -> Text {
    let value = value.to_string();
    Text::literal(format!("{}: {}", lang::get(caption), lang::get(&value)))
}

impl H {
    fn slider(self, s: &Settings) -> Option<f32> {
        let g = &s.horizon;
        match self {
            H::ChunkUpdates => Some(g.chunk_updates as f32 / 64.0),
            H::UploadBudget => {
                let i = UPLOAD_STEPS
                    .iter()
                    .position(|&b| b >= g.upload_budget_kib)
                    .unwrap_or(UPLOAD_STEPS.len() - 1);
                Some(i as f32 / (UPLOAD_STEPS.len() - 1) as f32)
            }
            H::LodDistance => {
                let i = LOD_STEPS
                    .iter()
                    .position(|&d| d >= g.lod_distance)
                    .unwrap_or(LOD_STEPS.len() - 1);
                Some(i as f32 / (LOD_STEPS.len() - 1) as f32)
            }
            H::Workers => Some(g.workers as f32 / max_workers() as f32),
            H::ResolutionScale => Some((g.resolution_scale as f32 - 50.0) / 150.0),
            // Auto, then 10..480 by 10.
            H::HdrPaperWhite => Some(g.hdr_paper_white as f32 / 480.0),
            // Auto, then 200..2000 by 50.
            H::HdrPeak => Some(match g.hdr_peak {
                0 => 0.0,
                n => (n as f32 - 150.0) / 1850.0,
            }),
            H::HdrBlackLevel => Some(g.hdr_black_level as f32 / 100.0),
            H::HdrUiBrightness => Some((g.hdr_ui_brightness as f32 - 25.0) / 75.0),
            _ => None,
        }
    }
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            H::ChunkUpdates => Some(64),
            H::UploadBudget => Some((UPLOAD_STEPS.len() - 1) as u32),
            H::Workers => Some(max_workers()),
            H::ResolutionScale => Some(30),
            H::HdrPaperWhite => Some(41),
            H::HdrPeak => Some(37),
            H::HdrBlackLevel => Some(100),
            H::HdrUiBrightness => Some(15),
            _ => None,
        }
    }
    fn label(self, s: &Settings) -> Text {
        let g = &s.horizon;
        match self {
            H::Preset => labelled("Preset", g.preset.label()),
            H::LodDistance => match g.lod_distance {
                0 => labelled("LOD Distance", "Off"),
                n => labelled("LOD Distance", lang::tr("%s chunks", &[&n])),
            },
            H::FarDetail => labelled("Far Detail", g.far_detail.label()),
            H::CaveCulling => labelled("Cave Culling", on_off(g.cave_culling)),
            H::EntityCulling => labelled("Entity Culling", g.entity_culling.label()),
            H::LeafCulling => labelled("Leaf Culling", g.leaf_culling.label()),
            H::ChunkUpdates => match g.chunk_updates {
                0 => labelled("Chunk Updates", "Unlimited"),
                n => labelled("Chunk Updates", lang::tr("%s per frame", &[&n])),
            },
            H::UploadBudget => match g.upload_budget_kib {
                0 => labelled("Upload Budget", "Unlimited"),
                k if k >= 1024 => {
                    labelled("Upload Budget", lang::tr("%s MiB/frame", &[&(k / 1024)]))
                }
                k => labelled("Upload Budget", lang::tr("%s KiB/frame", &[&k])),
            },
            H::Sky => labelled("Sky", on_off(g.sky)),
            H::Fog => labelled("Fog", on_off(g.fog)),
            H::Weather => labelled("Weather", on_off(g.weather)),
            H::AnimatedTextures => labelled("Animated Textures", on_off(g.animated_textures)),
            H::DitherCutoutMips => labelled("Distant Plant Dither", on_off(g.dither_cutout_mips)),
            H::PresentMode => labelled("Present Mode", g.present_mode.label()),
            H::Pacing => labelled(
                "Frame Pacing",
                match g.pacing {
                    Pacing::Efficient => "Efficient",
                    Pacing::Precise => "Precise",
                },
            ),
            H::Workers => match g.workers {
                0 => labelled(
                    "Worker Threads",
                    lang::tr("Auto (%s)", &[&g.worker_threads()]),
                ),
                n => labelled("Worker Threads", n),
            },
            H::ResolutionScale => labelled("Resolution Scale", format!("{}%", g.resolution_scale)),
            H::Antialiasing => labelled(
                "Anti-aliasing",
                match (g.fxaa, g.taa) {
                    (_, true) => "TAA",
                    (true, false) => "FXAA",
                    (false, false) => "Off",
                },
            ),
            H::Upscaling => labelled("Upscaling", g.upscaling.label()),
            H::ScreenshotQuality => {
                labelled("Screenshot Quality", format!("{}x", g.screenshot_quality))
            }
            H::FrameGeneration => labelled("Frame Generation", g.frame_generation.label()),
            H::Hdr => labelled("HDR", on_off(g.hdr)),
            H::HdrSettings => Text::literal(lang::get("HDR Settings...")),
            H::HdrCalibrate => Text::literal(lang::get(match setup(s) {
                Setup::Unknown => "Calibrate...",
                Setup::Automatic => "Calibrate Anyway...",
                Setup::Changed => "Use Automatic Setup",
            })),
            H::HdrPaperWhite => match g.hdr_paper_white {
                0 => labelled(
                    "Paper White",
                    lang::tr("Auto (%s)", &[&g.paper_white_nits()]),
                ),
                n => labelled("Paper White", n.to_string()),
            },
            H::HdrPeak => match g.hdr_peak {
                0 => labelled("Peak Brightness", lang::tr("Auto (%s)", &[&g.peak_nits()])),
                n => labelled("Peak Brightness", n.to_string()),
            },
            H::HdrBlackLevel => match g.hdr_black_level {
                0 => labelled(
                    "Black Level",
                    lang::tr("Auto (%s)", &[&format!("{:.2}", g.black_level_nits())]),
                ),
                n => labelled("Black Level", format!("{:.2}", n as f32 / 100.0)),
            },
            H::HdrUiBrightness => labelled("UI Brightness", format!("{}%", g.hdr_ui_brightness)),
            H::HdrKey => {
                Text::translate("Shortcut: %s", &[crate::keybinds::display_name(&g.hdr_key)])
            }
            H::BatteryMode => labelled("Battery Mode", on_off(g.battery)),
            H::ShadowCache => labelled("Shadow Cache", on_off(g.shadow_cache)),
            H::GraphicsApi => labelled("Graphics API", g.api.label()),
            // Greyed out where the device can't (`Item::blocked`): Off, as it is there,
            // while the saved value waits for a device that can.
            H::TranslucentBatching => labelled(
                "Translucent Batching",
                on_off(g.translucent_batching && crate::graphics::translucent_batching_offered()),
            ),
            H::Overlay => labelled(
                "Performance Overlay",
                match g.overlay {
                    Overlay::Off => "Off",
                    Overlay::Fps => "FPS",
                    Overlay::Graph => "FPS and Graph",
                },
            ),
        }
    }
    fn set_slider(self, s: &mut Settings, v: f32) {
        let g = &mut s.horizon;
        match self {
            H::ChunkUpdates => g.chunk_updates = (v * 64.0).round() as u32,
            H::UploadBudget => {
                g.upload_budget_kib =
                    UPLOAD_STEPS[(v * (UPLOAD_STEPS.len() - 1) as f32).round() as usize]
            }
            H::LodDistance => {
                g.lod_distance = LOD_STEPS[(v * (LOD_STEPS.len() - 1) as f32).round() as usize]
            }
            H::Workers => g.workers = (v * max_workers() as f32).round() as u32,
            H::ResolutionScale => g.resolution_scale = (50.0 + (v * 30.0).round() * 5.0) as u32,
            H::HdrPaperWhite => g.hdr_paper_white = (v * 48.0).round() as u32 * 10,
            H::HdrPeak => {
                g.hdr_peak = match (v * 37.0).round() as u32 {
                    0 => 0,
                    step => 150 + step * 50,
                }
            }
            H::HdrBlackLevel => g.hdr_black_level = (v * 100.0).round() as u32,
            H::HdrUiBrightness => g.hdr_ui_brightness = (25.0 + (v * 15.0).round() * 5.0) as u32,
            _ => {}
        }
    }
    fn cycle(self, s: &mut Settings, back: bool) {
        fn step<T: Copy + PartialEq>(all: &[T], now: T, back: bool) -> T {
            let i = all.iter().position(|x| *x == now).unwrap_or(0);
            let n = all.len();
            all[if back { (i + n - 1) % n } else { (i + 1) % n }]
        }
        if self == H::Preset {
            let preset = step(&Preset::ALL, s.horizon.preset, back);
            crate::graphics::apply_preset(preset, s);
            s.horizon.preset = preset;
            return;
        }
        let g = &mut s.horizon;
        match self {
            H::FarDetail => {
                g.far_detail = step(&[FarDetail::Full, FarDetail::Reduced], g.far_detail, back)
            }
            H::CaveCulling => g.cave_culling = !g.cave_culling,
            H::EntityCulling => {
                g.entity_culling = step(&EntityCullingMode::ALL, g.entity_culling, back)
            }
            H::LeafCulling => {
                g.leaf_culling = step(&LeafCullingMode::ALL, g.leaf_culling, back);
            }
            H::Sky => g.sky = !g.sky,
            H::Fog => g.fog = !g.fog,
            H::Weather => g.weather = !g.weather,
            H::AnimatedTextures => g.animated_textures = !g.animated_textures,
            H::DitherCutoutMips => g.dither_cutout_mips = !g.dither_cutout_mips,
            H::PresentMode => g.present_mode = step(&PresentMode::ALL, g.present_mode, back),
            // Off, FXAA, TAA.
            H::Antialiasing => {
                let i = if g.taa { 2 } else { usize::from(g.fxaa) };
                let i = (i + if back { 2 } else { 1 }) % 3;
                (g.fxaa, g.taa) = (i == 1, i == 2);
                if !g.taa {
                    g.upscaling = Upscaling::Native;
                }
            }
            // Upscaling is TAA's: picking a ratio turns it on.
            H::Upscaling => {
                g.upscaling = step(&Upscaling::ALL, g.upscaling, back);
                if g.upscaling != Upscaling::Native {
                    (g.fxaa, g.taa) = (false, true);
                }
            }
            H::ScreenshotQuality => {
                g.screenshot_quality = step(
                    &crate::graphics::SCREENSHOT_QUALITIES,
                    g.screenshot_quality,
                    back,
                )
            }
            H::FrameGeneration => {
                g.frame_generation = step(&FrameGeneration::ALL, g.frame_generation, back)
            }
            H::Hdr => g.hdr = !g.hdr,
            H::BatteryMode => g.battery = !g.battery,
            H::ShadowCache => g.shadow_cache = !g.shadow_cache,
            H::GraphicsApi => g.api = step(GraphicsApi::choices(), g.api, back),
            H::TranslucentBatching => g.translucent_batching = !g.translucent_batching,
            H::Pacing => {
                g.pacing = match g.pacing {
                    Pacing::Efficient => Pacing::Precise,
                    Pacing::Precise => Pacing::Efficient,
                }
            }
            H::Overlay => {
                g.overlay = step(
                    &[Overlay::Off, Overlay::Fps, Overlay::Graph],
                    g.overlay,
                    back,
                )
            }
            _ => {}
        }
    }
}
fn max_workers() -> u32 {
    std::thread::available_parallelism().map_or(8, |n| n.get() as u32)
}

impl Item {
    /// False for a setting whose system is not in Horizon yet (`Opt::available`).
    fn available(self) -> bool {
        !matches!(self, Item::Vanilla(o) if !o.available())
    }
    /// Why a control can't be used now, when it can't: HDR's screen opens only where
    /// the display offers HDR output, whatever `horizon-graphics.txt` says (a copied game
    /// folder, a display swapped for an SDR one: the renderer leaves HDR off there, and
    /// the saved values wait for an HDR display), and its sliders do something only
    /// while it is on; Translucent Batching switches only where the device can batch
    /// (`graphics::translucent_batching_offered`), its saved value waiting for one that can.
    fn blocked(self, s: &Settings) -> Option<&'static str> {
        if !self.available() {
            return Some("This feature is not yet available in Horizon.");
        }
        match self {
            Item::Horizon(H::HdrSettings | H::Hdr) if !crate::graphics::hdr_offered() => {
                Some("Your display or system doesn't offer HDR output.")
            }
            Item::Horizon(
                H::HdrCalibrate
                | H::HdrBlackLevel
                | H::HdrPeak
                | H::HdrPaperWhite
                | H::HdrUiBrightness,
            ) if !s.horizon.hdr => Some("Turn HDR on to use it."),
            Item::Horizon(H::TranslucentBatching)
                if !crate::graphics::translucent_batching_offered() =>
            {
                Some("Your GPU or graphics API doesn't support it.")
            }
            _ => None,
        }
    }
    /// On/off settings and their value.
    pub fn switch(self, s: &Settings) -> Option<bool> {
        let g = &s.horizon;
        Some(match self {
            Item::Horizon(H::CaveCulling) => g.cave_culling,
            Item::Horizon(H::Sky) => g.sky,
            Item::Horizon(H::Fog) => g.fog,
            Item::Horizon(H::Weather) => g.weather,
            Item::Horizon(H::AnimatedTextures) => g.animated_textures,
            Item::Horizon(H::DitherCutoutMips) => g.dither_cutout_mips,
            Item::Horizon(H::Hdr) => g.hdr,
            Item::Horizon(H::BatteryMode) => g.battery,
            Item::Horizon(H::ShadowCache) => g.shadow_cache,
            Item::Horizon(H::TranslucentBatching) => g.translucent_batching,
            Item::Vanilla(Opt::ViewBobbing) => s.view_bobbing,
            Item::Vanilla(Opt::Vsync) => s.vsync,
            Item::Vanilla(Opt::Fullscreen) => s.fullscreen,
            Item::Vanilla(Opt::SmoothLighting) => s.smooth_lighting,
            Item::Vanilla(Opt::EntityShadows) => s.entity_shadows,
            Item::Vanilla(Opt::Vignette) => s.vignette,
            Item::Vanilla(Opt::Autosave) => s.autosave_indicator,
            Item::Vanilla(Opt::CutoutLeaves) => s.cutout_leaves,
            Item::Vanilla(Opt::ImprovedTransparency) => s.improved_transparency,
            _ => return None,
        })
    }
    pub fn slider(self, s: &Settings) -> Option<f32> {
        match self {
            Item::Vanilla(o) => o.slider_position(s),
            Item::Horizon(h) => h.slider(s),
        }
    }
    pub fn label(self, s: &Settings) -> Text {
        match self {
            Item::Vanilla(Opt::RenderDistance) if s.horizon.lod_distance > 0 => labelled(
                "Full Detail Distance",
                lang::tr("%s chunks", &[&s.render_distance]),
            ),
            Item::Vanilla(o) => o.label(s),
            Item::Horizon(h) => h.label(s),
        }
    }
    pub fn set_slider(self, s: &mut Settings, v: f32) {
        if !self.available() {
            return;
        }
        match self {
            Item::Vanilla(o) => o.set_slider(s, v),
            Item::Horizon(h) => h.set_slider(s, v),
        }
        self.touched(s);
    }
    pub fn cycle(self, s: &mut Settings, back: bool, framebuffer: [i32; 2]) {
        if !self.available() {
            return;
        }
        match self {
            Item::Vanilla(o) => o.cycle(s, back, framebuffer),
            Item::Horizon(h) => h.cycle(s, back),
        }
        self.touched(s);
    }
    /// A preset no longer describes the settings once one of them changes.
    fn touched(self, s: &mut Settings) {
        let presetless = matches!(
            self,
            Item::Horizon(
                H::Preset
                    | H::Overlay
                    | H::ScreenshotQuality
                    | H::Pacing
                    | H::PresentMode
                    | H::Workers
                    | H::FrameGeneration
                    | H::BatteryMode
                    | H::ShadowCache
                    | H::GraphicsApi
                    | H::TranslucentBatching
            ) | Item::Vanilla(
                Opt::GuiScale
                    | Opt::Fullscreen
                    | Opt::Vsync
                    | Opt::MaxFps
                    | Opt::Fov
                    | Opt::Gamma
                    | Opt::InactivityFps
                    | Opt::ViewBobbing
                    | Opt::ChunkFade
                    | Opt::Vignette
                    | Opt::Autosave
                    | Opt::AttackIndicator
            )
        );
        if !presetless {
            s.horizon.preset = Preset::Custom;
        }
    }
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            Item::Vanilla(o) => o.slider_steps(),
            Item::Horizon(h) => h.slider_steps(),
        }
    }
    pub fn step_slider(self, s: &mut Settings, forward: bool, width: f32) {
        if !self.available() || self.blocked(s).is_some() {
            return;
        }
        let Some(pos) = self.slider(s) else {
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
        self.touched(s);
    }
}

/// What the options screens must do after an interaction here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ButtonClick,
    Changed,
    SliderDrag,
    OpenAdvanced,
    OpenSimple,
    OpenHdr,
    /// Wait for a key or mouse button to bind to this name.
    BindKey(&'static str),
    /// Start HDR calibration.
    Calibrate,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Item(Item),
    Button(Action),
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    target: Target,
    /// In the scrolling list (clipped to it).
    list: bool,
}

#[derive(Default)]
pub struct GraphicsUi {
    /// The list's scroll amount and its held scrollbar (`AbstractScrollArea`).
    scroll: ui::Scroll,
    dragging: Option<Item>,
    pub focus: Option<Target>,
    pub can_change_value: bool,
    /// The key binding the options screen is waiting on (`Action::BindKey`).
    pub binding: Option<&'static str>,
}

impl GraphicsUi {
    pub fn reset(&mut self) {
        self.scroll = ui::Scroll::default();
        self.dragging = None;
        self.focus = None;
        self.can_change_value = false;
    }
    /// The scrollbar's column: right of the descriptions.
    fn scrollbar_x(w: f32) -> f32 {
        STYLE.row_x(w) + STYLE.control_width + STYLE.gap + STYLE.text_width(w) + 6.0
    }
    fn bounds(h: f32) -> (f32, f32) {
        (STYLE.band, h - STYLE.band)
    }
    fn content_height(level: Level, s: &Settings) -> f32 {
        extra_rows(level, s).iter().fold(4.0, |height, row| {
            height
                + match row {
                    Row::Header(_) => STYLE.header_height,
                    Row::Setting(_) => STYLE.row_height,
                }
        })
    }
    /// Widgets and the list's content height, using a bounded offset.
    fn layout(
        &mut self,
        level: Level,
        s: &Settings,
        w: f32,
        h: f32,
    ) -> (Vec<Widget>, Vec<(f32, &'static str)>, f32) {
        let content = Self::content_height(level, s);
        self.clamp_scroll(h, content);
        let (top, _) = Self::bounds(h);
        let x = STYLE.row_x(w);
        let mut y = top + 4.0 - self.scroll.amount;
        let mut widgets = Vec::new();
        let mut headers = Vec::new();
        for row in extra_rows(level, s) {
            match row {
                Row::Header(name) => {
                    headers.push((y, name));
                    y += STYLE.header_height;
                }
                Row::Setting(s) => {
                    widgets.push(Widget {
                        x,
                        y,
                        w: STYLE.control_width,
                        target: Target::Item(s.item),
                        list: true,
                    });
                    y += STYLE.row_height;
                }
            }
        }
        let cx = (w / 2.0).floor();
        let by = h - 27.0;
        let other = match level {
            Level::Simple => Action::OpenAdvanced,
            Level::Advanced => Action::OpenSimple,
            // One Done, back to Advanced.
            Level::Hdr => {
                widgets.push(Widget {
                    x: cx - 100.0,
                    y: by,
                    w: 200.0,
                    target: Target::Button(Action::Done),
                    list: false,
                });
                return (widgets, headers, content);
            }
        };
        widgets.push(Widget {
            x: cx - 155.0,
            y: by,
            w: 150.0,
            target: Target::Button(other),
            list: false,
        });
        widgets.push(Widget {
            x: cx + 5.0,
            y: by,
            w: 150.0,
            target: Target::Button(Action::Done),
            list: false,
        });
        (widgets, headers, content)
    }
    fn clamp_scroll(&mut self, h: f32, content: f32) {
        let (top, bottom) = Self::bounds(h);
        self.scroll.clamp(top, bottom, content);
    }
    pub fn draw(
        &mut self,
        level: Level,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        in_world: bool,
    ) {
        let (w, h) = (batch.width, batch.height);
        let (widgets, headers, content) = self.layout(level, s, w, h);
        let (top, bottom) = Self::bounds(h);
        STYLE.background(batch, atlas, top, bottom, in_world);
        let shown = |y: f32, height: f32| y >= top && y + height <= bottom;
        let x = STYLE.row_x(w);
        for (y, name) in headers {
            if shown(y, STYLE.header_height) {
                STYLE.header(batch, atlas, &Text::literal(lang::get(name)), x, y);
            }
        }
        let descriptions: Vec<(Cost, &'static str)> = extra_rows(level, s)
            .into_iter()
            .filter_map(|r| match r {
                Row::Setting(s) => Some((s.cost, s.text)),
                Row::Header(_) => None,
            })
            .collect();
        let mut index = 0;
        let mut tooltip = None;
        for wd in &widgets {
            let is_focused = self.focus == Some(wd.target);
            let hover = (ui::hovered(cursor, wd.x, wd.y, wd.w, 20.0)
                && (!wd.list || (cursor.1 >= top && cursor.1 < bottom)))
                || is_focused;
            match wd.target {
                Target::Item(item) => {
                    let (cost, text) = descriptions[index];
                    index += 1;
                    if !shown(wd.y, 20.0) {
                        continue;
                    }
                    let mut label = item.label(s);
                    if item == Item::Horizon(H::HdrKey) && self.binding.is_some() {
                        // KeyBindsScreen: "> key <" while it waits.
                        label = Text::translate(
                            "Shortcut: %s",
                            &[Text::literal("> ")
                                .append(crate::keybinds::display_name(&s.horizon.hdr_key))
                                .append(" <")],
                        );
                    }
                    let blocked = item.blocked(s);
                    let live = blocked.is_none();
                    if hover && let Some(why) = blocked {
                        tooltip = Some(atlas.font.split(&Text::literal(lang::get(why)), 170.0));
                    }
                    match item.slider(s) {
                        Some(value) => STYLE.slider_ext(
                            batch,
                            atlas,
                            (wd.x, wd.y, wd.w),
                            &label,
                            value,
                            hover || self.dragging == Some(item),
                            is_focused,
                            self.can_change_value && is_focused,
                            live,
                            self.dragging == Some(item),
                        ),
                        None => STYLE.button(
                            batch,
                            atlas,
                            (wd.x, wd.y, wd.w),
                            &label,
                            hover && live,
                            live,
                        ),
                    }
                    STYLE.description(
                        batch,
                        atlas,
                        wd.x + wd.w + STYLE.gap,
                        wd.y + 1.0,
                        cost,
                        description(item, s, text),
                    );
                }
                Target::Button(action) => {
                    let label = match action {
                        Action::OpenAdvanced => "Advanced...",
                        Action::OpenSimple => "Simple...",
                        _ => "Done",
                    };
                    STYLE.button(
                        batch,
                        atlas,
                        (wd.x, wd.y, wd.w),
                        &Text::literal(lang::get(label)),
                        hover,
                        true,
                    );
                }
            }
        }
        STYLE.frame(batch, atlas, top, bottom, in_world);
        ui::scrollbar(
            batch,
            atlas,
            Self::scrollbar_x(w),
            top,
            bottom,
            content,
            self.scroll.amount,
        );
        let title = match level {
            Level::Simple => "Graphics",
            Level::Advanced => "Graphics: Advanced",
            Level::Hdr => "Graphics: HDR",
        };
        STYLE.title(batch, atlas, &Text::literal(lang::get(title)));
        if let Some(lines) = tooltip {
            ui::tooltip(batch, atlas, &lines, cursor);
        }
    }
    /// A press (or a drag with `drag`) at `(x, y)`.
    pub fn click(
        &mut self,
        level: Level,
        s: &mut Settings,
        (x, y): (f32, f32),
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Action> {
        let (widgets, _, content) = self.layout(level, s, w, h);
        let (top, bottom) = Self::bounds(h);
        if drag {
            // AbstractScrollArea.mouseDragged: the held scrollbar follows the pointer.
            if self.scroll.held() {
                self.scroll.drag(y, top, bottom, content);
                return Vec::new();
            }
            if let Some(item) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(wd.target, Target::Item(i) if i == item))
            {
                item.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                return vec![Action::SliderDrag];
            }
            return Vec::new();
        }
        if right {
            return Vec::new();
        }
        // AbstractScrollArea.updateScrolling: a press on the scrollbar holds it, silently.
        if self
            .scroll
            .press((x, y), Self::scrollbar_x(w), top, bottom, content)
        {
            return Vec::new();
        }
        for wd in widgets {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, 20.0) {
                continue;
            }
            if wd.list && (y < top || y >= bottom || wd.y < top || wd.y + 20.0 > bottom) {
                continue;
            }
            self.focus = Some(wd.target);
            return match wd.target {
                Target::Button(action) => {
                    self.can_change_value = false;
                    vec![Action::ButtonClick, action]
                }
                // A disabled control does nothing: no sound, no preset change.
                Target::Item(item) if item.blocked(s).is_some() => Vec::new(),
                Target::Item(Item::Horizon(H::HdrSettings)) => {
                    self.can_change_value = false;
                    vec![Action::ButtonClick, Action::OpenHdr]
                }
                Target::Item(Item::Horizon(H::HdrCalibrate)) if setup(s) == Setup::Changed => {
                    s.horizon.hdr_peak = 0;
                    s.horizon.hdr_paper_white = 0;
                    vec![Action::ButtonClick, Action::Changed]
                }
                Target::Item(Item::Horizon(H::HdrCalibrate)) => {
                    vec![Action::ButtonClick, Action::Calibrate]
                }
                Target::Item(Item::Horizon(H::HdrKey)) => {
                    vec![
                        Action::ButtonClick,
                        Action::BindKey(crate::graphics::HDR_KEY),
                    ]
                }
                Target::Item(item) => {
                    if item.slider(s).is_some() {
                        self.can_change_value = true;
                        self.dragging = Some(item);
                        item.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                        vec![Action::SliderDrag]
                    } else {
                        self.can_change_value = false;
                        item.cycle(s, right, framebuffer);
                        vec![Action::ButtonClick, Action::Changed]
                    }
                }
            };
        }
        Vec::new()
    }
    /// Lets go of a slider (true: it plays its click) or the scrollbar.
    pub fn release(&mut self) -> bool {
        self.scroll.release();
        self.dragging.take().is_some()
    }
    pub fn scroll(&mut self, level: Level, s: &Settings, h: f32, amount: f32) {
        let content = Self::content_height(level, s);
        let (top, bottom) = Self::bounds(h);
        self.scroll.clamp(top, bottom, content);
        self.scroll.wheel(amount, 18.0, top, bottom, content);
    }

    /// Mouse wheel over a control cycles it (cycle button). Sliders and disabled
    /// rows fall through to scroll the page, matching Vanilla list behavior.
    /// **Horizon improvement (diverges from Vanilla 26.3):** when a slider or
    /// cycle item is keyboard-focused (`self.focus`), the wheel steps/cycles it
    /// regardless of cursor position.  Vanilla 26.3 does not intercept the
    /// wheel for sliders at all.
    pub fn wheel(
        &mut self,
        level: Level,
        s: &mut Settings,
        cursor: (f32, f32),
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Action> {
        // Horizon improvement: focused slider/cycle intercepts wheel first.
        if amount != 0.0
            && self.can_change_value
            && let Some(Target::Item(item)) = self.focus
            && item.available()
            && item.blocked(s).is_none()
        {
            if item.slider(s).is_some() {
                item.step_slider(s, amount > 0.0, w);
                return vec![Action::SliderDrag, Action::Changed];
            } else if !matches!(item, Item::Horizon(H::HdrSettings)) {
                item.cycle(s, amount > 0.0, framebuffer);
                return vec![Action::ButtonClick, Action::Changed];
            }
        }
        let (widgets, _, _) = self.layout(level, s, w, h);
        let (top, bottom) = Self::bounds(h);
        let hovered = widgets.iter().find(|wd| {
            ui::hovered(cursor, wd.x, wd.y, wd.w, 20.0)
                && (!wd.list
                    || (cursor.1 >= top
                        && cursor.1 < bottom
                        && wd.y >= top
                        && wd.y + 20.0 <= bottom))
        });
        if let Some(wd) = hovered
            && let Target::Item(item) = wd.target
            && item.available()
            && item.blocked(s).is_none()
            && item.slider(s).is_none()
            && !matches!(item, Item::Horizon(H::HdrSettings))
        {
            if amount != 0.0 {
                item.cycle(s, amount > 0.0, framebuffer);
                return vec![Action::ButtonClick, Action::Changed];
            }
            return Vec::new();
        }
        self.scroll(level, s, h, amount);
        Vec::new()
    }

    /// Keyboard navigation and value adjustments.
    pub fn key_modifiers(
        &mut self,
        level: Level,
        s: &mut Settings,
        key: winit::keyboard::KeyCode,
        shift: bool,
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
    ) -> (Vec<Action>, bool) {
        use winit::keyboard::KeyCode as K;
        let (widgets, _, content) = self.layout(level, s, w, h);
        let (top, bottom) = Self::bounds(h);
        let focusables: Vec<(Target, [f32; 4], bool)> = widgets
            .iter()
            .filter_map(|wd| match wd.target {
                Target::Item(item) if item.available() && item.blocked(s).is_none() => {
                    Some((wd.target, [wd.x, wd.y, wd.w, 20.0], wd.list))
                }
                Target::Button(_) => Some((wd.target, [wd.x, wd.y, wd.w, 20.0], wd.list)),
                _ => None,
            })
            .collect();

        if focusables.is_empty() {
            return (Vec::new(), false);
        }

        // 1. Tab / Shift+Tab navigation
        if key == K::Tab {
            let cur = self
                .focus
                .and_then(|f| focusables.iter().position(|&(t, ..)| t == f));
            let next_idx = match cur {
                Some(i) => {
                    if shift {
                        if i == 0 { focusables.len() - 1 } else { i - 1 }
                    } else {
                        (i + 1) % focusables.len()
                    }
                }
                None => {
                    if shift {
                        focusables.len() - 1
                    } else {
                        0
                    }
                }
            };
            let (target, [_, wy, _, wh], is_list) = focusables[next_idx];
            self.focus = Some(target);
            let is_slider = matches!(target, Target::Item(item) if item.slider(s).is_some());
            self.can_change_value = is_slider;
            if is_list {
                if wy < top + 4.0 {
                    self.scroll.amount -= top + 4.0 - wy;
                } else if wy + wh > bottom - 4.0 {
                    self.scroll.amount += wy + wh - (bottom - 4.0);
                }
                self.clamp_scroll(h, content);
            }
            return (Vec::new(), true);
        }

        // 2. Left / Right Arrow keys
        if matches!(key, K::ArrowLeft | K::ArrowRight) {
            let forward = key == K::ArrowRight;
            if let Some(Target::Item(item)) = self.focus
                && item.slider(s).is_some()
                && self.can_change_value
            {
                let width = widgets
                    .iter()
                    .find(|wd| wd.target == Target::Item(item))
                    .map_or(STYLE.control_width, |wd| wd.w);
                item.step_slider(s, forward, width);
                return (vec![Action::SliderDrag], true);
            }
            if let Some(cur_target) = self.focus
                && let Some(cur_pos) = focusables.iter().position(|&(t, ..)| t == cur_target)
            {
                let from = focusables[cur_pos].1;
                let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                let dir = if forward {
                    crate::gamepad::Button::DPadRight
                } else {
                    crate::gamepad::Button::DPadLeft
                };
                if let Some(next_idx) = crate::gamepad::focus_step_rect(from, dir, &rects) {
                    let (target, [_, wy, _, wh], is_list) = focusables[next_idx];
                    self.focus = Some(target);
                    self.can_change_value = false;
                    if is_list {
                        if wy < top + 4.0 {
                            self.scroll.amount -= top + 4.0 - wy;
                        } else if wy + wh > bottom - 4.0 {
                            self.scroll.amount += wy + wh - (bottom - 4.0);
                        }
                        self.clamp_scroll(h, content);
                    }
                    return (Vec::new(), true);
                }
            }
            return (Vec::new(), false);
        }

        // 3. Up / Down Arrow keys
        if matches!(key, K::ArrowUp | K::ArrowDown) {
            let down = key == K::ArrowDown;
            if let Some(cur_target) = self.focus
                && let Some(cur_pos) = focusables.iter().position(|&(t, ..)| t == cur_target)
            {
                let from = focusables[cur_pos].1;
                let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                let dir = if down {
                    crate::gamepad::Button::DPadDown
                } else {
                    crate::gamepad::Button::DPadUp
                };
                if let Some(next_idx) = crate::gamepad::focus_step_rect(from, dir, &rects) {
                    let (target, [_, wy, _, wh], is_list) = focusables[next_idx];
                    self.focus = Some(target);
                    self.can_change_value = false;
                    if is_list {
                        if wy < top + 4.0 {
                            self.scroll.amount -= top + 4.0 - wy;
                        } else if wy + wh > bottom - 4.0 {
                            self.scroll.amount += wy + wh - (bottom - 4.0);
                        }
                        self.clamp_scroll(h, content);
                    }
                    return (Vec::new(), true);
                }
            } else if !focusables.is_empty() {
                let first = if down { 0 } else { focusables.len() - 1 };
                let (target, [_, wy, _, wh], is_list) = focusables[first];
                self.focus = Some(target);
                self.can_change_value = false;
                if is_list {
                    if wy < top + 4.0 {
                        self.scroll.amount -= top + 4.0 - wy;
                    } else if wy + wh > bottom - 4.0 {
                        self.scroll.amount += wy + wh - (bottom - 4.0);
                    }
                    self.clamp_scroll(h, content);
                }
                return (Vec::new(), true);
            }
            return (Vec::new(), false);
        }

        // 4. Space / Enter / NumpadEnter
        if matches!(key, K::Space | K::Enter | K::NumpadEnter)
            && let Some(target) = self.focus
        {
            match target {
                // A click focuses the greyed row; Enter there does nothing, as the click didn't.
                Target::Item(item @ Item::Horizon(H::TranslucentBatching))
                    if item.blocked(s).is_some() =>
                {
                    return (Vec::new(), true);
                }
                Target::Item(item) => {
                    if item.slider(s).is_some() {
                        self.can_change_value = !self.can_change_value;
                        return (Vec::new(), true);
                    } else if item == Item::Horizon(H::HdrSettings) {
                        return (vec![Action::ButtonClick, Action::OpenHdr], true);
                    } else {
                        item.cycle(s, shift, framebuffer);
                        return (vec![Action::ButtonClick, Action::Changed], true);
                    }
                }
                Target::Button(action) => {
                    return (vec![Action::ButtonClick, action], true);
                }
            }
        }

        // 5. Escape: the screen closes on the first press, as `Screen.keyPressed` handles it
        // before the focused slider.
        if key == K::Escape {
            self.can_change_value = false;
            return (Vec::new(), false);
        }

        (Vec::new(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_settings_open_where_hdr_is_offered() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (480.0, 2000.0);
        let at = |ui: &mut GraphicsUi, s: &Settings, level, item| {
            let (widgets, _, _) = ui.layout(level, s, size.0, size.1);
            let w = widgets
                .iter()
                .find(|w| matches!(w.target, Target::Item(i) if i == item))
                .unwrap();
            (w.x + 1.0, w.y + 1.0)
        };
        let button = at(&mut ui, &s, Level::Advanced, Item::Horizon(H::HdrSettings));
        let click = |ui: &mut GraphicsUi, s: &mut Settings, level, point| {
            ui.click(level, s, point, size, [480, 2000], false, false)
        };
        crate::graphics::offer_hdr(false);
        assert!(click(&mut ui, &mut s, Level::Advanced, button).is_empty());
        // Not even with HDR saved on, from another computer.
        s.horizon.hdr = true;
        assert!(click(&mut ui, &mut s, Level::Advanced, button).is_empty());
        s.horizon.hdr = false;
        crate::graphics::offer_hdr(true);
        assert_eq!(
            click(&mut ui, &mut s, Level::Advanced, button),
            [Action::ButtonClick, Action::OpenHdr]
        );
        // On HDR's screen, the sliders wait for the switch above them.
        let peak = at(&mut ui, &s, Level::Hdr, Item::Horizon(H::HdrPeak));
        assert!(click(&mut ui, &mut s, Level::Hdr, peak).is_empty());
        s.horizon.hdr = true;
        assert_eq!(
            click(&mut ui, &mut s, Level::Hdr, peak),
            [Action::SliderDrag]
        );
        let on = |level, item| {
            extra_rows(level, &s)
                .iter()
                .any(|r| matches!(r, Row::Setting(x) if x.item == Item::Horizon(item)))
        };
        assert!(on(Level::Hdr, H::Hdr) && !on(Level::Advanced, H::Hdr));
        assert!(on(Level::Hdr, H::HdrPeak) && !on(Level::Advanced, H::HdrPeak));
        for item in [H::HdrBlackLevel, H::HdrPaperWhite, H::HdrUiBrightness] {
            assert!(on(Level::Hdr, item) && !on(Level::Advanced, item));
        }
    }
    #[test]
    fn graphics_buttons_sound_on_press_and_sliders_on_release() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (480.0, 320.0);
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Button(_)))
            .unwrap();
        let point = (button.x + 1.0, button.y + 1.0);
        assert!(
            ui.click(
                Level::Advanced,
                &mut s,
                point,
                size,
                [854, 480],
                false,
                true
            )
            .is_empty()
        );
        assert_eq!(
            ui.click(
                Level::Advanced,
                &mut s,
                point,
                size,
                [854, 480],
                false,
                false
            )[0],
            Action::ButtonClick
        );
        assert!(!ui.release());
        let slider = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(i) if i.slider(&s).is_some()))
            .unwrap();
        assert_eq!(
            ui.click(
                Level::Advanced,
                &mut s,
                (slider.x + 1.0, slider.y + 1.0),
                size,
                [854, 480],
                false,
                false
            ),
            vec![Action::SliderDrag]
        );
        assert_eq!(
            ui.click(
                Level::Advanced,
                &mut s,
                (-100.0, -100.0),
                size,
                [854, 480],
                true,
                false
            ),
            vec![Action::SliderDrag]
        );
        assert!(ui.release());
        assert!(!ui.release());
    }

    #[test]
    fn scrolling_stops_at_both_limits_before_layout() {
        let s = Settings::default();
        for level in [Level::Simple, Level::Advanced] {
            let mut ui = GraphicsUi::default();
            let h = 2.0 * STYLE.band + 10.0;
            let (top, bottom) = GraphicsUi::bounds(h);
            let max = GraphicsUi::content_height(level, &s) - (bottom - top);
            assert!(max > 4.5);
            for _ in 0..3 {
                ui.scroll(level, &s, h, 1000.0);
                assert_eq!(ui.scroll.amount, 0.0);
            }
            for _ in 0..3 {
                ui.scroll(level, &s, h, -1000.0);
                assert_eq!(ui.scroll.amount, max);
            }
            ui.scroll(level, &s, h, 0.25);
            assert_eq!(ui.scroll.amount, max - 4.5);
            ui.scroll(level, &s, h, 1000.0);
            ui.scroll(level, &s, h, -0.25);
            assert_eq!(ui.scroll.amount, 4.5);
        }
    }

    #[test]
    fn the_scrollbar_can_be_held_and_dragged() {
        // #3552: AbstractScrollArea.updateScrolling holds the scrollbar on a press over its
        // column, mouseDragged moves the list with the pointer, onRelease lets go.
        let mut s = Settings::default();
        let size = (480.0, 2.0 * STYLE.band + 60.0);
        let (top, bottom) = GraphicsUi::bounds(size.1);
        // On the scrollbar column, as `draw` paints it.
        let x =
            STYLE.row_x(size.0) + STYLE.control_width + STYLE.gap + STYLE.text_width(size.0) + 8.0;
        let first_row = |ui: &mut GraphicsUi| {
            ui.layout(Level::Advanced, &Settings::default(), size.0, size.1)
                .0[0]
                .y
        };
        let mut ui = GraphicsUi::default();
        let start = first_row(&mut ui);
        let at = |y| (x, y);
        let press = |ui: &mut GraphicsUi, s: &mut Settings, y, drag| {
            ui.click(Level::Advanced, s, at(y), size, [854, 480], drag, false)
        };
        // The press itself makes no sound and changes nothing.
        assert!(press(&mut ui, &mut s, top + 5.0, false).is_empty());
        assert!(press(&mut ui, &mut s, top + 15.0, true).is_empty());
        let moved = first_row(&mut ui);
        assert!(moved < start, "dragging down scrolls the list down");
        // Past the bottom of the list: the last row.
        press(&mut ui, &mut s, bottom + 50.0, true);
        let content = GraphicsUi::content_height(Level::Advanced, &s);
        assert_eq!(first_row(&mut ui), start - (content - (bottom - top)));
        // Above the list: the first row again.
        press(&mut ui, &mut s, top - 50.0, true);
        assert_eq!(first_row(&mut ui), start);
        // Released: drags no longer scroll, and no slider click plays.
        assert!(!ui.release());
        press(&mut ui, &mut s, bottom + 50.0, true);
        assert_eq!(first_row(&mut ui), start);
    }

    #[test]
    fn resizing_clamps_before_positioning_graphics_rows() {
        let s = Settings::default();
        for level in [Level::Simple, Level::Advanced] {
            let mut ui = GraphicsUi::default();
            ui.scroll(level, &s, 2.0 * STYLE.band + 10.0, -1000.0);
            assert!(ui.scroll.amount > 0.0);
            let (widgets, headers, _) = ui.layout(level, &s, 480.0, 4000.0);
            assert_eq!(ui.scroll.amount, 0.0);
            if let Some((y, _)) = headers.first() {
                assert_eq!(*y, STYLE.band + 4.0);
            }
            let (expected, _, _) = GraphicsUi::default().layout(level, &s, 480.0, 4000.0);
            assert_eq!(
                widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
                expected.iter().map(|w| w.y).collect::<Vec<_>>()
            );
            ui.scroll(level, &s, 4000.0, -1000.0);
            assert_eq!(ui.scroll.amount, 0.0);
        }
    }

    #[test]
    fn classic_submenus_only_add_horizon_settings_without_empty_groups() {
        for level in [Level::Simple, Level::Advanced] {
            let rows = extra_rows(level, &Settings::default());
            assert!(rows.iter().any(|row| matches!(row, Row::Setting(_))));
            for (i, row) in rows.iter().enumerate() {
                match row {
                    Row::Setting(s) => assert!(matches!(s.item, Item::Horizon(_))),
                    Row::Header(_) => assert!(matches!(rows.get(i + 1), Some(Row::Setting(_)))),
                }
            }
            assert!(
                super::rows(level).iter().any(
                    |row| matches!(row, Row::Setting(s) if matches!(s.item, Item::Vanilla(_)))
                )
            );
        }
    }

    #[test]
    fn hdr_setup_follows_what_the_system_reported() {
        let mut g = crate::graphics::Graphics::default();
        assert_eq!(setup_of(&g, false), Setup::Unknown);
        assert_eq!(setup_of(&g, true), Setup::Automatic);
        g.hdr_paper_white = 80;
        assert_eq!(setup_of(&g, true), Setup::Changed);
        assert_eq!(setup_of(&g, false), Setup::Unknown);
    }

    #[test]
    fn every_advanced_setting_has_a_description() {
        for level in [Level::Simple, Level::Advanced, Level::Hdr] {
            for row in rows(level) {
                if let Row::Setting(s) = row {
                    assert!(!s.text.is_empty(), "{:?}", s.item);
                    assert!(s.text.len() < 80, "one line: {:?}", s.item);
                }
            }
        }
    }

    #[test]
    fn lod_distance_slides_in_both_levels_without_changing_near_distance() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LodDistance);
        let near = s.render_distance;
        assert_eq!(item.label(&s).plain(), "LOD Distance: Off");
        assert_eq!(item.slider(&s), Some(0.0));
        for level in [Level::Simple, Level::Advanced] {
            let items: Vec<_> = rows(level)
                .into_iter()
                .filter_map(|row| match row {
                    Row::Setting(setting) => Some(setting.item),
                    Row::Header(_) => None,
                })
                .collect();
            let render = items
                .iter()
                .position(|i| *i == Item::Vanilla(Opt::RenderDistance))
                .unwrap();
            assert_eq!(items[render + 1], item);
        }
        // Every stop is reachable and reads back as the same position.
        for (i, &distance) in LOD_STEPS.iter().enumerate() {
            let at = i as f32 / (LOD_STEPS.len() - 1) as f32;
            item.set_slider(&mut s, at);
            assert_eq!(s.horizon.lod_distance, distance);
            assert_eq!(item.slider(&s), Some(at));
            assert_eq!(s.render_distance, near);
        }
        // The ends are off and the whole world, which the far terrain plans.
        item.set_slider(&mut s, 0.0);
        assert_eq!(s.horizon.lod_distance, 0);
        item.set_slider(&mut s, 1.0);
        assert_eq!(s.horizon.lod_distance, crate::lod::MAX_CHUNKS);
        assert_eq!(
            item.label(&s).plain(),
            format!("LOD Distance: {} chunks", crate::lod::MAX_CHUNKS)
        );
        item.set_slider(&mut s, 5.0 / 17.0);
        assert_eq!(s.horizon.lod_distance, 1024);
        assert_eq!(item.label(&s).plain(), "LOD Distance: 1024 chunks");
        // A distance between stops (from the file) sits on the next one.
        s.horizon.lod_distance = 3000;
        assert_eq!(item.slider(&s), Some(7.0 / 17.0));
        assert_eq!(
            Item::Vanilla(Opt::RenderDistance).label(&s).plain(),
            format!("Full Detail Distance: {near} chunks")
        );
    }

    #[test]
    fn presets_set_vanilla_and_horizon_options() {
        let mut s = Settings::default();
        Item::Horizon(H::Preset).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!(s.render_distance, 8);
        assert_eq!(s.clouds, "false");
        // Any quality change leaves the preset.
        Item::Vanilla(Opt::Clouds).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Custom);
        // Display preferences keep it.
        Item::Horizon(H::Preset).cycle(&mut s, true, [854, 480]);
        Item::Vanilla(Opt::Vsync).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Ultra);
    }

    #[test]
    fn screenshot_quality_cycles_1_2_4_8_and_keeps_the_preset() {
        let mut s = Settings::default();
        Item::Horizon(H::Preset).cycle(&mut s, false, [854, 480]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            Item::Horizon(H::ScreenshotQuality).cycle(&mut s, false, [854, 480]);
            seen.push(s.horizon.screenshot_quality);
        }
        assert_eq!(seen, [2, 4, 8, 1]);
        Item::Horizon(H::ScreenshotQuality).cycle(&mut s, true, [854, 480]);
        assert_eq!(s.horizon.screenshot_quality, 8);
        assert_eq!(s.horizon.preset, Preset::Performance);
    }

    #[test]
    fn upload_budget_slider_steps() {
        let mut s = Settings::default();
        H::UploadBudget.set_slider(&mut s, 1.0);
        assert_eq!(s.horizon.upload_budget_kib, 16384);
        assert_eq!(H::UploadBudget.slider(&s), Some(1.0));
        H::UploadBudget.set_slider(&mut s, 0.0);
        assert_eq!(s.horizon.upload_budget_kib, 0);
    }

    #[test]
    fn antialiasing_cycles_off_fxaa_taa() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::Antialiasing);
        for label in ["FXAA", "TAA", "Off"] {
            item.cycle(&mut s, false, [854, 480]);
            assert_eq!(item.label(&s).plain(), format!("Anti-aliasing: {label}"));
        }
        item.cycle(&mut s, true, [854, 480]);
        assert_eq!((s.horizon.fxaa, s.horizon.taa), (false, true));

        // Upscaling needs TAA: a ratio turns it on, leaving TAA goes back to Native.
        s.horizon.taa = false;
        let upscaling = Item::Horizon(H::Upscaling);
        upscaling.cycle(&mut s, false, [854, 480]);
        assert_eq!(upscaling.label(&s).plain(), "Upscaling: Quality");
        assert!(s.horizon.taa);
        item.cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.upscaling, Upscaling::Native);

        // The Upscaling row is only on the screen while TAA is on.
        let shown = |s: &Settings| {
            extra_rows(Level::Advanced, s)
                .iter()
                .any(|row| matches!(row, Row::Setting(r) if r.item == upscaling))
        };
        assert!(!shown(&s));
        s.horizon.taa = true;
        assert!(shown(&s));
    }

    #[test]
    fn leaf_culling_cycles_modes_in_the_advanced_culling_group() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LeafCulling);
        assert_eq!(item.switch(&s), None);
        assert_eq!(item.label(&s).plain(), "Leaf Culling: Off");
        for label in ["Solid", "Solid Aggressive", "Hollow", "Off"] {
            item.cycle(&mut s, false, [854, 480]);
            assert_eq!(item.label(&s).plain(), format!("Leaf Culling: {label}"));
            assert_eq!(
                crate::graphics::Graphics::parse(&s.horizon.serialize()).leaf_culling,
                s.horizon.leaf_culling
            );
        }
        item.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Hollow);

        // The three culling settings share one group, below Rendering's chunk
        // updates and above Quality.
        let rows = rows(Level::Advanced);
        let culling = rows
            .iter()
            .position(|row| matches!(row, Row::Header("Culling")))
            .unwrap();
        assert!(matches!(
            rows.get(culling + 1),
            Some(Row::Setting(setting)) if setting.item == Item::Horizon(H::CaveCulling)
        ));
        assert!(matches!(
            rows.get(culling + 2),
            Some(Row::Setting(setting)) if setting.item == Item::Horizon(H::EntityCulling)
        ));
        assert!(matches!(
            rows.get(culling + 3),
            Some(Row::Setting(setting)) if setting.item == item
        ));
        assert!(matches!(
            rows.get(culling + 4),
            Some(Row::Header("Quality"))
        ));

        // The description names what the picked mode does.
        for mode in LeafCullingMode::ALL {
            s.horizon.leaf_culling = mode;
            let text = description(item, &s, "fallback");
            assert_ne!(text, "fallback", "{mode:?}");
            assert!(text.len() < 80, "{mode:?}: {text}");
        }

        // A click cycles it and leaves the preset, as the other quality
        // settings do.
        let mut ui = GraphicsUi::default();
        let size = (480.0, 4000.0);
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let control = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(i) if i == item))
            .unwrap();
        s.horizon.preset = Preset::Balanced;
        s.horizon.leaf_culling = LeafCullingMode::Off;
        assert_eq!(
            ui.click(
                Level::Advanced,
                &mut s,
                (control.x + 1.0, control.y + 1.0),
                size,
                [854, 480],
                false,
                false
            ),
            vec![Action::ButtonClick, Action::Changed]
        );
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Solid);
        assert_eq!(s.horizon.preset, Preset::Custom);
    }

    #[test]
    fn entity_culling_modes_are_opt_in_and_cycle_from_advanced_graphics() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::EntityCulling);
        assert_eq!(s.horizon.entity_culling, EntityCullingMode::Off);
        assert_eq!(item.label(&s).plain(), "Entity Culling: Off");
        for mode in EntityCullingMode::ALL {
            s.horizon.entity_culling = mode;
            for preset in Preset::ALL {
                crate::graphics::apply_preset(preset, &mut s);
                assert_eq!(s.horizon.entity_culling, mode);
            }
        }

        let mut ui = GraphicsUi::default();
        let size = (480.0, 4000.0);
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let switch = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(i) if i == item))
            .unwrap();
        let point = (switch.x + 1.0, switch.y + 1.0);
        for mode in [
            EntityCullingMode::Off,
            EntityCullingMode::Synchronous,
            EntityCullingMode::Asynchronous,
        ] {
            s.horizon.preset = Preset::Balanced;
            assert_eq!(
                ui.click(
                    Level::Advanced,
                    &mut s,
                    point,
                    size,
                    [854, 480],
                    false,
                    false
                ),
                vec![Action::ButtonClick, Action::Changed]
            );
            assert_eq!(s.horizon.entity_culling, mode);
            assert_eq!(s.horizon.preset, Preset::Custom);
        }
        assert_eq!(item.label(&s).plain(), "Entity Culling: Asynchronous");
    }

    /// Translucent Batching: a switch on Advanced (not Simple), on by default, no part of a
    /// preset, and greyed out, reading Off, where the device can't batch.
    #[test]
    fn translucent_batching_is_an_advanced_switch_where_the_device_can() {
        use winit::keyboard::KeyCode as K;
        let item = Item::Horizon(H::TranslucentBatching);
        let has = |level| {
            rows(level)
                .iter()
                .any(|r| matches!(r, Row::Setting(x) if x.item == item))
        };
        if cfg!(target_os = "macos") {
            // Metal draws flat: never offered, so not shown.
            assert!(!has(Level::Advanced));
            return;
        }
        assert!(has(Level::Advanced) && !has(Level::Simple));
        let mut s = Settings::default();
        s.horizon.preset = Preset::Balanced;
        assert_eq!(item.switch(&s), Some(true));
        let mut ui = GraphicsUi::default();
        let size = (480.0, 2000.0);
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let w = widgets
            .iter()
            .find(|w| w.target == Target::Item(item))
            .unwrap();
        let point = (w.x + 1.0, w.y + 1.0);
        let click = |ui: &mut GraphicsUi, s: &mut Settings| {
            ui.click(Level::Advanced, s, point, size, [480, 2000], false, false)
        };
        // Where the device can't, the saved On reads Off, and neither a click nor Enter
        // after it (the click focuses the row) does anything: no sound, no change, no save.
        crate::graphics::offer_translucent_batching(false);
        assert!(item.blocked(&s).is_some());
        assert_eq!(item.label(&s).plain(), "Translucent Batching: Off");
        assert!(click(&mut ui, &mut s).is_empty());
        let enter = ui.key_modifiers(Level::Advanced, &mut s, K::Enter, false, size, [480, 2000]);
        assert!(enter.0.is_empty());
        assert!(s.horizon.translucent_batching);
        // Where it can, a click switches it and keeps the preset.
        crate::graphics::offer_translucent_batching(true);
        assert_eq!(item.label(&s).plain(), "Translucent Batching: On");
        assert_eq!(
            click(&mut ui, &mut s),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(!s.horizon.translucent_batching);
        assert_eq!(item.label(&s).plain(), "Translucent Batching: Off");
        assert_eq!(s.horizon.preset, Preset::Balanced);
        // No preset sets it.
        for on in [false, true] {
            s.horizon.translucent_batching = on;
            for preset in Preset::ALL {
                crate::graphics::apply_preset(preset, &mut s);
                assert_eq!(s.horizon.translucent_batching, on, "{preset:?}");
            }
        }
    }

    #[test]
    fn graphics_slider_keyboard_navigation_and_selection_toggle_parity() {
        use winit::keyboard::KeyCode as K;
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (480.0, 600.0);
        let fb = [854, 480];

        s.horizon.chunk_updates = 10;
        s.horizon.resolution_scale = 100;

        // 1. Tab navigates down to Chunk Updates slider (in Level::Advanced)
        let mut found_chunk_updates = false;
        for _ in 0..15 {
            ui.key_modifiers(Level::Advanced, &mut s, K::Tab, false, size, fb);
            if ui.focus == Some(Target::Item(Item::Horizon(H::ChunkUpdates))) {
                found_chunk_updates = true;
                break;
            }
        }
        assert!(found_chunk_updates);
        assert!(
            ui.can_change_value,
            "Tab onto slider activates can_change_value"
        );

        // 2. Right Arrow steps chunk updates from 10 to 11
        let (actions, handled) =
            ui.key_modifiers(Level::Advanced, &mut s, K::ArrowRight, false, size, fb);
        assert!(handled);
        assert_eq!(actions, vec![Action::SliderDrag]);
        assert_eq!(s.horizon.chunk_updates, 11);

        // 3. Left Arrow steps back to 10
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowLeft, false, size, fb);
        assert_eq!(s.horizon.chunk_updates, 10);

        // 4. Space toggles can_change_value to false
        ui.key_modifiers(Level::Advanced, &mut s, K::Space, false, size, fb);
        assert!(!ui.can_change_value);

        // 5. Space toggles can_change_value back to true
        ui.key_modifiers(Level::Advanced, &mut s, K::Enter, false, size, fb);
        assert!(ui.can_change_value);

        // 6. Escape is left to the screen, which closes, even while a slider changes values
        let (actions, handled) =
            ui.key_modifiers(Level::Advanced, &mut s, K::Escape, false, size, fb);
        assert!(!handled);
        assert!(actions.is_empty());
        assert!(!ui.can_change_value);

        // 8. Tab to ResolutionScale (steps by 5%)
        let mut found_res = false;
        for _ in 0..20 {
            ui.key_modifiers(Level::Advanced, &mut s, K::Tab, false, size, fb);
            if ui.focus == Some(Target::Item(Item::Horizon(H::ResolutionScale))) {
                found_res = true;
                break;
            }
        }
        assert!(found_res);
        assert!(ui.can_change_value);
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowRight, false, size, fb);
        assert_eq!(s.horizon.resolution_scale, 105);
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowLeft, false, size, fb);
        assert_eq!(s.horizon.resolution_scale, 100);
    }

    #[test]
    fn graphics_cycle_wheel_cycles_and_slider_falls_through_to_scroll() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (480.0, 300.0);
        let fb = [854, 480];

        // 1. Wheel on a Cycle button: Far Detail cycles mode
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let far = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(Item::Horizon(H::FarDetail))))
            .unwrap();
        let cursor = (far.x + 5.0, far.y + 5.0);
        s.horizon.far_detail = FarDetail::Full;
        let actions = ui.wheel(Level::Advanced, &mut s, cursor, size, fb, 1.0);
        assert_eq!(actions, vec![Action::ButtonClick, Action::Changed]);
        assert_eq!(s.horizon.far_detail, FarDetail::Reduced);

        // 2. Wheel over a slider: Chunk Updates scrolls the page, doesn't change slider value
        s.horizon.chunk_updates = 16;
        let (widgets, _, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let cu = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(Item::Horizon(H::ChunkUpdates))))
            .unwrap();
        let cursor = (cu.x + 5.0, cu.y + 5.0);
        let scroll_before = ui.scroll.amount;
        let actions = ui.wheel(Level::Advanced, &mut s, cursor, size, fb, -1.0);
        assert!(actions.is_empty());
        assert_eq!(s.horizon.chunk_updates, 16);
        assert!(ui.scroll.amount > scroll_before);
    }

    #[test]
    fn graphics_step_slider_matches_set_slider_for_all_options() {
        let options = [
            H::ChunkUpdates,
            H::UploadBudget,
            H::Workers,
            H::ResolutionScale,
            H::HdrPaperWhite,
            H::HdrPeak,
            H::HdrBlackLevel,
            H::HdrUiBrightness,
        ];
        for h in options {
            let item = Item::Horizon(h);
            let steps = h.slider_steps().expect("steps defined");
            for i in 0..steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                s1.horizon.hdr = true;
                s2.horizon.hdr = true;
                h.set_slider(&mut s1, pos);

                item.step_slider(&mut s1, true, 200.0);
                let next_pos = (i + 1) as f32 / steps as f32;
                h.set_slider(&mut s2, next_pos);
                assert_eq!(
                    item.slider(&s1),
                    item.slider(&s2),
                    "{h:?} step forward from {i}"
                );
            }
            for i in 1..=steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                s1.horizon.hdr = true;
                s2.horizon.hdr = true;
                h.set_slider(&mut s1, pos);

                item.step_slider(&mut s1, false, 200.0);
                let prev_pos = (i - 1) as f32 / steps as f32;
                h.set_slider(&mut s2, prev_pos);
                assert_eq!(
                    item.slider(&s1),
                    item.slider(&s2),
                    "{h:?} step backward from {i}"
                );
            }
        }
    }
}
