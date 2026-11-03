//! Horizon's graphics settings screen: every video setting, vanilla's and
//! Horizon's, on one list of pages (General, Quality, Performance, Advanced,
//! HDR) with a page list beside it, a search field over it and a description
//! of the hovered setting with its cost. Which settings it shows, in what
//! pages, with what description and cost, and its input live here; how it
//! looks comes from `graphics_style`.
use crate::{
    graphics::{
        EntityCullingMode, FarDetail, FrameGeneration, GraphicsApi, LeafCullingMode, Overlay,
        Pacing, PresentMode, Preset, Upscaling,
    },
    graphics_style::{self as style, Cost},
    hud::{HudAtlas, HudBatch},
    lang,
    settings::{Opt, Settings},
    text::Text,
    ui,
};

/// The screen's pages, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Quality,
    Performance,
    Advanced,
    /// HDR output's own settings.
    Hdr,
}
impl Page {
    pub const ALL: [Page; 5] = [
        Page::General,
        Page::Quality,
        Page::Performance,
        Page::Advanced,
        Page::Hdr,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Quality => "Quality",
            Page::Performance => "Performance",
            Page::Advanced => "Advanced",
            Page::Hdr => "HDR",
        }
    }
    /// What the page holds, in the strip while no setting is pointed at.
    pub fn about(self) -> &'static str {
        match self {
            Page::General => "How far you see, the window and the HUD.",
            Page::Quality => "The sky and weather, lighting, textures and the image.",
            Page::Performance => "Chunk loading, culling and power use.",
            Page::Advanced => "How frames reach the screen, and the performance overlay.",
            Page::Hdr => "Brighter-than-white output on an HDR display.",
        }
    }
}

/// Horizon's own settings (`graphics::Graphics`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H {
    Preset,
    LodDistance,
    FarDetail,
    BlueMapAutoDetect,
    BlueMapRadius,
    CaveCulling,
    EntityCulling,
    LeafCulling,
    CacheShulkerBoxes,
    CacheSigns,
    CacheDecoratedPots,
    CacheBells,
    CacheChests,
    CacheBanners,
    CacheCopperGolemStatues,
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

#[derive(Clone, Copy)]
pub struct Setting {
    pub item: Item,
    pub cost: Cost,
    pub text: &'static str,
}
fn v(opt: Opt, cost: Cost, text: &'static str) -> Setting {
    Setting {
        item: Item::Vanilla(opt),
        cost,
        text,
    }
}
fn h(opt: H, cost: Cost, text: &'static str) -> Setting {
    Setting {
        item: Item::Horizon(opt),
        cost,
        text,
    }
}

/// Every page's settings, in named groups of related rows.
pub fn rows() -> Vec<(Page, Vec<(&'static str, Vec<Setting>)>)> {
    use Cost::*;
    // Suggested, never switched on, where far terrain costs the GPU most.
    let far = if crate::graphics::reduced_far_detail_suggested() {
        "Reduced draws far terrain coarser: faster on this integrated GPU."
    } else {
        "Reduced draws far terrain coarser, for GPUs it slows down."
    };
    let mut pages = vec![
        (
            Page::General,
            vec![
                (
                    "Preset",
                    vec![h(
                        H::Preset,
                        None,
                        "Sets the options below together. Changing one makes it Custom.",
                    )],
                ),
                (
                    "Distance",
                    vec![
                        v(
                            Opt::RenderDistance,
                            High,
                            "Full-detail terrain range. LOD Distance can extend local worlds farther.",
                        ),
                        h(
                            H::LodDistance,
                            Medium,
                            "Coarse distant terrain in local worlds. Integrated GPUs stop at 128 chunks.",
                        ),
                        h(H::FarDetail, Medium, far),
                        h(
                            H::BlueMapAutoDetect,
                            None,
                            "On servers, finds their BlueMap web map for far terrain. Next join.",
                        ),
                        h(
                            H::BlueMapRadius,
                            None,
                            "How far around you BlueMap tiles are downloaded. Next join.",
                        ),
                        v(
                            Opt::SimulationDistance,
                            Medium,
                            "How far mobs, crops and machines keep working. CPU time.",
                        ),
                        v(Opt::Gamma, None, "Brightens dark places."),
                    ],
                ),
                (
                    "Display",
                    vec![
                        v(Opt::GuiScale, None, "Size of menus and the HUD."),
                        v(Opt::Fullscreen, None, "Fills the screen (borderless)."),
                        v(
                            Opt::Vsync,
                            None,
                            "Waits for the display's refresh: no tearing, a little latency.",
                        ),
                        v(
                            Opt::MaxFps,
                            None,
                            "Caps frames per second; a cap saves power and heat.",
                        ),
                        v(
                            Opt::InactivityFps,
                            None,
                            "Lowers the frame rate when minimised, or idle too.",
                        ),
                        v(
                            Opt::Fov,
                            Low,
                            "Field of view. Wider shows more terrain at once.",
                        ),
                    ],
                ),
                (
                    "Interface",
                    vec![
                        v(Opt::ViewBobbing, None, "Sways the view while walking."),
                        v(Opt::AttackIndicator, None, "Shows attack cooldown."),
                        v(Opt::Autosave, None, "Shows when the world saves."),
                    ],
                ),
            ],
        ),
        (
            Page::Quality,
            vec![
                (
                    "Sky and weather",
                    vec![
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
                    ],
                ),
                (
                    "Lighting and entities",
                    vec![
                        v(
                            Opt::SmoothLighting,
                            Low,
                            "Soft shadows in corners. Costs chunk building time only.",
                        ),
                        v(
                            Opt::BiomeBlend,
                            Low,
                            "Blends grass and water colours across biomes when building chunks.",
                        ),
                        v(
                            Opt::EntityDistance,
                            Medium,
                            "How far away mobs and items are drawn, as a share of normal.",
                        ),
                        v(
                            Opt::EntityShadows,
                            Low,
                            "Round shadows under mobs and items.",
                        ),
                        v(Opt::Vignette, None, "Darkens the screen's edges."),
                        v(Opt::ChunkFade, None, "Fades newly loaded chunks in."),
                    ],
                ),
                (
                    "Textures",
                    vec![
                        v(
                            Opt::Mipmaps,
                            None,
                            "Smaller copies of block textures for distant terrain: less shimmer.",
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
                    ],
                ),
                (
                    "Image",
                    vec![
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
                    ],
                ),
            ],
        ),
        (
            Page::Performance,
            vec![
                (
                    "Chunk loading",
                    vec![
                        v(
                            Opt::ChunkBuilder,
                            Low,
                            "Which chunk rebuilds wait for the frame: none, nearby or yours.",
                        ),
                        h(
                            H::ChunkUpdates,
                            Low,
                            "Meshed sections uploaded per frame. Lower is smoother while loading.",
                        ),
                        h(
                            H::UploadBudget,
                            Low,
                            "Mesh data sent to the GPU per frame. Lower is smoother while loading.",
                        ),
                        h(
                            H::Workers,
                            None,
                            "Threads that generate, light and mesh chunks. Next start.",
                        ),
                    ],
                ),
                (
                    "Culling",
                    vec![
                        h(
                            H::CaveCulling,
                            Saves,
                            "Skips terrain hidden behind closed sections. Off costs a lot.",
                        ),
                        h(
                            H::EntityCulling,
                            None,
                            "Hides entities behind opaque blocks.",
                        ),
                        h(
                            H::LeafCulling,
                            Saves,
                            "Draws or drops leaf faces inside canopies. Off keeps vanilla's fancy leaves.",
                        ),
                        h(
                            H::ShadowCache,
                            Saves,
                            "Reuses the pack shadow map when sun, view and world match. Off by default.",
                        ),
                    ],
                ),
                (
                    "Block Entities",
                    vec![
                        h(
                            H::CacheShulkerBoxes,
                            Saves,
                            "Reuses settled boxes; lid movement stays animated.",
                        ),
                        h(
                            H::CacheSigns,
                            Saves,
                            "Reuses sign text until text, light, or font changes.",
                        ),
                        h(
                            H::CacheDecoratedPots,
                            Saves,
                            "Reuses still pots; break wobble and sherd changes redraw.",
                        ),
                        h(
                            H::CacheBells,
                            Saves,
                            "Reuses still bells; ringing stays animated.",
                        ),
                        h(
                            H::CacheChests,
                            Saves,
                            "Reuses closed chests; moving lids stay animated.",
                        ),
                        h(
                            H::CacheBanners,
                            Saves,
                            "Reuses poles; flags and pattern layers still update.",
                        ),
                        h(
                            H::CacheCopperGolemStatues,
                            Saves,
                            "Reuses statue geometry until its block state changes.",
                        ),
                    ],
                ),
                (
                    "Power",
                    vec![h(
                        H::BatteryMode,
                        None,
                        "60 FPS cap (30 in menus or unfocused), half the workers: less power.",
                    )],
                ),
            ],
        ),
        (
            Page::Advanced,
            vec![
                (
                    "Frames",
                    vec![
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
                    ],
                ),
                (
                    "Overlay",
                    vec![h(
                        H::Overlay,
                        None,
                        "Frame rate and frame times over the game, with a graph.",
                    )],
                ),
            ],
        ),
        (
            Page::Hdr,
            vec![
                (
                    "Output",
                    vec![
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
                    ],
                ),
                (
                    "Brightness",
                    vec![
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
                    ],
                ),
                (
                    "Shortcut",
                    vec![h(
                        H::HdrKey,
                        None,
                        "A key that opens this screen from the game. Not bound at first.",
                    )],
                ),
            ],
        ),
    ];
    // Only Windows and Linux have more than one graphics API, and translucent batching
    // (Metal draws flat).
    if cfg!(target_os = "macos") {
        for (_, groups) in &mut pages {
            for (_, rows) in groups {
                rows.retain(|r| {
                    !matches!(
                        r.item,
                        Item::Horizon(H::GraphicsApi | H::TranslucentBatching)
                    )
                });
            }
        }
    }
    pages
}

/// BlueMap download radii, in chunks.
const BLUE_MAP_RADII: [u32; 5] = [16, 32, 64, 128, 256];
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
        (Item::Horizon(H::EntityCulling), _) => match s.horizon.entity_culling {
            EntityCullingMode::Off => "No entity occlusion culling.",
            EntityCullingMode::Synchronous => "CPU occlusion test runs this frame.",
            EntityCullingMode::Asynchronous => "Occlusion checks run on a worker thread.",
        },
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
/// "Caption: value" (`options.generic_value`), both looked up in the language table.
fn labelled(caption: &str, value: impl std::fmt::Display) -> Text {
    let value = value.to_string();
    Text::option_name_value(
        Text::literal(lang::get(caption)),
        Text::literal(lang::get(&value)),
    )
}
/// "Caption: 50%" (`options.percent_value`, Options.percentValueLabel): French and German put a
/// space before the `%`.
fn labelled_percent(caption: &str, percent: u32) -> Text {
    Text::translate_with_fallback(
        "options.percent_value",
        "%s: %s%%",
        &[
            Text::literal(lang::get(caption)),
            Text::literal(percent.to_string()),
        ],
    )
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
            H::BlueMapAutoDetect => labelled("Find BlueMap", on_off(g.blue_map.auto_detect)),
            H::BlueMapRadius => labelled(
                "BlueMap Radius",
                lang::tr("%s chunks", &[&g.blue_map.radius_chunks()]),
            ),
            H::CaveCulling => labelled("Cave Culling", on_off(g.cave_culling)),
            H::EntityCulling => labelled("Entity Culling", g.entity_culling.label()),
            H::LeafCulling => labelled("Leaf Culling", g.leaf_culling.label()),
            H::CacheShulkerBoxes => labelled("Shulker Boxes", on_off(g.cache_shulker_boxes)),
            H::CacheSigns => labelled("Signs", on_off(g.cache_signs)),
            H::CacheDecoratedPots => labelled("Decorated Pots", on_off(g.cache_decorated_pots)),
            H::CacheBells => labelled("Bells", on_off(g.cache_bells)),
            H::CacheChests => labelled("Chests", on_off(g.cache_chests)),
            H::CacheBanners => labelled("Banners", on_off(g.cache_banners)),
            H::CacheCopperGolemStatues => {
                labelled("Copper Golem Statues", on_off(g.cache_copper_golem_statues))
            }
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
            H::ResolutionScale => labelled_percent("Resolution Scale", g.resolution_scale),
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
            H::HdrUiBrightness => labelled_percent("UI Brightness", g.hdr_ui_brightness),
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
            H::BlueMapAutoDetect => g.blue_map.auto_detect = !g.blue_map.auto_detect,
            H::BlueMapRadius => {
                g.blue_map.radius = step(&BLUE_MAP_RADII, g.blue_map.radius_chunks(), back)
            }
            H::CaveCulling => g.cave_culling = !g.cave_culling,
            H::EntityCulling => {
                g.entity_culling = step(&EntityCullingMode::ALL, g.entity_culling, back)
            }
            H::LeafCulling => {
                g.leaf_culling = step(&LeafCullingMode::ALL, g.leaf_culling, back);
            }
            H::CacheShulkerBoxes => g.cache_shulker_boxes = !g.cache_shulker_boxes,
            H::CacheSigns => g.cache_signs = !g.cache_signs,
            H::CacheDecoratedPots => g.cache_decorated_pots = !g.cache_decorated_pots,
            H::CacheBells => g.cache_bells = !g.cache_bells,
            H::CacheChests => g.cache_chests = !g.cache_chests,
            H::CacheBanners => g.cache_banners = !g.cache_banners,
            H::CacheCopperGolemStatues => {
                g.cache_copper_golem_statues = !g.cache_copper_golem_statues
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
    /// Why a control can't be used now, when it can't: HDR's page works only where
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
            Item::Horizon(
                H::Hdr
                | H::HdrCalibrate
                | H::HdrBlackLevel
                | H::HdrPeak
                | H::HdrPaperWhite
                | H::HdrUiBrightness,
            ) if !crate::graphics::hdr_offered() => {
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
    /// A value stepped between `<` `>`: not a tick box, a slider or a press.
    fn steps(self, s: &Settings) -> bool {
        self.switch(s).is_none()
            && self.slider(s).is_none()
            && !matches!(self, Item::Horizon(H::HdrKey | H::HdrCalibrate))
    }
    /// On/off settings and their value.
    pub fn switch(self, s: &Settings) -> Option<bool> {
        let g = &s.horizon;
        Some(match self {
            Item::Horizon(H::BlueMapAutoDetect) => g.blue_map.auto_detect,
            Item::Horizon(H::CaveCulling) => g.cave_culling,
            Item::Horizon(H::CacheShulkerBoxes) => g.cache_shulker_boxes,
            Item::Horizon(H::CacheSigns) => g.cache_signs,
            Item::Horizon(H::CacheDecoratedPots) => g.cache_decorated_pots,
            Item::Horizon(H::CacheBells) => g.cache_bells,
            Item::Horizon(H::CacheChests) => g.cache_chests,
            Item::Horizon(H::CacheBanners) => g.cache_banners,
            Item::Horizon(H::CacheCopperGolemStatues) => g.cache_copper_golem_statues,
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
                    | H::BlueMapAutoDetect
                    | H::BlueMapRadius
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

/// What the options screen must do after an interaction here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ButtonClick,
    Changed,
    SliderDrag,
    /// Wait for a key or mouse button to bind to this name.
    BindKey(&'static str),
    /// Start HDR calibration.
    Calibrate,
    Done,
}

/// What the keyboard or a controller can focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Item(Item),
    Page(Page),
    Search,
    Undo,
    Done,
}

/// A line of the list at `y` in its content: a page's name (only while
/// searching, which shows every page), a group's name with the height of the
/// panel its rows sit on, or a setting.
#[derive(Clone, Copy)]
struct Line {
    y: f32,
    page: Page,
    group: Option<(&'static str, f32)>,
    setting: Option<Setting>,
}
impl Line {
    fn h(&self) -> f32 {
        match (self.setting, self.group) {
            (Some(_), _) => style::ROW,
            (None, Some(_)) => style::GROUP_ROW,
            (None, None) => style::PAGE_ROW,
        }
    }
}

/// The screen's parts in GUI pixels. Down the left: the mark, the search
/// field, the page list, and Undo and Done at the foot. On the right: the
/// plate with the open page's name and its list, and the strip under it that
/// describes the hovered setting. `list_x`, `list_w`, `top` and `bottom` are
/// the list's view.
#[derive(Clone, Copy, Debug)]
struct Columns {
    side_x: f32,
    side_y: f32,
    plate: [f32; 4],
    dock: [f32; 4],
    list_x: f32,
    list_w: f32,
    top: f32,
    bottom: f32,
}
impl Columns {
    fn of(w: f32, h: f32) -> Self {
        let side_w = style::CONTROL_W;
        let gap = if w >= 560.0 { 16.0 } else { 8.0 };
        let pw = (w - 16.0 - side_w - gap).min(432.0).floor();
        let side_x = ((w - side_w - gap - pw) / 2.0).floor();
        let px = side_x + side_w + gap;
        // A short screen (a 720p window at scale 3) gives the margin to the list.
        let side_y = if h >= style::TITLED { 24.0 } else { 8.0 };
        let dock = [px, h - 8.0 - style::DOCK_H, pw, style::DOCK_H];
        let plate = [px, side_y, pw, dock[1] - 4.0 - side_y];
        let list_x = px + 8.0;
        Self {
            side_x,
            side_y,
            plate,
            dock,
            list_x,
            list_w: pw - 16.0 - style::SCROLLER_W - 2.0,
            top: side_y + 30.0,
            bottom: plate[1] + plate[3] - 6.0,
        }
    }
    /// The search field, under the mark.
    fn search(&self) -> [f32; 4] {
        [
            self.side_x,
            self.side_y + 26.0,
            style::CONTROL_W,
            style::SEARCH_H,
        ]
    }
    /// The search field's clear button, at its right end.
    fn clear(&self) -> [f32; 4] {
        let [x, y, w, h] = self.search();
        [x + w - h, y, h, h]
    }
    /// Room for vanilla's scroller, whose thumb is 32 high and 8 short of its track.
    fn scrolls(&self) -> bool {
        self.bottom - self.top >= 40.0
    }
    fn bar_x(&self) -> f32 {
        self.plate[0] + self.plate[2] - 8.0 - style::SCROLLER_W
    }
    /// A page in the page list, under the search field.
    fn page(&self, i: usize) -> [f32; 4] {
        [
            self.side_x,
            self.side_y + 56.0 + i as f32 * 18.0,
            style::CONTROL_W,
            16.0,
        ]
    }
    /// Done (`0`), and Undo over it (`1`): under the page list, level with the strip.
    fn button(&self, i: usize) -> [f32; 4] {
        let foot = self.dock[1] + self.dock[3] - style::BUTTON_H;
        let y = if i == 0 {
            foot
        } else {
            foot - 4.0 - style::BUTTON_H
        };
        [self.side_x, y, style::CONTROL_W, style::BUTTON_H]
    }
    /// A row's control column: its button, tick box or slider.
    fn track(&self) -> (f32, f32) {
        let x = self.list_x + self.list_w - style::INSET - style::CONTROL_W;
        (x, style::CONTROL_W)
    }
    /// A slider's value for the pointer at `x`: the handle's middle follows it.
    fn slider_at(&self, x: f32) -> f32 {
        let (tx, tw) = self.track();
        ((x - tx - 5.0) / (tw - 10.0)).clamp(0.0, 1.0)
    }
}

/// Whether a setting is on the screen now: Upscaling only while TAA is, which does it.
fn shown(item: Item, s: &Settings) -> bool {
    item != Item::Horizon(H::Upscaling) || s.horizon.taa
}
/// A control's label as its name and its value ("Render Distance" and "12 chunks").
fn parts(item: Item, s: &Settings, binding: bool) -> (String, Option<String>) {
    // The shortcut's label is one translated "Shortcut: %s"; the row shows its key apart,
    // as KeyBindsScreen does, "> key <" while it waits.
    if item == Item::Horizon(H::HdrKey) {
        let key = crate::keybinds::display_name(&s.horizon.hdr_key).plain();
        let key = if binding { format!("> {key} <") } else { key };
        return (lang::get("Shortcut"), Some(key));
    }
    let label = item.label(s).plain();
    // `options.generic_value` ("%s: %s") joins vanilla's; Horizon's use ": " too.
    let format = lang::get("options.generic_value");
    let sep = format
        .split("%s")
        .nth(1)
        .filter(|sep| !sep.is_empty() && label.contains(*sep))
        .unwrap_or(": ");
    match label.split_once(sep) {
        Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
        None => (label, None),
    }
}

pub struct GraphicsUi {
    /// The list's scroll amount and its held scrollbar (`AbstractScrollArea`).
    scroll: ui::Scroll,
    dragging: Option<Item>,
    pub focus: Option<Target>,
    pub can_change_value: bool,
    /// The key binding the options screen is waiting on (`Action::BindKey`).
    pub binding: Option<&'static str>,
    pub search: ui::EditBox,
    /// The page the plate shows, alone, while nothing is searched.
    page: Page,
    /// While searching, the page last picked in the page list: current until the list scrolls.
    picked: Option<Page>,
    /// The settings as the screen found them, for Undo, and whether they changed since.
    before: Option<Box<Settings>>,
    pub changed: bool,
}
impl Default for GraphicsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 0.0, 8.0);
        search.bordered = false;
        search.focused = false;
        search.message = lang::get("Search options");
        Self {
            scroll: ui::Scroll::default(),
            dragging: None,
            focus: None,
            can_change_value: false,
            binding: None,
            search,
            page: Page::General,
            picked: None,
            before: None,
            changed: false,
        }
    }
}

impl GraphicsUi {
    /// How far the list is scrolled (the options screen's tests).
    #[cfg(test)]
    pub(crate) fn scroll_amount(&self) -> f32 {
        self.scroll.amount
    }
    /// The screen opens, at `page`.
    pub fn open(&mut self, page: Page) {
        *self = Self {
            page,
            ..Self::default()
        };
    }
    fn query(&self) -> String {
        self.search.value.trim().to_lowercase()
    }
    /// The list's lines and its content height: the open page's settings in
    /// their groups or, while searching, the settings of every page whose name
    /// or description has the search in it, under their pages' names. Not
    /// their values (nor the descriptions that follow them), and the row being
    /// changed stays whatever it reads now: a change mustn't take its row away
    /// under the pointer.
    fn lines(&self, s: &Settings) -> (Vec<Line>, f32) {
        let query = self.query();
        let mut lines = Vec::new();
        let mut y: f32 = 0.0;
        for (page, groups) in rows() {
            if query.is_empty() && page != self.page {
                continue;
            }
            let groups: Vec<(&'static str, Vec<Setting>)> = groups
                .into_iter()
                .map(|(name, rows)| {
                    let rows = rows
                        .into_iter()
                        .filter(|r| shown(r.item, s))
                        .filter(|r| {
                            query.is_empty()
                                || self.focus == Some(Target::Item(r.item))
                                || self.dragging == Some(r.item)
                                || parts(r.item, s, false).0.to_lowercase().contains(&query)
                                || lang::get(r.text).to_lowercase().contains(&query)
                        })
                        .collect::<Vec<_>>();
                    (name, rows)
                })
                .filter(|(_, rows)| !rows.is_empty())
                .collect();
            if groups.is_empty() {
                continue;
            }
            if !query.is_empty() {
                if !lines.is_empty() {
                    y += style::PAGE_GAP;
                }
                lines.push(Line {
                    y,
                    page,
                    group: None,
                    setting: None,
                });
                y += style::PAGE_ROW;
            }
            for (name, rows) in groups {
                lines.push(Line {
                    y,
                    page,
                    group: Some((name, rows.len() as f32 * style::ROW)),
                    setting: None,
                });
                y += style::GROUP_ROW;
                for setting in rows {
                    lines.push(Line {
                        y,
                        page,
                        group: None,
                        setting: Some(setting),
                    });
                    y += style::ROW;
                }
                y += style::GROUP_GAP;
            }
        }
        (lines, (y - style::GROUP_GAP).max(0.0))
    }
    /// The columns and the list, with the scroll kept within it.
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Columns, Vec<Line>, f32) {
        let cols = Columns::of(w, h);
        let (lines, content) = self.lines(s);
        self.scroll.clamp(cols.top, cols.bottom, content);
        (cols, lines, content)
    }
    /// Pick `page`: the plate shows it alone or, while searching, scrolls to its matches.
    fn pick(&mut self, page: Page, lines: &[Line], cols: &Columns, content: f32) {
        self.page = page;
        self.scroll.amount = 0.0;
        if self.query().is_empty() {
            return;
        }
        if let Some(line) = lines.iter().find(|l| l.page == page) {
            self.scroll.amount = line.y;
            self.scroll.clamp(cols.top, cols.bottom, content);
            self.picked = Some(page);
        }
    }
    /// Whether the page list's `page` has anything to show.
    fn has(&self, page: Page, lines: &[Line]) -> bool {
        self.query().is_empty() || lines.iter().any(|l| l.page == page)
    }
    /// The page the plate shows: the open one or, while searching, the one
    /// picked, else the topmost line's.
    fn current(&self, lines: &[Line]) -> Option<Page> {
        if self.query().is_empty() {
            return Some(self.page);
        }
        self.picked.or_else(|| {
            lines
                .iter()
                .find(|l| l.y + l.h() > self.scroll.amount)
                .map(|l| l.page)
        })
    }
    fn row_y(&self, cols: &Columns, line: &Line) -> f32 {
        cols.top + line.y - self.scroll.amount
    }
    /// Remember the settings before the first change, for Undo.
    fn keep(&mut self, s: &Settings) {
        if self.before.is_none() {
            self.before = Some(Box::new(s.clone()));
        }
    }
    fn mark(&mut self, actions: &[Action]) {
        if actions.iter().any(|a| {
            matches!(
                a,
                Action::Changed | Action::SliderDrag | Action::BindKey(_) | Action::Calibrate
            )
        }) {
            self.changed = true;
        }
    }
    /// Undo: what this screen edits back to what it was when the screen
    /// opened: Horizon's graphics settings and the vanilla options on its rows.
    /// Not the other options (the narrator, or one `horizon-ctl options-set`
    /// changed meanwhile), nor fullscreen, which F11 and the window manager change.
    fn undo(&mut self, s: &mut Settings) {
        let Some(b) = self.before.as_deref() else {
            return;
        };
        s.horizon = b.horizon.clone();
        s.render_distance = b.render_distance;
        s.simulation_distance = b.simulation_distance;
        s.gamma = b.gamma;
        s.gui_scale = b.gui_scale;
        s.vsync = b.vsync;
        s.max_fps = b.max_fps;
        s.inactivity_fps_limit = b.inactivity_fps_limit.clone();
        s.fov = b.fov;
        s.view_bobbing = b.view_bobbing;
        s.attack_indicator = b.attack_indicator;
        s.autosave_indicator = b.autosave_indicator;
        s.clouds = b.clouds.clone();
        s.cloud_range = b.cloud_range;
        s.weather_radius = b.weather_radius;
        s.particles = b.particles;
        s.smooth_lighting = b.smooth_lighting;
        s.biome_blend = b.biome_blend;
        s.entity_distance = b.entity_distance;
        s.entity_shadows = b.entity_shadows;
        s.vignette = b.vignette;
        s.chunk_fade = b.chunk_fade;
        s.mipmap_levels = b.mipmap_levels;
        s.cutout_leaves = b.cutout_leaves;
        s.improved_transparency = b.improved_transparency;
        s.prioritize_chunk_updates = b.prioritize_chunk_updates;
        // Presets set it beside the options above.
        s.graphics_preset = b.graphics_preset.clone();
        self.changed = false;
    }

    pub fn draw(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        in_world: bool,
    ) {
        self.keep(s);
        let (w, h) = (batch.width, batch.height);
        let (cols, lines, content) = self.layout(s, w, h);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        style::fill(batch, atlas, [0.0, 0.0, w, h], style::VEIL);
        let current = self.current(&lines);
        self.draw_side(batch, atlas, &cols, &lines, current, cursor);
        self.draw_search(batch, atlas, &cols, cursor);

        // The plate, with the open page's name (or the search's) over its list.
        let [px, py, pw, _] = cols.plate;
        style::panel(batch, atlas, cols.plate);
        let query = self.search.value.trim();
        let heading = if query.is_empty() {
            lang::get(current.unwrap_or(self.page).name())
        } else {
            lang::get("Results for \"%s\"").replace("%s", query)
        };
        let heading = Text::literal(style::fit(atlas, &heading, pw - 24.0));
        batch.text_styled(atlas, &heading, px + 12.0, py + 10.0, style::WHITE, true);
        style::fill(
            batch,
            atlas,
            [px + 8.0, py + 24.0, pw - 16.0, 1.0],
            style::HEAD_RULE,
        );

        // The list, clipped to its view: each group's panel, then its rows on it.
        let over_list = cursor.1 >= cols.top && cursor.1 < cols.bottom;
        let start = batch.vertices.len();
        let mark = crate::widgets::targets_mark();
        let mut described = None;
        let shows = |y: f32, h: f32| y + h > cols.top && y < cols.bottom;
        for line in &lines {
            let y = self.row_y(&cols, line);
            if let Some((_, panel)) = line.group
                && shows(y + style::GROUP_ROW, panel)
            {
                let rect = [cols.list_x, y + style::GROUP_ROW, cols.list_w, panel];
                style::group(batch, atlas, rect);
            }
        }
        // A group's first row, which has no rule over it.
        let mut first = false;
        for line in &lines {
            let y = self.row_y(&cols, line);
            let after_name = std::mem::replace(&mut first, line.group.is_some());
            if !shows(y, line.h()) {
                continue;
            }
            let Some(setting) = line.setting else {
                let (name, color, dy) = match line.group {
                    Some((name, _)) => (name, style::MUTED, 3.0),
                    None => (line.page.name(), style::ACCENT, 4.0),
                };
                let name = Text::literal(lang::get(name));
                batch.text_styled(atlas, &name, cols.list_x + 2.0, y + dy, color, true);
                continue;
            };
            let item = setting.item;
            if !after_name {
                let rule = [cols.list_x + 8.0, y, cols.list_w - 16.0, 1.0];
                style::fill(batch, atlas, rule, style::ROW_RULE);
            }
            let rect = [cols.list_x, y, cols.list_w, style::ROW];
            let hover = over_list && ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3]);
            let focused = self.focus == Some(Target::Item(item));
            // The pointer's row wins over the keyboard's.
            if hover || self.dragging == Some(item) || (focused && described.is_none()) {
                described = Some(setting);
            }
            self.draw_row(batch, atlas, s, &cols, item, y, hover, focused);
        }
        if lines.is_empty() {
            let none = Text::literal(lang::get("No settings match."));
            batch.text_styled(
                atlas,
                &none,
                cols.list_x + 2.0,
                cols.top + 4.0,
                style::MUTED,
                true,
            );
        }
        batch.clip_since(
            start,
            cols.list_x,
            cols.top,
            cols.list_x + cols.list_w,
            cols.bottom,
        );
        crate::widgets::clip_targets(mark, cols.top, cols.bottom);
        self.draw_scroller(batch, atlas, &cols, content);
        self.draw_dock(batch, atlas, s, &cols, described, current);

        let hovered = |[x, y, w, h]: [f32; 4]| ui::hovered(cursor, x, y, w, h);
        let mut foot = vec![(cols.button(0), "Done", Target::Done)];
        // Undo only once there is something to undo.
        if self.changed {
            foot.push((cols.button(1), "Undo", Target::Undo));
        }
        for (rect, label, target) in foot {
            let label = Text::literal(lang::get(label));
            let focused = self.focus == Some(target);
            let hover = hovered(rect) || focused;
            crate::widgets::record(&label.plain(), rect, true);
            crate::widgets::narratable(|| {
                crate::widgets::Narratable::button(label.plain(), rect, true)
                    .hovered(hovered(rect))
                    .focused(focused)
            });
            crate::widgets::widget_cursor(hovered(rect), true);
            crate::widgets::target(rect);
            let primary = target == Target::Done;
            style::button(batch, atlas, rect, &label, (hover, true, primary));
        }
    }
    /// The mark over the search field, and the page list: the open page on a
    /// plate with the accent's bar beside it, the others bare text.
    fn draw_side(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cols: &Columns,
        lines: &[Line],
        current: Option<Page>,
        cursor: (f32, f32),
    ) {
        let (x, y) = (cols.side_x, cols.side_y);
        style::fill(batch, atlas, [x, y + 2.0, 6.0, 6.0], style::ACCENT);
        let horizon = Text::literal("Horizon");
        batch.text_styled(atlas, &horizon, x + 10.0, y, style::WHITE, true);
        let title = Text::literal(lang::get("Graphics"));
        batch.text_styled(atlas, &title, x, y + 10.0, style::MUTED, true);
        for (i, page) in Page::ALL.into_iter().enumerate() {
            let rect = cols.page(i);
            let [px, py, pw, ph] = rect;
            let has = self.has(page, lines);
            let open = current == Some(page);
            let name = Text::literal(lang::get(page.name()));
            crate::widgets::record(&name.plain(), rect, has);
            let hover = has && ui::hovered(cursor, px, py, pw, ph);
            let focused = self.focus == Some(Target::Page(page));
            crate::widgets::narratable(|| {
                crate::widgets::Narratable::button(name.plain(), rect, has)
                    .hovered(hover)
                    .focused(focused)
            });
            crate::widgets::widget_cursor(hover, has);
            if has {
                crate::widgets::target_as(rect, crate::widgets::Kind::Tab(open));
            }
            if open {
                style::fill(batch, atlas, rect, style::OPEN);
                style::fill(batch, atlas, [px, py, 2.0, ph], style::ACCENT);
            }
            let color = match (open || hover, has) {
                (true, _) => style::WHITE,
                (false, true) => style::SOFT,
                (false, false) => style::MUTED,
            };
            let ty = py + ((ph - 8.0) / 2.0).floor();
            let name = Text::literal(style::fit(atlas, &name.plain(), pw - 12.0));
            batch.text_styled(atlas, &name, px + 8.0, ty, color, true);
            if focused {
                style::ring(batch, atlas, rect, style::WHITE);
            }
        }
    }
    fn draw_search(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cols: &Columns,
        cursor: (f32, f32),
    ) {
        let rect = cols.search();
        style::well(batch, atlas, rect);
        let focused = self.focus == Some(Target::Search);
        self.search.focused = focused;
        let [x, y, w, h] = rect;
        let clear = cols.clear();
        (self.search.x, self.search.y) = (x + 5.0, y + ((h - 8.0) / 2.0).floor());
        self.search.width = clear[0] - self.search.x;
        self.search.text_color = style::WHITE;
        self.search.draw(batch, atlas, None);
        if self.search.value.is_empty() && !focused {
            let hint = Text::literal(self.search.message.clone());
            batch.text_styled(atlas, &hint, x + 5.0, self.search.y, style::MUTED, true);
        }
        if !self.search.value.is_empty() {
            let hover = ui::hovered(cursor, clear[0], clear[1], clear[2], clear[3]);
            crate::widgets::widget_cursor(hover, true);
            let color = if hover { style::WHITE } else { style::MUTED };
            let cy = y + ((h - 8.0) / 2.0).floor();
            ui::centered_text(
                batch,
                atlas,
                &Text::literal("×"),
                clear[0],
                cy,
                clear[2],
                color,
            );
        }
        if focused {
            style::ring(batch, atlas, [x, y, w, h], style::WHITE);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        s: &Settings,
        cols: &Columns,
        item: Item,
        y: f32,
        hover: bool,
        focused: bool,
    ) {
        let rect = [cols.list_x, y, cols.list_w, style::ROW];
        let live = item.blocked(s).is_none();
        let label = item.label(s).plain();
        let (name, value) = parts(item, s, self.binding.is_some());
        let (tx, tw) = cols.track();
        let right = tx + tw;
        let control = [
            tx,
            y + (style::ROW - style::CONTROL_H) / 2.0,
            tw,
            style::CONTROL_H,
        ];
        let active = live && (hover || focused || self.dragging == Some(item));
        if active {
            let lit = [rect[0] + 2.0, y + 1.0, rect[2] - 4.0, style::ROW - 2.0];
            style::fill(batch, atlas, lit, style::HOVER);
        }
        crate::widgets::widget_cursor(hover, live);
        let slider = item.slider(s);
        let switch = item.switch(s);
        // The control socket's widget tree, the narrator and a controller's focus.
        let kind = match slider {
            Some(value) => crate::widgets::Kind::Slider {
                value,
                track: [tx + 5.0, tw - 10.0],
            },
            None => crate::widgets::Kind::Button,
        };
        crate::widgets::record_as(&label, rect, live, kind);
        crate::widgets::narratable(|| {
            match (switch, slider) {
                (Some(on), _) => crate::widgets::Narratable::checkbox(name.clone(), on, rect, live),
                (_, Some(_)) => crate::widgets::Narratable::slider(label.clone(), rect, live),
                _ => crate::widgets::Narratable::button(label.clone(), rect, live),
            }
            .hovered(hover)
            .focused(focused)
        });
        if live {
            crate::widgets::target_as(rect, kind);
        }
        let color = if live { style::WHITE } else { style::MUTED };
        let soft = if live { style::SOFT } else { style::MUTED };
        let ty = y + ((style::ROW - 8.0) / 2.0).floor();
        let lit = live && (hover || self.dragging == Some(item));
        let left = rect[0] + style::INSET;
        // An action (Calibrate...) is a button across the row, as vanilla's lists have them.
        if switch.is_none() && slider.is_none() && value.is_none() {
            let wide = [left, control[1], right - left, style::CONTROL_H];
            style::button(batch, atlas, wide, &Text::literal(name), (lit, live, false));
        } else {
            let value = value.unwrap_or_default();
            let vw = atlas.font.width(&value);
            // Where the control starts, and the value's text for a slider at rest.
            let start = match (switch, slider) {
                (Some(_), _) => right - style::CONTROL_H,
                (None, Some(_)) if active => tx,
                (None, Some(_)) => right - vw,
                (None, None) => right - vw - 18.0,
            };
            match (switch, slider) {
                (Some(on), _) => style::checkbox(batch, atlas, right, y, on, live),
                // A slider shows its track, the value on it, while it is pointed at, focused or held.
                (None, Some(position)) => {
                    let value = Text::literal(value);
                    if active {
                        style::slider(batch, atlas, control, &value, position, (lit, live));
                    } else {
                        batch.text_styled(atlas, &value, start, ty, soft, true);
                    }
                }
                (None, None) => {
                    let value = Text::literal(value);
                    style::cycler(
                        batch,
                        atlas,
                        [start, y + 3.0, vw + 18.0, 14.0],
                        &value,
                        (lit, live),
                    );
                }
            }
            // The name, cut short before the control where a translation runs long.
            let shown = Text::literal(style::fit(atlas, &name, start - 6.0 - left));
            batch.text_styled(atlas, &shown, left, ty, color, true);
        }
        if focused {
            style::ring(
                batch,
                atlas,
                [rect[0] + 1.0, y, rect[2] - 2.0, style::ROW],
                style::WHITE,
            );
        }
    }
    /// Vanilla's scroller, inside the panel's right edge, sized as `ui::Scroll` drags it.
    fn draw_scroller(&self, batch: &mut HudBatch, atlas: &HudAtlas, cols: &Columns, content: f32) {
        if !cols.scrolls() {
            return;
        }
        let Some((y, bar)) =
            ui::scrollbar_thumb(cols.top, cols.bottom, content, self.scroll.amount)
        else {
            return;
        };
        let x = cols.bar_x();
        let track = [x, cols.top, style::SCROLLER_W, cols.bottom - cols.top];
        style::scroller(batch, atlas, track, [x, y, style::SCROLLER_W, bar]);
    }
    /// The strip under the plate: the hovered or focused setting's name and
    /// cost, then what it does (or why it can't be used now); with none, what
    /// the open page holds.
    fn draw_dock(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        s: &Settings,
        cols: &Columns,
        setting: Option<Setting>,
        current: Option<Page>,
    ) {
        let [x, y, w, _] = cols.dock;
        style::panel(batch, atlas, cols.dock);
        let left = x + 12.0;
        let (text, color) = match setting {
            Some(setting) => {
                let (name, _) = parts(setting.item, s, false);
                let cost = lang::get(setting.cost.label());
                let line = style::fit(atlas, &format!("{name} · {cost}"), w - 32.0);
                let line = Text::literal(line).with_color(setting.cost.rgb());
                let square = [left, y + 10.0, 4.0, 4.0];
                style::fill(batch, atlas, square, style::cost_color(setting.cost));
                batch.text_styled(atlas, &line, left + 8.0, y + 8.0, style::WHITE, true);
                match setting.item.blocked(s) {
                    Some(why) => (lang::get(why), style::MUTED),
                    None => (
                        lang::get(description(setting.item, s, setting.text)),
                        style::GREY,
                    ),
                }
            }
            None => {
                let Some(page) = current else { return };
                let name = Text::literal(lang::get(page.name()));
                batch.text_styled(atlas, &name, left, y + 8.0, style::WHITE, true);
                (lang::get(page.about()), style::GREY)
            }
        };
        let lines = atlas.font.split(&Text::literal(text), w - 24.0);
        for (i, line) in lines.iter().take(2).enumerate() {
            batch.text_styled(atlas, line, left, y + 20.0 + i as f32 * 10.0, color, true);
        }
    }

    /// A press (or a drag with `drag`) at `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn click(
        &mut self,
        s: &mut Settings,
        (x, y): (f32, f32),
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Action> {
        self.keep(s);
        let (cols, lines, content) = self.layout(s, w, h);
        let at = |[rx, ry, rw, rh]: [f32; 4]| ui::hovered((x, y), rx, ry, rw, rh);
        if drag {
            // AbstractScrollArea.mouseDragged: the held scrollbar follows the pointer.
            if self.scroll.held() && !cols.scrolls() {
                self.scroll.release();
            }
            if self.scroll.held() {
                self.picked = None;
                self.scroll.drag(y, cols.top, cols.bottom, content);
                return Vec::new();
            }
            if let Some(item) = self.dragging {
                item.set_slider(s, cols.slider_at(x));
                self.changed = true;
                return vec![Action::SliderDrag];
            }
            return Vec::new();
        }
        if right {
            return Vec::new();
        }
        if cols.scrolls()
            && self
                .scroll
                .press((x, y), cols.bar_x(), cols.top, cols.bottom, content)
        {
            return Vec::new();
        }
        if at(cols.search()) {
            if !self.search.value.is_empty() && at(cols.clear()) {
                self.search.set_value("");
                self.searched();
                return vec![Action::ButtonClick];
            }
            self.focus = Some(Target::Search);
            self.search.reset_blink();
            self.search.move_cursor_to_end(false);
            return Vec::new();
        }
        if self.focus == Some(Target::Search) {
            self.focus = None;
        }
        if at(cols.button(0)) {
            self.focus = Some(Target::Done);
            return vec![Action::ButtonClick, Action::Done];
        }
        if self.changed && at(cols.button(1)) {
            // Undo goes away with the change it undid.
            self.focus = None;
            self.undo(s);
            return vec![Action::ButtonClick, Action::Changed];
        }
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if at(cols.page(i)) && self.has(page, &lines) {
                self.focus = Some(Target::Page(page));
                self.pick(page, &lines, &cols, content);
                return vec![Action::ButtonClick];
            }
        }
        if y < cols.top || y >= cols.bottom {
            return Vec::new();
        }
        let Some(item) = lines.iter().find_map(|line| {
            let ry = self.row_y(&cols, line);
            let setting = line.setting?;
            at([cols.list_x, ry, cols.list_w, style::ROW]).then_some(setting.item)
        }) else {
            return Vec::new();
        };
        // A disabled control does nothing: no sound, no preset change.
        if item.blocked(s).is_some() {
            return Vec::new();
        }
        self.focus = Some(Target::Item(item));
        let actions = self.activate(s, item, false, framebuffer, Some((x, &cols)));
        self.mark(&actions);
        actions
    }
    /// Press `item`: `at` is where on its row and its slider's track (`None`
    /// from the keyboard).
    fn activate(
        &mut self,
        s: &mut Settings,
        item: Item,
        back: bool,
        framebuffer: [i32; 2],
        at: Option<(f32, &Columns)>,
    ) -> Vec<Action> {
        match item {
            Item::Horizon(H::HdrCalibrate) if setup(s) == Setup::Changed => {
                s.horizon.hdr_peak = 0;
                s.horizon.hdr_paper_white = 0;
                vec![Action::ButtonClick, Action::Changed]
            }
            Item::Horizon(H::HdrCalibrate) => vec![Action::ButtonClick, Action::Calibrate],
            Item::Horizon(H::HdrKey) => {
                vec![
                    Action::ButtonClick,
                    Action::BindKey(crate::graphics::HDR_KEY),
                ]
            }
            _ if item.slider(s).is_some() => {
                self.can_change_value = true;
                // A press on the track sets the value there; on the name, only focuses.
                match at {
                    Some((x, cols)) if x >= cols.track().0 => {
                        self.dragging = Some(item);
                        item.set_slider(s, cols.slider_at(x));
                        vec![Action::SliderDrag]
                    }
                    _ => Vec::new(),
                }
            }
            _ => {
                self.can_change_value = false;
                item.cycle(s, back, framebuffer);
                vec![Action::ButtonClick, Action::Changed]
            }
        }
    }
    /// Lets go of a slider (true: it plays its click) or the scrollbar.
    pub fn release(&mut self) -> bool {
        self.scroll.release();
        self.dragging.take().is_some()
    }
    /// The mouse wheel at the cursor. It scrolls the list, except that a hovered value
    /// stepped by `<` `>` takes the step (`CycleButton.mouseScrolled`: up goes back)
    /// while the "Scroll wheel changes settings" extra is on. A slider, a tick box and a
    /// focused row don't take it, wherever the focus came from: the keyboard and a
    /// controller step them with Left and Right.
    pub fn wheel(
        &mut self,
        s: &mut Settings,
        (w, h): (f32, f32),
        (x, y): (f32, f32),
        framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Action> {
        if amount != 0.0 && s.extras.wheel_cycles_settings {
            let (cols, lines, _) = self.layout(s, w, h);
            let hovered = (y >= cols.top && y < cols.bottom)
                .then(|| {
                    lines.iter().find_map(|line| {
                        let setting = line.setting?;
                        let ry = self.row_y(&cols, line);
                        ui::hovered((x, y), cols.list_x, ry, cols.list_w, style::ROW)
                            .then_some(setting.item)
                    })
                })
                .flatten();
            if let Some(item) = hovered
                && item.steps(s)
                && item.blocked(s).is_none()
            {
                item.cycle(s, amount > 0.0, framebuffer);
                self.changed = true;
                return vec![Action::ButtonClick, Action::Changed];
            }
        }
        self.scroll(s, (w, h), amount);
        Vec::new()
    }
    /// The mouse wheel scrolls the list.
    pub fn scroll(&mut self, s: &Settings, (w, h): (f32, f32), amount: f32) {
        let (cols, _, content) = self.layout(s, w, h);
        self.picked = None;
        self.scroll
            .wheel(amount, 18.0, cols.top, cols.bottom, content);
    }
    /// What Tab steps through, in order, with where each is on screen. The
    /// arrows (`near`) only reach the rows in view and the one next past either
    /// edge, which scrolls in: a row farther out sits under the tabs or the
    /// buttons and would take the step from them.
    fn focusables(
        &self,
        s: &Settings,
        cols: &Columns,
        lines: &[Line],
        near: bool,
    ) -> Vec<(Target, [f32; 4])> {
        let mut out = vec![(Target::Search, cols.search())];
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if self.has(page, lines) {
                out.push((Target::Page(page), cols.page(i)));
            }
        }
        // Every setting row, and whether it can take the focus.
        let rows: Vec<(Item, f32, bool)> = lines
            .iter()
            .filter_map(|line| {
                let setting = line.setting?;
                let live = setting.item.blocked(s).is_none();
                Some((setting.item, self.row_y(cols, line), live))
            })
            .collect();
        // For the arrows, the rows in view and the next live one past either
        // edge, over any greyed ones (HDR's page on a display without HDR).
        let in_view = |y: f32| y + style::ROW > cols.top && y < cols.bottom;
        let reach = match rows.iter().position(|r| in_view(r.1)) {
            Some(first) if near => {
                let last = rows.iter().rposition(|r| in_view(r.1)).unwrap_or(first);
                let lo = rows[..first].iter().rposition(|r| r.2).unwrap_or(first);
                let hi = rows[last + 1..]
                    .iter()
                    .position(|r| r.2)
                    .map_or(last, |i| last + 1 + i);
                lo..hi + 1
            }
            None if near => 0..0,
            _ => 0..rows.len(),
        };
        for &(item, y, live) in &rows[reach] {
            if !live {
                continue;
            }
            out.push((
                Target::Item(item),
                [cols.list_x, y, cols.list_w, style::ROW],
            ));
        }
        if self.changed {
            out.push((Target::Undo, cols.button(1)));
        }
        out.push((Target::Done, cols.button(0)));
        out
    }
    /// Focus `target`, scrolling its row into the list's view.
    fn focus_on(
        &mut self,
        s: &Settings,
        target: Target,
        rect: [f32; 4],
        cols: &Columns,
        content: f32,
    ) {
        self.focus = Some(target);
        self.can_change_value = matches!(target, Target::Item(i) if i.slider(s).is_some());
        if let Target::Item(_) = target {
            let [_, y, _, h] = rect;
            if y < cols.top {
                self.scroll.amount -= cols.top - y;
            } else if y + h > cols.bottom {
                self.scroll.amount += y + h - cols.bottom;
            }
            self.picked = None;
            self.scroll.clamp(cols.top, cols.bottom, content);
        }
    }

    /// Keys: typing in the search field, Tab and the arrows between controls,
    /// Left and Right on a slider, Enter or Space to press. False: the screen
    /// handles it (Escape closes it, as `Screen.keyPressed` does first).
    #[allow(clippy::too_many_arguments)]
    pub fn key_modifiers(
        &mut self,
        s: &mut Settings,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
        shift: bool,
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
    ) -> (Vec<Action>, bool) {
        use winit::keyboard::KeyCode as K;
        self.keep(s);
        if key == K::Escape {
            self.can_change_value = false;
            return (Vec::new(), false);
        }
        if self.focus == Some(Target::Search)
            && !matches!(key, K::Tab | K::ArrowDown | K::Enter | K::NumpadEnter)
        {
            let before = self.search.value.clone();
            if !self.search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                self.type_text(&typed);
            }
            if self.search.value != before {
                self.searched();
            }
            return (Vec::new(), true);
        }
        let (cols, lines, content) = self.layout(s, w, h);
        let near = key != K::Tab;
        let focusables = self.focusables(s, &cols, &lines, near);
        let current = self
            .focus
            .and_then(|f| focusables.iter().position(|&(t, _)| t == f));
        if key == K::Tab {
            let n = focusables.len();
            let next = match (current, shift) {
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
                (None, false) => 0,
                (None, true) => n - 1,
            };
            let (target, rect) = focusables[next];
            self.focus_on(s, target, rect, &cols, content);
            return (Vec::new(), true);
        }
        if matches!(key, K::ArrowLeft | K::ArrowRight)
            && self.can_change_value
            && let Some(Target::Item(item)) = self.focus
            && item.slider(s).is_some()
        {
            item.step_slider(s, key == K::ArrowRight, cols.track().1 + 8.0);
            self.changed = true;
            return (vec![Action::SliderDrag], true);
        }
        if matches!(
            key,
            K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight
        ) {
            use crate::gamepad::Button as B;
            let dir = match key {
                K::ArrowUp => B::DPadUp,
                K::ArrowDown => B::DPadDown,
                K::ArrowLeft => B::DPadLeft,
                _ => B::DPadRight,
            };
            let next = match current {
                Some(i) => {
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    crate::gamepad::focus_step_rect(focusables[i].1, dir, &rects)
                }
                // Nothing focused in view: going down, the first setting from the
                // view's top on (the next past its bottom where greyed rows fill it);
                // going up, the last one down to its bottom.
                None => {
                    let mut rows = focusables
                        .iter()
                        .enumerate()
                        .filter(|(_, (t, _))| matches!(t, Target::Item(_)));
                    let pick = if key == K::ArrowUp {
                        rows.rfind(|(_, (_, [_, y, ..]))| *y + style::ROW <= cols.bottom)
                    } else {
                        rows.find(|(_, (_, [_, y, ..]))| *y >= cols.top)
                    };
                    pick.map(|(i, _)| i)
                }
            };
            let Some(next) = next else {
                return (Vec::new(), false);
            };
            let (target, rect) = focusables[next];
            self.focus_on(s, target, rect, &cols, content);
            self.can_change_value = false;
            return (Vec::new(), true);
        }
        if matches!(key, K::Space | K::Enter | K::NumpadEnter)
            && let Some(target) = self.focus
        {
            let actions = match target {
                Target::Search => Vec::new(),
                Target::Page(page) => {
                    self.pick(page, &lines, &cols, content);
                    vec![Action::ButtonClick]
                }
                // A row greyed out while focused does nothing, as a click on it doesn't.
                Target::Item(item) if item.blocked(s).is_some() => Vec::new(),
                Target::Item(item) if item.slider(s).is_some() => {
                    self.can_change_value = !self.can_change_value;
                    Vec::new()
                }
                Target::Item(item) => self.activate(s, item, shift, framebuffer, None),
                Target::Undo => {
                    self.undo(s);
                    // Undo goes away with the change it undid.
                    self.focus = Some(Target::Done);
                    return (vec![Action::ButtonClick, Action::Changed], true);
                }
                Target::Done => vec![Action::ButtonClick, Action::Done],
            };
            self.mark(&actions);
            return (actions, true);
        }
        (Vec::new(), false)
    }
    /// Whether the search field takes the keys.
    pub fn typing(&self) -> bool {
        self.focus == Some(Target::Search)
    }
    /// Text typed into the search field (the keyboard, a controller's on-screen keyboard).
    pub fn type_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.search.insert_text(text);
            self.searched();
        }
    }
    /// Ctrl+A/C/V/X in the search field.
    pub fn shortcut(
        &mut self,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if !self.typing() {
            return false;
        }
        let before = self.search.value.clone();
        self.search.shortcut(shortcut, clipboard);
        if self.search.value != before {
            self.searched();
        }
        // Whatever it did: the key is the field's, not something to type.
        true
    }
    /// The search changed: the list starts over at its top.
    fn searched(&mut self) {
        self.scroll.amount = 0.0;
        self.picked = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    const SIZE: (f32, f32) = (640.0, 360.0);
    const FB: [i32; 2] = [1920, 1080];
    /// Held by the tests that set `offer_hdr`, which is global.
    static HDR_OFFER: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn items() -> Vec<(Page, Item)> {
        rows()
            .into_iter()
            .flat_map(|(page, groups)| {
                groups
                    .into_iter()
                    .flat_map(|(_, rows)| rows)
                    .map(move |r| (page, r.item))
            })
            .collect()
    }
    /// Where `item`'s row is on screen now (its page open and the list scrolled to it first).
    fn row(ui: &mut GraphicsUi, s: &Settings, item: Item) -> [f32; 4] {
        if ui.query().is_empty() {
            ui.page = items().into_iter().find(|(_, i)| *i == item).unwrap().0;
        }
        let (cols, lines, content) = ui.layout(s, SIZE.0, SIZE.1);
        let line = lines
            .iter()
            .find(|l| l.setting.is_some_and(|r| r.item == item))
            .copied()
            .unwrap();
        let rect = [cols.list_x, ui.row_y(&cols, &line), cols.list_w, style::ROW];
        ui.focus_on(s, Target::Item(item), rect, &cols, content);
        ui.focus = None;
        ui.can_change_value = false;
        [cols.list_x, ui.row_y(&cols, &line), cols.list_w, style::ROW]
    }
    fn press(ui: &mut GraphicsUi, s: &mut Settings, (x, y): (f32, f32)) -> Vec<Action> {
        ui.click(s, (x, y), SIZE, FB, false, false)
    }

    #[test]
    fn one_screen_holds_every_video_setting_once_with_a_short_description() {
        let all = items();
        for (i, (_, item)) in all.iter().enumerate() {
            assert!(!all[..i].iter().any(|(_, x)| x == item), "{item:?} twice");
        }
        // Vanilla's Video Settings options are here beside Horizon's.
        for opt in [
            Opt::RenderDistance,
            Opt::Vsync,
            Opt::GuiScale,
            Opt::Clouds,
            Opt::Mipmaps,
        ] {
            assert!(all.iter().any(|(_, i)| *i == Item::Vanilla(opt)), "{opt:?}");
        }
        for page in Page::ALL {
            assert!(all.iter().any(|(p, _)| *p == page), "{page:?} is empty");
        }
        for (_, groups) in rows() {
            for r in groups.into_iter().flat_map(|(_, rows)| rows) {
                assert!(!r.text.is_empty() && r.text.len() < 80, "{:?}", r.item);
            }
        }
    }

    #[test]
    fn a_picked_page_shows_alone_in_its_groups_from_the_top() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let settings = |ui: &mut GraphicsUi, s: &Settings| {
            let (_, lines, _) = ui.layout(s, SIZE.0, SIZE.1);
            assert!(
                lines[0].group.is_some(),
                "the list starts at a group's name"
            );
            lines
                .iter()
                .filter_map(|l| l.setting.map(|r| (l.page, r.item)))
                .collect::<Vec<_>>()
        };
        // Upscaling shows only with TAA, off by default.
        let on = |page| -> Vec<(Page, Item)> {
            items()
                .into_iter()
                .filter(|(p, i)| *p == page && *i != Item::Horizon(H::Upscaling))
                .collect()
        };
        assert_eq!(settings(&mut ui, &s), on(Page::General));
        ui.scroll(&s, SIZE, -3.0);
        assert!(ui.scroll.amount > 0.0);
        let [x, y, w, h] = Columns::of(SIZE.0, SIZE.1).page(1);
        assert_eq!(
            press(&mut ui, &mut s, (x + w / 2.0, y + h / 2.0)),
            [Action::ButtonClick]
        );
        assert_eq!(settings(&mut ui, &s), on(Page::Quality));
        assert_eq!(ui.scroll.amount, 0.0);
        // Groups sit apart: each name follows the gap under the panel before it.
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert_eq!(ui.current(&lines), Some(Page::Quality));
        let names: Vec<&Line> = lines.iter().filter(|l| l.group.is_some()).collect();
        assert!(names.len() > 1);
        for pair in names.windows(2) {
            let (_, panel) = pair[0].group.unwrap();
            assert_eq!(
                pair[1].y,
                pair[0].y + style::GROUP_ROW + panel + style::GROUP_GAP
            );
        }
        // Opened at a page (Advanced..., the HDR key), it shows that page.
        let mut ui = GraphicsUi::default();
        ui.open(Page::Hdr);
        assert_eq!(settings(&mut ui, &s), on(Page::Hdr));
    }

    #[test]
    fn a_720p_window_keeps_the_page_list_the_buttons_and_five_rows() {
        // 1280x720 at GUI scale 3.
        let size = (427.0, 240.0);
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let cols = Columns::of(size.0, size.1);
        assert!((cols.bottom - cols.top) / style::ROW >= 5.0);
        let [x, y, w, h] = cols.page(4);
        assert!(y >= 0.0 && x + w <= size.0);
        // Undo, over Done, stays clear of the page list.
        assert!(cols.button(1)[1] >= y + h);
        ui.click(&mut s, (x + w / 2.0, y + h / 2.0), size, FB, false, false);
        let (_, lines, _) = ui.layout(&s, size.0, size.1);
        assert_eq!(ui.current(&lines), Some(Page::Hdr));
    }

    #[test]
    fn growing_the_window_keeps_the_scroll_within_the_list() {
        let s = Settings::default();
        let mut ui = GraphicsUi::default();
        ui.scroll(&s, SIZE, -1000.0);
        assert!(ui.scroll.amount > 0.0);
        let (cols, lines, _) = ui.layout(&s, 640.0, 4000.0);
        assert_eq!(ui.scroll.amount, 0.0);
        assert_eq!(ui.row_y(&cols, &lines[0]), cols.top);
    }

    #[test]
    fn the_search_filters_as_you_type_and_drops_pages_left_empty() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, all, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert!(all.iter().all(|l| l.page == Page::General));
        let [x, y, ..] = cols.search();
        press(&mut ui, &mut s, (x + 4.0, y + 4.0));
        assert!(ui.typing());
        for c in ["c", "l", "o", "u", "d"] {
            ui.key_modifiers(&mut s, K::KeyA, Some(c), false, SIZE, FB);
        }
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        let found: Vec<Item> = lines
            .iter()
            .filter_map(|l| l.setting.map(|r| r.item))
            .collect();
        assert_eq!(
            found,
            [Item::Vanilla(Opt::Clouds), Item::Vanilla(Opt::CloudRange)]
        );
        assert!(lines.iter().all(|l| l.page == Page::Quality));
        // Words of a description find it too.
        ui.search.set_value("tearing");
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert!(lines.iter().any(|l| {
            l.setting
                .is_some_and(|r| r.item == Item::Vanilla(Opt::Vsync))
        }));
        ui.search.set_value("hdr");
        let [x, y, w, h] = cols.page(4);
        press(&mut ui, &mut s, (x + w / 2.0, y + h / 2.0));
        assert_eq!(ui.current(&ui.lines(&s).0), Some(Page::Hdr));
        let [x, y, ..] = cols.clear();
        assert_eq!(
            press(&mut ui, &mut s, (x + 2.0, y + 2.0)),
            [Action::ButtonClick]
        );
        // The clear button leaves the page picked meanwhile open, alone, from the top.
        let (lines, _) = ui.lines(&s);
        assert!(lines.len() < all.len() && lines.iter().all(|l| l.page == Page::Hdr));
        assert_eq!(ui.scroll.amount, 0.0);
        assert_eq!(ui.current(&lines), Some(Page::Hdr));
    }

    #[test]
    fn undo_appears_after_changes_and_puts_back_what_the_screen_opened_with() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, ..) = ui.layout(&s, SIZE.0, SIZE.1);
        let undo = cols.button(1);
        let at = |[x, y, ..]: [f32; 4]| (x + 2.0, y + 2.0);
        // Nothing changed: Undo does nothing.
        assert!(press(&mut ui, &mut s, at(undo)).is_empty());
        let fog = row(&mut ui, &s, Item::Horizon(H::Fog));
        assert_eq!(
            press(&mut ui, &mut s, at(fog)),
            [Action::ButtonClick, Action::Changed]
        );
        let sky = row(&mut ui, &s, Item::Horizon(H::Sky));
        press(&mut ui, &mut s, at(sky));
        assert!(!s.horizon.fog && !s.horizon.sky && ui.changed);
        // F11 and another option meanwhile: Undo leaves them as they are.
        s.fullscreen = true;
        s.chat_opacity = 0.25;
        assert_eq!(
            press(&mut ui, &mut s, at(undo)),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(s.horizon.fog && s.horizon.sky && !ui.changed);
        assert!(s.fullscreen);
        assert_eq!(s.chat_opacity, 0.25);
        // From the keyboard: Undo puts it back and goes away.
        let fog = row(&mut ui, &s, Item::Horizon(H::Fog));
        press(&mut ui, &mut s, at(fog));
        ui.focus = Some(Target::Undo);
        let (actions, _) = ui.key_modifiers(&mut s, K::Enter, None, false, SIZE, FB);
        assert_eq!(actions, [Action::ButtonClick, Action::Changed]);
        assert!(s.horizon.fog && !ui.changed);
        assert_eq!(ui.focus, Some(Target::Done));
    }

    #[test]
    fn a_slider_takes_a_press_on_its_track_and_its_click_plays_on_release() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.horizon.chunk_updates = 16;
        let r = row(&mut ui, &s, Item::Horizon(H::ChunkUpdates));
        let (tx, tw) = Columns::of(SIZE.0, SIZE.1).track();
        // On its name: focused, unchanged.
        assert!(press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)).is_empty());
        assert_eq!(s.horizon.chunk_updates, 16);
        assert!(ui.can_change_value);
        assert!(!ui.release());
        // At the track's end: the most, then dragged to its start.
        assert_eq!(
            press(&mut ui, &mut s, (tx + tw, r[1] + 4.0)),
            [Action::SliderDrag]
        );
        assert_eq!(H::ChunkUpdates.slider(&s), Some(1.0));
        assert_eq!(
            ui.click(&mut s, (-100.0, -100.0), SIZE, FB, true, false),
            [Action::SliderDrag]
        );
        assert_eq!(H::ChunkUpdates.slider(&s), Some(0.0));
        assert!(ui.release());
        assert!(!ui.release());
    }

    #[test]
    fn hdr_waits_for_a_display_that_offers_it_and_its_sliders_for_the_switch() {
        let _offer = HDR_OFFER.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let at = |[x, y, ..]: [f32; 4]| (x + 8.0, y + 4.0);
        crate::graphics::offer_hdr(false);
        // A 720p window opened at HDR shows only its greyed rows: Down still reaches the shortcut.
        let short = (427.0, 240.0);
        let mut at_hdr = GraphicsUi::default();
        at_hdr.open(Page::Hdr);
        at_hdr.key_modifiers(&mut s, K::ArrowDown, None, false, short, FB);
        assert_eq!(at_hdr.focus, Some(Target::Item(Item::Horizon(H::HdrKey))));
        let hdr = row(&mut ui, &s, Item::Horizon(H::Hdr));
        assert!(press(&mut ui, &mut s, at(hdr)).is_empty());
        // Not even with HDR saved on, from another computer: nor its sliders and calibration.
        s.horizon.hdr = true;
        for h in [H::Hdr, H::HdrCalibrate, H::HdrPeak, H::HdrPaperWhite] {
            assert!(Item::Horizon(h).blocked(&s).is_some(), "{h:?}");
        }
        s.horizon.hdr = false;
        crate::graphics::offer_hdr(true);
        let (tx, _) = Columns::of(SIZE.0, SIZE.1).track();
        let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
        assert!(press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)).is_empty());
        // The switch turns it on, and the sliders below it work.
        let hdr = row(&mut ui, &s, Item::Horizon(H::Hdr));
        assert_eq!(
            press(&mut ui, &mut s, at(hdr)),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(s.horizon.hdr);
        let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
        assert_eq!(
            press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)),
            [Action::SliderDrag]
        );
        ui.release();
        // Calibrate starts the calibration; the shortcut row waits for a key.
        let calibrate = row(&mut ui, &s, Item::Horizon(H::HdrCalibrate));
        assert_eq!(
            press(&mut ui, &mut s, at(calibrate)),
            [Action::ButtonClick, Action::Calibrate]
        );
        let key = row(&mut ui, &s, Item::Horizon(H::HdrKey));
        assert_eq!(
            press(&mut ui, &mut s, at(key)),
            [
                Action::ButtonClick,
                Action::BindKey(crate::graphics::HDR_KEY)
            ]
        );
        assert!(ui.changed);
    }

    #[test]
    fn the_scrollbar_can_be_held_and_dragged() {
        // #3552: AbstractScrollArea.updateScrolling holds the scrollbar on a press over its
        // column, mouseDragged moves the list with the pointer, onRelease lets go.
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, _, content) = ui.layout(&s, SIZE.0, SIZE.1);
        let x = cols.bar_x() + 2.0;
        let drag = |ui: &mut GraphicsUi, s: &mut Settings, y, drag| {
            ui.click(s, (x, y), SIZE, FB, drag, false)
        };
        assert!(drag(&mut ui, &mut s, cols.top + 5.0, false).is_empty());
        drag(&mut ui, &mut s, cols.top + 15.0, true);
        assert!(ui.scroll.amount > 0.0);
        drag(&mut ui, &mut s, cols.bottom + 50.0, true);
        assert_eq!(ui.scroll.amount, content - (cols.bottom - cols.top));
        drag(&mut ui, &mut s, cols.top - 50.0, true);
        assert_eq!(ui.scroll.amount, 0.0);
        assert!(!ui.release());
        drag(&mut ui, &mut s, cols.bottom + 50.0, true);
        assert_eq!(ui.scroll.amount, 0.0);
    }

    #[test]
    fn a_held_scroller_lets_go_when_the_window_gets_too_short_for_it() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, ..) = ui.layout(&s, SIZE.0, SIZE.1);
        let x = cols.bar_x() + 2.0;
        ui.click(&mut s, (x, cols.top + 5.0), SIZE, FB, false, false);
        assert!(ui.scroll.held());
        // A 130-high GUI leaves the list under the 40 px the scroller's thumb needs.
        let short = (SIZE.0, 130.0);
        assert!(!Columns::of(short.0, short.1).scrolls());
        assert!(
            ui.click(&mut s, (x, 80.0), short, FB, true, false)
                .is_empty()
        );
        assert!(!ui.scroll.held());
    }

    #[test]
    fn the_wheel_scrolls_and_stops_at_both_ends_without_changing_values() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.fov = 90.0;
        let fov = row(&mut ui, &s, Item::Vanilla(Opt::Fov));
        // A slider being changed doesn't take the wheel either: a controller scrolls with it.
        press(&mut ui, &mut s, (fov[0] + 4.0, fov[1] + 4.0));
        assert!(ui.can_change_value);
        let before = ui.scroll.amount;
        ui.scroll(&s, SIZE, -1.0);
        assert_eq!(ui.scroll.amount, before + 18.0);
        assert_eq!(s.fov, 90.0);
        let (cols, _, content) = ui.layout(&s, SIZE.0, SIZE.1);
        ui.scroll(&s, SIZE, -1000.0);
        assert_eq!(ui.scroll.amount, content - (cols.bottom - cols.top));
        ui.scroll(&s, SIZE, 1000.0);
        assert_eq!(ui.scroll.amount, 0.0);
    }

    #[test]
    fn the_arrows_move_between_rows_in_view_and_over_to_the_page_list() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let in_view = |ui: &mut GraphicsUi, s: &Settings, item| {
            let (cols, lines, _) = ui.layout(s, SIZE.0, SIZE.1);
            let line = lines
                .iter()
                .find(|l| l.setting.is_some_and(|r| r.item == item))
                .unwrap();
            let y = ui.row_y(&cols, line);
            y >= cols.top && y + style::ROW <= cols.bottom
        };
        // General's last row, against the bottom of the view, then Up: the row over it.
        let last = Item::Vanilla(Opt::Autosave);
        row(&mut ui, &s, last);
        ui.focus = Some(Target::Item(last));
        ui.key_modifiers(&mut s, K::ArrowUp, None, false, SIZE, FB);
        let indicator = Item::Vanilla(Opt::AttackIndicator);
        assert_eq!(ui.focus, Some(Target::Item(indicator)));
        assert!(in_view(&mut ui, &s, indicator));
        // Left leaves the plate.
        ui.key_modifiers(&mut s, K::ArrowLeft, None, false, SIZE, FB);
        assert!(!matches!(ui.focus, Some(Target::Item(_))), "{:?}", ui.focus);
        // Up from Done: the page list's last page.
        ui.focus = Some(Target::Done);
        ui.key_modifiers(&mut s, K::ArrowUp, None, false, SIZE, FB);
        assert_eq!(ui.focus, Some(Target::Page(Page::Hdr)));
        // Its row wheeled far out of view: Down takes the first row in view, as it is.
        ui.focus = Some(Target::Item(Item::Horizon(H::Preset)));
        ui.scroll(&s, SIZE, -1000.0);
        let before = ui.scroll.amount;
        assert!(before > 0.0);
        ui.key_modifiers(&mut s, K::ArrowDown, None, false, SIZE, FB);
        let Some(Target::Item(item)) = ui.focus else {
            panic!("{:?}", ui.focus)
        };
        assert!(in_view(&mut ui, &s, item), "{item:?}");
        assert_eq!(ui.scroll.amount, before);
    }

    #[test]
    fn the_keyboard_tabs_through_settings_and_steps_sliders() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.horizon.chunk_updates = 10;
        let tab_to = |ui: &mut GraphicsUi, s: &mut Settings, target| {
            (0..40).any(|_| {
                ui.key_modifiers(s, K::Tab, None, false, SIZE, FB);
                ui.focus == Some(target)
            })
        };
        // Tab reaches the page list; Enter opens Performance and Tab goes on into it.
        assert!(tab_to(&mut ui, &mut s, Target::Page(Page::Performance)));
        let (actions, _) = ui.key_modifiers(&mut s, K::Enter, None, false, SIZE, FB);
        assert_eq!(actions, [Action::ButtonClick]);
        let chunk_updates = Target::Item(Item::Horizon(H::ChunkUpdates));
        assert!(tab_to(&mut ui, &mut s, chunk_updates) && ui.can_change_value);
        // Tab scrolled its row into view.
        let r = {
            let (cols, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
            let line = lines
                .iter()
                .find(|l| {
                    l.setting
                        .is_some_and(|r| r.item == Item::Horizon(H::ChunkUpdates))
                })
                .copied()
                .unwrap();
            ui.row_y(&cols, &line)
        };
        let cols = Columns::of(SIZE.0, SIZE.1);
        assert!(r >= cols.top && r + style::ROW <= cols.bottom);
        let (actions, handled) = ui.key_modifiers(&mut s, K::ArrowRight, None, false, SIZE, FB);
        assert!(handled && actions == [Action::SliderDrag]);
        assert_eq!(s.horizon.chunk_updates, 11);
        ui.key_modifiers(&mut s, K::ArrowLeft, None, false, SIZE, FB);
        assert_eq!(s.horizon.chunk_updates, 10);
        // Space lets go of the value: the arrows move the focus again.
        ui.key_modifiers(&mut s, K::Space, None, false, SIZE, FB);
        assert!(!ui.can_change_value);
        ui.key_modifiers(&mut s, K::ArrowDown, None, false, SIZE, FB);
        assert_eq!(ui.focus, Some(Target::Item(Item::Horizon(H::UploadBudget))));
        // Enter presses a switch; Escape is left to the screen, which closes.
        ui.focus = Some(Target::Item(Item::Horizon(H::Fog)));
        let (actions, _) = ui.key_modifiers(&mut s, K::Enter, None, false, SIZE, FB);
        assert_eq!(actions, [Action::ButtonClick, Action::Changed]);
        assert!(!s.horizon.fog);
        assert_eq!(
            ui.key_modifiers(&mut s, K::Escape, None, false, SIZE, FB),
            (Vec::new(), false)
        );
    }

    #[test]
    fn rows_split_vanilla_labels_into_name_and_value() {
        let s = Settings::default();
        let (name, value) = parts(Item::Horizon(H::LodDistance), &s, false);
        assert_eq!(
            (name.as_str(), value.as_deref()),
            ("LOD Distance", Some("Off"))
        );
        let (name, value) = parts(Item::Horizon(H::HdrCalibrate), &s, false);
        assert!(!name.is_empty() && value.is_none());
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
    fn lod_distance_slides_after_render_distance_without_changing_it() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LodDistance);
        let near = s.render_distance;
        assert_eq!(item.label(&s).plain(), "LOD Distance: Off");
        assert_eq!(item.slider(&s), Some(0.0));
        let items: Vec<_> = items().into_iter().map(|(_, i)| i).collect();
        let render = items
            .iter()
            .position(|i| *i == Item::Vanilla(Opt::RenderDistance))
            .unwrap();
        assert_eq!(items[render + 1], item);
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
    fn blue_map_settings_sit_with_far_terrain() {
        let mut s = Settings::default();
        let find = Item::Horizon(H::BlueMapAutoDetect);
        let radius = Item::Horizon(H::BlueMapRadius);
        let distance: Vec<Item> = rows()
            .into_iter()
            .flat_map(|(_, groups)| groups)
            .find(|(name, _)| *name == "Distance")
            .unwrap()
            .1
            .iter()
            .map(|r| r.item)
            .collect();
        assert!(distance.contains(&find) && distance.contains(&radius));
        assert!(distance.contains(&Item::Horizon(H::FarDetail)));
        assert_eq!(find.switch(&s), Some(false), "off by default");
        assert_eq!(radius.label(&s).plain(), "BlueMap Radius: 64 chunks");
        find.cycle(&mut s, false, FB);
        assert!(s.horizon.blue_map.auto_detect);
        radius.cycle(&mut s, false, FB);
        assert_eq!(s.horizon.blue_map.radius, 128);
        radius.cycle(&mut s, true, FB);
        radius.cycle(&mut s, true, FB);
        assert_eq!(radius.label(&s).plain(), "BlueMap Radius: 32 chunks");
        assert_eq!(s.horizon.preset, Preset::Custom);
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
        assert!(!shown(upscaling, &s));
        s.horizon.taa = true;
        assert!(shown(upscaling, &s));
    }

    #[test]
    fn entity_culling_modes_are_opt_in_and_cycle_from_its_row() {
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
        s.horizon.entity_culling = EntityCullingMode::Off;
        let mut ui = GraphicsUi::default();
        let r = row(&mut ui, &s, item);
        for mode in [
            EntityCullingMode::Synchronous,
            EntityCullingMode::Asynchronous,
        ] {
            s.horizon.preset = Preset::Balanced;
            assert_eq!(
                press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)),
                [Action::ButtonClick, Action::Changed]
            );
            assert_eq!(s.horizon.entity_culling, mode);
            assert_eq!(s.horizon.preset, Preset::Custom);
        }
        assert_eq!(item.label(&s).plain(), "Entity Culling: Asynchronous");
    }

    /// Translucent Batching: a switch on the Advanced page, on by default, no part of a
    /// preset, and greyed out, reading Off, where the device can't batch.
    #[test]
    fn translucent_batching_is_an_advanced_switch_where_the_device_can() {
        let item = Item::Horizon(H::TranslucentBatching);
        let page = items()
            .into_iter()
            .find(|(_, i)| *i == item)
            .map(|(p, _)| p);
        if cfg!(target_os = "macos") {
            // Metal draws flat: never offered, so not shown.
            assert_eq!(page, None);
            return;
        }
        assert_eq!(page, Some(Page::Advanced));
        let mut s = Settings::default();
        s.horizon.preset = Preset::Balanced;
        assert_eq!(item.switch(&s), Some(true));
        let mut ui = GraphicsUi::default();
        // Where the device can't, the saved On reads Off, and neither a click nor Enter
        // after it does anything: no sound, no change, no save.
        crate::graphics::offer_translucent_batching(false);
        assert!(item.blocked(&s).is_some());
        assert_eq!(item.label(&s).plain(), "Translucent Batching: Off");
        let r = row(&mut ui, &s, item);
        let point = (r[0] + 4.0, r[1] + 4.0);
        assert!(press(&mut ui, &mut s, point).is_empty());
        ui.focus = Some(Target::Item(item));
        let enter = ui.key_modifiers(&mut s, K::Enter, None, false, SIZE, FB);
        assert!(enter.0.is_empty());
        assert!(s.horizon.translucent_batching);
        // Where it can, a click switches it and keeps the preset.
        crate::graphics::offer_translucent_batching(true);
        assert_eq!(item.label(&s).plain(), "Translucent Batching: On");
        assert_eq!(
            press(&mut ui, &mut s, point),
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
    fn leaf_culling_cycles_modes_beside_the_other_culling() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LeafCulling);
        assert_eq!(item.switch(&s), None);
        assert_eq!(item.label(&s).plain(), "Leaf Culling: Off");
        for label in ["Solid", "Solid Aggressive", "Hollow", "Off"] {
            item.cycle(&mut s, false, FB);
            assert_eq!(item.label(&s).plain(), format!("Leaf Culling: {label}"));
            assert_eq!(
                crate::graphics::Graphics::parse(&s.horizon.serialize()).leaf_culling,
                s.horizon.leaf_culling
            );
        }
        item.cycle(&mut s, true, FB);
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Hollow);

        // The three culling settings share one group of the Performance page.
        let culling = rows()
            .into_iter()
            .flat_map(|(page, groups)| groups.into_iter().map(move |(_, b)| (page, b)))
            .find(|(_, b)| b.iter().any(|r| r.item == item))
            .unwrap();
        assert_eq!(culling.0, Page::Performance);
        let block: Vec<Item> = culling.1.iter().map(|r| r.item).collect();
        for h in [H::CaveCulling, H::EntityCulling, H::LeafCulling] {
            assert!(block.contains(&Item::Horizon(h)), "{h:?}");
        }
        // The block-entity caches are the group after it, in this order.
        let groups: Vec<(&str, Vec<Item>)> = rows()
            .into_iter()
            .find(|(page, _)| *page == Page::Performance)
            .unwrap()
            .1
            .into_iter()
            .map(|(name, rows)| (name, rows.iter().map(|r| r.item).collect()))
            .collect();
        let at = groups
            .iter()
            .position(|(name, _)| *name == "Culling")
            .unwrap();
        assert_eq!(
            groups[at + 1],
            (
                "Block Entities",
                [
                    H::CacheShulkerBoxes,
                    H::CacheSigns,
                    H::CacheDecoratedPots,
                    H::CacheBells,
                    H::CacheChests,
                    H::CacheBanners,
                    H::CacheCopperGolemStatues,
                ]
                .map(Item::Horizon)
                .to_vec()
            )
        );

        // The description names what the picked mode does.
        for mode in LeafCullingMode::ALL {
            s.horizon.leaf_culling = mode;
            let text = description(item, &s, "fallback");
            assert_ne!(text, "fallback", "{mode:?}");
            assert!(text.len() < 80, "{mode:?}: {text}");
        }

        // A click cycles it and leaves the preset, as the other quality settings do.
        let mut ui = GraphicsUi::default();
        s.horizon.preset = Preset::Balanced;
        s.horizon.leaf_culling = LeafCullingMode::Off;
        let r = row(&mut ui, &s, item);
        assert_eq!(
            press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)),
            [Action::ButtonClick, Action::Changed]
        );
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Solid);
        assert_eq!(s.horizon.preset, Preset::Custom);
    }

    #[test]
    fn graphics_step_slider_matches_set_slider_for_all_options() {
        // HDR's sliders step only where the display offers HDR.
        let _offer = HDR_OFFER.lock().unwrap_or_else(|e| e.into_inner());
        crate::graphics::offer_hdr(true);
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
    /// Horizon's graphics captions are English sentences that the language table keys by
    /// themselves, so a caption, description or value that is not in `en_us.json` shows in
    /// English in every language. Every text the screen shows, with each setting cycled
    /// through its values (#1927).
    #[test]
    fn every_graphics_text_is_in_the_language_table() {
        let (_, missed) = lang::scoped(lang::Language::load(&[], "de_de"), || {
            for hdr in [false, true] {
                let mut s = Settings::default();
                s.horizon.hdr = hdr;
                s.horizon.taa = true;
                for _ in 0..8 {
                    for (page, groups) in rows() {
                        lang::get(page.name());
                        lang::get(page.about());
                        for (group, rows) in groups {
                            lang::get(group);
                            for row in rows {
                                // Vanilla's captions are vanilla's keys, not loaded here.
                                if let Item::Horizon(_) = row.item {
                                    row.item.label(&s);
                                }
                                let text = description(row.item, &s, row.text);
                                lang::get(text);
                                lang::get(row.cost.label());
                                if let Some(why) = row.item.blocked(&s) {
                                    lang::get(why);
                                }
                                if let Item::Horizon(h) = row.item {
                                    h.cycle(&mut s, false);
                                }
                            }
                        }
                    }
                }
            }
            for text in [
                "Search options",
                "Results for \"%s\"",
                "No settings match.",
                "Undo",
                "Done",
            ] {
                lang::get(text);
            }
        });
        // `labelled` also looks up values that are already words in the language ("An") or
        // numbers with a unit ("12 chunks", "Auto (8)"); those are not keys, and "FSR 3.1" is
        // a name.
        let missed: Vec<_> = missed
            .into_iter()
            .filter(|m| !m.starts_with(|c: char| c.is_ascii_digit()) && !m.starts_with("Auto ("))
            .filter(|m| !["An", "Aus", "FSR 3.1", "key.keyboard.unknown"].contains(&m.as_str()))
            .collect();
        assert_eq!(missed, Vec::<String>::new());
    }
    /// Horizon's captions join label and value with the language's `options.generic_value` and
    /// `options.percent_value`, as vanilla's beside them on the same page do (#6072). The
    /// templates are 26.3's.
    #[test]
    fn horizon_captions_use_the_languages_separator() {
        struct Vanilla;
        impl lang::ResourceSource for Vanilla {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                let (generic, percent) = match path {
                    "assets/minecraft/lang/en_us.json" => ("%s: %s", "%s: %s%%"),
                    "assets/minecraft/lang/fr_fr.json" => ("%s : %s", "%s : %s %%"),
                    "assets/minecraft/lang/zh_cn.json" => ("%s：%s", "%s：%s%%"),
                    "assets/minecraft/lang/en_ud.json" => ("%2$s :%1$s", "%s: %s%%"),
                    _ => return None,
                };
                serde_json::to_vec(&serde_json::json!({
                    "options.generic_value": generic,
                    "options.percent_value": percent,
                }))
                .ok()
            }
        }
        let mut s = Settings::default();
        s.horizon.leaf_culling = LeafCullingMode::Hollow;
        s.horizon.resolution_scale = 75;
        for (code, leaf, scale) in [
            ("en_us", "Leaf Culling: Hollow", "Resolution Scale: 75%"),
            (
                "fr_fr",
                "Occlusion feuillage : Creux",
                "Échelle de résolution : 75 %",
            ),
            ("zh_cn", "树叶剔除：镂空", "渲染比例：75%"),
            // Value first.
            ("en_ud", "Hollow :Leaf Culling", "Resolution Scale: 75%"),
        ] {
            let (shown, _) = lang::scoped(lang::Language::load(&[&Vanilla], code), || {
                (
                    Item::Horizon(H::LeafCulling).label(&s).plain(),
                    Item::Horizon(H::ResolutionScale).label(&s).plain(),
                )
            });
            assert_eq!(shown, (leaf.into(), scale.into()), "{code}");
        }
    }
    /// Leaf Culling, Frame Generation and Shadow Cache came with English captions only (#1927).
    #[test]
    fn newer_graphics_settings_are_translated() {
        let mut s = Settings::default();
        s.horizon.leaf_culling = LeafCullingMode::Hollow;
        s.horizon.entity_culling = EntityCullingMode::Asynchronous;
        for (code, leaf, entity, leaf_text) in [
            (
                "de_de",
                "Laub-Culling: Hohl",
                "Objekt-Culling: Asynchron",
                "Lässt Flächen zwischen Blättern weg: schneller, aber Lücken sind durchsichtig.",
            ),
            (
                "pl_pl",
                "Pomijanie liści: Puste",
                "Pomijanie bytów: Asynchroniczne",
                "Pomija ściany między liśćmi: szybciej, ale luki są przezroczyste.",
            ),
            (
                "uk_ua",
                "Відсікання листя: Порожнє",
                "Відсік. сутностей: Асинхронно",
                "Прибирає грані між листям: швидше, але прогалини прозорі.",
            ),
            (
                "zh_cn",
                "树叶剔除: 镂空",
                "实体剔除: 异步",
                "省略树叶之间的面：更快，但缝隙会透视。",
            ),
            (
                "fr_fr",
                "Occlusion feuillage: Creux",
                "Occlusion des entités: Asynchrone",
                "Supprime les faces entre feuilles : plus rapide, mais on voit à travers les trous.",
            ),
        ] {
            let (shown, _) = lang::scoped(lang::Language::load(&[], code), || {
                let item = |h| Item::Horizon(h);
                (
                    item(H::LeafCulling).label(&s).plain(),
                    item(H::EntityCulling).label(&s).plain(),
                    lang::get(description(item(H::LeafCulling), &s, "")),
                )
            });
            assert_eq!(
                shown,
                (leaf.into(), entity.into(), leaf_text.into()),
                "{code}"
            );
        }
    }
}
