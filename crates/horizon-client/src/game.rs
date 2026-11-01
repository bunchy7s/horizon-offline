//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.

/// The content tables the world plays with, as field-level borrows: the
/// world's data packs' over the game's own. The `Game::items`/`recipes`/
/// `stations` methods read the same tables, but taking `&self` blocks a
/// `&mut self.…` in the same scope; these expand to a match on the fields,
/// which borrows stay disjoint from.
macro_rules! world_items {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.items,
            None => $s.res.items(),
        }
    };
}
macro_rules! world_recipes {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.recipes,
            None => $s.res.recipes(),
        }
    };
}
macro_rules! world_stations {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.stations,
            None => $s.res.stations(),
        }
    };
}
/// The biome table the world plays with (`Game::biomes`), as a field-level
/// borrow: the world's packs' over the game's own.
macro_rules! world_biomes {
    ($s:expr) => {
        match &$s.world_biomes {
            Some(b) => b,
            None => $s.res.biomes(),
        }
    };
}

/// The world's packs' content files, as a field-level borrow (see the macros).
fn pack_files(world_tables: &Option<WorldTables>) -> horizon_world::loot::PackOverlay<'_> {
    match world_tables {
        Some(t) => horizon_world::loot::PackOverlay { files: &t.files },
        None => horizon_world::loot::PackOverlay::NONE,
    }
}

mod advancements;
mod advancements_screen;
mod agent;
mod agent_settings;
mod allays;
mod armor_stands;
mod atlas_slots;
mod banners;
mod beacon;
mod beds;
mod beehives;
mod block_editor;
mod block_entity_anims;
mod block_entity_ticks;
mod block_items;
mod blockctl;
mod boat_leads;
mod boat_passengers;
mod boats;
mod books;
mod bossbars;
mod breezes;
mod brush;
mod bundles;
mod campfire;
mod campfire_douse;
mod cartography;
mod cat_spawner;
mod chat;
mod chat_filter;
mod chiseled_bookshelf;
mod client_ambient;
mod clipboard;
mod code_of_conduct;
mod combat;
mod command_blocks;
mod command_harness;
mod command_minecarts;
mod commands;
mod composter;
mod consume;
mod control;
mod copper_golems;
mod creaking;
mod creative_hotbar;
mod creatures;
mod crossbow;
pub(crate) mod cushions;
mod debug_keys;
mod debug_lines;
mod debug_options;
mod debug_renderers;
mod debug_stick;
mod decorated_pot;
mod dedicated;
mod deflect;
mod dialogs;
mod discord;
mod dispense_fixture;
mod dispenser;
mod dragon;
mod dynamic_lights;
mod enchant_effects;
mod end;
mod entity_culling;
mod event_loop;
mod events;
mod execution_role;
mod explosion;
mod extras;
mod falling_damage;
mod fangs;
mod fireworks;
mod fishing;
mod fov;
mod frames;
mod friends;
mod functions;
mod game_mode;
mod game_tests;
mod gamemode_switcher;
mod ghasts;
mod glowing;
mod golems;
mod growth;
mod hand;
mod headless;
mod ime;
mod initial_spawn;
mod integrated;
mod interactions;
mod item_draw;
mod item_particles;
mod item_use;
mod jigsaw_blocks;
mod jigsaw_edit;
mod jukebox;
mod key_focus;
mod lan;
mod lan_actions;
mod lan_attributes;
mod lan_camera;
mod lan_chat;
mod lan_crafting;
mod lan_entities;
mod lan_mobs;
mod lan_mount_menu;
mod lan_particles;
mod lan_pvp;
mod lan_quiet;
mod lan_screen;
mod lan_stations;
mod lan_trading;
mod lan_vehicles;
mod lan_waypoints;
mod launch_api;
mod leads;
mod level_particles;
mod levels;
mod lightning;
mod llamas;
mod locator_bar;
mod loom;
mod mace;
mod machines;
mod maps;
mod minecarts;
mod mob_blocks;
mod mob_gear;
mod mob_looting;
mod mob_sounds;
mod mods;
mod mount_menu;
mod mounts;
mod moving_pistons;
mod music;
mod names;
mod narrator;
mod native_block_entity_draw;
mod native_components;
mod native_criteria;
mod native_entities;
mod native_explosions;
mod native_falling;
mod native_hooks;
mod native_lifecycle;
mod native_menus;
mod native_mobs;
mod native_motion;
mod native_net;
mod native_panel;
mod native_panel_recipes;
mod native_recipes;
mod native_server;
mod native_shaders;
mod native_visuals;
mod native_world_read;
mod natural_spawning;
mod nautili;
mod net_harness;
mod nether_portal;
mod no_resources;
mod ominous_item_spawner;
mod options;
mod packs;
mod pad;
mod pad_menu;
mod paintings;
mod pandas;
mod passengers;
mod persist;
mod perspective;
mod pick_entity;
mod pickups;
mod piglins;
mod piston_push;
mod player_equip;
mod postprocess;
mod potent_sulfur;
mod potions;
mod powder_snow;
mod prediction;
mod pumpkin;
mod quick_actions;
mod raids;
mod recipe_book;
mod remote;
mod remote_camera;
mod remote_chat;
mod remote_chunks;
mod remote_mob_data;
mod remote_mob_particles;
mod remote_mob_use;
mod remote_objects;
mod remote_projectiles;
mod remote_stations;
mod remote_vehicles;
mod render_control;
mod render_tick;
mod respawn_anchor;
mod saves_control;
mod saving;
mod scoreboard;
mod screen_info;
mod screens;
mod sculk;
mod sculk_fixture;
mod server_links;
mod server_packs;
mod server_travel;
mod shader_packs;
mod shelf;
mod shield;
mod shoulders;
mod sign_edit;
mod sniffers;
mod spawner;
mod spear;
mod spectator;
mod spill;
mod splash_water;
mod split;
mod startup_menu;
mod station_control;
mod stations;
mod stats;
mod stats_screen;
mod storage;
mod structure_blocks;
mod structure_edit;
mod structures;
mod subtitle_hud;
mod sulfur_cubes;
mod test_block_edit;
mod test_blocks;
mod test_entities;
mod test_environments;
mod test_instance_edit;
mod test_instances;
mod text_displays;
mod titles;
mod tnt;
mod totem;
mod trading;
mod transient_blocks;
mod trial_spawner;
mod trial_visuals;
mod trident;
mod tutorial;
mod tweaks;
mod ui_control;
mod vault;
mod villagers;
mod wandering_trader;
mod win;
mod wind;
mod wither;
mod world_loading;
mod zombie_reinforcements;
mod zombie_villagers;
use event_loop::EventLoopCtl;
use horizon_client::{
    Result,
    assets::{Assets, RenderAssets},
    audio::Audio,
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    clock::FixedStep,
    collisions,
    containers::{BlockEntity, Providers},
    crafting::Recipes,
    creative::{Creative, Names},
    entities::{Experience, ExperienceOrb, ItemEntity},
    environment::Environment,
    first_person::{Hand, SwingAnimation},
    furnace::FurnaceCooking,
    gfx::{Frame, Renderer},
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, hud_overlays,
    items::{ItemStack, Items},
    keymapping::{KeyAction, SNEAK, SPRINT},
    light::LightTables,
    mesher::{MeshTables, Vertex},
    mobs::{Mob, MobType, MobTypes},
    models,
    motion::{MotionEvent, MotionResources, MotionWorld, PlayerMotion},
    mouse::{LookOptions, MouseLook},
    particles::Particle,
    save::{Level, Persistent, SaveDir},
    settings::Settings,
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    testgen::TestTerrain,
    title::{Action, Title},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    geometry::{Aabb, Direction},
    position::BlockPos,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::{EntityIdentity, EntityObstructionWorld, ObstructionEntity},
    fluid::FluidState,
    level::{
        Behaviours, BucketContent, Dimension as LevelDimension, EntityQuery, Landing, LevelAccess,
        LevelEvent, PlaceContext, PlaceOutcome, PlacementContext, PlayerContext, PlayerEffect,
        Ticks, flags as level_flags,
    },
    registry::BlockStateId,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

/// The time a frame spends uploading chunk meshes when the player set no
/// Chunk Updates or Upload Budget: a burst past it waits for the next frames.
const AUTO_UPLOAD_TIME: Duration = Duration::from_millis(2);
/// A frame's finished meshes up to this many upload at once, unqueued.
const AUTO_UPLOADS_AT_ONCE: usize = 8;

#[derive(Clone)]
struct Options {
    resources: PathBuf,
    distance: i32,
    distance_override: Option<i32>,
    seed: i64,
    creative: bool,
    test_terrain: bool,
    frames: Option<u64>,
    /// A splitscreen player: the split window paces every player's frames, not each its own.
    paced_by_split: bool,
    /// A splitscreen player other than P1: its windows are the split's (P1 alone decides
    /// fullscreen; this game neither asks for it nor adopts it into its options).
    split_guest: bool,
    /// A splitscreen guest drawn in Window 1, the game's own window: its F11 leaves that
    /// window to P1.
    split_in_main_window: bool,
    /// `--world-frames N`: exit after N frames drawn in the world (smoke runs; the world may load slowly).
    world_frames: Option<u64>,
    screenshot_after: Option<u64>,
    timeout: Duration,
    report: Option<PathBuf>,
    world: Option<String>,
    look: Option<(f32, f32)>,
    time: Option<i64>,
    weather: Option<String>,
    dimension: Option<String>,
    /// Options.pauseOnLostFocus off (F3+P), for scripted testing.
    no_pause: bool,
    /// Control socket path (`--control`), for `horizon-ctl`.
    control: Option<PathBuf>,
    /// `HORIZON_SPIKES=MS`: print the timed spans of frames slower than MS.
    spikes: Option<f32>,
    /// Render into a texture instead of the window (`--offscreen`), so a
    /// GPU the display cannot present from can still be benchmarked. The
    /// window stays hidden and never grabs the system pointer.
    offscreen: bool,
    /// `--headless` (#4951): no display, no window and no renderer. The game runs its own
    /// loop (`game/headless.rs`) instead of winit's, so it starts where no display server
    /// is (an SSH session, a server). It counts as `offscreen` for everything else (an
    /// automated run that never shows a window or grabs the pointer).
    headless: bool,
    /// `--width` and `--height` (vanilla's own arguments): the window's
    /// starting size, 1280x720 by default.
    window_size: [u32; 2],
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    server: Option<String>,
    /// `--name`, `HORIZON_PLAYER_NAME` or the signed-in account's name; else
    /// [`Options::player_name`] uses the kept offline name.
    player_name: Option<String>,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    dedicated: Option<Vec<String>>,
    /// `--benchmark SCENARIO`: run the benchmark, write its report and quit.
    benchmark: Option<String>,
    /// `--dev-mod PATH` (repeatable): content mods always on.
    dev_mods: Vec<PathBuf>,
    /// Single player runs on an integrated server thread (game/integrated.rs),
    /// as vanilla's does: the default. `--in-frame` (or `--no-server-thread`,
    /// or `HORIZON_SERVER_THREAD=0`) runs the world in the frame instead.
    /// DEPRECATED: never use it; it is unsupported and will be removed soon.
    server_thread: bool,
    /// What this Game executes; independent of its current server connection.
    execution_role: execution_role::ExecutionRole,
    /// `--fullscreen` (vanilla's): the window opens fullscreen whatever options.txt says,
    /// as vanilla's DisplayData does, without saving it there.
    fullscreen: bool,
    /// `--game-dir DIR`: the folder `read` makes the working directory (saves, options.txt...).
    game_dir: Option<PathBuf>,
    /// `--quickPlaySingleplayer`, checked once the game folder is known.
    quick_play: Option<String>,
    session_file: Option<String>,
    /// `--session-stdin`: the session file's JSON comes on standard input instead.
    session_stdin: bool,
}
/// `--in-frame`, `--no-server-thread` and `HORIZON_SERVER_THREAD=0` are deprecated:
/// the in-frame world is unmaintained, gets no fixes and will be removed soon.
fn warn_in_frame_deprecated(what: &str) {
    eprintln!(
        "Horizon: DEPRECATED: {what} runs single player's world in the window, a path that is no longer supported, \
         gets no fixes and will be removed soon. Do not use it: single player runs on its server thread by default."
    );
}
impl Options {
    /// The name to join servers with: the one given, else the offline name kept
    /// next to options.txt (created on first use).
    fn player_name(&self) -> String {
        self.player_name.clone().unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| horizon_client::offline_name::generate(0),
                |dir| horizon_client::offline_name::load_or_create(&dir),
            )
        })
    }
    /// The command line, then what it asks of the process: the game folder, the account.
    /// A bad command line is a [`launch_api::UsageError`] (exit code 2, no crash report).
    fn read() -> Result<Option<Self>> {
        let Some(mut o) = Self::parse(std::env::args().skip(1))
            .map_err(|e| launch_api::UsageError(e.to_string()))?
        else {
            return Ok(None);
        };
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        let session = if o.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| horizon_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            o.session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| horizon_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                o.player_name = Some(session.name.clone());
                horizon_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                horizon_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {}
            Err(e) => eprintln!("Horizon: not signed in: {e}"),
        }
        if let Some(dir) = o.game_dir.take() {
            // Paths given on the command line stay relative to where the game was started.
            o.resources = std::path::absolute(&o.resources)?;
            o.report = o.report.map(std::path::absolute).transpose()?;
            o.control = o.control.map(std::path::absolute).transpose()?;
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::env::set_current_dir(&dir))
                .map_err(|e| format!("game folder {}: {e}", dir.display()))?;
        }
        // Vanilla's quick play (Main, QuickPlay.joinSingleplayerWorld): only a
        // world that exists opens; else the title screen. --world also creates one.
        if let Some(world) = o.quick_play.take() {
            if std::env::current_dir()?
                .join("saves")
                .join(&world)
                .join("level.dat")
                .is_file()
            {
                o.world = Some(world);
            } else {
                eprintln!("Quick play: there is no world {world} in saves/");
            }
        }
        Ok(Some(o))
    }
    /// The command line alone: no file is read and nothing changes. `None` for `--help` and
    /// `--launch-api`, which print and end the run.
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                std::path::Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION)
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            creative: false,
            test_terrain: false,
            frames: None,
            paced_by_split: false,
            split_guest: false,
            split_in_main_window: false,
            world_frames: None,
            screenshot_after: None,
            timeout: Duration::from_secs(120),
            report: None,
            world: None,
            look: None,
            time: None,
            weather: None,
            dimension: None,
            no_pause: false,
            control: None,
            offscreen: false,
            headless: false,
            window_size: [1280, 720],
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME")
                .ok()
                .filter(|n| !n.is_empty()),
            dedicated: None,
            benchmark: None,
            dev_mods: Vec::new(),
            server_thread: std::env::var("HORIZON_SERVER_THREAD").map_or(true, |v| {
                let on = v != "0";
                if !on {
                    warn_in_frame_deprecated("HORIZON_SERVER_THREAD=0");
                }
                on
            }),
            execution_role: execution_role::ExecutionRole::Client,
            fullscreen: false,
            game_dir: None,
            quick_play: None,
            session_file: std::env::var("HORIZON_SESSION_FILE").ok(),
            session_stdin: false,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition {} in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..128 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --width N, --height N   Window size (default 1280 x 720)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --world-frames N        Exit after N frames drawn in the world (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --benchmark SCENARIO    Run a benchmark (standard, village, forest, ocean, mountain), write its report to benchmarks/ and quit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --quickPlaySingleplayer WORLD  Open this world of saves/ at start, only if it exists (vanilla's)\n  --quickPlayMultiplayer HOST[:PORT]  The same as --server (vanilla's name)\n  --name NAME             Player name for multiplayer (default: generated once, kept in horizon-player.txt)\n  --session-file PATH     Signed-in account from the launcher (read once, then deleted)\n  --session-stdin         The same account as JSON on standard input\n  --game-dir DIR          Game folder: saves, options.txt, packs, mods (default: the working directory)\n  --home DIR              Logs and crash reports (default $HORIZON_HOME, else the platform data folder)\n  --fullscreen            Open the window fullscreen\n  --launch-api            Print the Launch API version and options as JSON (docs/api/launch.md)\n  --dev-mod PATH          A content mod (.hmod or folder) to develop: always on\n  --in-frame              DEPRECATED, do not use: unsupported, will be removed soon (also --no-server-thread)\n  --pregen                Add Horizon's /pregen command (docs/pregeneration.md)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause.",
                        horizon_core::VANILLA_VERSION
                    );
                    return Ok(None);
                }
                "--launch-api" => {
                    println!("{}", launch_api::reply(&horizon_client::crash::version()));
                    return Ok(None);
                }
                "--fullscreen" => o.fullscreen = true,
                "--session-stdin" => o.session_stdin = true,
                "--dedicated" => {
                    o.execution_role = execution_role::ExecutionRole::DedicatedServer;
                    o.dedicated = Some(args.by_ref().collect());
                    break;
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                "--no-pause" => o.no_pause = true,
                "--offscreen" => o.offscreen = true,
                "--headless" => {
                    o.headless = true;
                    o.offscreen = true;
                }
                "--control" => o.control = Some(horizon_client::control::default_path()),
                "--server-thread" => o.server_thread = true,
                "--in-frame" | "--no-server-thread" => {
                    warn_in_frame_deprecated(&arg);
                    o.server_thread = false;
                }
                // Horizon's own commands (/pregen), off by default as on vanilla.
                "--pregen" => horizon_client::command::vanilla::set_horizon_commands(true),
                _ => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("missing value after {arg}"))?;
                    // A launcher shows the error to the player: it names the option.
                    let read = || -> Result<()> {
                        match arg.as_str() {
                            "--resources" => o.resources = value.into(),
                            "--render-distance" | "--radius" => {
                                o.distance = value.parse()?;
                                o.distance_override = Some(o.distance);
                                let max = horizon_client::options_file::MAX_RENDER_DISTANCE;
                                if !(2..=max).contains(&o.distance) {
                                    return Err(format!("render distance must be 2..{max}").into());
                                }
                            }
                            "--seed" => {
                                o.seed = value.parse().unwrap_or_else(|_| {
                                    // Text seeds hash like String.hashCode.
                                    value
                                        .encode_utf16()
                                        .fold(0i32, |h, c| {
                                            h.wrapping_mul(31).wrapping_add(i32::from(c))
                                        })
                                        .into()
                                })
                            }
                            "--frames" => o.frames = Some(value.parse()?),
                            "--world-frames" => {
                                o.world_frames = Some(value.parse()?);
                                // A frame-limited run in every other respect; --timeout-seconds still applies.
                                o.frames.get_or_insert(u64::MAX);
                            }
                            "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                            "--smoke-report" => o.report = Some(value.into()),
                            "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                            "--width" => o.window_size[0] = value.parse::<u32>()?.max(1),
                            "--height" => o.window_size[1] = value.parse::<u32>()?.max(1),
                            // Vanilla opens only a level id that is one folder of saves/. --world, for
                            // harnesses, also takes a path (tools/bundle_tooltip_e2e.py).
                            "--world" => o.world = Some(value),
                            "--quickPlaySingleplayer" => {
                                let mut parts = std::path::Path::new(&value).components();
                                if !matches!(
                                    (parts.next(), parts.next()),
                                    (Some(std::path::Component::Normal(_)), None)
                                ) {
                                    return Err("not a folder name of saves/".into());
                                }
                                o.quick_play = Some(value);
                            }
                            "--game-dir" | "--gameDir" => o.game_dir = Some(value.into()),
                            // main has made it the game home already (logging::set_game_home).
                            "--home" => {}
                            "--dev-mod" => o.dev_mods.push(std::path::absolute(&value)?),
                            "--benchmark" => {
                                if horizon_client::benchmark::scenes(&value).is_none() {
                                    return Err(format!(
                                    "unknown benchmark {value}: standard, village, forest, ocean or mountain"
                                )
                                .into());
                                }
                                o.benchmark = Some(value);
                            }
                            "--server" | "--quickPlayMultiplayer" => o.server = Some(value),
                            "--name" => o.player_name = Some(value),
                            "--session-file" => o.session_file = Some(value),
                            "--control-socket" => o.control = Some(value.into()),
                            "--time" => o.time = Some(value.parse()?),
                            "--weather" => o.weather = Some(value),
                            "--dimension" => {
                                o.dimension = Some(if value.contains(':') {
                                    value
                                } else {
                                    format!("minecraft:{value}")
                                })
                            }
                            "--look" => {
                                let (y, p) =
                                    value.split_once(',').ok_or("--look takes YAW,PITCH")?;
                                o.look = Some((y.parse()?, p.parse()?));
                            }
                            _ => return Err("unknown option".into()),
                        }
                        Ok(())
                    };
                    read().map_err(|e| format!("{arg}: {e}"))?;
                }
            }
        }
        Ok(Some(o))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Playing,
    GameModeSwitcher,
    Paused,
    Options,
    Chat,
    Creative,
    Inventory,
    Dead,
    /// The Statistics screen.
    Stats,
    /// The Advancements screen.
    Advancements,
    /// WorldOptionsScreen (game/lan_screen.rs).
    WorldOptions,
    /// Play with Friends, from the pause menu's Friends button
    /// (`friends_screen`, game/friends.rs).
    Friends,
    /// CommandBlockEditScreen (game/command_blocks.rs).
    CommandBlock,
    /// A game master block's edit screen (game/block_editor.rs).
    BlockEditor,
    /// ServerLinksDialogScreen: a server's links (game/server_links.rs).
    ServerLinks,
    /// ConfirmLinkScreen for a server link (game/server_links.rs).
    ConfirmLink,
    /// DialogScreen.WarningScreen: the note about custom screens.
    DialogWarning,
    /// WinScreen: the end poem and the credits (game/win.rs).
    Win,
    /// DebugOptionsScreen, F3+F6 (game/debug_options.rs).
    DebugOptions,
}
impl Screen {
    /// The id native mods see for this screen (`NativeScreenView.id`, the
    /// `screen_render_post` hook): one per variant, and a hook's contract, so
    /// it stays the same from release to release. A new variant must add one.
    fn id(self) -> &'static str {
        match self {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "game_mode_switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Chat => "chat",
            Screen::Creative => "creative",
            Screen::Inventory => "inventory",
            Screen::Dead => "dead",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world_options",
            Screen::Friends => "friends",
            Screen::CommandBlock => "command_block",
            Screen::BlockEditor => "block_editor",
            Screen::ServerLinks => "server_links",
            Screen::ConfirmLink => "confirm_link",
            Screen::DialogWarning => "dialog_warning",
            Screen::Win => "win",
            Screen::DebugOptions => "debug_options",
        }
    }
}
struct Mining {
    pos: [i32; 3],
    progress: f32,
    ticks: u32,
}
#[derive(Clone, Copy)]
struct Hit {
    pos: [i32; 3],
    face: [i32; 3],
    point: [f64; 3],
}
/// What a falling block's tick did (`FallingBlockEntity.tick`).
enum FallStep {
    Falling,
    Gone,
    /// Gave up falling: drops itself as an item here.
    Dropped(Vec3),
    /// Hit the ground in this block.
    Landed([i32; 3]),
}
/// A guest's right click comes as one packet per hand and part
/// (`Minecraft.startUseItem`): `ServerGamePacketListenerImpl.handleUseItemOn`
/// does the block's and `ItemStack.useOn`'s part, `handleUseItem` the
/// item's own (`Item.use`). Running both for either would use the item twice.
#[derive(Clone, Copy)]
enum GuestUse {
    OnBlock(Hand),
    Item(Hand),
    /// `handleInteract`: `Player.interactOn` with the hand, nothing else (the
    /// client sends UseItem itself when the entity passed), on the entity the
    /// packet named: `ServerGamePacketListenerImpl.handleInteract` resolves
    /// `packet.entityId()` once and range checks it, so the click never picks
    /// an entity again from the view.
    Entity(Hand, InteractTarget),
}

/// The entity an interact packet names and where its client clicked it (the
/// packet's location, measured from the entity's position): what
/// `ServerGamePacketListenerImpl.handleInteract` takes from
/// `ServerboundInteractPacket` and hands to `Player.interactOn`, which passes
/// the point on to `interactAt`.
#[derive(Clone, Copy)]
struct InteractTarget {
    id: i32,
    location: [f64; 3],
}

/// The client world as seen by the vanilla block-behaviour library.
struct GameLevel<'w> {
    world: &'w mut World,
    facts: &'w BlockFacts,
    collision: &'w BlockCollisionRegistry<'w>,
    events: &'w mut Vec<LevelEvent>,
    dimension: LevelDimension,
    darken: i32,
    /// The environment of the level's dimension (`SUN_ANGLE`).
    environment: &'w Environment,
    raining: bool,
    thundering: bool,
    player_box: Option<([f64; 3], [f64; 3])>,
    /// The LAN guests in this level who trigger blocks (not spectators).
    guest_boxes: Vec<([f64; 3], [f64; 3])>,
    player_collision: Option<PlayerCollision>,
    /// Minecart boxes (detector rails).
    cart_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The item frames a comparator reads through a conductor.
    item_frames: &'w [horizon_client::entities::ItemFrame],
    /// The minecarts, in `cart_boxes` order, for a detector rail's comparator.
    carts: &'w [minecarts::Minecart],
    /// Arrow and thrown trident boxes (wooden buttons).
    arrow_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The other entities that keep blocks from being placed into them.
    blockers: Blockers<'w>,
    /// Item entities, experience orbs and projectiles, which trigger plates and
    /// tripwires but never keep a block from being placed.
    dropped: &'w [ItemEntity],
    orbs: &'w [ExperienceOrb],
    projectiles: &'w [horizon_client::projectiles::Projectile],
    biomes: &'w Biomes,
    seed: i64,
    behaviours: &'w Behaviours,
    items: &'w Items,
    /// Cooking recipes' experience, which a removed furnace pays out.
    recipes: &'w horizon_client::crafting::Recipes,
    /// Container loot tables (`unpackLootTable`).
    loot: Option<std::sync::Arc<horizon_world::loot::LootData>>,
    registry: &'static horizon_world::registry::BlockRegistry,
    rules: horizon_world::level::GameRules,
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    difficulty: i32,
    /// The feet of the players who keep fire spreading around them: the local
    /// player and the LAN guests in this level, spectators aside.
    fire_players: Vec<[f64; 3]>,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    rand_sequences: &'w mut horizon_client::random_sequences::RandomSequences,
    pending: &'w mut spill::Pending,
    spill: &'w mut spill::Spill,
    /// Where the chunk tickets let blocks tick.
    tickets: horizon_client::ticket_levels::Levels,
    random_tick_sections: &'w mut horizon_client::random_tick_sections::RandomTickSections,
    /// Players with each chest open (a trapped chest's signal).
    lid_openers: &'w HashMap<(i32, i32, i32), i32>,
    /// `Level.isClientSide`: a client's copy of the level (a guest of a LAN
    /// world, a window on its server thread, a dedicated server's client).
    client_side: bool,
    /// Only the integrated or dedicated server dispatches native observations.
    native_authoritative: bool,
    /// `Level.isDebug`: `setBlock` changes nothing.
    debug: bool,
    native_sounds: &'w mut Vec<creatures::NativeSound>,
    native_particles: &'w mut Vec<lan_particles::QueuedParticle>,
}
struct PlayerCollision {
    y: f64,
    descending: bool,
    fall_distance: f64,
    main_hand_item: Arc<str>,
    main_hand_count: i32,
    leather_boots: bool,
}
/// The entities `Entity.blocksBuilding` sets besides the players and
/// minecarts, borrowed for `Level.isUnobstructed`: living entities (mobs,
/// armor stands), boats, end crystals, primed TNT and falling blocks.
#[derive(Clone, Copy)]
struct Blockers<'w> {
    mobs: &'w [Mob],
    armor_stands: &'w [horizon_client::entities::ArmorStand],
    boats: &'w [boats::Boat],
    crystals: &'w [dragon::Crystal],
    tnts: &'w [tnt::PrimedTnt],
    falling: &'w [FallingBlock],
}
/// The boxes of the entities that block building, as obstruction candidates.
struct BuildingObstruction(Vec<Aabb>);
impl EntityObstructionWorld for BuildingObstruction {
    type Error = std::convert::Infallible;
    fn obstruction_candidates(
        &mut self,
        _excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> std::result::Result<Vec<ObstructionEntity>, Self::Error> {
        Ok(self
            .0
            .iter()
            .filter(|b| b.intersects(bounds))
            .enumerate()
            .map(|(i, &bounds)| ObstructionEntity {
                identity: EntityIdentity(i as u64),
                root_vehicle: EntityIdentity(i as u64),
                bounds,
                removed: false,
                spectator: false,
                blocks_building: true,
            })
            .collect())
    }
}
impl GameLevel<'_> {
    /// The living entities that trigger plates and tripwires and don't ignore
    /// block triggers (`Entity.isIgnoringBlockTriggers`: bats and marker armor
    /// stands). Armor stands live apart from `mobs`.
    fn mob_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        self.blockers
            .mobs
            .iter()
            .filter(|m| !m.removed && m.species() != horizon_client::mobs::Species::Bat)
            .map(Mob::bounds)
            .chain(
                self.blockers
                    .armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
    }
    /// `LilyPadBlock.entityInside`: the boxes of the boats that can break a pad.
    fn boat_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        let b = self.blockers;
        b.boats.iter().map(boats::Boat::bounds)
    }
    /// The non-living entities that trigger plates and tripwires (boats,
    /// crystals, primed TNT, falling blocks, item entities, experience orbs and
    /// projectiles).
    fn other_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        let b = self.blockers;
        let square = |p: Vec3, size: f64| {
            Aabb::new(
                Vec3::new(p.x - size / 2.0, p.y, p.z - size / 2.0),
                Vec3::new(p.x + size / 2.0, p.y + size, p.z + size / 2.0),
            )
        };
        b.boats
            .iter()
            .map(boats::Boat::bounds)
            .chain(b.crystals.iter().map(|c| {
                // EntityType.END_CRYSTAL: 2 x 2.
                let p = c.position;
                Aabb::new(
                    Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
                    Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
                )
            }))
            .chain(b.tnts.iter().map(tnt::PrimedTnt::bounds))
            .chain(
                b.falling
                    .iter()
                    .map(|f| horizon_client::falling_block::bounds(f.position)),
            )
            .chain(self.dropped.iter().map(ItemEntity::bounds))
            // EntityType.EXPERIENCE_ORB: 0.5 x 0.5.
            .chain(self.orbs.iter().map(move |o| square(o.position, 0.5)))
            .chain(
                self.projectiles
                    .iter()
                    .map(move |p| square(p.position, p.kind.size())),
            )
    }
    /// `DetectorRailBlock.getAnalogOutputSignal`: a powered rail reads the
    /// minecart on it, a command block cart first (its success count), else the first container cart's fullness.
    fn detector_rail_signal(&self, pos: BlockPos, powered: bool) -> i32 {
        if !powered {
            return 0;
        }
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        let area = Aabb::new(
            Vec3::new(x + 0.2, y, z + 0.2),
            Vec3::new(x + 0.8, y + 0.8, z + 0.8),
        );
        let mut container = None;
        for ((lo, hi), cart) in self.cart_boxes.iter().zip(self.carts) {
            let bounds = Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            );
            if !area.intersects(bounds) {
                continue;
            }
            match cart.kind {
                minecarts::CartKind::CommandBlock => return cart.command.success_count,
                minecarts::CartKind::Chest | minecarts::CartKind::Hopper => {
                    // Counted here, for the cart a comparator reads, not for
                    // every cart on every block tick.
                    container = container.or_else(|| Some(self.cart_signal(cart)));
                }
                _ => {}
            }
        }
        container.unwrap_or(0)
    }
    /// A container minecart's comparator signal: its open menu's slots while
    /// someone has it open, else its own.
    fn cart_signal(&self, cart: &minecarts::Minecart) -> i32 {
        let key = (i32::MAX - 15, minecarts::CART_KEY_Y, cart.id as i32);
        let slots = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Chest(open)) => open,
            _ => &cart.items,
        };
        minecarts::Minecart::container_signal(slots, self.items)
    }
    /// Every entity box that blocks building: the players who aren't
    /// spectators, the minecarts and the `Blockers` (a removed mob or a marker
    /// armor stand, whose box is empty, doesn't).
    fn building_blocker_boxes(&self) -> Vec<Aabb> {
        let boxed = |(lo, hi): &([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        let b = self.blockers;
        self.player_box
            .iter()
            .chain(&self.guest_boxes)
            .chain(&self.cart_boxes)
            .map(boxed)
            .chain(b.mobs.iter().filter(|m| !m.removed).map(Mob::bounds))
            .chain(
                b.armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
            .chain(b.boats.iter().map(boats::Boat::bounds))
            .chain(b.crystals.iter().map(|c| {
                // EntityType.END_CRYSTAL: 2 x 2.
                let p = c.position;
                Aabb::new(
                    Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
                    Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
                )
            }))
            .chain(b.tnts.iter().map(tnt::PrimedTnt::bounds))
            .chain(
                b.falling
                    .iter()
                    .map(|f| horizon_client::falling_block::bounds(f.position)),
            )
            .collect()
    }
}
impl LevelAccess for GameLevel<'_> {
    fn observe_block_changed(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        if self.native_authoritative {
            self.native_block_changed(self.dimension, pos, old, new);
        }
    }
    fn observe_neighbor_changed(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        changed: horizon_world::level::BlockId,
        moved: bool,
    ) {
        if self.native_authoritative {
            self.native_neighbor_changed(self.dimension, pos, state, changed, moved);
        }
    }
    fn allow_spread(
        &self,
        kind: horizon_world::level::Spread,
        from: BlockPos,
        to: BlockPos,
    ) -> bool {
        let mut canceled = false;
        if self.native_authoritative {
            let kind = match kind {
                horizon_world::level::Spread::Fluid => "fluid",
                horizon_world::level::Spread::Fire => "fire",
            };
            self.native_block_spread(self.dimension, kind, from, to, &mut canceled);
        }
        !canceled
    }
    fn allow_piston_move(
        &self,
        piston: BlockPos,
        direction: horizon_core::geometry::Direction,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        let mut canceled = false;
        if self.native_authoritative {
            let step = direction.normal();
            self.native_piston_move(self.dimension, piston, step, push, destroy, &mut canceled);
        }
        !canceled
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut source = vanilla;
        if self.native_authoritative {
            self.native_is_signal_source(state, &mut source);
        }
        source
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        let mut signal = vanilla;
        if self.native_authoritative {
            let face = direction.opposite();
            self.native_block_signal(self.dimension, pos, state, face, direct, &mut signal);
        }
        signal.clamp(0, 15)
    }
    fn adjust_has_analog_output(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_has_analog_output(state, &mut output);
        }
        output
    }
    fn adjust_analog_output(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: i32,
    ) -> i32 {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_analog_output(self.dimension, pos, state, direction, &mut output);
        }
        output.clamp(0, 15)
    }
    fn before_random_tick(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_random_tick(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    fn before_tick_block(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_tick_block(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    fn before_update_shape(
        &self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
        new: &mut Option<BlockStateId>,
    ) {
        if self.native_authoritative {
            self.native_update_shape(
                ticks,
                self.dimension,
                pos,
                state,
                direction,
                neighbour_pos,
                neighbour,
                new,
            );
        }
    }
    #[allow(clippy::option_option)]
    fn before_state_for_placement(
        &self,
        state: BlockStateId,
        context: &PlacementContext,
        fluid: FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        if self.native_authoritative {
            self.native_state_for_placement(self.dimension, state, context, fluid, answer);
        }
    }
    fn before_can_survive(&self, pos: BlockPos, state: BlockStateId, survives: &mut Option<bool>) {
        if self.native_authoritative {
            self.native_can_survive(self.dimension, pos, state, survives);
        }
    }
    fn before_may_place_on(
        &self,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_may_place_on(self.dimension, state, ground, ground_pos, may);
        }
    }
    fn is_client_side(&self) -> bool {
        self.client_side
    }
    fn is_valid_bonemeal_target(&self, pos: BlockPos, state: BlockStateId) -> Option<bool> {
        // Asked on both sides, as vanilla asks `isValidBonemealTarget`: a
        // client predicts the use with it. Only a mod answers.
        let mut valid = None;
        self.native_is_valid_bonemeal_target(self.dimension, pos, state, &mut valid);
        valid
    }
    fn is_bonemeal_success(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let mut success = false;
        if self.native_authoritative {
            self.native_is_bonemeal_success(self.dimension, pos, state, &mut success);
        }
        success
    }
    fn perform_bonemeal(&mut self, ticks: &mut Ticks, pos: BlockPos, state: BlockStateId) {
        if self.native_authoritative {
            self.native_perform_bonemeal(ticks, self.dimension, pos, state);
        }
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
    fn precipitation(&self, pos: BlockPos) -> Option<horizon_world::level::Precipitation> {
        self.behaviours.precipitation_info(self, pos)
    }
    fn is_raining_at(&self, pos: BlockPos) -> bool {
        self.behaviours.is_raining_at(self, pos)
    }
    fn block_entity_analog_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
    ) -> i32 {
        if let Some(s) = self.registry.state(state)
            && s.name == "minecraft:detector_rail"
        {
            return self.detector_rail_signal(
                pos,
                s.properties.get("powered").is_some_and(|v| v == "true"),
            );
        }
        if let Some(signal) = command_blocks::analog_signal(self.world, (pos.x, pos.y, pos.z)) {
            return signal;
        }
        if let Some(signal) = sculk::sensor_signal(self.world, self.behaviours, state, pos) {
            return signal;
        }
        if let Some((_, comparator)) = jukebox::jukebox_signals(self.world, self.items, pos) {
            return comparator;
        }
        if let Some(signal) = creaking::heart_signal(self.world, pos) {
            return if self.registry.state(state).is_some_and(|s| {
                s.properties
                    .get("creaking_heart_state")
                    .is_some_and(|v| v == "uprooted")
            }) {
                0
            } else {
                signal
            };
        }
        if let Some(signal) = books::lectern_signal(self.world, pos) {
            return signal;
        }
        if let Some(signal) =
            machines::shelf_signal(self.world, self.registry, state, pos, direction)
        {
            return signal;
        }
        machines::container_signal(self.world, self.behaviours, self.items, self.registry, pos)
    }
    fn item_frame_analog_output(&self, pos: BlockPos, facing: Direction) -> Option<i32> {
        let mut frames = self
            .item_frames
            .iter()
            .filter(|f| f.pos == [pos.x, pos.y, pos.z] && f.facing == facing as u8);
        let frame = frames.next().filter(|_| frames.next().is_none())?;
        // ItemFrame.getAnalogOutput.
        Some(
            frame
                .item
                .as_ref()
                .map_or(0, |_| i32::from(frame.rotation % 8) + 1),
        )
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        // TestBlock.ownSignal.
        if let Some(BlockEntity::TestBlock(b)) =
            self.world.block_entities.get(&(pos.x, pos.y, pos.z))
        {
            return b.own_signal();
        }
        // TrappedChestBlock.getSignal: the players with it open, at most 15.
        if let Some(count) = self.lid_openers.get(&(pos.x, pos.y, pos.z)) {
            return (*count).clamp(0, 15);
        }
        jukebox::jukebox_signals(self.world, self.items, pos).map_or(0, |(signal, _)| signal)
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }
    fn world_seed(&self) -> i64 {
        self.seed
    }
    fn day_time(&self) -> i64 {
        self.world.day_time
    }
    fn game_rules(&self) -> horizon_world::level::GameRules {
        self.rules
    }
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    /// `ServerLevel.canSpreadFireAround`: a player (not a spectator) closer
    /// than the `fire_spread_radius_around_player` rule to the block's corner
    /// (`ChunkMap.anyPlayerCloseEnoughTo`); -1 spreads everywhere.
    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        let radius = self.rules.fire_spread_radius_around_player;
        radius == -1
            || self.fire_players.iter().any(|p| {
                let d = [
                    p[0] - f64::from(pos.x),
                    p[1] - f64::from(pos.y),
                    p[2] - f64::from(pos.z),
                ];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < f64::from(radius) * f64::from(radius)
            })
    }
    fn increased_fire_burnout(&self, pos: BlockPos) -> bool {
        self.world
            .chunks
            .get(&(pos.x >> 4, pos.z >> 4))
            .is_some_and(|chunk| {
                self.biomes
                    .get(chunk.biome(pos.x, pos.y, pos.z))
                    .increased_fire_burnout
            })
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        // `Level.setBlock`: a server level of the Debug preset never changes.
        if self.debug && !self.client_side {
            return None;
        }
        if !self.world.loaded(pos.x, pos.z)
            || pos.y < self.world.bottom_y()
            || pos.y >= self.world.top_y()
        {
            return None;
        }
        let previous = self.world.block(pos.x, pos.y, pos.z);
        self.world.set(pos.x, pos.y, pos.z, state).ok()?;
        Some(previous)
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn max_y(&self) -> i32 {
        // Exclusive, as LevelAccess::max_y documents.
        self.world.shape.max_y()
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.world.random
    }
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 15)
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn sun_angle(&self, _pos: BlockPos) -> f32 {
        self.environment
            .sample(self.world.day_time, None, None)
            .sun_angle
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn is_thundering(&self) -> bool {
        self.thundering
    }
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
    /// `Level.isUnobstructed`: the placed state's collision shape against the player.
    /// `Level.isUnobstructed(state, pos, CollisionContext.of(player))`: no
    /// entity that blocks building is in the state's collision shape.
    fn is_unobstructed(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let player = self.player_collision.as_ref();
        let context = player.map_or_else(
            || CollisionContext::empty(false),
            |player| {
                CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(0),
                        y: player.y,
                        descending: player.descending,
                        main_hand: Some(CollisionItem {
                            item: &player.main_hand_item,
                            count: player.main_hand_count,
                        }),
                    },
                    false,
                )
            },
        );
        let mut scene = collisions::SceneCollisions {
            world: &*self.world,
            binding: self.collision,
            subject: player.map(|player| horizon_client::collisions::PowderSnowSubject {
                fall_distance: player.fall_distance,
                falling_block: false,
                leather_boots: player.leather_boots,
                walkable_type: false,
                lava_walker: false,
            }),
        };
        let Ok(shape) = self
            .collision
            .collision_shape_with_context(&mut scene, state, pos, &context)
        else {
            return false;
        };
        if shape.is_empty() {
            return true;
        }
        let shape = shape.moved(horizon_core::geometry::Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        horizon_world::entity_obstruction::is_unobstructed(
            &mut BuildingObstruction(self.building_blocker_boxes()),
            None,
            &shape,
        )
        .unwrap_or(false)
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded`: block ticking, and
    /// a ticking chunk.
    fn is_ticking_chunk(&self, chunk: horizon_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z) && ticking_chunk(self.world, chunk.x, chunk.z)
    }
    /// Entities that trigger pressure plates, tripwires, detector rails and
    /// wooden buttons: the players (the host and the LAN guests), the mobs,
    /// the minecarts and the other entities that don't ignore block triggers,
    /// and for `EntityQuery::Arrows` the arrows.
    fn count_entities(&self, area: Aabb, query: EntityQuery) -> usize {
        let hit = |(lo, hi): &([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ))
        };
        let carts = || self.cart_boxes.iter().filter(|b| hit(b)).count();
        let mobs = || {
            self.mob_trigger_boxes()
                .filter(|b| area.intersects(*b))
                .count()
        };
        let others = || {
            self.other_trigger_boxes()
                .filter(|b| area.intersects(*b))
                .count()
        };
        let arrows = || self.arrow_boxes.iter().filter(|b| hit(b)).count();
        let players = || {
            self.player_box
                .iter()
                .chain(&self.guest_boxes)
                .filter(|b| hit(b))
                .count()
        };
        match query {
            EntityQuery::Minecart => carts(),
            EntityQuery::Arrows => arrows(),
            EntityQuery::Mobs => players() + mobs(),
            EntityQuery::Everything => players() + carts() + mobs() + others(),
        }
    }
    fn emit(&mut self, event: LevelEvent) {
        match event {
            // Block.dropResources draws its loot and spots right away.
            // `popResource` only drops server-side (`!level.isClientSide`):
            // a client's moveBlocks (a piston's block event) skips it.
            LevelEvent::DropResources { pos, state } if !self.client_side => {
                self.drop_resources_now(pos, state)
            }
            LevelEvent::DropResources { .. } => {}
            // `ClientLevel.playSeededSound` plays only when the excluded
            // player is this client's own: the block's `playSound(null, ...)`
            // (a piston's move, a note) reaches it from the server instead.
            LevelEvent::Sound { .. } if self.client_side => {}
            // NoteBlock.triggerEvent plays the head's sound with a null
            // player, so only the server's level plays it; a guest or a
            // remote client hears it from the sound packet.
            LevelEvent::BlockEvent {
                kind: "note_custom_sound",
                ..
            } if self.client_side => {}
            // Block.popResource of the block's own item (tall flowers, flower beds).
            LevelEvent::BlockEvent {
                pos,
                state,
                kind: "pop_block_item",
            } => self.pop_block_item(pos, state),
            // ComposterBlock.extractProduce: bone meal above the composter.
            LevelEvent::Player {
                pos,
                effect: PlayerEffect::DropItem(item),
                ..
            } => self.pop_above(pos, item),
            // Picking a sweet berry bush or cave vines: the harvest loot,
            // popped before the pick sound draws its pitch.
            LevelEvent::Player {
                pos,
                state,
                effect: PlayerEffect::InteractLoot(table),
            } => self.drop_interact_loot(pos, state, table),
            event => self.events.push(event),
        }
    }
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        self.remove_entity(pos, old, side_effects)
    }
    fn load_block_entity(&mut self, pos: BlockPos, mut nbt: horizon_world::nbt::Compound) {
        use horizon_world::nbt::Tag;
        for (k, v) in [("x", pos.x), ("y", pos.y), ("z", pos.z)] {
            nbt.insert(k.into(), Tag::Int(v));
        }
        if let Some((key, entity)) = horizon_client::save::decode_block_entity(&Tag::Compound(nbt))
        {
            self.world.block_entities.insert(key, entity);
        }
    }
    fn set_command_block_powered(&mut self, pos: BlockPos, powered: bool) -> Option<(bool, bool)> {
        command_blocks::set_powered(self.world, self.registry, (pos.x, pos.y, pos.z), powered)
    }
    /// `PotentSulfurBlockEntity.resetCountdown`.
    fn reset_potent_sulfur_countdown(&mut self, pos: BlockPos) {
        if let Some(BlockEntity::PotentSulfur(sulfur)) =
            self.world.block_entities.get_mut(&(pos.x, pos.y, pos.z))
        {
            sulfur.waiting_countdown = -1;
        }
    }
}
/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    /// The entity id a server tracks it by (0 until it is tracked).
    id: u32,
    state: BlockStateId,
    /// `DATA_START_POS`: spawn position used to select weighted block models.
    start_pos: BlockPos,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    /// `FallingBlockEntity.time`: signed, a save can start it below zero.
    age: i32,
    /// `Entity.isNoGravity`.
    no_gravity: bool,
    /// `FallingBlockEntity.dropItem`.
    drop_item: bool,
    /// `Entity.fallDistance` (powder snow holds a block that fell far at 0.9).
    fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: a cobweb it is in scales its next move.
    stuck: Vec3,
    damage: horizon_client::falling_block::Damage,
    /// The UUID and compound it was loaded with, kept while it moves.
    saved: Option<Box<([i32; 4], horizon_world::nbt::Tag)>>,
}
impl FallingBlock {
    /// Where `FallingBlockRenderer` draws the block's model corner, `alpha`
    /// into the tick: centred on the entity, then moved by `offset`, the
    /// block's random model offset (`ModelBlockRenderer.tesselateBlock`:
    /// `BlockState.getOffset` of `BlockPos.containing(x, maxY, z)`), so a
    /// pointed dripstone keeps the place it hung at while it falls.
    fn model_origin(
        &self,
        alpha: f64,
        offset: impl Fn(BlockStateId, i32, i32) -> Option<[f64; 3]>,
    ) -> glam::DVec3 {
        let [dx, dy, dz] = offset(
            self.state,
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        )
        .unwrap_or([0.0; 3]);
        glam::DVec3::new(
            self.previous.x + (self.position.x - self.previous.x) * alpha - 0.5 + dx,
            self.previous.y + (self.position.y - self.previous.y) * alpha + dy,
            self.previous.z + (self.position.z - self.previous.z) * alpha - 0.5 + dz,
        )
    }
}

/// `FallingBlockRenderer.shouldRender` drops a falling block whose own state is
/// the state of the cell it is in (`level.getBlockState(entity.blockPosition())`):
/// a `NoGravity` block, or one that has just landed, sits inside an identical
/// block, and vanilla draws no model for it.
fn falling_block_visible(state: BlockStateId, cell: BlockStateId) -> bool {
    state != cell
}

/// The data loaded once at startup and never changed: blocks, items,
/// recipes, biomes, mobs, collision and motion. Read-only and `Send + Sync`,
/// so the client and an integrated server thread can share one copy.
pub struct Resources {
    core: WorldCore,
    rest: std::sync::OnceLock<WorldRest>,
}

struct WorldCore {
    facts: BlockFacts,
    biomes: Arc<Biomes>,
    environment: Environment,
    collision: BlockCollisionRegistry<'static>,
    motion: MotionResources,
    threads: usize,
}

struct WorldRest {
    items: Items,
    recipes: Recipes,
    transformers: horizon_client::block_transform::Transformers,
    mob_types: HashMap<String, Arc<MobType>>,
    mob_data: horizon_client::mobs::MobData,
    providers: Providers,
    block_entity_render: horizon_client::block_entity_render::BlockEntityRenderer,
    animate: horizon_client::animate_tick::AnimateTables,
    fog: horizon_client::fog::FogTables,
    music: horizon_client::music::MusicTables,
    ambient: horizon_client::ambience::AmbientTables,
    nether_environment: Environment,
    end_environment: Environment,
    behaviours: Behaviours,
    /// The player model's right and left arms.
    player_arm: [Option<horizon_client::mobs::Part>; 2],
    /// The slim model's right and left arms (3 pixels wide).
    player_arm_slim: [Option<horizon_client::mobs::Part>; 2],
    player_type: Option<Arc<horizon_client::mobs::MobType>>,
    /// `SpinAttackEffectModel` (Riptide's spinning boxes).
    spin_attack: Option<horizon_client::mobs::Part>,
    stations: horizon_client::stations::StationData,
}

/// The content tables a world's data packs change, loaded when the world
/// opens and on `/reload`: `ReloadableServerResources` loads a server's
/// recipes, loot tables and tags from the world's pack stack, over the
/// resource root's own. Worlds without packs use `Resources.rest` directly.
struct WorldTables {
    /// The pack files the tables were built from, for the advancement tree
    /// reload and the loot data.
    files: Vec<(String, Vec<Vec<u8>>)>,
    items: Items,
    recipes: Recipes,
    stations: horizon_client::stations::StationData,
}

impl Resources {
    fn rest(&self) -> &WorldRest {
        self.rest
            .get()
            .expect("world resources joined before entry")
    }
    fn facts(&self) -> &BlockFacts {
        &self.core.facts
    }
    fn items(&self) -> &Items {
        &self.rest().items
    }
    fn recipes(&self) -> &Recipes {
        &self.rest().recipes
    }
    fn transformers(&self) -> &horizon_client::block_transform::Transformers {
        &self.rest().transformers
    }
    fn biomes(&self) -> &Arc<Biomes> {
        &self.core.biomes
    }
    fn environment(&self) -> &Environment {
        &self.core.environment
    }
    fn mob_types(&self) -> &HashMap<String, Arc<MobType>> {
        &self.rest().mob_types
    }
    fn mob_data(&self) -> &horizon_client::mobs::MobData {
        &self.rest().mob_data
    }
    fn providers(&self) -> &Providers {
        &self.rest().providers
    }
    fn block_entity_render(&self) -> &horizon_client::block_entity_render::BlockEntityRenderer {
        &self.rest().block_entity_render
    }
    fn animate(&self) -> &horizon_client::animate_tick::AnimateTables {
        &self.rest().animate
    }
    fn fog(&self) -> &horizon_client::fog::FogTables {
        &self.rest().fog
    }
    fn music(&self) -> &horizon_client::music::MusicTables {
        &self.rest().music
    }
    fn ambient(&self) -> &horizon_client::ambience::AmbientTables {
        &self.rest().ambient
    }
    fn nether_environment(&self) -> &Environment {
        &self.rest().nether_environment
    }
    fn end_environment(&self) -> &Environment {
        &self.rest().end_environment
    }
    fn behaviours(&self) -> &Behaviours {
        &self.rest().behaviours
    }
    fn collision(&self) -> &BlockCollisionRegistry<'static> {
        &self.core.collision
    }
    fn motion(&self) -> &MotionResources {
        &self.core.motion
    }
    fn threads(&self) -> usize {
        self.core.threads
    }
    fn player_arm(&self, slim: bool, left: bool) -> &Option<horizon_client::mobs::Part> {
        let arms = if slim {
            &self.rest().player_arm_slim
        } else {
            &self.rest().player_arm
        };
        &arms[usize::from(left)]
    }
    fn player_type(&self) -> &Option<Arc<horizon_client::mobs::MobType>> {
        &self.rest().player_type
    }
    fn spin_attack(&self) -> &Option<horizon_client::mobs::Part> {
        &self.rest().spin_attack
    }
    fn stations(&self) -> &horizon_client::stations::StationData {
        &self.rest().stations
    }
}

/// The equipment slot of `armor[index]`: head, chest, legs, feet.
pub(super) fn armor_slot(index: usize) -> horizon_world::player_inventory::EquipmentSlot {
    use horizon_world::player_inventory::EquipmentSlot;
    match index {
        0 => EquipmentSlot::Head,
        1 => EquipmentSlot::Chest,
        2 => EquipmentSlot::Legs,
        _ => EquipmentSlot::Feet,
    }
}

/// Everything the title and menus draw: the GUI atlas with its font and the
/// title panorama. Kept apart from the world bundle so the title can appear
/// before the block sprites, models and gameplay tables are ready.
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<horizon_client::gfx::panorama::Faces>,
}
impl UiResources {
    /// From a resource root layering packs over `defaults`, the game's own
    /// resources (`root` itself when no pack is on).
    pub fn load(root: &std::path::Path, defaults: &std::path::Path) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::build_over(root, defaults)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::load(root)),
        })
    }
    /// What a server's game holds, since it draws nothing: the font, which books
    /// and chat measure with, and a one-pixel panorama.
    pub fn server(root: &std::path::Path) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::fonts(root)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces {
                size: 1,
                pixels: [0u8; 4].repeat(6),
            }),
        })
    }
}
/// What a resource pack reload replaces (`Minecraft.reloadResourcePacks`):
/// block models and sprites with their meshing tables, item models,
/// translations and the textures read on the CPU. Each game holds its own;
/// cloning shares the loaded data.
#[derive(Clone)]
pub struct PackResources {
    assets: Arc<Assets>,
    render: Arc<RenderAssets>,
    context: Arc<WorkerContext>,
    layer_names: Arc<HashMap<u32, String>>,
    /// Item definitions, baked item models and special renderers.
    item_renderer: Arc<horizon_client::item_render::ItemRenderer>,
    names: Arc<Names>,
    /// clouds.png: (width, height, filled cells).
    clouds: Arc<(u32, u32, Vec<bool>)>,
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    forcefield: Arc<(u32, u32, Vec<u8>)>,
}
impl PackResources {
    /// Everything a pack overlay at `root` provides, over `assets` and `render`
    /// loaded from it.
    fn load(
        root: &std::path::Path,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
    ) -> Result<Self> {
        let visibility = models::load_visibility(root, assets.registry)?;
        let tables = Arc::new(MeshTables::new(&assets, &render, facts, visibility)?);
        let item_renderer = horizon_client::item_render::ItemRenderer::load_with_library(
            root,
            render.model_library.clone(),
        )?;
        Ok(Self::assemble(
            root,
            assets,
            render,
            tables,
            item_renderer,
            biomes,
            light,
        ))
    }
    /// The pack tables over their parts, which `load_resources` builds on
    /// separate workers.
    fn assemble(
        root: &std::path::Path,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        tables: Arc<MeshTables>,
        item_renderer: horizon_client::item_render::ItemRenderer,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
    ) -> Self {
        Self {
            layer_names: Arc::new(
                render
                    .sprite_layers
                    .iter()
                    .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
                    .collect(),
            ),
            context: Arc::new(WorkerContext {
                assets: assets.clone(),
                render: render.clone(),
                tables,
                biomes,
                light,
            }),
            assets,
            render,
            item_renderer: Arc::new(item_renderer),
            names: Arc::new(Names::load(root)),
            clouds: Arc::new(load_clouds(root)),
            forcefield: Arc::new(load_forcefield(root)),
        }
    }
}
/// A quick craft drag as the screen draws it: each dragged slot's count and
/// whether the slot caps it (None: hidden), and what stays in hand.
struct DragView {
    slots: HashMap<usize, Option<(u32, bool)>>,
    left: Option<u32>,
}
/// Whether an item modifier of slot `group` applies in the worn or held `slot`
/// (`EquipmentSlotGroup.test`).
fn equipment_modifier_applies(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "armor" && slot != "mainhand" && slot != "offhand")
        || (group == "hand" && (slot == "mainhand" || slot == "offhand"))
}
/// Ctrl+Q reads the physical keyboard, independently of sprint key mappings.
/// A chunk whose ticking future can complete (`ChunkMap.prepareTickingChunk`):
/// lit, with its neighbours loaded, as `postprocess_chunks` waits for.
fn ticking_chunk(world: &World, x: i32, z: i32) -> bool {
    let Some(center) = world.chunks.get(&(x, z)) else {
        return false;
    };
    if !center.lit {
        return false;
    }
    for dz in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dz == 0 {
                continue;
            }
            if !world.chunks.contains_key(&(x + dx, z + dz)) {
                return false;
            }
        }
    }
    true
}
/// Key events carry the scancode's US-layout code, which ignores XKB options like
/// `caps:swapescape`. For the keys whose meaning the keymap may remap, take the code
/// from the keysym the keymap resolved instead.
fn follow_keymap(event: &mut WindowEvent) {
    use winit::keyboard::{Key, NamedKey};
    if let WindowEvent::KeyboardInput { event, .. } = event
        && let Key::Named(named) = &event.logical_key
    {
        let code = match named {
            NamedKey::Escape => KeyCode::Escape,
            NamedKey::CapsLock => KeyCode::CapsLock,
            _ => return,
        };
        event.physical_key = PhysicalKey::Code(code);
    }
}

/// Q throws the whole stack with Control down: the same modifier word the
/// screens read (`has_control_down`), never a latched sprint toggle.
fn drop_whole_stack(modifiers: winit::keyboard::ModifiersState) -> bool {
    modifiers.control_key()
}

/// The modifier `modifiers` stands for, as GLFW words it in the native mods'
/// input hooks: `GLFW_MOD_SHIFT` 1, `GLFW_MOD_CONTROL` 2, `GLFW_MOD_ALT` 4,
/// `GLFW_MOD_SUPER` 8 — the word a NeoForge listener reads off its event
/// (`getModifiers`). The lock keys are not modifiers there.
fn glfw_modifiers(modifiers: winit::keyboard::ModifiersState) -> u32 {
    let mut bits = 0;
    if modifiers.shift_key() {
        bits |= 1;
    }
    if modifiers.control_key() {
        bits |= 2;
    }
    if modifiers.alt_key() {
        bits |= 4;
    }
    if modifiers.super_key() {
        bits |= 8;
    }
    bits
}

/// The GLFW mouse button index `button` is, as the native mods' mouse hooks
/// hand it over: 0 left, 1 right, 2 middle, 3 back, 4 forward. `Other(n)` is
/// winit's n, the platform's own number (X11 or evdev on Linux), so a button
/// past 4 is not GLFW's index everywhere. Buttons 0–4 are the numbering
/// `keybinds::mouse_name` names (`key.mouse.<index + 1>`) and vanilla's
/// `InputConstants` reads.
fn glfw_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => i32::from(n),
    }
}

fn select_creative_tab(
    creative: &mut Creative,
    clicks: &mut horizon_client::container_screen::Clicks,
    tab: usize,
) {
    creative.select(tab);
    *clicks = Default::default();
    // Tabs are outside the panel; their release must not drop a carried stack.
    clicks.skip_release();
}

/// The icon the Select World list shows for the world in play
/// (`GameRenderer.tryTakeScreenshotIfNeeded`): its path in the save, and when
/// the last frame grab was asked for.
struct WorldIcon {
    path: PathBuf,
    last_attempt: Option<Instant>,
}

struct Game<'a> {
    /// Shared with an integrated server; `collision` and `motion` borrow from it.
    res: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pending_entry: Option<PendingWorldEntry>,
    world_loader: Option<world_loading::WorldLoader>,
    /// `DebugEntryChunkGeneration`'s `lastPos` and `result`: F3's `Density` line is sampled
    /// again only when the feet block (or the dimension) changes.
    debug_density: RefCell<debug_keys::DensityCache>,
    pending_pack_reload: bool,
    /// When `resourcepacks/` was last looked at for the open pack screen, and what it held
    /// (`watch_pack_folder`).
    pack_folder: Option<(
        std::time::Instant,
        Vec<(String, u64, Option<std::time::SystemTime>)>,
    )>,
    /// A pack reload staged on a worker (`LoadingOverlay`): the frame draws
    /// its progress until `drive_pack_reload` applies it.
    pack_reload: Option<packs::PackReload>,
    /// A server's resource packs (game/server_packs.rs).
    server_packs: server_packs::ServerPackState,
    /// The link a chat click asks to open (`ConfirmLinkScreen`).
    link_prompt: Option<String>,
    /// A server's Code of Conduct to accept (game/code_of_conduct.rs).
    code_of_conduct: Option<code_of_conduct::Prompt>,
    dialogs: dialogs::DialogState,
    /// Single player's server while it loads its world (`Minecraft.doWorldLoad`).
    integrated_starting: Option<integrated::Starting>,
    world_renderer_ready: bool,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    /// This game's resource packs (`reload_packs` replaces them).
    pack: PackResources,
    /// The title and menu bundle (GUI atlas, font and panorama).
    gui: UiResources,
    /// `FogRenderer`'s evolving state.
    fog: horizon_client::fog::FogState,
    screen_fx: horizon_client::screen_effects::ScreenEffects,
    /// `LocalPlayer.itemActivation`: the totem flying up to the screen.
    item_activation: horizon_client::item_activation::ItemActivation,
    weather_parameters: horizon_client::weather::ParameterCache,
    panorama_rotation: horizon_client::gfx::panorama::Rotation,
    world: World,
    /// The biome table the world plays with: the game's own with the world's
    /// packs over it (`Biomes::with_packs`), `None` before a world opens.
    world_biomes: Option<Arc<Biomes>>,
    streaming: Streaming,
    lod: Option<horizon_client::lod::Lod>,
    lod_source_attempted: bool,
    lod_observe: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    /// Beacon beam sections kept after their chunk unloads, so far terrain
    /// LOD still draws their beams (`beacon_geometry` refreshes them).
    far_beacons: HashMap<(i32, i32, i32), Vec<(u32, i32)>>,
    /// The detail revision, centre and radius `Lod::update` last had covered for.
    lod_covered_at: Option<(u64, (i32, i32), i32)>,
    options: Options,
    player: Option<PlayerMotion>,
    spawn: Option<Vec3>,
    clock: FixedStep,
    previous_position: Vec3,
    /// `yRotO` / `xRotO` of the local player: the view at the start of the tick.
    rope_rot_old: (f32, f32),
    eye: f32,
    previous_eye: f32,
    yaw: f32,
    pitch: f32,
    inventory: Vec<Option<ItemStack>>,
    selected: usize,
    carried: Option<ItemStack>,
    craft_grid: Vec<Option<ItemStack>>,
    craft_size: usize,
    /// The crafting table the 3x3 grid belongs to (`CraftingMenu.access`).
    craft_table: Option<[i32; 3]>,
    /// A quick craft drag over inventory slots: (button, game slots so far).
    inv_drag: Option<(i32, Vec<usize>)>,
    /// `AbstractContainerScreen`'s quick craft and skip-release state.
    inv_clicks: horizon_client::container_screen::Clicks,
    /// The Mouse Tweaks extra's drag and wheel (`tweaks`).
    tweaks: tweaks::Tweaks,
    /// The wheel and hovered slot of `BundleMouseActions`.
    bundle_mouse: bundles::BundleMouse,
    /// The last inventory click, for the double click that gathers a stack.
    last_inv_click: Option<(std::time::Instant, usize, bool)>,
    /// `lastQuickMoved`: what the last Shift click moved, which a Shift
    /// double click moves every stack of.
    last_quick_moved: Option<ItemStack>,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    station: Option<stations::Station>,
    /// Status effects and thrown potions (game/potions.rs).
    potions: potions::PotionState,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    /// The other players' cracks (`LevelRenderer.destroyingBlocks`): breaker id to the block,
    /// the stage (0..=9) and the game time it last changed.
    destroying: HashMap<i32, ([i32; 3], u8, i64)>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    /// The game mode; `creative` mirrors `mode == Creative`.
    mode: horizon_client::command::args::GameMode,
    keys: HashSet<KeyCode>,
    // Physical input is separate from remapped and toggled movement keys.
    physical_keys: HashSet<KeyCode>,
    physical_buttons: HashSet<MouseButton>,
    /// Control + left click as a right click (macOS).
    right_click_emulation: horizon_client_input::mouse::RightClickEmulation,
    left: bool,
    right: bool,
    cursor: (f32, f32),
    /// The cursor icon the window shows (`widgets::take_cursor` each frame).
    cursor_icon: horizon_client::widgets::Cursor,
    captured: bool,
    /// Only the local player owning keyboard and mouse changes the native pointer.
    keyboard_mouse: bool,
    /// This view's corner in its shared window; cursor warps include the offset.
    view_at: [u32; 2],
    /// Splitscreen: what it draws over this player's view (its badge, questions, notices).
    split_hud: split::Hud,
    focused: bool,
    /// Whether the window currently accepts input method events.
    ime_allowed: bool,
    /// The candidate window's area last given to the window, in pixels.
    ime_area: Option<[i32; 4]>,
    /// When the window lost focus, while it is still without
    /// (`Minecraft.lastActiveTime`: pausing waits 500 ms).
    focus_lost: Option<Instant>,
    /// The keyboard's modifier state (`KeyEvent.modifiers`).
    modifiers: winit::keyboard::ModifiersState,
    /// The window's fullscreen request and reported state.
    fullscreen: horizon_client::window::FullscreenState,
    /// The monitor whose video modes `settings.video_modes` holds.
    video_modes: horizon_client::window::VideoModeList,
    /// MouseHandler's accumulated look and wheel state.
    mouse: MouseLook,
    /// An agent moved the mouse (`mouse-move`) since the pointer was last
    /// grabbed or released: its motion turns the view like a grabbed pointer.
    agent_pointer: bool,
    /// `Options.smoothCamera` (cinematic camera), not persisted.
    smooth_camera: bool,
    /// Sneak, sprint, attack and use as ToggleKeyMappings.
    toggles: horizon_client::keymapping::ToggleKeys,
    /// A world screen with the pointer captured was open last frame.
    was_playing: bool,
    /// Minecraft.pause: a single-player screen that pauses the game is open.
    pause: bool,
    debug: bool,
    screen: Screen,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// CPU geometry storage retained across frames, including empty/title frames.
    entity_vertices: Vec<Vertex>,
    /// Horizon Extras' dynamic lights this frame (`dynamic_lights.rs`), and how brightly each
    /// item glows (built the first time the extra is on).
    dynamic_lights: Vec<[f32; 4]>,
    luminance: Option<horizon_client::dynamic_light::Luminance>,
    /// Name tags drawn over everything (`RenderPipelines.TEXT_SEE_THROUGH`).
    see_through_vertices: Vec<Vertex>,
    /// The first-person player's model, which only a native pack's shadow pass draws.
    shadow_only_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// Runs of `entity_vertices` by entity, for a shader pack's per-entity draws; recorded only
    /// while `pack_ids` is set (game/shader_packs.rs).
    entity_spans: std::cell::RefCell<Vec<horizon_client::gfx::pack_draw::EntitySpan>>,
    /// The glowing entities' silhouettes this frame (game/glowing.rs).
    glowing_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// The shader pack's entity and block entity ids, while one draws entities.
    pack_ids: Option<shader_packs::PackIds>,
    started: Instant,
    /// `SpectatorGui`: the menu a spectator opens with the hotbar keys.
    spectator_gui: horizon_client::spectator_menu::SpectatorGui,
    previous: Instant,
    /// FramerateLimitTracker.latestInputTime.
    last_input: Instant,
    /// Controllers (Horizon's own).
    pad: pad::Pads,
    frames: u64,
    /// The glint clock and options, read once at the start of each frame
    /// (game/item_draw.rs).
    glint_frame: horizon_client::item_render::GlintFrame,
    /// Mod block entities' geometry their mods kept
    /// (game/native_block_entity_draw.rs).
    kept_block_entities: std::cell::RefCell<native_block_entity_draw::KeptGeometry>,
    /// `item_level` for one frame, game time, day time and dimension.
    item_level_cache: std::cell::RefCell<
        Option<(
            (u64, i64, i64, &'static str),
            horizon_assets::item_model::LevelState,
        )>,
    >,
    /// Frames drawn with the player in the world, for `--world-frames`.
    world_frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    health_hud: hud_layout::HealthHud,
    /// What ends the game with an error; kept as it is, so main can tell a debug crash.
    error: Option<Box<dyn std::error::Error>>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    session_lock: Option<horizon_client::save::SessionLock>,
    audio: Audio,
    music: horizon_client::music::MusicManager,
    /// `ToastManager.nowPlayingToast`: there while Music Toast is not Never.
    now_playing: Option<horizon_client::toasts::NowPlayingToast>,
    /// The Music Toast option the toast was last set up for.
    music_toast_state: u8,
    music_notes: horizon_client::toasts::MusicNotes,
    ambience: horizon_client::ambience::Ambience,
    /// The sound engine was told the game paused (pauseAllExcept).
    sound_paused: bool,
    /// Minecarts whose `MinecartSoundInstance` is playing.
    cart_sounds: HashSet<u32>,
    /// The minecart the riding sounds were started for (`LocalPlayer.startRiding`).
    riding_cart_sounds: Option<u32>,
    /// The happy ghast its riding sound was started for (`LocalPlayer.startRiding`).
    riding_sound: Option<u32>,
    /// `LocalPlayer.wasFallFlying` for the elytra's flying sound.
    was_fall_flying: bool,
    /// Bees whose buzz is playing, and whether it is the aggressive one.
    bee_sounds: HashMap<u32, bool>,
    /// Guardians whose attack hum is playing.
    guardian_sounds: HashSet<u32>,
    dropped: Vec<ItemEntity>,
    next_entity: u32,
    /// The last entity id the running server tick found before its levels
    /// ticked: an item with a later one was added this tick and first ticks
    /// on the next (`EntityTickList`). `u32::MAX` until a tick has run.
    ticked_entities_through: u32,
    particles: horizon_client::particles::ParticleEngine,
    /// `ItemPickupParticle`s: taken items, orbs and arrows flying to their collector.
    pickups: pickups::Pickups,
    /// Sign text being edited (`SignEditScreen`).
    sign_edit: Option<sign_edit::SignEditor>,
    /// An open book, book and quill or lectern screen.
    book: Option<books::BookScreen>,
    /// The last click on the book screen, for double clicks.
    book_last_click: Option<std::time::Instant>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    /// LevelLoadingScreen.smoothedProgress while the player is not placed yet.
    loading_progress: f32,
    /// `CreateWorldScreen.openCreateWorldScreen`'s data read, on a worker,
    /// while the title says `createWorld.preparing`.
    creation_job: Option<std::thread::JoinHandle<std::result::Result<(), String>>>,
    /// When `loading_progress` last eased (the server thread's load, 50 ms a step).
    loading_progress_at: Option<std::time::Instant>,
    /// The world spawn search's origin chunk and fallback height, once known.
    spawn_origin: Option<((i32, i32), i32)>,
    spawn_spiral: horizon_client::world_spawn::ChunkSpiral,
    /// A new player's spawn search, while its candidates' chunks load.
    player_spawn: Option<horizon_client::world_spawn::PlayerSpawnFinder>,
    /// Chunks a respawn search waits for this tick, kept loading beside the
    /// player's own (taken by `update_world`).
    spawn_wait: Vec<(i32, i32)>,
    mobs: Vec<Mob>,
    creatures: creatures::Creatures,
    sculk: sculk::SculkState,
    raiding: raids::Raiding,
    attack_ticks: u32,
    /// The main hand's item at the last player tick, whose attack damage and
    /// speed modifiers apply (`LivingEntity.detectEquipmentUpdates`): a hotbar
    /// swap just before an attack keeps them, with the new item's enchantments
    /// (attribute swapping).
    attack_item: Option<Arc<str>>,
    /// Mobs a player hurt (`LivingEntity.lastHurtByPlayer`), with the guest
    /// it was by connection (None: the host).
    last_hurt_by_player: HashMap<u32, Option<u32>>,
    /// Leashed mobs a player killed, with the killer's UUID: the lead drops
    /// on the corpse's next tick (`Leashable.tickLeash`), for the mods. With
    /// the corpse's dimension: each level's tick prunes only its own.
    lead_killers: HashMap<u32, (u128, Dim)>,
    /// Animals a player fed into love (`Animal.loveCause`), with the guest it was by
    /// connection (None: the host). Breeding credits them, wherever they are.
    love_cause: HashMap<u32, Option<u32>>,
    /// Mobs whose death (`LivingEntity.die`) has been handled.
    /// Looked up per mob each tick: a fast hasher (never iterated).
    died: horizon_client::fast_hash::FxHashSet<u32>,
    /// Client-ticked block-entity animation state (`game/block_entity_anims.rs`).
    be_anims: horizon_client::block_entity_anim::BlockEntityAnims,
    /// Item being used (eaten): hotbar slot and ticks used.
    using: Option<(usize, u32)>,
    /// `getUseDuration` when that use started, as a mod left it
    /// (`game/item_use.rs`): consumables and mod items count to it.
    use_duration: i32,
    /// Camera.oldFovModifier / fovModifier.
    /// `Camera` (eye height and field-of-view easing, view placement).
    view: horizon_client::camera::Camera,
    /// `FramerateLimiter` and its clock.
    limiter: horizon_client::window::FramerateLimiter,
    frame_clock: horizon_client::window::SystemClock,
    /// Hud.scopeScale (spyglass overlay size).
    scope_scale: f32,
    /// Duration of the last frame in ticks (GUI easing).
    frame_ticks: f32,
    title: Option<Title>,
    /// Armor slots: head, chest, legs, feet.
    armor: [Option<ItemStack>; 4],
    /// The off hand (Inventory slot 40, saved as slot 150; menu slot 504 here).
    offhand: Option<ItemStack>,
    /// `ItemStack.popTime` of the nine hotbar stacks, then the off hand: set to 5
    /// when a slot gains items, counted down each tick.
    hotbar_pop: [u8; 10],
    /// The hand an item use acts with (`InteractionHand`): the off hand only
    /// for its turn in `use_item` and while its item is being used.
    hand: Hand,
    /// The shield's item cooldown after an axe disabled it (ticks).
    shield_cooldown: u32,
    /// `Player.getCooldowns()`: the `use_cooldown` component's entries, what
    /// `itemDecorations` shades a slot by. A server's players have their own
    /// (`GuestPlay::cooldowns`), swapped in for their turn.
    cooldowns: horizon_client::cooldowns::Cooldowns,
    /// The source of a command run through /execute (the player otherwise).
    command_source: Option<commands::execute::Source>,
    titles: titles::Titles,
    /// `Hud.toolHighlightTimer`: the selected item name.
    tool_highlight: horizon_client::hud_overlays::ToolHighlight,
    /// `Hud.autosaveIndicatorValue`.
    saving_indicator: horizon_client::hud_overlays::SavingIndicator,
    /// This game's world is autosaving this tick (`MinecraftServer.isSaving`).
    autosaving: bool,
    applied_modifiers: Vec<(String, String)>,
    experience: Experience,
    orbs: Vec<ExperienceOrb>,
    xp_delay: u32,
    last_level_up: u64,
    /// The arm swing (`LivingEntity.SwingState`) and how many swings started.
    swing: horizon_client::first_person::SwingState,
    swings_started: u32,
    /// The first-person equip heights and the items the hands show (with
    /// the slot each came from).
    heights: horizon_client::first_person::HandHeights,
    /// The hands' lag behind the view (`LocalPlayer.xBob`, `yBob`); a new
    /// LocalPlayer (joining, respawning, changing dimension) starts at 0.
    sway: horizon_client::first_person::HandSway,
    visible_hands: [(usize, Option<ItemStack>); 2],
    /// Whether the main and off hand's slots were set to a new stack that
    /// the hand doesn't show yet (`Game::hand_stack_replaced`).
    hands_replaced: [bool; 2],
    /// `Minecraft`'s attack handling, attack clicks since the last tick and
    /// the attack strength tickers the hand reads.
    clicks: horizon_client::first_person::Clicks,
    attack_clicks: u32,
    /// Use clicks since the last tick; `handleKeybinds` consumes them after the attack clicks.
    use_clicks: u32,
    hand_strength: horizon_client::attack_strength::AttackStrength,
    settings: Settings,
    tutorial: tutorial::Tutorial,
    tutorial_toasts: Vec<tutorial::HintToast>,
    /// The `SystemToastId.NARRATOR_TOGGLE` toast, wherever the player is.
    narrator_toast: Option<horizon_client::toasts::SystemToast>,
    /// The open screen's narration (`Screen`'s narration state and timing).
    narration: horizon_client::screen_narrator::ScreenNarrator,
    /// Where the options screen returns: true = title, false = pause menu.
    options_from_title: bool,
    dragging: bool,
    /// `ClientAvatarState`: walked distance and view bob.
    avatar: horizon_client::camera::AvatarBob,
    flash: u32,
    /// `ClientLevel.rainSoundTime`: ticks of rain since the last rain sound.
    rain_sound_time: i32,
    /// Bolts that struck here and sky flashes they made, in total (`status`).
    lightning_seen: [u32; 2],
    /// The head of the bed while sleeping; the visual timer also survives waking.
    sleeping: Option<[i32; 3]>,
    sleep_timer: u32,
    /// The beds players sleep in that this game marked `occupied`, by level
    /// (game/beds.rs).
    player_beds: Vec<(Dim, [i32; 3])>,
    /// `SleepStatus`'s active and sleeping players at the last announcement.
    sleep_seen: (i64, i64),
    dim: Dim,
    world_root: PathBuf,
    /// The world's data packs' content tables, when it has packs (`None`
    /// keeps `Resources.rest`'s own in use).
    world_tables: Option<WorldTables>,
    /// The world is superflat (`ServerLevel.isFlat`, for every dimension).
    world_flat: bool,
    /// The world is the Debug preset (`Level.isDebug`, for every dimension): its blocks
    /// cannot be changed and nothing ticks.
    world_debug: bool,
    /// A player carried through a portal, re-placed once the destination loads.
    travelling: Option<(PlayerMotion, Vec3)>,
    /// A server's players on their way to another level, each waiting for
    /// the destination to load around them.
    server_trips: Vec<server_travel::Trip>,
    /// Players a command sends to another level, once it has run.
    level_teleports: Vec<server_travel::LevelTeleport>,
    /// A server's levels other than the one in place (game/levels.rs).
    levels: levels::Levels,
    /// Which pass of a server tick runs (`tick_server`): the first one reads
    /// the network, the last one ends the players' tick.
    level_pass: levels::Pass,
    /// Nether portals: the player's, items' and mobs' (game/nether_portal.rs).
    portals: nether_portal::Portals,
    /// The pending travel goes through an End portal (EndPortalBlock destinations).
    end_travel: bool,
    /// The respawn point when it is not the world spawn or a bed: a respawn
    /// anchor (`game/respawn_anchor.rs`).
    respawn_config: Option<respawn_anchor::RespawnConfig>,
    /// A respawn travelling to its anchor's dimension and chunks.
    respawn_travel: Option<respawn_anchor::RespawnConfig>,
    /// The next enter_dimension call comes from an End portal.
    end_portal_travel: bool,
    /// The open WinScreen (game/win.rs).
    win: Option<win::Win>,
    end: end::EndState,
    block_ticks: Ticks,
    tick_chunks: horizon_client::tick_chunk_membership::TickChunkMembership,
    tickets: horizon_client::ticket_levels::ChunkTickets,
    random_tick_sections: horizon_client::random_tick_sections::RandomTickSections,
    level_events: Vec<LevelEvent>,
    falling: Vec<FallingBlock>,
    /// Blocks the server shows for a moment (`ClientboundAddTransientBlockPacket`).
    transient_blocks: transient_blocks::TransientBlocks,
    paintings: Vec<horizon_client::entities::Painting>,
    painting_variants: paintings::PaintingVariants,
    item_frames: Vec<horizon_client::entities::ItemFrame>,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    /// The `map=true` frame variants.
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stands: Vec<horizon_client::entities::ArmorStand>,
    /// `Cushion` decoration entities (game/cushions.rs).
    cushions: Vec<horizon_client::entities::Cushion>,
    /// Entities of types Horizon does not simulate.
    others: Vec<commands::OtherEntity>,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    /// `WaypointStyleManager`.
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    /// `ClientWaypointManager`: the locator bar's waypoints.
    waypoints: horizon_client::locator_bar::ClientWaypoints,
    /// The camera the last frame drew from (position, yaw, pitch, fov), for the locator bar.
    locator_camera: Option<([f64; 3], f32, f32, f32)>,
    /// `LocalPlayer.postEffects`: the chains the server has the screen run
    /// (`ClientboundPostEffectsPacket`), in order.
    post_effects: Vec<String>,
    /// `GameRenderer.spectatedEntityPostEffect` (`checkEntityPostEffect`).
    spectated_post_effect: Option<&'static str>,
    /// `LocalPlayer.experienceDisplayStartTick`: (last progress, seen a change, tick).
    experience_display: (f32, bool, u64),
    boats: Vec<boats::Boat>,
    /// The boats a mob's lead can hang on, for this tick's mob ticks.
    leash_boats: Vec<(u32, horizon_client::mobs::leash::HolderState)>,
    boat_models: boats::BoatModels,
    /// EndCrystalModel's baked layer.
    crystal_model: Option<horizon_client::mobs::Part>,
    /// CushionModel's baked layer.
    cushion_model: Option<horizon_client::mobs::Part>,
    carts: Vec<minecarts::Minecart>,
    /// Songs playing from jukeboxes (tracked sound handles).
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: Vec<lightning::Bolt>,
    maps: maps::Maps,
    /// The ender dragon fight (game/dragon.rs).
    fight: dragon::Fight,
    dragon: Option<dragon::Dragon>,
    crystals: Vec<dragon::Crystal>,
    dragon_fireballs: Vec<dragon::DragonFireball>,
    breath_clouds: Vec<dragon::Breath>,
    dragon_model: Option<horizon_client::mobs::Part>,
    /// Map textures waiting for the next HUD batch to upload them.
    map_patches: Vec<horizon_client::font::Patch>,
    text_displays: std::collections::HashMap<i32, text_displays::RemoteTextDisplay>,
    interactions: std::collections::HashMap<i32, interactions::RemoteInteraction>,
    /// The fence knots a server tracks to this client, by entity id, at their
    /// block (`LeashFenceKnotEntity`).
    remote_knots: HashMap<u32, [i32; 3]>,
    /// The entity ids a hosted server gives the knots its mobs hang on, by
    /// block, while a lead is tied there.
    knot_ids: HashMap<[i32; 3], u32>,
    text_display_vertices: usize,
    /// Player skin downloads, and which skin each atlas skin slot holds
    /// (least recently shown first).
    skin_downloads: RefCell<horizon_client::skins::Downloader>,
    /// `ProfileResolver.Cached`: `Dynamic` profiles (name- or id-only heads
    /// and mannequins) resolve through the services in the background.
    skin_profiles: RefCell<horizon_client::skins::Resolver>,
    skin_slots: horizon_client::skins::SkinSlots,
    /// Atlas trim slots in use: (sprite name, slot), least recently drawn first.
    trim_slots: Vec<(String, usize)>,
    cart_models: minecarts::CartModels,
    /// The boat the player rides.
    riding: Option<u32>,
    /// The vehicle the player's file names, waiting for it to be in the world
    /// (`ServerPlayer.loadAndSpawnParentVehicle`).
    pending_vehicle: Option<[i32; 4]>,
    /// The ender inventory (27 slots) and the ender chest showing it.
    ender_items: Vec<Option<ItemStack>>,
    ender_open: Option<(i32, i32, i32)>,
    /// Block entity data of broken blocks waiting for their dropped item.
    pending: spill::Pending,
    /// Item entities spawned and block entities removed while the block
    /// behaviour ran (spilled contents, popped resources).
    spill: spill::Spill,
    /// The recipe book of the open screen (game/recipe_book.rs).
    recipe_ui: recipe_book::RecipeUi,
    /// The horse inventory (game/mount_menu.rs).
    mount_menu: mount_menu::MountMenu,
    /// The player's fishing hook.
    hook: Option<fishing::Hook>,
    /// The other players' hooks a server tracks to this client.
    remote_hooks: Vec<fishing::RemoteHook>,
    tridents: Vec<trident::ThrownTrident>,
    rockets: Vec<fireworks::Rocket>,
    /// Ominous trial spawners' item spawners.
    ominous_spawners: Vec<ominous_item_spawner::OminousItemSpawner>,
    creaking: creaking::CreakingState,
    /// Monster spawners' display mobs (`game/spawner.rs`).
    spawners: spawner::SpawnerState,
    /// Reused candidate buffers for natural spawning.
    spawn_scratch: natural_spawning::SpawnScratch,
    trident_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    bee_stinger_model: Option<horizon_client::mobs::Part>,
    tnts: Vec<tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    /// The screen of a mod block's menu open on this client (`native_screen`).
    mod_screen: Option<native_hooks::OpenNativeScreen>,
    /// Where item entities are, while the block entities tick (hoppers).
    item_columns: Option<machines::ItemColumns>,
    creative_screen: Creative,
    control: Option<horizon_client::control::Server>,
    /// The Agent Control setting the socket was last opened or closed for (`agent_settings`).
    agent_control_on: bool,
    /// When an agent's last command came.
    agent_last: Option<Instant>,
    frame_stats: horizon_client::control::FrameStats,
    /// This play session's frame and tick times, for opt-in performance statistics.
    perf: horizon_client::telemetry::Session,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>)>,
    /// Recent frame times for the performance overlay.
    frame_graph: horizon_client::perf_overlay::FrameGraph,
    /// Horizon's extras (minimap, auto-reconnect; mods' counters): `game/extras.rs`.
    extras: extras::ExtrasState,
    /// Horizon mods (`game/mods.rs`), none running unless turned on.
    mods: mods::ModsState,
    /// Every item and recipe, for mods (`game/native_recipes.rs`), made once.
    recipe_tables: native_recipes::RecipeTables,
    /// The item panel beside container screens (`game/native_panel.rs`).
    panel: native_panel::Panel,
    /// Horizon's own shader pack, while one is picked (`game/native_shaders.rs`).
    native_shaders: native_shaders::NativeShaders,
    /// The native lifecycle bookkeeping.
    native_lifecycle: native_lifecycle::NativeLifecycle,
    /// Native mods' messages waiting (`game/native_net.rs`).
    native_net: native_net::NativeNet,
    /// Meshed sections waiting for an upload budget (Horizon settings).
    pending_uploads: Vec<((i32, i32, i32), horizon_client::mesher::SectionMesh)>,
    /// This frame's view frustum, for culling entities and block entities.
    view_frustum: Option<horizon_client::gfx::Frustum>,
    entity_culler: Option<horizon_client::entity_culling::Culler>,
    /// `Renderer::pack_shadow_casters` for this frame's entities: (keep what the culler hides,
    /// build the player's model in first person); (false, false) without a shader pack.
    pack_shadow_casters: (bool, bool),
    /// A scripted flight from the control socket (`fly`).
    flight: Option<control::Flight>,
    /// The running agent action from the control socket (game/agent.rs).
    agent: agent::Agent,
    /// Widget commands waiting for the frame after a click (game/ui_control.rs).
    ui_wait: ui_control::UiWait,
    /// What happened, for `events` and `wait-for` (game/events.rs).
    events: events::Events,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((i64, u32, u32), i32)>>,
    /// The last water plane `water_plane` found, kept while none is in reach.
    water_plane_memo: std::cell::Cell<Option<f32>>,
    /// The camera's block and the second `water_plane` last scanned at.
    water_plane_scan: std::cell::Cell<Option<([i32; 3], i64)>>,
    persist: persist::EntityMemory,
    /// Connected to a server: the world comes from the network (game/remote.rs).
    remote: Option<remote::Remote>,
    /// This world is open to the network (game/lan.rs).
    lan: Option<lan::LanHost>,
    /// Single player's server thread, which `remote` is connected to.
    integrated: Option<integrated::Integrated>,
    /// The shapes of the open world's levels, from its dimension types.
    level_shapes: horizon_client::chunk::LevelShapes,
    /// This game is that server thread's (game/integrated.rs).
    serving: Option<integrated::Serving>,
    /// The open world is shared behind an invite code (game/friends.rs).
    friends: Option<friends::Sharing>,
    world_options: Option<lan_screen::WorldOptions>,
    /// Chat Settings opened from RestrictionsScreen: Done returns to World Options.
    options_to_world_options: bool,
    /// HDR's settings opened with their key from the game: Done returns to the game.
    options_to_game: bool,
    command_block_screen: Option<command_blocks::CommandBlockScreen>,
    block_editor: Option<block_editor::BlockEditScreen>,
    /// `StructureTemplateManager.structureRepository`: the templates in memory
    /// by id, a miss included (game/structure_blocks.rs).
    structure_templates: HashMap<String, Option<structure_blocks::TemplateEntry>>,
    /// The game tests read so far, by id (the test instance registry).
    test_defs: std::cell::RefCell<HashMap<String, Option<test_instances::TestDef>>>,
    /// `GameTestBlockHighlightRenderer.markers`: block, text, removal time.
    test_highlights: HashMap<(i32, i32, i32), (String, std::time::Instant)>,
    /// `GameTestTicker.SINGLETON`: the game tests running on this server.
    game_tests: game_tests::GameTests,
    /// KeyboardHandler's debug key state (game/debug_keys.rs), the physical
    /// keys held, and the debug renderers F3+B and F3+G switch on.
    debug_keys: horizon_client::debug_keys::DebugKeys,
    /// The F3 overlay's charts (F3+2, F3+3).
    debug_charts: horizon_client::debug_chart::DebugCharts,
    /// `Minecraft.debugEntries`: what the overlay and the debug renderers
    /// show (`debug-profile.json`, F3+F6).
    debug_entries: horizon_client::debug_entries::DebugEntries,
    /// `DebugOptionsScreen` while it is open (game/debug_options.rs).
    debug_options: Option<debug_options::DebugOptionsUi>,
    /// `DebugEntryMemory`'s readings (game/debug_lines.rs).
    memory_meter: std::cell::RefCell<horizon_client::debug_overlay::MemoryMeter>,
    /// `Minecraft.gpuUtilization`: the last timed frame's GPU time over its
    /// CPU time, in percent.
    gpu_utilization: f64,
    /// The mobs `mob_geometry` drew last frame (`lastEntityRenderStateCount`).
    rendered_mobs: std::cell::Cell<usize>,
    /// `ServerChunkCache.lastSpawnState` of each level: its spawnable chunks
    /// and each `MobCategory`'s count.
    spawn_counts: HashMap<Dim, (i32, [i32; 8])>,
    /// PauseScreen.showPauseMenu (false after F3+Escape).
    pause_menu: bool,
    pause_focus: Option<u8>,
    death_focus: Option<u8>,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    previous_mode: Option<horizon_client::command::args::GameMode>,
    gamemode_switcher: Option<horizon_client::game_mode_switcher::GameModeSwitcher>,
    /// The system clipboard (`KeyboardHandler.clipboardManager`).
    clipboard: horizon_client::clipboard::SystemClipboard,
    /// Select All in the append-only text fields.
    text_selection: horizon_client::clipboard::AppendSelection,
    /// The world was saved for the application ending (`save_for_exit`).
    saved_for_exit: bool,
    pending_save: Option<saving::ClosingSave>,
    /// The world list's icon in progress (`GameRenderer.tryTakeScreenshotIfNeeded`):
    /// `None` without a local world, or once the save has its icon.png.
    world_icon: Option<WorldIcon>,
    /// The block a guest is acting on (game/lan_actions.rs).
    forced_hit: Option<Hit>,
    /// `Minecraft.hitResult` while the clicks of a tick act (game/remote.rs
    /// `remote_handle_input`): picked once, so a creative break does not
    /// move the use that follows it past the broken block.
    tick_hit: Option<Option<Hit>>,
    /// `Player.entityInteractionRange` and `blockInteractionRange`, refreshed
    /// each tick from the attribute (a server's reach modifier, creative's
    /// +2): the pick helpers' `&self` reach.
    entity_reach: f64,
    block_reach: f64,
    /// The part of a right click a guest's packet asks for (game/lan_actions.rs).
    guest_use: Option<GuestUse>,
    /// The last `use_hand` that ended the click did so with `FAIL`, which
    /// does not consume the action (handleUseItemOn's ANY_BLOCK_USE).
    use_failed: bool,
    /// Level events of this tick for the guests, and the guest acting now.
    lan_events: Vec<(i32, [i32; 3], i32, Option<u32>)>,
    /// Jukebox level events 1010 (song name) and 1011 (`None`) for the guests;
    /// the song's network id is only known once the hub is in hand.
    lan_songs: Vec<([i32; 3], Option<String>)>,
    lan_cushion_breaks: Vec<(horizon_world::registry::BlockStateId, [f64; 3])>,
    /// Block events (position, b0, b1, state) and entity events (id, event,
    /// position) for the guests.
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8, [f64; 3])>,
    /// Successful crits (attacking guest, hit entity), queued while the hub is taken.
    lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// Spear stabs that landed (the stabbing guest), queued while the hub is
    /// taken: `broadcastEntityEvent(player, 2)`.
    lan_kinetic_hits: Vec<Option<u32>>,
    /// `LivingEntity.recentKineticEnemies`: when each player (a guest, or the
    /// window's own) last stabbed each entity this use.
    stabbed: HashMap<(Option<u32>, u32), i64>,
    /// The movement a guest's client reported this tick
    /// (`ServerPlayer.getKnownMovement`), for their turn.
    known_movement: Vec3,
    /// `LivingEntity.lastKineticHitFeedbackTime` of the entities that felt a
    /// spear's hit (`spear::OWN` is this window's player), in game time.
    kinetic_feedback: HashMap<i32, i64>,
    /// Global level events for every guest (`ServerLevel.globalLevelEvent`).
    lan_global_events: Vec<(i32, [i32; 3], i32)>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_acting: Option<u32>,
    /// The block the acting player is breaking (`break_block`): its destroy
    /// event, and the ones before it, are the player's own (`Level.levelEvent`
    /// with the player excluded); the events after it are not.
    lan_break_pos: Option<[i32; 3]>,
    /// The name of the guest `lan_acting` acts for (`Player.getName`).
    lan_acting_name: Option<String>,
    /// Their UUID (`Player.getUUID`).
    lan_acting_uuid: Option<u128>,
    /// Their permission level: an op's above 0 (`PlayerList.isOp`).
    lan_acting_permission: Option<u8>,
    /// Command feedback to forward to a guest.
    chat_capture: Option<Vec<horizon_client::text::Text>>,
    /// Packets for the guest acting now, sent once its action returns.
    guest_packets: Vec<horizon_protocol::packets::play::Clientbound>,
    /// The acting guest's cooldowns, as (item, ticks): their packet needs the
    /// hub's item registry, which the guest's turn does not hold.
    guest_cooldowns: Vec<(Arc<str>, i32)>,
    /// The acting guest's own entity events (`ServerPlayer.connection.send`),
    /// sent with their entity id, which the guest's turn does not hold.
    guest_entity_events: Vec<i8>,
    /// The acting guest's equipment breaks (`onEquippedItemBroken`: event and
    /// position), broadcast with their entity id once their turn ends.
    guest_equipment_breaks: Vec<(i8, [f64; 3])>,
    /// `Player.playSound` of the acting guest (event, position, volume,
    /// pitch): `level.playSound(player, ...)` leaves the guest out, whose
    /// own client plays it.
    guest_player_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// `broadcastToAdmins` lines a guest's command queued, sent once it
    /// returns: the line, whether operators get it and whether the console
    /// logs it.
    admin_feedback: Vec<(horizon_client::text::Text, bool, bool)>,
    /// The server console is running a command: whether its feedback goes to
    /// the operators (`DedicatedServer.shouldInformAdmins`,
    /// `broadcast-console-to-ops`).
    console_broadcast: Option<bool>,
    /// A dedicated server's operator, whitelist and ban lists, for `/op`,
    /// `/ban` and friends (`DedicatedPlayerList`).
    server_lists: Option<std::sync::Arc<std::sync::Mutex<dedicated::Lists>>>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    cmds: commands::extra::CommandExtras,
    ui: chat::UiState,
    combat: combat::CombatTracker,
    /// Each guest's own `CombatTracker`, out of `combat` between turns.
    guest_combat: std::collections::HashMap<u32, combat::CombatTracker>,
    mount: mounts::MountState,
    stats: stats::Stats,
    score: scoreboard::ScoreState,
    fns: functions::FnState,
    bosses: bossbars::BossState,
    /// `Hud.bossOverlay`: the bars this window is showing.
    boss_overlay: horizon_client::boss_overlay::BossOverlay,
    exec: commands::execute::ExecState,
    rand_sequences: horizon_client::random_sequences::RandomSequences,
    /// `BrushableBlockEntity`'s brush counts of the blocks being brushed.
    brushing: brush::Brushing,
    border: horizon_client::worldborder::WorldBorder,
    /// `WorldBorderRenderer` and its render state.
    border_renderer: horizon_client::border_render::BorderRenderer,
    border_state: horizon_client::border_render::BorderRenderState,
    /// `Hud.tabList`: the player list's header, footer and heart states.
    tab_list: horizon_client::tab_list::TabList,
    subtitles: subtitle_hud::SubtitleState,
    adv: advancements::AdvState,
    restore_player_state: bool,
    stats_ui: stats_screen::StatsUi,
    adv_ui: advancements_screen::AdvUi,
    /// The Server Links screens' state (game/server_links.rs).
    links_ui: server_links::LinksUi,
    camera_type: perspective::CameraType,
    fangs: fangs::FangState,
    /// The local player as a model (third person, the inventory).
    player_model: Option<Mob>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dim {
    Overworld,
    Nether,
    End,
}
impl Dim {
    fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
        }
    }
    fn from_id(id: &str) -> Self {
        match id {
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::Overworld,
        }
    }
    /// The level's shape in a world of vanilla's dimension types.
    fn shape(self) -> horizon_client::save::LevelShape {
        use horizon_client::save::LevelShape;
        match self {
            Self::Overworld => LevelShape::OVERWORLD,
            Self::Nether => LevelShape::NETHER,
            Self::End => LevelShape::END,
        }
    }
}
/// `Inventory.add` on a player's 36 slots (hotbar first) and off hand,
/// `max` the stack's own maximum: onto equal stacks first
/// (`getSlotWithRemainingSpace`: the selected slot, the off hand, then the
/// hotbar and the main inventory), then into empty slots of the inventory
/// (`getFreeSlot`, never the off hand). What does not fit stays in `stack`;
/// true when some of it is left.
fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    offhand: &mut Option<ItemStack>,
    selected: usize,
    max: u32,
    stack: &mut ItemStack,
) -> bool {
    let order = std::iter::once(selected)
        .chain(std::iter::once(shield::OFFHAND_SLOT))
        .chain((0..36).filter(|&i| i != selected));
    for i in order {
        let slot = if i == shield::OFFHAND_SLOT {
            &mut *offhand
        } else {
            &mut inventory[i]
        };
        if let Some(slot) = slot
            && slot.same_item_same_components(stack)
            && slot.count < max
        {
            let moved = (max - slot.count).min(stack.count);
            slot.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return false;
            }
        }
    }
    for slot in inventory.iter_mut().take(36) {
        if slot.is_none() {
            let moved = stack.count.min(max);
            *slot = Some(ItemStack {
                count: moved,
                ..stack.clone()
            });
            stack.count -= moved;
            if stack.count == 0 {
                return false;
            }
        }
    }
    true
}

fn facing_offset(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
/// How far along `dir` (within `reach`) a ray from `eye` enters `b`.
fn ray_box(eye: [f64; 3], dir: [f64; 3], reach: f64, b: Aabb) -> Option<f64> {
    let (min, max) = (b.min_position(), b.max_position());
    let (lo, hi) = ([min.x, min.y, min.z], [max.x, max.y, max.z]);
    let (mut t0, mut t1) = (0.0f64, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
/// `Camera.setRotation` (`rotationYXZ(π - yaw, -pitch, 0)`) for the player's
/// view, turned round in third-person front view (`Camera.setup`).
fn camera_rotation(camera_type: perspective::CameraType, yaw: f32, pitch: f32) -> glam::Quat {
    let (view_yaw, view_pitch) = if camera_type == perspective::CameraType::ThirdPersonFront {
        (yaw + 180.0, -pitch)
    } else {
        (yaw, pitch)
    };
    glam::Quat::from_euler(
        glam::EulerRot::YXZ,
        std::f32::consts::PI - view_yaw.to_radians(),
        -view_pitch.to_radians(),
        0.0,
    )
}
impl Game<'_> {
    fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ])
    }
    /// Where targeting starts and points: the eyes along the view. Every
    /// crosshair target (blocks, fluids, entities, mod raycasts) is found
    /// along this ray, so a tracked controller can stand in for it.
    fn aim(&self) -> Option<([f64; 3], [f64; 3])> {
        // Minecraft.pick: from the camera entity's eyes along its view
        // (LivingEntity.getViewYRot: the head's yaw).
        if let Some(m) = self.camera_mob() {
            return Some((
                [m.position.x, m.eye_y(), m.position.z],
                view_vector(m.head_yaw, m.head_pitch),
            ));
        }
        Some((self.eye_position()?, view_vector(self.yaw, self.pitch)))
    }
    /// `Player.blockInteractionRange`: the block pick's and mining's reach.
    fn reach(&self) -> f64 {
        self.block_reach
    }
    /// Outline shapes that depend on more than the block state, as one box
    /// in block space: `LightBlock` and `ScaffoldingBlock.getShape` are a full
    /// block while the main hand holds the block's own item
    /// (`CollisionContext.isHoldingItem`), and `ShulkerBoxBlock.getShape`
    /// grows with the lid (`ShulkerBoxBlockEntity.getBoundingBox`).
    fn context_outline(&self, pos: [i32; 3], state: BlockStateId) -> Option<[f64; 6]> {
        let name = &self.res.facts().get(state).name;
        if matches!(&**name, "minecraft:light" | "minecraft:scaffolding") {
            return self
                .held()
                .is_some_and(|s| *s.item == **name)
                .then_some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        }
        if !name.ends_with("shulker_box") {
            return None;
        }
        let progress = self.be_anims.shulkers.get(&pos).map_or(0.0, |s| s.progress);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned());
        let d = facing_offset(facing.as_deref().unwrap_or("up"));
        let mut b = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for a in 0..3 {
            match d[a] {
                1 => b[a + 3] += 0.5 * f64::from(progress),
                -1 => b[a] -= 0.5 * f64::from(progress),
                _ => {}
            }
        }
        Some(b)
    }
    /// A block's outline boxes in world space (`getShape` plus the random offset).
    fn outline_boxes(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if let Some(b) = self.context_outline(pos, state) {
            return vec![std::array::from_fn(|i| f64::from(pos[i % 3]) + b[i])];
        }
        let f = self.res.facts().get(state);
        let o = if f.outline_offset {
            self.pack
                .context
                .tables
                .offset(state, pos[0], pos[2])
                .unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        let base = [
            f64::from(pos[0]) + o[0],
            f64::from(pos[1]) + o[1],
            f64::from(pos[2]) + o[2],
        ];
        f.outline
            .iter()
            .map(|b| std::array::from_fn(|i| base[i % 3] + b[i]))
            .collect()
    }
    /// Voxel traversal against non-air, non-fluid blocks, clipped against
    /// each block's outline shape (`BlockGetter.clip` with `ClipContext.Block.OUTLINE`).
    fn pick(&self) -> Option<Hit> {
        // A guest of a hosted world acts on the block their client targeted.
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        if let Some(hit) = self.tick_hit {
            return hit;
        }
        let (origin, dir) = self.aim()?;
        // GameRenderer.pick: the block ray runs out to the larger of the two
        // interaction ranges; filterHitResult then drops a hit past the
        // block's.
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
            .filter(|hit| {
                let distance: [f64; 3] = std::array::from_fn(|i| hit.point[i] - origin[i]);
                distance.iter().map(|v| v * v).sum::<f64>() <= self.block_reach.powi(2)
            })
    }
    /// The same traversal without `filterHitResult`: the block hit that
    /// bounds the entity ray in `GameRenderer.pick`, out to the larger
    /// interaction range — a wall between the two still hides the entity.
    fn pick_unfiltered(&self) -> Option<Hit> {
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.aim()?;
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
    }
    /// The first block outline hit within `reach` along the unit `dir`.
    fn clip_ray(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Hit> {
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| {
            if d == 0.0 {
                f64::INFINITY
            } else {
                (1.0 / d).abs()
            }
        });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 {
                f64::from(pos[i]) + 1.0
            } else {
                f64::from(pos[i])
            };
            t_max[i] = if dir[i] == 0.0 {
                f64::INFINITY
            } else {
                (boundary - origin[i]) / dir[i]
            };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts().get(state);
            if !self.res.facts().is_air(state)
                && (f.fluid == Fluid::None
                    || f.waterlogged
                    || &*f.name == "minecraft:kelp"
                    || f.render_shape == horizon_client::blocks::RenderShape::Model)
                && !matches!(
                    &*f.name,
                    "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
                )
            {
                // The nearest entry into one of the outline's boxes (slab method).
                let mut best: Option<(f64, [i32; 3])> = None;
                for b in self.outline_boxes(pos) {
                    let (mut t0, mut t1, mut axis_in, mut sign) = (0.0f64, f64::INFINITY, None, 0);
                    let mut miss = false;
                    for a in 0..3 {
                        if dir[a] == 0.0 {
                            miss |= origin[a] < b[a] || origin[a] > b[a + 3];
                            continue;
                        }
                        let (lo, hi) =
                            ((b[a] - origin[a]) / dir[a], (b[a + 3] - origin[a]) / dir[a]);
                        let (near, far) = if lo < hi { (lo, hi) } else { (hi, lo) };
                        if near > t0 {
                            t0 = near;
                            axis_in = Some(a);
                            sign = if dir[a] > 0.0 { -1 } else { 1 };
                        }
                        t1 = t1.min(far);
                    }
                    if miss || t0 > t1 || t0 > reach {
                        continue;
                    }
                    let mut normal = face;
                    if let Some(a) = axis_in {
                        normal = [0; 3];
                        normal[a] = sign;
                    }
                    if best.is_none_or(|(bt, _)| t0 < bt) {
                        best = Some((t0, normal));
                    }
                }
                if let Some((t, face)) = best {
                    return Some(Hit {
                        pos,
                        face,
                        point: [
                            origin[0] + dir[0] * t,
                            origin[1] + dir[1] * t,
                            origin[2] + dir[2] * t,
                        ],
                    });
                }
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
    /// The stack in the hand that acts (`getItemInHand(hand)`).
    fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }
    fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }
    /// The slot of the hand that acts: the selected one, or `OFFHAND_SLOT`.
    /// `LivingEntity.canGlideUsing`: a glider in the chest slot that will
    /// not break with its next point of wear.
    fn glider_worn(&self) -> bool {
        self.armor[1].as_ref().is_some_and(|s| {
            self.res
                .items()
                .get(&s.item)
                .is_some_and(|i| i.glider && (i.max_damage == 0 || s.damage + 1 < i.max_damage))
        })
    }
    /// `LivingEntity.canFreeze`'s armor half: any worn piece in
    /// `freeze_immune_wearables`.
    pub(crate) fn wears_freeze_immune(&self) -> bool {
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        self.armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)))
    }
    /// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners
    /// (aiStep, unless a spectator or a passenger): a corner inside a
    /// suffocating block sets a 0.1 speed towards the nearest side that is
    /// free, on that side's axis; a later corner overwrites an earlier one.
    /// Suffocating is a full collision cube that is also a full render cube,
    /// as `in_wall` has it.
    fn closest_space_nudge(&self) -> [Option<f64>; 2] {
        match self.player.as_ref() {
            Some(p) if !p.spectator && self.riding.is_none() => {
                closest_space_nudge(p, &self.world, self.res.facts())
            }
            _ => [None; 2],
        }
    }
    /// What the window decides for its player's body every tick, in frame and
    /// on a server alike: the look (Player.travel steers sprint-swimming by
    /// the pitch), Auto Jump (LocalPlayer.sendPosition), the slowdown of an
    /// item in use (LocalPlayer.modifyInput), the helmet's Respiration
    /// (decreaseAirSupply), the wings, leather boots on powder snow
    /// (PowderSnowBlock.canEntityWalkOnPowderSnow), freeze-immune wear
    /// (LivingEntity.canFreeze), the dimension's FAST_LAVA, the sleeping
    /// pose and blindness (Player.isMobilityRestricted, which stops a sprint).
    pub(crate) fn sync_player_body(&mut self) {
        let glider = self.glider_worn();
        let sleeping = self.sleeping.is_some();
        let item_in_use = self
            .using
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    self.offhand.as_ref()
                } else {
                    self.inventory.get(slot)?.as_ref()
                }
            })
            .and_then(|s| self.items().get(&s.item));
        let item_use_speed = item_in_use.map(|i| i.use_speed);
        let item_use_slows_sprint = item_in_use.is_some_and(|i| !i.can_sprint);
        let respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        let leather_boots = self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots");
        let freeze_immune = self.wears_freeze_immune();
        let burn_immune = self.wears_burn_from_stepping_immunity();
        let nudge = self.closest_space_nudge();
        let blind = self.potions.effects.get("blindness").is_some();
        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.nudge = nudge;
        player.mobility_restricted = blind;
        player.yaw = self.yaw;
        player.pitch = self.pitch;
        player.auto_jump_enabled = self.settings.auto_jump;
        player.item_use_speed = item_use_speed;
        player.item_use_slows_sprint = item_use_slows_sprint;
        player.respiration = respiration;
        player.glider = glider;
        let _ = player.set_sleeping(sleeping);
        player.leather_boots = leather_boots;
        player.freeze_immune = freeze_immune;
        player.burn_from_stepping_immune = burn_immune;
        player.fluids.fast_lava = self.dim == Dim::Nether;
    }
    fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }
    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    fn used_hand(&self) -> Option<Hand> {
        self.using.map(|(slot, _)| {
            if slot == shield::OFFHAND_SLOT {
                Hand::Off
            } else {
                Hand::Main
            }
        })
    }
    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }
    /// Inventory.add: returns the stack that did not fit.
    fn give(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let tutorial_item = matches!(
            self.tutorial.step,
            tutorial::Step::FindTree | tutorial::Step::PunchTree | tutorial::Step::CraftPlanks
        )
        .then(|| stack.clone());
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
        let before = stack.count;
        let left = inventory_add(
            &mut self.inventory,
            &mut self.offhand,
            self.selected,
            max,
            &mut stack,
        );
        if stack.count < before {
            self.tutorial_get_item(tutorial_item.as_ref());
        }
        left.then_some(stack)
    }
    /// The block's first up face (or any face) and its fixed tint, for block
    /// models drawn as entities (falling blocks, carried blocks).
    fn block_face_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let pack = &self.pack;
        let models = pack
            .render
            .model_catalog
            .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = pack.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts().get(state).name;
            if n.contains("leaves") || n.contains("vine") {
                0x48B518
            } else {
                0x91BD59
            }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    /// `TerrainParticle` at `pos`: the model's particle sprite and tint source
    /// 0's `colorAsTerrainParticle` (white when the block has none).
    /// `HoneyBlock.showSlideParticles` (entity event 53): five honey block
    /// particles at the entity.
    fn honey_slide_particles(&mut self, at: [f64; 3]) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let Some(honey) = self
            .pack
            .assets
            .registry
            .default_state("minecraft:honey_block")
        else {
            return;
        };
        let block = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        if let Some((sprite, tint)) = self.particle_sprite(honey, block) {
            for _ in 0..5 {
                self.particles.add(
                    &ParticleOptions::block(ParticleType::Block, sprite.clone(), tint),
                    at,
                    [0.0; 3],
                );
            }
        }
    }
    fn particle_sprite(&self, state: BlockStateId, pos: [i32; 3]) -> Option<(String, u32)> {
        let sprite = Self::block_particle_sprite(&self.pack, state)?;
        let chunks = &self.world.chunks;
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&horizon_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        let tint = self
            .pack
            .context
            .tables
            .terrain_particle_tint(
                state,
                pos,
                &region,
                world_biomes!(self),
                self.streaming.biome_zoom,
                self.streaming.biome_blend,
            )
            .unwrap_or(0xFFFFFF);
        Some((sprite, tint))
    }
    /// `BlockStateModelSet.getParticleMaterial`: the sprite of the model's
    /// `particle` texture, else its first face (entity-rendered blocks may
    /// have a particle texture without faces, and faces without one).
    fn block_particle_sprite(pack: &PackResources, state: BlockStateId) -> Option<String> {
        let catalog = &pack.render.model_catalog;
        let layer = catalog.particle(state).or_else(|| {
            let models = catalog
                .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
                .ok()?;
            models
                .iter()
                .flat_map(|(_, m)| m.faces.iter())
                .next()
                .map(|f| f.layer)
        })?;
        pack.layer_names.get(&layer).cloned()
    }
    fn particle_shape(&self, pos: [i32; 3], state: BlockStateId) -> Vec<[f64; 6]> {
        let f = self.res.facts().get(state);
        let offset = if f.outline_offset {
            self.pack
                .context
                .tables
                .offset(state, pos[0], pos[2])
                .unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        f.outline
            .iter()
            .map(|b| std::array::from_fn(|i| b[i] + offset[i % 3]))
            .collect()
    }
    /// ClientLevel.addDestroyBlockEffect: a burst per outline box.
    fn destroy_particles(&mut self, pos: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        horizon_client::particles::destroy_positions(pos, &boxes, |p, v| {
            let particle = Particle::terrain(self.particles.random(), p, v, sprite.clone(), tint);
            self.particles.push(particle);
        });
    }
    /// ClientLevel.addBreakingParticles: one crumb on the face being mined.
    fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        // Keep client effects separate from the authoritative world RNG.
        let r = self.particles.random();
        let Some(p) = horizon_client::particles::breaking_position(r, pos, face, &boxes) else {
            return;
        };
        let mut particle = Particle::terrain(r, p, [0.0; 3], sprite, tint);
        particle.set_power(0.2);
        particle.scale(0.6);
        self.particles.push(particle);
    }
    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction.
    fn drop_inventory(&mut self, at: [f64; 3]) {
        if self.ui.rules.bool("keep_inventory") {
            return;
        }
        let mut stacks: Vec<ItemStack> =
            self.inventory.iter_mut().filter_map(Option::take).collect();
        stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
        stacks.extend(self.offhand.take());
        stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
        self.apply_equipment();
        for stack in stacks {
            let r = &mut self.world.random;
            let strength = f64::from(r.next_float()) * 0.5;
            let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
            let v = [-angle.sin() * strength, 0.2, angle.cos() * strength];
            self.spawn_item(stack, at, v, 40);
        }
    }
    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.dropped.remove(i);
        self.native_entities_left("minecraft:item", &[e.id]);
        e
    }
    /// Experience orb `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_orb(&mut self, i: usize) -> ExperienceOrb {
        let o = self.orbs.remove(i);
        self.native_entities_left("minecraft:experience_orb", &[o.id]);
        o
    }
    /// The item entity `spawn_item` just added, unless a native mod kept it
    /// out of the level (`native_entity_join_level`).
    pub(super) fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        let id = self.next_entity;
        self.dropped.last_mut().filter(|d| d.id == id)
    }
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) {
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        // ItemEntity.bobOffs comes from the entity's own random, not the level's.
        let bob = self.particles.random().next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            health: 5,
            fire_ticks: 0,
            thrower: None,
            target: None,
            id: self.next_entity,
            stack,
            position: p,
            previous: p,
            velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
            on_ground: false,
            age: 0,
            pickup_delay,
            bob_offset: bob,
            ticks: 0,
            needs_sync: false,
        });
        // ServerLevel.addFreshEntity.
        if !self.native_entity_joins(self.next_entity, "minecraft:item", position, false) {
            self.dropped.pop();
        }
    }
    /// `Block.getDrops` for a block a player breaks: the block's loot table
    /// with the player as `this`, drawing from its server random sequence
    /// (`LootContext.Builder.create`), else the level random.
    fn block_loot_at(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        let registry = self.pack.assets.registry;
        let dynamic = self.pending.dynamic(pos);
        let mut drops = match world_items!(self).block_loot_sequence(registry, state) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.level.seed);
                self.res
                    .items()
                    .block_drops_by_player(registry, state, held, &dynamic, random)
            }
            None => world_items!(self).block_drops_by_player(
                registry,
                state,
                held,
                &dynamic,
                &mut self.world.random,
            ),
        };
        let player = self.native_acting_id();
        self.native_block_drops(pos, state, Some(player), held, None, &mut drops);
        drops
    }
    /// Block.popResource
    fn pop_resource(&mut self, pos: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        // Block.popResource: Mth.nextDouble offsets from the level random; the
        // motion is `new ItemEntity(..)`'s own, from the item's random.
        let p = spill::pop_position(&mut self.world.random, pos);
        let v = dispenser::new_item_motion();
        self.spawn_item(stack, p, v, 10);
    }
    /// Player.drop: thrown from the eyes along the view direction.
    fn drop_held(&mut self, all: bool) {
        if self.eye_position().is_none() {
            return;
        }
        let spectator = self.spectator();
        let Some(thrown) = horizon_client::items::take_held_drop(
            &mut self.inventory[self.selected],
            all,
            spectator,
        ) else {
            return;
        };
        self.player_drop(thrown);
    }
    /// `Player.drop`: the statistics and a thrown item entity in front of the player.
    pub(super) fn player_drop(&mut self, thrown: ItemStack) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        // LivingEntity.drop: every drop swings the main hand, a menu's
        // THROW and a creative tab's drop included.
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        // Player.drop: the drop statistics.
        self.stats.add("dropped", &thrown.item, thrown.count as i32);
        self.stats.custom("drop", 1);
        let player = self.native_acting_id();
        self.native_item_dropped(player, &thrown.item, thrown.count);
        self.throw_from_eyes(thrown, eye, true);
    }
    /// `Player.createItemStackToDrop`: an item entity thrown from the eyes
    /// along the view direction. `thrown_from_hand` (`drop(stack, true)`, the
    /// drop key) names the player as the thrower; `drop(stack, false)`, for
    /// what no longer fits in the inventory, does not.
    fn throw_from_eyes(&mut self, thrown: ItemStack, eye: [f64; 3], thrown_from_hand: bool) {
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        let r = &mut self.world.random;
        let spread = f64::from(r.next_float()) * std::f64::consts::TAU;
        let strength = 0.02 * f64::from(r.next_float());
        let v = [
            -yaw.sin() * pitch.cos() * 0.3 + spread.cos() * strength,
            -pitch.sin() * 0.3 + 0.1 + f64::from(r.next_float() - r.next_float()) * 0.1,
            yaw.cos() * pitch.cos() * 0.3 + spread.sin() * strength,
        ];
        self.spawn_item(thrown, [eye[0], eye[1] - 0.3, eye[2]], v, 40);
        // A native mod can keep the item out of the level (native_entity_join_level).
        if thrown_from_hand {
            let uuid = self.acting_player_uuid();
            if let Some(item) = self.just_spawned_item() {
                item.thrower = Some(uuid);
            }
        }
    }
    /// The UUID of the player acting: the guest whose turn it is, else the host.
    pub(super) fn acting_player_uuid(&mut self) -> [i32; 4] {
        match self.lan_acting_uuid {
            Some(guest) => horizon_client::lan::playerdata::uuid_ints(guest),
            None => self.level.ensure_player_uuid(),
        }
    }
    fn tick_items(&mut self) {
        let mut items = std::mem::take(&mut self.dropped);
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut removed = vec![false; items.len()];
        let mut cells = horizon_client::entities::ItemCells::new(&items);
        let catalog = world_items!(self);
        let max_stack =
            |stack: &ItemStack| catalog.get(&stack.item).map_or(64, |i| i.max_stack).max(1);
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        for i in 0..items.len() {
            if removed[i] {
                continue;
            }
            let item = &mut items[i];
            // `addFreshEntity`: an entity added this tick ticks from the next
            // one (`EntityTickList`), so a drop's `Age` stays 0 for the tick
            // function that looks for it.
            if merging && item.id > self.ticked_entities_through {
                continue;
            }
            if !ticking(
                item.position.x.floor() as i32,
                item.position.z.floor() as i32,
            ) {
                continue;
            }
            // ItemEntity.tick's noPhysics (servers only): out of a block
            // placed on it, towards the closest open side (#1252).
            let no_physics = merging && item.inside_block(&self.world, self.collision);
            if no_physics {
                let speed = self.world.random.next_float() * 0.2 + 0.1;
                item.move_towards_closest_space(&self.world, self.res.facts(), speed);
            }
            let alive = item.tick(
                &self.world,
                self.pack.assets.registry,
                self.res.facts(),
                self.collision,
                no_physics,
                &self.res.motion().fluids,
            );
            cells.moved(i, item.previous, item.position);
            // ItemEntity.tick: merge every 2 ticks while moving between
            // blocks, every 40 otherwise, before the lifetime check.
            let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
            let rate = if block(item.previous) == block(item.position) {
                40
            } else {
                2
            };
            if merging
                && (alive || item.age == 6000 && item.position.y > -128.0)
                && item.ticks.is_multiple_of(rate)
            {
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    i,
                    &max_stack,
                );
            }
            let item = &mut items[i];
            if !alive || removed[i] {
                removed[i] = true;
                cells.remove(i, item.position);
                continue;
            }
            let environment = item.environment_tick(
                &self.world,
                self.res.facts(),
                self.pack.assets.registry,
                world_items!(self),
            );
            // shortcut: age is the fresh-drop clock; use entity ticks for saved/frozen-age phase parity.
            if environment.should_play_lava_hurt_sound(item.age) {
                let pitch = 2.0 + self.world.random.next_float() * 0.4;
                self.audio.play(
                    "minecraft:entity.generic.burn",
                    Some([item.position.x, item.position.y, item.position.z]),
                    0.4,
                    pitch,
                );
            }
            if !environment.alive {
                removed[i] = true;
                cells.remove(i, item.position);
            }
        }
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap_or(false));
        let Some(p) = self.player.as_ref() else {
            self.dropped = items;
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.dropped = items;
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.level.ensure_player_uuid();
        let mut kept = Vec::with_capacity(items.len());
        for mut item in items {
            let b = item.bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            // ItemEntity.playerTouch: no pickup delay, and the target (if any) is this player.
            if touching && item.pickup_delay == 0 && item.target.is_none_or(|t| t == own) {
                if self.native_pickup_refused(&item.stack) {
                    item.pickup_delay = 10;
                    kept.push(item);
                    continue;
                }
                let before = item.stack.count;
                let kind = item.stack.item.clone();
                let thrower = item.thrower;
                match self.give(item.stack.clone()) {
                    None => {
                        self.stats.add("picked_up", &kind, before as i32);
                        if let Some(thrower) = thrower {
                            self.trigger_thrown_picked_up_by_player(&item.stack, thrower);
                        }
                        self.add_pickup(
                            pickups::Taken::Item(Box::new(item)),
                            pickups::Collector::Player,
                        );
                        let pitch = ((self.world.random.next_float()
                            - self.world.random.next_float())
                            * 0.7
                            + 1.0)
                            * 2.0;
                        self.audio
                            .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        continue;
                    }
                    Some(rest) => {
                        if rest.count < before {
                            self.stats
                                .add("picked_up", &kind, (before - rest.count) as i32);
                            if let Some(thrower) = thrower {
                                self.trigger_thrown_picked_up_by_player(&rest, thrower);
                            }
                            self.add_pickup(
                                pickups::Taken::Item(Box::new(item.clone())),
                                pickups::Collector::Player,
                            );
                            // handleTakeItemEntity: the random pitch of a full pickup.
                            let pitch = ((self.world.random.next_float()
                                - self.world.random.next_float())
                                * 0.7
                                + 1.0)
                                * 2.0;
                            self.audio
                                .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        }
                        item.stack = rest;
                    }
                }
            }
            kept.push(item);
        }
        self.dropped = kept;
    }
    /// Run vanilla block behaviour against the world through `LevelAccess`.
    fn with_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        let darken = self.sky_darken();
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Spectators ignore block triggers (Entity.isIgnoringBlockTriggers).
        let spectator = self.spectator();
        let player_box = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            (
                [p.position.x - w, p.position.y, p.position.z - w],
                [
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ],
            )
        });
        let player_collision = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let main_hand = self.inventory.get(self.selected).and_then(Option::as_ref);
            PlayerCollision {
                y: p.position.y,
                descending: p.velocity.y < 0.0,
                fall_distance: p.fall_distance,
                main_hand_item: main_hand.map_or_else(
                    || Arc::from("minecraft:air"),
                    |stack| Arc::clone(&stack.item),
                ),
                main_hand_count: main_hand.map_or(0, |stack| stack.count as i32),
                leather_boots: self.armor[3]
                    .as_ref()
                    .is_some_and(|stack| stack.item.as_ref() == "minecraft:leather_boots"),
            }
        });
        let cart_boxes = self
            .carts
            .iter()
            .map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            })
            .collect();
        let arrow_boxes = self
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.arrow.is_some())
            .map(|p| p.position)
            .chain(self.tridents.iter().map(|t| t.position))
            .map(|at| {
                let b = horizon_client::arrows::arrow_bounds(at);
                (b.min_position().components(), b.max_position().components())
            })
            .collect();
        let guest_boxes = self.lan_block_trigger_boxes();
        let dimension = match self.dim {
            Dim::Overworld => LevelDimension::Overworld,
            Dim::Nether => LevelDimension::Nether,
            Dim::End => LevelDimension::End,
        };
        let rules = self.level_rules();
        let fire_players: Vec<[f64; 3]> = self
            .player
            .iter()
            .filter(|_| !spectator)
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .chain(self.lan.iter().flat_map(|h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| h.hub.here(g) && !g.spectator())
                    .map(|g| g.position)
            }))
            .collect();
        let blockers = Blockers {
            mobs: &self.mobs,
            armor_stands: &self.armor_stands,
            boats: &self.boats,
            crystals: &self.crystals,
            tnts: &self.tnts,
            falling: &self.falling,
        };
        let mut level = GameLevel {
            cart_boxes,
            item_frames: &self.item_frames,
            carts: &self.carts,
            arrow_boxes,
            blockers,
            dropped: &self.dropped,
            orbs: &self.orbs,
            projectiles: &self.creatures.projectiles,
            world: &mut self.world,
            facts: self.res.facts(),
            collision: self.collision,
            events: &mut self.level_events,
            dimension,
            darken,
            environment: match dimension {
                LevelDimension::Overworld => self.res.environment(),
                LevelDimension::Nether => self.res.nether_environment(),
                LevelDimension::End => self.res.end_environment(),
            },
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            player_box,
            guest_boxes,
            player_collision,
            biomes: world_biomes!(self),
            seed: self.level.seed,
            behaviours: self.res.behaviours(),
            items: world_items!(self),
            recipes: world_recipes!(self),
            // A client rolls no loot (LootTable.getRandomItems needs a
            // ServerLevel): it doesn't load the tables on its game thread.
            loot: if self.remote.is_some() {
                None
            } else {
                horizon_client::structure_blocks::loot_data(
                    &self.options.resources,
                    &pack_files(&self.world_tables),
                )
            },
            registry: self.pack.assets.registry,
            rules,
            difficulty: i32::from(self.ui.difficulty),
            fire_players,
            rand_sequences: &mut self.rand_sequences,
            pending: &mut self.pending,
            spill: &mut self.spill,
            tickets: self.tickets.levels(),
            random_tick_sections: &mut self.random_tick_sections,
            lid_openers: &self.lid_openers,
            client_side: self.remote.is_some(),
            native_authoritative: self.serving.is_some() || self.options.dedicated.is_some(),
            debug: self.world_debug,
            native_sounds: &mut self.creatures.lan_sounds,
            native_particles: &mut self.creatures.lan_particles,
        };
        let result = f(self.res.behaviours(), &mut level, &mut self.block_ticks);
        self.flush_spill();
        if self.lan.is_some() {
            self.flush_lan_particles();
            self.flush_native_sounds();
        }
        result
    }
    /// Level.setBlock(pos, state, UPDATE_ALL) with neighbour/shape updates.
    fn level_set(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.level_set_with_flags(pos, state, level_flags::UPDATE_ALL)
    }
    /// Level.setBlock with the caller's update flags.
    fn level_set_with_flags(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                flags,
            );
        });
        self.process_level_events()
    }
    fn attack_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let note = self.with_level(|b, level, ticks| {
            b.attack_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        });
        // NoteBlock.attack awards the statistic.
        if note {
            self.stats.custom("play_noteblock", 1);
        }
        self.process_level_events()
    }
    /// `AbstractCandleBlock.getParticleOffsets`, in sixteenths of a block:
    /// `CandleBlock.PARTICLE_OFFSETS` by its `candles` (one to four wicks),
    /// then `CandleCakeBlock.PARTICLE_OFFSETS`, whose one wick sits a block up.
    fn candle_offsets(name: &str, candles: Option<usize>) -> &'static [[u8; 3]] {
        const OFFSETS: [&[[u8; 3]]; 5] = [
            &[[8, 8, 8]],
            &[[6, 7, 8], [10, 8, 7]],
            &[[8, 5, 10], [6, 7, 8], [9, 8, 7]],
            &[[7, 5, 9], [10, 7, 9], [6, 7, 6], [9, 8, 6]],
            &[[8, 16, 8]],
        ];
        if name.ends_with("candle_cake") {
            OFFSETS[4]
        } else {
            OFFSETS[candles.unwrap_or(1).clamp(1, 4) - 1]
        }
    }
    /// `AbstractCandleBlock.extinguish`: the smoke from each wick, at the
    /// block's `getParticleOffsets` (sixteenths of a block), rising 0.1.
    fn candle_smoke(&mut self, pos: [i32; 3], state: BlockStateId) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let candles = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("candles")?.parse::<usize>().ok());
        let offsets = Self::candle_offsets(&self.res.facts().get(state).name, candles);
        for o in offsets {
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Smoke),
                [
                    f64::from(pos[0]) + f64::from(o[0]) / 16.0,
                    f64::from(pos[1]) + f64::from(o[1]) / 16.0,
                    f64::from(pos[2]) + f64::from(o[2]) / 16.0,
                ],
                [0.0, 0.1, 0.0],
            );
        }
    }
    /// `LeverBlock.makeParticle` at full scale: a powered lever puffs one red
    /// dust on the clicking client (`LeverBlock.useWithoutItem`).
    fn lever_puff(&mut self, pos: [i32; 3]) {
        // The server's side of a guest's pull draws nothing.
        if self.lan_acting.is_some() {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(registry_state) = self.pack.assets.registry.state(state) else {
            return;
        };
        let prop = |key: &str| registry_state.properties.get(key).map(String::as_str);
        if prop("powered") != Some("true") {
            return;
        }
        // FACING is horizontal; getConnectedDirection is the face's direction.
        let facing = match prop("facing") {
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => return,
        };
        let connected = match prop("face") {
            Some("ceiling") => [0, -1, 0],
            Some("floor") => [0, 1, 0],
            _ => facing,
        };
        let at = |base: i32, i: usize| {
            f64::from(base) + 0.5 + 0.1 * f64::from(-facing[i]) + 0.2 * f64::from(-connected[i])
        };
        let p = [at(pos[0], 0), at(pos[1], 1), at(pos[2], 2)];
        self.particles.add(
            &horizon_client::particles::ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
            p,
            [0.0; 3],
        );
    }
    /// Track which chunks are ticking (Ticks needs chunk load/unload).
    fn sync_tick_chunks(&mut self) {
        if !self.tick_chunks.synchronize(&self.world.chunks) {
            return;
        }
        // Move only the added keys out so registration can borrow all of Game.
        let added = self.tick_chunks.take_added();
        for &key in &added {
            self.register_chunk_ticks(key);
        }
        for &key in self.tick_chunks.removed() {
            self.random_tick_sections.remove(&key);
            self.portals.sections.remove(&key);
            self.block_ticks
                .remove_chunk(horizon_core::position::ChunkPos::new(key.0, key.1));
            // Its moving pistons were saved with it and load back with it.
            self.block_ticks
                .moving_pistons
                .retain(|(p, _)| (p.x >> 4, p.z >> 4) != key);
        }
        self.tick_chunks.recycle_added(added);
    }
    /// `DistanceManager`'s tickets of the level in place, from every player in
    /// it (the local player, LAN guests, a server's players; a spectator the
    /// rule skips holds none), its forced chunks and its portal tickets. A game
    /// with neither a server nor a player (the headless fixtures) ticks
    /// everywhere.
    fn update_chunk_tickets(&mut self) {
        let rule = self.ui.rules.bool("spectators_generate_chunks");
        let mut players: Vec<((i32, i32), bool)> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g))
                .map(|g| (g.chunk(), g.spectator()))
                .collect()
        });
        if let Some(p) = &self.player {
            players.push((
                (
                    (p.position.x.floor() as i32) >> 4,
                    (p.position.z.floor() as i32) >> 4,
                ),
                self.spectator(),
            ));
        }
        // A spectator the rule skips is still a player: a game that has one
        // must not take the headless fixtures' tick-everywhere case below.
        let any_player = !players.is_empty();
        let sources = (!self.options.execution_role.is_client() || any_player).then(|| {
            horizon_client::ticket_levels::Sources {
                players: players
                    .into_iter()
                    .filter(|&(_, spectator)| game_mode::loads_chunks(rule, spectator))
                    .map(|(chunk, _)| chunk)
                    .collect(),
                forced: self.streaming.forced.iter().copied().collect(),
                // Filled from the `PORTAL` tickets by `update`.
                portal: Vec::new(),
                view_distance: self.settings.render_distance.clamp(2, 32),
                simulation_distance: self.settings.simulation_distance.max(2),
            }
        });
        self.tickets.update(sources);
    }
    /// `ServerLevel.startTickingChunk`: a loaded chunk's saved ticks are
    /// scheduled from the game time at which it becomes a ticking chunk
    /// (block ticking, with its neighbours loaded). Chunks become ticking
    /// between ticks in vanilla: before this tick's clock advanced.
    fn start_ticking_chunks(&mut self) {
        let levels = self.tickets.levels();
        let world = &self.world;
        let started = self
            .tick_chunks
            .start_ticking(|(x, z)| levels.loaded_ticking(x, z) && ticking_chunk(world, x, z));
        let time = self.world.game_time - 1;
        for (x, z) in started {
            let pos = horizon_core::position::ChunkPos::new(x, z);
            self.block_ticks.blocks.unpack_chunk(pos, time);
            self.block_ticks.fluids.unpack_chunk(pos, time);
        }
    }
    /// ServerLevel.tick block phases: scheduled ticks, random ticks in the
    /// chunks the tickets simulate, block events and moving pistons.
    fn tick_blocks(&mut self) -> Result<()> {
        // ServerChunkCache.tick: the tickets that time out.
        self.tickets.tick_portal();
        self.sync_tick_chunks();
        self.start_ticking_chunks();
        let levels = self.tickets.levels();
        let bounds = levels.random_tick_bounds();
        // Keep the world map's chunk order, including empty chunks
        // (tick_chunk_random still draws for precipitation in those).
        let ticking: Vec<_> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| {
                if let Some((min_x, min_z, max_x, max_z)) = bounds
                    && (x < min_x || x > max_x || z < min_z || z > max_z)
                {
                    return false;
                }
                levels.random_ticking(x, z) && ticking_chunk(&self.world, x, z)
            })
            .collect();
        // Entity.checkInsideBlocks for the plates, tripwires, detector rails and
        // buttons under the players (the host and the LAN guests), the minecarts
        // and the arrows (the mobs and other entities are added in `with_level`
        // below).
        let mut inside = Vec::new();
        let guests = self.lan_block_trigger_boxes();
        let boxes = self
            .player
            .iter()
            .filter(|_| !self.spectator())
            .map(|p| {
                let w = f64::from(p.dimensions.width) / 2.0;
                let h = f64::from(p.dimensions.height);
                (
                    [p.position.x - w, p.position.y, p.position.z - w],
                    [p.position.x + w, p.position.y + h, p.position.z + w],
                )
            })
            .chain(self.carts.iter().map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            }))
            .chain(guests);
        for (lo, hi) in boxes {
            for x in (lo[0] + 1e-7).floor() as i32..=(hi[0] - 1e-7).floor() as i32 {
                for y in (lo[1] + 1e-7).floor() as i32..=(hi[1] - 1e-7).floor() as i32 {
                    for z in (lo[2] + 1e-7).floor() as i32..=(hi[2] - 1e-7).floor() as i32 {
                        inside.push(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        // AbstractArrow.applyEffectsFromBlocks: the blocks every arrow passed
        // through this tick (a wooden button is pressed by one inside it), and
        // the ones a thrown trident is in.
        for p in self
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.arrow.is_some())
        {
            inside.extend(horizon_client::arrows::blocks_inside(
                p.previous, p.position,
            ));
        }
        for t in &self.tridents {
            inside.extend(horizon_client::arrows::blocks_inside(
                t.position, t.position,
            ));
        }
        // BigDripleafBlock.entityInside also asks whether the player stands on
        // the ground and how high its feet are: the leaves its box's feet
        // layer is inside (the host and the LAN guests).
        let mut standing = Vec::new();
        let feet = self
            .player
            .iter()
            .filter(|_| !self.spectator())
            .map(|p| {
                (
                    [p.position.x, p.position.y, p.position.z],
                    p.ground.on_ground,
                    f64::from(p.dimensions.width) / 2.0,
                )
            })
            .chain(
                self.lan_guest_feet()
                    .into_iter()
                    .map(|(at, on_ground)| (at, on_ground, 0.3)),
            );
        for (at, on_ground, w) in feet {
            let y = (at[1] + 1e-7).floor() as i32;
            for x in (at[0] - w + 1e-7).floor() as i32..=(at[0] + w - 1e-7).floor() as i32 {
                for z in (at[2] - w + 1e-7).floor() as i32..=(at[2] + w - 1e-7).floor() as i32 {
                    standing.push((BlockPos::new(x, y, z), on_ground, at[1]));
                }
            }
        }
        self.with_level(|b, level, ticks| {
            // LilyPadBlock.entityInside: a boat inside a lily pad breaks it.
            let mut boat_cells: Vec<BlockPos> = Vec::new();
            for bounds in level.boat_trigger_boxes() {
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                for x in (lo.x + 1e-7).floor() as i32..=(hi.x - 1e-7).floor() as i32 {
                    for y in (lo.y + 1e-7).floor() as i32..=(hi.y - 1e-7).floor() as i32 {
                        for z in (lo.z + 1e-7).floor() as i32..=(hi.z - 1e-7).floor() as i32 {
                            boat_cells.push(BlockPos::new(x, y, z));
                        }
                    }
                }
            }
            for pos in boat_cells {
                b.boat_inside(level, ticks, pos);
            }
            // The mobs and the other entities (items, orbs, projectiles...) too.
            let mut touched: Vec<BlockPos> = Vec::new();
            for bounds in level.mob_trigger_boxes().chain(level.other_trigger_boxes()) {
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                for x in (lo.x + 1e-7).floor() as i32..=(hi.x - 1e-7).floor() as i32 {
                    for y in (lo.y + 1e-7).floor() as i32..=(hi.y - 1e-7).floor() as i32 {
                        for z in (lo.z + 1e-7).floor() as i32..=(hi.z - 1e-7).floor() as i32 {
                            touched.push(BlockPos::new(x, y, z));
                        }
                    }
                }
            }
            touched.sort_unstable_by_key(|p| (p.x, p.y, p.z));
            touched.dedup();
            for pos in inside.into_iter().chain(touched) {
                b.entity_inside(level, ticks, pos);
            }
            for (pos, on_ground, feet_y) in standing {
                b.entity_on_big_dripleaf(level, ticks, pos, on_ground, feet_y);
            }
        });
        self.process_level_events()?;
        // ServerLevel.tick: a Debug world runs no scheduled block or fluid ticks, and
        // ServerChunkCache.tickChunks no random ticks.
        let debug = self.world_debug;
        if !debug {
            self.tick_scheduled()?;
        }
        self.with_level(|b, level, ticks| {
            // Each chunk's ticking sections and their masks, read again only
            // after a block changed.
            let mut sections = Vec::new();
            horizon_client::spans::time("random_ticks", || {
                for (x, z) in ticking.into_iter().filter(|_| !debug) {
                    let mut read = None;
                    let pos = horizon_core::position::ChunkPos::new(x, z);
                    b.tick_chunk_random_masked(level, ticks, pos, |level, from| {
                        if read != Some(level.world.revision) {
                            read = Some(level.world.revision);
                            sections.clear();
                            if let Some(chunk) = level.world.chunks.get(&(x, z)) {
                                level.random_tick_sections.masked(
                                    (x, z),
                                    chunk,
                                    level.facts,
                                    |id| b.random_tick_candidate(BlockStateId(id)),
                                    &mut sections,
                                );
                            }
                        }
                        sections
                            .iter()
                            .find(|(y, _)| *y >= from)
                            .map(|(y, mask)| (*y, Some(mask.clone())))
                    });
                }
            });
            b.run_block_events(level, ticks);
            b.tick_block_entities(level, ticks);
        });
        // `PistonMovingBlockEntity.tick` pushes what its block just ran into,
        // with the progress it is about to take.
        let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
        self.push_piston_entities(pushes);
        self.process_level_events()?;
        self.sync_moving_pistons();
        Ok(())
    }
    /// Copies the moving-piston block entities into the world, where the
    /// collision queries and the renderer read them.
    fn sync_moving_pistons(&mut self) {
        let previous = std::mem::take(&mut self.world.moving_pistons);
        let current: Vec<_> = self
            .block_ticks
            .moving_pistons
            .iter()
            .map(|(p, piston)| ([p.x, p.y, p.z], piston.shape_state()))
            .collect();
        let mut fallback: Vec<_> = previous
            .into_iter()
            .filter(|(pos, _)| !current.iter().any(|(next, _)| next == pos))
            .collect();
        // A finished piston stays drawn until the chunk mesh shows the block
        // it placed, however many ticks that takes.
        let older = std::mem::take(&mut self.world.moving_piston_fallback);
        for (pos, piston) in older {
            let replaced = current.iter().chain(&fallback).any(|(p, _)| *p == pos);
            if !replaced && self.section_mesh_pending(pos) {
                fallback.push((pos, piston));
            }
        }
        self.world.moving_piston_fallback = fallback;
        self.world.moving_pistons = current;
    }
    /// `ServerLevel.tick`'s scheduled block then fluid ticks, one at a time:
    /// the side effects of each (a shrieker summoning a warden, drops, game
    /// events) happen before the next tick runs, as they do inline in vanilla.
    pub(super) fn tick_scheduled(&mut self) -> Result<()> {
        for fluids in [false, true] {
            if !self.with_level(|b, level, ticks| b.scheduled_begin(level, ticks, fluids)) {
                continue;
            }
            loop {
                // Due ticks run against one level while each leaves nothing for
                // the game to apply (level events, spilled items or orbs): the
                // next one would see the same level built again.
                let more = self.with_level(|b, level, ticks| {
                    loop {
                        let more = b.scheduled_step(level, ticks, fluids);
                        if !more || !level.events.is_empty() || !level.spill.is_empty() {
                            break more;
                        }
                    }
                });
                if !self.level_events.is_empty() {
                    self.process_level_events()?;
                }
                if !more {
                    break;
                }
            }
            self.with_level(|b, _, ticks| b.scheduled_finish(ticks, fluids));
        }
        Ok(())
    }
    /// Side effects reported by the behaviour library, in vanilla order.
    fn process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.level_events);
        self.lan_record_events(&events);
        dispense_fixture::record_events(&events);
        for event in events {
            match event {
                LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data,
                } => {
                    // LevelEventHandler: destroy particles and the block's
                    // break sound (none for air).
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.destroy_particles(p, state);
                    if self.res.facts().is_air(state) {
                        continue;
                    }
                    let sounds = self.res.facts().get(state).sounds.clone();
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio.play(
                        &sounds.break_sound,
                        Some(c),
                        (sounds.volume + 1.0) / 2.0,
                        sounds.pitch * 0.8,
                    );
                }
                LevelEvent::LevelEvent {
                    id: 3006,
                    pos,
                    data,
                } => self.sculk_charge_event(pos, data),
                LevelEvent::LevelEvent { id: 3007, pos, .. } => self.shriek_event(pos),
                // LevelEventHandler 1010 and 1011: a jukebox's song starts
                // (`playJukeboxSong`) or stops, and the parrots nearby dance or stop.
                LevelEvent::LevelEvent {
                    id: 1010,
                    pos,
                    data,
                } => self.jukebox_song_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 1011, pos, .. } => {
                    self.jukebox_stop_event([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 2019 and 2020: the crack on a face of the
                // block being mined, with its hit sound for 2020.
                LevelEvent::LevelEvent {
                    id: id @ (2019 | 2020),
                    pos,
                    data,
                } => {
                    let face = horizon_core::geometry::Direction::ALL[data.clamp(0, 5) as usize];
                    self.breaking_block_effects([pos.x, pos.y, pos.z], face.normal(), id == 2020);
                }
                LevelEvent::LevelEvent {
                    id: 3008,
                    pos,
                    data,
                } => self.brush_complete_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2004, pos, .. } => {
                    self.spawner_burst([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 1505: the growth particles, then the sound.
                LevelEvent::LevelEvent {
                    id: 1505,
                    pos,
                    data,
                } => {
                    self.growth_particles(pos, data);
                    let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
                    self.audio
                        .play("minecraft:item.bone_meal.use", Some(at), 1.0, 1.0);
                }
                LevelEvent::LevelEvent {
                    id: 2015,
                    pos,
                    data,
                } => self.dragon_egg_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2016,
                    pos,
                    data,
                } => self.shulker_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2017,
                    pos,
                    data,
                } => self.consume_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2018,
                    pos,
                    data,
                } => self.enderman_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2003, pos, .. } => {
                    self.eye_break_particles([pos.x, pos.y, pos.z]);
                }
                LevelEvent::LevelEvent {
                    id: 2013,
                    pos,
                    data,
                } => self.smash_dust(pos, data),
                // LevelEventHandler 2002 and 2007: a potion's splash.
                LevelEvent::LevelEvent {
                    id: 2002,
                    pos,
                    data,
                } => self.potion_splash_particles(pos, data, false),
                LevelEvent::LevelEvent {
                    id: 2007,
                    pos,
                    data,
                } => self.potion_splash_particles(pos, data, true),
                LevelEvent::LevelEvent {
                    id: id @ 3002..=3005,
                    pos,
                    data,
                } => self.block_particle_event(id, pos, data),
                LevelEvent::LevelEvent {
                    id: 1500,
                    pos,
                    data,
                } => self.composter_fill_event(pos, data == 1),
                // LevelEventHandler 1504: the stalactite's drip particle.
                LevelEvent::LevelEvent { id: 1504, pos, .. } => self.dripstone_drip_particle(pos),
                // LevelEventHandler 2006: the dragon fireball's burst.
                LevelEvent::LevelEvent {
                    id: 2006,
                    pos,
                    data,
                } => self.dragon_fireball_burst(pos, data),
                // LevelEventHandler 2009: a wet sponge drying.
                LevelEvent::LevelEvent { id: 2009, pos, .. } => {
                    self.puffs_above_block(pos, horizon_client::particles::ParticleType::Cloud)
                }
                // LevelEventHandler 2012: a turtle egg laid.
                LevelEvent::LevelEvent {
                    id: 2012,
                    pos,
                    data,
                } => self.turtle_egg_particles(pos, data),
                // LevelEventHandler 2014: a straw bed destroyed, with no sound.
                LevelEvent::LevelEvent {
                    id: 2014,
                    pos,
                    data,
                } => self.destroy_particles([pos.x, pos.y, pos.z], BlockStateId(data as u32)),
                // EnderDragonFight.spawnNewGateway: an always-visible burst
                // and a sound at the gateway for every nearby player.
                LevelEvent::LevelEvent { id: 3000, pos, .. } => {
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.particles.add_with(
                        &horizon_client::particles::ParticleOptions::simple(
                            horizon_client::particles::ParticleType::ExplosionEmitter,
                        ),
                        at,
                        [0.0; 3],
                        true,
                        true,
                    );
                    let random = self.particles.level_random();
                    let pitch = (1.0 + (random.next_float() - random.next_float()) * 0.2) * 0.7;
                    self.audio.play_unshared(
                        "minecraft:block.end_gateway.spawn",
                        Some(at),
                        10.0,
                        pitch,
                    );
                }
                // LevelEventHandler 3009: a sniffer egg cracking.
                LevelEvent::LevelEvent { id: 3009, pos, .. } => {
                    self.sniffer_egg_crack_particles(pos)
                }
                LevelEvent::LevelEvent {
                    id: id @ 3011..=3021,
                    pos,
                    data,
                } => self.trial_level_event(id, [pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ]);
                    let sound = match id {
                        1503 => Some(("minecraft:block.end_portal_frame.fill", 1.0)),
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1033 => Some(("minecraft:block.chorus_flower.grow", 1.0)),
                        1034 => Some(("minecraft:block.chorus_flower.death", 1.0)),
                        1045 => Some(("minecraft:block.pointed_dripstone.land", 2.0)),
                        1052 => Some(("minecraft:block.sulfur_spike.land", 2.0)),
                        1046 => Some((
                            "minecraft:block.pointed_dripstone.drip_lava_into_cauldron",
                            2.0,
                        )),
                        1047 => Some((
                            "minecraft:block.pointed_dripstone.drip_water_into_cauldron",
                            2.0,
                        )),
                        // LevelEvent.SOUND_ZOMBIE_WOODEN_DOOR (attack, break).
                        1019 => Some(("minecraft:entity.zombie.attack_wooden_door", 2.0)),
                        1021 => Some(("minecraft:entity.zombie.break_wooden_door", 2.0)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1029 => Some(("minecraft:block.anvil.destroy", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1042 => Some(("minecraft:block.grindstone.use", 1.0)),
                        1044 => Some(("minecraft:block.smithing_table.use", 1.0)),
                        // LevelEvent.SOUND_PAGE_TURN (a lectern's page).
                        1043 => Some(("minecraft:item.book.page_turn", 1.0)),
                        // LevelEvent.SOUND_SPLASH_POTION and its instant twin.
                        1053 | 1054 => Some(("minecraft:entity.splash_potion.break", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        let pitch = match id {
                            1009 => {
                                2.6 + (self.world.random.next_float()
                                    - self.world.random.next_float())
                                    * 0.8
                            }
                            // LevelEventHandler draws these from the client
                            // level's random, before 1501's and 1502's smoke.
                            1501 | 1502 => {
                                let r = self.particles.level_random();
                                2.6 + (r.next_float() - r.next_float()) * 0.8
                            }
                            1045 | 1046 | 1047 | 1052 => {
                                self.particles.level_random().next_float() * 0.1 + 0.9
                            }
                            1029 | 1030 | 1042 | 1043 | 1044 | 1053 | 1054 => {
                                self.world.random.next_float() * 0.1 + 0.9
                            }
                            1019 | 1021 => {
                                (self.world.random.next_float() - self.world.random.next_float())
                                    * 0.2
                                    + 1.0
                            }
                            _ => 1.0,
                        };
                        if matches!(
                            id,
                            1019 | 1021 | 1029 | 1030 | 1045 | 1052 | 1053 | 1054 | 1503
                        ) {
                            // Clients play these sounds from their level events;
                            // capturing it also sends a duplicate sound packet.
                            self.audio.play_unshared(name, c, volume, pitch);
                        } else {
                            self.audio.play(name, c, volume, pitch);
                        }
                    }
                    // LevelEventHandler draws smoke after the sound.
                    match id {
                        1501 => self.puffs_above_block(
                            pos,
                            horizon_client::particles::ParticleType::LargeSmoke,
                        ),
                        1502 => self.torch_burnout_smoke(pos),
                        1503 => self.end_portal_frame_smoke(pos),
                        _ => {}
                    }
                }
                LevelEvent::Sound {
                    pos,
                    sound,
                    volume,
                    pitch,
                } => {
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    let name = if sound.contains(':') {
                        sound.to_string()
                    } else {
                        format!("minecraft:{sound}")
                    };
                    self.audio.play(&name, Some(c), volume, pitch);
                }
                // Handled as the behaviour emits it (GameLevel::emit).
                LevelEvent::DropResources { pos, state } => {
                    self.with_level(|_, level, _| level.drop_resources_now(pos, state));
                }
                LevelEvent::FallingBlock {
                    pos,
                    state,
                    hurt_per_distance,
                } => {
                    let mut damage = horizon_client::falling_block::Damage::falling(
                        &self.res.facts().get(state).name,
                    );
                    if let Some(per_distance) = hurt_per_distance {
                        damage.hurt_entities = true;
                        damage.per_distance = per_distance;
                    }
                    let p = Vec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y),
                        f64::from(pos.z) + 0.5,
                    );
                    self.falling.push(FallingBlock {
                        id: 0,
                        state,
                        start_pos: pos,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        age: 0,
                        no_gravity: false,
                        drop_item: true,
                        fall_distance: 0.0,
                        stuck: Vec3::ZERO,
                        damage,
                        saved: None,
                    });
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep if self.beds_destroy_on_use() => {
                        self.destroy_bed_on_use([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::Sleep => self.try_sleep([pos.x, pos.y, pos.z]),
                    PlayerEffect::GiveItem(item) => {
                        if let Some(rest) = self.give(ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        }) {
                            self.pop_resource([pos.x, pos.y, pos.z], rest);
                        }
                    }
                    // Popped as the behaviour emits it (GameLevel::emit).
                    PlayerEffect::DropItem(_) => {}
                    PlayerEffect::EatCake => {
                        // CakeBlock.eat
                        self.stats.custom("eat_cake_slice", 1);
                        if let Some(p) = self.player.as_mut() {
                            let food = &mut p.health.food;
                            food.food = (food.food + 2).min(20);
                            food.saturation = (food.saturation + 0.4).min(food.food as f32);
                        }
                    }
                    PlayerEffect::RespawnAnchor => {
                        self.respawn_anchor_used([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::InteractLoot(_) => {}
                },
                LevelEvent::PrimeTnt { pos, rot } => self.add_primed_tnt(pos, rot, 80),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "dispense",
                } => self.dispense_from(pos, state)?,
                LevelEvent::BlockEvent {
                    pos, kind: "golem", ..
                } => self.check_golem_spawn([pos.x, pos.y, pos.z])?,
                LevelEvent::BlockEvent {
                    pos,
                    kind: "command_block",
                    ..
                } => self.command_block_tick(pos),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "structure_block",
                    ..
                } => self.structure_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block",
                    ..
                } => self.test_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block_tick",
                    ..
                } => self.reset_test_block((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "brushable",
                    ..
                } => self.brushable_tick([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos, kind: "hatch", ..
                } => {
                    // FrogspawnBlock.hatchFrogspawn: 2-5 tadpoles.
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio
                        .play("minecraft:block.frogspawn.hatch", Some(at), 1.0, 1.0);
                    let count = 2 + self.world.random.next_int_bound(4).unwrap_or(0);
                    for _ in 0..count {
                        // getRandomTadpolePositionOffset: 0.2..0.8 inside the block.
                        let r = &mut self.world.random;
                        let off = |r: &mut horizon_core::random::LegacyRandom| {
                            f64::from(r.next_float()) * 0.6 + 0.2
                        };
                        let (dx, dz) = (off(r), off(r));
                        let spawn = [
                            f64::from(pos.x) + dx,
                            f64::from(pos.y) - 0.5,
                            f64::from(pos.z) + dz,
                        ];
                        let _ = self.spawn_mob("minecraft:tadpole", spawn, true);
                    }
                }
                LevelEvent::BlockEvent {
                    pos,
                    kind: "hatch_sniffer",
                    ..
                } => self.hatch_sniffer([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "hatch_turtles",
                } => self.hatch_turtles([pos.x, pos.y, pos.z], state),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "portal_spawn",
                    ..
                } => self.portal_spawn([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "spawn_ghastling",
                } => self.spawn_ghastling([pos.x, pos.y, pos.z], state),
                LevelEvent::GameEvent { pos, event, state } => {
                    self.sculk_level_event(pos, event, state)
                }
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "shrieker_respond",
                } => self.shrieker_block_event(pos, state),
                LevelEvent::BlockEntityEvent { pos, a, b, .. } => {
                    self.block_entity_event([pos.x, pos.y, pos.z], a, b)
                }
                // NoteBlock.triggerEvent: a NOTE particle above, coloured by
                // the note (`note / 24.0` in the x speed).
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "note_particle",
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let note = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("note")?.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    self.particles.add(
                        &ParticleOptions::simple(ParticleType::Note),
                        [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 1.2,
                            f64::from(pos.z) + 0.5,
                        ],
                        [note / 24.0, 0.0, 0.0],
                    );
                }
                // NoteBlock.getCustomSoundId: the skull above's note block sound,
                // played at volume 3 (vanilla's pitch for a head is 1).
                LevelEvent::BlockEvent {
                    pos,
                    kind: "note_custom_sound",
                    ..
                } => {
                    if let Some(BlockEntity::Skull(skull)) =
                        self.world.block_entities.get(&(pos.x, pos.y + 1, pos.z))
                        && let Some(sound) = skull.note_block_sound.as_deref()
                    {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.audio.play(sound, Some(at), 3.0, 1.0);
                    }
                }
                // `level.addParticle`: only the local player's own click draws it.
                LevelEvent::BlockEvent {
                    kind: "candle_smoke",
                    ..
                } if self.lan_acting.is_some() => {}
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "candle_smoke",
                } => self.candle_smoke([pos.x, pos.y, pos.z], state),
                // The block at `pos` has just replaced `state`: Block.pushEntitiesUp.
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "push_entities_up",
                } => self.push_entities_up(pos, state),
                LevelEvent::GrowTree { .. }
                | LevelEvent::PlaceFeature { .. }
                | LevelEvent::BlockEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Menus opened by PlayerEffect::OpenMenu.
    fn open_menu(&mut self, pos: [i32; 3]) {
        let name = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let key = (pos[0], pos[1], pos[2]);
        if self.chest_menu_blocked(key, &name) {
            // The use still succeeds, but no menu, stat or piglin anger.
            return;
        }
        if &*name == "minecraft:lectern" {
            self.open_lectern(pos);
            return;
        }
        if horizon_client::command_block::Mode::of_block(&name).is_some() {
            self.open_command_block(key);
            return;
        }
        // The game master blocks' screens open on the player's own client
        // (`BlockEntity.usedBy`); the server has nothing to open.
        if matches!(
            &*name,
            "minecraft:structure_block"
                | "minecraft:jigsaw"
                | "minecraft:test_block"
                | "minecraft:test_instance_block"
        ) {
            return;
        }
        // SignBlock.useWithoutItem: the text editor, not a container.
        if horizon_client::signs::sign_kind(&name).is_some() {
            self.use_sign_empty_hand(pos);
            return;
        }
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
            self.craft_table = Some(pos);
        } else if &*name == "minecraft:ender_chest" {
            self.open_ender_chest(key);
            // EnderChestBlock.useWithoutItem
            self.stats.custom("open_enderchest", 1);
            self.anger_piglins(true);
            if self.container_opener_event(key, true) {
                self.container_edge_sound(key, true);
            }
        } else if &*name == "minecraft:beacon" && self.open_station(pos, &name) {
            self.world.block_entities.get_or_insert_with(key, || {
                BlockEntity::for_block(&name).unwrap_or(BlockEntity::Chest(Vec::new()))
            });
        } else if storage::is_shulker_box(&name) && !self.shulker_can_open(pos) {
            // ShulkerBoxBlock.canOpen: the lid has no room; nothing opens.
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.get_or_insert_with(key, || entity);
            self.unpack_opened_loot(key, &name);
            let centre = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            self.resolve_container_maps(key, centre);
            self.container = Some(key);
            let first = self.container_opener_event(key, true);
            self.post_block_game_event(
                "minecraft:container_open",
                [key.0, key.1, key.2],
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            let stat = match &*name {
                "minecraft:chest" => "open_chest",
                "minecraft:trapped_chest" => "trigger_trapped_chest",
                "minecraft:barrel" => "open_barrel",
                "minecraft:hopper" => "inspect_hopper",
                "minecraft:dropper" => "inspect_dropper",
                "minecraft:dispenser" => "inspect_dispenser",
                n if storage::is_shulker_box(n) => "open_shulker_box",
                _ => "",
            };
            if !stat.is_empty() {
                self.stats.custom(stat, 1);
            }
            // Opening a piglin-guarded container angers the piglins who see it.
            if self.guarded_by_piglins(&name) {
                self.anger_piglins(true);
            }
            if first {
                self.container_edge_sound(key, true);
            }
        } else if self.open_mod_menu(key) {
            // A mod block's container (`native_block_entity_container`).
            self.container = Some(key);
        } else if self.open_station(pos, &name) {
            // Workstation menus: game/stations.rs.
        } else {
            self.message = Some((
                format!(
                    "{} is not available yet",
                    name.trim_start_matches("minecraft:").replace('_', " ")
                ),
                Instant::now(),
            ));
            return;
        }
        self.screen = Screen::Inventory;
        self.capture(false);
        self.right = false;
    }
    /// FallingBlockEntity.tick: fall with 0.04 gravity and 0.98 drag, then land.
    fn tick_falling(&mut self) -> Result<()> {
        let falling = std::mem::take(&mut self.falling);
        let mut kept = Vec::with_capacity(falling.len());
        let mut collision_error = None;
        let entity_drops = self.ui.rules.bool("entity_drops");
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        // FallingBlockEntity.tick places or drops each block in its own tick,
        // so a block falling onto one that just landed lands on top of it.
        for mut f in falling {
            if !ticking(f.position.x.floor() as i32, f.position.z.floor() as i32) {
                kept.push(f);
                continue;
            }
            let original = f.clone();
            match self.fall_step(&mut f, entity_drops) {
                Ok(FallStep::Falling) => kept.push(f),
                Ok(FallStep::Gone) => {}
                Ok(FallStep::Dropped(at)) => self.drop_falling_block(f.state, at),
                Ok(FallStep::Landed(pos)) => self.finish_falling_block(f, pos),
                Err(error) => {
                    kept.push(original);
                    collision_error.get_or_insert(error);
                }
            }
        }
        // Blocks that started falling meanwhile tick from the next tick on.
        let started = std::mem::replace(&mut self.falling, kept);
        self.falling.extend(started);
        self.process_level_events()?;
        if let Some(error) = collision_error {
            return Err(error);
        }
        Ok(())
    }
    /// One `FallingBlockEntity.tick` move of `f`.
    fn fall_step(&self, f: &mut FallingBlock, entity_drops: bool) -> Result<FallStep> {
        f.previous = f.position;
        f.age = f.age.wrapping_add(1);
        if !f.no_gravity {
            f.velocity.y -= 0.04;
        }
        let mut wanted = f.velocity;
        // Entity.move: a cobweb it was in scales this move, which then
        // starts from rest.
        if f.stuck.length_squared() > 1.0e-7 {
            wanted = wanted.multiply(f.stuck);
            f.stuck = Vec3::ZERO;
            f.velocity = Vec3::ZERO;
        }
        let mut scene = collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: Some(collisions::PowderSnowSubject {
                fall_distance: f.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        };
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: f.position.y,
                descending: wanted.y < 0.0,
                main_hand: None,
            },
            false,
        );
        // On an error the caller keeps the block as it was before this tick.
        let movement = scene.resolve_with_context(
            horizon_client::falling_block::bounds(f.position),
            wanted,
            0.0,
            false,
            context,
        )?;
        let next = Vec3::new(
            f.position.x + movement.x,
            f.position.y + movement.y,
            f.position.z + movement.z,
        );
        let on_ground = wanted.y < 0.0 && movement.y != wanted.y;
        // FallingBlockEntity.tick: off the ground, it gives up after 600
        // ticks (100 outside the world) and drops itself as an item.
        let y = next.y.floor() as i32;
        let outside = y <= self.world.bottom_y() || y >= self.world.top_y();
        if !on_ground && (f.age > 100 && outside || f.age > 600) {
            return Ok(if f.drop_item && entity_drops {
                FallStep::Dropped(next)
            } else {
                FallStep::Gone
            });
        }
        if next.y < f.position.y {
            f.fall_distance += f.position.y - next.y;
        }
        let from = f.position;
        f.position = next;
        if on_ground {
            let pos = [
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            ];
            return Ok(FallStep::Landed(pos));
        }
        // Entity.move stops it on the axes it collided on.
        if movement.x != wanted.x {
            f.velocity.x = 0.0;
        }
        if movement.y != wanted.y {
            f.velocity.y = 0.0;
        }
        if movement.z != wanted.z {
            f.velocity.z = 0.0;
        }
        // applyEffectsFromBlocks: WebBlock.entityInside makes it stuck
        // (Entity.makeStuckInBlock), BubbleColumnBlock.entityInside lifts or
        // drags it. Entity.move records the move unless it was blocked short
        // of a tiny one.
        let moved = movement.length_squared();
        let recorded = moved > 1.0e-7 || wanted.length_squared() - moved < 1.0e-7;
        let world = &self.world;
        let facts = self.res.facts();
        let (registry, collision) = (self.pack.assets.registry, self.collision);
        let stuck = horizon_client::falling_block::apply_effects_from_blocks(
            from,
            f.position,
            recorded.then_some(wanted),
            &mut f.velocity,
            &mut f.fall_distance,
            |p| {
                use horizon_client::falling_block::Inside;
                if &*facts.get(world.block(p.x, p.y, p.z)).name == "minecraft:cobweb" {
                    Inside::Cobweb
                } else if let Some(column) =
                    horizon_client::motion::bubble_column_at(world, registry, facts, collision, p)
                {
                    Inside::BubbleColumn(column)
                } else {
                    Inside::Nothing
                }
            },
        );
        if stuck {
            f.stuck = horizon_client::falling_block::web_multiplier();
        }
        f.velocity = Vec3::new(
            f.velocity.x * 0.98,
            f.velocity.y * 0.98,
            f.velocity.z * 0.98,
        );
        Ok(FallStep::Falling)
    }
    /// `Entity.spawnAtLocation`: an item entity where the block was.
    fn drop_falling_block(&mut self, state: BlockStateId, at: Vec3) {
        // Entity.spawnAtLocation: an item entity where the block was.
        let item = self.res.facts().get(state).item.clone();
        if &*item != "minecraft:air" {
            let r = &mut self.world.random;
            let v = [
                r.next_double() * 0.2 - 0.1,
                0.2,
                r.next_double() * 0.2 - 0.1,
            ];
            let stack = ItemStack {
                item,
                count: 1,
                damage: 0,
                extra: None,
            };
            self.spawn_item(stack, [at.x, at.y, at.z], v, 10);
        }
    }
    /// `DecoratedPotBlock.playerWillDestroy`: a pot mined with an item in
    /// `breaks_decorated_pots`, unless an enchantment prevents shattering, is
    /// cracked first (flags 260), so it shatters into its sherds.
    fn crack_mined_pot(&mut self, pos: [i32; 3], state: BlockStateId) -> BlockStateId {
        let registry = &self.pack.assets.registry;
        let Some(mut pot) = registry
            .state(state)
            .filter(|s| s.name == "minecraft:decorated_pot")
            .cloned()
        else {
            return state;
        };
        let Some(held) = self.held() else {
            return state;
        };
        let breaks = self
            .res
            .items()
            .item_tags
            .get("minecraft:breaks_decorated_pots")
            .is_some_and(|t| t.contains(&*held.item));
        let enchantments = &self.stations().enchantments;
        let kept = held
            .enchantments()
            .iter()
            .any(|(e, _)| enchantments.in_tag("minecraft:prevents_decorated_pot_shattering", e));
        if !breaks || kept {
            return state;
        }
        pot.properties.insert("cracked".into(), "true".into());
        let Ok(cracked) = registry.resolve(&pot) else {
            return state;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
        cracked
    }
    /// `ItemStack.mineBlock` for the held stack once a survival player has
    /// mined the block `name`: an item with a `tool` component counts as used
    /// and loses its `damage_per_block` on a block that doesn't break
    /// instantly (`ShearsItem`: on any block but fire).
    fn mine_block_with_held(&mut self, name: &str, hardness: f32) {
        let Some(stack) = self.inventory[self.selected].clone() else {
            return;
        };
        let Some((per_block, max)) = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| Some((i.tool_damage_per_block?, i.max_damage)))
        else {
            return;
        };
        let wears = if &*stack.item == "minecraft:shears" {
            !self.items().block_in_tag("minecraft:fire", name)
        } else {
            hardness != 0.0
        };
        if wears && per_block > 0 && max > 0 {
            self.damage_held(per_block);
        }
        self.stats.add("used", &stack.item, 1);
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.lan_break_pos = Some(pos);
        let broken = self.with_player_actor(|game| game.break_block_as_player(pos));
        self.lan_break_pos = None;
        broken
    }
    fn break_block_as_player(&mut self, pos: [i32; 3]) -> Result<()> {
        // DebugStickItem.canDestroyBlock: a left click selects a property and
        // never destroys the block.
        if !self.debug_stick_can_destroy(pos)? {
            return Ok(());
        }
        // ServerPlayerGameMode.destroyBlock.
        if self.block_action_restricted(pos) {
            return Ok(());
        }
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            return Ok(());
        }
        // ServerPlayerGameMode.destroyBlock.
        let state = self.world.block(pos[0], pos[1], pos[2]);
        // Native mods, once the game allowed the break: a canceled one keeps
        // the block, and a guest's acknowledgement puts it back in its world.
        let mut canceled = false;
        self.native_destroy_block(pos, state, &mut canceled);
        if canceled {
            return Ok(());
        }
        self.pad_rumble(horizon_client::gamepad::Rumble::BlockBroken);
        let broken = state;
        let facts = self.res.facts().get(state).clone();
        let state = self.crack_mined_pot(pos, state);
        // Block.playerWillDestroy: mining their gold angers all nearby piglins.
        if self.guarded_by_piglins(&facts.name) {
            self.anger_piglins(false);
        }
        // The block entity as it was, for what the drops copy from it.
        let key = (pos[0], pos[1], pos[2]);
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let name = self.world.block_names.get(&key).cloned();
            self.pending.record(pos, &entity, &facts.name, name);
        } else if let Some(tag) = spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        self.creaking_heart_broken(pos, true);
        let hive = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Beehive(h)) => self.hive_state(pos).map(|s| ((**h).clone(), s)),
            _ => None,
        };
        // playerWillDestroy and Level.removeBlock through the vanilla behaviour
        // library: the block entity's side effects (containers spill), neighbour
        // and shape updates, and the destroy event (particles, sound).
        let creative = self.creative;
        // TripWireBlock.playerWillDestroy: shears disarm the string first (flags
        // 260), so taking it down does not trip the hook.
        if &*facts.name == "minecraft:tripwire"
            && self.held().is_some_and(|s| &*s.item == "minecraft:shears")
            && let Some(disarmed) = self.with_props(state, &[("disarmed", "true")])
        {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, disarmed, 260));
            self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        }
        // BeehiveBlock.playerWillDestroy: a creative break still drops the hive
        // with its bees and honey level.
        if creative
            && self.ui.rules.bool("block_drops")
            && let Some((h, (honey, _))) = &hive
            && (!h.bees.is_empty() || honey.is_some_and(|l| l > 0))
            && let Some(stack) = self.picked_stack(pos, true)
        {
            let at = pos.map(f64::from);
            self.spawn_item(stack, at, dispenser::new_item_motion(), 10);
        }
        let correct_tool =
            !facts.requires_tool || self.res.items().correct_for_drops(self.held(), &facts.name);
        let removed = self.with_level(|b, level, ticks| {
            b.player_destroy_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                creative,
                correct_tool,
            )
        });
        if removed && !creative {
            let held = self.held().cloned();
            self.mine_block_with_held(&facts.name, facts.hardness);
            let harvest = !facts.requires_tool
                || self
                    .res
                    .items()
                    .correct_for_drops(held.as_ref(), &facts.name);
            if harvest {
                // Block.playerDestroy: the mined statistic, the exhaustion and the drops.
                self.stats.add("mined", &facts.name, 1);
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(0.005);
                }
                let drops = self.block_loot_at(pos, state, held.as_ref());
                let silk = held
                    .as_ref()
                    .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
                for d in drops {
                    if &*d.item != "minecraft:air" {
                        self.pop_resource(pos, d);
                    }
                }
                // BeehiveBlock.playerDestroy: silk touch keeps the bees in, so
                // the trigger counts them; otherwise they are out already.
                if let Some((hive, (honey, facing))) = hive {
                    let bees = if silk { hive.bees.len() as i64 } else { 0 };
                    if !silk {
                        self.hive_broken_by_player(pos, hive, honey, facing);
                    }
                    self.trigger_bee_nest_destroyed(broken, held.as_ref(), bees);
                }
                // InfestedBlock.spawnAfterBreak: its silverfish comes out unless the
                // tool has silk touch (#prevents_infested_spawns) or blocks don't drop.
                if facts.name.starts_with("minecraft:infested_")
                    && !silk
                    && self.ui.rules.bool("block_drops")
                {
                    let at = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]),
                        f64::from(pos[2]) + 0.5,
                    ];
                    if let Some(id) = self.spawn_mob("minecraft:silverfish", at, false) {
                        self.mob_poof(id);
                    }
                }
                // SpawnerBlock.spawnAfterBreak pops its experience whatever the tool.
                let xp = if silk && &*facts.name != "minecraft:spawner" {
                    0
                } else {
                    self.block_experience(&facts.name)
                };
                if xp > 0 {
                    self.spawn_orbs(
                        [
                            f64::from(pos[0]) + 0.5,
                            f64::from(pos[1]) + 0.5,
                            f64::from(pos[2]) + 0.5,
                        ],
                        xp,
                    );
                }
                // TurtleEggBlock.playerDestroy: after the drops, one egg of a
                // mined stack is taken and the rest stay.
                self.with_level(|b, level, ticks| {
                    b.turtle_egg_destroyed_by_player(
                        level,
                        ticks,
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        broken,
                    );
                });
            }
            // IceBlock.playerDestroy: the ice melts back into water (the
            // #prevents_ice_melting tool tag holds only silk touch).
            let prevents_melting = held
                .as_ref()
                .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
            self.with_level(|b, level, ticks| {
                b.ice_destroyed_by_player(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    broken,
                    prevents_melting,
                );
            });
        }
        self.pending.clear(pos);
        self.process_level_events()?;
        Ok(())
    }
    /// The hand whose use ended the click, if any.
    fn use_item(&mut self) -> Result<Option<Hand>> {
        self.with_player_actor(|game| game.use_item_as_player())
    }
    /// `Minecraft.startUseItem`: the main hand, then the off hand.
    fn use_item_as_player(&mut self) -> Result<Option<Hand>> {
        // Spectators only look (MultiPlayerGameMode.useItemOn/useItem pass),
        // but on a server a spectator's use on a block opens the block's
        // menu (ServerPlayerGameMode.useItemOn).
        if self.spectator() {
            if matches!(self.guest_use, Some(GuestUse::OnBlock(_)))
                && let Some(hit) = self.pick()
            {
                self.spectator_open_menu(hit.pos);
            }
            return Ok(None);
        }
        // A carrot or warped fungus on a stick boosts the ridden pig or strider.
        // A guest's use packets name one hand and touch no entity: the client
        // sends one per hand and part of its click, and Interact for entities.
        let guest = self.guest_use;
        if let Some(GuestUse::Entity(hand, target)) = guest {
            return Ok(self
                .with_hand(hand, |g| g.interact_entity(Some(target)))?
                .then_some(hand));
        }
        let on_block = matches!(guest, Some(GuestUse::OnBlock(_)));
        if !on_block
            && self.riding.is_some()
            && let Some(item) = self.held().map(|s| s.item.clone())
            && self.use_steering_stick(&item)
        {
            return Ok(Some(Hand::Main));
        }
        if guest.is_none() && self.interact_entity(None)? {
            return Ok(Some(Hand::Main));
        }
        // An EntityHitResult may fall through to Item.use, never to useItemOn.
        let block_hit = if matches!(guest, Some(GuestUse::Item(_))) || self.entity_targeted() {
            None
        } else {
            self.pick()
        };
        let hands = match guest {
            Some(GuestUse::OnBlock(hand) | GuestUse::Item(hand) | GuestUse::Entity(hand, _)) => {
                vec![hand]
            }
            None => Hand::BOTH.to_vec(),
        };
        let entity = guest.is_none() && self.entity_targeted();
        for hand in hands {
            // The off hand's turn starts on the entity too (Minecraft.startUseItem).
            if entity && hand == Hand::Off && self.with_hand(hand, |g| g.interact_entity(None))? {
                return Ok(Some(hand));
            }
            // A client of a server never hits a fence knot with its crosshair:
            // `LeadItem.useOn` and `LeashFenceKnotEntity.interact` reach the
            // server as a click on the fence block.
            if on_block && self.with_hand(hand, Game::interact_knot) {
                return Ok(Some(hand));
            }
            if self.with_hand(hand, |game| game.use_hand(block_hit.as_ref()))? {
                return Ok(Some(hand));
            }
        }
        Ok(None)
    }
    /// The index in `list` of the entity an interact packet's `target` names,
    /// by its server id (`Level.getEntity(id)`), or None for an id this level
    /// does not hold: `handleInteract` finds nothing then either.
    fn named_entity<T>(list: &[T], target: i32, id: impl Fn(&T) -> u32) -> Option<usize> {
        list.iter().position(|e| id(e) as i32 == target)
    }
    /// `Player.interactOn` with the acting hand: whether it took the click. A
    /// guest's interact packet names its entity (`target`), which the helpers
    /// use as it is; the local player picks from the crosshair instead, the
    /// way `Minecraft.hitResult` does.
    fn interact_entity(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        // Native mods, where Player.interactOn starts: before the entity's
        // own interact and the item's interactLivingEntity. The entity is the
        // one the packet named, or the one the crosshair picked.
        let named = match target {
            Some(t) => Some(t.id),
            None => self.pick_entity_hit().flatten(),
        };
        if let Some(id) = named
            && let Some(entity) = self.level_entity(id)
        {
            let kind = self.target_type(entity);
            let stack = self.held().cloned();
            let mut claimed = false;
            self.native_interact_entity(id, &kind, stack.as_ref(), &mut claimed);
            if claimed {
                self.swing_arm();
                return Ok(true);
            }
        }
        Ok(self.interact_frame(target)
            || self.interact_armor_stand(target)
            || self.interact_cushion(target)
            || self.interact_boat(target)
            || self.interact_cart(target)
            || self.interact_mob(target)?
            // Knots are reached only through the crosshair pick for now: a
            // guest's Interact naming a knot's id is a known gap.
            || target.is_none() && self.interact_knot())
    }
    /// `ItemStack.applyAfterUseComponentSideEffects`: a used stack with a
    /// `use_cooldown` component starts its entry (`UseCooldown.apply`). A
    /// server's guest also gets `ClientboundCooldownPacket`
    /// (`ServerItemCooldowns.onCooldownStarted`), queued for their turn's end.
    pub(super) fn apply_use_cooldown(&mut self, stack: &ItemStack) {
        let default = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.use_cooldown.clone());
        let Some(component) = horizon_client::cooldowns::component(stack, default.as_ref()) else {
            return;
        };
        let group = horizon_client::cooldowns::group(stack, default.as_ref());
        self.add_cooldown(group, component.ticks());
    }
    /// `ItemCooldowns.addCooldown` (a guest also gets the packet).
    pub(super) fn add_cooldown(&mut self, group: Arc<str>, ticks: i32) {
        self.cooldowns.add(Arc::clone(&group), ticks);
        if self.lan_acting.is_some() {
            self.guest_cooldowns.push((group, ticks));
        }
    }
    /// One hand's turn in `startUseItem`: `useItemOn` the block aimed at,
    /// then `useItem`. Whether the click ends here (it acted or failed).
    fn use_hand(&mut self, block_hit: Option<&Hit>) -> Result<bool> {
        self.use_failed = false;
        // isSecondaryUseActive with something in either hand keeps blocks
        // from reacting.
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        let mut anchor_passed = false;
        // Native mods, where ServerPlayerGameMode.useItemOn starts: a
        // canceled click does nothing (a guest's block is sent back).
        if let Some(hit) = block_hit {
            let mut canceled = false;
            self.native_right_click_block(hit.pos, hit.face, &mut canceled);
            if canceled {
                // Nothing was used, so nothing consumed the action.
                self.use_failed = true;
                return Ok(true);
            }
        }
        // MultiPlayerGameMode.useItemOn: blocks react first.
        if !(sneaking && hands_full)
            && let Some(hit) = block_hit
        {
            let held = self.held().cloned();
            if self.use_lectern(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_sign_applicator(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_golem_statue(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_jukebox(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if let Some(swings) = self.use_shelf(hit) {
                let used = held.unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                self.trigger_block("minecraft:item_used_on_block", hit.pos, &used);
                if swings {
                    self.swing_arm();
                }
                return Ok(true);
            }
            if let Some(swings) = self.use_chiseled_bookshelf(hit, held.as_ref()) {
                if swings {
                    self.swing_arm();
                }
                return Ok(true);
            }
            if self.use_composter(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_vault(hit.pos, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_decorated_pot(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_pumpkin(hit.pos, hit.face, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_beehive(hit.pos, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_campfire_douse(hit.pos, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_redstone_ore(hit.pos, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            match self.use_respawn_anchor(hit.pos, held.as_ref())? {
                respawn_anchor::AnchorUse::Charged => {
                    self.swing_arm();
                    return Ok(true);
                }
                // The main hand passes; its item is used, then the off hand's glowstone.
                respawn_anchor::AnchorUse::Pass => anchor_passed = true,
                respawn_anchor::AnchorUse::Other => {}
            }
            // CeilingHangingSignBlock.useItemOn: a hanging sign aimed at the
            // underside passes, so the next sign chains below instead of
            // opening the editor, unless the side has click commands to run
            // (`shouldTryToChainAnotherHangingSign`).
            let chains_sign = hit.face == [0, -1, 0]
                && held
                    .as_ref()
                    .is_some_and(|s| s.item.ends_with("_hanging_sign"))
                && {
                    let name = &self
                        .res
                        .facts()
                        .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                        .name;
                    name.ends_with("_hanging_sign") && !name.contains("_wall_")
                }
                && matches!(
                    self.world
                        .block_entities
                        .get(&(hit.pos[0], hit.pos[1], hit.pos[2])),
                    Some(BlockEntity::Sign(_))
                )
                && !self.sign_can_execute_click_commands((hit.pos[0], hit.pos[1], hit.pos[2]));
            // TRY_WITH_EMPTY_HAND: only the main hand's turn uses the block
            // itself (useWithoutItem).
            if !anchor_passed && !chains_sign && self.hand == Hand::Main {
                let face = match hit.face {
                    [0, 1, 0] => Direction::Up,
                    [0, -1, 0] => Direction::Down,
                    [0, 0, -1] => Direction::North,
                    [0, 0, 1] => Direction::South,
                    [-1, 0, 0] => Direction::West,
                    _ => Direction::East,
                };
                let player = self.use_context();
                let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
                let item = held.as_ref().map(|s| s.item.to_string());
                let cake = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                let adds_candle =
                    self.with_level(|b, _, _| b.cake_candle_state(cake, item.as_deref()).is_some());
                let pots_flower = self
                    .with_level(|b, _, _| b.potted_flower_state(cake, item.as_deref()).is_some());
                let (result, branch) = self.with_level(|b, level, ticks| {
                    b.use_item_on_branch(
                        level,
                        ticks,
                        BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                        face,
                        at,
                        player,
                        item.as_deref(),
                    )
                });
                self.process_level_events()?;
                // CakeBlock.useItemOn: the candle is used up and counted.
                if adds_candle
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.stats.add("used", &used.item, 1);
                    self.consume_held();
                }
                // FlowerPotBlock.useItemOn: the plant is potted, used up, and counted.
                if pots_flower
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.stats.add("used", &used.item, 1);
                    self.stats.custom("pot_flower", 1);
                    self.consume_held();
                }
                // NoteBlock.useWithoutItem (tuned) and BellBlock.onHit (rung).
                if result == horizon_world::level::InteractionResult::Success {
                    let name = self
                        .res
                        .facts()
                        .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                        .name
                        .clone();
                    match &*name {
                        "minecraft:note_block" => self.stats.custom("tune_noteblock", 1),
                        "minecraft:bell" => self.stats.custom("bell_ring", 1),
                        "minecraft:lever" => self.lever_puff(hit.pos),
                        _ => {}
                    }
                }
                if result.consumes_action() {
                    // ServerPlayerGameMode.useItemOn: ITEM_USED_ON_BLOCK with
                    // the stack before the use, or DEFAULT_BLOCK_USE.
                    match branch {
                        horizon_world::level::UseBranch::ItemOn => {
                            let used = held.unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                            self.trigger_block("minecraft:item_used_on_block", hit.pos, &used);
                        }
                        horizon_world::level::UseBranch::WithoutItem => {
                            self.trigger_default_block_use(hit.pos);
                        }
                    }
                    self.swing_arm();
                    return Ok(true);
                }
            }
        }
        let Some(stack) = self.held().cloned() else {
            return Ok(false);
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return Ok(false);
        };
        // ServerPlayerGameMode.useItem and useItemOn: an item on cooldown is
        // not used (PASS, so the other hand may still act).
        if self
            .cooldowns
            .is_on_cooldown(&horizon_client::cooldowns::group(
                &stack,
                info.use_cooldown.as_ref(),
            ))
        {
            return Ok(false);
        }
        // Native mods, where ItemStack.useOn and Item.use start: the block
        // aimed at had its turn, and the item is off cooldown. A guest's
        // UseItemOn reaches only useOn, its UseItem only use.
        let on_block = matches!(self.guest_use, Some(GuestUse::OnBlock(_)));
        let mut claimed = false;
        if let Some(hit) = block_hit {
            self.native_use_item_on(&stack, hit.pos, hit.face, &mut claimed);
        }
        if !claimed && !on_block {
            self.native_use_item(&stack, &mut claimed);
        }
        if claimed {
            self.swing_arm();
            return Ok(true);
        }
        // ItemStack.useOn: after the block's own useItemOn/useWithoutItem
        // passed. DebugStickItem.useOn cycles the selected property.
        if let Some(hit) = block_hit
            && let Some(success) = self.debug_stick_use_on(hit.pos)?
        {
            // `Minecraft.startUseItem`: the stick's client-side `useOn` is
            // SUCCESS, so the actor's hand swings; a remote guest swings in
            // its own client, not on the server.
            if self.lan_acting.is_none() {
                self.swing_arm();
            }
            if success {
                // `ServerPlayerGameMode.useItemOn`: a consuming `useOn`
                // triggers ITEM_USED_ON_BLOCK with the stack before the use.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, &stack);
            } else {
                // DebugStickItem.useOn's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        }
        if block_hit.is_some() && (self.use_bone_meal(&stack)? || self.ignite_tnt(&stack)?) {
            return Ok(true);
        }
        // ShearsItem.useOn: shears crop a growing plant's head to its top age.
        if let Some(hit) = block_hit
            && self.shears_crop(hit.pos, &stack)?
        {
            return Ok(true);
        }
        // A guest's UseItemOn stops at ItemStack.useOn: what Item.use does
        // (throw, drink, eat, raise, equip) waits for its UseItem.
        let spawn_egg = stack.item.ends_with("_spawn_egg");
        use horizon_client::first_person::UseResult;
        let swings = self.swings_started;
        match self.use_tool(&stack, block_hit.is_some())? {
            UseResult::Success { swing } => {
                if (UseResult::Success { swing }).should_swing(swings, self.swings_started) {
                    self.swing_arm();
                }
                return Ok(true);
            }
            UseResult::Fail => {
                self.use_failed = true;
                return Ok(true);
            }
            UseResult::Pass => {}
        }
        // PotionItem.useOn comes before PotionItem.use's drink.
        if let Some(hit) = block_hit
            && self.use_water_bottle_on_block(hit.pos, hit.face, &stack)?
        {
            return Ok(true);
        }
        // a mod's item used over time: its Item.use, once a block item's
        // useOn had its turn.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_mod_use(&stack)
        {
            return Ok(true);
        }
        if (!on_block && self.use_potion_item(&stack)?)
            || ((block_hit.is_some() || !spawn_egg)
                && (!on_block || spawn_egg)
                && self.throw_item(&stack)?)
        {
            return Ok(true);
        }
        // BrushItem.useOn: brushing starts on the block aimed at (CONSUME).
        if block_hit.is_some() && &*stack.item == "minecraft:brush" {
            self.start_using_item();
            return Ok(true);
        }
        if !on_block
            && (self.raise_shield(self.held_slot())
                || self.use_fishing_rod(&stack)
            || self.use_crossbow(&stack)
            || self.use_trident(&stack)
            || self.use_spear(&stack)
            || self.boost_firework(&stack)
            || self.use_empty_map(&stack)
                || self.use_book(&stack)
                // KnowledgeBookItem.use: the recipes it names become known.
                || self.use_knowledge_book(&stack)
                // Block items (heads, carved pumpkins) aimed at a block are
                // placed first (useItemOn before useItem).
                || (!(info.block.is_some() && block_hit.is_some())
                    && self.equip_from_hand(&stack)))
        {
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:spyglass" {
            self.use_spyglass();
            return Ok(true);
        }
        // BundleItem.use: startUsingItem, then onUseTick empties it.
        if !on_block && info.bundle {
            self.start_using_item();
            self.swing_arm();
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:goat_horn" {
            self.use_instrument(&stack);
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:bow" {
            if self.creative || self.projectile_slot("minecraft:bow").is_some() {
                self.start_using_item();
            }
            return Ok(true);
        }
        // Block foods attempt placement before starting consumption.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_eating(&info) {
            return Ok(true);
        }
        // BoatItem.use also performs its own fluid/block raycast.
        if !on_block && self.place_boat(&stack) {
            return Ok(true);
        }
        if let Some(hit) = block_hit
            && (self.launch_firework(hit, &stack)
                || self.place_minecart(hit, &stack)
                || self.place_painting(hit, &stack)
                || self.place_item_frame(hit, &stack)
                || self.use_lodestone_compass(hit, &stack)
                || self.use_map_on_banner(hit, &stack)
                || self.place_armor_stand(hit, &stack)
                || self.place_cushion(hit, &stack)
                || self.place_end_crystal(hit, &stack))
        {
            return Ok(true);
        }
        if self.use_ender_eye(&stack, block_hit.is_some())? {
            return Ok(true);
        }
        // PlaceOnWaterBlockItem.use: the lily pad and frogspawn aim at the block
        // or water source the SOURCE_ONLY ray meets and are placed on the
        // position above it.
        let water_hit;
        let hit = if matches!(&*stack.item, "minecraft:lily_pad" | "minecraft:frogspawn") {
            let Some(h) = self.pick_fluid() else {
                return Ok(false);
            };
            water_hit = Hit {
                pos: [h.pos[0], h.pos[1] + 1, h.pos[2]],
                ..h
            };
            &water_hit
        } else {
            let Some(hit) = block_hit else {
                return Ok(false);
            };
            hit
        };
        if info.block.is_none() {
            return Ok(false);
        }
        if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
            // ItemStack.useOn passes in adventure mode; useItem eats.
            return Ok(!on_block && self.start_eating(&info));
        }
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        // BlockItem.place through the vanilla placement rules.
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face,
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: false,
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.keys.contains(&KeyCode::ShiftLeft),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        let outcome = self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx));
        let PlaceOutcome::Placed { pos, state } = outcome else {
            self.process_level_events()?;
            // BlockItem.useOn: a consumable that could not be placed is used,
            // which ItemStack.useOn counts as a use of the item.
            if self.start_eating(&info) {
                self.stats.add("used", &stack.item, 1);
            } else {
                // BlockItem.place's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        };
        let target = [pos.x, pos.y, pos.z];
        let state = self.apply_item_block_state(target, state, &stack)?;
        let name = self.res.facts().get(state).name.clone();
        if let Some(entity) = self.new_block_entity(&name) {
            let key = (target[0], target[1], target[2]);
            self.world.block_entities.insert(key, entity);
            let game_master = self.can_use_game_master_blocks();
            self.apply_placed_item(target, &stack, game_master);
            // Block.setPlacedBy.
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => {
                    let data = stack.data().is_some_and(|d| {
                        d.other(horizon_client::block_entity_item::BLOCK_ENTITY_DATA)
                            .is_some()
                    });
                    self.command_block_placed(key, !data);
                }
                Some(BlockEntity::StructureBlock(_)) => self.structure_block_placed(key),
                Some(BlockEntity::JigsawBlock(_)) => self.jigsaw_block_placed(key),
                Some(BlockEntity::TestBlock(_)) => self.test_block_placed(key),
                Some(BlockEntity::Sign(sign)) if !sign.waxed => self.open_sign_editor(key, true),
                _ => {}
            }
        }
        self.swing_arm();
        // BlockItem.place's sound is among the level events;
        // SolidBucketItem.getPlaceSound replaces it with the bucket's own.
        let solid_bucket = &*stack.item == "minecraft:powder_snow_bucket";
        if solid_bucket {
            for event in &mut self.level_events {
                if let LevelEvent::Sound { pos: at, sound, .. } = event
                    && *at == pos
                {
                    *sound = "minecraft:item.bucket.empty_powder_snow".into();
                }
            }
        }
        self.process_level_events()?;
        // BlockItem.place: the used statistic and the placed_block trigger.
        self.stats.add("used", &stack.item, 1);
        self.trigger_block("minecraft:placed_block", target, &stack);
        self.native_place_block(&stack, target, state);
        self.check_wither_spawn(target)?;
        self.check_golem_spawn(target)?;
        self.consume_held();
        // SolidBucketItem.useOn: BucketItem.getEmptySuccessItem in hand.
        if solid_bucket && !self.creative {
            *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
        }
        Ok(true)
    }
    /// The player as a block's `useItemOn` / `useWithoutItem` sees them.
    fn use_context(&self) -> PlayerContext {
        PlayerContext {
            yaw: self.yaw,
            pitch: self.pitch,
            may_build: self.may_build(),
            can_eat: self.creative
                || self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.food.food < 20),
            game_master: self.can_use_game_master_blocks(),
        }
    }
    /// `Consumable.startConsuming` for a held consumable: food needs
    /// `Player.canEat` (`Consumable.canConsume`), anything else is always
    /// consumed. The right click is held for `consume_seconds`.
    fn start_eating(&mut self, info: &horizon_client::items::ItemInfo) -> bool {
        let Some(stack) = self.held().cloned() else {
            return false;
        };
        if info.consumable.is_none()
            || (info.food.is_some()
                && (self.player.is_none() || !self.can_eat(&stack, info.can_always_eat)))
        {
            return false;
        }
        // as Consumable.startConsuming does, no time is eaten at once, without a use
        // (the server's eating reaches a client through the inventory).
        if let Some(consumable) = info.consumable.as_ref()
            && item_use::consumes_at_once(consumable.ticks())
        {
            if self.remote.is_none() {
                // the cooldown of ItemStack.use's applyAfterUseComponentSideEffects.
                self.apply_use_cooldown(&stack);
                self.finish_consuming(info, consumable, &stack);
            }
            return true;
        }
        self.start_using_item();
        true
    }
    /// `InstrumentItem.use` for a goat horn: the player holds it up for the
    /// instrument's `use_duration`, its sound plays at the player
    /// (`SoundSource.RECORDS`, volume `range / 16`), `INSTRUMENT_PLAY` is
    /// posted, the used statistic counts and the horn is on cooldown as long.
    /// A horn whose instrument is unknown fails, which still ends the click.
    fn use_instrument(&mut self, stack: &ItemStack) {
        let Some(instrument) = self.res.items().instrument(stack) else {
            return;
        };
        self.start_using_item();
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        self.audio
            .play(&instrument.sound, at, instrument.range / 16.0, 1.0);
        self.post_entity_event(
            "minecraft:instrument_play",
            horizon_client::mobs::Who::Player,
        );
        if instrument.use_duration > 0.0 {
            let group = horizon_client::cooldowns::group(stack, None);
            self.add_cooldown(group, instrument.use_ticks());
        }
        self.stats.add("used", &stack.item, 1);
    }
    fn apply_consume_effect(&mut self, effect: &horizon_client::items::ConsumeEffect) {
        use horizon_client::items::ConsumeEffect;
        match effect {
            ConsumeEffect::ApplyEffects {
                effects,
                probability,
            } => {
                // ApplyStatusEffectsConsumeEffect.apply: the user's random
                // (the level's stands in, as for every player draw) is drawn
                // even at probability 1.
                if self.world.random.next_float() < *probability {
                    for eff in effects {
                        if let Some(def) = horizon_client::brewing::effect(&eff.id) {
                            if def.instant {
                                if let Some(a) =
                                    horizon_client::effects::instant(def.id, eff.amplifier, 1.0)
                                {
                                    self.apply_effect_action(a);
                                }
                            } else {
                                self.add_player_effect(
                                    horizon_client::effects::EffectInstance::new(
                                        def.id,
                                        eff.duration,
                                        eff.amplifier,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            ConsumeEffect::ClearAllEffects => {
                self.potions.effects.clear();
                self.apply_effect_status();
            }
            ConsumeEffect::RemoveEffects(to_remove) => {
                for target in to_remove {
                    let id = target.trim_start_matches("minecraft:");
                    self.potions.effects.list.retain(|e| e.id != id);
                }
                self.apply_effect_status();
            }
            ConsumeEffect::TeleportRandomly(teleport) => self.teleport_randomly(teleport),
            ConsumeEffect::PlaySound(sound) => {
                // PlaySoundConsumeEffect: at the user's block position.
                let at = self.player.as_ref().map(|p| {
                    let b = horizon_core::position::BlockPos::containing(p.position);
                    [b.x, b.y, b.z].map(|v| f64::from(v) + 0.5)
                });
                self.audio.play(sound, at, 1.0, 1.0);
            }
            // A mod's own kind: the owning mod's hook answers.
            ConsumeEffect::Native { kind, data } => {
                self.native_consume_effect(kind, data);
            }
        }
    }
    fn consume_held(&mut self) {
        if self.creative {
            return;
        }
        let cell = self.held_mut();
        if let Some(s) = cell.as_mut() {
            s.count -= 1;
            if s.count == 0 {
                *cell = None;
            }
        }
    }
    fn tick_using(&mut self) {
        let Some(hand) = self.used_hand() else {
            return;
        };
        self.with_hand(hand, Self::tick_used_item);
    }
    /// `LivingEntity.updatingUsingItem` for the item in use, its hand acting.
    fn tick_used_item(&mut self) {
        let Some((slot, ticks)) = self.using else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            self.using = None;
            return;
        };
        // Switching hotbar slots only ends a main-hand use.
        let hand_changed = slot != self.selected && slot != shield::OFFHAND_SLOT;
        if !self.right || hand_changed || self.screen != Screen::Playing {
            self.using = None;
            self.release_used_item(&stack, ticks, hand_changed);
            return;
        }
        // ServerPlayer.updateUsingItem: USING_ITEM every tick of the use.
        self.trigger_using_item(&stack);
        let Some(info) = self.items().get(&stack.item).cloned() else {
            self.using = None;
            return;
        };
        if self.blocks_attacks(&stack).is_some() {
            // ShieldItem: used for 72000 ticks, i.e. until released.
            self.using = Some((slot, ticks + 1));
            return;
        }
        if self.tick_drinking(slot, ticks, &stack) {
            if self.using.is_none() {
                self.use_item_finished(&stack);
            }
            return;
        }
        if &*stack.item == "minecraft:bow" {
            self.using = Some((slot, ticks + 1));
            return;
        }
        if &*stack.item == "minecraft:crossbow" {
            self.tick_crossbow(slot, ticks, &stack);
            return;
        }
        // TridentItem and EnderEyeItem: used until released (an eye's
        // getUseDuration 0 never counts down to finishUsingItem).
        if matches!(&*stack.item, "minecraft:trident" | "minecraft:ender_eye") {
            self.using = Some((slot, ticks + 1));
            return;
        }
        // ItemStack.onUseTick: a spear stabs what its speed allows, until released.
        if horizon_client::spear::Kinetic::of(&stack.item).is_some() {
            self.spear_tick(&stack, ticks);
            self.using = Some((slot, ticks + 1));
            return;
        }
        if &*stack.item == "minecraft:spyglass" {
            // SpyglassItem.USE_DURATION, then finishUsingItem.
            if ticks + 1 >= 1200 {
                self.send_use_item_complete();
                self.stop_spyglass();
                self.using = None;
            } else {
                self.using = Some((slot, ticks + 1));
            }
            return;
        }
        if let Some(duration) = self
            .res
            .items()
            .instrument(&stack)
            .map(|i| i.use_ticks().max(0) as u32)
        {
            // InstrumentItem.getUseDuration: held up that long, nothing more.
            self.using = (ticks + 1 < duration).then_some((slot, ticks + 1));
            return;
        }
        if &*stack.item == "minecraft:brush" {
            self.tick_brush(slot, ticks);
            return;
        }
        if info.bundle {
            self.tick_bundle(slot, ticks);
            return;
        }
        if self.mod_use(&stack) {
            self.tick_mod_use(slot, ticks, &stack);
            return;
        }
        let Some(consumable) = info.consumable.as_ref() else {
            self.using = None;
            return;
        };
        // ItemStack.onUseTick: Consumable.shouldEmitParticlesAndSounds with
        // the remaining ticks before this tick counts them down.
        // the duration is getUseDuration's when the use started.
        let remaining = self.use_duration - ticks as i32;
        let done = item_use::count_down(ticks, self.use_duration).is_none();
        let ticks = ticks + 1;
        if consumable.should_emit(remaining) {
            self.emit_consume(consumable, &stack, 5);
        }
        if done {
            self.using = None;
            self.send_use_item_complete();
            // ItemStack.finishUsingItem's applyAfterUseComponentSideEffects.
            self.apply_use_cooldown(&stack);
            self.finish_consuming(&info, consumable, &stack);
            self.use_item_finished(&stack);
        } else {
            self.using = Some((slot, ticks));
        }
    }
    /// `BundleItem.onUseTick` and `getUseDuration` (200): the first tick and
    /// then every second tick from the 11th on drop one stack out of the
    /// bundle in `slot`; the use ends after 200 ticks.
    fn tick_bundle(&mut self, slot: usize, ticks: u32) {
        const USE_DURATION: u32 = 200;
        let remaining = USE_DURATION - ticks.min(USE_DURATION);
        if ticks == 0 || (remaining < USE_DURATION - 10 && remaining.is_multiple_of(2)) {
            self.drop_bundle_content(slot);
        }
        self.using = (ticks + 1 < USE_DURATION).then_some((slot, ticks + 1));
    }
    /// `BundleItem.dropContent`: `removeOneItemFromBundle` and `Player.drop`,
    /// then the drop sound and the used statistic.
    fn drop_bundle_content(&mut self, slot: usize) {
        let items = self.items();
        let Some(mut bundle) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(mut contents) = horizon_client::bundle::Contents::of(items, &bundle) else {
            return;
        };
        let Some(removed) = contents.remove_one(items) else {
            return;
        };
        contents.store(&mut bundle);
        let item = Arc::clone(&bundle.item);
        *self.stack_in_mut(slot) = Some(bundle);
        let Some(position) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let at = [position.x, position.y, position.z];
        // playRemoveOneSound, then Player.drop and playDropContentsSound at
        // the player's block.
        let pitch = 0.8 + self.world.random.next_float() * 0.4;
        self.audio
            .play(horizon_client::bundle::REMOVE_ONE, Some(at), 0.8, pitch);
        self.player_drop(removed);
        let block = BlockPos::containing(position);
        let pitch = 0.8 + self.world.random.next_float() * 0.4;
        self.audio.play(
            horizon_client::bundle::DROP_CONTENTS,
            Some([block.x, block.y, block.z].map(|v| f64::from(v) + 0.5)),
            0.8,
            pitch,
        );
        self.stats.add("used", &item, 1);
    }
    /// `LivingEntity.releaseUsingItem` (`Item.releaseUsing`) for `stack`,
    /// used for `ticks`, its hand acting: a bow or a trident lets go (not
    /// when its hand changed), a spyglass stops scoping.
    pub(super) fn release_used_item(&mut self, stack: &ItemStack, ticks: u32, hand_changed: bool) {
        match &*stack.item {
            "minecraft:bow" if !hand_changed => self.release_bow(ticks),
            "minecraft:trident" if !hand_changed => self.release_trident(ticks),
            "minecraft:spyglass" => self.stop_spyglass(),
            _ => {}
        }
    }
    /// `ServerPlayerGameMode.tick` while a player digs: level event 2020 every
    /// fourth tick since the START (the hit sound), else 2019 (the crack only),
    /// for everyone around, the digger included (`levelEvent(null, ...)`).
    fn destroy_progress_event(&mut self, pos: [i32; 3], face: [i32; 3], ticks: u32) {
        let id = if ticks.is_multiple_of(4) { 2020 } else { 2019 };
        let direction = horizon_core::geometry::Direction::ALL
            .iter()
            .position(|d| d.normal() == face)
            .unwrap_or(1);
        self.level_event_at(id, pos, direction as i32);
    }
    fn tick_mining(&mut self) -> Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
        }
        if !self.left || self.screen != Screen::Playing || self.dialog_open() {
            self.mining = None;
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return Ok(());
        };
        // Player.blockActionRestricted: spectators, and adventure players
        // without a matching can_break item.
        if self.block_action_restricted(hit.pos)
            || !self
                .res
                .items()
                .can_destroy_block(self.held(), self.creative)
        {
            self.mining = None;
            return Ok(());
        }
        if self.creative {
            if self.destroy_delay == 0 {
                self.break_block(hit.pos)?;
                self.destroy_delay = 5;
            }
            return Ok(());
        }
        if self.destroy_delay > 0 {
            return Ok(());
        }
        if self.mining.as_ref().is_none_or(|m| m.pos != hit.pos) {
            self.attack_block(hit.pos)?;
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                ticks: 0,
            });
        }
        let block_break_speed = self.block_break_speed();
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts().get(state);
        if facts.hardness < 0.0 {
            // The server still counts its destroy ticks on an unbreakable block.
            let m = self.mining.as_mut().ok_or("mining")?;
            m.ticks += 1;
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
            return Ok(());
        }
        // Player.getDestroySpeed / BlockBehaviour.getDestroyProgress.
        let held = self.held();
        let mut speed = self.items().mining_speed(held, &facts.name);
        speed = self.modify_destroy_speed(speed);
        speed *= block_break_speed;
        let player = self.player.as_ref().ok_or("no player")?;
        if !player.ground.on_ground {
            speed /= 5.0;
        }
        let eye = self.eye_position().unwrap_or_default();
        if self
            .res
            .facts()
            .get(self.world.block(
                eye[0].floor() as i32,
                eye[1].floor() as i32,
                eye[2].floor() as i32,
            ))
            .fluid
            == Fluid::Water
        {
            speed *= self.submerged_mining_speed();
        }
        let harvest = !facts.requires_tool || self.items().correct_for_drops(held, &facts.name);
        let progress = if facts.hardness == 0.0 {
            1.0
        } else {
            speed / facts.hardness / if harvest { 30.0 } else { 100.0 }
        };
        self.swing_arm();
        let m = self.mining.as_mut().ok_or("mining")?;
        m.progress += progress;
        m.ticks += 1;
        // An instant break at START sends no destroy events.
        if m.progress < 1.0 || m.ticks > 1 {
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
        }
        let m = self.mining.as_mut().ok_or("mining")?;
        // ServerPlayerGameMode breaks a player's block at once only when it is instant (START's
        // insta-mine); otherwise it waits for the client's STOP_DESTROY_BLOCK
        // (`guest_stop_digging`), never breaking it on its own count. Breaking it here would
        // take the block from under the player's crack before the player's client predicts the
        // break, so it gets neither particles nor sound.
        if m.progress >= 1.0 && (self.lan_acting.is_none() || m.ticks == 1) {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        Ok(())
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        if self.dim == Dim::Nether {
            // Below the bedrock ceiling: a solid floor with two air blocks above.
            for y in (32..120).rev() {
                let floor = self.res.facts().get(self.world.block(x, y - 1, z));
                if floor.full_collision
                    && floor.fluid == Fluid::None
                    && self.res.facts().is_air(self.world.block(x, y, z))
                    && self.res.facts().is_air(self.world.block(x, y + 1, z))
                {
                    return Some(y);
                }
            }
            return None;
        }
        let mut y = self.world.top_y() - 1;
        while y > self.world.bottom_y() {
            let s = self.world.block(x, y, z);
            if !self.res.facts().is_air(s) {
                let f = self.res.facts().get(s);
                let ok = f.full_collision
                    && f.fluid == Fluid::None
                    && !f.name.contains("leaves")
                    && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    fn spawn_player(&mut self) -> Result<()> {
        if self.player.is_some() || self.remote.is_some() {
            return Ok(());
        }
        if let Some((_, target)) = &self.travelling {
            if self.win_screen_holds_arrival() {
                return Ok(());
            }
            let (cx, cz) = (
                (target.x.floor() as i32) >> 4,
                (target.z.floor() as i32) >> 4,
            );
            self.streaming.center = (cx, cz);
            if (-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                let (player, target) = self.travelling.take().ok_or("travel")?;
                if let Some(config) = self.respawn_travel.take() {
                    drop(player);
                    self.place_respawned(config)?;
                } else if self.end_travel {
                    self.place_after_end_travel(player, target)?;
                } else {
                    self.place_after_travel(player, target)?;
                }
            }
            return Ok(());
        }
        let saved = self.level.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        if self.spawn.is_none() {
            self.spawn = self
                .level
                .spawn
                .map(|s| Vec3::new(s[0] + 0.5, s[1], s[2] + 0.5));
        }
        let feet = if let Some(p) = saved {
            let (cx, cz) = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
            self.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                return Ok(());
            }
            self.level.position = None;
            p
        } else if self.spawn.is_none() || self.player_spawn.is_some() {
            // A new world: its spawn, then the first player around it.
            if self.spawn.is_none() {
                let Some([x, y, z]) = self.find_world_spawn()? else {
                    return Ok(());
                };
                self.spawn = Some(Vec3::new(
                    f64::from(x) + 0.5,
                    f64::from(y),
                    f64::from(z) + 0.5,
                ));
                self.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
                // The world's spawn from the start, not only once saved: a bed set
                // before the first save must not become it.
                self.level.world_spawn.get_or_insert([x, y, z]);
                // The "Bonus Chest" create-world option, placed once with the spawn.
                if horizon_client::save::WorldGen::read(&self.world_root)
                    .is_some_and(|g| g.bonus_chest)
                {
                    self.place_bonus_chest([x, y, z])?;
                }
            }
            let s = self
                .level
                .spawn
                .unwrap_or_default()
                .map(|v| v.floor() as i32);
            let Some(feet) = self.find_player_spawn(s)? else {
                return Ok(());
            };
            feet
        } else {
            let s = self.spawn.unwrap_or_default();
            let (cx, cz) = ((s.x.floor() as i32) >> 4, (s.z.floor() as i32) >> 4);
            self.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                return Ok(());
            }
            let column = [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32];
            if self.level.world_spawn == Some(column) {
                // TeleportTransition.missingRespawnBlock: PlayerSpawnFinder
                // around the world spawn, not its exact column; its candidates'
                // chunks load first.
                let Some(feet) = self.find_player_spawn(column)? else {
                    return Ok(());
                };
                feet
            } else {
                // A spawnpoint: the stored column, lifted onto its current surface.
                let y = self
                    .land_column(column[0], column[2])
                    .map_or(s.y, f64::from);
                Vec3::new(s.x, y.max(s.y), s.z)
            }
        };
        let mut player = PlayerMotion::new(self.motion, feet)?;
        player.yaw = self.yaw;
        let entering = self.restore_player_state;
        player.load_saved_state_once(&mut self.restore_player_state, &self.level)?;
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        if spectator {
            player.flying = true;
        } else if !player.may_fly {
            player.flying = false;
        }
        player.respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        player.pitch = self.pitch;
        player.glider = self.glider_worn();
        // A saved death already dropped inventory/XP and counted the death.
        if player.health.dead_or_dying() {
            self.screen = Screen::Dead;
            self.death_focus = None;
            self.ui.died_at = Some(Instant::now());
            self.ui.death_text = None;
            self.ui.death_confirm = None;
            self.capture(false);
        }
        self.previous_position = feet;
        if entering {
            // The world's first player: a server's client gets it with its
            // level (Minecraft.setLevel), which stops every sound.
            self.stop_sounds_for_new_level();
        }
        self.player = Some(player);
        self.pad.player_replaced();
        self.sway = Default::default();
        self.apply_equipment();
        self.eye = 1.62;
        self.previous_eye = 1.62;
        self.clock.reset();
        self.capture_loaded_world();
        Ok(())
    }
    /// Landing rules (farmland trampling, turtle eggs) wrote or removed the block
    /// raw: finish `Level.setBlock`'s neighbour updates, so a crop on trampled
    /// farmland breaks. The local player's and every server player's landings.
    fn landing_block_updates(&mut self, event: &horizon_client::landing::FallEvent) {
        use horizon_client::landing::FallEvent;
        let (position, flags) = match *event {
            FallEvent::Write {
                position, flags, ..
            } => (position, flags as i32),
            FallEvent::Destroy { position, .. } => (position, level_flags::UPDATE_ALL),
            _ => return,
        };
        self.with_level(|b, level, ticks| b.notify_written(level, ticks, position, flags));
    }
    fn tick(&mut self) -> Result<()> {
        if self.agent.hold_server {
            return Ok(());
        }
        // `Player.entityInteractionRange`/`blockInteractionRange` for the tick's
        // picks: the attribute carries creative's and a server's modifiers.
        self.entity_reach = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        self.block_reach = self.interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                if let Some(c) = self.carried.take() {
                    let _ = self.give(c);
                }
                self.screen = Screen::Inventory;
            }
        }
        if self.remote.is_some() {
            return self.tick_remote();
        }
        // LevelLoadingScreen.tick.
        self.loading_progress = match self.loading_reason() {
            Some(_) => horizon_client::level_loading::smooth(
                self.loading_progress,
                self.level_load_progress(),
            ),
            None => 0.0,
        };
        self.ticks += 1;
        // An in-frame LAN host's player has the entity id its guests see it by.
        if !self.serving()
            && let (Some(host), Some(p)) = (&self.lan, self.player.as_mut())
        {
            p.entity_id = host.host_entity;
        }
        self.track_experience_display();
        self.tick_enchanting_book();
        // `/tick freeze` stops the world but not the player.
        let running = match self.level_pass.running {
            Some(running) => running,
            None => self.clock.world_runs(),
        };
        // GameRenderer.tick: a wither's bar darkens the world, then fades;
        // the activated item counts down.
        self.boss_overlay.tick_darken(running);
        self.tick_item_activation();
        self.native_inventory_ticks();
        if running {
            self.world.game_time += 1;
            if self.ui.rules.bool("advance_weather") {
                self.world.weather.tick_timers(&mut self.world.random);
            }
            self.world.weather.tick_levels();
            self.tick_weather_effects();
        }
        self.autosaving = self.ticks.is_multiple_of(6000);
        if let Some(serving) = &self.serving {
            serving.set_saving(self.autosaving);
        }
        if self.autosaving {
            self.save_game_with(false)?;
        }
        if running {
            self.advance_clock();
            // `MinecraftServer.tickChildren`: `commandFunctions` runs before
            // any level ticks, so a tick function that polls for the drop a
            // block just made still finds it with `Age: 0`
            // (`EntityTickList.ensureActiveIsNotIterated` gives an entity
            // added during the iteration its first tick on the next one).
            // Vanilla's SavedData is loaded long before this point; the lazy
            // scoreboard/storage read has to be done by then too, or a load
            // tag writes over what it restores.
            self.load_scoreboard();
            self.tick_functions();
            // What the functions summoned ticks this tick, as it would in
            // vanilla; what the levels add from here waits for the next.
            self.ticked_entities_through = self.next_entity;
        }
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        self.tick_totem();
        self.update_totem_ready();
        self.tick_command_extras();
        self.native_tick();
        self.tell_native_now();
        let camera_eye = self.camera_eye_height();
        let stick = self.pad_move();
        if self.riding.is_some() && self.player.is_some() {
            let playing = self.screen == Screen::Playing && !self.dialog_open();
            let held = |k| playing && self.keys.contains(&k);
            let keys = Keys {
                forward: held(KeyCode::KeyW),
                backward: held(KeyCode::KeyS),
                left: held(KeyCode::KeyA),
                right: held(KeyCode::KeyD),
                jump: held(KeyCode::Space),
                shift: held(KeyCode::ShiftLeft),
                sprint: false,
                analog: None,
            }
            .with_stick(stick.filter(|_| playing));
            self.tick_riding(keys)?;
        }
        let riding = self.riding.is_some();
        let entity_collisions = self.player_entity_collisions();
        let spin_targets = self.spin_targets();
        let sprint_window = self.settings.sprint_window;
        self.sync_player_body();
        // Read before `player` borrows `self` mutably.
        let dialog = self.dialog_open();
        let Some(player) = self.player.as_mut() else {
            // A dedicated server has no local player: the world still runs.
            if self.options.dedicated.is_some() {
                return self.tick_world(running);
            }
            return Ok(());
        };
        // Hold position until the surrounding terrain exists.
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        if !self.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        let playing = self.screen == Screen::Playing && self.sleeping.is_none() && !dialog;
        let held = |k| playing && self.keys.contains(&k);
        let keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        self.rope_rot_old = (self.yaw, self.pitch);
        if !riding {
            self.previous_position = player.position;
        }
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        player.spin_targets = spin_targets;
        let grounded = player.ground.on_ground;
        let before = (
            grounded,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        if !riding {
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .controlled_simulation_tick_with_sprint_window(keys, sprint_window)?;
            // Player.jumpFromGround: the jump statistic.
            if grounded && keys.jump && !player.flying && player.velocity.y > 0.2 {
                self.stats.custom("jump", 1);
            }
        }
        let events = std::mem::take(&mut player.events);
        let spin_hit = player.spin_hit.take();
        let glider_wear = std::mem::take(&mut player.glider_wear);
        let feet = [player.position.x, player.position.y, player.position.z];
        let player_velocity = [player.velocity.x, player.velocity.y, player.velocity.z];
        self.previous_eye = self.eye;
        let eye_height = if self.sleeping.is_some() {
            0.2
        } else {
            player.dimensions.eye_height
        };
        self.eye += (camera_eye.unwrap_or(eye_height) - self.eye) * 0.5;
        // `LivingEntity.baseTick` (Soul Speed) comes before `aiStep` (the
        // glide's wear): both draw from the level random.
        if let Some((at, velocity)) = self.tick_soul_speed() {
            self.particles.send_particles(
                &horizon_client::particles::ParticleOptions::simple(
                    horizon_client::particles::ParticleType::Soul,
                ),
                at,
                velocity,
                1.0,
                0,
            );
            if let Some(host) = self.lan.as_ref() {
                lan_particles::send_soul_particle(host, at, velocity);
            }
        }
        // `ItemStack.hurtAndBreak` for the glide: Unbreaking can save the wear.
        if glider_wear > 0
            && !self.creative
            && let Some(stack) = self.armor[1].clone()
        {
            let loss = self.wear(&stack, glider_wear);
            if let Some(elytra) = self.armor[1].as_mut() {
                elytra.damage += loss;
            }
        }
        self.tick_avatar(before, riding);
        self.end_spin_attack(spin_hit);
        self.record_combat(&events);
        self.wear_armor(&events);
        for event in events {
            match event {
                MotionEvent::HurtSound(sound) => self.player_hurt_sound(sound, feet),
                MotionEvent::Emission { state, muffled, .. } => {
                    let sounds = self.res.facts().get(state).sounds.clone();
                    // Entity.playStepSound: the block's step sound at 0.15 volume,
                    // muffled under the swim sound in water. Whether it plays is
                    // Player.getMovementEmission's call (shift on the ground),
                    // not the pose: a forced crouch still steps.
                    let (volume, pitch) = if muffled {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.audio.play(&sounds.step, Some(feet), volume, pitch);
                }
                MotionEvent::Swim { volume, pitch } => {
                    self.audio
                        .play("minecraft:entity.player.swim", Some(feet), volume, pitch);
                }
                MotionEvent::Chime { volume, pitch } => {
                    self.audio.play(
                        "minecraft:block.amethyst_block.chime",
                        Some(feet),
                        volume,
                        pitch,
                    );
                }
                MotionEvent::Drown => {
                    // The guests see the entity event 67 too.
                    if let Some(host) = &self.lan {
                        host.hub
                            .entity_event(host.host_entity, combat::DROWN_EVENT, feet);
                    }
                    self.drown_particles(feet, player_velocity);
                }
                MotionEvent::BubbleColumnParticle { bubble, position } => {
                    // ServerLevel.sendParticles: SPLASH (speed 1) or BUBBLE
                    // (y spread 0.01, speed 0.2), one each.
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let (kind, spread, speed) = if bubble {
                        (ParticleType::Bubble, [0.0, 0.01, 0.0], 0.2)
                    } else {
                        (ParticleType::Splash, [0.0; 3], 1.0)
                    };
                    self.particles.send_particles(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        spread,
                        speed,
                        1,
                    );
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    let Some(name) = sound.player_fall_sound() else {
                        continue;
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                // Player.causeFallDamage: a fall of 2 blocks or more counts.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Statistic(cm)) => {
                    self.stats.custom("fall_one_cm", cm)
                }
                // LivingEntity.checkFallDamage: the block's crumbs where the fall landed.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Particles {
                    state,
                    position,
                    count,
                    speed,
                }) => self.landing_dust(None, state, position, count, speed),
                MotionEvent::Fall(event) => self.landing_block_updates(&event),
                MotionEvent::Snowflake { position, velocity } => self.snowflake(position, velocity),
                MotionEvent::EnterBlock { state } => self.trigger_enter_block(state),
                MotionEvent::HoneySlide {
                    pos,
                    sound,
                    particles,
                } => {
                    if self.world.game_time % 20 == 0 {
                        self.trigger_honey_slide([pos.x, pos.y, pos.z]);
                    }
                    if sound {
                        self.audio
                            .play("minecraft:block.honey_block.slide", Some(feet), 1.0, 1.0);
                    }
                    if particles {
                        self.honey_slide_particles(feet);
                        // The server's event 53 reaches the guests too.
                        let id = self.lan.as_ref().map(|h| h.host_entity);
                        if let Some(id) = id {
                            self.lan_entity_event(id, 53, feet);
                        }
                    }
                }
                MotionEvent::InsidePortal { pos } => self.player_inside_nether_portal(pos),
                MotionEvent::CauldronExtinguish { pos } => self.cauldron_extinguish(pos),
                MotionEvent::InsideWitherRose => self.wither_rose_contact(),
                // Entity.restituteMovementAfterCollisions: GameEvent.BOUNCE.
                MotionEvent::Bounce { .. } => {
                    self.post_entity_event("minecraft:bounce", horizon_client::mobs::Who::Player)
                }
                MotionEvent::SmashLanding {
                    state,
                    position,
                    fall_distance,
                } => self.smash_landing_particles(state, position, fall_distance),
                MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                _ => {}
            }
        }
        self.powder_snow_fire()?;
        self.rain_extinguish_in_frame();
        // LivingEntity.baseTick: a new block position runs the worn
        // enchantments' location_changed effects (frost walker) on the server.
        if self.remote.is_none() {
            self.location_changed_effects(before.2, before.0);
        }
        self.tick_totem();
        self.handle_shoulder_entities();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if player.position.y < f64::from(self.world.bottom_y() - 64) {
            // Out of the world.
            let _ = player.set_health(0.0, false);
        }
        let dead = player.health.dead_or_dying();
        if dead && self.screen != Screen::Dead {
            // Inventory.dropAll: every stack thrown in a random direction.
            let feet = [player.position.x, player.position.y, player.position.z];
            let at = [feet[0], feet[1] + 1.32, feet[2]];
            let killer = self.combat_killer_kind();
            // Player.die: the dimension and block of the death.
            self.level.last_death =
                Some((self.dim.id().to_owned(), feet.map(|v| v.floor() as i32)));
            self.death_message();
            // The mobs bound to a guest see that guest as their player.
            let bindings = match self.lan.as_ref() {
                Some(host) if !host.hub.guests.is_empty() => host.mob_players.clone(),
                _ => Default::default(),
            };
            self.tell_neutral_mobs_player_died(feet, None, &bindings);
            // ServerPlayer.die: deaths, and the time-since counters restart.
            self.stats.custom("deaths", 1);
            self.score_player_died();
            if let Some(kind) = killer {
                self.stats.add("killed_by", &kind, 1);
                let blow = self.combat_killing_blow();
                let view = blow.as_ref().and_then(|b| b.source.clone()).unwrap_or(
                    horizon_client::advancements::EntityView {
                        kind,
                        ..Default::default()
                    },
                );
                self.trigger_killed_as("minecraft:entity_killed_player", &view, blow.as_ref());
            }
            self.stats.reset("custom", "minecraft:time_since_death");
            self.stats.reset("custom", "minecraft:time_since_rest");
            let reward = self
                .experience
                .death_reward(self.ui.rules.bool("keep_inventory") || self.spectator());
            self.spawn_orbs(feet, reward);
            if !self.spectator() {
                self.drop_inventory(at);
            }
            self.audio
                .play("minecraft:entity.player.death", Some(at), 1.0, 1.0);
            // LivingEntity.die: the LAN guests see the host's death animation.
            if let Some(host) = &self.lan {
                host.hub.entity_event(host.host_entity, 3, feet);
            }
            if self.ui.rules.bool("immediate_respawn") {
                // Gui.setScreen: without the death screen the player respawns at once.
                self.respawn_player()?;
            } else {
                self.screen = Screen::Dead;
                self.death_focus = None;
                self.capture(false);
                self.ui.death_time = 0;
            }
        } else if dead {
            // LivingEntity.tickDeath: 20 ticks after dying the player is
            // removed, and Player.remove's inventoryMenu.removed drops what
            // the cursor held.
            self.ui.death_time += 1;
            if self.ui.death_time >= 20
                && let Some(stack) = self.carried.take()
                && let Some(eye) = self.eye_position()
            {
                self.throw_from_eyes(stack, eye, false);
            }
        }
        self.tick_world(running)
    }
    /// What the tick runs after the local player's own update: effects, the
    /// world's entities and blocks, and the network hub.
    fn tick_world(&mut self, running: bool) -> Result<()> {
        self.share_spawn_conditions();
        horizon_client::mobs::sulfur_cube::set_tnt_explodes(self.ui.rules.bool("tnt_explodes"));
        self.update_chunk_tickets();
        horizon_client::spans::time("tick_effects", || self.tick_effects(running));
        self.tick_sign_editor();
        // Player.tick's ItemCooldowns.tick for the window's player (a
        // server's players count theirs down in `tick_guests`).
        self.shield_cooldown = self.shield_cooldown.saturating_sub(1);
        self.cooldowns.tick();
        horizon_client::spans::time("tick_using", || self.tick_using());
        self.tick_bundle_hover();
        horizon_client::spans::time("tick_hud", || self.tick_hud());
        // The player's digging comes in with its packets, before the level
        // ticks; ServerLevel.tick then runs the scheduled and random block
        // ticks before any entity, so what they drop or spawn (dispensed
        // items, popped crops) moves on the tick it appears, as in vanilla.
        // On the server thread the window is a hub guest: drain its packets
        // here too, before falling-block schedules run (#4510).
        horizon_client::spans::time("tick_local_hand", || self.tick_local_hand())?;
        horizon_client::spans::time("postprocess_chunks", || self.postprocess_chunks())?;
        if running {
            horizon_client::spans::time("tick_lan_packets", || self.tick_lan_packets());
            horizon_client::spans::time("tick_blocks", || self.tick_blocks())?;
        }
        if running {
            horizon_client::spans::time("tick_paintings", || self.tick_paintings());
            horizon_client::spans::time("tick_item_frames", || self.tick_item_frames());
            horizon_client::spans::time("tick_armor_stands", || self.tick_armor_stands());
            horizon_client::spans::time("tick_cushions", || self.tick_cushions());
            horizon_client::spans::time("tick_boats", || self.tick_boats());
            horizon_client::spans::time("tick_carts", || self.tick_carts());
            horizon_client::spans::time("tick_thunder", || self.tick_thunder());
            horizon_client::spans::time("tick_lecterns", || self.tick_lecterns());
            horizon_client::spans::time("tick_maps", || self.tick_maps());
            self.tick_lodestone_compasses();
            horizon_client::spans::time("tick_bolts", || self.tick_bolts());
            horizon_client::spans::time("tick_fishing", || self.tick_fishing());
            horizon_client::spans::time("tick_tridents", || self.tick_tridents());
            horizon_client::spans::time("tick_rockets", || self.tick_rockets());
            horizon_client::spans::time("tick_ominous_item_spawners", || {
                self.tick_ominous_item_spawners()
            });
        }
        if running {
            horizon_client::spans::time("entity_portals", || {
                self.handle_entity_portals();
                self.handle_end_gateway_portals();
            });
            let before = self.dropped_ids();
            horizon_client::spans::time("tick_items", || self.tick_items());
            // Merged, burnt, despawned or picked up.
            self.native_items_left_since(&before);
            horizon_client::spans::time("touch_arrows", || self.touch_arrows());
        }
        horizon_client::spans::time("spawn_loaded_entities", || self.spawn_loaded_entities());
        // The ride the player's file names, once the vehicle is in the world.
        self.restore_root_vehicle();
        if running {
            horizon_client::spans::time("tick_mobs", || self.tick_mobs())?;
            horizon_client::spans::time("recheck_golem_openers", || self.recheck_golem_openers());
            let before = self.orb_ids();
            horizon_client::spans::time("tick_experience", || self.tick_experience());
            // Merged, despawned or picked up.
            self.native_orbs_left_since(&before);
            horizon_client::spans::time("entity_portals", || {
                self.find_entity_portals();
                self.resolve_arrivals();
            });
        }
        horizon_client::spans::time("tick_lan", || self.tick_lan(false));
        self.tick_friends();
        horizon_client::spans::time("tick_hand", || self.tick_hand());
        horizon_client::spans::time("tick_sleep", || self.tick_sleep());
        self.announce_sleep_status();
        self.free_left_beds();
        horizon_client::spans::time("tick_portal", || self.tick_nether_portal())?;
        if running {
            horizon_client::spans::time("tick_end", || self.tick_end())?;
            horizon_client::spans::time("end_gateways", || {
                self.find_end_gateway_portals();
                self.resolve_end_gateway_arrivals()
            })?;
            self.keep_gateway_destinations();
        }
        if running {
            horizon_client::spans::time("tick_falling", || self.tick_falling())?;
            horizon_client::spans::time("tick_tnt", || self.tick_tnt())?;
        }
        if running {
            horizon_client::spans::time("tick_block_entities", || self.tick_block_entities())?;
            horizon_client::spans::time("tick_sculk_entities", || self.tick_sculk_entities());
            horizon_client::spans::time("tick_sculk_blocks", || self.tick_sculk_blocks())?;
        }
        if running {
            // ClientLevel.tickBlockEntities (not while paused or frozen):
            // chest lids, books, shulker boxes, bells, conduits, heads.
            horizon_client::spans::time("tick_block_entity_anims", || {
                self.tick_block_entity_anims()
            });
            horizon_client::spans::time("tick_particles", || self.tick_particles());
            self.tick_client_ambient();
        }
        if running {
            // MinecraftServer.tickServer: GameTestTicker.SINGLETON.tick().
            self.tick_game_tests();
        }
        // ServerBossEvent broadcasts, after this level's entities have ticked.
        self.publish_boss_bars();
        // Minecraft.handleKeybinds: held use repeats only while no item is in
        // use (a raised shield or a drawn bow places and uses nothing).
        if self.right
            && self.use_delay == 0
            && self.using.is_none()
            && self.screen == Screen::Playing
            && !self.dialog_open()
            && self.sleeping.is_none()
        {
            self.use_with_hand()?;
            self.use_delay = 4;
        }
        Ok(())
    }
    /// Keep the held-key state of toggled sneak and sprint in step with their mapping.
    fn sync_toggle(&mut self, slot: usize) {
        let key = [KeyCode::ShiftLeft, KeyCode::ControlLeft][slot];
        if self.toggles.is_down(slot) {
            self.keys.insert(key);
        } else {
            self.keys.remove(&key);
        }
    }
    /// Let a held sneak, sprint or modifier key go when the keyboard reports
    /// its modifier up. A release does not always reach the mapping: a screen
    /// reads presses only, and an unfocused window is given no release at all,
    /// so the latch would keep the player sneaking or sprinting until the key
    /// is tapped again. The modifier state is what the keyboard itself
    /// asserts, so it settles the question. A mapping that toggles, or one a
    /// pad latched, keeps its state: `set_down(false, needs_toggle)` leaves
    /// those alone, and a toggle the player asked for is meant to outlive the
    /// key. A mapping rebound away from its modifier is left alone too: the
    /// modifier says nothing about it.
    ///
    /// Only `physical_keys` is cleared directly here; the logical `self.keys`
    /// entries for sneak and sprint are left to the toggle loop below, because
    /// they stand for the mapping and not the physical Shift or Control.
    /// `Screen.hasShiftDown` and `hasControlDown` read the physical state
    /// (vanilla's `InputConstants.isKeyDown`), so a Shift no one is holding
    /// makes every inventory click a shift-click and every throw a whole-stack
    /// drop. Those do not care what sneak and sprint are bound to. A modifier
    /// says nothing about which side is down, so one being up means both
    /// Shifts, or both Controls, are.
    fn heal_hold_latches(&mut self) {
        let (shift, control) = (self.modifiers.shift_key(), self.modifiers.control_key());
        if !shift {
            for key in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
                self.physical_keys.remove(&key);
            }
        }
        if !control {
            for key in [KeyCode::ControlLeft, KeyCode::ControlRight] {
                self.physical_keys.remove(&key);
            }
        }
        for (slot, mapping, needs_toggle) in [
            (0, "key.sneak", self.settings.toggle_crouch),
            (1, "key.sprint", self.settings.toggle_sprint),
        ] {
            let held = match horizon_client::keybinds::key_code(self.settings.keys.get(mapping)) {
                Some(KeyCode::ShiftLeft | KeyCode::ShiftRight) => shift,
                Some(KeyCode::ControlLeft | KeyCode::ControlRight) => control,
                _ => continue,
            };
            if !held {
                self.toggles.0[slot].set_down(false, needs_toggle);
                self.sync_toggle(slot);
            }
        }
    }
    /// A key event after rebinding, as the default key of its mapping (the
    /// keyboard and controllers both end here).
    fn mapped_key(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> Result<()> {
        // Sneak and sprint are ToggleKeyMappings: a press (or a key
        // repeat) flips them when toggling; only releases reach them
        // behind a screen (KeyboardHandler.keyPress).
        let toggle = match key {
            KeyCode::ShiftLeft => Some((0, self.settings.toggle_crouch)),
            KeyCode::ControlLeft => Some((1, self.settings.toggle_sprint)),
            _ => None,
        };
        if let Some((slot, option)) = toggle {
            let action = match (pressed, repeat) {
                (false, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            let screen_open =
                !gameplay_has_keyboard(self.screen, self.title.is_some(), self.dialog_open());
            self.toggles.key(slot, action, screen_open, option);
            self.sync_toggle(slot);
            if !pressed {
                return Ok(());
            }
        } else if !pressed {
            self.keys.remove(&key);
            return Ok(());
        } else {
            self.keys.insert(key);
        }
        // KeyboardHandler.keyPress: a repeat (InputConstants.REPEAT) reaches
        // Screen.keyPressed and KeyMapping.click like a press, so holding
        // key.drop keeps throwing, in game and over a menu slot.
        if repeat && key != KeyCode::KeyQ {
            return Ok(());
        }
        self.key_pressed(key)
    }
    /// `KeyMapping.releaseAll` when a screen opens and
    /// `restoreToggleStatesOnScreenClosed` when it closes.
    fn track_toggle_screens(&mut self) {
        let dialog = self.dialog_open();
        drop_held_behind_dialog(dialog, &mut self.keys, &mut self.left, &mut self.right);
        let playing = gameplay_has_keyboard(self.screen, self.title.is_some(), dialog);
        if playing == self.was_playing {
            return;
        }
        self.was_playing = playing;
        let options = [
            self.settings.toggle_crouch,
            self.settings.toggle_sprint,
            self.settings.toggle_attack,
            self.settings.toggle_use,
        ];
        if playing {
            self.toggles.screen_closed(options);
        } else {
            self.toggles.screen_opened(options);
        }
        self.sync_toggle(0);
        self.sync_toggle(1);
        self.left = self.toggles.is_down(2);
        self.right = self.toggles.is_down(3);
    }
    /// The options `MouseHandler.turnPlayer` reads.
    fn look_options(&self) -> LookOptions {
        LookOptions {
            sensitivity: f64::from(self.settings.sensitivity),
            smooth_camera: self.smooth_camera,
            scoping: self.scoping(),
            invert_x: self.settings.invert_x,
            invert_y: self.settings.invert_y,
        }
    }
    /// `MouseHandler.handleAccumulatedMovement` and `Entity.turn`, once per frame.
    /// An agent moving the mouse (`mouse-move`) counts as a grabbed pointer in
    /// an active window, as it cannot grab the real one.
    fn apply_mouse_look(&mut self) {
        let look = self.look_options();
        let time = self.started.elapsed().as_secs_f64();
        let pointer = self.captured || self.agent_pointer;
        let grabbed = pointer && self.screen == Screen::Playing && self.player.is_some();
        let active = self.focused || self.agent_pointer;
        if let Some((xo, yo)) = self
            .mouse
            .handle_accumulated_movement(time, active, grabbed, look)
        {
            self.tutorial.mouse(xo, yo);
            let zoom = self.zoom_turn_scale();
            self.turn_player(xo * zoom, yo * zoom);
        }
    }
    /// `Minecraft.pause`: a single-player game with a pausing screen open.
    fn pauses_game(&self) -> bool {
        self.has_singleplayer_server()
            && !self.is_published()
            && self.title.is_none()
            && (matches!(
                self.screen,
                Screen::Paused
                    | Screen::Options
                    | Screen::Stats
                    | Screen::Advancements
                    | Screen::WorldOptions
                    | Screen::Friends
                    | Screen::ServerLinks
                    | Screen::ConfirmLink
                    | Screen::DialogWarning
                    | Screen::CommandBlock
                    | Screen::DebugOptions
            ) || self.book.is_some()
                || self.win_screen_pauses())
    }
    /// A screen open over gameplay while `screen` stays `Playing`: a book or
    /// lectern, a sign, a server dialog, the code of conduct, a link or
    /// server pack prompt (vanilla's `Minecraft.screen` is set for each).
    fn screen_over_playing(&self) -> bool {
        self.book.is_some()
            || self.sign_edit.is_some()
            || self.dialog_open()
            || self.code_of_conduct.is_some()
            || self.link_prompt.is_some()
            || self.server_packs.prompt.is_some()
    }
    /// `Minecraft.pauseIfInactive`: a window without focus for more than 500 ms
    /// pauses the game. An open screen (books, signs, the pack prompt, a server
    /// dialog) stays instead of pausing (`Gui.setPauseScreen`).
    fn pause_if_inactive(&mut self) {
        if self
            .focus_lost
            .is_some_and(|t| t.elapsed() > Duration::from_millis(500))
            && self.control.is_none()
            && self.screen == Screen::Playing
            && self.player.is_some()
            && !self.options.no_pause
            && self.settings.file.bool("pauseOnLostFocus")
            && self.book.is_none()
            && self.sign_edit.is_none()
            && self.server_packs.prompt.is_none()
            && !self.dialog_open()
        {
            self.pause_focus = None;
            self.screen = Screen::Paused;
        }
    }
    /// `KeyMapping.setAll`: every key physically held is held again, as the
    /// mapping it is bound to, so a key pressed or kept down behind a screen
    /// counts once the game has the keyboard back. A sneak or sprint that
    /// toggles keeps its toggle state (`shouldSetOnIngameFocus`).
    fn restore_held_keys(&mut self) {
        let held: Vec<KeyCode> = self.physical_keys.iter().copied().collect();
        let toggles = [self.settings.toggle_crouch, self.settings.toggle_sprint];
        for key in held {
            for mapped in self.settings.keys.translate_all(key) {
                match mapped {
                    Ok(slot_key @ (KeyCode::ShiftLeft | KeyCode::ControlLeft)) => {
                        let slot = usize::from(slot_key == KeyCode::ControlLeft);
                        if !toggles[slot] {
                            self.toggles.0[slot].set_down(true, false);
                            self.sync_toggle(slot);
                        }
                    }
                    Ok(other) => {
                        self.keys.insert(other);
                    }
                    Err(_) => {}
                }
            }
        }
    }
    /// `MouseHandler.grabMouse` / `releaseMouse`. Both put the pointer at
    /// the window's centre, so a screen opened from gameplay (inventory,
    /// chest, pause, chat, sign, book, death) starts with the cursor in the
    /// middle instead of wherever the system last left it. Releasing when
    /// already released does nothing, so moving from screen to screen keeps
    /// the cursor where it is. Like GLFW's `glfwSetCursorPos`, nothing warps
    /// while the window is unfocused (losing focus releases the pointer).
    fn capture(&mut self, capture: bool) {
        // A guest's action (`as_guest`) opens and closes screens on the
        // guest's own client: the window's pointer stays the host's (#687).
        if self.lan_acting.is_some() {
            return;
        }
        // MouseHandler.grabMouse never captures an inactive window, and vanilla
        // grabs only on `setScreen(null)`: the pack prompt and server dialogs
        // are screens over the world that keep the pointer.
        if capture && (!self.focused || self.server_packs.prompt.is_some() || self.dialog_open()) {
            return;
        }
        // The container screen's press state belongs to the screen: a screen
        // that takes the pointer over starts with its own state, as
        // `AbstractContainerScreen` does, so a press whose release never
        // arrived (the pointer left the window) cannot wedge the next
        // screen's quick move and clicks.
        self.reset_inventory_clicks();
        if !capture {
            self.stop_mining();
        }
        // The real pointer takes over from an agent's.
        self.agent_pointer = false;
        // Splitscreen: another player has the pointer; this one's menus leave it alone.
        if !self.keyboard_mouse {
            self.captured = false;
            return;
        }
        if capture && !self.captured && !cfg!(target_os = "macos") {
            // MouseHandler.grabMouse: KeyMapping.setAll unless
            // InputQuirks.RESTORE_KEY_STATE_AFTER_MOUSE_GRAB is off (macOS).
            self.restore_held_keys();
        }
        let Some(window) = &self.window else { return };
        let size = self.renderer.as_ref().map_or_else(
            || window.inner_size().into(),
            |r| [r.config.width, r.config.height],
        );
        let [vx, vy] = self.view_at.map(f64::from);
        let centre = winit::dpi::PhysicalPosition::new(
            vx + f64::from(size[0]) / 2.0,
            vy + f64::from(size[1]) / 2.0,
        );
        let warp = |window: &Window| {
            let _ = window.set_cursor_position(centre);
        };
        let scale = HudBatch::new(size).scale;
        let was_grabbed = self.captured;
        if self.options.offscreen {
            // `--offscreen` shows no window: the system pointer stays where the
            // person at the computer has it, and the grab is the game's own (an
            // agent turns the view with `mouse-move`).
            if capture {
                self.mouse.set_ignore_first_move();
            }
            self.captured = capture;
        } else if capture {
            self.mouse.set_ignore_first_move();
            // macOS warps by re-associating the mouse with the pointer,
            // which undoes a Locked grab and leaves the pointer free: warp
            // first there. Wayland moves a pointer only while it is locked,
            // so everywhere else warp after grabbing.
            #[cfg(target_os = "macos")]
            warp(window);
            self.captured = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            #[cfg(not(target_os = "macos"))]
            if self.captured && self.focused {
                warp(window);
            }
        } else {
            if self.captured {
                // Wayland moves a pointer only as the locked pointer's
                // position hint, which the compositor applies as the lock
                // goes (sway and Hyprland do): set it before ungrabbing.
                // X11, Windows and macOS warp, so warp again once free
                // (Wayland refuses that one).
                if self.focused {
                    warp(window);
                }
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                if self.focused {
                    warp(window);
                }
            } else {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
            }
            self.captured = false;
        }
        // MouseHandler.grabMouse and releaseMouse: the mouse position is
        // the window's centre whenever the pointer's grab state changes, so
        // a screen opened from gameplay starts with the cursor in the middle
        // and the game starts with it away from the screen's last spot.
        if let Some(pos) = horizon_client::mouse::mouse_position_after_grab_change(
            capture,
            was_grabbed,
            size,
            scale,
        ) {
            self.cursor = pos;
        }
        if !self.options.offscreen {
            window.set_cursor_visible(!self.captured);
        }
    }
    /// `GuiGraphicsExtractor.applyCursor` -> `Window.selectCursor` at the end
    /// of a frame: the cursor the frame's widgets asked for, set on the window
    /// only when it changes.
    fn apply_cursor_icon(&mut self) {
        use horizon_client::widgets::Cursor;
        let wanted = horizon_client::widgets::take_cursor();
        // The widgets under the keyboard's hidden pointer aren't under the
        // mouse (`handleCursor` asks only for a hovered widget).
        let effective = if self.settings.allow_cursor_changes && !self.key_focus_driving() {
            wanted
        } else {
            Cursor::Arrow
        };
        if effective == self.cursor_icon || !self.keyboard_mouse {
            return;
        }
        if let Some(window) = &self.window {
            window.set_cursor(match effective {
                Cursor::Arrow => winit::window::CursorIcon::Default,
                Cursor::PointingHand => winit::window::CursorIcon::Pointer,
                Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
            });
        }
        self.cursor_icon = effective;
    }
    /// LevelLoadingScreen.onClose -> Gui.setScreen(null) -> grabMouse.
    fn capture_loaded_world(&mut self) {
        // Loading may finish after focus was lost or another screen opened.
        if self.title.is_none() && self.screen == Screen::Playing {
            self.capture(true);
        }
    }
    fn draw_item(&self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        // `GuiGraphics.renderItem`: the resolved item model (tints, foil and
        // special renderers included), then the decorations.
        self.draw_item_model(batch, stack, x, y);
        self.draw_item_decorations(batch, stack, x, y, None);
    }
    /// `extractSlot` of slot `slot` holding `stack` at (x, y): during a quick
    /// craft drag a dragged slot shows what it would hold over a white
    /// highlight (its count yellow when the slot caps it), and hides while
    /// it is the drag's only slot.
    fn draw_slot_item(
        &self,
        batch: &mut HudBatch,
        drag: Option<&DragView>,
        slot: usize,
        stack: &Option<ItemStack>,
        x: f32,
        y: f32,
    ) {
        let carried;
        let (stack, capped) = match drag.and_then(|d| d.slots.get(&slot)) {
            Some(None) => return,
            Some(Some((count, capped))) => {
                batch.fill(&self.gui.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                carried = self
                    .carried
                    .clone()
                    .map(|c| ItemStack { count: *count, ..c });
                (carried.as_ref(), *capped)
            }
            None => (stack.as_ref(), false),
        };
        if let Some(stack) = stack {
            self.draw_item_model(batch, stack, x, y);
            let text = capped.then(|| format!("\u{a7}e{}", stack.count));
            self.draw_item_decorations(batch, stack, x, y, text);
        }
    }
    /// `extractCarriedItem`: the carried stack at the cursor, showing what a
    /// quick craft drag over several slots leaves in hand.
    fn draw_carried(&mut self, batch: &mut HudBatch) {
        let Some(mut stack) = self.carried.clone() else {
            return;
        };
        if let Some(left) = self.drag_view().and_then(|d| d.left) {
            if left == 0 {
                return;
            }
            stack.count = left;
        }
        self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
    }
    /// The quick craft drag in progress, as the menu engine previews it.
    fn drag_view(&mut self) -> Option<DragView> {
        let (ty, slots) = self.inv_drag.clone()?;
        let mut view = None;
        self.menu_session(|menu, env, to_menu| {
            let menu_slots: Option<Vec<usize>> = slots.iter().map(|&g| to_menu(g)).collect();
            let shown = match menu.quick_craft_preview(env, ty, &menu_slots?) {
                Some((shown, left)) => {
                    let slots = slots.iter().copied().zip(shown).collect();
                    DragView {
                        slots,
                        left: Some(left),
                    }
                }
                // A single dragged slot is hidden.
                None => DragView {
                    slots: slots.iter().map(|&g| (g, None)).collect(),
                    left: None,
                },
            };
            view = Some(shown);
            Some(false)
        });
        view
    }
    /// Whether a quick craft drag takes `slot`
    /// (`shouldAddSlotToQuickCraft`); screens the menu engine doesn't
    /// model take every slot.
    fn quick_craft_accepts(&mut self, slot: usize, ty: i32, dragged: usize) -> bool {
        let mut accepts = true;
        self.menu_session(|menu, env, to_menu| {
            accepts = to_menu(slot).is_some_and(|s| menu.quick_craft_accepts(env, s, ty, dragged));
            Some(false)
        });
        accepts
    }
    /// `GuiGraphicsExtractor.itemDecorations`: the count (or `count_text`)
    /// and damage bar.
    fn draw_item_decorations(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        count_text: Option<String>,
    ) {
        // GuiGraphicsExtractor.itemDecorations, laid out at the origin and
        // moved to (x, y): the damage bar, the cooldown shade and the count.
        let info = self.items().get(&stack.item);
        let max_damage = info.map_or(0, |info| info.max_damage as i32);
        let cooldown =
            horizon_client::cooldowns::group(stack, info.and_then(|i| i.use_cooldown.as_ref()));
        let partial = self.clock.fraction() as f32;
        // BundleItem overrides the bar with its fullness.
        let mut bar = horizon_client::bundle::Contents::of(self.items(), stack)
            .map(|c| c.bar())
            .unwrap_or_else(|| hud_overlays::damage_bar(stack.damage as i32, max_damage));
        // Client-only: a mod may replace or hide the bar vanilla just chose.
        self.native_item_bar(stack, &mut bar);
        let decorations = hud_overlays::ItemDecorations {
            count: stack.count,
            count_text,
            bar,
            cooldown: self.cooldowns.percent(&cooldown, partial),
        };
        let mut draws = vec![hud_layout::GuiDraw::Push(vec![
            hud_layout::PoseOp::Translate(x, y),
        ])];
        draws.extend(hud_overlays::item_decorations(
            0,
            0,
            &decorations,
            &self.gui.atlas.font,
        ));
        draws.push(hud_layout::GuiDraw::Pop);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
        self.native_item_decorations(stack, x, y, batch);
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        // The pack prompt replaces the connecting screen (setScreen), but
        // in game it opens over the world.
        let mut batch = if (self.server_packs.prompt.is_some()
            || self.code_of_conduct.is_some()
            || self.dialog_open())
            && self.player.is_none()
        {
            HudBatch::new(size)
        } else {
            self.hud_layers(size)
        };
        self.draw_dialog(&mut batch);
        self.draw_pack_prompt(&mut batch);
        self.draw_code_of_conduct(&mut batch);
        self.draw_link_prompt(&mut batch);
        // The ToastManager draws over every screen, the title and the
        // options included: the narrator's toast follows the player there.
        self.draw_narrator_toast(&mut batch);
        // Gui.extractRenderState: the debug overlay comes after the screen
        // and the toasts, so it is over everything, menus included.
        self.draw_debug_overlay(&mut batch, size);
        self.frame_graph
            .draw(self.settings.horizon.overlay, &mut batch, &self.gui.atlas);
        self.draw_pad_cursor(&mut batch);
        self.resolve_inline_profiles(&batch.inline_profiles);
        batch
    }
    fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size);
        // GlobalSettingsUniform's GameTime, for a resource pack's text shader.
        batch.game_time = ((self.world.game_time.rem_euclid(24000)) as f32
            + self.clock.fraction() as f32)
            / 24000.0;
        let (w, h) = (batch.width, batch.height);
        // HDR calibration ends when its page does, whatever moved the screen
        // off Options (a death screen opened by a server, say).
        if self.screen != Screen::Options && self.ui.options.calibration_fullscreen.is_some() {
            self.show_calibration(h);
        }
        if self.screen == Screen::Win {
            self.draw_win_screen(&mut batch);
            // The early return skips the match below: the hook still sees the
            // screen, after its own drawing.
            self.fire_screen_render_post(&mut batch);
            return batch;
        }
        if self.screen == Screen::Options {
            if self.title.is_none() {
                self.flush_skin_slots();
                self.update_remote_skins();
            }
            self.draw_options(&mut batch);
            self.fire_screen_render_post(&mut batch);
            self.draw_split_hud(&mut batch, true);
            return batch;
        }
        if let Some(title) = &mut self.title {
            title.poll_backup();
            if std::mem::take(&mut title.onboarding_finished) {
                // Options.onboardingAccessibilityFinished.
                self.settings.file.set(
                    "onboardAccessibility",
                    horizon_client::options_file::V::Bool(false),
                );
                self.settings.save(&Self::options_path());
            }
            if let Some(sound) = title.update_toast(self.settings.notification_display_time) {
                self.audio.play(sound, None, 1.0, 1.0);
            }
        }
        if let Some(title) = &self.title {
            for found in title.lan_found_narrations((w, h)) {
                self.say_system_queued(&found);
            }
        }
        if self.title.is_some() {
            // TitleScreen.extractRenderState, then ToastManager over it: vanilla
            // draws the toast manager on every screen, so a pack download or a
            // resource-reload failure still shows on the title (#4428).
            if let Some(title) = &self.title {
                title.draw_with_items(
                    &self.gui.atlas,
                    Some(&self.pack.item_renderer),
                    &mut batch,
                    self.cursor,
                    self.settings.file.bool("hideSplashTexts"),
                );
            }
            self.draw_toasts(&mut batch, false);
            self.draw_tutorial_toasts(&mut batch, false);
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        let atlas_ptr: *const HudAtlas = &self.gui.atlas;
        let _ = atlas_ptr;
        // The ToastManager draws over every screen: a server's packs
        // download while the connection screen shows.
        if self.remote_overlay(&mut batch) {
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        if let Some(reason) = self.loading_reason() {
            self.draw_level_loading(&mut batch, reason);
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.spectator();
        let (wi, hi) = (w as i32, h as i32);
        let show_hud = !self.ui.hide_gui;
        // max(Hud.leftHeight, Hud.rightHeight) after the bars: the item name and
        // the action bar sit above it.
        let mut bar_height = 39;
        if show_hud {
            self.draw_spyglass(&mut batch);
            // Hud.extractCrosshair / extractItemHotbar / extractPlayerHealth /
            // extractVehicleHealth and the experience bar, laid out by hud_layout.
            let indicator = self.settings.attack_indicator;
            let delay = horizon_client::attack_strength::delay(self.main_hand_attack_speed());
            let strength = self.hand_strength.scale(0.0, self.main_hand_attack_speed());
            // The 3D crosshair (F3) takes the 2D one's place.
            // Hud.extractCrosshair: a spectator only sees it over something to open.
            if self.camera_type.first_person()
                && (!spectator || self.spectator_crosshair())
                && !self.debug_crosshair()
            {
                let attack = (indicator == 1).then_some(strength);
                let full = attack.is_some_and(|a| {
                    hud_layout::full_attack_indicator(a, delay, self.crosshair_living())
                });
                let draws = hud_layout::crosshair(wi, hi, attack, full);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
            }
            if spectator {
                self.draw_spectator_hotbar(&mut batch);
            } else {
                let partial = self.clock.fraction() as f32;
                let inputs = hud_layout::HotbarInputs {
                    selected: self.selected as i32,
                    right_handed: !self.settings.main_hand_left,
                    offhand: self.offhand.is_some(),
                    slots: std::array::from_fn(|i| {
                        self.inventory[i]
                            .as_ref()
                            .map(|_| (f32::from(self.hotbar_pop[i]) - partial).max(0.0))
                    }),
                    offhand_pop: (f32::from(self.hotbar_pop[9]) - partial).max(0.0),
                    attack: (indicator == 2).then_some(strength),
                };
                let draws = hud_layout::hotbar(wi, hi, &inputs);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
                for d in &draws {
                    if let hud_layout::GuiDraw::Item { slot, x, y, .. } = *d {
                        let stack = if slot == 40 {
                            self.offhand.as_ref()
                        } else {
                            self.inventory[slot as usize].as_ref()
                        };
                        if let Some(stack) = stack {
                            self.draw_item(&mut batch, stack, x as f32, y as f32);
                        }
                    }
                }
            }
            self.draw_effect_icons(&mut batch);
            // Hud: the action bar and titles draw over the hotbar (nextStratum).
            self.draw_boss_bars(&mut batch);
            self.draw_extras(&mut batch);
        }
        if show_hud {
            // Hud.nextContextualInfoState: the jump bar, the experience bar or
            // the locator bar above the hotbar.
            let has_experience = !self.creative && !spectator;
            let contextual = horizon_client::locator_bar::ContextualBar::next(
                self.waypoints.has_waypoints(),
                !spectator && self.jumpable_mount(),
                self.mount_jumping(),
                has_experience,
                self.experience_recent(),
            );
            use horizon_client::locator_bar::ContextualBar;
            // The mount's hearts (and the jump bar).
            let vehicle_hearts = if spectator {
                0
            } else {
                self.draw_mount_hud(
                    &mut batch,
                    wi,
                    hi,
                    contextual == ContextualBar::JumpableVehicle,
                )
            };
            // Without the survival bars only the mount's hearts raise it.
            bar_height = 39 + 10 * ((vehicle_hearts + 9) / 10);
            if contextual == ContextualBar::Locator {
                let (sprite, [x, y, w, h]) = horizon_client::locator_bar::background(wi, hi);
                horizon_client::ui::nine_slice(
                    &mut batch,
                    &self.gui.atlas,
                    sprite,
                    x as f32,
                    y as f32,
                    w as f32,
                    h as f32,
                    u32::MAX,
                );
            }
            if !self.creative
                && !spectator
                && let Some(p) = &mut self.player
            {
                let atlas = &self.gui.atlas;
                let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                let draws = hud_layout::experience(
                    wi,
                    hi,
                    self.experience.progress,
                    self.experience.level,
                    contextual == ContextualBar::Experience,
                    &width,
                );
                hud_layout::paint(&mut batch, atlas, &draws);
                let effects = &self.potions.effects;
                let kind = if effects.get("poison").is_some() {
                    hud_layout::HeartKind::Poisoned
                } else if effects.get("wither").is_some() {
                    hud_layout::HeartKind::Withered
                } else if p.is_fully_frozen() {
                    hud_layout::HeartKind::Frozen
                } else {
                    hud_layout::HeartKind::Normal
                };
                // Hud.extractAirBubbles: `isEyeInFluidMatching`, so the eyes are below
                // the surface, not merely in the water block.
                let under_water = p.is_eye_in_fluid(horizon_client::motion::WATER);
                let inputs = hud_layout::HealthInputs {
                    tick: self.ticks as i32,
                    millis: self.started.elapsed().as_millis() as i64,
                    health: p.health.health(),
                    max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
                    absorption: p.health.absorption(),
                    armor: p
                        .attribute("minecraft:armor")
                        .or_else(|_| p.attribute("armor"))
                        .unwrap_or(0.0) as i32,
                    food: p.health.food.food,
                    saturation: p.health.food.saturation,
                    kind,
                    regeneration: effects.get("regeneration").is_some(),
                    hunger: effects.get("hunger").is_some(),
                    hardcore: self.level.hardcore,
                    damage_cooldown: p.health.hurt.cooldown > 0,
                    air: p.air,
                    max_air: 300,
                    under_water,
                    vehicle_hearts,
                };
                let at = [p.position.x, p.position.y, p.position.z];
                // The native mods' health HUD (`native_health_hud`): a mod
                // that answers draws every row instead.
                let mut native_health_hud = None;
                self.native_health_hud(&inputs, wi, hi, &mut native_health_hud);
                let draws = match native_health_hud {
                    Some(draws) => draws,
                    None => self.health_hud.layout(&inputs, wi, hi),
                };
                // A fresh borrow: the hook ran with all of `self`.
                let atlas = &self.gui.atlas;
                hud_layout::paint(&mut batch, atlas, &draws);
                bar_height = self
                    .health_hud
                    .left_height
                    .max(self.health_hud.right_height);
                // Hud.playAirBubblePoppedSound: Player.playSound (PLAYERS), heard
                // by this client only.
                if let Some((volume, pitch)) = self.health_hud.bubble_pop_sound.take() {
                    let players = horizon_client::settings::SOUND_SOURCES
                        .iter()
                        .position(|s| *s == "player")
                        .unwrap_or(0);
                    let capture = self.audio.capture.take();
                    self.audio
                        .play_on(players, "ui.hud.bubble_pop", Some(at), volume, pitch);
                    self.audio.capture = capture;
                }
            }
            if contextual == ContextualBar::Locator {
                let draws = self.locator_bar_draws(wi, hi);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
            }
            if !spectator {
                self.draw_selected_name(&mut batch, bar_height, !self.creative);
            }
        }
        // Hud.extractSleepOverlay: over all health/food bars, under titles and chat.
        let draws = hud_overlays::sleep_overlay(wi, hi, self.sleep_timer as i32);
        hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
        if show_hud {
            self.draw_titles(&mut batch, bar_height);
            self.draw_scoreboard(&mut batch);
            self.draw_subtitles(&mut batch);
        }
        let atlas = &self.gui.atlas;
        if show_hud
            && let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        self.draw_chat(&mut batch);
        self.draw_tab_list(&mut batch);
        // Screen.extractBlurredBackground blurs the world and HUD, while the
        // screen's background, widgets, toasts and debug overlay stay sharp.
        batch.blur_at = batch.vertices.len() as u32;
        // PackConfirmScreen takes the open screen's place (it comes back once
        // the prompt is answered): the screen under the prompt isn't drawn.
        let screen = if self.server_packs.prompt.is_some() {
            Screen::Playing
        } else {
            self.screen
        };
        match screen {
            Screen::Paused if self.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                let title = self.translate("menu.paused", "Game Paused");
                horizon_client::widgets::begin_screen(|| {
                    horizon_client::widgets::ScreenInfo::new("paused", title.clone())
                });
                let tw = HudBatch::text_width(&self.gui.atlas, &title);
                batch.text(
                    &self.gui.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
            }
            Screen::GameModeSwitcher => self.draw_gamemode_switcher(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Friends => self.draw_friends_screen(&mut batch),
            Screen::ServerLinks => self.draw_server_links(&mut batch),
            Screen::ConfirmLink => self.draw_confirm_link(&mut batch),
            Screen::DialogWarning => self.draw_dialog_warning(&mut batch),
            Screen::CommandBlock => self.draw_command_block(&mut batch),
            Screen::BlockEditor => self.draw_block_editor(&mut batch),
            Screen::DebugOptions => self.draw_debug_options(&mut batch),
            Screen::Inventory => {
                self.draw_inventory(&mut batch);
                self.draw_mod_browser(&mut batch);
            }
            Screen::Creative => {
                self.draw_creative(&mut batch);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Options | Screen::Win => {}
        }
        // ScreenEvent.Render.Post: the open screen has drawn itself and its
        // tooltips, so a native mod draws over it here. Never `Playing`: the
        // prompts, books and sign editors drawn over gameplay keep it, draw
        // after this point and so over what a mod drew.
        self.fire_screen_render_post(&mut batch);
        self.draw_saving_indicator(&mut batch);
        // ToastManager.extractRenderState: F1 hides the toasts too.
        self.draw_toasts(&mut batch, !show_hud);
        self.draw_tutorial_toasts(&mut batch, !show_hud);
        self.draw_pack_toasts(&mut batch, !show_hud);
        self.draw_music_toast(&mut batch, !show_hud);
        if self.server_packs.prompt.is_none() {
            self.remote_hud(&mut batch);
            self.draw_sign_editor(&mut batch);
            self.draw_book(&mut batch);
        }
        batch
    }
    /// `native_screen_render_post` (`ScreenEvent.Render.Post`) for the open
    /// screen, once a frame; the Win and Options screens call this on their
    /// own path. Reads `self.screen`, the one `hud_layers` drew. Not while
    /// playing with no screen. Client only, read-only: what a mod draws is
    /// its own.
    fn fire_screen_render_post(&self, batch: &mut HudBatch) {
        let screen = self.screen;
        if screen == Screen::Playing {
            return;
        }
        // slot_count mirrors slots() without allocating; keep them in step.
        debug_assert_eq!(self.slot_count(), self.slots().len());
        let view = self.screen_view(batch, screen);
        self.native_screen_render_post(batch, &view);
    }
    /// `CreativeModeInventoryScreen.hasPermissions` (`canUseGameMasterBlocks`
    /// and the Operator Items Tab option) given to `tryRefreshInvalidatedTabs`,
    /// as the screen's constructor and `containerTick` do.
    fn refresh_creative_tabs(&mut self) {
        let permissions = self.can_use_game_master_blocks() && self.settings.operator_items_tab;
        self.creative_screen
            .set_permissions(permissions, &self.pack.names);
    }
    fn creative_inventory_open(&self) -> bool {
        self.screen == Screen::Creative && self.creative_screen.tab().inventory
    }
    /// An item tab of the creative screen (its ItemPickerMenu).
    fn creative_tab_open(&self) -> bool {
        self.screen == Screen::Creative && !self.creative_screen.tab().inventory
    }
    fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        if self.screen == Screen::Creative {
            return Self::creative_origin(batch);
        }
        let width = self.native_screen_width().unwrap_or(176.0);
        let (x, y) = batch.centered_origin(width, self.screen_height());
        (x + self.recipe_book_shift(batch.width), y)
    }
    /// Slot ids: 0..36 player inventory, 100..109 crafting grid, 200 result.
    fn container_kind(&self) -> Option<&BlockEntity> {
        self.container
            .and_then(|k| self.world.block_entities.get(&k))
    }
    /// 300.. are the open container's slots.
    fn slots(&self) -> Vec<usize> {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().collect();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().collect();
        }
        if let Some(v) = self.station_slots().or_else(|| self.mount_slots()) {
            return v;
        }
        let mut v: Vec<usize> = (0..36).collect();
        if let Some(shown) = self.native_screen_slots() {
            v.extend(shown);
            return v;
        }
        match self.container_kind() {
            Some(BlockEntity::Chest(_)) => v.extend((0..self.container_size()).map(|i| 300 + i)),
            Some(BlockEntity::Furnace(_)) => v.extend(300..303),
            Some(BlockEntity::Brewing(_)) => v.extend(300..305),
            Some(BlockEntity::Crafter(_)) => v.extend(300..310),
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => {}
            None => {
                v.extend((0..self.craft_size * self.craft_size).map(|i| 100 + i));
                v.push(200);
                if self.craft_size == 2 {
                    v.extend(500..505);
                }
            }
        }
        v
    }
    /// How many slots `slots` lists, counted without building the list: the
    /// screen render hook asks once a frame and must not allocate. Same
    /// branches in the same order, so the two stay in step.
    fn slot_count(&self) -> usize {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().count();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().count();
        }
        if let Some(n) = self
            .station_slot_count()
            .or_else(|| self.mount_slot_count())
        {
            return n;
        }
        let inventory = 36;
        if let Some(shown) = self.native_screen_slot_count() {
            return inventory + shown;
        }
        let container = match self.container_kind() {
            Some(BlockEntity::Chest(_)) => self.container_size(),
            Some(BlockEntity::Furnace(_)) => 3,
            Some(BlockEntity::Brewing(_)) => 5,
            Some(BlockEntity::Crafter(_)) => 10,
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => 0,
            None => {
                self.craft_size * self.craft_size + 1 + if self.craft_size == 2 { 5 } else { 0 }
            }
        };
        inventory + container
    }
    /// InventoryMenu / CraftingMenu slot layouts.
    fn slot_position(&self, i: usize) -> (f32, f32) {
        if self.creative_inventory_open()
            && let Some(position) = Creative::inventory_slot_position(i)
        {
            return position;
        }
        if self.creative_tab_open()
            && let Some(position) = Creative::tab_slot_position(i)
        {
            return position;
        }
        if let Some(p) = self
            .station_slot_position(i)
            .or_else(|| {
                Self::stock_machine_slot(self.machine_slot_position(i), self.native_screen_shown())
            })
            .or_else(|| self.crafter_slot_position(i))
            .or_else(|| self.mount_slot_position(i))
        {
            return p;
        }
        let table = self.craft_size == 3;
        // ContainerScreen (3 rows) shifts the player inventory down by one pixel.
        // ChestMenu: the inventory sits `18 * (rows - 3) + 1` lower than the
        // 3-row layout of the other menus.
        let shift = match (self.chest_rows(), self.native_screen_inventory_y()) {
            (_, Some(y)) => y - 84.0,
            (Some(rows), None) => 1.0 + (rows as f32 - 3.0) * 18.0,
            (None, None) => 0.0,
        };
        let inventory_x = self.native_screen_inventory_x().unwrap_or(8.0);
        match i {
            0..9 => (inventory_x + i as f32 * 18.0, 142.0 + shift),
            9..36 => (
                inventory_x + ((i - 9) % 9) as f32 * 18.0,
                84.0 + shift + ((i - 9) / 9) as f32 * 18.0,
            ),
            500..504 => (8.0, 8.0 + (i - 500) as f32 * 18.0),
            // InventoryMenu: the shield slot.
            504 => (77.0, 62.0),
            300..500 if let Some(p) = self.native_screen_slot(i - 300) => p,
            300..500 => match self.container_kind() {
                Some(BlockEntity::Furnace(_)) => {
                    [(56.0, 17.0), (56.0, 53.0), (116.0, 35.0)][(i - 300).min(2)]
                }
                _ => (
                    8.0 + ((i - 300) % 9) as f32 * 18.0,
                    18.0 + ((i - 300) / 9) as f32 * 18.0,
                ),
            },
            200 => {
                if table {
                    (124.0, 35.0)
                } else {
                    (154.0, 28.0)
                }
            }
            _ => {
                let g = i - 100;
                let (r, c) = (g / self.craft_size, g % self.craft_size);
                if table {
                    (30.0 + c as f32 * 18.0, 17.0 + r as f32 * 18.0)
                } else {
                    (98.0 + c as f32 * 18.0, 18.0 + r as f32 * 18.0)
                }
            }
        }
    }
    /// The stock hopper/dispenser slot layout, unless a mod screen is shown for
    /// that menu: a mod screen places its own container slots
    /// (`native_screen_slot`) and the player's inventory (`native_screen_inventory_y`),
    /// and this layout answers first in `slot_position`, which kept a mod's screen
    /// on a hopper or a dispenser on the stock grid (#5141). The rest of those
    /// menus is already the mod's: `draw_native_screen` owns the background and
    /// `screen_height` its height.
    fn stock_machine_slot(stock: Option<(f32, f32)>, mod_screen_shown: bool) -> Option<(f32, f32)> {
        if mod_screen_shown { None } else { stock }
    }

    fn slot_at(&self, batch: &HudBatch, x: f32, y: f32) -> Option<usize> {
        if self.book_covers_menu(batch.width) {
            return None;
        }
        let (ox, oy) = self.inventory_origin(batch);
        self.slots().into_iter().find(|&i| {
            let (sx, sy) = self.slot_position(i);
            // AbstractContainerScreen.isHovering: the 16x16 slot grows by one
            // pixel on every side, so the gaps between slots still hover one.
            x >= ox + sx - 1.0 && x < ox + sx + 17.0 && y >= oy + sy - 1.0 && y < oy + sy + 17.0
        })
    }
    /// `AbstractContainerScreen.extractSlotHighlightBack` and
    /// `AbstractContainerScreen.extractSlotHighlightFront`: the 24 x 24
    /// `container/slot_highlight_back` sprite behind the slot at `x`, `y` (its
    /// 16 x 16 item box), the `_front` one over its item.
    fn draw_slot_highlight(&self, batch: &mut HudBatch, front: bool, x: f32, y: f32) {
        let sprite = if front {
            "gui/sprites/container/slot_highlight_front"
        } else {
            "gui/sprites/container/slot_highlight_back"
        };
        horizon_client::ui::blit_sprite(
            batch,
            &self.gui.atlas,
            sprite,
            x - 4.0,
            y - 4.0,
            24.0,
            24.0,
            u32::MAX,
        );
    }
    /// The highlight of the slot under the cursor, for the screens whose
    /// slots all sit in `slot_at`'s grid. `Slot.isHighlightable` is false
    /// for the crafter's result (`NonInteractiveResultSlot`).
    fn draw_hovered_slot_highlight(&self, batch: &mut HudBatch, front: bool, ox: f32, oy: f32) {
        let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        if slot == 309 && matches!(self.container_kind(), Some(BlockEntity::Crafter(_))) {
            return;
        }
        let (sx, sy) = self.slot_position(slot);
        self.draw_slot_highlight(batch, front, ox + sx, oy + sy);
    }
    fn slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        if i >= mount_menu::SADDLE {
            return self.mount_slot_mut(i);
        }
        if i >= stations::INPUT {
            return self.station_slot_mut(i);
        }
        if (500..504).contains(&i) {
            return &mut self.armor[i - 500];
        }
        if i == 504 {
            return &mut self.offhand;
        }
        if i >= 300 {
            return self.container_slot_mut(i - 300);
        }
        if (100..109).contains(&i) {
            &mut self.craft_grid[i - 100]
        } else {
            &mut self.inventory[i]
        }
    }
    /// What a slot shows: the crafting result slots are computed.
    fn slot_view(&mut self, i: usize) -> Option<ItemStack> {
        match i {
            200 => self.craft_result(),
            309 if matches!(self.container_kind(), Some(BlockEntity::Crafter(_))) => {
                self.crafter_result()
            }
            _ => self.slot_mut(i).clone(),
        }
    }
    fn craft_result(&self) -> Option<ItemStack> {
        self.craft_recipe().map(|(_, result)| result)
    }
    /// The recipe id and result of the crafting grid.
    /// `RecipeCraftingHolder.awardUsedRecipes`: a crafted recipe becomes known
    /// (special ones never do).
    fn award_crafted(&mut self, crafts: &[(String, Vec<Option<ItemStack>>)]) {
        for (recipe, ingredients) in crafts {
            self.trigger_recipe_crafted(recipe, ingredients);
        }
        let recipes: Vec<String> = crafts.iter().map(|(r, _)| r.clone()).collect();
        let index = &world_stations!(self).recipe_index;
        let (_, unlocked) = self.level.recipe_book.add_recipes(index, &recipes);
        self.recipe_unlocked(&unlocked);
    }
    fn craft_recipe(&self) -> Option<(String, ItemStack)> {
        let n = self.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n).map(|i| self.craft_grid[i].clone()).collect();
        let scale = |id: i32| self.maps.data.get(&id).map(|m| m.scale);
        self.res
            .recipes()
            .find_with(&grid, n, &scale)
            .map(|(name, result)| (name.to_owned(), result))
    }
    fn draw_inventory_slots(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let drag = self.drag_view();
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            if i == 504 && self.offhand.is_none() {
                batch.sprite(
                    &self.gui.atlas,
                    "gui/sprites/container/slot/shield",
                    ox + sx,
                    oy + sy,
                );
            }
            if (500..504).contains(&i) && self.armor[i - 500].is_none() {
                let icon = ["helmet", "chestplate", "leggings", "boots"][i - 500];
                batch.sprite(
                    &self.gui.atlas,
                    &format!("gui/sprites/container/slot/{icon}"),
                    ox + sx,
                    oy + sy,
                );
            }
            let stack = self.slot_view(i);
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
    }
    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        screen_info::begin_container("container");
        if self.draw_station(batch) {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
            let part = |batch: &mut HudBatch,
                        name: &str,
                        sx: f32,
                        sy: f32,
                        w: f32,
                        h: f32,
                        dx: f32,
                        dy: f32| {
                if let Some(mut r) = atlas.sprites.get(name).copied() {
                    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
                    r.u0 += sx * du;
                    r.v0 += sy * dv;
                    r.u1 = r.u0 + w * du;
                    r.v1 = r.v0 + h * dv;
                    batch.rect_uv(dx, dy, w, h, r, u32::MAX);
                }
            };
            let grey = rgba(64, 64, 64, 255);
            match self
                .container
                .and_then(|k| self.world.block_entities.get(&k))
            {
                Some(_) if self.draw_native_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) if self.draw_machine_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) => self.draw_chest_screen(batch, ox, oy),
                Some(BlockEntity::Furnace(f)) => {
                    let (tex, key, fallback) = match f.kind {
                        horizon_client::containers::FurnaceKind::Furnace => {
                            ("gui/container/furnace", "container.furnace", "Furnace")
                        }
                        horizon_client::containers::FurnaceKind::BlastFurnace => (
                            "gui/container/blast_furnace",
                            "container.blast_furnace",
                            "Blast Furnace",
                        ),
                        horizon_client::containers::FurnaceKind::Smoker => {
                            ("gui/container/smoker", "container.smoker", "Smoker")
                        }
                    };
                    part(batch, tex, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    // A server's furnace menu has the title it was given.
                    let title = self.remote_menu_title().unwrap_or_else(|| {
                        horizon_client::text::Text::literal(
                            self.container_title(self.translate(key, fallback)),
                        )
                        .resolve_legacy()
                    });
                    screen_info::name_container(|| title.plain());
                    let tw = atlas.font.width_text(&title);
                    batch.text_styled(
                        atlas,
                        &title,
                        ox + (176.0 - tw) / 2.0,
                        oy + 6.0,
                        grey,
                        false,
                    );
                    let inventory = self.translate("container.inventory", "Inventory");
                    batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    if f.lit_remaining > 0 {
                        // AbstractFurnaceScreen: flame height ceil(lit * 13) + 1.
                        let total = if f.lit_total == 0 { 200 } else { f.lit_total };
                        let h = ((f.lit_remaining as f32 / total as f32) * 13.0).ceil() + 1.0;
                        part(
                            batch,
                            "gui/sprites/container/furnace/lit_progress",
                            0.0,
                            14.0 - h,
                            14.0,
                            h,
                            ox + 56.0,
                            oy + 36.0 + 14.0 - h,
                        );
                    }
                    if f.cooking_timer > 0 && f.cooking_total > 0 {
                        let w = ((f.cooking_timer as f32 / f.cooking_total as f32) * 24.0).ceil();
                        part(
                            batch,
                            "gui/sprites/container/furnace/burn_progress",
                            0.0,
                            0.0,
                            w,
                            16.0,
                            ox + 79.0,
                            oy + 34.0,
                        );
                    }
                }
                Some(BlockEntity::Brewing(_)) => {}
                Some(BlockEntity::Crafter(_)) => self.draw_crafter_screen(batch, ox, oy),
                Some(
                    BlockEntity::Sign(_)
                    | BlockEntity::Jukebox(_)
                    | BlockEntity::Beacon(_)
                    | BlockEntity::Lectern(_)
                    | BlockEntity::Sculk(_)
                    | BlockEntity::Banner(_)
                    | BlockEntity::CreakingHeart(_)
                    | BlockEntity::Spawner(_)
                    | BlockEntity::TrialSpawner(_)
                    | BlockEntity::Vault(_)
                    | BlockEntity::Beehive(_)
                    | BlockEntity::CommandBlock(_)
                    | BlockEntity::EndGateway(_)
                    | BlockEntity::Skull(_)
                    | BlockEntity::DaylightDetector
                    | BlockEntity::Campfire(_)
                    | BlockEntity::ChiseledBookshelf(_)
                    | BlockEntity::Brushable(_)
                    | BlockEntity::Shelf(_)
                    | BlockEntity::Mod(_)
                    | BlockEntity::PotentSulfur(_)
                    | BlockEntity::StructureBlock(_)
                    | BlockEntity::JigsawBlock(_)
                    | BlockEntity::TestBlock(_)
                    | BlockEntity::TestInstanceBlock(_),
                ) => {}
                None if self.mount_active() => self.draw_mount_background(batch, ox, oy),
                None => {
                    let texture = if self.craft_size == 3 {
                        "gui/container/crafting_table"
                    } else {
                        "gui/container/inventory"
                    };
                    part(batch, texture, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let title = self.translate("container.crafting", "Crafting");
                    screen_info::name_container(|| title.clone());
                    let tx = if self.craft_size == 3 { 29.0 } else { 97.0 };
                    batch.text(atlas, &title, ox + tx, oy + 6.0, grey, false);
                    if self.craft_size == 3 {
                        let inventory = self.translate("container.inventory", "Inventory");
                        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        if self.container.is_none() && self.craft_size != 3 && !self.mount_active() {
            self.draw_inventory_player(batch, ox, oy);
        }
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        self.draw_inventory_slots(batch, ox, oy);
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        self.draw_sort_buttons(batch, ox, oy);
        self.draw_recipe_book(batch);
        let effect_tip = if self.container.is_none() && self.craft_size == 2 && !self.mount_active()
        {
            self.draw_inventory_effects(batch, (ox, oy), 176)
        } else {
            None
        };
        self.draw_carried(batch);
        self.draw_slot_tooltip(batch);
        self.draw_native_screen_tooltip(batch);
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// `AbstractContainerScreen` keeps `isQuickCrafting` and the drag it
    /// belongs to on the screen. Horizon keeps both on `Game` (`inv_clicks`,
    /// `inv_drag`, `last_inv_click`), so they are dropped when the pointer
    /// changes hands: a screen that opens or closes starts with its own
    /// state, and a press whose release never reached the screen -- the
    /// pointer left the window, or the screen closed first -- would
    /// otherwise leave every later press on every later screen swallowed.
    pub(super) fn reset_inventory_clicks(&mut self) {
        self.inv_clicks.cancel_drag();
        self.inv_drag = None;
        self.last_inv_click = None;
    }
    pub(super) fn close_container(&mut self) {
        self.mod_browser_closed();
        self.reset_inventory_clicks();
        self.close_station();
        self.mount_closed();
        self.recipe_ui.reset_screen();
        if self.remote.is_some() {
            return self.remote_close_container();
        }
        let mut returned: Vec<ItemStack> = self.carried.take().into_iter().collect();
        returned.extend(self.craft_grid.iter_mut().filter_map(Option::take));
        for stack in returned {
            if let Some(rest) = self.give(stack)
                && let Some(eye) = self.eye_position()
            {
                // AbstractContainerMenu.dropOrPlaceInInventory: `drop(stack, false)`.
                self.throw_from_eyes(rest, eye, false);
            }
        }
        self.craft_size = 2;
        self.craft_table = None;
        if let Some(key) = self.container.take() {
            // A cart's or chest boat's items go back to it.
            self.close_cart_container(key);
            let last = self.container_opener_event(key, false);
            self.post_block_game_event(
                "minecraft:container_close",
                [key.0, key.1, key.2],
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            if last {
                self.container_edge_sound(key, false);
            }
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        self.close_ender_chest();
    }
    /// `quickMoveStack` of the open plain menu for a shift-click.
    fn inventory_quick_move(&mut self, slot: usize) -> bool {
        self.menu_click(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
    /// `AbstractContainerMenu.clicked` on the open inventory, crafting table,
    /// chest, hopper, dispenser or furnace menu, run by the verified menu
    /// engine (`horizon_client::menu`). `slot` None is outside the window.
    /// False when the open screen is not one the engine models.
    fn menu_click(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: horizon_client::menu::Input,
    ) -> bool {
        if let Some(slot) = slot
            && slot >= Creative::DISPLAY
            && self.creative_tab_open()
        {
            self.creative_display_click(slot, button, input);
            return true;
        }
        if let Some(slot) = slot
            && input == horizon_client::menu::Input::Throw
            && self.carried.is_some()
            && self.remote.is_some()
            && ((slot < 9 && self.creative_tab_open()) || self.creative_inventory_open())
        {
            self.creative_throw_carrying(slot, button);
            return true;
        }
        self.menu_ops(&[(slot, button, input)])
    }
    /// `CreativeModeInventoryScreen.slotClicked`: THROW while a stack is carried, on an item
    /// tab's hotbar slot or any slot of the Survival Inventory tab. The menu's own THROW needs an
    /// empty hand, so the screen drops from the slot itself: one item, or with the control key
    /// the whole stack (on the inventory tab `Slot.remove(getMaxStackSize())`), through
    /// `handleCreativeModeItemDrop`, then sends the slot (`broadcastChanges`,
    /// `handleCreativeModeItemAdd`); the carried stack stays. Only the server drops it: this is
    /// the client of a server (`remote`).
    fn creative_throw_carrying(&mut self, slot: usize, button: i32) {
        let Some(old) = self.slot_mut(slot).clone() else {
            return;
        };
        let count = if button == 0 {
            1
        } else if self.creative_inventory_open() {
            let max = world_items!(self)
                .get(&old.item)
                .map_or(64, |i| i.max_stack);
            old.count.min(max)
        } else {
            old.count
        };
        let before = self.player_equipment();
        let left = old.count - count;
        *self.slot_mut(slot) = (left > 0).then(|| ItemStack {
            count: left,
            ..old.clone()
        });
        self.player_equipment_changed(&before);
        self.apply_equipment();
        self.remote_creative_throw(ItemStack { count, ..old });
        self.send_creative_slots();
    }
    /// A click on an item tab's display slot: the screen's own handling
    /// (`creative::click_display`), never the menu's.
    fn creative_display_click(
        &mut self,
        slot: usize,
        button: i32,
        input: horizon_client::menu::Input,
    ) {
        use horizon_client::creative::{DisplayClick, click_display};
        if self.creative_screen.tab().search {
            // CreativeModeInventoryScreen.slotClicked: moveCursorToEnd, then
            // setHighlightPos(0), so typing replaces the whole query.
            let search = &mut self.creative_screen.search;
            search.move_cursor_to_end(false);
            search.highlight = 0;
        }
        let first = self.creative_screen.first_row(&self.pack.names);
        let shown = self
            .creative_screen
            .shown(&self.pack.names, first * 9 + slot - Creative::DISPLAY);
        let catalog = world_items!(self);
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        match click_display(&mut self.carried, shown.as_ref(), max, button, input) {
            DisplayClick::Hotbar(i, stack) if i < self.inventory.len() => {
                self.inventory[i] = Some(stack);
            }
            DisplayClick::Offhand(stack) => self.offhand = Some(stack),
            // handleCreativeModeItemDrop: on a server the stack goes to it (SetCreativeModeSlot
            // to slot -1), which drops it; a local drop here would never reach the server.
            DisplayClick::Drop(stack) if self.remote.is_some() => self.remote_creative_throw(stack),
            DisplayClick::Drop(stack) => self.player_drop(stack),
            _ => {}
        }
        // handleCreativeModeItemAdd / broadcastChanges: a swapped-in hotbar slot goes out now.
        if self.remote.is_some() {
            self.send_creative_slots();
        }
    }
    /// Several clicks on one menu instance (a quick craft drag is three).
    fn menu_ops(&mut self, ops: &[(Option<usize>, i32, horizon_client::menu::Input)]) -> bool {
        use horizon_client::menu::Input;
        // BundleMouseActions.onSlotClicked: a Shift click or a swap clears the bundle's selection.
        for &(slot, _, input) in ops {
            if let (Some(slot), Input::QuickMove | Input::Swap) = (slot, input) {
                self.unselect_bundle(slot);
            }
        }
        if self.remote.is_some() && self.screen == Screen::Inventory {
            // On a server the clicks are the server's: predicted and sent.
            return self.remote_menu_ops(ops);
        }
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let ops = ops.to_vec();
        let done = self.menu_session(move |menu, env, to_menu| {
            let mut clicks = Vec::new();
            for &(slot, button, input) in &ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        match to_menu(g) {
                            Some(i) => i as i32,
                            None => return None,
                        }
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            for (menu_slot, button, input) in clicks {
                if menu.click(env, menu_slot, button, input).is_err() {
                    // Vanilla's server throws: nothing of this click is kept.
                    return Some(false);
                }
            }
            Some(true)
        });
        // CreativeModeInventoryScreen.slotClicked: the changed slots go out with the click.
        if done && self.remote.is_some() && self.screen == Screen::Creative {
            self.send_creative_slots();
        }
        done
    }
    /// A sound a menu click made the player play (a bundle's insert or
    /// remove): `Entity.playSound` at the player.
    pub(super) fn player_menu_sound(&mut self, sound: &str) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let (volume, pitch) = if sound == horizon_client::bundle::INSERT_FAIL {
            (1.0, 1.0)
        } else {
            (0.8, 0.8 + self.world.random.next_float() * 0.4)
        };
        self.audio.play(sound, Some(at), volume, pitch);
    }
    /// The open menu as the verified engine (`horizon_client::menu`): the
    /// game's slots are copied in, `run` acts on it, and the slots, carried
    /// stack, menu data and drops are copied back when it returns Some(true)
    /// (Some(false): handled, nothing kept; None: not handled). False when the
    /// open screen is not one the engine models or `run` did not handle it.
    pub(super) fn menu_session(
        &mut self,
        run: impl FnOnce(
            &mut horizon_client::menu::Menu,
            &horizon_client::menu::Env,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        use horizon_client::menu::{Env, Kind, Menu};
        #[derive(Clone, Copy)]
        struct Map {
            container: usize,
            table: bool,
            cells: usize,
            plain: bool,
        }
        impl Map {
            fn main(&self) -> usize {
                match (self.plain, self.table) {
                    (false, _) => self.container,
                    (true, true) => 10,
                    (true, false) => 9,
                }
            }
            fn to_menu(self, g: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match g {
                        200 => Some(0),
                        100..109 if g - 100 < self.cells => Some(1 + g - 100),
                        500..504 if !self.table => Some(5 + g - 500),
                        504 if !self.table => Some(45),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                } else {
                    match g {
                        300.. if g < 300 + self.container => Some(g - 300),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                }
            }
            fn to_game(self, i: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match i {
                        0 => None,
                        1..=9 if i - 1 < self.cells => Some(100 + i - 1),
                        5..9 if !self.table => Some(500 + i - 5),
                        45 if !self.table => Some(504),
                        _ if i >= main && i < main + 27 => Some(9 + i - main),
                        _ if i >= main + 27 && i < main + 36 => Some(i - main - 27),
                        _ => None,
                    }
                } else if i < self.container {
                    Some(300 + i)
                } else if i < self.container + 27 {
                    Some(9 + i - self.container)
                } else {
                    Some(i - self.container - 27)
                }
            }
        }
        // Menu slot -> game slot, for every menu slot.
        let mut plain_map = None;
        let crafter_open = matches!(self.container_kind(), Some(BlockEntity::Crafter(_)));
        let loom_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Loom);
        let merchant_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Merchant);
        let stonecutter_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Stonecutter);
        let station_kind = self.station.as_ref().map(|s| s.kind);
        let grindstone_open = station_kind == Some(stations::Kind::Grindstone);
        let cartography_open = station_kind == Some(stations::Kind::Cartography);
        let anvil_open = station_kind == Some(stations::Kind::Anvil);
        let smithing_open = station_kind == Some(stations::Kind::Smithing);
        let enchanting_open = station_kind == Some(stations::Kind::Enchanting);
        let beacon_open = station_kind == Some(stations::Kind::Beacon);
        let (kind, game_of): (Kind, Vec<Option<usize>>) = if self.creative_tab_open() {
            // ItemPickerMenu: the display slots (clicked by the screen), then the hotbar.
            let mut v = vec![None; 45];
            v.extend((0..9).map(Some));
            (Kind::ItemPicker, v)
        } else if let Some(columns) = self
            .mount_menu_columns()
            .filter(|_| self.remote.is_some() && self.mount_active())
        {
            // A server's HorseInventoryMenu: the saddle, the body armour,
            // the chest, then the inventory.
            let mut v = vec![Some(mount_menu::SADDLE), Some(mount_menu::ARMOR)];
            v.extend((0..columns * 3).map(|i| Some(mount_menu::CHEST + i)));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Mount(columns), v)
        } else if merchant_open {
            // MerchantMenu: two payment slots, the result, then the inventory.
            let mut v: Vec<Option<usize>> = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Merchant, v)
        } else if crafter_open {
            // CrafterMenu: the grid, the inventory, then the result.
            let mut v: Vec<Option<usize>> = (300..309).map(Some).collect();
            v.extend((9..36).chain(0..9).map(Some));
            v.push(None);
            (Kind::CrafterBlock, v)
        } else if loom_open {
            // LoomMenu: banner, dye, pattern, result, then the inventory.
            let mut v: Vec<Option<usize>> = (0..3).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Loom, v)
        } else if stonecutter_open {
            // StonecutterMenu: the input, the result, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::RESULT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Stonecutter, v)
        } else if grindstone_open || cartography_open {
            // GrindstoneMenu and CartographyTableMenu: the two inputs, the
            // result, then the inventory.
            let mut v = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            let kind = if grindstone_open {
                Kind::Grindstone
            } else {
                Kind::Cartography
            };
            (kind, v)
        } else if enchanting_open {
            // EnchantmentMenu: the item, the lapis, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::INPUT + 1)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Enchanting, v)
        } else if beacon_open {
            // BeaconMenu: the payment, then the inventory.
            let mut v = vec![Some(stations::INPUT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Beacon, v)
        } else if anvil_open || smithing_open {
            // ItemCombinerMenu: the inputs, the result, then the inventory.
            let inputs = if anvil_open { 2 } else { 3 };
            let mut v: Vec<Option<usize>> =
                (0..inputs).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (
                if anvil_open {
                    Kind::Anvil
                } else {
                    Kind::Smithing
                },
                v,
            )
        } else {
            let (kind, map) = if self.container.is_some() {
                let size = self.container_size();
                let (kind, count) = match self.container_kind() {
                    Some(BlockEntity::Chest(_)) => {
                        let shulker = self.container.is_some_and(|k| {
                            let id = self.world.block(k.0, k.1, k.2);
                            storage::is_shulker_box(&self.res.facts().get(id).name)
                        });
                        let kind = match size {
                            5 => Kind::Hopper,
                            9 => Kind::Dispenser,
                            27 if shulker => Kind::Shulker,
                            n => Kind::Chest((n / 9).clamp(1, 6)),
                        };
                        (kind, size)
                    }
                    Some(BlockEntity::Brewing(_)) => (Kind::Brewing, 5),
                    Some(BlockEntity::Furnace(f)) => (
                        Kind::Furnace(
                            match f.kind.recipe_type().trim_start_matches("minecraft:") {
                                "blasting" => "blasting",
                                "smoking" => "smoking",
                                _ => "smelting",
                            },
                        ),
                        3,
                    ),
                    _ => return false,
                };
                let map = Map {
                    container: count,
                    table: false,
                    cells: 0,
                    plain: false,
                };
                (kind, map)
            } else if self.station.is_none() && !self.mount_active() {
                let table = self.craft_size == 3;
                let map = Map {
                    container: 0,
                    table,
                    cells: self.craft_size * self.craft_size,
                    plain: true,
                };
                (
                    if table {
                        Kind::Crafting
                    } else {
                        Kind::Inventory
                    },
                    map,
                )
            } else {
                return false;
            };
            let total = Menu::new(kind).slot_count();
            plain_map = Some(map);
            (kind, (0..total).map(|i| map.to_game(i)).collect())
        };
        // The plain map also takes the crafting result slot (a shift-click on it).
        let to_menu = |g: usize| match plain_map {
            Some(map) => map.to_menu(g),
            None => game_of.iter().position(|x| *x == Some(g)),
        };
        let mut menu = Menu::new(kind);
        if matches!(kind, Kind::Mount(_)) {
            menu.mount = self.mount_menu_entity();
        }
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        let carried = self.carried.clone();
        let creative = self.creative;
        // CreativeModeInventoryScreen clicks the menu itself and sends every
        // changed slot, which the server sets by player.
        let equipment_before = (self.screen == Screen::Creative).then(|| self.player_equipment());
        let station_menu = self.station.as_ref().map(|s| {
            (
                s.loom.selected.map_or(-1, |i| i as i32),
                s.loom.patterns.clone(),
            )
        });
        let stone = self
            .station
            .as_ref()
            .map(|s| (s.selected.map_or(-1, |i| i as i32), s.stone.clone()));
        let crafter_data = match self.container_kind() {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.triggered)),
            _ => None,
        };
        // The merchant's session, lent to the menu for this click.
        let mut session = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map(|m| (m.session.clone(), m.mob));
        if let Some((s, mob)) = session.as_mut() {
            self.merchant_sync_in(s, *mob);
        }
        // The cartography table reads and makes the level's maps; a
        // server's (the client menu's ContainerLevelAccess.NULL) doesn't.
        let level_maps = (kind == Kind::Cartography && self.remote.is_none()).then(|| {
            let held: Vec<ItemStack> = before
                .iter()
                .flatten()
                .chain(self.inventory.iter().flatten())
                .chain(self.carried.iter())
                .cloned()
                .collect();
            self.level_maps(&held)
        });
        let next_map = if merchant_open { self.peek_map_id() } else { 0 };
        let tag = |t: &str| self.structure_tag(t);
        let maps = trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = horizon_client::merchant::TradeEnv {
            trades: &self.stations().trades,
            enchantments: &self.stations().enchantments,
            maps: Some(&maps),
        };
        let env = Env {
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: &self.stations().recipes,
            enchantments: &self.stations().enchantments,
            names: &self.pack.names,
            trade: Some(&trade_env),
            maps: level_maps
                .as_ref()
                .map(|m| m as &dyn horizon_client::menu::Maps),
        };
        menu.creative = creative;
        menu.selected = self.selected;
        menu.levels = self.experience.level;
        if anvil_open {
            // The rename box as AnvilScreen.onNameChanged gives it to the menu.
            menu.anvil.name = self.station.as_ref().map(|s| self.anvil_item_name(s));
        }
        for (i, stack) in before.iter().enumerate() {
            menu.set_slot(&env, i, stack.clone());
        }
        // The off hand is part of the inventory even when the menu has no slot for it.
        let offhand_mapped = game_of.contains(&Some(504));
        if !offhand_mapped {
            menu.set_player_item(40, self.offhand.clone());
        }
        menu.settle(&env);
        if kind == Kind::Loom
            && let Some((selected, patterns)) = station_menu
        {
            // The menu's state between clicks: what set_slot recomputed is replaced.
            menu.data[0] = selected;
            menu.loom_patterns = patterns;
            menu.set_slot(&env, 3, before[3].clone());
        }
        if kind == Kind::Stonecutter
            && let Some((selected, state)) = stone
        {
            // The same: the list and selection between clicks, then the result.
            menu.data[0] = selected;
            menu.stone = state;
            menu.set_slot(&env, 1, before[1].clone());
        }
        if kind == Kind::Enchanting
            && let Some(s) = self.station.as_ref()
        {
            // The offers between clicks (what copying the slots in flagged is not a change).
            menu.enchant_changed = false;
            for i in 0..3 {
                menu.data[i] = s.offers.costs[i];
                menu.data[7 + i] = s.offers.clues[i].as_ref().map_or(-1, |(_, l)| *l as i32);
            }
            menu.data[3] = self.level.xp_seed;
        }
        if let Some((disabled, triggered)) = crafter_data {
            for (i, d) in disabled.iter().enumerate() {
                menu.data[i] = i32::from(*d);
            }
            menu.data[9] = i32::from(triggered);
        }
        menu.carried = carried;
        menu.drops.clear();
        if kind == Kind::Merchant {
            // Attached after the slots were set: setting them is not a click.
            menu.merchant = session.map(|(s, _)| s);
        }
        // `FurnaceResultSlot.checkTakeAchievements` pays out on what left the output slot.
        let furnace_before = if matches!(kind, Kind::Furnace(_)) {
            menu.slot(2).cloned().map_or(0, |s| s.count)
        } else {
            0
        };
        let outcome = run(&mut menu, &env, &to_menu);
        let made = maps.made.take();
        match outcome {
            None => return false,
            Some(false) => return true,
            Some(true) => {}
        }
        // On a server the clicks only predict: the server drops, crafts and
        // makes maps itself.
        let predicting = self.remote.is_some();
        if !predicting {
            self.register_exploration_maps(made);
        }
        if let Some(s) = menu.merchant.take() {
            self.merchant_sync_out(s);
        }
        let after: Vec<Option<ItemStack>> = (0..total).map(|i| menu.slot(i).cloned()).collect();
        // Only a furnace has a result slot to compare: an enchanting table has two slots.
        let furnace_after = if matches!(kind, Kind::Furnace(_)) {
            after[2].as_ref().map_or(0, |s| s.count)
        } else {
            0
        };
        let carried = menu.carried.take();
        let dropped = std::mem::take(&mut menu.dropped);
        let sounds = std::mem::take(&mut menu.sounds);
        // A bundle's sounds are the player's own (`Entity.playSound`): the
        // client plays them as it clicks, predicting or not.
        for sound in std::mem::take(&mut menu.player_sounds) {
            self.player_menu_sound(sound);
        }
        let crafted = std::mem::take(&mut menu.crafted);
        let brewed = std::mem::take(&mut menu.brewed);
        let equipped = std::mem::take(&mut menu.equipped);
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        self.carried = carried;
        if !offhand_mapped {
            self.offhand = menu.player_item(40).cloned();
        }
        if !predicting {
            for stack in dropped {
                self.player_drop(stack);
            }
            self.award_crafted(&crafted);
            for stack in &brewed {
                self.trigger_brewed_potion(stack);
            }
            if furnace_before > furnace_after
                && self.remote.is_none()
                && let (Some(pos), Some(p)) = (self.container, self.player.as_ref())
            {
                let at = [p.position.x, p.position.y, p.position.z];
                self.award_furnace_recipes(pos, at);
            }
        } else if self.screen == Screen::Creative {
            // A THROW in the creative screen with an empty hand: on an item tab's hotbar
            // `AbstractContainerMenu.doClick` calls `Player.handleCreativeModeItemDrop`, on the
            // Survival Inventory tab the screen does. The stack goes to the server, the slot
            // follows with the others (SetCreativeModeSlot). Only the server can drop it; the
            // slot has already given it up.
            for stack in dropped {
                self.remote_creative_throw(stack);
            }
        }
        if kind == Kind::Loom {
            if let Some(s) = self.station.as_mut() {
                s.loom.selected = usize::try_from(menu.data[0]).ok();
                s.loom.patterns = std::mem::take(&mut menu.loom_patterns);
            }
            // A server's loom plays its own sounds (the client's menu has no level).
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
            self.loom_screen_changed();
        } else if kind == Kind::Cartography {
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        } else if kind == Kind::Stonecutter {
            if let Some(s) = self.station.as_mut() {
                s.selected = usize::try_from(menu.data[0]).ok();
                s.stone = std::mem::take(&mut menu.stone);
                s.stonecutter_changed();
            }
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        }
        if let Some(amount) = menu.ground.take().filter(|_| !predicting)
            && let Some(pos) = self.station.as_ref().map(|s| s.pos)
        {
            self.grindstone_taken(pos, amount);
        }
        if kind == Kind::Anvil || kind == Kind::Smithing {
            self.combiner_session_end(&menu, predicting);
        }
        if kind == Kind::Enchanting && (menu.enchant_changed || before[0] != menu.slot(0).cloned())
        {
            // EnchantmentMenu.slotsChanged: new offers for the item.
            self.station_changed();
        }
        if kind == Kind::CrafterBlock
            && let Some(BlockEntity::Crafter(c)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            for (i, d) in c.disabled.iter_mut().enumerate() {
                *d = menu.data[i] == 1;
            }
        }
        if matches!(kind, Kind::Furnace(_))
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
            f.input_clicked(world_recipes!(self), before[0].clone());
        }
        match equipment_before {
            Some(before) => self.player_equipment_changed(&before),
            None => {
                for (slot, stack) in equipped {
                    self.player_equipped(slot, stack.as_ref());
                }
            }
        }
        self.apply_equipment();
        if let Some(maps) = level_maps {
            self.register_made_maps(maps);
        }
        true
    }
    /// `CreativeModeInventoryScreen.keyPressed` on the search tab: a hotbar
    /// key over a slot swaps before the search box sees it.
    fn search_tab_hotbar_key(&mut self, code: KeyCode) -> bool {
        use horizon_client::creative::{Hover, search_key_swaps};
        let hotbar_key = HOTBAR_DIGITS.contains(&code);
        let numeric = hotbar_key || NUMERIC_KEYS.contains(&code);
        if !numeric {
            return false;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size);
        let hover = match self.slot_at(&batch, self.cursor.0, self.cursor.1) {
            None => Hover::Nothing,
            Some(slot) if slot >= Creative::DISPLAY => {
                let first = self.creative_screen.first_row(&self.pack.names);
                Hover::Display {
                    filled: self
                        .creative_screen
                        .shown(&self.pack.names, first * 9 + slot - Creative::DISPLAY)
                        .is_some(),
                }
            }
            Some(_) => Hover::Slot,
        };
        search_key_swaps(numeric, hotbar_key, hover, self.carried.is_some())
            && self.inventory_hotkey(code)
    }
    /// Number keys, F and Q over a slot of the open menu: SWAP with a hotbar
    /// slot or the off hand, THROW one or the whole stack.
    fn inventory_hotkey(&mut self, key: KeyCode) -> bool {
        use horizon_client::menu::Input;
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return false;
        };
        let digit = HOTBAR_DIGITS.iter().position(|k| *k == key);
        let done = match (digit, key) {
            (Some(n), _) => self.menu_click(Some(slot), n as i32, Input::Swap),
            (None, KeyCode::KeyF) => self.menu_click(Some(slot), 40, Input::Swap),
            (None, KeyCode::KeyQ) => {
                let all = drop_whole_stack(self.modifiers);
                self.menu_click(Some(slot), i32::from(all), Input::Throw)
            }
            _ => false,
        };
        if done && let Some(key) = self.container {
            self.container_changed(key);
        }
        done
    }
    /// The pick block key bound to a keyboard key, over a slot of the open
    /// menu (`AbstractContainerScreen.keyPressed`): `CLONE` with button 0. The
    /// search tab's box takes the key instead.
    fn inventory_pick_key(&mut self) {
        let menu = (self.screen == Screen::Inventory
            || (self.screen == Screen::Creative && !self.creative_screen.tab().search))
            && !self.dialog_open();
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size);
        let Some(slot) = self
            .slot_at(&batch, self.cursor.0, self.cursor.1)
            .filter(|_| menu)
        else {
            return;
        };
        if self.menu_click(Some(slot), 0, horizon_client::menu::Input::Clone)
            && let Some(key) = self.container
        {
            self.container_changed(key);
        }
    }
    /// The plain inventory mouse press: starts a quick craft drag when a
    /// stack is held, gathers on a double click, else the normal click.
    fn inventory_press(&mut self, right: bool) {
        use horizon_client::container_screen::Target;
        use horizon_client::menu::Input;
        if self.mod_browser_press(right) {
            return;
        }
        if !right && self.sort_press() {
            return;
        }
        if self.native_screen_press(right) {
            self.inv_clicks.skip_release();
            return;
        }
        // AbstractRecipeBookScreen lets the book handle its buttons before
        // AbstractContainerScreen starts a drag or chooses an outside slot.
        // Toggling it can move the inventory under the pointer before release.
        if self.recipe_book_click(right) {
            self.inv_clicks.skip_release();
            return;
        }
        let (slot, outside) = self.inventory_target();
        let shift = self.has_shift_down();
        let target = Target::of(slot, outside);
        let input = self.inv_clicks.press(
            &target,
            Self::screen_button(right),
            self.carried.is_none(),
            shift,
            false,
        );
        if outside {
            // slotId -999: an empty hand throws nothing; a carried stack
            // starts a drag whose release drops it.
            match input {
                Some(input) => self.throw_click(right, input),
                None if self.inv_clicks.quick_crafting().is_some() => {
                    self.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => {}
            }
            return;
        }
        if self.tweak_press(slot, right, shift) {
            return;
        }
        if let Some(slot) = slot
            && shift
            && slot != 200
        {
            let now = std::time::Instant::now();
            let double = self.last_inv_click.is_some_and(|(at, last, was_right)| {
                last == slot && was_right == right && now.duration_since(at).as_millis() < 250
            });
            self.last_inv_click = Some((now, slot, right));
            // mouseClicked's QUICK_MOVE (and Horizon's at-press QUICK_MOVE
            // of a carried stack, mouseReleased's in vanilla) remember the
            // stack it moves.
            if input == Some(Input::QuickMove) || (self.carried.is_some() && !double) {
                self.last_quick_moved = self.slot_view(slot);
            }
            if double && !right {
                // mouseReleased: every stack of lastQuickMoved's item in the
                // clicked slot's container is quick-moved.
                if let Some(input) = input
                    && !self.menu_click(Some(slot), 0, input)
                {
                    // A screen the menu engine doesn't model: its own click.
                    self.click_inventory(right);
                    return;
                }
                if let Some(moved) = self.last_quick_moved.clone() {
                    // The creative screen clicks its own menu, on a server too.
                    if self.remote.is_some() && self.screen != Screen::Creative {
                        self.remote_quick_move_all(slot, &moved);
                    } else {
                        self.menu_session(|menu, env, to_menu| {
                            Some(menu.quick_move_all(env, to_menu(slot)?, &moved).is_ok())
                        });
                        if self.remote.is_some() {
                            self.send_creative_slots();
                        }
                    }
                }
                self.last_inv_click = None;
                self.inv_clicks.skip_release();
                return;
            }
        }
        if let Some(slot) = slot
            && slot != 200
        {
            // An item tab's display slots take no drags or PICKUP_ALL.
            let display = slot >= Creative::DISPLAY;
            let now = std::time::Instant::now();
            let double = !shift
                && !right
                && !display
                && self.carried.is_some()
                && self.last_inv_click.is_some_and(|(at, last, was_right)| {
                    last == slot && !was_right && now.duration_since(at).as_millis() < 250
                });
            self.last_inv_click = Some((now, slot, right));
            if double && self.menu_click(Some(slot), 0, Input::PickupAll) {
                self.last_inv_click = None;
                // The double click's own release (`mouseReleased`'s PICKUP_ALL).
                self.inv_clicks.skip_release();
                return;
            }
            if self.carried.is_some() {
                let ty = i32::from(right);
                // `mouseClicked` starts a quick craft drag whenever the hand holds a stack,
                // Shift or not: `mouseDragged` fills the slots the cursor crosses, so a
                // Shift held over the drag spreads the same way (#4166). The drag starts
                // empty on a Shift press: a release with nothing spread is `mouseReleased`'s
                // click on the hovered slot (Pickup, or QuickMove under Shift).
                if !shift && !display && self.quick_craft_accepts(slot, ty, 0) {
                    self.inv_drag = Some((ty, vec![slot]));
                } else {
                    self.inv_drag = Some((ty, Vec::new()));
                }
                return;
            }
        }
        if self.creative_tab_open() {
            // CreativeModeInventoryScreen goes through the same mouseClicked:
            // a click now, or a drag that its release ends.
            match input {
                Some(input) => {
                    self.menu_click(slot, i32::from(right), input);
                }
                None if self.inv_clicks.quick_crafting().is_some() => {
                    self.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => {}
            }
            return;
        }
        // mouseClicked: an empty hand's PICKUP is the menu's click at once
        // (a right click on a bundle takes its first stack out).
        if let (Some(input), Some(_)) = (input, slot)
            && self.remote.is_none()
            && self.menu_click(slot, i32::from(right), input)
        {
            if let Some(key) = self.container {
                self.container_changed(key);
            }
            return;
        }
        self.click_inventory(right);
    }
    /// Mouse movement while a quick craft drag is running.
    fn inventory_drag_move(&mut self) {
        if self.tweaks_on() && self.tweak_drag_move(self.inventory_target().0) {
            return;
        }
        let Some((ty, slots)) = self.inv_drag.clone() else {
            return;
        };
        let Some(r) = self.renderer.as_ref() else {
            return;
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        if let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1)
            && slot != 200
            && slot < Creative::DISPLAY
            && !slots.contains(&slot)
            && self.quick_craft_accepts(slot, ty, slots.len())
            && let Some((_, slots)) = self.inv_drag.as_mut()
        {
            slots.push(slot);
        }
    }
    /// The hovered slot and whether the cursor is outside the window.
    fn inventory_target(&self) -> (Option<usize>, bool) {
        let Some(r) = self.renderer.as_ref() else {
            return (None, false);
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        (
            self.slot_at(&batch, self.cursor.0, self.cursor.1),
            self.clicked_outside(&batch),
        )
    }
    fn screen_button(right: bool) -> horizon_client::container_screen::Button {
        use horizon_client::container_screen::Button;
        if right { Button::Right } else { Button::Left }
    }
    /// `Screen.hasShiftDown` / `InputWithModifiers.hasShiftDown`: 26.3 keeps
    /// no key cache — every event answers with its own modifiers bit
    /// (`InputWithModifiers.java:54-56`, `keys = 3`), and the pointer and
    /// keyboard events carry the OS's live word, refreshed at the focus. The
    /// window's `ModifiersChanged` is that word here; key releases can die
    /// on the way from the OS (a stuck Shift then kept every screen's Shift
    /// and a sneak the key no longer held: #4166's in-game report), so a
    /// cached key set is not vanilla's answer.
    pub(super) fn has_shift_down(&self) -> bool {
        self.modifiers.shift_key()
    }
    /// The same live word for Control (`InputWithModifiers.hasControlDown`).
    pub(super) fn has_control_down(&self) -> bool {
        self.modifiers.control_key()
    }
    /// Real events word the modifiers by their own `ModifiersChanged`; a
    /// control socket's `key` is synthesized, so it words the bit directly
    /// and every reader can ask the one live state.
    fn press_modifier_key(&mut self, key: KeyCode, down: bool) {
        horizon_client_input::modifiers::set_key(
            &mut self.modifiers,
            &mut self.physical_keys,
            key,
            down,
        );
    }
    /// `hasClickedOutside` of the open screen at the cursor.
    fn clicked_outside(&self, batch: &HudBatch) -> bool {
        use horizon_client::container_screen::{Book, clicked_outside};
        let image = self.container_image(batch);
        let book = self
            .recipe_book_area(batch)
            .map(|(visible, tab_hovered)| Book {
                visible,
                selected_tab_hovered: tab_hovered,
            });
        clicked_outside(
            f64::from(self.cursor.0),
            f64::from(self.cursor.1),
            image,
            book,
        )
    }
    /// The open container screen's window: its place and size.
    fn container_image(&self, batch: &HudBatch) -> horizon_client::container_screen::Image {
        let (ox, oy) = self.inventory_origin(batch);
        let (shift, width, height) = if self.screen == Screen::Creative {
            (0.0, Creative::WIDTH as i32, Creative::HEIGHT as i32)
        } else {
            let (shift, width) = match self.station.as_ref().map(|s| s.kind) {
                Some(stations::Kind::Merchant) => (-50.0, 276),
                Some(stations::Kind::Beacon) => (beacon::SHIFT, 230),
                _ => (0.0, self.native_screen_width().map_or(176, |w| w as i32)),
            };
            (shift, width, self.screen_height() as i32)
        };
        horizon_client::container_screen::Image {
            left: (ox + shift) as i32,
            top: oy as i32,
            width,
            height,
        }
    }
    /// A click outside the window (slot id -999): `THROW` does nothing; a
    /// `PICKUP` drops the carried stack (left) or one of it (right), thrown
    /// from the hand (`AbstractContainerMenu.doClick`, `LivingEntity.drop`).
    fn throw_click(&mut self, right: bool, input: horizon_client::menu::Input) {
        use horizon_client::menu::Input;
        let button = i32::from(right);
        if self.remote.is_some() {
            if self.screen == Screen::Creative {
                self.remote_creative_drop(right);
            } else {
                self.remote_throw(right, input);
            }
            return;
        }
        let before = self.carried.clone();
        if !self.menu_click(None, button, input) && input == Input::Pickup {
            // A screen the menu engine doesn't model: the same doClick.
            if let Some(c) = self.carried.as_mut() {
                let thrown = if right {
                    let one = ItemStack {
                        count: 1,
                        ..c.clone()
                    };
                    c.count -= 1;
                    if c.count == 0 {
                        self.carried = None;
                    }
                    one
                } else {
                    self.carried.take().expect("some")
                };
                self.player_drop(thrown);
            }
        }
        if self.carried != before {
            self.swing_arm();
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
    }
    /// Mouse release over an inventory screen: `mouseReleased` ends or
    /// cancels the drag, drops a stack released outside, or clicks.
    fn inventory_mouse_release(&mut self, right: bool) {
        use horizon_client::container_screen::{Release, Target};
        self.tweak_release();
        let (slot, outside) = self.inventory_target();
        let dragged = self.inv_drag.as_ref().is_some_and(|(_, s)| !s.is_empty());
        let release = self.inv_clicks.release(
            &Target::of(slot, outside),
            Self::screen_button(right),
            self.carried.is_none(),
            dragged,
            self.has_shift_down(),
            false,
        );
        match (release, Target::of(slot, outside)) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Target::Outside) => {
                self.inv_drag = None;
                self.throw_click(right, input);
            }
            (Release::Click(input), Target::Slot(slot)) if self.inv_drag.is_some() => {
                // The drag never reached a slot: a plain click where it ends.
                self.inv_drag = None;
                if !self.menu_click(Some(slot), i32::from(right), input) {
                    self.click_inventory(right);
                }
                if let Some(key) = self.container {
                    self.container_changed(key);
                }
            }
            _ => self.inv_drag = None,
        }
    }
    /// A press of the pick block button (`keyPickItem`, the middle button by
    /// default) over an inventory screen: `mouseClicked` with `cloning`
    /// (infinite materials). An empty hand clones the hovered stack at its
    /// full size (`CLONE`), a carried stack starts a quick craft drag of type
    /// 2 that fills every slot it crosses; without infinite materials the
    /// press does nothing.
    fn inventory_pick_press(&mut self) {
        use horizon_client::container_screen::{Button, Target};
        let (slot, outside) = self.inventory_target();
        let was_crafting = self.inv_clicks.quick_crafting().is_some();
        let input = self.inv_clicks.press(
            &Target::of(slot, outside),
            Button::Middle,
            self.carried.is_none(),
            self.has_shift_down(),
            self.creative,
        );
        match (input, slot) {
            // Slot -999 (outside the window) clones nothing.
            (Some(input), Some(slot)) if !outside => {
                self.menu_click(Some(slot), 2, input);
            }
            (None, _) if !was_crafting && self.inv_clicks.quick_crafting().is_some() => {
                // Item tab display slots take no drags.
                let first = slot.filter(|&s| {
                    s != 200 && s < Creative::DISPLAY && self.quick_craft_accepts(s, 2, 0)
                });
                self.inv_drag = Some((2, first.into_iter().collect()));
            }
            _ => {}
        }
    }
    /// The pick block button's release (`mouseReleased`): ends its quick
    /// craft drag, cancels another button's, or clones onto the slot under a
    /// carried stack (`CLONE`, which only an item tab's display slot takes).
    fn inventory_pick_release(&mut self) {
        use horizon_client::container_screen::{Button, Release, Target};
        let (slot, outside) = self.inventory_target();
        let dragged = self.inv_drag.as_ref().is_some_and(|(_, s)| !s.is_empty());
        let release = self.inv_clicks.release(
            &Target::of(slot, outside),
            Button::Middle,
            self.carried.is_none(),
            dragged,
            self.has_shift_down(),
            true,
        );
        match (release, slot) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Some(slot)) if !outside => {
                self.inv_drag = None;
                self.menu_click(Some(slot), 2, input);
            }
            _ => self.inv_drag = None,
        }
    }
    /// Mouse release: finishes a quick craft drag (one slot is a plain click).
    fn inventory_release(&mut self) {
        use horizon_client::menu::Input;
        let Some((ty, slots)) = self.inv_drag.take() else {
            return;
        };
        if slots.len() == 1 {
            if !self.menu_click(Some(slots[0]), ty, Input::Pickup) {
                self.click_inventory(ty == 1);
            }
        } else {
            let mask = |header: i32| header | (ty << 2);
            let mut ops = vec![(None, mask(0), Input::QuickCraft)];
            ops.extend(
                slots
                    .into_iter()
                    .map(|slot| (Some(slot), mask(1), Input::QuickCraft)),
            );
            ops.push((None, mask(2), Input::QuickCraft));
            if !self.menu_ops(&ops) {
                // Not a menu the engine models: the drag is a plain click.
                self.click_inventory(ty == 1);
            }
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
    }
    fn click_inventory(&mut self, right: bool) {
        if self.mod_browser_press(right) || self.mount_click(right) {
            return;
        }
        if self.recipe_book_click(right) {
            return;
        }
        if self.station_click(right) {
            return;
        }
        if self.remote.is_some() {
            return self.remote_click(right);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let held_shift = self.has_shift_down();
        if self.crafter_click(slot, held_shift, right) {
            return;
        }
        if held_shift && self.container.is_some() && self.container_quick_move(slot) {
            if let Some(key) = self.container {
                self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            }
            return;
        }
        if let Some(c) = &self.carried
            && self.refuses(slot, c)
        {
            return;
        }
        let max = |items: &Items, s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let shift = held_shift;
        if slot >= 300
            && let Some(key) = self.container
        {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        let is_furnace = matches!(self.container_kind(), Some(BlockEntity::Furnace(_)));
        if is_furnace && slot == 302 {
            // FurnaceResultSlot: take only.
            let Some(result) = self.slot_mut(302).take() else {
                return;
            };
            match &mut self.carried {
                None => self.carried = Some(result),
                Some(c)
                    if c.item == result.item
                        && c.count + result.count <= max(world_items!(self), c) =>
                {
                    c.count += result.count
                }
                _ => *self.slot_mut(302) = Some(result),
            }
            return;
        }
        // Changing the input restarts cooking (AbstractFurnaceBlockEntity.setItem).
        let furnace_input = (is_furnace && slot == 300).then(|| self.slot_mut(300).clone());
        if slot == 200 {
            // ResultSlot: take the crafted stack and consume one of each ingredient.
            let mut crafted = 0;
            while let Some((recipe, result)) = self.craft_recipe() {
                let made = (result.item.clone(), result.count);
                let cells = self.craft_size * self.craft_size;
                let ingredients = self.craft_grid[..cells].to_vec();
                let remainders = world_recipes!(self).remainders(
                    &recipe,
                    &self.craft_grid[..cells],
                    self.craft_size,
                );
                if shift {
                    if self.give(result).is_some() {
                        break;
                    }
                } else {
                    match &mut self.carried {
                        None => self.carried = Some(result),
                        Some(c)
                            if c.item == result.item
                                && c.count + result.count <= max(world_items!(self), c) =>
                        {
                            c.count += result.count
                        }
                        _ => return,
                    }
                }
                for cell in self.craft_grid.iter_mut() {
                    if let Some(s) = cell {
                        s.count -= 1;
                        if s.count == 0 {
                            *cell = None;
                        }
                    }
                }
                for (i, rest) in remainders.into_iter().enumerate() {
                    let Some(rest) = rest else { continue };
                    match &mut self.craft_grid[i] {
                        cell @ None => *cell = Some(rest),
                        Some(c) if c.same_item_same_components(&rest) => c.count += rest.count,
                        _ => self.give_or_drop(rest),
                    }
                }
                crafted += 1;
                self.stats.add("crafted", &made.0, made.1 as i32);
                self.award_crafted(&[(format!("minecraft:{recipe}"), ingredients)]);
                if !shift || crafted >= 64 {
                    break;
                }
            }
            return;
        }
        if shift
            && (slot < 36 || (500..504).contains(&slot) || (100..109).contains(&slot))
            && self.inventory_quick_move(slot)
        {
            return;
        }
        if (500..504).contains(&slot) {
            let wanted = ["head", "chest", "legs", "feet"][slot - 500];
            let fits = self.carried.as_ref().is_none_or(|c| {
                self.res
                    .items()
                    .get(&c.item)
                    .and_then(|i| i.equip_slot.as_deref())
                    == Some(wanted)
            });
            if !fits {
                return;
            }
            std::mem::swap(&mut self.carried, &mut self.armor[slot - 500]);
            self.apply_equipment();
            return;
        }
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let items = self.items();
        let (new_slot, new_carried) = match (carried, current) {
            (None, Some(mut s)) if right => {
                let half = s.count.div_ceil(2);
                s.count -= half;
                let taken = ItemStack {
                    count: half,
                    ..s.clone()
                };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (None, s) => (None, s),
            (Some(mut c), None) if right => {
                let one = ItemStack {
                    count: 1,
                    ..c.clone()
                };
                c.count -= 1;
                (Some(one), (c.count > 0).then_some(c))
            }
            (Some(c), None) => (Some(c), None),
            (Some(mut c), Some(mut s)) if c.same_item_same_components(&s) => {
                let room = max(items, &s).saturating_sub(s.count);
                let moved = if right {
                    1.min(room).min(c.count)
                } else {
                    room.min(c.count)
                };
                s.count += moved;
                c.count -= moved;
                (Some(s), (c.count > 0).then_some(c))
            }
            (Some(c), Some(s)) => (Some(c), Some(s)),
        };
        self.carried = new_carried;
        *self.slot_mut(slot) = new_slot;
        if let Some(before) = furnace_input
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            f.input_clicked(world_recipes!(self), before);
        }
    }
    /// `Camera.setRotation`: the camera's orientation, turned round in
    /// third-person front view.
    pub(super) fn camera_orientation(&self) -> glam::Quat {
        camera_rotation(self.camera_type, self.yaw, self.pitch)
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        mut out: Vec<Vertex>,
    ) -> Vec<Vertex> {
        self.entity_spans.borrow_mut().clear();
        // LevelExtractor.extract: the transient blocks queued since the last frame.
        self.drain_transient_block_queue();
        self.prepare_entity_culling(camera);
        self.prepare_trims();
        self.mark_entity(out.len(), "minecraft:entity_flame");
        // One scan for both passes that draw them (block entities below and the
        // overlay near the end of this function).
        let cracks = self.cracks(camera);
        let fire_yaw = if self.camera_type == perspective::CameraType::ThirdPersonFront {
            self.yaw + 180.0
        } else {
            self.yaw
        };
        for m in &self.mobs {
            // Blaze.isOnFire: a charged blaze burns.
            let blazing = m.species() == horizon_client::mobs::Species::Blaze && m.extra.flag;
            if (m.is_on_fire() || blazing) && !m.removed {
                let at = m.previous.lerp(m.position, alpha);
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    m.width(),
                    m.height(),
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        if self.own_body_in_view()
            && let Some(p) = &self.player
            && p.fire_ticks > 0
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            horizon_client::fire_render::flames(
                [feet.x, feet.y, feet.z],
                p.dimensions.width,
                p.dimensions.height,
                camera,
                fire_yaw,
                &self.gui.atlas.sprites,
                self.world.light(
                    feet.x.floor() as i32,
                    feet.y.floor() as i32,
                    feet.z.floor() as i32,
                ),
                &mut out,
            );
        }
        for item in &self.dropped {
            if item.fire_ticks > 0
                && !self
                    .res
                    .items()
                    .resists_fire(&item.stack, "minecraft:in_fire")
            {
                let at = item.previous.lerp(item.position, alpha);
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.25,
                    0.25,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        for stand in &self.armor_stands {
            if stand.fire_ticks > 0 && !stand.marker {
                let scale = if stand.small { 0.5 } else { 1.0 };
                let at = stand.render_position(alpha);
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.5 * scale,
                    1.975 * scale,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        // `ItemEntityRenderer`.
        self.mark_entity(out.len(), "minecraft:item");
        self.dropped_item_geometry(camera, alpha, &mut out);
        // `VaultRenderer`.
        self.mark_entity(out.len(), "");
        self.vault_geometry(camera, alpha, &mut out);
        // `CampfireRenderer`.
        self.mark_entity(out.len(), "");
        self.campfire_geometry(camera, &mut out);
        self.brushable_geometry(camera, &mut out);
        self.shelf_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:ominous_item_spawner");
        self.ominous_item_spawner_geometry(camera, alpha, &mut out);
        // Mobs and their projectiles (a span each).
        self.mob_geometry(camera, alpha, &mut out);
        // In first person only a shader pack's shadow pass draws the player.
        self.mark_player(out.len(), !self.own_body_in_view());
        let player_start = out.len();
        self.player_model_geometry(camera, self.clock.camera_fraction(), &mut out);
        // A native pack has no per-entity draws to hide it with: its shadow pass alone gets
        // the first-person model.
        if self.pack_ids.is_none() && !self.own_body_in_view() {
            self.shadow_only_vertices
                .borrow_mut()
                .extend(out.drain(player_start..));
        }
        self.mark_entity(out.len(), "minecraft:player");
        self.guest_model_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:evoker_fangs");
        self.fangs_geometry(camera, alpha, &mut out);
        self.block_item_display_geometry(camera, alpha, &mut out);
        let mut see_through = std::mem::take(&mut self.see_through_vertices);
        see_through.clear();
        self.name_tag_geometry(camera, alpha, &mut out, &mut see_through);
        self.test_instance_marker_geometry(camera, &mut out, &mut see_through);
        self.debug_renderer_geometry(camera, &mut out);
        self.text_display_geometry(camera, alpha, &mut out, &mut see_through);
        self.see_through_vertices = see_through;
        // Entity shadows (EntityRenderer.extractShadow).
        self.mark_translucent_entity(out.len(), "minecraft:entity_shadow");
        // A native shader pack's sun shadows already fall under the entities in the Overworld:
        // the round blob would be a second one. The Nether and the End have no sun to cast them.
        let pack_shadows =
            self.dim == Dim::Overworld && self.renderer.as_ref().is_some_and(|r| r.native_shadow());
        let shadow_sprite = (self.settings.entity_shadows && !pack_shadows)
            .then(|| self.gui.atlas.sprites.get("misc/shadow").copied())
            .flatten();
        if let Some(sprite) = shadow_sprite {
            // DimensionType.ambientLight: the_nether 0.1, the_end 0.25.
            let ambient = match self.dim {
                Dim::Nether => 0.1,
                Dim::End => 0.25,
                Dim::Overworld => 0.0,
            };
            let darken = self.sky_darken();
            let a = alpha;
            let lerp = |p: Vec3, q: Vec3| {
                glam::DVec3::new(
                    p.x + (q.x - p.x) * a,
                    p.y + (q.y - p.y) * a,
                    p.z + (q.z - p.z) * a,
                )
            };
            let mut casters: Vec<(glam::DVec3, f32, f32)> =
                horizon_client::shadows::mob_casters(&self.mobs, a).collect();
            let item = horizon_client::shadows::shadow_params("minecraft:item");
            casters.extend(
                self.dropped
                    .iter()
                    .map(|i| (lerp(i.previous, i.position), item.0, item.1)),
            );
            let orb = horizon_client::shadows::shadow_params("minecraft:experience_orb");
            casters.extend(
                self.orbs
                    .iter()
                    .map(|o| (lerp(o.previous, o.position), orb.0, orb.1)),
            );
            // Text, block and item displays (DisplayRenderer's shadow radius).
            casters.extend(self.text_displays.values().filter_map(|d| d.shadow(a)));
            for (pos, radius, strength) in casters {
                horizon_client::shadows::shadow(
                    &self.world,
                    self.res.facts(),
                    &|state| {
                        self.res
                            .collision()
                            .cached_shapes()
                            .cached_shape(state)
                            .ok()
                            .flatten()
                            .and_then(|shape| shape.bounds())
                    },
                    sprite,
                    camera,
                    pos,
                    radius,
                    strength,
                    ambient,
                    darken,
                    &mut out,
                );
            }
        }
        // Block entities (BlockEntityRenderers): chests, shulker boxes, banners, skulls...
        if let Some(r) = &self.renderer {
            let lid = |p: [i32; 3]| self.chest_lid(p, alpha as f32);
            let anim = |p: [i32; 3]| self.block_anim(p, alpha as f32);
            let scene = horizon_client::block_entity_render::Scene {
                camera,
                orientation: self.camera_orientation(),
                anim: &anim,
                texts: None,
                partial: alpha as f32,
                game_time: self.world.game_time,
                max_y: self.world.shape.max_y() - 1,
                world: &self.world,
                sprites: &self.gui.atlas.sprites,
                nether: self.dim == Dim::Nether,
                lid: &lid,
                sign: &|p| match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(BlockEntity::Sign(sign)) => Some(sign),
                    _ => None,
                },
                banner: &|p| self.banner_layers_at(p),
                skull: &|p| self.skull_skin_at(p),
                glyph: &|c| self.gui.atlas.glyph_cell(c),
                glyphs: self.gui.atlas.font.epoch(),
            };
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| {
                let name = &*self.res.facts().get(BlockStateId(state)).name;
                matches!(
                    name,
                    "minecraft:beacon"
                        | "minecraft:end_gateway"
                        | "minecraft:structure_block"
                        | "minecraft:test_instance_block"
                )
            };
            let in_view = |p: &[i32; 3]| {
                view.is_none_or(|v| {
                    let o = (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                        - camera)
                        .as_vec3();
                    v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            };
            let visible =
                r.visible_block_entities(self.settings.chunk_fade)
                    .filter(|(p, state)| {
                        self.res.block_entity_render().renders(*state)
                            && (off_screen(*state) || in_view(p))
                    });
            if self.entity_culler.is_some() || self.pack_ids.is_some() {
                for block in visible {
                    let start = out.len();
                    self.mark_block_entity(start, block.1);
                    self.res
                        .block_entity_render()
                        .render(&scene, std::iter::once(block), &mut out);
                    if !off_screen(block.1) && self.entity_culler.is_some() {
                        self.cull_entity_vertices(&mut out, start);
                    }
                }
            } else {
                self.res
                    .block_entity_render()
                    .render(&scene, visible, &mut out);
            }
            // Mod block entities: their mods draw them (`native_block_entity_submit`),
            // culled by the bounds they give.
            let mods = r
                .visible_block_entities(self.settings.chunk_fade)
                .filter(|(_, state)| !self.res.block_entity_render().renders(*state));
            self.mod_block_entity_geometry(mods, camera, alpha as f32, &mut out);
            self.mark_stage(out.len(), "DESTROY");
            for &(pos, stage) in &cracks {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                self.res
                    .block_entity_render()
                    .render_breaking(&scene, pos, state.0, stage, &mut out);
            }
        }
        // Experience orbs (ExperienceOrbRenderer): pulsing billboards from a 4x4 icon sheet.
        self.mark_entity(out.len(), "minecraft:experience_orb");
        if let Some(r) = self
            .gui
            .atlas
            .sprites
            .get("entity/experience/experience_orb")
            .copied()
        {
            let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
            let forward = glam::Vec3::new(
                -yaw.sin() * pitch.cos(),
                -pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
            let up = right.cross(forward).normalize_or_zero();
            let live = self.orbs.iter().map(|o| {
                let at = [
                    o.previous.x + (o.position.x - o.previous.x) * alpha,
                    o.previous.y + (o.position.y - o.previous.y) * alpha,
                    o.previous.z + (o.position.z - o.previous.z) * alpha,
                ];
                (o, at, alpha as f32)
            });
            // ItemPickupParticle: the render state extracted at partial tick 1.
            let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
                pickups::Taken::Orb(o) => Some((&**o, pickups::position(p, alpha), 1.0)),
                _ => None,
            });
            for (o, at, partial) in live.chain(taken) {
                let p = glam::DVec3::new(at[0], at[1] + 0.1, at[2]) - camera;
                let icon = o.icon();
                let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
                let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
                let t = (o.age as f32 + partial) / 2.0;
                let red = ((t.sin() + 1.0) * 0.5 * 255.0) as u32;
                let blue = (((t + 4.188_79).sin() + 1.0) * 0.1 * 255.0) as u32;
                let color = 0x8000_0000 | blue << 16 | 255 << 8 | red;
                let light = self.world.light(
                    o.position.x.floor() as i32,
                    o.position.y.floor() as i32,
                    o.position.z.floor() as i32,
                );
                let packed = (u32::from(light >> 4).max(10) * 16)
                    | ((u32::from(light & 15).max(10) * 16) << 8);
                let c = p.as_vec3();
                let s = 0.15;
                for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .iter()
                    .zip([[u0, v0 + dv], [u0 + du, v0 + dv], [u0 + du, v0], [u0, v0]])
                {
                    let v = c + right * (k.0 * s) + up * (k.1 * s);
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: v.to_array(),
                        uv,
                        // ExperienceOrbRenderer: `RenderTypes.entityTranslucent`; in HDR
                        // output the orbs shine (entity.wgsl fs_hdr_mark).
                        layer: horizon_client::gfx::OIT_ENTITY
                            | u32::from(horizon_client::mesher::VERY_BRIGHT) << 16,
                        color,
                        light: packed,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
        // Falling blocks use their baked model, including non-cubic shapes.
        self.mark_entity(out.len(), "minecraft:falling_block");
        for f in &self.falling {
            // `FallingBlockEntity.blockPosition()`: the cell its feet are in.
            let (x, y, z) = (
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            );
            if !falling_block_visible(f.state, self.world.block(x, y, z)) {
                continue;
            }
            let Some((_, tint)) = self.block_face_sprite(f.state) else {
                continue;
            };
            let tables = &self.pack.context.tables;
            let p = f.model_origin(alpha, |state, x, z| tables.offset(state, x, z)) - camera;
            let o = p.as_vec3();
            let light = self.world.light(x, y + 1, z);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let start = out.len();
            self.block_model_vertices(
                f.state,
                horizon_core::random::position_seed(f.start_pos),
                glam::Mat4::from_translation(o),
                packed,
                false,
                &mut out,
            );
            let tint = horizon_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = horizon_client::hud_layout::multiply(vertex.color, tint);
            }
        }
        self.mark_entity(out.len(), "");
        self.moving_piston_geometry(camera, alpha, &mut out);
        // LevelRenderer.submitTransientBlocks.
        self.transient_block_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:tnt");
        self.tnt_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:painting");
        self.painting_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:item_frame");
        self.frame_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:armor_stand");
        self.armor_stand_geometry(camera, alpha, &mut out);
        self.cushion_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:oak_boat");
        self.boat_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:minecart");
        self.cart_geometry(camera, alpha, &mut out);
        // Beams: `gbuffers_beaconbeam`, not a block entity's program (5.4 has no stage for them).
        self.mark_stage(out.len(), "NONE");
        self.beacon_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:fishing_bobber");
        self.fishing_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:leash_knot");
        self.leash_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:trident");
        self.trident_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:firework_rocket");
        self.rocket_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:splash_potion");
        self.thrown_potion_geometry(camera, alpha, &mut out);
        // Particles (ParticleEngine.extract): camera-facing quads.
        self.mark_stage(out.len(), "PARTICLES");
        let world = &self.world;
        self.particles.render(
            camera,
            self.yaw,
            self.pitch,
            alpha as f32,
            &|b| world.light(b[0], b[1], b[2]),
            &mut out,
        );
        // Breaking progress overlays (destroy_stage_0..9): ours and the other players'.
        self.mark_stage(out.len(), "DESTROY");
        for &(pos, stage) in &cracks {
            let o = (glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - camera)
                .as_vec3();
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if self.res.facts().get(state).render_shape
                == horizon_client::blocks::RenderShape::Model
                && let Ok(models) = self
                    .pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(pos[0], pos[1], pos[2]))
            {
                let offset = self
                    .pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3]);
                let origin = o + glam::DVec3::from(offset).as_vec3();
                let translucent = models
                    .iter()
                    .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                for (_, model) in models {
                    horizon_client::breaking::geometry(model, origin, translucent, stage, &mut out);
                }
            }
        }
        out
    }

    /// `LevelRenderer.destroyBlockProgress`: a player's crack on a block (0..=9), or none
    /// (any other progress). Cracks not updated for 400 ticks are dropped.
    pub(crate) fn destroy_block_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        let now = self.world.game_time;
        self.destroying
            .retain(|_, c| now.saturating_sub(c.2) <= 400);
        if (0..10).contains(&progress) {
            self.destroying.insert(breaker, (pos, progress as u8, now));
        } else {
            self.destroying.remove(&breaker);
        }
    }

    /// The cracks to draw and their stage sprite's atlas origin: our own mining and the
    /// other players' (`renderBlockDestroyAnimation`: within 32 blocks of the camera, the
    /// furthest progress on each block).
    fn cracks(&self, camera: glam::DVec3) -> Vec<([i32; 3], [f32; 2])> {
        // `destroy_stage_0` .. `destroy_stage_9`: the sprite name by stage, so the
        // lookup does not format a string per crack.
        const DESTROY_STAGES: [&str; 10] = [
            "block/destroy_stage_0",
            "block/destroy_stage_1",
            "block/destroy_stage_2",
            "block/destroy_stage_3",
            "block/destroy_stage_4",
            "block/destroy_stage_5",
            "block/destroy_stage_6",
            "block/destroy_stage_7",
            "block/destroy_stage_8",
            "block/destroy_stage_9",
        ];
        let mut stages: Vec<([i32; 3], u32)> = Vec::new();
        let mut add = |pos: [i32; 3], stage: u32| match stages.iter_mut().find(|c| c.0 == pos) {
            Some(c) => c.1 = c.1.max(stage),
            None => stages.push((pos, stage)),
        };
        if let Some(m) = &self.mining
            && m.progress > 0.0
        {
            add(m.pos, ((m.progress * 10.0) as u32).min(9));
        }
        let now = self.world.game_time;
        for (pos, stage, at) in self.destroying.values() {
            if now.saturating_sub(*at) <= 400 {
                add(*pos, u32::from(*stage));
            }
        }
        stages
            .into_iter()
            .filter(|(pos, _)| {
                let d = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                    + 0.5
                    - camera;
                d.length_squared() <= 1024.0
            })
            .filter_map(|(pos, stage)| {
                let name = DESTROY_STAGES.get(stage as usize)?;
                let r = self.gui.atlas.sprites.get(*name)?;
                Some((pos, [r.u0, r.v0]))
            })
            .collect()
    }

    fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        if self.dim == Dim::End {
            let sky = self
                .res
                .end_environment()
                .sample(self.world.day_time, None, None);
            return (15.0 - 15.0 * sky.sky_light_level).round() as i32;
        }
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Sampled once per (time, weather): every mob asks each tick.
        let key = (self.world.day_time, rain.to_bits(), thunder.to_bits());
        if let Some((k, darken)) = self.sky_darken_memo.get()
            && k == key
        {
            return darken;
        }
        let darken = horizon_client::environment::sky_darken(
            self.res.environment(),
            self.world.day_time,
            rain,
            thunder,
        );
        self.sky_darken_memo.set(Some((key, darken)));
        darken
    }
    fn mob_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, m) in self.mobs.iter().enumerate() {
            // ProjectileUtil.getEntityHitResult: the mob the player rides
            // shares its root vehicle and is never the hit, nor is the
            // camera entity the pick starts from. EntitySelector.CAN_BE_PICKED
            // is isPickable — `!isRemoved`, not alive: a health-0 fake player
            // still answers a click.
            if m.removed || Some(m.id) == self.riding || Some(m.id) == self.remote_camera_id() {
                continue;
            }
            if let Some(t) = ray_box(eye, dir, reach, m.bounds())
                && best.is_none_or(|b| t < b.1)
            {
                best = Some((i, t));
            }
        }
        best
    }
    /// `Player.attack`'s damage against `target_kind` (with the attack
    /// strength and whether it is a critical hit): the weapon and effects
    /// scaled by the attack strength, a mace's smash, the critical hit, then
    /// enchantments.
    pub(super) fn melee_damage(
        &mut self,
        target_kind: &str,
        living: bool,
    ) -> (f32, f32, bool, bool) {
        let hand_damage = self.attack_damage();
        // Riptide's spin replaces the attack damage attribute.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 { spin } else { hand_damage };
        let scale = self.attack_strength_scale();
        // getEnchantedDamage(target, damage, source) - damage, taken before the
        // attack strength scales the base damage.
        let enchant_bonus = self.enchanted_damage(target_kind, damage) - damage;
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        // Item.getAttackDamageBonus (a mace's smash), before the critical hit.
        amount += self.smash_damage_bonus(target_kind);
        let crit = living && self.critical_attack(scale);
        if crit {
            amount *= 1.5;
        }
        // Enchantment damage (sharpness, smite, bane) scaled by the attack strength.
        amount += enchant_bonus * scale;
        (amount, scale, crit, enchant_bonus * scale > 0.0)
    }
    /// `Player.getAttackStrengthScale(0.5)`.
    fn attack_strength_scale(&self) -> f32 {
        let delay = horizon_client::attack_strength::delay(self.main_hand_attack_speed());
        ((self.attack_ticks as f32 + 0.5) / delay).clamp(0.0, 1.0)
    }
    /// `Player.isSweepAttack`: a charged, grounded sword hit below the movement limit.
    pub(super) fn sweep_attack_position(&mut self, scale: f32, crit: bool) -> Option<[f64; 3]> {
        // Player.getSpeed: the movement speed attribute as it is now (the
        // field is only refreshed by the local movement step, never for a
        // server's player).
        let p = self.player.as_mut()?;
        if let Ok(speed) = p.attribute("minecraft:movement_speed") {
            p.speed = speed as f32;
        }
        let p = self.player.as_ref()?;
        let sword = self.held().is_some_and(|s| {
            self.res
                .items()
                .item_tags
                .get("minecraft:swords")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        if !p.is_sweep_attack(scale, crit, sword) {
            return None;
        }
        // Mth.sin / Mth.cos of yRot * (PI / 180), from its sine table.
        use horizon_core::geometry::trig;
        let yaw = f64::from(self.yaw * trig::DEG_TO_RAD);
        Some([
            p.position.x - f64::from(trig::sin(yaw)),
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z + f64::from(trig::cos(yaw)),
        ])
    }
    /// `Player.doSweepAttack` after a successful hit, in vanilla's order: the
    /// sweep sound, the victims around `target` (its box, and the mob that
    /// was hit, when there is one) and then the arc.
    pub(super) fn sweep_attack(
        &mut self,
        at: [f64; 3],
        from: Option<[f64; 3]>,
        target: Option<(Aabb, mace::Spared)>,
        scale: f32,
    ) {
        self.audio
            .play("minecraft:entity.player.attack.sweep", from, 1.0, 1.0);
        if let Some((area, mob)) = target {
            self.sweep_damage(area, mob, scale);
        }
        let arc = horizon_client::particles::ParticleOptions::simple(
            horizon_client::particles::ParticleType::SweepAttack,
        );
        self.particles.add(&arc, at, [0.0; 3]);
        // Player.sweepAttack's `sendParticles(SWEEP_ATTACK, ..., 0, dx, 0, dz, 0)`:
        // the offset is the one from the player to the particle.
        let origin = from.unwrap_or(at);
        let spread = [at[0] - origin[0], 0.0, at[2] - origin[2]];
        self.lan_send_particles(lan_particles::QueuedParticle {
            dimension: self.dim.id(),
            options: arc,
            state: None,
            at,
            spread,
            speed: 0.0,
            count: 0,
            randomization: 0,
        });
    }
    /// `Player.attack`'s sounds when a vehicle takes the hit (it always does,
    /// and can't be hit critically): knockback when sprinting at full
    /// strength, then strong or weak, or the sweep around the vehicle's
    /// box `area`. Before `swing_arm` resets the charge.
    pub(super) fn vehicle_attack_sounds(&mut self, area: Aabb, magic: Option<i32>) {
        self.attack_effects(area, None, mace::Spared::Nothing, magic);
    }
    /// `Player.attack` after a target that is not a mob took the hit:
    /// the sprint knockback sound, a living target's critical hit (`living`:
    /// its entity id, which makes the hit critical when the player falls),
    /// the sweep around the target's box `area` and the strong or weak sound.
    pub(super) fn attack_effects(
        &mut self,
        area: Aabb,
        living: Option<i32>,
        spared: mace::Spared,
        magic: Option<i32>,
    ) {
        let scale = self.attack_strength_scale();
        let full = scale > 0.9;
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if full && player.is_some_and(|p| p.sprinting) {
            self.audio
                .play("minecraft:entity.player.attack.knockback", from, 1.0, 1.0);
        }
        let crit = living.is_some() && self.critical_attack(scale);
        let sweep = self.sweep_attack_position(scale, crit);
        if let Some(at) = sweep {
            self.sweep_attack(at, from, Some((area, spared)), scale);
        }
        if crit {
            self.audio
                .play("minecraft:entity.player.attack.crit", from, 1.0, 1.0);
            if let Some(id) = living {
                self.area_critical_hit(id, area, false);
            }
        } else if sweep.is_none() {
            let sound = if full {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            self.audio.play(sound, from, 1.0, 1.0);
        }
        // Player.magicCrit: the enchantments added damage.
        if let Some(id) = magic {
            self.area_critical_hit(id, area, true);
        }
    }
    /// `critical_hit` around the box `area` of entity `id` (`magic`: the
    /// enchanted hit instead of the crit).
    fn area_critical_hit(&mut self, id: i32, area: Aabb, magic: bool) {
        let (feet, size) = (area.bottom_center(), area.size());
        self.critical_hit(
            self.lan_acting,
            id,
            [feet.x, feet.y, feet.z],
            size.x,
            size.y,
            magic,
        );
    }
    /// `Player.canCriticalAttack` for a living target at attack strength
    /// `scale`: falling, off the ground, not in water, not blind (mobility
    /// restricted), not riding, not sprinting.
    pub(super) fn critical_attack(&self, scale: f32) -> bool {
        let player = self.player.as_ref();
        let falling = player.is_some_and(|p| {
            !p.ground.on_ground
                && p.fall_distance > 0.0
                && !p.fluids.was_touching_water
                && !p.sprinting
        });
        let restricted = self.potions.effects.get("blindness").is_some() || self.riding.is_some();
        scale > 0.9 && falling && !restricted
    }
    /// Player.attack with the attack-strength charge, crits and knockback.
    fn attack(&mut self) -> bool {
        if self.spectator() {
            return false;
        }
        // A painting nearer than the targeted block and mob breaks instead.
        if let Some((i, d)) = self.painting_hit()
            && self.mob_hit().is_none_or(|m| d < m.1)
            && self.before_block(d)
        {
            self.break_painting(i, true);
            return true;
        }
        // A ghast's fireball or a wind charge in front: turned back.
        if self.attack_redirectable() {
            return true;
        }
        if self.attack_frame()
            || self.attack_armor_stand()
            || self.attack_cushion()
            || self.attack_boat()
            || self.attack_cart()
            || self.attack_dragon()
            || self.attack_guest()
        {
            return true;
        }
        let Some((index, distance)) = self.mob_hit() else {
            return false;
        };
        if !self.before_block(distance) {
            return false;
        }
        self.attack_mob(index);
        // Minecraft.startAttack swings after the attack.
        self.swing_arm();
        true
    }
    /// `Player.attack` on a mob: the damage, knockback, sweep, particles and
    /// sounds.
    pub(super) fn attack_mob(&mut self, index: usize) {
        let target_kind = self.mobs[index].kind.id.clone();
        let target_kind = format!("minecraft:{}", target_kind.trim_start_matches("minecraft:"));
        let (amount, scale, crit, magic) = self.melee_damage(&target_kind, true);
        let sweep = self.sweep_attack_position(scale, crit);
        let (knockback, knockback_attack) = self.extra_knockback(&target_kind, scale);
        self.attack_ticks = 0;
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if knockback_attack {
            self.audio
                .play("minecraft:entity.player.attack.knockback", from, 1.0, 1.0);
        }
        // LivingEntity.setLastHurtMob (tamed wolves join in).
        self.combat.last_hurt_mob = Some((self.mobs[index].id, self.world.game_time));
        let old_health = self.mobs[index].health;
        let source = match from {
            Some(f) => self.attack_source(f),
            None => horizon_client::mobs::Damage::of(horizon_client::mobs::DamageKind::Generic),
        };
        let landed = self.hurt_by_player(index, amount, source);
        if landed && old_health > 0.0 && !self.mobs[index].alive() {
            self.native_kill_event(index);
        }
        if landed {
            let victim = self.mob_entity(&self.mobs[index]);
            self.player_hit(
                &victim,
                creatures::damage_type(source.kind, true),
                None,
                amount,
            );
        }
        if let Some(p) = self.player.as_mut() {
            if p.attack_log.len() >= 64 {
                p.attack_log.remove(0);
            }
            p.attack_log.push(horizon_client::health::AttackRecord {
                game_time: p.health.game_time(),
                target: target_kind.clone(),
                id: self.mobs[index].id,
                killed: landed && !self.mobs[index].alive(),
                amount,
                scale,
                crit,
                landed,
                dealt: old_health - self.mobs[index].health,
            });
        }
        if landed {
            if knockback > 0.0 {
                let (xd, zd) = self.extra_knockback_direction();
                self.mobs[index].knockback(f64::from(knockback), xd, zd);
                self.stop_sprint_after_extra_knockback();
            }
            let mob = &self.mobs[index];
            // Player.damageStatsAndHearts: the damage the hit actually did.
            let actual = old_health - mob.health;
            // Player.attack: crit emitter and damage indicator hearts.
            let (feet, width, height) = (
                [mob.position.x, mob.position.y, mob.position.z],
                mob.kind.width,
                mob.kind.height,
            );
            self.stats
                .custom("damage_dealt", (actual * 10.0).round() as i32);
            if crit {
                self.critical_hit(
                    self.lan_acting,
                    self.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    false,
                );
            }
            // Player.attackVisualEffects: magicCrit when the enchantments added damage.
            if magic {
                self.critical_hit(
                    self.lan_acting,
                    self.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    true,
                );
            }
            if actual > 2.0 {
                let indicator = horizon_client::particles::ParticleOptions::simple(
                    horizon_client::particles::ParticleType::DamageIndicator,
                );
                self.particles.send_particles(
                    &indicator,
                    [feet[0], feet[1] + height * 0.5, feet[2]],
                    [0.1, 0.0, 0.1],
                    0.2,
                    (actual * 0.5) as u32,
                );
                // The players a server serves are sent them too.
                self.lan_send_particles(lan_particles::QueuedParticle {
                    dimension: self.dim.id(),
                    options: indicator,
                    state: None,
                    at: [feet[0], feet[1] + height * 0.5, feet[2]],
                    spread: [0.1, 0.0, 0.1],
                    speed: 0.2,
                    count: (actual * 0.5) as u32 as i32,
                    randomization: 0,
                });
            }
            self.last_hurt_by_player
                .insert(self.mobs[index].id, self.lan_acting);
            let sound = if crit {
                "minecraft:entity.player.attack.crit"
            } else if scale > 0.9 {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            // Player.playServerSideSound: at the player.
            if let Some(at) = sweep {
                let target = (
                    self.mobs[index].bounds(),
                    mace::Spared::Mob(self.mobs[index].id),
                );
                self.sweep_attack(at, from, Some(target), scale);
            } else {
                self.audio.play(sound, from, 1.0, 1.0);
            }
            // The hurt or death sound the mob raised (getHurtSound / getDeathSound,
            // its voice pitch from its own random).
            let queued = std::mem::take(&mut self.mobs[index].outbox);
            let mut infested = false;
            for event in queued {
                match event {
                    horizon_client::mobs::MobEvent::Sound(s, at, volume, pitch) => {
                        self.audio
                            .play(&format!("minecraft:{s}"), Some(at), volume, pitch);
                    }
                    horizon_client::mobs::MobEvent::Infested { .. } => infested = true,
                    other => {
                        // Left for the end of the tick.
                        self.mobs[index].outbox.push(other);
                    }
                }
            }
            if infested {
                let id = self.mobs[index].id;
                self.mob_infested(id);
            }
            self.weapon_hit(index);
            // Player.attack: causeFoodExhaustion(0.1F) only when the hit hurt
            // (inside `if (wasHurt)`), after damageStatsAndHearts.
            if let Some(p) = self.player.as_mut()
                && !self.creative
            {
                p.cause_food_exhaustion(0.1);
            }
        } else {
            self.audio
                .play("minecraft:entity.player.attack.nodamage", from, 1.0, 1.0);
        }
    }
    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    /// A mob explosion (`ServerLevel.explode` with `ExplosionInteraction.MOB`:
    /// blocks are kept without mobGriefing, else the mob_explosion_drop_decay
    /// rule picks decay).
    fn explode(&mut self, center: [f64; 3], radius: f32) -> Result<()> {
        let interaction = if !self.ui.rules.bool("mob_griefing") {
            explosion::BlockInteraction::Keep
        } else {
            self.destroy_type("mob_explosion_drop_decay")
        };
        self.server_explode(center, radius, interaction, false)
    }
    /// Copies of a save batch carrying its live block entities and save data.
    fn with_entities<'a>(
        &self,
        chunks: impl IntoIterator<Item = &'a Arc<horizon_client::chunk::Chunk>>,
    ) -> Vec<Arc<horizon_client::chunk::Chunk>> {
        horizon_client::save::chunks_with_block_entities(
            chunks,
            &self.world.block_entities,
            &self.world.block_names,
        )
        .into_iter()
        .map(|mut c| {
            self.attach_save_data(&mut c);
            Arc::new(c)
        })
        .collect()
    }
    fn tick_mobs(&mut self) -> Result<()> {
        self.tick_creatures()?;
        self.position_rider();
        self.tick_stats();
        self.tick_scoreboard();
        self.load_bossbars();
        self.tick_worldborder();
        self.tick_subtitles();
        self.spawn_phantoms();
        self.tick_advancements();
        self.tick_player_model();
        Ok(())
    }
    /// SkyRenderer.renderSunMoonAndStars: bodies on a frame rotated Y -90 then X by angle.
    /// Returns the sun and moon quads, and the stars' frame and brightness
    /// (the renderer keeps their geometry, built once). The bodies fade with
    /// the rain (`rainBrightness = 1 - rainLevel`, their colour's alpha).
    fn celestial_geometry(
        &self,
        sky: &horizon_client::environment::Sky,
        rain: f32,
    ) -> (Vec<Vertex>, Option<(glam::Mat4, f32)>) {
        let body_color =
            0x00FF_FFFF | (((1.0 - rain).clamp(0.0, 1.0) * 255.0).round() as u32) << 24;
        let mut out = Vec::new();
        let atlas = &self.gui.atlas;
        let frame = |angle: f32| {
            glam::Mat4::from_rotation_y((-90.0f32).to_radians())
                * glam::Mat4::from_rotation_x(angle.to_radians())
        };
        let mut body = |name: &str, angle: f32, size: f32, moon: bool| {
            let Some(r) = atlas.sprites.get(name).copied() else {
                return;
            };
            // How brightly HDR output lets the body shine (mesher::hdr_tier):
            // the moon at the low tier, not the sun's.
            let tier = if moon {
                horizon_client::mesher::LOW
            } else {
                horizon_client::mesher::VERY_BRIGHT
            };
            let m = frame(angle);
            let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
            // buildCelestialQuad (sun) vs buildMoonPhases: the moon's UVs run
            // the other way round.
            let uv = if moon {
                [[r.u1, r.v1], [r.u0, r.v1], [r.u0, r.v0], [r.u1, r.v0]]
            } else {
                [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]]
            };
            for ((x, z), uv) in corners.iter().zip(uv) {
                let p = m.transform_point3(glam::Vec3::new(*x, 100.0, *z));
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 2 | u32::from(tier) << 16,
                    color: body_color,
                    light: 0,

                    glint_uv: [0.0, 0.0],
                });
            }
        };
        body("environment/celestial/sun", sky.sun_angle, 30.0, false);
        let phases = [
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ];
        let phase = phases[(self.world.day_time / 24000).rem_euclid(8) as usize];
        body(
            &format!("environment/celestial/moon/{phase}"),
            sky.moon_angle,
            20.0,
            true,
        );
        // SkyRenderer.renderStars: the static star buffer on the same frame.
        let stars =
            (sky.star_brightness > 0.0).then(|| (frame(sky.star_angle), sky.star_brightness));
        (out, stars)
    }
    /// `onAbort`: the confirm screen's Back or Escape reopens the Data
    /// Packs screen with the selection it had.
    fn reopen_data_packs(&mut self, event_loop: &dyn EventLoopCtl) -> Result<()> {
        let action = self
            .title
            .as_mut()
            .and_then(|t| t.reopen_data_packs_action());
        if let Some(action) = action {
            self.title_action(event_loop, action)?;
        }
        Ok(())
    }
    /// What the title screen asked for, from a click or Enter.
    fn title_action(&mut self, event_loop: &dyn EventLoopCtl, action: Action) -> Result<()> {
        match action {
            // A splitscreen player's title never ends the process (P1's does).
            Action::Quit if self.options.split_guest => {}
            Action::Quit => event_loop.exit(),
            Action::Options => self.open_options(true),
            Action::Accessibility => {
                self.open_options_at(true, horizon_client::options_ui::Page::Accessibility)
            }
            Action::Language => {
                self.open_options_at(true, horizon_client::options_ui::Page::Language)
            }
            Action::Sounds => self.open_options_at(true, horizon_client::options_ui::Page::Sound),
            Action::Credits => {
                self.open_options_at(true, horizon_client::options_ui::Page::Credits)
            }
            Action::OpenUri(uri) => packs::open_uri(&uri),
            Action::CopyText(text) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&text);
            }
            Action::Play { folder } => self.open_world(&folder, None, None, false, None)?,
            Action::Create {
                folder,
                name,
                seed,
                creative,
                options,
            } => self.open_world(&folder, Some(&name), seed, creative, Some(&options))?,
            Action::Connect { address, lan } => {
                self.connect_server_with_options(&address, &self.options.player_name(), None, lan)?
            }
            Action::DataPacks { dir, selected } => {
                self.keys.clear();
                self.screen = Screen::Options;
                self.options_from_title = true;
                let resources = self.vanilla_root();
                self.ui.options.open_data_packs(dir, selected, resources);
            }
            Action::None => {}
        }
        Ok(())
    }
    /// Enter a local world from the client, selecting its simulation mode.
    fn open_world(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&horizon_client::title::WorldOptions>,
    ) -> Result<()> {
        // openWorldLoadLevelData: an existing world whose level data does not read opens
        // RecoverWorldDataScreen instead.
        if create.is_none()
            && let Some(title) = self.title.as_mut()
            && title.open_recover(folder)
        {
            return Ok(());
        }
        world_loading::world_data_used();
        if self.defer_world_entry(PendingWorldEntry::Local {
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned().map(Box::new),
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        // Create World's data packs, in the world before it opens.
        if let Some(dir) = create.and_then(|o| o.pack_dir.as_deref()) {
            let world = std::env::current_dir()?.join("saves").join(folder);
            let packs = create.map_or(&[][..], |o| &o.data_packs[..]);
            if let Err(e) = horizon_client::datapack::install(dir, &world, packs) {
                log::warn!("Horizon: data packs for {folder}: {e}");
            }
            let _ = std::fs::remove_dir_all(dir);
        }
        // The world's pack selection (`DataPackConfig`), for the pack stack
        // to read: the picked packs on, the feature packs left out off.
        // Only when `install` did not run (no Data Packs screen opened): it
        // writes the file itself, and its `disabled` must not be lost.
        if let Some(o) = create.filter(|o| o.pack_dir.is_none()) {
            let world = std::env::current_dir()?.join("saves").join(folder);
            if let Err(e) = std::fs::create_dir_all(&world) {
                log::warn!("Horizon: data packs for {folder}: {}: {e}", world.display());
            } else {
                let disabled: Vec<String> =
                    horizon_client::datapack::feature_packs(&self.vanilla_root())
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| !o.data_packs.contains(id))
                        .collect();
                let json = serde_json::json!({"enabled": o.data_packs, "disabled": disabled});
                let file = world.join("datapacks.json");
                if let Err(e) = std::fs::write(&file, json.to_string()) {
                    log::warn!("Horizon: data packs for {folder}: {}: {e}", file.display());
                }
            }
        }
        if self
            .options
            .execution_role
            .starts_integrated_server(self.options.server_thread)
        {
            return self.open_integrated(folder, name, seed, creative, create);
        }
        let request = world_loading::WorldRequest::new(folder, name, seed, creative, create)?;
        if self.window.is_some() {
            self.perf.world_opening();
            if self.title.is_none() {
                self.title = Some(
                    Title::new(
                        std::env::current_dir()?.join("saves"),
                        &self.options.resources,
                    )
                    .with_user(|| self.options.player_name()),
                );
            }
            self.world_loader = Some(world_loading::WorldLoader::start(
                request,
                self.res.clone(),
                self.pack.clone(),
                self.options.clone(),
                self.settings.clone(),
            )?);
            return Ok(());
        }
        self.load_world(request)
    }

    /// Load authoritative world state synchronously, without client entry or
    /// integrated-server selection. Server startup must use this entry point.
    fn load_world(&mut self, request: world_loading::WorldRequest) -> Result<()> {
        self.join_world_rest()?;
        self.perf.world_opening();
        let prepared = request.prepare(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            &horizon_world::datafix::progress::UpgradeProgress::new(),
        )?;
        self.finish_open_world(prepared)
    }

    fn finish_open_world(&mut self, prepared: world_loading::PreparedWorld) -> Result<()> {
        self.reset_tutorial();
        // Another world: its generator, not the last one's Density line.
        self.debug_density.borrow_mut().clear();
        let world_loading::PreparedWorld {
            save_root,
            shapes,
            level,
            world,
            streaming,
            save,
            session_lock,
            rand_sequences,
            locator,
        } = prepared;
        let dim = Dim::from_id(&level.dimension);
        self.level_shapes = shapes;
        self.world_root = save_root.clone();
        // The world's packs over the game's biomes (`Biomes::with_packs`),
        // the table its chunks were built with.
        self.world_biomes = self.packed_biomes();
        horizon_client::crash::set_detail(
            "World",
            format!("{} ({})", level.name, save_root.display()),
        );
        self.end.set_locator(locator);
        self.share_spawn_conditions();
        self.score = Default::default();
        self.fns = Default::default();
        self.world_tables = self.build_world_tables();
        // Native tag reads use the lock to tell that the tag tables are ready.
        self.session_lock = Some(session_lock);
        // The registries a preload or an earlier world read are not this
        // world's packs': a world with packs sends its own (their dialogs,
        // recipes and advancements). For a world without packs, `sync_data`
        // drops what was read with packs.
        if self.world_tables.is_some() {
            lan::forget_sync_data();
        }
        // Warm the loot data with this world's packs before chunks decode
        // (`save::roll_loaded_loot` reads the cached copy).
        let _ = horizon_client::structure_blocks::world_loot_data(
            &self.options.resources,
            &pack_files(&self.world_tables),
        );
        self.bosses = Default::default();
        reset_world_progress(&mut self.adv, &mut self.stats);
        self.prefetch_advancement_tree();
        self.restore_player_state = true;
        self.rand_sequences = rand_sequences;
        self.border = Default::default();
        self.set_mode_id(level.mode());
        self.previous_mode = level.previous_game_mode.map(game_mode::mode_from_id);
        let mut inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.offhand = None;
        self.ender_items = if level.ender_items.len() == 27 {
            level.ender_items.clone()
        } else {
            vec![None; 27]
        };
        for (slot, id, count, damage) in &level.inventory {
            let stack = ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra: level.inventory_components.get(slot).cloned(),
            };
            if (100..104).contains(slot) {
                self.armor[usize::from(103 - slot)] = Some(stack);
            } else if *slot == 150 {
                self.offhand = Some(stack);
            } else if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
                *cell = Some(stack);
            }
        }
        if self.creative && level.inventory.is_empty() && level.position.is_none() {
            for (i, name) in [
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:oak_planks",
                "minecraft:glass",
                "minecraft:torch",
                "minecraft:oak_log",
                "minecraft:chest",
                "minecraft:furnace",
                "minecraft:diamond_pickaxe",
            ]
            .iter()
            .enumerate()
            {
                inventory[i] = Some(ItemStack {
                    item: Arc::from(*name),
                    count: 1,
                    damage: 0,
                    extra: None,
                });
            }
        }
        let (yaw, pitch) = self
            .options
            .look
            .unwrap_or((level.rotation[0], level.rotation[1]));
        self.yaw = yaw;
        self.pitch = pitch;
        self.selected = level.selected_slot.clamp(0, 8) as usize;
        self.streaming = streaming;
        self.world = world;
        self.inventory = inventory;
        self.save = save;
        // The ride the player's file names, once the vehicle is in the world.
        self.pending_vehicle = level.root_vehicle;
        self.level = level;
        // ServerRecipeBook.loadUntrusted: recipes the manager does not have are dropped.
        let index = &world_stations!(self).recipe_index;
        let book = &mut self.level.recipe_book;
        book.recipes.retain(|r| index.contains(r));
        book.to_be_displayed.retain(|r| index.contains(r));
        self.dim = dim;
        self.audio.set_world(dim.id());
        // LevelStorageAccess.getIconFile: this save's icon.png is what the
        // Select World list shows.
        self.world_icon = Some(WorldIcon {
            path: save_root.join("icon.png"),
            last_attempt: None,
        });
        self.world_flat = horizon_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == horizon_client::save::WorldPreset::Flat);
        self.world_debug = horizon_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == horizon_client::save::WorldPreset::Debug);
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.clear_entities();
        self.travelling = None;
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.spawn = None;
        self.respawn_travel = None;
        self.respawn_config = self
            .level
            .respawn_elsewhere
            .as_ref()
            .map(respawn_anchor::RespawnConfig::from_saved);
        self.spawn_origin = None;
        self.spawn_spiral = Default::default();
        self.player_spawn = None;
        self.creatures.reset_trader();
        self.creatures.cat_tick = 0;
        self.portals = Default::default();
        self.experience = Experience {
            level: self.level.xp_level,
            progress: self.level.xp_progress,
            total: self.level.xp_total,
        };
        self.restore_effects();
        self.mining = None;
        self.container = None;
        let kinds = std::mem::take(&mut self.be_anims.kinds);
        self.be_anims = horizon_client::block_entity_anim::BlockEntityAnims {
            kinds,
            ..Default::default()
        };
        self.carried = None;
        self.using = None;
        self.screen = Screen::Playing;
        self.title = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.streaming.sky_light = dim != Dim::Nether;
        self.streaming.nether_shading = dim == Dim::Nether;
        self.load_world_state();
        Ok(())
    }
    /// Streaming and region storage for one dimension of a world.
    fn make_streaming(
        &self,
        world_root: &std::path::Path,
        dim: Dim,
        seed: i64,
    ) -> Result<(Streaming, Arc<SaveDir>)> {
        let (streaming, save, locator) = world_loading::make_streaming(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            world_root,
            dim,
            self.level_shapes.get(dim.id()),
            seed,
            &self.level.enabled_features,
        )?;
        self.end.set_locator(locator);
        self.share_spawn_conditions();
        Ok((streaming, save))
    }
    /// Change dimension through a portal (Entity.changeDimension with
    /// PortalForcer placement once the destination has loaded).
    fn enter_dimension(&mut self, target: Dim) -> Result<()> {
        let Some(player) = self.player.take() else {
            return Ok(());
        };
        let from = self.dim;
        let destination = self.switch_level(target, player.position)?;
        let at = player.position;
        self.trigger_dimension(from, target, [at.x, at.y, at.z]);
        self.travelling = Some((player, destination));
        self.end_travel = (target == Dim::End || from == Dim::End) && self.end_portal_travel;
        Ok(())
    }
    /// The level swap under a dimension change: save the departing level,
    /// open `target`'s, and return where a player leaving from `from` goes
    /// (scaled coordinates, or the End portal's fixed destination).
    pub(super) fn switch_level(&mut self, target: Dim, from: Vec3) -> Result<Vec3> {
        // Save the departing dimension's changed chunks.
        self.save.save_forced_chunks(&self.streaming.forced)?;
        self.streaming.save(&mut self.world, true);
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.save
            .save(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        self.save_dimension_entities()?;
        // Coordinates scale by 8 between the Overworld and the Nether.
        let scale = match (self.dim, target) {
            (Dim::Overworld, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            (Dim::End, Dim::Nether) => 1.0 / 8.0,
            _ => 1.0,
        };
        let destination = self.end_destination(target).unwrap_or(Vec3::new(
            from.x * scale,
            from.y,
            from.z * scale,
        ));
        let (streaming, save) =
            self.make_streaming(&self.world_root.clone(), target, self.level.seed)?;
        let mut world = World::new(self.res.facts().air, self.level.seed);
        world.shape = self.level_shapes.get(target.id());
        world.fast_lava = target == Dim::Nether;
        world.game_time = self.world.game_time;
        world.day_time = self.world.day_time;
        world.weather = self.world.weather;
        // The game rules and the difficulty hold in every dimension.
        world.health_rules = self.world.health_rules;
        // An open hub keeps hearing the new level's block changes.
        world.block_log = self.world.block_log.as_ref().map(|_| Vec::new());
        world.light_log = self.world.light_log.as_ref().map(|_| Default::default());
        // The sculk tables' watched blocks are built once per game.
        world.watched = std::mem::take(&mut self.world.watched);
        self.streaming = streaming;
        self.world = world;
        self.save = save;
        self.dim = target;
        self.audio.set_world(target.id());
        self.level.dimension = target.id().to_owned();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.falling.clear();
        self.transient_blocks.clear();
        self.tnts.clear();
        self.mobs.clear();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.orbs.clear();
        self.mining = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        Ok(destination)
    }
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    fn portal_shape(&self, start: [i32; 3], x_axis: bool) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                .clone()
        };
        let empty = |p: [i32; 3]| {
            let n = name(p);
            matches!(
                &*n,
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:nether_portal"
            )
        };
        let frame = |p: [i32; 3]| &*name(p) == "minecraft:obsidian";
        let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
        let mut bottom = start;
        let mut n = 0;
        while empty([bottom[0], bottom[1] - 1, bottom[2]]) && n < 21 {
            bottom[1] -= 1;
            n += 1;
        }
        if !frame([bottom[0], bottom[1] - 1, bottom[2]]) {
            return None;
        }
        let mut left = bottom;
        n = 0;
        while empty([left[0] - d[0], left[1], left[2] - d[2]]) && n < 21 {
            left = [left[0] - d[0], left[1], left[2] - d[2]];
            n += 1;
        }
        if !frame([left[0] - d[0], left[1], left[2] - d[2]]) {
            return None;
        }
        let mut width = 0;
        while width <= 21 {
            let p = [left[0] + d[0] * width, left[1], left[2] + d[2] * width];
            if !empty(p) || !frame([p[0], p[1] - 1, p[2]]) {
                break;
            }
            width += 1;
        }
        if !(2..=21).contains(&width)
            || !frame([left[0] + d[0] * width, left[1], left[2] + d[2] * width])
        {
            return None;
        }
        let mut height = 0;
        'rows: while height <= 21 {
            let y = left[1] + height;
            if !frame([left[0] - d[0], y, left[2] - d[2]])
                || !frame([left[0] + d[0] * width, y, left[2] + d[2] * width])
            {
                break;
            }
            for i in 0..width {
                if !empty([left[0] + d[0] * i, y, left[2] + d[2] * i]) {
                    break 'rows;
                }
            }
            height += 1;
        }
        let top_ok =
            (0..width).all(|i| frame([left[0] + d[0] * i, left[1] + height, left[2] + d[2] * i]));
        ((3..=21).contains(&height) && top_ok).then_some((left, width, height))
    }
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal,
    /// only in the Overworld and the Nether (`inPortalDimension`).
    fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        if !matches!(self.dim, Dim::Overworld | Dim::Nether) {
            return Ok(false);
        }
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.pack.assets.registry;
                let Some(default) = registry.default_state("minecraft:nether_portal") else {
                    return Ok(false);
                };
                let mut portal = registry.state(default).cloned().ok_or("portal state")?;
                portal
                    .properties
                    .insert("axis".into(), if x_axis { "x" } else { "z" }.into());
                let id = registry.resolve(&portal).unwrap_or(default);
                // PortalShape.createPortalBlocks uses BlockPos.betweenClosed:
                // X varies fastest, then Y, then Z.
                let first = BlockPos::new(origin[0], origin[1], origin[2]);
                let last = BlockPos::new(
                    origin[0] + d[0] * (width - 1),
                    origin[1] + height - 1,
                    origin[2] + d[2] * (width - 1),
                );
                for pos in BlockPos::between_closed(first, last).expect("portal size is bounded") {
                    self.level_set_with_flags(
                        [pos.x, pos.y, pos.z],
                        id,
                        level_flags::UPDATE_CLIENTS | level_flags::UPDATE_KNOWN_SHAPE,
                    )?;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Arrival through a nether portal, once the destination is loaded.
    fn place_after_travel(&mut self, player: PlayerMotion, target: Vec3) -> Result<()> {
        self.arrive_through_nether_portal(player, target)
    }
    /// Leaving the game with a world open (window closed, Quit, `quit`):
    /// menu items go back (Player.closeContainer), then the world is saved,
    /// once.
    /// Keeps this play session's performance summary for the launcher to send, if the player
    /// allows performance statistics (`horizon_client::telemetry`), and starts the next one.
    pub(super) fn finish_perf(&mut self) {
        // One session per computer: P1's (a splitscreen player's would count it again).
        if self.options.split_guest {
            return;
        }
        let s = &self.settings;
        let settings = horizon_telemetry::payload::Settings {
            render_distance: s.render_distance.clamp(0, 255) as u8,
            simulation_distance: s.simulation_distance.clamp(0, 255) as u8,
            // 260 is the slider's "unlimited".
            max_fps: if s.max_fps >= 260 {
                0
            } else {
                s.max_fps.min(u32::from(u16::MAX)) as u16
            },
            vsync: s.vsync,
            fullscreen: s.fullscreen,
            graphics: s.graphics_preset.clone(),
        };
        // Again now: on Wayland the window is on a screen only once it has been shown.
        if let Some(window) = &self.window {
            horizon_client::telemetry::note_display(window);
        }
        self.perf
            .finish(&horizon_client::logging::game_home(), settings);
    }

    pub(super) fn save_for_exit(&mut self) -> Result<()> {
        // Native application termination can bypass CloseRequested. Finish any
        // save screen synchronously here, before the process can go away.
        while self.pending_save.is_some() {
            self.poll_closing_save()?;
            if self.pending_save.is_some() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if self.title.is_some() || self.saved_for_exit {
            return Ok(());
        }
        if let Some(server) = self.integrated.take() {
            // The server thread saves the world.
            server.stop()?;
            self.saved_for_exit = true;
            return Ok(());
        }
        if let Some(mut remote) = self.remote.take() {
            // Minecraft.disconnect closes the connection, so the host sees a clean leave.
            remote.session.disconnect();
            self.saved_for_exit = true;
            return Ok(());
        }
        self.close_container();
        if self.remote.is_none() {
            self.stats.custom("leave_game", 1);
        }
        self.lan_close(false);
        self.free_player_beds()?;
        self.save_game()?;
        self.saved_for_exit = true;
        Ok(())
    }
    fn close_world(&mut self) -> Result<()> {
        self.clear_waypoints();
        self.post_effects.clear();
        self.spectated_post_effect = None;
        if self.external_server() {
            return self.leave_server(None);
        }
        self.begin_closing_save(false)
    }
    fn finish_close_world(&mut self) -> Result<()> {
        self.ambience.clear(&mut self.audio);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        if self.remote.is_some() {
            self.world_icon = None;
            return self.leave_server(None);
        }
        // The save screen has finished every write before releasing storage.
        self.session_lock = None;
        self.world_icon = None;
        horizon_client::crash::clear_detail("World");
        self.debug_charts.reset();
        self.boss_overlay.reset();
        // GameRenderer.resetData (Minecraft.clearClientLevel).
        self.item_activation.reset();
        self.kept_block_entities.get_mut().clear();
        self.bosses = Default::default();
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.clear_entities();
        self.screen = Screen::Playing;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        // Clear sync data (registries, dialog tags) so dialogs don't leak
        // between worlds.
        lan::forget_sync_data();
        self.title = Some(
            Title::new(
                std::env::current_dir()?.join("saves"),
                &self.options.resources,
            )
            .with_user(|| self.options.player_name()),
        );
        if let Some(title) = &mut self.title {
            title.last_server = self.settings.file.string("lastServer");
            title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        }
        self.capture(false);
        Ok(())
    }
    /// Let go of every entity of the level (saved first): the next world
    /// opened must not inherit them, nor save them as its own.
    fn clear_entities(&mut self) {
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.creatures.projectiles.clear();
        self.falling.clear();
        self.transient_blocks.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.cushions.clear();
        self.item_frames.clear();
        self.paintings.clear();
        self.others.clear();
        self.dragon = None;
        self.crystals.clear();
        self.dragon_fireballs.clear();
        self.breath_clouds.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.bolts.clear();
        self.particles.clear();
        self.pickups.clear();
        self.riding = None;
        self.persist = Default::default();
    }
    /// Equipment attribute modifiers (LivingEntity.collectEquipmentChanges).
    fn apply_equipment(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        for (attribute, id) in std::mem::take(&mut self.applied_modifiers) {
            let _ = player.set_modifier(&attribute, &id, 0.0, "add_value", false);
        }
        let slots = ["head", "chest", "legs", "feet"];
        let mut equipped: Vec<(Option<&ItemStack>, &str)> = self
            .armor
            .iter()
            .zip(slots)
            .map(|(s, n)| (s.as_ref(), n))
            .collect();
        equipped.push((self.inventory[self.selected].as_ref(), "mainhand"));
        equipped.push((self.offhand.as_ref(), "offhand"));
        for (stack, slot) in equipped {
            let Some(info) = stack.and_then(|s| world_items!(self).get(&s.item)) else {
                continue;
            };
            for (attribute, id, amount, op, group) in &info.modifiers {
                // The attack code still reads the held item's attack damage and
                // speed directly; the map copy is what `/attribute` reports.
                if !equipment_modifier_applies(group, slot) {
                    continue;
                }
                if player
                    .set_modifier(attribute, id, *amount, op, true)
                    .is_ok()
                {
                    self.applied_modifiers.push((attribute.clone(), id.clone()));
                }
            }
        }
        self.apply_enchantment_attributes();
    }
    /// The armor hits among a tick's motion events (`Player.hurtArmor` from
    /// `getDamageAfterArmorAbsorb`), whatever the damage: a mob, an explosion,
    /// a cactus, lava, a campfire.
    fn wear_armor(&mut self, events: &[MotionEvent]) {
        for event in events {
            if let MotionEvent::Health(horizon_client::health::HealthEvent::ArmorHurt {
                source,
                amount,
            }) = event
            {
                self.hurt_armor(*source, *amount);
            }
        }
    }
    /// `Player.hurtArmor` -> `LivingEntity.doHurtEquipment`: each worn piece that
    /// wears when its wearer is hurt loses max(1, damage / 4) durability, except
    /// to a fire source when the piece resists fire (`ItemStack.canBeHurtBy`).
    fn hurt_armor(&mut self, source: horizon_world::damage_source::DamageTypeId, damage: f32) {
        if damage <= 0.0 {
            return;
        }
        let amount = ((damage / 4.0) as u32).max(1);
        let name = self
            .motion
            .health
            .damage
            .definition(source)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let fire = self
            .motion
            .health
            .damage
            .in_tag(source, "minecraft:is_fire");
        let mut broke = false;
        let mut broken_items = Vec::new();
        let losses: Vec<u32> = (0..4)
            .map(|i| match self.armor[i].clone() {
                Some(s)
                    if self
                        .res
                        .items()
                        .get(&s.item)
                        .is_some_and(|info| info.equip_damage_on_hurt)
                        && !(fire && world_items!(self).resists_fire(&s, &name)) =>
                {
                    self.wear(&s, amount)
                }
                _ => 0,
            })
            .collect();
        for (index, (slot, amount)) in self.armor.iter_mut().zip(losses).enumerate() {
            if let Some(stack) = slot.as_mut()
                && let Some(info) = world_items!(self).get(&stack.item)
                && info.max_damage > 0
            {
                stack.damage += amount;
                if stack.damage >= info.max_damage {
                    broken_items.push((stack.clone(), index));
                    *slot = None;
                    broke = true;
                }
            }
        }
        if broke {
            // LivingEntity.onEquippedItemBroken for each armour piece that broke.
            for (stack, index) in &broken_items {
                self.player_equipment_broke(armor_slot(*index), stack);
            }
            self.apply_equipment();
        }
    }
    /// Voxel traversal that also stops at fluid source blocks (bucket pick).
    fn pick_fluid(&self) -> Option<Hit> {
        let (origin, dir) = self.aim()?;
        let mut t = 0.0;
        let mut last = origin.map(|v| v.floor() as i32);
        let mut first = true;
        while t <= self.reach() {
            let p = [
                origin[0] + dir[0] * t,
                origin[1] + dir[1] * t,
                origin[2] + dir[2] * t,
            ];
            let b = p.map(|v| v.floor() as i32);
            // `BlockGetter.traverseBlocks` visits the block the ray starts in
            // first: the eyes' own water, under water (#3787).
            if first || b != last {
                let state = self.world.block(b[0], b[1], b[2]);
                let f = self.res.facts().get(state);
                let face = [last[0] - b[0], last[1] - b[1], last[2] - b[2]];
                if f.fluid != Fluid::None
                    && f.fluid_level == 8
                    && !f.fluid_falling
                    && !f.waterlogged
                    && f.render_shape != horizon_client::blocks::RenderShape::Model
                {
                    if !first {
                        return Some(Hit {
                            pos: b,
                            face,
                            point: p,
                        });
                    }
                    let above = self.world.block(b[0], b[1] + 1, b[2]);
                    let same_above = self.res.facts().get(above).fluid == f.fluid;
                    if let Some((face, point)) =
                        clip_start_source(origin, dir, self.reach(), b, same_above)
                    {
                        return Some(Hit {
                            pos: b,
                            face,
                            point,
                        });
                    }
                } else if !first && !self.res.facts().is_air(state) && f.fluid == Fluid::None {
                    return Some(Hit {
                        pos: b,
                        face,
                        point: p,
                    });
                }
                last = b;
                first = false;
            }
            t += 0.05;
        }
        None
    }
    /// `getPlayerPOVHitResult(level, player, SOURCE_ONLY)` for an empty
    /// bucket: the block outline or a source fluid, whichever is nearer. A
    /// waterlogged block is a block hit (its outline is the block's, its
    /// fluid a source); flowing fluid has no shape to pick.
    pub(super) fn bucket_hit(&self) -> Option<Hit> {
        let origin = self.aim().map_or([0.0; 3], |(origin, _)| origin);
        let distance = |p: [f64; 3]| {
            (p[0] - origin[0]).powi(2) + (p[1] - origin[1]).powi(2) + (p[2] - origin[2]).powi(2)
        };
        match (self.pick(), self.pick_fluid()) {
            (Some(block), Some(fluid)) => Some(if distance(block.point) <= distance(fluid.point) {
                block
            } else {
                fluid
            }),
            (block, fluid) => block.or(fluid),
        }
    }
    /// `BucketItem.use`'s place position: a water bucket fills the clicked
    /// block itself when that block is a liquid container (a waterloggable
    /// stair, slab, fence, ...); every other bucket empties into the block the
    /// clicked face leads to. `SolidBucketItem` places like a block item, into
    /// a replaceable clicked block.
    pub(super) fn bucket_target(&self, content: BucketContent, hit: &Hit) -> [i32; 3] {
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if content == BucketContent::Powder {
            if self.res.facts().get(clicked).replaceable {
                return hit.pos;
            }
        } else if content.is_water() && self.res.behaviours().is_liquid_container(clicked) {
            return hit.pos;
        }
        [
            hit.pos[0] + hit.face[0],
            hit.pos[1] + hit.face[1],
            hit.pos[2] + hit.face[2],
        ]
    }
    /// `CauldronInteractions` for the held item aimed at a cauldron.
    fn use_on_cauldron(&mut self, pos: [i32; 3]) -> Result<bool> {
        use horizon_client::cauldron::{self, Outcome};
        struct Hand<'g, 'a>(&'g mut Game<'a>);
        impl cauldron::Player for Hand<'_, '_> {
            fn creative(&self) -> bool {
                self.0.creative
            }
            fn hand(&mut self) -> &mut Option<ItemStack> {
                self.0.held_mut()
            }
            fn add(&mut self, stack: &mut ItemStack) -> bool {
                let before = stack.count;
                match self.0.give(stack.clone()) {
                    None => stack.count = 0,
                    Some(rest) => stack.count = rest.count,
                }
                stack.count < before
            }
            fn contains(&self, stack: &ItemStack) -> bool {
                self.0
                    .inventory
                    .iter()
                    .flatten()
                    .any(|have| have.same_item_same_components(stack))
            }
            fn drop_stack(&mut self, stack: ItemStack) {
                if let Some(eye) = self.0.eye_position() {
                    // ItemUtils.createFilledResult: `player.drop(stack, false)`.
                    self.0.throw_from_eyes(stack, eye, false);
                }
            }
        }
        let registry = self.pack.assets.registry;
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let Some(state) = registry
            .state(id)
            .filter(|s| s.name.ends_with("cauldron"))
            .cloned()
        else {
            return Ok(false);
        };
        let Some(held) = self.held() else {
            return Ok(false);
        };
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        let above = self.world.block(pos[0], pos[1] + 1, pos[2]);
        let under_water = registry
            .state(above)
            .is_some_and(|s| s.name == "minecraft:water");
        let result = cauldron::interact(&state, under_water, removes_dye, &mut Hand(self));
        if result.outcome == Outcome::Pass {
            return Ok(false);
        }
        if let Some(stat) = result.stat {
            self.stats.custom(stat, 1);
        }
        if let Some(block) = result.block
            && let Ok(new) = registry.resolve(&block)
        {
            self.level_set(pos, new)?;
        }
        if let Some(sound) = result.sound {
            self.audio.play(
                sound,
                Some([
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 0.5,
                    f64::from(pos[2]) + 0.5,
                ]),
                1.0,
                1.0,
            );
        }
        Ok(true)
    }
    /// What `BlockTransformer.transformBlock` would do with a held axe, hoe
    /// or shovel on the block at `pos`, without doing it: the new state, its
    /// sound, loot table, the durability it costs and its particle.
    pub(super) fn block_transform(
        &self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
    ) -> Option<(BlockStateId, String, Option<String>, u32, String)> {
        use horizon_client::block_transform::Around;
        struct Column<'w> {
            world: &'w World,
            registry: &'w horizon_world::registry::BlockRegistry,
            pos: [i32; 3],
        }
        impl Around for Column<'_> {
            fn state(&self, o: [i32; 3]) -> horizon_world::chunk::BlockState {
                let id =
                    self.world
                        .block(self.pos[0] + o[0], self.pos[1] + o[1], self.pos[2] + o[2]);
                self.registry.state(id).cloned().unwrap_or_else(|| {
                    horizon_world::chunk::BlockState {
                        name: "minecraft:air".into(),
                        properties: Default::default(),
                    }
                })
            }
        }
        let id = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.block_transformer.clone())?;
        // playerHasBlockingItemUseIntent: a shield in the off hand is raised
        // instead, unless sneaking.
        let shield = self
            .offhand
            .as_ref()
            .is_some_and(|s| self.blocks_attacks(s).is_some());
        if shield && !self.keys.contains(&KeyCode::ShiftLeft) {
            return None;
        }
        let face_name = match face {
            [0, -1, 0] => "down",
            [0, 1, 0] => "up",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let registry = self.pack.assets.registry;
        let around = Column {
            world: &self.world,
            registry,
            pos,
        };
        let (_, t, state) =
            self.res
                .transformers()
                .apply(self.items(), registry, &id, face_name, &around)?;
        let new = registry.resolve(&state).ok()?;
        Some((
            new,
            t.sound.clone(),
            t.loot.clone(),
            t.damage,
            t.particle.clone(),
        ))
    }
    /// `BlockTransformer.TransformParticle.send`: its level event.
    pub(super) fn transform_particle(&mut self, particle: &str, pos: [i32; 3]) {
        match particle {
            "scrape" => self.level_event_at(3005, pos, 0),
            "wax_on" => self.level_event_at(3003, pos, 0),
            "wax_off" => self.level_event_at(3004, pos, 0),
            _ => {}
        }
    }
    /// `BlockTransformer.transformBlock` for a held axe, hoe or shovel.
    fn use_block_transformer(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
    ) -> Result<bool> {
        let Some((new, sound, loot, damage, particle)) = self.block_transform(stack, pos, face)
        else {
            return Ok(false);
        };
        // BlockTransformer.transformBlock: ITEM_USED_ON_BLOCK before the swap.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        self.level_set(pos, new)?;
        // BlockTransformer.TransformParticle.send.
        self.transform_particle(&particle, pos);
        if loot.as_deref() == Some("minecraft:till/rooted_dirt") {
            self.pop_resource(
                [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]],
                ItemStack::new("minecraft:hanging_roots", 1),
            );
        }
        // `Level.playSound(player, ...)`: the actor's client already played
        // the strip/scrape/till sound in `predict_item_on_block` (#4681).
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if self.lan_acting.is_some() {
            self.audio.play_unshared(&sound, Some(at), 1.0, 1.0);
            self.guest_player_sounds.push((sound.clone(), at, 1.0, 1.0));
        } else {
            self.audio.play(&sound, Some(at), 1.0, 1.0);
        }
        // ItemStack.useOn: a use that acted counts as one of the item.
        self.stats.add("used", &stack.item, 1);
        self.damage_held(damage);
        self.post_block_game_event("minecraft:block_change", pos, self.sculk_actor(), Some(new));
        Ok(true)
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.pack.assets.registry;
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let Some(default) = registry.default_state(name) else {
            return Ok(false);
        };
        let mut state = registry.state(default).cloned().ok_or("state")?;
        if let Some(previous) = registry.state(old) {
            for k in keep {
                if let Some(v) = previous.properties.get(*k) {
                    state.properties.insert((*k).to_owned(), v.clone());
                }
            }
        }
        let id = registry.resolve(&state).unwrap_or(default);
        self.level_set(pos, id)?;
        Ok(true)
    }
    /// `LayeredCauldronBlock.handleEntityOnFireInside` for a burning player
    /// in the cauldron at `pos` (`mayInteract`: not a spectator): the fill
    /// level drops (`lowerFillLevel`, a `block_change` with no source). A
    /// server's cauldron is the server's.
    pub(super) fn cauldron_extinguish(&mut self, pos: BlockPos) {
        if self.remote.is_some() {
            return;
        }
        let at = [pos.x, pos.y, pos.z];
        let registry = self.pack.assets.registry;
        let Some(old) = registry
            .state(self.world.block(at[0], at[1], at[2]))
            .filter(|s| {
                matches!(
                    s.name.as_str(),
                    "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron"
                )
            })
            .cloned()
        else {
            return;
        };
        let Ok(new) = registry.resolve(&horizon_client::cauldron::extinguished(&old)) else {
            return;
        };
        if self.level_set(at, new).is_ok() {
            self.post_block_game_event("minecraft:block_change", at, None, Some(new));
        }
    }
    pub(super) fn damage_held(&mut self, amount: u32) {
        if self.creative {
            return;
        }
        let amount = match self.held().cloned() {
            Some(held) => self.wear(&held, amount),
            None => amount,
        };
        let max = self
            .held()
            .and_then(|s| self.items().get(&s.item))
            .map_or(0, |i| i.max_damage);
        let cell = self.held_mut();
        if let Some(stack) = cell.as_mut()
            && max > 0
        {
            stack.damage += amount;
            if stack.damage >= max {
                let broken = stack.item.clone();
                let gone = stack.clone();
                *cell = None;
                self.stats.add("broken", &broken, 1);
                // LivingEntity.onEquippedItemBroken: the sound and the crumbs.
                self.player_equipment_broke(
                    horizon_world::player_inventory::EquipmentSlot::MainHand,
                    &gone,
                );
            }
        }
    }
    /// `ItemStack.hurtWithoutBreaking`: Unbreaking applies, but the stack stops at
    /// one point of durability left instead of breaking.
    fn hurt_held_without_breaking(&mut self, amount: u32) {
        if self.creative {
            return;
        }
        let Some(held) = self.held().cloned() else {
            return;
        };
        let amount = self.wear(&held, amount);
        let max = self.items().get(&held.item).map_or(0, |i| i.max_damage);
        if amount == 0 || max == 0 {
            return;
        }
        if let Some(stack) = self.held_mut().as_mut() {
            stack.damage = (stack.damage + amount).min(max - 1);
        }
    }
    /// Block tool use: distinguish success, failure and fall-through for hand feedback.
    fn use_tool(
        &mut self,
        stack: &ItemStack,
        allow_block_actions: bool,
    ) -> Result<horizon_client::first_person::UseResult> {
        use horizon_client::first_person::UseResult;
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        if allow_block_actions
            && let Some(hit) = self.pick()
            && self.use_on_cauldron(hit.pos)?
        {
            return Ok(UseResult::Success { swing: true });
        }
        if allow_block_actions
            && let Some(hit) = self.pick()
            && let Some(placed) = self.use_on_campfire(hit.pos)?
        {
            // SUCCESS_SERVER swings; a full campfire's CONSUME does not.
            return Ok(UseResult::Success { swing: placed });
        }
        // BucketItem has no useOn: a guest's UseItemOn leaves it to UseItem.
        let bucket = horizon_client::items::bucket_contents(&stack.item);
        if matches!(self.guest_use, Some(GuestUse::OnBlock(_)))
            && (item == "bucket" || bucket.is_some())
        {
            return Ok(UseResult::Pass);
        }
        if item == "bucket" {
            let Some(hit) = self.bucket_hit() else {
                return Ok(UseResult::Pass);
            };
            // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
            if !self.may_build() {
                return Ok(UseResult::Fail);
            }
            let mut canceled = false;
            self.native_fill_bucket(stack, hit.pos, &mut canceled);
            if canceled {
                return Ok(UseResult::Fail);
            }
            if self.is_powder_snow(hit.pos) {
                self.pick_up_powder_snow(hit.pos)?;
                return Ok(UseResult::Success { swing: true });
            }
            // `BucketPickup.pickupBlock`: a fluid source, a waterlogged block
            // or powder snow. A block that gives nothing fails the click.
            let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            let Some(filled) =
                self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, pos, creative))
            else {
                return Ok(UseResult::Fail);
            };
            self.process_level_events()?;
            // `player.awardStat(Stats.ITEM_USED.get(this))`,
            // `getPickupSound` and `level.gameEvent(player, FLUID_PICKUP)`.
            self.stats.add("used", "minecraft:bucket", 1);
            let sound = if filled == "minecraft:lava_bucket" {
                "minecraft:item.bucket.fill_lava"
            } else {
                "minecraft:item.bucket.fill"
            };
            self.audio.play(sound, Some(at(hit.pos)), 1.0, 1.0);
            self.post_block_game_event(
                "minecraft:fluid_pickup",
                hit.pos,
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            let filled = ItemStack::new(filled, 1);
            self.filled_result(filled.clone());
            self.trigger_item("minecraft:filled_bucket", &filled);
            return Ok(UseResult::Success { swing: true });
        }
        // BucketItem.use has its own ray, even when the crosshair hits an entity.
        if !allow_block_actions && bucket.is_none() {
            return Ok(UseResult::Pass);
        }
        let Some(hit) = self.pick() else {
            return Ok(UseResult::Pass);
        };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if let Some((content, sound, mob)) = bucket {
            if !self.may_build() {
                return Ok(UseResult::Fail);
            }
            let target = self.bucket_target(content, &hit);
            let mut canceled = false;
            self.native_fill_bucket(stack, target, &mut canceled);
            if canceled {
                return Ok(UseResult::Fail);
            }
            let pos = BlockPos::new(target[0], target[1], target[2]);
            // `Player.isShiftKeyDown`: only air takes a fluid under Shift.
            let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
            let creative = self.creative;
            let emptied = self.with_level(|b, level, ticks| {
                b.bucket_empty(level, ticks, pos, content, sound, sneaking, creative)
            });
            self.process_level_events()?;
            if !emptied {
                return Ok(UseResult::Fail);
            }
            if self.dim == Dim::Nether && content.is_water() {
                self.evaporation_smoke(target);
            }
            // `BucketItem.emptyContents` uses `playSound(player, …)`: the
            // actor's client already played the empty / evaporate sound from
            // its DryLevel prediction (#4684).
            if self.lan_acting.is_some()
                && let Some(capture) = self.audio.capture.as_mut()
            {
                let want = sound.trim_start_matches("minecraft:");
                let mut keep = Vec::with_capacity(capture.len());
                for entry in capture.drain(..) {
                    let name = entry.0.trim_start_matches("minecraft:");
                    if name == want || name == "block.fire.extinguish" {
                        self.guest_player_sounds.push(entry);
                    } else {
                        keep.push(entry);
                    }
                }
                *capture = keep;
            }
            if let Some(kind) = mob {
                if let Some(id) = self.spawn_mob_at(kind, (target[0], target[1], target[2]), true) {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        horizon_client::entity_nbt::apply_item_components(m, stack);
                    }
                    self.mob_placed(id)?;
                }
                self.post_block_game_event(
                    "minecraft:entity_place",
                    target,
                    self.sculk_actor(),
                    None,
                );
            }
            self.trigger_block("minecraft:placed_block", target, stack);
            if !self.creative {
                *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
            }
            return Ok(UseResult::Success { swing: true });
        }
        if self.use_block_transformer(stack, hit.pos, hit.face)? {
            return Ok(UseResult::Success { swing: true });
        }
        if item == "honeycomb" {
            let registry = self.pack.assets.registry;
            let old = registry.state(clicked).cloned();
            if let Some(waxed) = old.and_then(|old| self.res.transformers().waxed(registry, &old))
                && let Ok(state) = registry.resolve(&waxed)
            {
                // HoneycombItem.useOn: ITEM_USED_ON_BLOCK before the swap.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, stack);
                self.level_set(hit.pos, state)?;
                self.level_event_at(3003, hit.pos, 0);
                self.audio.play(
                    "minecraft:item.honeycomb.wax_on",
                    Some(at(hit.pos)),
                    1.0,
                    1.0,
                );
                // HoneycombItem.useOn's game event, and ItemStack.useOn's used statistic.
                self.post_block_game_event(
                    "minecraft:block_change",
                    hit.pos,
                    self.sculk_actor(),
                    Some(state),
                );
                self.stats.add("used", &stack.item, 1);
                if !self.creative {
                    self.consume_held();
                }
                return Ok(UseResult::Success { swing: true });
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            if !self.may_build() {
                return Ok(UseResult::Fail);
            }
            let name = self.res.facts().get(clicked).name.clone();
            if self.can_light(clicked, &name) {
                let registry = self.pack.assets.registry;
                let mut state = registry
                    .state(clicked)
                    .cloned()
                    .ok_or("missing lit block state")?;
                state.properties.insert("lit".into(), "true".into());
                self.level_set(hit.pos, registry.resolve(&state)?)?;
                self.audio.play(
                    if item == "fire_charge" {
                        "minecraft:item.firecharge.use"
                    } else {
                        "minecraft:item.flintandsteel.use"
                    },
                    Some(at(hit.pos)),
                    1.0,
                    horizon_client::items::fire_starter_pitch(
                        item == "fire_charge",
                        &mut self.world.random,
                    ),
                );
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            if self.can_place_used_fire(target) {
                if !self.try_light_portal(target)? {
                    self.place_used_fire(target)?;
                }
                let sound = horizon_client::items::fire_starter_sound(item == "fire_charge");
                let pitch = horizon_client::items::fire_starter_pitch(
                    item == "fire_charge",
                    &mut self.world.random,
                );
                self.audio.play(sound, Some(at(target)), 1.0, pitch);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            return Ok(UseResult::Fail);
        }
        Ok(UseResult::Pass)
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    fn spawn_orbs(&mut self, at: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            self.award_orb(at, v, merge_id);
        }
    }
    /// `ExperienceOrb.tryMergeToExisting`: an orb of the same value within a
    /// 1 block box of `at`, whose id is a multiple of 40 from `merge_id`, takes
    /// the new one as one more in its count (and restarts its age); otherwise
    /// a fresh orb spawns.
    fn award_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) {
        let centre = Vec3::new(at[0], at[1], at[2]);
        let half = Vec3::new(0.5, 0.5, 0.5);
        let area = Aabb::new(centre.subtract(half), centre.add_vector(half));
        // A server's orbs are its own; a LAN or server guest only shows them.
        let target = self.remote.is_none().then(|| {
            self.orbs
                .iter_mut()
                .find(|o| o.can_merge(merge_id as u32, value) && o.bounds().intersects(area))
        });
        if let Some(Some(orb)) = target {
            orb.count += 1;
            orb.age = 0;
        } else {
            self.add_orb(at, value);
        }
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs on an
    /// orb whose box is in a block: the free spot nearest its box's centre. An
    /// orb in open air keeps its position.
    fn unstuck_orb(&self, at: [f64; 3]) -> [f64; 3] {
        let b = self.res.behaviours();
        let border = self.border_bounds();
        ExperienceOrb::unstuck(Vec3::new(at[0], at[1], at[2]), &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        })
        .map_or(at, |p| [p.x, p.y, p.z])
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    fn add_orb(&mut self, at: [f64; 3], value: u32) {
        let at = self.unstuck_orb(at);
        let mut r = dispenser::entity_random();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        self.next_entity += 1;
        self.orbs.push(ExperienceOrb {
            id: self.next_entity,
            position: p,
            previous: p,
            velocity: Vec3::new(
                (r.next_double() * 0.2 - 0.1) * 2.0,
                r.next_double() * 0.2 * 2.0,
                (r.next_double() * 0.2 - 0.1) * 2.0,
            ),
            value,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            health: 5,
        });
        // ServerLevel.addFreshEntity.
        if !self.native_entity_joins(self.next_entity, "minecraft:experience_orb", at, false) {
            self.orbs.pop();
        }
    }
    fn can_place_used_fire(&mut self, target: [i32; 3]) -> bool {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let direction = match ((self.yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
            0 => Direction::South,
            1 => Direction::West,
            2 => Direction::North,
            _ => Direction::East,
        };
        self.with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, pos, direction))
    }
    fn place_used_fire(&mut self, target: [i32; 3]) -> Result<()> {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let state = self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, pos));
        self.level_set(target, state)
    }
    /// DropExperienceBlock / RedStoneOreBlock / SculkBlock experience ranges.
    fn block_experience(&mut self, name: &str) -> u32 {
        let (min, max) = match name.trim_start_matches("minecraft:") {
            "coal_ore" | "deepslate_coal_ore" => (0, 2),
            "nether_gold_ore" => (0, 1),
            "lapis_ore" | "deepslate_lapis_ore" | "nether_quartz_ore" => (2, 5),
            "diamond_ore" | "deepslate_diamond_ore" | "emerald_ore" | "deepslate_emerald_ore" => {
                (3, 7)
            }
            "redstone_ore" | "deepslate_redstone_ore" => (1, 5),
            "sculk" => (1, 1),
            "sculk_catalyst" | "sculk_sensor" | "calibrated_sculk_sensor" | "sculk_shrieker" => {
                (5, 5)
            }
            // SpawnerBlock.spawnAfterBreak: 15 + nextInt(15) + nextInt(15).
            "spawner" => {
                let r = &mut self.world.random;
                let a = r.next_int_bound(15).unwrap_or(0);
                return (15 + a + r.next_int_bound(15).unwrap_or(0)) as u32;
            }
            _ => (0, 0),
        };
        if max == 0 {
            return 0;
        }
        (min + self.world.random.next_int_bound(max - min + 1).unwrap_or(0)) as u32
    }
    fn tick_experience(&mut self) {
        if self.xp_delay > 0 {
            self.xp_delay -= 1;
        }
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        // ExperienceOrb.followNearbyPlayer: each orb goes for the nearest
        // living player within 8 blocks, the host's own or a guest.
        let mut followed: Vec<([f64; 3], [f64; 3])> = player
            .map(|p| {
                let feet = [p.position.x, p.position.y, p.position.z];
                let eye = f64::from(p.dimensions.eye_height);
                (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
            })
            .into_iter()
            .collect();
        if let Some(host) = self.lan.as_ref() {
            followed.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.placed && g.loaded && !g.dead && g.health > 0.0)
                    .filter(|g| !g.spectator())
                    .map(|g| {
                        let p = g.position;
                        (p, [p[0], p[1] + g.eye_height() / 2.0, p[2]])
                    }),
            );
        }
        // Player.aiStep: the pickup area is the box inflated by (1, 0.5, 1).
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            (
                [
                    p.position.x - w - 1.0,
                    p.position.y - 0.5,
                    p.position.z - w - 1.0,
                ],
                [
                    p.position.x + w + 1.0,
                    p.position.y + f64::from(p.dimensions.height) + 0.5,
                    p.position.z + w + 1.0,
                ],
            )
        });
        let mut orbs = std::mem::take(&mut self.orbs);
        // An orb has no random of its own here: lava draws from one made on
        // first use, apart from the level stream.
        let mut lava_random = None;
        let mut next_float = || {
            lava_random
                .get_or_insert_with(dispenser::entity_random)
                .next_float()
        };
        orbs.retain_mut(|o| {
            let distance = |feet: &[f64; 3]| -> f64 {
                (feet[0] - o.position.x).powi(2)
                    + (feet[1] - o.position.y).powi(2)
                    + (feet[2] - o.position.z).powi(2)
            };
            let target = followed
                .iter()
                .filter(|(feet, _)| distance(feet) < 64.0)
                .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                .map(|(_, target)| *target);
            o.tick(
                &self.world,
                self.res.facts(),
                self.collision,
                target,
                self.remote.is_none(),
                &mut next_float,
            )
        });
        // ExperienceOrb.scanForMerges runs on the server only.
        if self.remote.is_none() {
            horizon_client::entities::merge_orbs(&mut orbs);
        }
        let mut gained = 0;
        let mut taken = Vec::new();
        if let Some((min, max)) = bounds {
            orbs.retain_mut(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
                if touching && self.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks; a
                    // merged orb gives its value once per count.
                    self.xp_delay = 2;
                    gained += o.value;
                    taken.push(o.clone());
                    o.count -= 1;
                    return o.count > 0;
                }
                true
            });
        }
        self.orbs = orbs;
        for o in taken {
            self.add_pickup(pickups::Taken::Orb(Box::new(o)), pickups::Collector::Player);
        }
        if gained > 0 {
            gained = self.mending(gained);
        }
        if gained > 0 {
            let at = self.eye_position();
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            self.audio
                .play("minecraft:entity.experience_orb.pickup", at, 0.1, pitch);
            self.give_orb_points(gained);
        }
    }
    /// `FurnaceResultSlot.checkTakeAchievements`: the player took from the output of the
    /// furnace at `pos`, which pops the experience of what it made at them
    /// (`awardUsedRecipesAndPopExperience`) and forgets it.
    pub(super) fn award_furnace_recipes(&mut self, pos: (i32, i32, i32), at: [f64; 3]) {
        let used = match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Furnace(f)) => {
                let used = f.recipe_experience(world_recipes!(self));
                f.recipes_used.clear();
                used
            }
            _ => return,
        };
        for (count, experience) in used {
            let xp = horizon_client::containers::create_experience(
                count,
                experience,
                &mut self.world.random,
            );
            self.spawn_orbs(at, xp);
        }
    }
    /// `Player.giveExperiencePoints`, and `giveExperienceLevels`' level-up
    /// sound every fifth level (at most every 100 ticks).
    pub(super) fn give_orb_points(&mut self, gained: u32) {
        let at = self.eye_position();
        // Player.giveExperienceLevels runs per level, so the first multiple of 5
        // crossed plays the sound (and starts the cooldown for the rest).
        if let Some(level) = self.experience.give(gained as i32)
            && self.ticks > self.last_level_up + 100
        {
            self.last_level_up = self.ticks;
            let volume = if level > 30 { 1.0 } else { level as f32 / 30.0 };
            self.audio
                .play("minecraft:entity.player.levelup", at, volume * 0.75, 1.0);
        }
    }
    /// BowItem.releaseUsing: power from charge time, arrows at power * 3.
    fn release_bow(&mut self, ticks: u32) {
        let f = ticks as f32 / 20.0;
        let power = ((f * f + f * 2.0) / 3.0).min(1.0);
        if power < 0.1 {
            return;
        }
        let arrow_slot = self.projectile_slot("minecraft:bow");
        if arrow_slot.is_none() && !self.creative {
            return;
        }
        let Some(eye) = self.eye_position() else {
            return;
        };
        // BowItem.shootProjectile: shootFromRotation(power * 3, inaccuracy 1).
        let p = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let ammo = arrow_slot
            .and_then(|i| self.stack_in(i).cloned())
            .unwrap_or_else(|| ItemStack::new("minecraft:arrow", 1));
        let weapon = self.held().cloned();
        let (power_bonus, infinity) = self.bow_enchantments(&ammo.item);
        let effect = dispenser::tipped_effect(&ammo).or_else(|| {
            (&*ammo.item == "minecraft:spectral_arrow").then_some(("minecraft:glowing", 200))
        });
        let kind = horizon_client::projectiles::ProjectileKind::Arrow {
            base_damage: 2.0 + power_bonus,
            critical: power >= 1.0,
            effect,
        };
        let mut arrow = horizon_client::projectiles::Projectile::new(
            kind,
            p,
            Some(horizon_client::mobs::Who::Player),
        );
        // ProjectileWeaponItem.useAmmo: creative and Infinity draw without
        // using the ammo, so the arrow is CREATIVE_ONLY; else setOwner(player)
        // lets it be picked up.
        if let Some(a) = arrow.arrow.as_mut() {
            a.set_item(&horizon_client::arrows::drawn_ammo(
                &ammo,
                self.creative || infinity,
            ));
            a.set_owner(Some(horizon_client::mobs::Who::Player));
            a.fired_from_weapon = weapon;
        }
        // Flame: the arrow is spawned burning (EnchantmentHelper.onProjectileSpawned).
        if let Some(seconds) = self.bow_projectile_ignite()
            && let Some(a) = arrow.arrow.as_mut()
        {
            a.ignite_for_seconds(seconds);
        }
        let (velocity, on_ground) = self.player.as_ref().map_or((Vec3::ZERO, true), |pl| {
            (self.known_player_movement(), pl.ground.on_ground)
        });
        arrow.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            power * 3.0,
            1.0,
            velocity,
            on_ground,
        );
        self.add_projectile(arrow);
        self.player_projectile_shot();
        let pitch = 1.0 / (self.world.random.next_float() * 0.4 + 1.2) + power * 0.5;
        self.audio
            .play("minecraft:entity.arrow.shoot", Some(eye), 1.0, pitch);
        if !self.creative
            && !infinity
            && let Some(i) = arrow_slot
        {
            let cell = self.stack_in_mut(i);
            if let Some(s) = cell.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *cell = None;
                }
            }
        }
        self.damage_held(1);
    }
    /// `renderPlayerHand` (`AvatarRenderer.renderHand`): the player's own
    /// `left` or right arm with its skin at pose `m`, in its rest pose
    /// turned 0.1 radians outwards.
    fn first_person_arm(&self, left: bool, m: glam::Mat4, light: u32, out: &mut Vec<Vertex>) {
        let slim = self.player_model.as_ref().is_some_and(|m| m.slim);
        let Some(arm) = self.res.player_arm(slim, left).as_ref() else {
            return;
        };
        // The player's own skin, as the inventory's model shows it.
        let sprites = &self.gui.atlas.sprites;
        let Some(r) = self
            .player_model
            .as_ref()
            .and_then(|m| m.skin.as_deref())
            .and_then(|skin| sprites.get(skin))
            .or_else(|| sprites.get("entity/player/wide/steve"))
            .copied()
        else {
            return;
        };
        let z_rot = if left { -0.1 } else { 0.1 };
        let pose = |p: &horizon_client::mobs::Part| {
            let mut pose = p.pose;
            pose[3] = 0.0;
            pose[4] = 0.0;
            pose[5] = if p.name == arm.name { z_rot } else { 0.0 };
            pose
        };
        horizon_client::mobs::render_model(arm, m, &pose, [r.u0, r.v0, r.u1, r.v1], light, 0, out);
    }
    /// FirstPersonHandsAndItemsRenderer: the right arm or the held items of
    /// both hands in view space, converted to camera-relative world space.
    fn hand_geometry(&mut self, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        // GameRenderer.renderItemInHand: F1 hides the hand with the HUD.
        if self.spectator()
            || self.ui.hide_gui
            || self.sleeping.is_some()
            || !self.camera_type.first_person()
            || self
                .player
                .as_ref()
                .is_none_or(|p| p.health.dead_or_dying())
            || self.screen == Screen::Dead
        {
            return out;
        }
        let a = alpha as f32;
        // FirstPersonHandsAndItemsRenderer: the swing goes to the hand that
        // swings; each hand has its inverse arm height (the swap scale is 1
        // for every item but spears).
        let swing = self.swing.animation(a);
        let attacks = Hand::BOTH.map(|h| {
            if self.swing.attack_hand() == h {
                swing
            } else {
                0.0
            }
        });
        let heights = self.heights.interpolated(a);
        // The hand sway: both hands turn by a tenth of the bobs' lag.
        let to_world = self.sway.to_world(a, &self.view);
        let eye = self.eye_position().unwrap_or_default();
        let light = self.world.light(
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let shown = self.hands_to_render();
        let used_hand = self.used_hand();
        let spinning = self
            .player
            .as_ref()
            .is_some_and(|p| p.auto_spin_attack_ticks > 0);
        let items = self.visible_hand_items();
        let off_hand_empty = items[1].is_none();
        for (i, (hand, item)) in Hand::BOTH.into_iter().zip(items).enumerate() {
            if !shown[i] {
                continue;
            }
            let scale = item.as_ref().map_or(1.0, |stack| {
                self.pack
                    .item_renderer
                    .models
                    .swap_animation_scale(&horizon_client::item_render::stack_view(stack))
            });
            let attack = attacks[i];
            let inverse_height =
                horizon_client::first_person::equip_displacement(heights[i], scale);
            let mut canceled = false;
            self.native_render_hand(
                hand,
                item.as_ref(),
                a,
                attack,
                inverse_height,
                &mut canceled,
            );
            if canceled {
                continue;
            }
            // The main hand on its arm, the off hand on the other one.
            let left = self.settings.main_hand_left != (hand == Hand::Off);
            let Some(stack) = item else {
                // renderArmWithItem: only the main hand's empty arm shows,
                // on the main arm (the left one with the main hand set to
                // left), and not while the player is invisible.
                if hand == Hand::Off || self.own_invisible() {
                    continue;
                }
                // renderPlayerArm
                let m = to_world
                    * horizon_client::first_person::player_arm_pose(left, inverse_height, attack);
                self.first_person_arm(left, m, packed, &mut out);
                continue;
            };
            if let Some(id) = maps::map_id(&stack) {
                use horizon_client::first_person::{MapHold, map_arm_poses, one_handed_map_pose};
                let hold = MapHold::of(hand, off_hand_empty);
                // renderOneHandedMap and renderTwoHandedMap: the arms under
                // the map, unless the player is invisible.
                if !self.own_invisible() {
                    for (arm, pose) in map_arm_poses(hold, left, self.pitch, inverse_height, attack)
                    {
                        self.first_person_arm(arm, to_world * pose, packed, &mut out);
                    }
                }
                match hold {
                    MapHold::TwoHanded => self.two_handed_map_geometry(
                        to_world,
                        attack,
                        inverse_height,
                        id,
                        packed,
                        &mut out,
                    ),
                    MapHold::OneHanded => self.held_map(
                        to_world * one_handed_map_pose(left, inverse_height, attack),
                        id,
                        packed,
                        &mut out,
                    ),
                }
                continue;
            }
            let mut owner = self.player_owner(Some(&stack));
            // The other hand may hold an equal stack: only the used
            // hand's item is in use, only the main hand's selected.
            owner.using_this_item &= used_hand == Some(hand);
            if hand == Hand::Off {
                owner.is_selected = false;
                owner.is_carried = false;
            }
            // The hand renderer's copy of the main hand stack.
            owner.fishing_cast_with_this_item =
                self.main_hand_rod() && self.cast_rod() == Some(&stack);
            let using = owner.using_this_item && owner.use_item_remaining_ticks > 0;
            let ticks_used =
                owner.use_duration as f32 - (owner.use_item_remaining_ticks as f32 - a + 1.0);
            use horizon_client::first_person::ItemPose;
            // ItemUseAnimation.SPEAR.
            let spear = using
                .then(|| horizon_client::spear::Kinetic::of(&stack.item))
                .flatten()
                .map(|kinetic| ItemPose::Spear {
                    kinetic,
                    time_held: ticks_used,
                    ticks_since_hit: self.kinetic_hit_ticks(a),
                });
            let pose = match &*stack.item {
                _ if let Some(pose) = spear => pose,
                "minecraft:bow" if using => ItemPose::Bow { ticks_used },
                "minecraft:crossbow" => {
                    let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
                    ItemPose::Crossbow {
                        ticks_used: (using && !charged).then_some(ticks_used),
                        charge_duration: owner.crossbow_charge_duration as f32,
                        // Only the main hand turns a charged crossbow aside.
                        charged: charged && hand == Hand::Main,
                    }
                }
                "minecraft:trident" if using => ItemPose::Trident { ticks_used },
                // ItemUseAnimation.BRUSH: applyBrushTransform.
                "minecraft:brush" if using => ItemPose::Brush {
                    remaining: owner.use_item_remaining_ticks as f32,
                    partial: a,
                },
                _ if using => {
                    let components = self
                        .pack
                        .item_renderer
                        .models
                        .components(&horizon_client::item_render::stack_view(&stack));
                    let mut animation = components
                        .get("minecraft:consumable")
                        .map_or(horizon_world::consumable::UseAnimation::None, |c| {
                            horizon_client::first_person::consumable_animation(c)
                        });
                    self.native_use_animation(&stack, &mut animation);
                    item_use::use_pose(
                        animation,
                        owner.use_item_remaining_ticks as f32,
                        owner.use_duration as f32,
                        a,
                    )
                }
                _ if spinning => ItemPose::SpinAttack,
                // ItemInHandRenderer: a STAB swing of this hand jabs.
                _ if self
                    .swing
                    .current
                    .is_some_and(|c| c.hand == hand && c.animation.kind == 2) =>
                {
                    ItemPose::Stab
                }
                _ => ItemPose::Swing,
            };
            let m = to_world
                * horizon_client::first_person::item_pose(left, inverse_height, attack, pose);
            // Seeded by the player id plus the context ordinal.
            let (context, seed) = if left {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonLeftHand,
                    3,
                )
            } else {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonRightHand,
                    4,
                )
            };
            let state = self.resolve_item(&stack, context, Some(&owner), seed);
            let light = self.world.light(
                eye[0].floor() as i32,
                eye[1].floor() as i32,
                eye[2].floor() as i32,
            );
            self.submit_item_world(
                &state,
                m,
                horizon_client::block_entity_render::vertex_light(light),
                &mut out,
            );
        }
        // The hand has its own submit storage, which never uses improved
        // transparency (`LevelRenderer` alone calls `setUseImprovedTransparency`):
        // a held player head stays classic.
        for vertex in &mut out {
            vertex.layer &= !horizon_client::gfx::OIT_ENTITY;
        }
        out
    }
    /// The renderer for the window, with the current block sprites, GUI
    /// atlas and world border texture.
    fn new_renderer(&self, window: Arc<Window>) -> Result<Renderer> {
        self.renderer_for(window, &self.pack, &self.gui)
    }
    fn renderer_for(
        &self,
        window: Arc<Window>,
        pack: &PackResources,
        gui: &UiResources,
    ) -> Result<Renderer> {
        build_renderer(
            window,
            self.options.offscreen,
            pack,
            gui,
            self.renderer.as_ref(),
            None,
            self.settings.horizon.api,
        )
    }
    fn apply_settings(&mut self) {
        // LocalPlayer.refreshChatAbilities: the chat shows what the Chat option allows.
        let abilities = self.chat_abilities();
        self.ui.chat.set_abilities(&self.gui.atlas.font, abilities);
        self.streaming.radius = self.settings.render_distance;
        // Options.biomeBlendRadius, ambientOcclusion and cutoutLeaves:
        // LevelExtractor.allChanged when they change.
        let (s, st) = (&self.settings, &self.streaming);
        let remesh = (s.biome_blend, s.smooth_lighting, s.cutout_leaves)
            != (st.biome_blend, st.smooth_lighting, st.cutout_leaves);
        self.streaming.biome_blend = self.settings.biome_blend;
        self.streaming.smooth_lighting = self.settings.smooth_lighting;
        self.streaming.cutout_leaves = self.settings.cutout_leaves;
        if remesh {
            self.all_sections_changed();
        }
        self.audio.master = self.settings.master_volume;
        if self.settings.sound_device != self.audio.device() {
            let _ = self.audio.set_device(&self.settings.sound_device);
        }
        horizon_client::hud::GUI_SCALE.store(
            self.settings.gui_scale,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.apply_ui_settings();
    }
    /// `LevelExtractor.allChanged`: every loaded section is rebuilt.
    fn all_sections_changed(&mut self) {
        for &(x, z) in self.world.chunks.keys() {
            for sy in 0..self.world.layout().height() / 16 {
                self.world.dirty_sections.insert((x, sy, z));
            }
        }
    }
    /// Whether precipitation at a column is snow (Biome.coldEnoughToSnow:
    /// height adjusted temperature) or none (no precipitation).
    fn precipitation(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        self.res
            .biomes()
            .get(chunk.biome(x, y, z))
            .snows_at(x, y, z)
    }
    /// `Level.isRainingAt`: it rains, the sky light is 15, nothing that stops
    /// motion is above, and the biome's precipitation there is rain.
    fn raining_at(&self, p: BlockPos) -> bool {
        self.world.weather.levels(1.0).0 > 0.2
            && self.world.light(p.x, p.y, p.z) >> 4 == 15
            && self.rain_height(p.x, p.z) <= p.y
            && self.precipitation(p.x, p.y, p.z) == Some(false)
    }
    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block in `blocks_motion_in_heightmap` or holding a fluid.
    fn rain_height(&self, x: i32, z: i32) -> i32 {
        render_tick::motion_blocking_height(&self.world, &self.pack.assets.properties, x, z)
    }
    fn tick_weather_effects(&mut self) {
        if self.flash > 0 {
            self.flash -= 1;
        }
        // The rain sounds come with the rain splashes (`tick_particles`).
    }
    /// WeatherEffectRenderer: rain and snow columns within the weather radius.
    fn weather_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> Vec<horizon_client::weather::Vertex> {
        use horizon_client::weather;
        let (intensity, _) = self.world.weather.levels(alpha as f32);
        if intensity <= 0.0 {
            return Vec::new();
        }
        let atlas = &self.gui.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (
            atlas.sprites.get("environment/rain").copied(),
            atlas.sprites.get("environment/snow").copied(),
        ) else {
            return Vec::new();
        };
        let (cx, cy, cz) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        // The 32x32 direction table supports at most a 15-block radius.
        let radius = self.settings.weather_radius.clamp(0, 15);
        let eye = camera.to_array();
        let mut rain = Vec::new();
        let mut snow_vertices = Vec::new();
        // Saturation keeps extreme camera coordinates from overflowing the loop.
        for z in cz.saturating_sub(radius)..=cz.saturating_add(radius) {
            for x in cx.saturating_sub(radius)..=cx.saturating_add(radius) {
                let top = self.rain_height(x, z);
                let y0 = cy.saturating_sub(radius).max(top);
                let y1 = cy.saturating_add(radius).max(top);
                if y1 == y0 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, cy, z) else {
                    continue;
                };
                let parameters = self.weather_parameters.get(x, z);
                let light = self.world.light(x, top.max(cy), z);
                let coords = i32::from(light & 15) << 4 | i32::from(light >> 4) << 20;
                // Only light brightening is needed here; UV phases stay zero.
                let lc = if snow {
                    let (block, sky) = (coords >> 4 & 15, coords >> 20 & 15);
                    ((block * 3 + 15) / 4) << 4 | ((sky * 3 + 15) / 4) << 20
                } else {
                    coords
                };
                let column = weather::Column {
                    x,
                    z,
                    bottom_y: y0,
                    top_y: y1,
                    u_offset: 0.0,
                    v_offset: 0.0,
                    light_coords: lc,
                };
                let r = if snow { snow_tex } else { rain_tex };
                let vertices = weather::vertices(
                    &column,
                    parameters,
                    snow,
                    eye,
                    radius,
                    intensity,
                    [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0],
                );
                if snow {
                    snow_vertices.extend(vertices);
                } else {
                    rain.extend(vertices);
                }
            }
        }
        // Vanilla draws every rain column before the snow ones.
        rain.extend(snow_vertices);
        rain
    }
    /// A translated message for the player the game acts for: `overlay` is
    /// `sendOverlayMessage` (the action bar), else `sendSystemMessage` (chat).
    /// A guest gets a packet, so its own client translates it.
    fn tell_sleeper(&mut self, key: &str, fallback: &str, overlay: bool) {
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content: horizon_protocol::text::translatable(key, &[]),
                        overlay,
                    },
                ));
        } else if overlay {
            self.message = Some((self.translate(key, fallback), Instant::now()));
        } else {
            self.system_message(horizon_client::text::Text::translate(key, &[]));
        }
    }
    /// ServerPlayer.startSleepInBed: in reach, not obstructed, night or thunder,
    /// and (outside creative) no monsters within 8 blocks; the bed becomes the
    /// respawn point either way.
    fn try_sleep(&mut self, pos: [i32; 3]) {
        let Some((position, yaw)) = self.player.as_ref().map(|p| (p.position, p.yaw)) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        // The head and the foot: the block faces from the foot to the head.
        let d = facing_offset(props.get("facing").map_or("north", String::as_str));
        let (head, foot) = if props.get("part").map(String::as_str) == Some("foot") {
            ([pos[0] + d[0], pos[1], pos[2] + d[2]], pos)
        } else {
            (pos, [pos[0] - d[0], pos[1], pos[2] - d[2]])
        };
        // AbstractBedBlock.useWithoutItem: in an occupied bed a villager
        // asleep there is woken (kickVillagerOutOfBed), else someone else
        // sleeps in it; the player stays up either way.
        let head_state = self.world.block(head[0], head[1], head[2]);
        let occupied = self
            .pack
            .assets
            .registry
            .state(head_state)
            .is_some_and(|s| s.properties.get("occupied").is_some_and(|v| v == "true"));
        if occupied {
            if !self.kick_villager_out_of_bed(head) {
                self.tell_sleeper("block.minecraft.bed.occupied", "This bed is occupied", true);
            }
            return;
        }
        // ServerPlayer.bedInRange / isReachableBedBlock: within 3 blocks
        // sideways and 2 vertically of the head or the foot.
        let reachable = |bed: [i32; 3]| {
            (position.x - f64::from(bed[0]) - 0.5).abs() <= 3.0
                && (position.y - f64::from(bed[1])).abs() <= 2.0
                && (position.z - f64::from(bed[2]) - 0.5).abs() <= 3.0
        };
        if !reachable(head) && !reachable(foot) {
            self.tell_sleeper(
                "block.minecraft.bed.too_far_away",
                "You may not rest now; the bed is too far away",
                true,
            );
            return;
        }
        // ServerPlayer.bedBlocked: Player.freeAt (not suffocating) for the
        // blocks above the head and the foot, as `in_wall` reads suffocating.
        let blocked = [head, foot].iter().any(|bed| {
            let f = self
                .res
                .facts()
                .get(self.world.block(bed[0], bed[1] + 1, bed[2]));
            f.full_collision && f.solid_render
        });
        if blocked {
            self.tell_sleeper(
                "block.minecraft.bed.obstructed",
                "This bed is obstructed",
                true,
            );
            return;
        }
        // In reach (bedInRange), day or night: setRespawnPosition(
        // RespawnConfig(the head, the player's yaw), not forced). It replaces
        // an anchor or a spawnpoint; respawning checks the bed is still there
        // and finds where to stand up (findRespawnAndUseSpawnBlock).
        // Straw beds never set it (`straw_bed_rule`).
        if self.bed_sets_spawn(state) {
            let config = respawn_anchor::RespawnConfig::wrapped(self.dim, head, yaw, false);
            // ServerPlayer.setRespawnPosition: the chat message only when the
            // point moved (`isSamePosition`).
            let moved = self
                .respawn_config
                .is_none_or(|c| (c.dim, c.pos) != (config.dim, config.pos));
            self.respawn_config = Some(config);
            if moved {
                self.tell_sleeper("block.minecraft.set_spawn", "Respawn point set", false);
            }
            // The fallback becomes the world spawn: a guest's is none of its
            // own, so it follows `setworldspawn`.
            self.spawn = if self.lan_acting.is_some() {
                None
            } else {
                self.level
                    .world_spawn
                    .map(|[x, y, z]| {
                        Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                    })
                    .or(self.spawn)
            };
        }
        if !(self.dim == Dim::Overworld && self.sky_darken() >= 4) {
            self.tell_sleeper(
                "block.minecraft.bed.no_sleep",
                "You can sleep only at night or during thunderstorms",
                true,
            );
            return;
        }
        // Creative players skip the monster check (`!this.isCreative()`).
        let monsters = !self.creative
            && self.mobs.iter().any(|m| {
                // `Monster.class` entities that `isPreventingPlayerRest`: a
                // zombified piglin only while it is angry.
                horizon_client::mobs::ai::is_monster(m.species())
                    && (m.species() != horizon_client::mobs::Species::ZombifiedPiglin
                        || (m.extra.anger > 0 && m.ai.target.is_some()))
                    && m.alive()
                    && (m.position.x - position.x).abs() < 8.0
                    && (m.position.y - position.y).abs() < 5.0
                    && (m.position.z - position.z).abs() < 8.0
            });
        if monsters {
            self.tell_sleeper(
                "block.minecraft.bed.not_safe",
                "You may not rest now; there are monsters nearby",
                true,
            );
            return;
        }
        // ServerPlayer.startSleepInBed: a percentage above 100 cannot skip the night.
        if self.ui.rules.int("players_sleeping_percentage") > 100 {
            self.tell_sleeper(
                "sleep.not_possible",
                "No amount of rest can pass this night",
                true,
            );
        }
        self.stop_mining();
        self.left = false;
        self.right = false;
        self.using = None;
        self.sleeping = Some(head);
        self.sleep_timer = 0;
        // LivingEntity.startSleeping and ServerPlayer.startSleeping: setPosToBed
        // and connection.teleport.
        let at = Vec3::new(
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.5625 + 0.125,
            f64::from(head[2]) + 0.5,
        );
        if let Some(p) = self.player.as_mut() {
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.pitch = 0.0;
            p.teleported = true;
        }
        self.pitch = 0.0;
        if self.lan_acting.is_none() {
            self.previous_position = at;
            self.snap_player_model_pitch();
        }
        self.occupy_bed(head);
        self.stats.custom("sleep_in_bed", 1);
        self.trigger_player("minecraft:slept_in_bed");
        self.stats.reset("custom", "minecraft:time_since_rest");
    }
    fn wake_up(&mut self) {
        if self.remote.is_some() {
            // InBedChatScreen.sendWakeUp: the server wakes the player.
            self.remote_stop_sleeping();
            return;
        }
        let Some(head) = self.sleeping.take() else {
            return;
        };
        self.sleep_timer = 100;
        let stand = self.bed_wake_pose(head, self.yaw);
        if let Some(p) = self.player.as_mut() {
            if let Some((at, yaw)) = stand {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.yaw = yaw;
                p.pitch = 0.0;
            }
            let _ = p.set_sleeping(false);
        }
        if let Some((at, yaw)) = stand {
            self.previous_position = at;
            self.yaw = yaw;
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
    }
    fn tick_sleep(&mut self) {
        self.sleep_timer = tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        if let Some(head) = self.sleeping {
            // LivingEntity.tick's checkBedExists: a sleeper whose bed is
            // gone gets up.
            let bed = self.world.block(head[0], head[1], head[2]);
            let bed_gone = !self.res.facts().get(bed).name.ends_with("_bed");
            // Vanilla has no Shift wake-up: the chat screen's capitals must
            // not get the sleeper up.
            if (self.screen != Screen::Chat && self.keys.contains(&KeyCode::ShiftLeft))
                || bed_gone
                || self
                    .player
                    .as_ref()
                    .is_none_or(|p| p.health.dead_or_dying())
            {
                self.wake_up();
                return;
            }
            if let Some(p) = self.player.as_mut() {
                p.position = Vec3::new(
                    f64::from(head[0]) + 0.5,
                    // setPosToBed: the bed's top (getSleepHeight) and 0.125 above it.
                    f64::from(head[1]) + 0.5625 + 0.125,
                    f64::from(head[2]) + 0.5,
                );
                p.velocity = Vec3::ZERO;
                p.pitch = 0.0;
            }
            // LivingEntity.tick: a sleeper's xRot is 0.
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // ServerLevel.tick: once enough of the players have slept 100 ticks,
        // skip to morning if `advance_time` is on, wake everyone, and
        // resetWeatherCycle if `advance_weather` is on and it is raining.
        if self.enough_slept() {
            if self.ui.rules.bool("advance_time") {
                let t = self.world.day_time;
                self.world.day_time = t + 24000 - t.rem_euclid(24000);
            }
            // Level.isRaining: the rain level is above 0.2.
            if self.ui.rules.bool("advance_weather") && self.world.weather.levels(1.0).0 > 0.2 {
                self.world.weather.raining = false;
                self.world.weather.thundering = false;
                self.world.weather.rain_time = 0;
                self.world.weather.thunder_time = 0;
            }
            if self.sleeping.is_some() {
                self.wake_up();
            }
            self.wake_guests();
            // SleepStatus.removeAllSleepers: waking everyone is not announced.
            self.sleep_seen.1 = 0;
        }
    }
    fn creative_origin(batch: &HudBatch) -> (f32, f32) {
        batch.centered_origin(Creative::WIDTH, Creative::HEIGHT)
    }
    /// A creative tab's title: `itemGroup.*` for vanilla's; a mod tab's
    /// `itemGroup.<mod>.<name>`, else its `title` as a lang key (a mod's own
    /// `itemGroup.<mod>`), else `title` as English text.
    fn creative_tab_title(&self, tab: &horizon_client::creative::Tab) -> String {
        match horizon_client::creative::item_group_key(&tab.id) {
            Some(key) => self.translate(key, &tab.title),
            None => {
                let own = self.translate(&tab.title, &tab.title);
                self.translate(&format!("itemGroup.{}", tab.id.replace(':', ".")), &own)
            }
        }
    }
    /// Vanilla tooltip: dark box with a purple gradient border, laid out and
    /// drawn like an item tooltip (4 px of padding on every side).
    fn tooltip(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32) {
        let atlas = &self.gui.atlas;
        let lines = vec![(horizon_client::text::Text::literal(text), u32::MAX)];
        let (x, y, w, h) =
            Self::tooltip_geometry(atlas, &lines, (x, y), (batch.width, batch.height), [0.0; 2]);
        Self::tooltip_box(batch, atlas, x, y, w, h);
        Self::tooltip_text(batch, atlas, &lines, x, y);
    }
    /// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the lines
    /// that follow the item's name, none for a category tab's own grid slots
    /// (`grid`): the names of the tabs containing the stack, in blue, then on
    /// the search tab's grid the tags a `#` search shows, in dark purple.
    fn creative_tooltip_tabs(
        &self,
        stack: &ItemStack,
        grid: bool,
    ) -> Vec<(horizon_client::text::Text, u32)> {
        let screen = &self.creative_screen;
        let tab = screen.tab();
        let category = !tab.search && !tab.inventory;
        if !horizon_client::creative::tooltip_lists_tabs(category, grid) {
            return Vec::new();
        }
        let blue = rgba(85, 85, 255, 255);
        let dark_purple = rgba(170, 0, 170, 255);
        let mut lines: Vec<(horizon_client::text::Text, u32)> = screen
            .tabs_containing(stack)
            .into_iter()
            .map(|i| {
                let title = self.creative_tab_title(&screen.tabs[i]);
                (horizon_client::text::Text::literal(title), blue)
            })
            .collect();
        if tab.search && grid {
            for tag in screen.search_tag_lines(stack) {
                lines.push((horizon_client::text::Text::literal(tag), dark_purple));
            }
        }
        lines
    }
    fn draw_creative(&mut self, batch: &mut HudBatch) {
        // CreativeModeInventoryScreen: `CommonComponents.EMPTY` for a title.
        screen_info::begin_container("creative");
        let (ox, oy) = Self::creative_origin(batch);
        let selected = self.creative_screen.selected;
        // Only what is drawn of the tabs: their stacks stay where they are.
        let tabs = self.creative_screen.tab_headers();
        let tab = &tabs[selected.min(tabs.len() - 1)];
        let tab_sprite = |t: &horizon_client::creative::Tab, on: bool| {
            format!(
                "gui/sprites/container/creative_inventory/tab_{}_{}_{}",
                if t.top { "top" } else { "bottom" },
                if on { "selected" } else { "unselected" },
                t.column + 1
            )
        };
        batch.fill(
            &self.gui.atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(16, 16, 16, 160),
        );
        // Unselected tabs behind the panel, then the panel, then the selected tab.
        for (i, t) in tabs.iter().enumerate() {
            if i != selected && self.creative_screen.on_page(t) {
                let (tx, ty) = Creative::tab_position(t);
                batch.sprite(&self.gui.atlas, &tab_sprite(t, false), ox + tx, oy + ty);
                let mouse = (self.cursor.0 - ox, self.cursor.1 - oy);
                if Creative::tab_wants_hand(t, false, mouse) {
                    horizon_client::widgets::request_cursor(
                        horizon_client::widgets::Cursor::PointingHand,
                    );
                }
            }
        }
        let background = if tab.search {
            "gui/container/creative_inventory/tab_item_search"
        } else if tab.inventory {
            "gui/container/creative_inventory/tab_inventory"
        } else {
            "gui/container/creative_inventory/tab_items"
        };
        if let Some(mut r) = self.gui.atlas.sprites.get(background).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * Creative::WIDTH / r.w as f32;
            r.v1 = r.v0 + (r.v1 - r.v0) * Creative::HEIGHT / r.h as f32;
            batch.rect_uv(ox, oy, Creative::WIDTH, Creative::HEIGHT, r, u32::MAX);
        }
        let (tx, ty) = Creative::tab_position(tab);
        batch.sprite(&self.gui.atlas, &tab_sprite(tab, true), ox + tx, oy + ty);
        for t in tabs.iter().filter(|t| self.creative_screen.on_page(t)) {
            let (tx, ty) = Creative::tab_position(t);
            let stack = ItemStack {
                item: Arc::from(t.icon.as_str()),
                count: 1,
                damage: 0,
                extra: None,
            };
            self.draw_item(
                batch,
                &stack,
                ox + tx + 5.0,
                oy + ty + if t.top { 9.0 } else { 7.0 },
            );
        }
        let atlas = &self.gui.atlas;
        if !tab.inventory {
            let title = self.creative_tab_title(tab);
            batch.text(
                atlas,
                &title,
                ox + 8.0,
                oy + 6.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }
        if tab.search {
            // CreativeModeInventoryScreen: an unbordered EditBox of 50 at (82, 6).
            let search = &mut self.creative_screen.search;
            search.x = ox + 82.0;
            search.y = oy + 6.0;
            search.width = 80.0;
            search.bordered = false;
            search.focused = true;
            search.text_color = u32::MAX;
            search.max_length = clipboard::CREATIVE_SEARCH_CHARS;
            if horizon_client::widgets::narrating() {
                search.message = screen_info::tr("itemGroup.search", &[]);
            }
            search.draw(batch, atlas, None);
        }
        // The hovered stack, and whether it is a grid slot (`CustomCreativeSlot`).
        let mut hovered: Option<(ItemStack, bool)> = None;
        if tab.inventory {
            self.draw_player_preview(
                batch,
                ox + 73.0,
                oy + 6.0,
                ox + 105.0,
                oy + 49.0,
                20.0,
                0.0625,
            );
            let trash = Self::creative_trash_hovered(batch, self.cursor);
            let (tx, ty) = Creative::TRASH_POSITION;
            if trash {
                self.draw_slot_highlight(batch, false, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, false, ox, oy);
            self.draw_inventory_slots(batch, ox, oy);
            if trash {
                self.draw_slot_highlight(batch, true, ox + tx, oy + ty);
            }
            self.draw_hovered_slot_highlight(batch, true, ox, oy);
        } else {
            let items = self.creative_screen.items(&self.pack.names);
            let first = self.creative_screen.first_row(&self.pack.names);
            for r in 0..5 {
                for c in 0..9 {
                    let Some(stack) = items.get((first + r) * 9 + c).filter(|s| s.count > 0) else {
                        continue;
                    };
                    let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 18.0 + r as f32 * 18.0);
                    let over = self.cursor.0 >= x
                        && self.cursor.0 < x + 16.0
                        && self.cursor.1 >= y
                        && self.cursor.1 < y + 16.0;
                    if over {
                        self.draw_slot_highlight(batch, false, x, y);
                    }
                    self.draw_item(batch, stack, x, y);
                    if over {
                        self.draw_slot_highlight(batch, true, x, y);
                        hovered = Some((stack.clone(), true));
                    }
                }
            }
            let drag = self.drag_view();
            for c in 0..9 {
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
                let over = self.cursor.0 >= x
                    && self.cursor.0 < x + 16.0
                    && self.cursor.1 >= y
                    && self.cursor.1 < y + 16.0;
                if over {
                    self.draw_slot_highlight(batch, false, x, y);
                }
                self.draw_slot_item(batch, drag.as_ref(), c, &self.inventory[c], x, y);
                if over {
                    self.draw_slot_highlight(batch, true, x, y);
                    hovered = self.inventory[c].clone().map(|s| (s, false));
                }
            }
            let scrollable = self.creative_screen.rows(&self.pack.names) > 5;
            let scroller = if scrollable {
                "gui/sprites/container/creative_inventory/scroller"
            } else {
                "gui/sprites/container/creative_inventory/scroller_disabled"
            };
            batch.sprite(
                &self.gui.atlas,
                scroller,
                ox + Creative::SCROLL_X,
                oy + Creative::SCROLL_Y + self.creative_screen.scroller_offset(),
            );
        }
        let effect_tip = self.draw_inventory_effects(batch, (ox, oy), Creative::WIDTH as i32);
        self.draw_creative_pages(batch, ox, oy);
        for t in tabs.iter().filter(|t| self.creative_screen.on_page(t)) {
            let (tx, ty) = Creative::tab_position(t);
            if self.cursor.0 >= ox + tx
                && self.cursor.0 < ox + tx + 26.0
                && self.cursor.1 >= oy + ty
                && self.cursor.1 < oy + ty + 32.0
            {
                self.tooltip(
                    batch,
                    &self.creative_tab_title(t),
                    self.cursor.0,
                    self.cursor.1,
                );
            }
        }
        self.draw_carried(batch);
        if tab.inventory {
            if Self::creative_trash_hovered(batch, self.cursor) {
                // `CreativeModeInventoryScreen`: the destroy slot's tooltip is
                // `inventory.binSlot`, not an English literal.
                let destroy = self.translate("inventory.binSlot", "Destroy Item");
                self.tooltip(batch, &destroy, self.cursor.0, self.cursor.1);
            } else if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1)
                && let Some(stack) = self.slot_view(slot)
                && self.tooltip_with_carried(&stack)
            {
                let tabs = self.creative_tooltip_tabs(&stack, false);
                self.draw_stack_tooltip_with(
                    batch,
                    &stack,
                    false,
                    self.cursor.0,
                    self.cursor.1,
                    tabs,
                );
            }
        } else if let Some((item, grid)) = hovered
            && self.tooltip_with_carried(&item)
        {
            let tabs = self.creative_tooltip_tabs(&item, grid);
            // A `CustomCreativeSlot`: TooltipFlag.asCreative.
            self.draw_stack_tooltip_with(batch, &item, grid, self.cursor.0, self.cursor.1, tabs);
        }
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    fn creative_trash_hovered(batch: &HudBatch, cursor: (f32, f32)) -> bool {
        let (ox, oy) = Self::creative_origin(batch);
        let (x, y) = Creative::TRASH_POSITION;
        cursor.0 >= ox + x
            && cursor.0 < ox + x + 16.0
            && cursor.1 >= oy + y
            && cursor.1 < oy + y + 16.0
    }
    /// NeoForge's creative page buttons, 20 square, 50 above the screen at
    /// its left and right edges: -1 or 1 for the one at `cursor`, when there
    /// is more than one page.
    fn creative_page_button(ox: f32, oy: f32, cursor: (f32, f32), pages: u32) -> Option<i64> {
        if pages <= 1 {
            return None;
        }
        let y = oy - 50.0;
        let inside =
            |x: f32| cursor.0 >= x && cursor.0 < x + 20.0 && cursor.1 >= y && cursor.1 < y + 20.0;
        if inside(ox) {
            Some(-1)
        } else if inside(ox + Creative::WIDTH - 20.0) {
            Some(1)
        } else {
            None
        }
    }
    /// The page buttons and "page / pages" between them.
    fn draw_creative_pages(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let pages = self.creative_screen.pages();
        if pages <= 1 {
            return;
        }
        let atlas = &self.gui.atlas;
        let y = oy - 50.0;
        for (x, label, step) in [(ox, "<", -1), (ox + Creative::WIDTH - 20.0, ">", 1)] {
            let hover = Self::creative_page_button(ox, oy, self.cursor, pages) == Some(step);
            let page = self.creative_screen.page as i64 + step;
            let active = (0..pages as i64).contains(&page);
            let sprite = match (active, hover) {
                (false, _) => "gui/sprites/widget/button_disabled",
                (true, true) => "gui/sprites/widget/button_highlighted",
                (true, false) => "gui/sprites/widget/button",
            };
            horizon_client::ui::blit_sprite(batch, atlas, sprite, x, y, 20.0, 20.0, u32::MAX);
            let color = if active {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            let w = HudBatch::text_width(atlas, label);
            batch.text(atlas, label, x + (20.0 - w) / 2.0, y + 6.0, color, true);
        }
        let text = format!("{} / {}", self.creative_screen.page + 1, pages);
        let w = HudBatch::text_width(atlas, &text);
        batch.text(
            atlas,
            &text,
            ox + (Creative::WIDTH - w) / 2.0,
            oy - 44.0,
            u32::MAX,
            true,
        );
    }
    fn click_creative(&mut self, right: bool) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size);
        let (ox, oy) = Self::creative_origin(&batch);
        let (cx, cy) = self.cursor;
        // NeoForge's page buttons, when mods add tabs.
        if let Some(step) =
            Self::creative_page_button(ox, oy, (cx, cy), self.creative_screen.pages())
        {
            // Page buttons are outside the inventory panel; their release must
            // not throw a carried stack, including when the button is disabled.
            self.inv_clicks.skip_release();
            let page = self.creative_screen.page as i64 + step;
            if (0..self.creative_screen.pages() as i64).contains(&page) {
                self.audio.play_button_click();
                self.creative_screen.set_page(page as u32);
                self.refresh_saved_hotbars();
            }
            return;
        }
        let tabs = self.creative_screen.tabs.clone();
        for (i, t) in tabs.iter().enumerate() {
            if !self.creative_screen.on_page(t) {
                continue;
            }
            let (tx, ty) = Creative::tab_position(t);
            if cx >= ox + tx && cx < ox + tx + 26.0 && cy >= oy + ty && cy < oy + ty + 32.0 {
                select_creative_tab(&mut self.creative_screen, &mut self.inv_clicks, i);
                self.refresh_saved_hotbars();
                self.inv_drag = None;
                self.last_inv_click = None;
                return;
            }
        }
        if self.creative_inventory_open() {
            if Self::creative_trash_hovered(&batch, self.cursor) {
                if self.has_shift_down() {
                    let before = self.player_equipment();
                    self.inventory.fill(None);
                    self.armor.fill(None);
                    self.offhand = None;
                    self.craft_grid[..4].fill(None);
                    self.player_equipment_changed(&before);
                    self.apply_equipment();
                } else {
                    self.carried = None;
                }
                self.inv_drag = None;
                self.inv_clicks = Default::default();
            } else if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
                self.inventory_press(right);
            }
            return;
        }
        if !right && self.creative_search_click(ox, oy) {
            return;
        }
        // Holding the scrollbar scrolls as the mouse moves (mouseDragged).
        if !right
            && self
                .creative_screen
                .press_scrollbar(cx - ox, cy - oy, &self.pack.names)
        {
            return;
        }
        if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
            self.inventory_press(right);
        }
    }
    /// A click on the search tab's box (`EditBox.onClick`).
    fn creative_search_click(&mut self, ox: f32, oy: f32) -> bool {
        if !self.creative_screen.tab().search {
            return false;
        }
        let (x, y, w, h) = (ox + 82.0, oy + 6.0, 80.0, 9.0);
        let (mx, my) = self.cursor;
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }
        let shift = self.has_shift_down();
        let search = &mut self.creative_screen.search;
        search.x = x;
        search.y = y;
        search.width = w;
        search.click(&self.gui.atlas, mx, shift, false);
        true
    }
    /// Copy the live player and world state into the `Level` that gets saved.
    pub(super) fn sync_level(&mut self) {
        // Level.write saves the game rules (game_rules.dat) with the rest.
        self.level.game_rules = self.ui.rules.by_name();
        // PrimaryLevelData.setDifficulty: /difficulty and the options button change it.
        self.level.difficulty =
            horizon_client::title::DIFFICULTIES[usize::from(self.ui.difficulty) & 3].into();
        self.level.time = self.world.game_time;
        self.level.weather = self.world.weather;
        self.level.day_time = self.world.day_time;
        // A server has no in-frame player of its own; `enter_guest` puts one in
        // `self.player` only while a command reads it.
        if self.player.is_none() && self.options.dedicated.is_some() {
            return;
        }
        self.level.selected_slot = self.selected as i32;
        let mode = self.mode_id();
        // The vehicle the player rides (`ServerPlayer.saveParentVehicle`).
        let others = self
            .lan
            .as_ref()
            .map(|h| self.ridden_by_others(&h.hub.guests, self.dim.id(), None, false))
            .unwrap_or_default();
        let vehicle = self.parent_vehicle_tag(self.riding, &others);
        // Only the local player's file is this one's to rewrite.
        let local = self.serving.is_none() && self.options.dedicated.is_none();
        let level = &mut self.level;
        level.creative = self.creative;
        level.game_mode = mode;
        level.previous_game_mode = self.previous_mode.map(game_mode::mode_id_of);
        level.spawn = self.spawn.map(|s| [s.x, s.y, s.z]);
        level.respawn_elsewhere = self.respawn_config.map(|c| c.saved());
        level.rotation = [self.yaw, self.pitch];
        level.xp_level = self.experience.level;
        level.xp_progress = self.experience.progress;
        level.xp_total = self.experience.total;
        level.root_vehicle = vehicle.as_ref().map(|v| v.2).or(self.pending_vehicle);
        // The ride's tree, or none once the file's own came back.
        match vehicle {
            Some((tag, ..)) => {
                let extra = level
                    .player_extra
                    .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
                if let horizon_world::nbt::Tag::Compound(map) = extra {
                    map.insert("RootVehicle".into(), tag);
                }
            }
            None if local && self.pending_vehicle.is_none() => {
                if let Some(horizon_world::nbt::Tag::Compound(map)) = &mut level.player_extra {
                    map.remove(&"RootVehicle".into());
                }
            }
            None => {}
        }
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.absorption = p.health.absorption();
            let extra = level
                .player_extra
                .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
            if let horizon_world::nbt::Tag::Compound(map) = extra {
                map.insert(
                    "Fire".into(),
                    horizon_world::nbt::Tag::Short(
                        p.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                    ),
                );
                map.insert("Air".into(), horizon_world::nbt::Tag::Short(p.air as i16));
                // LivingEntity.addAdditionalSaveData: AttributeMap.pack.
                map.insert("attributes".into(), p.saved_attributes());
            }
            level.fall_distance = p.fall_distance;

            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
            level.flying = p.flying;
        }
        level.inventory = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref()
                    .map(|s| (i as u8, s.item.to_string(), s.count, s.damage))
            })
            .collect();
        level.inventory_components = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i as u8, s.as_ref()?.extra.clone()?)))
            .collect();
        if let Some(s) = &self.offhand {
            level
                .inventory
                .push((150, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = s.extra.clone() {
                level.inventory_components.insert(150, extra);
            }
        }
        // Vanilla stores armor in slots 100 (feet) to 103 (head).
        for (i, s) in self.armor.iter().enumerate() {
            if let Some(s) = s {
                level
                    .inventory
                    .push((103 - i as u8, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(103 - i as u8, extra);
                }
            }
        }
        level.dimension = self.dim.id().to_owned();
        // An open ender chest holds the live copy.
        if let Some(key) = self.ender_open
            && let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key)
        {
            self.ender_items = items.clone();
        }
        level.ender_items = self.ender_items.clone();
        level.active_effects = self.potions.effects.saved();
    }
    fn save_game(&mut self) -> Result<()> {
        self.save_game_with(true)
    }
    /// The autosave doesn't `wait` for the chunks to be on disk: every
    /// generated chunk is saved, hundreds at a time, encoded and written on
    /// the region writer meanwhile.
    fn save_game_with(&mut self, wait: bool) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        self.save_level(wait)?;
        self.save_parked_levels(wait)?;
        self.save_game_metadata()
    }
    fn save_game_metadata(&mut self) -> Result<()> {
        self.save_maps();
        self.save_random_sequences();
        self.save_fight();
        self.save_wandering_trader();
        self.save_portal_arrivals();
        self.save_stats();
        self.save_advancements();
        self.save_scoreboard();
        self.save_bossbars();
        self.save_schedule();
        self.save_warden_tracker();
        self.sync_level();
        self.store_owner();
        write_level_for_role(
            &mut self.level,
            &self.world_root,
            self.options.execution_role,
        )?;
        // PlayerList.saveAll: every placed player saves with the server, to
        // their own file (`PlayerDataStorage.save`); the owner is also in the
        // file level.dat names, which is the one it loads from
        // (`playerdata::load_owner`).
        if self.options.dedicated.is_some()
            && !self.options.execution_role.is_client()
            && let Some(host) = self.lan.take()
        {
            // The vehicle tree goes into a player's file when they leave
            // (`take_parent_vehicle`); here it stays in the chunk saves.
            for g in host.hub.guests.iter().filter(|g| g.placed) {
                horizon_client::lan::playerdata::save(g, &self.world_root);
            }
            self.lan = Some(host);
        }
        self.save_world_state();
        self.message = Some(("Saved the game".into(), Instant::now()));
        Ok(())
    }
    /// Save the level in place: its changed chunks, its entities and its raids.
    fn save_level(&mut self, wait: bool) -> Result<()> {
        self.save.save_forced_chunks(&self.streaming.forced)?;
        self.streaming.save(&mut self.world, wait);
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        Ok(())
    }
    /// `GameRenderer.tryTakeScreenshotIfNeeded`: while a local world without a
    /// saved icon is on the screen, grab the rendered frame once a second until
    /// the save has one. The Select World list shows it as the world's preview.
    fn tick_world_icon(&mut self) {
        let Some(icon) = self.world_icon.as_ref() else {
            return;
        };
        if icon
            .last_attempt
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        if icon.path.is_file() {
            // tryTakeScreenshotIfNeeded's hasWorldScreenshot: it has one, stop asking.
            self.world_icon = None;
            return;
        }
        // takeAutoScreenshot only saves with every loaded section rendered, or
        // the icon would picture the holes the streamer has not filled yet.
        if self.streaming.pending() != 0
            || !self.pending_uploads.is_empty()
            || self.remote_backlog() != 0
        {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // countRenderedSections() > 10: a client that joined its server
        // thread has nothing pending before the first chunks arrive, and the
        // icon would be only sky.
        if renderer.drawn_sections <= 10 {
            return;
        }
        if renderer.screenshot.is_some() {
            // F2's or an agent's screenshot owns the frame this time; ours
            // comes around again with the next attempt.
            return;
        }
        let path = icon.path.clone();
        if let Some(icon) = self.world_icon.as_mut() {
            icon.last_attempt = Some(Instant::now());
        }
        renderer.screenshot = Some(horizon_client::gfx::ScreenshotRequest {
            path,
            size: Some([64, 64]),
            scaled: false,
        });
    }
    fn render(&mut self) -> Result<bool> {
        self.glint_frame = self.capture_glint_frame();
        if self.renderer.as_mut().is_some_and(|r| r.display_changed()) {
            self.apply_ui_settings();
        }
        // Minecraft.doWorldLoad: the LevelLoadingScreen while single player's
        // server starts, with the server's chunk map and progress.
        if let Some(starting) = &self.integrated_starting {
            // The server's progress once it has some (`hasProgress`).
            let progress = Some(starting.handle.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            horizon_client::widgets::begin_screen(|| {
                screen_info::level_loading(progress, &screen_info::tr)
            });
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                let (w, h) = (hud.width, hud.height);
                horizon_client::ui::menu_background(
                    &mut hud,
                    &self.gui.atlas,
                    0.0,
                    0.0,
                    w,
                    h,
                    false,
                );
                // The chunk map and progress are the server thread's.
                let view = self
                    .integrated_starting
                    .as_ref()
                    .map(|s| s.handle.load_view().clone());
                let status = |x: i32, z: i32| view.as_ref().and_then(|v| v.status(x, z));
                let progress = view
                    .as_ref()
                    .is_some_and(|v| v.has_progress())
                    .then_some(self.loading_progress);
                horizon_client::level_loading::draw(
                    &mut hud,
                    &self.gui.atlas,
                    Some(&status),
                    progress,
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        if let Some(loader) = &self.world_loader {
            // The world opens before its LevelLoadingScreen has progress;
            // FileFixerProgressScreen narrates its title.
            if loader.upgrade.is_file_fixing() && !loader.cancelled {
                horizon_client::widgets::begin_screen(|| {
                    horizon_client::widgets::ScreenInfo::new(
                        "file_fixer",
                        screen_info::tr("upgradeWorld.title", &[]),
                    )
                });
            } else {
                horizon_client::widgets::begin_screen(|| {
                    screen_info::level_loading(None, &screen_info::tr)
                });
            }
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                // WorldOpenFlows.upgradeAndOpenWorld: the file fixers run on
                // FileFixerProgressScreen.
                if loader.upgrade.is_file_fixing() && !loader.cancelled {
                    horizon_client::level_loading::draw_file_fixer(
                        &mut hud,
                        &self.gui.atlas,
                        &loader.upgrade,
                        self.cursor,
                    );
                    return renderer.render(&loading_frame(&hud));
                }
                horizon_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    &if loader.cancelled {
                        horizon_client::lang::get_or(
                            "horizon.loading.cancelling_world",
                            "Cancelling world",
                        )
                    } else {
                        horizon_client::lang::get_or(
                            "horizon.loading.opening_world",
                            "Opening world",
                        )
                    },
                    0.0,
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        if self.pending_entry.is_some() {
            if let (Some(renderer), Some(loader)) = (&mut self.renderer, &self.rest_loader) {
                horizon_client::widgets::begin_screen(|| {
                    screen_info::level_loading(Some(loader.progress.fraction()), &screen_info::tr)
                });
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                horizon_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    loader.progress.line(),
                    loader.progress.fraction(),
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        // A pack reload covers the frame with its progress
        // (`LoadingOverlay`) until `drive_pack_reload` applies it.
        if let Some(reload) = &self.pack_reload {
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                horizon_client::level_loading::draw_loading_overlay(
                    &mut hud,
                    &self.gui.atlas,
                    reload.progress.line(),
                    reload.progress.fraction(),
                    self.settings.file.bool("darkMojangStudiosBackground"),
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        let alpha = self.clock.fraction();
        // Camera.getCameraEntityPartialTicks: the view, the hand and the
        // player's own model stay smooth while the level is frozen.
        let camera_alpha = self.clock.camera_fraction();
        let view = self.camera_view(camera_alpha as f32);
        let eye = view.map(|_| {
            (
                glam::DVec3::from_array(self.view.eye),
                self.view.eye_rotation,
            )
        });
        let (camera, underwater, view_yaw, view_pitch) = match view {
            Some((c, yaw, pitch)) => {
                let s =
                    self.world
                        .block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts().get(s).fluid == Fluid::Water, yaw, pitch)
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.yaw,
                self.pitch,
            ),
        };
        self.audio
            .set_listener([camera.x, camera.y, camera.z], view_yaw, view_pitch);
        let block = [
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        ];
        // The camera inside an opaque block sees nothing through it: disable.
        let cave_culling = self.settings.horizon.cave_culling
            && !self
                .res
                .facts()
                .get(self.world.block(block[0], block[1], block[2]))
                .solid_render;
        // The cave culling search runs on its worker while the frame is prepared.
        if cave_culling
            && self.title.is_none()
            && let Some(renderer) = self.renderer.as_mut()
        {
            horizon_client::spans::time("occlusion", || {
                renderer.start_occlusion(camera, (self.streaming.radius * 16) as f32)
            });
        }
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let environment = self.dim_environment();
        // EnvironmentAttributeProbe: the spatially interpolated biome colours,
        // Gaussian-weighted over the noise biomes around the camera (plains
        // where no chunk is loaded, as ClientLevel.getUncachedNoiseBiome).
        let biomes = world_biomes!(self);
        let weights = {
            let chunks = &self.world.chunks;
            let plains = biomes.id("minecraft:plains");
            horizon_client::biome::gaussian_weights(camera.to_array(), |qx, qy, qz| {
                chunks
                    .get(&(qx >> 2, qz >> 2))
                    .map_or(plains, |c| c.biome(qx * 4, qy * 4, qz * 4))
            })
        };
        let blend = |pick: fn(&horizon_client::biome::BiomeInfo) -> Option<u32>, base: [f32; 3]| {
            horizon_client::biome::blend_rgb(&weights, |b| {
                pick(biomes.get(b)).map_or(base, horizon_client::biome::rgb)
            })
        };
        let biome_sky = blend(
            |b| b.sky,
            environment.base_rgb("minecraft:visual/sky_color"),
        );
        let biome_fog = blend(
            |b| b.fog,
            environment.base_rgb("minecraft:visual/fog_color"),
        );
        // visual/water_fog_color: no dimension sets it, its default is #050533.
        let water_fog = blend(|b| Some(b.water_fog), [0.0; 3]);
        let mut sky = environment.sample(self.world.day_time, Some(biome_sky), Some(biome_fog));
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(alpha as f32)
        } else {
            (0.0, 0.0)
        };
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        self.end_flash_sky(&mut sky, alpha);
        if self.flash > 0 && !self.settings.file.bool("hideLightningFlashes") {
            // ClientLevel's flash layer while skyFlashTime > 0: the sky colour
            // moves 22% of the way to (0.8, 0.8, 1.0).
            let flash = [0.8, 0.8, 1.0];
            sky.sky_color =
                std::array::from_fn(|i| sky.sky_color[i] + (flash[i] - sky.sky_color[i]) * 0.22);
            sky.sky_light_factor = 1.0;
        }
        // FishingHookRenderer's line, then DebugRenderer: chunk borders and
        // hitboxes (F3+G, F3+B).
        let mut lines = self.fishing_line(alpha, size);
        let g = self.debug_gizmos(alpha as f32, camera.to_array());
        lines.extend(g.lines);
        let on_top = g.on_top;
        let colored_lights = if self.title.is_none() {
            self.native_frame_visuals(camera, &mut lines)
        } else {
            Vec::new()
        };
        let emissive_layers = if self.title.is_none() {
            let mut requests = Vec::new();
            self.native_emissive(&mut requests);
            horizon_client::native_emissive::mappings(
                &self.pack.render.sprite_layers,
                requests
                    .into_iter()
                    .take(horizon_client::native_emissive::MAX),
            )
        } else {
            Vec::new()
        };
        let view = horizon_client::camera::view_matrix(view_yaw, view_pitch);
        let at = camera.to_array();
        // GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown.
        let mut crosshair = horizon_client::gizmos::Gizmos::default();
        if debug_crosshair_drawn(
            self.debug_crosshair(),
            self.camera_type.first_person(),
            self.ui.hide_gui,
            self.shows_panorama(),
        ) {
            let scale = HudBatch::new(size).scale;
            crosshair.debug_crosshair(at, view_pitch, view_yaw, scale);
        }
        let debug_lines = horizon_client::gizmos::camera_relative(&lines, at, view);
        let debug_lines_on_top = horizon_client::gizmos::camera_relative(&on_top, at, view);
        let debug_crosshair = horizon_client::gizmos::camera_relative(&crosshair.on_top, at, view);
        // LevelExtractor.extractBlockOutline: model translucency controls draw order.
        let mut outline_after_terrain = false;
        // GameRenderer.shouldRenderBlockOutline: not with the HUD hidden (F1), and only for a
        // player who may build. Adventure players only get one with a CAN_BREAK or
        // CAN_PLACE_ON item matching the block, and spectators with a menu block; Horizon
        // does not open menus for spectators, so those never show it. An open
        // screen does not hide it: vanilla checks the HUD and the game mode, not the screen.
        let adventure = self.mode == horizon_client::command::args::GameMode::Adventure;
        let outline = if !self.ui.hide_gui && (self.may_build() || adventure) {
            self.pick()
                .filter(|h| {
                    self.may_build()
                        || !self.block_action_restricted(h.pos)
                        || self.can_place_on_in_adventure(h.pos)
                })
                .filter(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let mut canceled = false;
                    self.native_block_highlight(h.pos, state, alpha as f32, &mut canceled);
                    !canceled
                })
                .map(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let f = self.res.facts().get(state);
                    outline_after_terrain = self
                        .pack
                        .render
                        .model_catalog
                        .collect(state, BlockPos::new(h.pos[0], h.pos[1], h.pos[2]))
                        .is_ok_and(|models| {
                            models
                                .iter()
                                .any(|(_, model)| model.faces.iter().any(|f| f.translucent))
                        });
                    let o = if f.outline_offset {
                        self.pack
                            .context
                            .tables
                            .offset(state, h.pos[0], h.pos[2])
                            .unwrap_or([0.0; 3])
                    } else {
                        [0.0; 3]
                    };
                    let p = h.pos.map(f64::from);
                    if let Some(b) = self.context_outline(h.pos, state) {
                        return horizon_client::blocks::cube_edges()
                            .iter()
                            .map(|e| {
                                std::array::from_fn(|i| {
                                    let k = i % 3;
                                    p[k] + b[k] + e[i] * (b[k + 3] - b[k])
                                })
                            })
                            .collect();
                    }
                    f.outline_edges
                        .iter()
                        .map(|e| std::array::from_fn(|i| p[i % 3] + o[i % 3] + e[i]))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut hud = horizon_client::spans::time("hud", || self.hud(size));
        self.flush_ime(&mut hud);
        let title = self.shows_panorama();
        let (yaw, pitch) = if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (
                self.panorama_rotation.update(
                    self.started.elapsed().as_secs_f32(),
                    self.settings.file.double("panoramaScrollSpeed") as f32,
                ),
                10.0,
            )
        } else {
            (view_yaw, view_pitch)
        };
        let fov_degrees = if title {
            85.0
        } else {
            self.fov_degrees(camera_alpha as f32, camera.to_array())
        };
        self.locator_camera =
            (!title).then_some((camera.to_array(), view_yaw, view_pitch, fov_degrees));
        let bob = if title {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(camera_alpha as f32)
        };
        let spin = if title {
            glam::Mat4::IDENTITY
        } else {
            self.spinning_matrix(camera_alpha as f32)
        };
        let requested_post_effects = self.requested_post_effects();
        let terrain_distance = self.terrain_distance();
        let distance = (terrain_distance * 16) as f32;
        let depth_far =
            horizon_client::camera::depth_far(terrain_distance, self.settings.cloud_range);
        self.view_frustum = self
            .renderer
            .as_ref()
            .map(|r| r.camera_frustum(fov_degrees, yaw, pitch, bob * spin, depth_far));
        // Keep the buffer local while builders need mutable access to the game.
        let mut entities = std::mem::take(&mut self.entity_vertices);
        entities.clear();
        self.see_through_vertices.clear();
        self.glowing_vertices.get_mut().clear();
        self.shadow_only_vertices.get_mut().clear();
        if !title {
            entities = horizon_client::spans::time("entity_geometry", || {
                self.entity_geometry(camera, alpha, entities)
            });
        }
        let bolts = if title {
            Vec::new()
        } else {
            self.bolt_instances(camera)
        };
        let mut additive = Vec::new();
        if !title && self.dim == Dim::End {
            let mut dragon = Vec::new();
            self.dragon_geometry(camera, alpha, &mut dragon, &mut additive);
            self.dragon_fireball_geometry(camera, alpha, &mut dragon);
            entities.extend(dragon);
        }
        let (celestial, stars) = if title {
            (Vec::new(), None)
        } else if self.dim == Dim::Overworld
            && self.potions.effects.get("darkness").is_none()
            && self.potions.effects.get("blindness").is_none()
        {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky, rain)
        } else if self.dim == Dim::End && self.settings.horizon.sky {
            (self.end_flash_geometry(), None)
        } else {
            (Vec::new(), None)
        };
        let mut hand = if title || self.scoping() {
            Vec::new()
        } else {
            self.hand_geometry(camera_alpha)
        };
        // GameRenderer.render3dHud: the screen effects after the hand, in
        // its pass (the hand pass draws with `bob * view`).
        if !title {
            let view = bob * horizon_client::camera::view_matrix(yaw, pitch);
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            self.item_activation_geometry(camera_alpha as f32, view, aspect, &mut hand);
        }
        let screen = if title {
            Vec::new()
        } else {
            let scale = self
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:scale").ok())
                .unwrap_or(1.0) as f32;
            self.screen_quads(alpha as f32, size, scale)
        };
        if !title {
            self.end_geometry(camera, &mut entities);
        }
        let weather = if !title && self.dim == Dim::Overworld && self.settings.horizon.weather {
            horizon_client::spans::time("weather", || self.weather_geometry(camera, alpha))
        } else {
            Vec::new()
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        let see_through = std::mem::take(&mut self.see_through_vertices);
        if title {
            self.dynamic_lights.clear();
        } else {
            self.gather_dynamic_lights(camera, alpha);
        }
        let pack_inputs = if title {
            Default::default()
        } else {
            self.shader_pack_inputs(camera, alpha as f32)
        };
        // Map textures and font glyphs changed while building this frame's
        // geometry (game/maps.rs, world text) upload with this batch; draining
        // map_patches any earlier leaves them a frame behind.
        hud.patches.append(&mut self.map_patches);
        let entity_spans = std::mem::take(self.entity_spans.get_mut());
        let glowing = std::mem::take(self.glowing_vertices.get_mut());
        let shadow_only = std::mem::take(self.shadow_only_vertices.get_mut());
        let frame = Frame {
            weather: &weather,
            weather_time: horizon_client::weather::Time::new(self.world.game_time, alpha as f32),
            nether_lighting: self.dim == Dim::Nether,
            camera,
            yaw,
            pitch,
            fov_degrees,
            eye: eye.filter(|_| !title).map_or(camera, |e| e.0),
            eye_rotation: eye.filter(|_| !title).map_or([yaw, pitch], |e| e.1),
            first_person: eye.is_none() || title || !self.view.detached,
            bob,
            spin,
            sky,
            shadow_scene_revision: (self.world.revision, self.world.chunks.membership_revision()),
            shadow_cache: self.settings.horizon.shadow_cache,
            distance,
            detail_distance: (self.streaming.radius * 16) as f32,
            depth_far,
            underwater,
            water_plane: if title
                || underwater
                || !self
                    .renderer
                    .as_ref()
                    .is_some_and(|r| r.wants_water_plane())
            {
                None
            } else {
                self.water_plane(camera)
            },
            title,
            sky_blocked: self.potions.effects.get("blindness").is_some()
                || self.potions.effects.get("darkness").is_some()
                || matches!(
                    horizon_client::fog::fog_type(
                        &self.world,
                        self.res.facts(),
                        [camera.x, camera.y, camera.z],
                        self.forward(),
                    ),
                    horizon_client::fog::FogType::Lava | horizon_client::fog::FogType::PowderSnow
                ),
            skybox: self.dim == Dim::Overworld && self.settings.horizon.sky,
            end_sky: self.dim == Dim::End && self.settings.horizon.sky,
            end_sky_sprite: self
                .gui
                .atlas
                .sprites
                .get("environment/end_sky")
                .map(|r| [r.u0, r.v0, r.u1, r.v1]),
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            cave_culling,
            water_fog,
            fog: Some(self.frame_fog(camera, &sky, biome, water_fog)),
            dark_disc: self.dark_disc(camera),
            outline,
            outline_after_terrain,
            outline_high_contrast: self.settings.file.bool("highContrastBlockOutline"),
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            debug_crosshair: &debug_crosshair,
            hud: &hud,
            glint_alpha: self.glint_frame.alpha,
            glint_ticks: self.glint_frame.scroll_ticks(),
            clouds: (!title
                && self.dim == Dim::Overworld
                && horizon_client::clouds::visible(&self.settings.clouds, sky.cloud_color[3]))
            .then(|| self.gui.atlas.sprites.get("horizon/white"))
            .flatten()
            .map(|white| horizon_client::clouds::Input {
                texture: &self.pack.clouds,
                uv: [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0],
                camera,
                time: self.world.game_time,
                partial_tick: alpha,
                range: self.settings.cloud_range,
                fast: self.settings.clouds == "fast",
                color: sky.cloud_color,
            }),
            entities: &entities,
            entity_spans: &entity_spans,
            glowing: &glowing,
            dynamic_lights: &self.dynamic_lights,
            colored_lights: &colored_lights,
            emissive_layers: &emissive_layers,
            lightning: horizon_client::gfx::lightning::Input {
                bolts: &bolts,
                uv: horizon_client::gfx::lightning::white_uv(&self.gui.atlas.font.white()),
            },
            additive: &additive,
            see_through: &see_through,
            shadow_only: &shadow_only,
            celestial: if self.settings.horizon.sky || title {
                &celestial
            } else {
                &[]
            },
            stars: stars.filter(|_| self.settings.horizon.sky || title),
            hand: &hand,
            hand_fov_degrees: self.hud_fov_degrees(camera_alpha as f32, camera.to_array()),
            screen: &screen,
            pack: pack_inputs,
            border,
            game_time: self.world.game_time as f32,
            shader_game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
            crumbling_sprite: self
                .gui
                .atlas
                .sprites
                .get("block/destroy_stage_0")
                .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0]),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.gui
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.ticks,
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(alpha as f32),
            block_flicker: self.fog.block_light_flicker,
            boss_darken: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            menu_blur: self.menu_blur(),
            post_effects: horizon_client::post_chain::Request {
                effects: &requested_post_effects,
                root: &self.options.resources,
                menu_blur_radius: self.settings.menu_blur as f32,
                // GlobalSettingsUniform.update.
                globals: horizon_client::post_chain::Globals {
                    camera_block_pos: [
                        camera.x.floor() as i32,
                        camera.y.floor() as i32,
                        camera.z.floor() as i32,
                    ],
                    camera_offset: [
                        (camera.x.floor() - camera.x) as f32,
                        (camera.y.floor() - camera.y) as f32,
                        (camera.z.floor() - camera.z) as f32,
                    ],
                    game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
                    ..Default::default()
                },
            },
            chunk_fade: self.settings.chunk_fade,
        };
        let result = match self.renderer.as_mut() {
            Some(renderer) => {
                renderer.set_pass_timing(self.debug);
                horizon_client::spans::time("renderer", || renderer.render(&frame))
            }
            None => Err("renderer".into()),
        };
        // Restore storage even when rendering fails (for example, a lost surface).
        self.entity_vertices = entities;
        *self.entity_spans.get_mut() = entity_spans;
        *self.glowing_vertices.get_mut() = glowing;
        *self.shadow_only_vertices.get_mut() = shadow_only;
        self.see_through_vertices = see_through;
        result
    }
    /// The frame shows the title's panorama, not a level: the title screen, or a
    /// LevelLoadingScreen (Reason.OTHER), which draws it too.
    fn shows_panorama(&self) -> bool {
        self.title.is_some()
            || self.loading_reason() == Some(horizon_client::level_loading::Reason::Other)
            || self.remote_loading()
    }
    /// The LevelLoadingScreen shown while the player waits for its chunks:
    /// opening a world, or after a portal or a respawn into another dimension.
    fn loading_reason(&self) -> Option<horizon_client::level_loading::Reason> {
        use horizon_client::level_loading::Reason;
        if self.title.is_some() || self.remote.is_some() || self.player.is_some() {
            return None;
        }
        Some(
            if self.travelling.is_none() || self.respawn_travel.is_some() {
                Reason::Other
            } else if self.end_travel {
                Reason::EndPortal
            } else {
                Reason::NetherPortal
            },
        )
    }
    /// `LevelLoadTracker.serverProgress`: the player's 7x7 chunks loaded.
    fn level_load_progress(&self) -> f32 {
        let (cx, cz) = self.streaming.center;
        let ready = (-3..=3)
            .flat_map(|dz| (-3..=3).map(move |dx| (cx + dx, cz + dz)))
            .filter(|&(x, z)| self.streaming.is_lit(x, z))
            .count();
        horizon_client::level_loading::progress(ready)
    }
    /// `LevelLoadingScreen.extractBackground`.
    pub(super) fn draw_level_loading_background(
        &self,
        batch: &mut HudBatch,
        reason: horizon_client::level_loading::Reason,
    ) {
        use horizon_client::level_loading::Reason;
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        match reason {
            // The panorama and its blur are drawn by the renderer.
            Reason::Other => {
                horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false)
            }
            Reason::NetherPortal => {
                let overlay = hud_overlays::portal_overlay(w as i32, h as i32, 1.0);
                hud_layout::paint(batch, atlas, &[overlay]);
            }
            Reason::EndPortal => batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255)),
        }
    }
    /// `LevelLoadingScreen.extractBackground` and `extractRenderState`.
    fn draw_level_loading(
        &self,
        batch: &mut HudBatch,
        reason: horizon_client::level_loading::Reason,
    ) {
        use horizon_client::level_loading::{self, STATUS_RADIUS};
        // The level's progress, as the bar shows it (`hasProgress`).
        horizon_client::widgets::begin_screen(|| {
            screen_info::level_loading(Some(self.level_load_progress()), &screen_info::tr)
        });
        self.draw_level_loading_background(batch, reason);
        let atlas = &self.gui.atlas;
        let (cx, cz) = self.streaming.center;
        let status = |x: i32, z: i32| {
            self.streaming.latest_status(
                &self.world,
                cx + x - STATUS_RADIUS,
                cz + z - STATUS_RADIUS,
            )
        };
        level_loading::draw(batch, atlas, Some(&status), Some(self.loading_progress));
    }
    /// `GameRenderer.update`'s `requestedPostEffects`: `end_of_frame` (a resource
    /// pack's own chain), then the server's `LocalPlayer.postEffects`. Only the
    /// world is drawn through them.
    fn requested_post_effects(&self) -> Vec<String> {
        if self.title.is_some() || self.player.is_none() {
            return Vec::new();
        }
        std::iter::once("minecraft:end_of_frame".to_owned())
            .chain(self.post_effects.iter().cloned())
            .chain(self.spectated_post_effect.map(str::to_owned))
            .collect()
    }
    /// `Screen.extractBlurredBackground` over the panorama or the world and
    /// HUD, when `menuBackgroundBlurriness` is at least 1.
    fn menu_blur(&self) -> Option<f32> {
        let loading = self.loading_reason() == Some(horizon_client::level_loading::Reason::Other)
            || self.remote_loading();
        let blurred = loading
            || self.title.as_ref().is_some_and(|t| {
                matches!(self.screen, Screen::Options | Screen::Win)
                    || !matches!(t.mode, horizon_client::title::Mode::Main)
            })
            || (self.title.is_none()
                && self.player.is_some()
                && (self.screen.blurs_background(self.pause_menu)
                    || self.dialog_open()
                    || (self.screen == Screen::Win && self.win_screen_pauses())));
        let radius = self.settings.menu_blur as f32;
        (blurred && radius >= 1.0).then_some(radius)
    }
    /// `LevelExtractor` / `LevelRenderer`: the world border wall this frame.
    fn border_draw(
        &mut self,
        camera: [f64; 3],
        partial: f32,
    ) -> Option<horizon_client::gfx::BorderDraw> {
        use horizon_client::border_render::{self, BorderRenderer};
        let distance = f64::from(self.streaming.radius * 16);
        border_render::extract(
            &self.border,
            partial,
            camera,
            distance,
            &mut self.border_state,
        );
        // Camera.depthFar, as the renderer's far plane.
        let depth_far =
            horizon_client::camera::depth_far(self.terrain_distance(), self.settings.cloud_range);
        if !self
            .border_renderer
            .prepare(&self.border_state, camera, distance, depth_far)
        {
            return None;
        }
        let millis = self.started.elapsed().as_millis() as i64;
        Some(horizon_client::gfx::BorderDraw {
            vertices: self.border_renderer.vertices,
            rebuilt: self.border_renderer.take_rebuilt(),
            transforms: self
                .border_renderer
                .transforms(&self.border_state, camera, millis),
            sides: BorderRenderer::sides(&self.border_state, camera, distance),
        })
    }
    /// A key press while a world is open (keyboard or control socket).
    fn key_pressed(&mut self, key: KeyCode) -> Result<()> {
        if self.sleeping.is_some()
            && self.screen == Screen::Playing
            && !matches!(
                key,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F11 | KeyCode::KeyT | KeyCode::Slash
            )
        {
            if matches!(key, KeyCode::Escape | KeyCode::ShiftLeft) {
                self.wake_up();
            }
            return Ok(());
        }
        if self.sort_key_press(key) || self.hdr_key_press(key) {
            return Ok(());
        }
        if self.remote.is_some() && self.remote_key(key)? {
            return Ok(());
        }
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Options => self.options_key(KeyCode::Escape, None)?,
                Screen::Playing => {
                    self.pause_focus = None;
                    self.screen = Screen::Paused;
                    self.pause_menu = true;
                    self.capture(false);
                }
                Screen::Creative => {
                    self.carried = None;
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                Screen::Paused | Screen::Inventory => {
                    self.close_container();
                    self.pause_focus = None;
                    self.screen = Screen::Playing;
                    self.pause_menu = true;
                    self.capture(true);
                }
                Screen::GameModeSwitcher => self.close_gamemode_switcher(false),
                Screen::Dead => {
                    if self.ui.death_confirm.is_some() {
                        self.death_confirm_answer(false)?;
                    }
                }
                Screen::Stats | Screen::Friends => self.screen = Screen::Paused,
                Screen::ServerLinks => self.screen = Screen::Paused,
                // ConfirmScreen.keyPressed: Escape answers no; both return to
                // the Server Links screen they came from.
                Screen::ConfirmLink => self.screen = self.confirm_link_back(),
                Screen::DialogWarning => self.screen = Screen::ServerLinks,
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.world_options_escape(),
                Screen::CommandBlock => self.close_command_block(),
                Screen::BlockEditor => self.close_block_editor(),
                Screen::Chat => self.close_chat(chat::ChatExit::Intentional),
                Screen::Win => self.close_win_screen()?,
                Screen::DebugOptions => self.close_debug_options(),
            },
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
            | KeyCode::KeyF
            | KeyCode::KeyQ
                if (self.screen == Screen::Inventory
                    || (self.screen == Screen::Creative && !self.creative_screen.tab().search))
                    && self.inventory_hotkey(key) => {}
            // The search tab's number keys (scripted presses; the window's
            // keys reach search_tab_hotbar_key before the search box).
            _ if self.screen == Screen::Creative
                && self.creative_screen.tab().search
                && self.search_tab_hotbar_key(key) => {}
            KeyCode::KeyT if self.screen == Screen::Inventory => self.recipe_focus_search(),
            KeyCode::KeyT if self.screen == Screen::Playing => self.open_chat_method(""),
            // key.advancements.
            KeyCode::KeyL if self.screen == Screen::Playing => {
                self.open_advancements(false);
                // Opened from the keybinds, like the inventory: vanilla releases
                // an active use in the same tick.
                self.release_use_for_keybind_screen();
            }
            KeyCode::KeyL if self.screen == Screen::Advancements => self.close_advancements(),
            // key.quickActions: only open if a datapack provides dialog actions.
            KeyCode::KeyG if self.screen == Screen::Playing => {
                if self.has_quick_actions() {
                    self.open_quick_actions();
                }
            }
            KeyCode::Slash if self.screen == Screen::Playing => self.open_chat_method("/"),
            // key.toggleGui is a KeyMapping: it clicks only with no screen open.
            KeyCode::F1 if self.screen == Screen::Playing => self.toggle_hide_gui(),
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::F5 if self.screen == Screen::Playing => self.cycle_camera(),
            KeyCode::KeyE
                if self.screen == Screen::Creative && self.creative_screen.tab().search => {}
            KeyCode::KeyE => match self.screen {
                Screen::Playing if self.spectator() => {}
                Screen::Playing => {
                    // key.inventory while riding a tamed mount opens its inventory.
                    if !self.open_ridden_mount_menu() {
                        if self.tutorial.open_inventory() {
                            self.persist_tutorial_step();
                        }
                        if self.creative {
                            self.screen = Screen::Creative;
                            self.refresh_creative_tabs();
                            self.creative_screen.reopen();
                            self.refresh_saved_hotbars();
                        } else {
                            self.screen = Screen::Inventory;
                        }
                        self.capture(false);
                    }
                }
                Screen::Creative => {
                    if let Some(c) = self.carried.take() {
                        let _ = self.give(c);
                    }
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                Screen::Inventory => {
                    self.close_container();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                _ => {}
            },
            // key.swapOffhand: on a server, ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND
            // (the server swaps the hands and sends both slots back; spectators can't).
            KeyCode::KeyF if self.screen == Screen::Playing && self.remote.is_some() => {
                if !self.spectator()
                    && let Some(r) = self.remote.as_mut()
                {
                    r.pending_keys.push(key);
                }
            }
            KeyCode::KeyF if self.screen == Screen::Playing => {
                // handlePlayerAction SWAP_ITEM_WITH_OFFHAND: setItemInHand for
                // the off hand, then the main hand.
                std::mem::swap(&mut self.offhand, &mut self.inventory[self.selected]);
                self.hand_stack_replaced(self.selected);
                self.hand_stack_replaced(shield::OFFHAND_SLOT);
                let (offhand, main) = (self.offhand.clone(), self.inventory[self.selected].clone());
                self.player_on_equip("offhand", main.as_ref(), offhand.as_ref());
                self.player_on_equip("mainhand", offhand.as_ref(), main.as_ref());
                self.using = None;
            }
            KeyCode::KeyQ if self.screen == Screen::Playing => {
                let all = drop_whole_stack(self.modifiers);
                self.drop_held(all);
            }
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
                if self.screen == Screen::Playing =>
            {
                let slot = match key {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    KeyCode::Digit6 => 5,
                    KeyCode::Digit7 => 6,
                    KeyCode::Digit8 => 7,
                    _ => 8,
                };
                // Minecraft.handleKeybinds: a spectator's number keys drive
                // the spectator menu, not the hotbar; a creative player
                // holding the load or save toolbar key loads or saves a
                // toolbar instead (load first when both are held).
                let load = self.mapped_key_down("key.loadToolbarActivator");
                if self.spectator() {
                    self.spectator_hotbar_key(slot as i32);
                } else if self.creative
                    && (load || self.mapped_key_down("key.saveToolbarActivator"))
                {
                    self.hotbar_load_or_save(slot, load);
                } else {
                    self.selected = slot;
                }
            }
            _ if self.screen == Screen::Paused && self.pause_menu => {
                self.pause_key(key)?;
            }
            _ if self.screen == Screen::Dead => {
                self.death_key(key)?;
            }
            _ => {}
        }
        Ok(())
    }
    /// A mouse button while playing (window or control socket).
    fn play_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        if self.sleeping.is_some() {
            return Ok(());
        }
        // key.spectatorHotbar (the middle button) opens the spectator menu
        // and uses its selected item.
        if button == MouseButton::Middle && self.spectator() {
            if pressed {
                self.spectator_action_key();
            }
            return Ok(());
        }
        if self.remote.is_some() {
            return self.remote_mouse(button, pressed);
        }
        match button {
            MouseButton::Left => {
                // KeyMapping.click: Minecraft.tick starts the attack.
                if pressed {
                    self.attack_clicks += 1;
                }
                self.toggles
                    .button(2, pressed, false, self.settings.toggle_attack);
                self.left = self.toggles.is_down(2);
            }
            MouseButton::Right => {
                self.toggles
                    .button(3, pressed, false, self.settings.toggle_use);
                self.right = self.toggles.is_down(3);
                // handleKeybinds consumes use clicks without acting while an
                // item is in use. Minecraft.tick runs them after the attack
                // clicks, so a place and a break in one tick replace the block.
                if pressed && self.using.is_none() {
                    self.use_clicks += 1;
                }
            }
            MouseButton::Middle if pressed => {
                // Minecraft.pickBlockOrEntity -> the server's handlePickItemFromBlock:
                // the block's clone item, with its block entity's data on Ctrl in
                // creative, picked from the inventory (`pick_item`).
                if let Some(entity) = self.pick_entity_hit() {
                    // handlePickItemFromEntity: the entity's pick result.
                    if let Some(stack) = entity.and_then(|id| self.entity_pick_result(id)) {
                        let infinite = self.creative;
                        horizon_client::pick_item::pick(
                            &mut self.inventory[..36],
                            &mut self.selected,
                            &stack,
                            infinite,
                        );
                    }
                } else if let Some(hit) = self.pick() {
                    let include_data = self.creative && self.has_control_down();
                    if let Some(stack) = self.picked_stack(hit.pos, include_data) {
                        let infinite = self.creative;
                        horizon_client::pick_item::pick(
                            &mut self.inventory[..36],
                            &mut self.selected,
                            &stack,
                            infinite,
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// LOD Distance and Far Detail Reduced as the player chose them.
    fn far_terrain_choice(&self) -> (u32, bool) {
        let g = &self.settings.horizon;
        let reduced = g.far_detail == horizon_client::graphics::FarDetail::Reduced;
        (g.lod_distance, reduced)
    }
    /// LOD Distance as far terrain uses it (`graphics::lod_distance_capped`).
    fn lod_distance(&self) -> u32 {
        let (distance, _) = self.far_terrain_choice();
        match &self.renderer {
            Some(renderer) => {
                horizon_client::graphics::lod_distance_capped(distance, &renderer.info)
            }
            None => distance.min(horizon_client::lod::MAX_CHUNKS),
        }
    }
    /// Only enabled LOD worlds extend the renderer's range, never simulation.
    fn terrain_distance(&self) -> i32 {
        if self.lod.is_some() {
            self.streaming.radius.max(self.lod_distance() as i32)
        } else {
            self.streaming.radius
        }
    }
    fn clear_lod(&mut self) {
        self.lod = None;
        self.lod_source_attempted = false;
        self.lod_observe.clear();
        self.far_beacons.clear();
        self.lod_covered_at = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_lods();
        }
    }
    fn prepare_lod(&mut self) {
        let enabled = self.far_terrain_choice().0 > 0
            && self.renderer.is_some()
            && self.player.is_some()
            && self.title.is_none()
            && (self.remote.is_none() || self.integrated.is_some());
        if !enabled {
            if self.lod.is_some() || self.lod_source_attempted {
                self.clear_lod();
            }
            return;
        }
        if self.lod_source_attempted {
            return;
        }
        self.lod_source_attempted = true;
        let dim = self.dim;
        let source = match &self.integrated {
            Some(server) => server.call(move |game| game.lod_source_in(dim)).flatten(),
            None => Some(self.streaming.lod_source()),
        };
        if let Some(source) = source {
            self.lod = Some(horizon_client::lod::Lod::new(
                source,
                self.pack.context.clone(),
            ));
            self.lod_observe.extend(self.world.chunks.keys().copied());
        }
    }
    fn update_world(&mut self) {
        if let Err(e) = self.poll_remote() {
            log::warn!("Horizon: {e}");
        }
        self.prepare_lod();
        if self.lod.is_some() {
            for &((x, _, z), _, _) in &self.world.light_edits {
                self.lod_observe.insert((x >> 4, z >> 4));
            }
        }
        if let Some(p) = &self.player {
            self.streaming.center = (
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            );
        }
        // The loading screen's chunk map fills ring by ring, as vanilla's.
        self.streaming.loading =
            self.options.execution_role.is_client() && self.loading_reason().is_some();
        self.tick_pregen();
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.options.headless {
            self.streaming.mesh = self.renderer.is_some();
        }
        let areas = self.streaming.extra.len();
        let waiting = std::mem::take(&mut self.spawn_wait);
        self.streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
        let (uploads, unloaded) =
            horizon_client::spans::time("streaming", || self.streaming.update(&mut self.world));
        self.streaming.extra.truncate(areas);
        self.spawn_fresh_chunks();
        // Before the unloaded chunks' block entities are written and dropped.
        self.native_chunks_changed(&unloaded);
        if let Some(lod) = &mut self.lod {
            self.lod_observe
                .extend(uploads.iter().map(|(key, _)| (key.0, key.2)));
            for chunk in &unloaded {
                self.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then a few resident ones (an edit copies a chunk the worker
            // still holds). The worker summarizes them.
            horizon_client::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.res.facts());
                }
                while lod.observing() < horizon_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.lod_observe.iter().next()
                {
                    self.lod_observe.remove(&key);
                    if let Some(chunk) = self.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.res.facts());
                    }
                }
            });
        }
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = horizon_client::spans::time("with_entities", || self.with_entities(&dirty));
        horizon_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            self.world
                .block_entities
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
            self.world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        if !modified.is_empty()
            && let Err(e) = horizon_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.pack.assets.registry, world_biomes!(self))
            })
        {
            log::warn!("Horizon: failed to save unloaded chunks: {e}");
        }
        let (far_distance, far_reduced) = self.far_terrain_choice();
        if let Some(renderer) = self.renderer.as_mut() {
            let g = &self.settings.horizon;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            // The meshes, once copied, are freed by a worker (`Streaming::free`).
            if !budgeted && self.pending_uploads.is_empty() && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                horizon_client::spans::time("upload", || {
                    for (key, mesh) in &uploads {
                        renderer.upload(*key, mesh);
                    }
                });
                self.streaming.free(uploads);
            } else {
                // Horizon's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let fresh: horizon_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|(k, _)| *k).collect();
                let chunks = &self.world.chunks;
                let loaded = |k: &(i32, i32, i32)| chunks.contains_key(&(k.0, k.2));
                let mut spent: Vec<_> = pending
                    .extract_if(.., |(k, _)| fresh.contains(k) || !loaded(k))
                    .collect();
                for upload in uploads {
                    if loaded(&upload.0) {
                        pending.push(upload);
                    } else {
                        spent.push(upload);
                    }
                }
                let centre = self.streaming.center;
                let far = |k: &(i32, i32, i32)| (k.0 - centre.0).pow(2) + (k.2 - centre.1).pow(2);
                pending.sort_unstable_by_key(|(k, _)| std::cmp::Reverse(far(k)));
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took
                // AUTO_UPLOAD_TIME of this frame (at least one goes): a burst of
                // finished meshes spreads over frames, nearest first.
                let auto = !budgeted;
                let started = Instant::now();
                while let Some((_, mesh)) = pending.last() {
                    if auto && count > 0 && started.elapsed() >= AUTO_UPLOAD_TIME {
                        break;
                    }
                    let size = ((mesh.solid.len() + mesh.translucent.len())
                        * std::mem::size_of::<horizon_client::mesher::TerrainVertex>())
                        as u64;
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let (key, mesh) = pending.pop().expect("checked above");
                    renderer.upload(key, &mesh);
                    spent.push((key, mesh));
                    count += 1;
                    bytes += size;
                }
                self.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            if let (Some(lod), Some(player)) = (&mut self.lod, &self.player) {
                let lod_distance =
                    horizon_client::graphics::lod_distance_capped(far_distance, &renderer.info);
                lod.set_near_loading(self.streaming.busy());
                lod.set_reduced_detail(far_reduced);
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.streaming.center,
                    self.streaming.radius,
                );
                let covered = (self.lod_covered_at != Some(at)).then(|| {
                    self.lod_covered_at = Some(at);
                    horizon_client::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                horizon_client::streaming::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                let meshes = horizon_client::spans::time("lod_update", || {
                    lod.update(
                        [player.position.x, player.position.y, player.position.z],
                        self.streaming.radius,
                        lod_distance,
                        covered,
                    )
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                horizon_client::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
            }
        }
    }
}
impl ApplicationHandler for Game<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen = self.settings.fullscreen || self.options.fullscreen;
        let size = self.options.window_size.map(f64::from);
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon());
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "horizon", "horizon",
        );
        // `--offscreen` renders into a texture: its window would only show white.
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.offscreen),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        horizon_client::telemetry::note_display(&window);
        match self.new_renderer(window.clone()) {
            Ok(r) => {
                self.renderer = Some(r);
                self.world_renderer_ready = true;
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.ime_allowed = false;
        self.ime_area = None;
        self.apply_settings();
        self.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, id, event);
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // The real mouse never steers an offscreen game: its pointer is not grabbed.
        if self.pending_save.is_none()
            && self.captured
            && self.focused
            && !self.options.offscreen
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            // MouseHandler.onMove with the pointer grabbed; applied once per frame.
            self.mouse.on_move(0.0, 0.0, x, y, true, true);
            self.last_input = Instant::now();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.idle(event_loop);
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shut_down();
    }
}

/// winit's events for a window's game, with any loop: a headless client (#4951) has no
/// winit loop and calls these from its own.
impl Game<'_> {
    /// winit's `window_event`. The control socket and the pad menu call it too, with the
    /// input they make up.
    pub(super) fn handle_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        mut event: WindowEvent,
    ) {
        // A headless game has no window: its events (the frame, the control socket's input)
        // carry `WindowId::dummy()`.
        let ours = match &self.window {
            Some(w) => w.id() == id,
            None => self.options.headless,
        };
        if !ours {
            return;
        }
        follow_keymap(&mut event);
        // MacosUtil.setCtrlClickEmulatesRightClick: Control + left click is a right click.
        if let WindowEvent::MouseInput { state, button, .. } = &mut event
            && *button == MouseButton::Left
            && self.right_click_emulation.is_right_click(
                *state == ElementState::Pressed,
                self.modifiers.control_key(),
                cfg!(target_os = "macos") && self.settings.ctrl_click_emulates_right_click,
            )
        {
            *button = MouseButton::Right;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.physical_keys.insert(key);
                horizon_client::keybinds::learn_layout(event);
            } else {
                self.physical_keys.remove(&key);
            }
        }
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            if *state == ElementState::Pressed {
                self.physical_buttons.insert(*button);
            } else {
                self.physical_buttons.remove(button);
            }
        }
        if let WindowEvent::Focused(false) = &event {
            self.physical_keys.clear();
            self.physical_buttons.clear();
        }
        if let WindowEvent::ModifiersChanged(m) = &event {
            self.modifiers = m.state();
            horizon_client_input::modifiers::reconcile(
                self.modifiers,
                &mut self.physical_keys,
                &self.settings.keys,
                &mut self.toggles,
                [self.settings.toggle_crouch, self.settings.toggle_sprint],
                [
                    self.pad
                        .holds_action(&self.settings.controller, "key.sneak"),
                    self.pad
                        .holds_action(&self.settings.controller, "key.sprint"),
                ],
            );
            self.sync_toggle(SNEAK);
            self.sync_toggle(SPRINT);
        }
        // `PackSelectionScreen.onFilesDrop` on the Create New World screen.
        if let WindowEvent::DroppedFile(path) = &event
            && let Some(title) = &mut self.title
        {
            title.drop_file(path.clone());
        }
        // Fullscreen is a window shortcut, before screens consume keyboard input.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && horizon_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
            && !(self.screen == Screen::Options && self.options_waiting_for_key())
        {
            self.last_input = Instant::now();
            if !event.repeat {
                self.toggle_fullscreen();
            }
            return;
        }
        // A screenshot is a window shortcut too: vanilla takes it before a
        // screen or a text box consumes the key (`handleGlobalKeyPress`), so
        // it works with chat, a sign or a book open.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
        {
            self.last_input = Instant::now();
            if self.screenshot_shortcut(key, event.repeat) {
                return;
            }
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && let PhysicalKey::Code(key) = key_event.physical_key
        {
            use horizon_client::debug_keys::KeyAction;
            let action = match (key_event.state, key_event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            if self.debug_key(key, action) {
                return;
            }
        }
        // KeyboardHandler.keyPress: the narrator hotkey, on every screen and
        // in game, before the screen takes the key.
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            self.narrator_hotkey(event);
        }
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        // The real mouse takes the pointer back from the keyboard's focus.
        self.key_focus_mouse(event_loop, &event);
        // The screen's narration hears of the input before the screen takes
        // it; the pointer events the keyboard's focus sends aren't the mouse.
        if !self.key_focus_quiet() {
            narrator::narrate_input(&mut self.narration, &event, self.cursor);
        }
        // FileFixerProgressScreen: Cancel is the only way out (`shouldCloseOnEsc`
        // is false), so it takes the pointer and Escape does nothing.
        if let Some(loader) = &self.world_loader
            && loader.upgrade.is_file_fixing()
            && !loader.cancelled
        {
            match &event {
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new([r.config.width, r.config.height]);
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if !loader.upgrade.is_canceled() => {
                    let (w, h) = self.gui_size();
                    let (x, y) = horizon_client::level_loading::file_fixer_cancel(w, h);
                    if horizon_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
                        self.audio.play_button_click();
                        loader.upgrade.set_canceled();
                    }
                }
                _ => {}
            }
            if matches!(&event, WindowEvent::KeyboardInput { .. }) {
                return;
            }
        }
        if (self.pending_entry.is_some() || self.world_loader.is_some())
            && matches!(&event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape))
        {
            self.pending_entry = None;
            if let Some(loader) = self.world_loader.take() {
                self.world_loader = Some(if loader.cancelled {
                    loader
                } else {
                    loader.cancel()
                });
            }
            self.perf.cancel_world_opening();
            return;
        }
        if (self.pending_entry.is_some()
            || self.world_loader.is_some()
            || self.integrated_starting.is_some())
            && !matches!(
                &event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        // The pack reload's progress overlay takes no input
        // (`LoadingOverlay`): `spawn_pack_reload` released what was held.
        if self.pack_reload.is_some()
            && !matches!(
                &event,
                WindowEvent::CloseRequested
                    | WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        if self.pending_save.is_some() {
            match event {
                WindowEvent::CloseRequested => {
                    if !self.settings.quit_shortcuts
                        && horizon_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        )
                    {
                        return;
                    }
                    self.pending_save.as_mut().unwrap().quit = true;
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused {
                        self.capture(false);
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = self.render_closing_save() {
                        self.error = Some(error);
                        self.exit_on_error(event_loop);
                    }
                }
                _ => {}
            }
            return;
        }
        // ScreenEvent.KeyPressed.Pre: the open screen hears of the key before
        // it acts on it, and a cancelled press goes no further — no focus
        // move, no Escape-close, no inventory or hotbar key, no typing by the
        // key itself, no `native_key`. It fires after the window's own
        // shortcuts (fullscreen, screenshot, the debug keys, the narrator) so
        // those keep working with a mod that cancels; the key's release still
        // comes through.
        //
        // A key's characters are a separate callback of GLFW's, and it comes
        // after the key callback (`KeyboardHandler.charTyped`): they are taken
        // off the key here and dispatched once the screen has acted on it, on
        // the screen that is open then (`ScreenEvent.CharacterTyped.Pre`).
        // Options keeps its text on the key: its own key path routes a key's
        // text between its boxes and its key-bind capture, and no key that
        // carries text closes that screen or moves its focus.
        let mut typed: Option<String> = None;
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &mut event
            && key_event.state == ElementState::Pressed
        {
            if self.screen_hooks_open() && self.screen != Screen::Options {
                typed = key_event.text.take().map(|text| text.to_string());
            }
            let canceled = match key_event.physical_key {
                PhysicalKey::Code(key) => self.screen_key_pressed(key, key_event.repeat),
                PhysicalKey::Unidentified(_) => false,
            };
            if self.screen == Screen::Options && self.screen_hooks_open() {
                // The hook after the key's own, then the text the key path
                // types in place (its own routing keeps it).
                if let Some(text) = key_event.text.as_deref() {
                    let left = self.screen_text_typed(text);
                    if left.as_deref() != Some(text) {
                        key_event.text = left.map(Into::into);
                    }
                }
            }
            if canceled {
                // charTyped is a separate callback: the characters still
                // reach the text box, without the key's own effects. Options'
                // characters already passed the hook in place.
                if let Some(text) = typed.as_deref() {
                    self.screen_chars_typed(Some(text));
                } else if self.screen == Screen::Options
                    && let Some(text) = key_event.text.as_deref()
                    && self.types_text()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !text.is_empty() {
                        // Err only says no box of the screen takes it.
                        let _ = self.type_chars_into_screen(&text);
                    }
                }
                return;
            }
        }
        // Screen.keyPressed: Tab, the arrows and Enter move and press the
        // menu's focus, unless the focused widget or the screen keeps them.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = key_event.physical_key
            && self.key_focus_key(event_loop, key)
        {
            // The keyboard's focus took the key; its characters are a separate
            // callback and still come (`KeyboardHandler.charTyped`).
            self.screen_chars_typed(typed.as_deref());
            return;
        }
        let result: Result<()> = (|| {
            // ScreenEvent.MouseButtonPressed.Pre / MouseButtonReleased.Pre:
            // the open screen hears of the press before anything reads it, and
            // a cancelled one goes no further — no hotbar key, no slot, no
            // button, no sort, no drag. First in the event, so none of the
            // arms below (which hold the container hotkey and the sort and
            // header buttons too) see it. A click the pad or the keyboard's
            // focus made up is not the mouse: it fires nothing.
            if let WindowEvent::MouseInput { state, button, .. } = &event
                && !self.pad_synthetic_input()
                && let Some(screen) = self.screen_view_now()
            {
                let (button, modifiers) = (glfw_button(*button), glfw_modifiers(self.modifiers));
                let mut canceled = false;
                if *state == ElementState::Pressed {
                    self.native_screen_mouse_button_pressed_pre(
                        &screen,
                        button,
                        modifiers,
                        &mut canceled,
                    );
                } else {
                    self.native_screen_mouse_button_released_pre(
                        &screen,
                        button,
                        modifiers,
                        &mut canceled,
                    );
                }
                if canceled {
                    return Ok(());
                }
            }
            self.key_focus_mouse_press(&event);
            match event {
                WindowEvent::CloseRequested => {
                    if !self.settings.quit_shortcuts
                        && horizon_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        )
                    {
                        return Ok(());
                    }
                    self.begin_closing_save(true)?;
                }
                // Input method composition: the committed text types like a key's text.
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => self.ime_commit(&text),
                WindowEvent::Ime(winit::event::Ime::Preedit(text, cursor)) => {
                    self.ime_preedit(&text, cursor)
                }
                WindowEvent::Ime(winit::event::Ime::Disabled) => {
                    horizon_client::preedit::set_preedit(None)
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                    self.observe_fullscreen();
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    self.focus_lost = (!focused).then(Instant::now);
                    if !focused && self.control.is_none() {
                        self.keys.clear();
                        // Latched sneak/sprint live in `toggles`, not in the
                        // physical keys: put them back so a short alt-tab
                        // (no pause screen) does not drop them. Only a mapping
                        // whose toggle option is on is latched; a held one has
                        // no state to keep, and an unfocused window never gets
                        // its key release, so leaving it down would keep the
                        // player sneaking or sprinting until the key is tapped.
                        for (slot, needs_toggle) in [
                            self.settings.toggle_crouch,
                            self.settings.toggle_sprint,
                            self.settings.toggle_attack,
                            self.settings.toggle_use,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            self.toggles.0[slot].set_down(false, needs_toggle);
                        }
                        self.sync_toggle(0);
                        self.sync_toggle(1);
                        self.left = false;
                        self.right = false;
                        self.capture(false);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new([r.config.width, r.config.height]);
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                        if self.dialog_open() {
                            self.dialog_drag();
                        }
                        if self.screen == Screen::BlockEditor {
                            self.block_editor_drag();
                        }
                        if self.screen == Screen::Advancements {
                            self.advancements_drag();
                        }
                        if self.screen == Screen::Stats {
                            self.stats_drag();
                        }
                        if self.screen == Screen::DebugOptions {
                            self.debug_options_drag();
                        }
                        if self.screen == Screen::WorldOptions {
                            self.world_options_drag();
                        }
                        if self.screen == Screen::Creative {
                            let (_, oy) = Self::creative_origin(&b);
                            self.creative_screen.drag_scrollbar(self.cursor.1 - oy);
                        }
                        if self.screen == Screen::Inventory {
                            self.station_drag();
                            self.recipe_book_drag();
                        }
                        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
                            self.inventory_drag_move();
                        }
                        if let Some(title) = &mut self.title {
                            title.drag(self.cursor, (b.width, b.height));
                            title.drag_field(&self.gui.atlas, self.cursor.0);
                        }
                        if self.dragging && self.screen == Screen::Options {
                            let _ = b;
                            self.options_drag()?;
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (p.x / 30.0, p.y / 30.0),
                    };
                    // MouseHandler.onScroll: discrete scrolling and the wheel sensitivity.
                    let discrete = self.settings.discrete_scroll;
                    let wheel_sensitivity = f64::from(self.settings.mouse_wheel_sensitivity);
                    let scaled = |v: f64| {
                        (if discrete {
                            horizon_client::mouse::java_signum(v)
                        } else {
                            v
                        }) * wheel_sensitivity
                    };
                    let d = scaled(raw_y) as f32;
                    // ScreenEvent.MouseScrolled.Pre: the open screen hears of
                    // the wheel before anything scrolls, and a cancelled turn
                    // scrolls nothing — no bundle, no list, slider or
                    // scrollbar. The hotbar below reads the wheel only while
                    // playing, which the gate excludes. A wheel the pad or the
                    // keyboard's focus made up is not the mouse: it fires
                    // nothing.
                    if !self.pad_synthetic_input()
                        && let Some(screen) = self.screen_view_now()
                    {
                        let mut canceled = false;
                        self.native_screen_mouse_scrolled_pre(
                            &screen,
                            [scaled(raw_x), scaled(raw_y)],
                            glfw_modifiers(self.modifiers),
                            &mut canceled,
                        );
                        if canceled {
                            return Ok(());
                        }
                    }
                    if self.dialog_open() {
                        self.dialog_scroll(d);
                    } else if self.code_of_conduct.is_some() {
                        self.code_of_conduct_scroll(d);
                    } else if let Some(title) = &mut self.title
                        && self.screen != Screen::Win
                    {
                        let size = self
                            .window
                            .as_ref()
                            .map(|w| w.inner_size())
                            .unwrap_or_default();
                        let batch = HudBatch::new([size.width, size.height]);
                        title.scroll_worlds(d, self.cursor, (batch.width, batch.height));
                    }
                    // MouseHandler.onScroll: the hotbar (or the spectator
                    // menu) takes the wheel only when no screen is open; a
                    // book, lectern, sign or prompt drawn over gameplay is one.
                    if self.screen == Screen::Playing
                        && !self.screen_over_playing()
                        && self.sleeping.is_none()
                        && self.player.is_some()
                        && !self.native_wheel_event(d)
                        && let Some((wheel, (_, vertical_steps))) =
                            self.mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity)
                    {
                        if self.spectator() && self.spectator_gui.is_menu_active() {
                            self.spectator_scroll(-wheel);
                        } else if self.spectator() {
                            if let Some(player) = &mut self.player {
                                player.flying_speed = horizon_client::mouse::spectator_flying_speed(
                                    player.flying_speed,
                                    vertical_steps,
                                );
                            }
                        } else {
                            self.selected =
                                horizon_client::mouse::ScrollWheelHandler::next_selection(
                                    f64::from(wheel),
                                    self.selected as i32,
                                    9,
                                ) as usize;
                        }
                    }
                    if self.screen == Screen::Chat && d != 0.0 {
                        self.chat_scroll(d);
                    }
                    if self.screen == Screen::CommandBlock && d != 0.0 {
                        self.command_block_scroll(d);
                    }
                    if self.screen == Screen::Options && d != 0.0 {
                        self.options_scroll(d)?;
                    }
                    // AbstractContainerScreen.mouseScrolled: a bundle under the
                    // cursor takes the wheel before the screen's own scrolling.
                    let bundle = bundles::takes_bundle_wheel(self.screen)
                        && d != 0.0
                        && self.bundle_scroll(scaled(raw_x), scaled(raw_y));
                    if self.screen == Screen::Creative && d != 0.0 && !bundle {
                        self.creative_screen.wheel(f64::from(d), &self.pack.names);
                    }
                    if self.screen == Screen::Inventory
                        && d != 0.0
                        && !bundle
                        && !self.tweak_scroll(d, self.inventory_target().0)
                    {
                        self.station_scroll(d);
                    }
                    if self.screen == Screen::Stats && d != 0.0 {
                        self.stats_scroll(d);
                    }
                    if self.screen == Screen::WorldOptions && d != 0.0 {
                        self.world_options_scroll(d);
                    }
                    if self.screen == Screen::ServerLinks && d != 0.0 {
                        self.server_links_scroll(d);
                    }
                    if self.screen == Screen::DebugOptions && d != 0.0 {
                        self.debug_options_scroll(d);
                    }
                    if self.screen == Screen::Advancements {
                        self.advancements_scroll(scaled(raw_x) as f32, d);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.link_prompt.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.link_prompt_click();
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.code_of_conduct.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.code_of_conduct_click()?;
                    }
                }
                WindowEvent::MouseInput { state, button, .. }
                    if self.server_packs.prompt.is_some() =>
                {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.pack_prompt_click()?;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.dialog_open() => {
                    if button == MouseButton::Left {
                        if state == ElementState::Pressed {
                            self.dialog_click()?;
                        } else {
                            self.dialog_release();
                        }
                    }
                }
                // ConnectScreen's Cancel, under the prompts and dialogs above.
                WindowEvent::MouseInput { state, button, .. } if self.remote_loading() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.connect_screen_click()?;
                    }
                }
                WindowEvent::MouseInput { .. } if self.screen == Screen::Win => {}
                WindowEvent::MouseInput { state, button, .. } if self.screen == Screen::Options => {
                    if self.options_waiting_for_key() && state == ElementState::Pressed {
                        self.options_bind_mouse(button)?;
                    } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                        self.options_mouse(
                            state == ElementState::Pressed,
                            button == MouseButton::Right,
                        )?;
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } if self.title.is_some() && self.screen != Screen::Options => {
                    if let Some(title) = &mut self.title {
                        title.release();
                        title.release_field();
                    }
                }
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } if self.title.is_some() && state == ElementState::Pressed => {
                    let size = self
                        .renderer
                        .as_ref()
                        .map(|r| [r.config.width, r.config.height])
                        .unwrap_or([1, 1]);
                    let b = HudBatch::new(size);
                    let atlas = &self.gui.atlas;
                    let shift = self.has_shift_down();
                    let (action, sound) = self.title.as_mut().map_or((Action::None, false), |t| {
                        let clicked = t.click(self.cursor.0, self.cursor.1, (b.width, b.height));
                        t.press_field(atlas, self.cursor, (b.width, b.height), shift);
                        let rules = t.press_rules(atlas, self.cursor, (b.width, b.height));
                        (clicked.0, clicked.1 || rules)
                    });
                    self.save_title_options();
                    if sound {
                        self.audio.play_button_click();
                    }
                    self.title_action(event_loop, action)?;
                    self.reopen_data_packs(event_loop)?;
                }
                WindowEvent::MouseInput { .. } if self.title.is_some() => {}
                WindowEvent::MouseInput { state, button, .. } if self.book.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.book_click(size);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.sign_edit.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.sign_edit_click(size);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    if matches!(self.screen, Screen::Inventory | Screen::Creative)
                        && let Some(key) = self
                            .settings
                            .keys
                            .container_mouse_key(button, self.creative)
                    {
                        if pressed && self.carried.is_none() {
                            self.inventory_hotkey(key);
                        }
                        // Neither edge of a bound button is an ordinary slot click.
                        return Ok(());
                    }
                    if pressed && (self.sort_mouse_press(button) || self.hdr_mouse_press(button)) {
                        return Ok(());
                    }
                    match self.screen {
                        Screen::Playing => {
                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }
                            self.extras_mouse(button, pressed);
                            if pressed
                                && self.settings.keys.get("key.smoothCamera")
                                    == horizon_client::keybinds::mouse_name(button)
                            {
                                self.smooth_camera = !self.smooth_camera;
                            }
                            for mapped in self.settings.keys.translate_mouse_all(button) {
                                match mapped {
                                    Err(b) => self.play_mouse(b, pressed)?,
                                    Ok(k) if pressed => {
                                        self.keys.insert(k);
                                        self.key_pressed(k)?;
                                    }
                                    Ok(k) => {
                                        self.keys.remove(&k);
                                    }
                                }
                            }
                        }
                        Screen::Inventory | Screen::Creative
                            if !matches!(button, MouseButton::Left | MouseButton::Right) =>
                        {
                            // AbstractContainerScreen: another button clicks
                            // a slot only as the pick block key (its hotbar
                            // keys were handled above).
                            if self.settings.keys.get("key.pickItem")
                                == horizon_client::keybinds::mouse_name(button)
                            {
                                if pressed {
                                    self.inventory_pick_press();
                                } else {
                                    self.inventory_pick_release();
                                }
                                if let Some(key) = self.container {
                                    self.container_changed(key);
                                }
                            }
                        }
                        Screen::Inventory if pressed => {
                            self.inventory_press(button == MouseButton::Right);
                            if let Some(key) = self.container {
                                self.container_changed(key);
                            }
                        }
                        Screen::Inventory => {
                            self.inventory_mouse_release(button == MouseButton::Right);
                            self.station_release();
                            self.recipe_book_release();
                        }
                        Screen::Chat if pressed && button == MouseButton::Left => self.chat_click(),
                        Screen::Creative if pressed => {
                            self.click_creative(button == MouseButton::Right);
                        }
                        Screen::Creative => {
                            if button == MouseButton::Left {
                                self.creative_screen.release();
                            }
                            self.inventory_mouse_release(button == MouseButton::Right);
                        }
                        Screen::Paused
                            if pressed && button == MouseButton::Left && self.pause_menu =>
                        {
                            self.pause_click()?
                        }
                        Screen::WorldOptions if pressed && button == MouseButton::Left => {
                            self.world_options_click()
                        }
                        Screen::ServerLinks if pressed && button == MouseButton::Left => {
                            self.server_links_click()?
                        }
                        Screen::DebugOptions if pressed && button == MouseButton::Left => {
                            self.debug_options_click()
                        }
                        Screen::DebugOptions if button == MouseButton::Left => {
                            self.debug_options_release()
                        }
                        Screen::ConfirmLink if pressed && button == MouseButton::Left => {
                            self.confirm_link_click()
                        }
                        Screen::DialogWarning if pressed && button == MouseButton::Left => {
                            self.dialog_warning_click()?
                        }
                        Screen::WorldOptions if button == MouseButton::Left => {
                            self.world_options_release()
                        }
                        Screen::Friends if pressed && button == MouseButton::Left => {
                            self.friends_screen_click()
                        }
                        Screen::CommandBlock if pressed && button == MouseButton::Left => {
                            self.command_block_click()
                        }
                        Screen::BlockEditor if pressed && button == MouseButton::Left => {
                            self.block_editor_click()
                        }
                        Screen::BlockEditor if button == MouseButton::Left => {
                            self.block_editor_release()
                        }
                        Screen::Dead if pressed && button == MouseButton::Left => {
                            self.death_click()?
                        }
                        Screen::Advancements if button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.advancements_mouse(pressed, size);
                        }
                        Screen::Stats if pressed && button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.stats_click(size);
                        }
                        Screen::Stats if button == MouseButton::Left => self.stats_release(),
                        _ => {}
                    }
                }
                // ConfirmScreen.keyPressed: Escape answers no, once its delay is over.
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Dead && self.ui.death_confirm.is_some() =>
                {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                        && self
                            .ui
                            .death_confirm
                            .is_some_and(|t| t.elapsed().as_millis() >= 1000)
                    {
                        self.death_confirm_answer(false)?;
                    }
                }
                // ConfirmScreen.keyPressed: Escape answers no.
                WindowEvent::KeyboardInput { event, .. } if self.link_prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.answer_link_prompt(false);
                    }
                }
                // CodeOfConductScreen.shouldCloseOnEsc is false: no key leaves the screen.
                WindowEvent::KeyboardInput { .. } if self.code_of_conduct.is_some() => {}
                WindowEvent::KeyboardInput { event, .. } if self.server_packs.prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.answer_pack_prompt(false)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.dialog_open() => {
                    let shortcut = self.edit_shortcut(&event);
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(key) = event.physical_key
                    {
                        self.dialog_key(key, shortcut, event.text.as_deref())?
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Chat && self.title.is_none() =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        if event.state == ElementState::Pressed {
                            self.keys.insert(key);
                            self.chat_key(&event);
                        } else {
                            self.keys.remove(&key);
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.sign_edit.is_some() => {
                    self.sign_edit_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.book.is_some() => {
                    self.book_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Creative
                        && self.creative_screen.tab().search
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key
                        && self.search_tab_hotbar_key(key)
                    {
                        // The hovered slot took the key, and its characters
                        // go nowhere (`ignoreTextInput`).
                        typed = None;
                    } else if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.creative_search_shortcut(shortcut);
                    } else if let PhysicalKey::Code(key) = event.physical_key {
                        // EditBox.keyPressed: arrows, Home, End, Delete and Backspace.
                        let before = self.creative_screen.search.value.clone();
                        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                        if self.creative_screen.search.key(key, ctrl, shift) {
                            if self.creative_screen.search.value != before {
                                self.creative_screen.scroll = 0.0;
                            }
                        } else {
                            self.creative_search_edit(event.text.as_deref());
                        }
                    } else {
                        self.creative_search_edit(event.text.as_deref());
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.recipe_search_focused()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The recipe book's search box takes every key but Escape.
                    self.recipe_key(&event);
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.mod_browser_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // A mod's item panel's search box, as the recipe book's.
                    self.mod_browser_key(&event);
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.station_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The anvil rename box takes every key but Escape (AnvilScreen.keyPressed).
                    if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.station_shortcut(shortcut);
                    } else if let PhysicalKey::Code(key) = event.physical_key
                        && self.station_key(key)
                    {
                    } else {
                        let backspace = event.physical_key == PhysicalKey::Code(KeyCode::Backspace);
                        self.station_text(event.text.as_deref(), backspace);
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::WorldOptions
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    self.world_options_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Win => {
                    self.win_key(&event)?
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::CommandBlock => {
                    self.command_block_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::DebugOptions => {
                    self.debug_options_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::BlockEditor => {
                    self.block_editor_key(&event)
                }
                // Options consumes presses; releases still reach the bound
                // gameplay mapping below, including sneak/sprint toggles.
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Options && event.state == ElementState::Pressed =>
                {
                    if let Some(shortcut) = self.edit_shortcut(&event)
                        && self.options_shortcut(shortcut)
                    {
                        return Ok(());
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.options_key(key, event.text.as_deref())?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Paused
                        && event.state == ElementState::Pressed
                        && self.pause_menu
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.native_key_event(key, true, event.repeat);
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.pause_key(key)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Dead && event.state == ElementState::Pressed =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.native_key_event(key, true, event.repeat);
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.death_key(key)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.title.is_some() => {
                    if event.state == ElementState::Pressed {
                        if let Some(shortcut) = self.edit_shortcut(&event) {
                            self.title_shortcut(shortcut);
                            return Ok(());
                        }
                        let (w, h) = self.gui_size();
                        match event.physical_key {
                            // `WorldListEntry.keyPressed`: Space joins too, with the button click sound.
                            PhysicalKey::Code(
                                code @ (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space),
                            ) if code != KeyCode::Space
                                || self.title.as_ref().is_some_and(|t| {
                                    t.list_focused() || t.focused_button.is_some()
                                }) =>
                            {
                                let (action, sound) = self
                                    .title
                                    .as_mut()
                                    .map_or((Action::None, false), |t| t.confirm_key_size((w, h)));
                                self.save_title_options();
                                if sound {
                                    self.audio.play_button_click();
                                }
                                self.title_action(event_loop, action)?;
                            }
                            PhysicalKey::Code(
                                key @ (KeyCode::Backspace
                                | KeyCode::Delete
                                | KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::Home
                                | KeyCode::End),
                            ) if self.title.as_mut().is_some_and(|t| t.has_caret_field()) => {
                                let (ctrl, shift) =
                                    (self.has_control_down(), self.has_shift_down());
                                if let Some(t) = self.title.as_mut() {
                                    t.field_key(key, ctrl, shift);
                                }
                            }
                            PhysicalKey::Code(
                                key @ (KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::ArrowUp
                                | KeyCode::ArrowDown),
                            ) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.arrow_key(key, (w, h));
                                }
                            }
                            PhysicalKey::Code(KeyCode::Backspace) => self.title_edit(None, true),
                            PhysicalKey::Code(KeyCode::Tab) => {
                                let shift = self.has_shift_down();
                                if let Some(t) = self.title.as_mut() {
                                    t.tab_dir(shift, (w, h));
                                }
                            }
                            PhysicalKey::Code(KeyCode::Escape) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.escape();
                                    self.reopen_data_packs(event_loop)?;
                                }
                                self.save_title_options();
                            }
                            PhysicalKey::Code(KeyCode::F5) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.refresh_servers();
                                }
                            }
                            _ => self.title_edit(event.text.as_deref(), false),
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else {
                        return Ok(());
                    };
                    self.native_key_event(key, event.state == ElementState::Pressed, event.repeat);
                    // Screen.keyPressed sees the physical key, repeats included.
                    if event.state == ElementState::Pressed
                        && self.screen == Screen::Inventory
                        && (self.merchant_key(key) || self.recipe_book_key(key))
                    {
                        return Ok(());
                    }
                    // key.smoothCamera toggles the cinematic camera.
                    if self.screen == Screen::Playing
                        && event.state == ElementState::Pressed
                        && !event.repeat
                        && horizon_client::keybinds::key_name(key)
                            .is_some_and(|n| self.settings.keys.get("key.smoothCamera") == n)
                    {
                        self.smooth_camera = !self.smooth_camera;
                    }
                    // Key binds: act as the default key of every mapping bound
                    // to it (KeyMapping.set drives them all).
                    for mapped in self.settings.keys.translate_all(key) {
                        match mapped {
                            Ok(k) => {
                                self.mapped_key(
                                    k,
                                    event.state == ElementState::Pressed,
                                    event.repeat,
                                )?;
                            }
                            Err(button) => {
                                let action = match (event.state, event.repeat) {
                                    (ElementState::Released, _) => KeyAction::Release,
                                    (_, false) => KeyAction::Press,
                                    (_, true) => KeyAction::Repeat,
                                };
                                if let Some(pressed) = keyboard_button(
                                    gameplay_has_keyboard(
                                        self.screen,
                                        self.title.is_some(),
                                        self.dialog_open(),
                                    ),
                                    action,
                                ) {
                                    self.play_mouse(button, pressed)?;
                                } else if container_key_clones(button, action) {
                                    self.inventory_pick_key();
                                }
                            }
                        }
                    }
                }
                WindowEvent::RedrawRequested => {
                    self.observe_fullscreen();
                    self.sync_ime();
                    let now = Instant::now();
                    let frame_cpu = self
                        .options
                        .spikes
                        .and_then(|_| horizon_client::spans::thread_cpu());
                    let elapsed = now.duration_since(self.previous);
                    self.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.previous = now;
                    // Arrow-key camera controls belong to gameplay; chat and
                    // other screens use the same keys for their own navigation.
                    if self.screen == Screen::Playing
                        && self.title.is_none()
                        && self.sign_edit.is_none()
                        && self.book.is_none()
                        && !self.dialog_open()
                    {
                        let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
                        let secs = elapsed.as_secs_f32().min(0.1);
                        let yaw = (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0;
                        let pitch = (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0;
                        if yaw != 0.0 || pitch != 0.0 {
                            // 90 degrees a second, as `Entity.turn` degrees.
                            self.turn_player(
                                f64::from(yaw * secs / 0.15),
                                f64::from(pitch * secs / 0.15),
                            );
                        }
                    }
                    if self.title.is_none() {
                        horizon_client::spans::time("update_world", || self.update_world());
                        horizon_client::spans::time("spawn_player", || self.spawn_player())?;
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    self.heal_hold_latches();
                    // For the "I'm playing" ping: an atomic, sent on when it changes.
                    horizon_client::heartbeat::set_lan(self.is_published());
                    self.update_discord_presence();
                    let steps = self.clock.advance(elapsed);
                    // The world ticking in the frame: once its ticks took a
                    // tick's length, the rest are dropped (`FixedStep::tick_fits`).
                    let in_frame = self.remote.is_none();
                    let mut spent = Duration::ZERO;
                    for step in 0..steps {
                        if in_frame && !self.clock.tick_fits(spent) {
                            if let Some(warning) = self.clock.drop_ticks(steps - step) {
                                log::warn!("{warning}");
                            }
                            break;
                        }
                        let started = Instant::now();
                        if self.title.is_none() && !self.pause {
                            let tick = Instant::now();
                            horizon_client::spans::time("tick", || self.tick())?;
                            // In single player the tick is the integrated server's.
                            if self.remote.is_none() {
                                self.perf.tick(tick.elapsed().as_secs_f32() * 1000.0);
                                self.debug_charts.log_tick(tick, tick.elapsed());
                            }
                            self.tick_ambience();
                            self.tick_fov();
                        }
                        self.tick_win_screen()?;
                        // Gui.tick runs ChatListener.tick paused or not.
                        self.tick_chat_queue();
                        self.tick_music();
                        self.tick_sound_engine();
                        self.clock.record(started.elapsed());
                        spent += started.elapsed();
                        if let Some(report) = self.clock.take_sprint_report() {
                            self.sprint_report(&report);
                        }
                    }
                    self.extras_frame(elapsed);
                    self.extras_retry()?;
                    self.apply_mouse_look();
                    self.pad_look(elapsed.as_secs_f32());
                    self.advance_flight();
                    self.tick_world_icon();
                    let ticked = Instant::now();
                    let rendered = self.render()?;
                    self.apply_cursor_icon();
                    if let Some(threshold) = self.options.spikes
                        && let Some(r) = self.renderer.as_ref()
                    {
                        let cpu = now.elapsed().saturating_sub(r.last_wait).as_secs_f32() * 1000.0;
                        let spans = horizon_client::spans::take();
                        if cpu > threshold {
                            // The render thread's own CPU time in the frame:
                            // far below the wall time, the frame waited.
                            let worked = horizon_client::spans::thread_cpu()
                                .zip(frame_cpu)
                                .map_or_else(String::new, |(end, start)| {
                                    format!(", cpu {:.1} ms", (end - start).as_secs_f32() * 1000.0)
                                });
                            log::info!(
                                "Horizon: slow frame {cpu:.1} ms{worked} ({steps} ticks): {}",
                                horizon_client::spans::describe(&spans)
                            );
                        }
                    }
                    if let Some(r) = self.renderer.as_mut() {
                        let cpu = now.elapsed().saturating_sub(r.last_wait);
                        if rendered {
                            let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                            self.frame_graph.record(
                                ms(elapsed),
                                ms(cpu),
                                r.gpu_times.last().copied(),
                            );
                            if let Some(&gpu) = r.gpu_times.last() {
                                self.gpu_utilization =
                                    f64::from(gpu) * 100.0 / f64::from(ms(cpu).max(0.001));
                            }
                            self.frame_stats.record_cpu(ms(cpu));
                            let render = r.last_encode;
                            self.frame_stats.record_phases([
                                ms(updated - now),
                                ms(ticked - updated),
                                ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                                ms(render),
                            ]);
                        }
                        self.frame_stats.record_gpu(&r.gpu_times);
                        r.gpu_times.clear();
                    }
                    self.limit_frame();
                    self.pause = self.pauses_game();
                    if let Some(server) = &self.integrated {
                        server.set_paused(self.pause);
                        server.set_distances(
                            self.settings.render_distance,
                            self.settings.simulation_distance,
                            self.settings.entity_distance,
                        );
                    }
                    self.clock.end_frame(self.pause);
                    let mut consumed = false;
                    let finished = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.screenshot_result.take());
                    if let Some(result) = &finished
                        && self.ui.screenshot_message.is_some()
                    {
                        self.finish_screenshot(result);
                        consumed = true;
                    }
                    if let Some(result) = &finished
                        && let Some(reply) = self.pending_screenshot.take()
                    {
                        let _ = reply.send(
                            match result {
                                Ok([w, h]) => {
                                    serde_json::json!({ "ok": true, "width": w, "height": h })
                                }
                                Err(e) => serde_json::json!({ "error": e }),
                            }
                            .to_string(),
                        );
                        consumed = true;
                    }
                    // A result nobody asked for was the world icon's
                    // (`tick_world_icon`): a failure tries again a second
                    // later, as `tryTakeScreenshotIfNeeded` does.
                    if !consumed && let Some(Err(error)) = finished {
                        log::warn!("Horizon: couldn't save the world icon: {error}");
                    }
                    if rendered {
                        self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
                        self.debug_charts.frame_times.log(elapsed.as_nanos() as i64);
                        if self.frames == 0 {
                            // The launcher times Play to this line (launcher/src/perf.rs).
                            log::info!(
                                "Horizon: first frame after {} ms",
                                self.started.elapsed().as_millis()
                            );
                        }
                        if self.title.is_some() {
                            self.perf.title_shown();
                        } else if self.screen == Screen::Playing && self.player.is_some() {
                            self.perf.frame(elapsed.as_secs_f32() * 1000.0);
                            self.world_frames += 1;
                        }
                        self.frames += 1;
                        self.fps_frames += 1;
                        horizon_client::crash::debug_crash_tick();
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        if self.options.frames.is_some_and(|f| self.frames >= f)
                            || self
                                .options
                                .world_frames
                                .is_some_and(|f| self.world_frames >= f)
                        {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }

            Ok(())
        })();
        // ScreenEvent.CharacterTyped.Pre: GLFW's char callback comes after its
        // key callback, so a key's characters reach the screen the key left
        // open, after the key's own effects (`KeyboardHandler.charTyped`).
        self.screen_chars_typed(typed.as_deref());
        if let Err(e) = result {
            self.error = Some(e);
            self.exit_on_error(event_loop);
        }
    }
    /// The loop ended: close the pads' output, save the world, write the perf report.
    pub(super) fn shut_down(&mut self) {
        self.pad_close_output();
        if let Err(e) = self.save_for_exit() {
            log::warn!("Horizon: failed to save the world on exit: {e}");
        }
        self.finish_perf();
    }
    /// winit's `about_to_wait`: what happens between frames (stop requests, the closing
    /// save, world entry, the control socket, pads), then the next frame.
    pub(super) fn idle(&mut self, event_loop: &dyn EventLoopCtl) {
        // A splitscreen player's game has nothing to save: P1's shuts the process down.
        if horizon_threads::stop_requests() > 0
            && !self.options.split_guest
            && self.pending_save.as_ref().is_none_or(|save| !save.quit)
        {
            // SIGTERM, SIGINT or SIGHUP: the shutdown hook's server halt, as a window close.
            if let Err(error) = self.begin_closing_save(true) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
        }
        if self.pending_save.is_some() {
            match self.poll_closing_save() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    // Yield between polls even when disk I/O is slow.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(4),
                    ));
                }
                Err(error) => {
                    self.error = Some(error);
                    self.exit_on_error(event_loop);
                }
            }
            return;
        }
        if let Err(error) = self.resume_world_entry() {
            self.error = Some(error);
            self.exit_on_error(event_loop);
            return;
        }
        // The world a backup made before its upgrade opens (EditWorldScreen's callback).
        if let Some(folder) = self.title.as_mut().and_then(|t| t.take_open_request())
            && let Err(error) = self.open_world(&folder, None, None, false, None)
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        self.tick_server_packs();
        self.poll_control(event_loop);
        if self.pending_save.is_some() {
            return;
        }
        self.poll_pads(event_loop);
        self.pause_if_inactive();
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.error = Some("frame-limited run timed out".into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.options.offscreen {
                // Offscreen frames don't wait for the window to be drawn: an
                // occluded window (a locked macOS session) gets no redraws.
                let id = w.id();
                self.handle_window_event(event_loop, id, WindowEvent::RedrawRequested);
            } else {
                w.request_redraw();
            }
        } else if self.options.headless {
            // The headless loop paces the frames itself (`run_headless`).
            self.handle_window_event(event_loop, WindowId::dummy(), WindowEvent::RedrawRequested);
            return;
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
}
/// clouds.png as filled cells (`CloudRenderer.prepare`: alpha 10 or more).
fn load_clouds(root: &std::path::Path) -> (u32, u32, Vec<bool>) {
    horizon_client::hud::decode(&root.join("assets/minecraft/textures/environment/clouds.png"))
        .map(|(w, h, px)| {
            (
                w,
                h,
                px.as_chunks::<4>().0.iter().map(|p| p[3] >= 10).collect(),
            )
        })
        .unwrap_or((1, 1, vec![false]))
}
/// misc/forcefield.png for the world border wall (square, power of two).
fn load_forcefield(root: &std::path::Path) -> (u32, u32, Vec<u8>) {
    horizon_client::hud::decode(&root.join("assets/minecraft/textures/misc/forcefield.png"))
        .ok()
        .filter(|(w, h, _)| w == h && w.is_power_of_two())
        .unwrap_or((1, 1, vec![0; 4]))
}
impl<'a> Game<'a> {
    /// A game with no world open, over the loaded resources: the window's,
    /// or an integrated server's on its own thread.
    fn new(
        res: &'a Arc<Resources>,
        pack: PackResources,
        gui: UiResources,
        options: Options,
        settings: Settings,
        audio: Audio,
        prepared: Option<GameCpuResources>,
    ) -> Result<Self> {
        let root = options.resources.clone();
        let registry = pack.assets.registry;
        // Until a world is opened the streamer is idle.
        let mut streaming = Streaming::new(
            Arc::new(TestTerrain::new(registry, res.biomes())?),
            pack.context.clone(),
            1,
        );
        streaming.paused = true;
        let level = Level::default();
        let save = Arc::new(SaveDir::new(
            std::env::current_dir()?
                .join("saves")
                .join(".horizon-title"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        let prepared = prepared
            .map(Ok)
            .unwrap_or_else(|| GameCpuResources::load(&root))?;
        let world = World::new(res.facts().air, 0);
        let tutorial = tutorial::Tutorial::new(tutorial::Step::from_option(
            settings.file.int("tutorialStep"),
        ));
        let creative = options.creative;
        let inventory = vec![None; 36];
        let now = Instant::now();
        let look = options.look;
        let particles = horizon_client::particles::ParticleEngine::new(
            &root,
            &gui.atlas.sprites,
            &gui.atlas.translucent,
        );
        Ok(Game {
            world_biomes: None,
            res: res.clone(),
            rest_loader: None,
            pending_entry: None,
            world_loader: None,
            debug_density: RefCell::default(),
            pending_pack_reload: false,
            pack_folder: None,
            pack_reload: None,
            server_packs: Default::default(),
            link_prompt: None,
            code_of_conduct: None,
            dialogs: Default::default(),
            integrated_starting: None,
            world_renderer_ready: false,
            collision: res.collision(),
            motion: res.motion(),
            pack,
            gui,
            fog: Default::default(),
            screen_fx: Default::default(),
            item_activation: Default::default(),
            weather_parameters: Default::default(),
            panorama_rotation: Default::default(),
            world,
            streaming,
            lod: None,
            lod_source_attempted: false,
            far_beacons: HashMap::new(),
            lod_observe: Default::default(),
            lod_covered_at: None,
            options,
            player: None,
            spawn: None,
            clock: FixedStep::default(),
            previous_position: Vec3::ZERO,
            rope_rot_old: (0.0, 10.0),
            eye: 1.62,
            previous_eye: 1.62,
            yaw: look.map_or(level.rotation[0], |l| l.0),
            pitch: look.map_or(level.rotation[1], |l| l.1),
            inventory,
            selected: 0,
            carried: None,
            craft_grid: vec![None; 9],
            craft_size: 2,
            craft_table: None,
            inv_drag: None,
            tweaks: Default::default(),
            bundle_mouse: Default::default(),
            inv_clicks: Default::default(),
            last_inv_click: None,
            last_quick_moved: None,
            container: None,
            station: None,
            potions: Default::default(),
            scratch_slot: None,
            mining: None,
            destroying: HashMap::new(),
            destroy_delay: 0,
            use_delay: 0,
            creative,
            mode: if creative {
                horizon_client::command::args::GameMode::Creative
            } else {
                horizon_client::command::args::GameMode::Survival
            },
            keys: HashSet::new(),
            physical_keys: HashSet::new(),
            physical_buttons: HashSet::new(),
            right_click_emulation: Default::default(),
            left: false,
            right: false,
            cursor: (0.0, 0.0),
            cursor_icon: horizon_client::widgets::Cursor::Arrow,
            captured: false,
            keyboard_mouse: true,
            view_at: [0, 0],
            split_hud: Default::default(),
            focused: true,
            ime_allowed: false,
            ime_area: None,
            focus_lost: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            fullscreen: horizon_client::window::FullscreenState::new(false),
            video_modes: Default::default(),
            mouse: MouseLook::default(),
            agent_pointer: false,
            smooth_camera: false,
            toggles: Default::default(),
            was_playing: false,
            pause: false,
            debug: false,
            screen: Screen::Playing,
            window: None,
            renderer: None,
            entity_vertices: Vec::new(),
            dynamic_lights: Vec::new(),
            luminance: None,
            see_through_vertices: Vec::new(),
            shadow_only_vertices: Default::default(),
            entity_spans: Default::default(),
            glowing_vertices: Default::default(),
            pack_ids: None,
            health_hud: hud_layout::HealthHud::default(),
            started: now,
            spectator_gui: Default::default(),
            previous: now,
            last_input: now,
            pad: Default::default(),
            frames: 0,
            glint_frame: Default::default(),
            kept_block_entities: Default::default(),
            item_level_cache: Default::default(),
            world_frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_at: now,
            ticks: 0,
            error: None,
            message: None,
            save,
            level,
            session_lock: None,
            audio,
            music: {
                // The manager starts with the saved option (MusicManager's constructor).
                let mut music = horizon_client::music::MusicManager::default();
                music.frequency = horizon_client::music::MusicFrequency::parse(
                    &settings.file.name("musicFrequency"),
                );
                music
            },
            // ToastManager.initializeMusicToast: Pause and Pause and Toast make the toast.
            now_playing: (settings.file.int("musicToast").clamp(0, 2) != 0).then(Default::default),
            music_toast_state: settings.file.int("musicToast").clamp(0, 2) as u8,
            music_notes: Default::default(),
            ambience: Default::default(),
            sound_paused: false,
            cart_sounds: HashSet::new(),
            riding_cart_sounds: None,
            riding_sound: None,
            was_fall_flying: false,
            bee_sounds: HashMap::new(),
            guardian_sounds: HashSet::new(),
            dropped: Vec::new(),
            next_entity: 0,
            ticked_entities_through: u32::MAX,
            particles,
            pickups: Default::default(),
            sign_edit: None,
            book: None,
            book_last_click: None,
            lectern_pulses: Vec::new(),
            loading_progress: 0.0,
            creation_job: None,
            loading_progress_at: None,
            spawn_origin: None,
            spawn_spiral: Default::default(),
            player_spawn: None,
            spawn_wait: Vec::new(),
            mobs: Vec::new(),
            creatures: Default::default(),
            sculk: Default::default(),
            raiding: Default::default(),
            attack_ticks: 100,
            attack_item: None,
            last_hurt_by_player: HashMap::new(),
            lead_killers: HashMap::new(),
            love_cause: HashMap::new(),
            died: Default::default(),
            be_anims: {
                let mut anims = horizon_client::block_entity_anim::BlockEntityAnims::default();
                anims.classify(registry);
                anims
            },
            using: None,
            use_duration: 0,
            view: Default::default(),
            limiter: horizon_client::window::FramerateLimiter::new(0),
            frame_clock: Default::default(),
            scope_scale: 0.5,
            frame_ticks: 0.0,
            title: None,
            armor: [None, None, None, None],
            offhand: None,
            hotbar_pop: [0; 10],
            hand: Hand::Main,
            shield_cooldown: 0,
            cooldowns: Default::default(),
            command_source: None,
            titles: Default::default(),
            tool_highlight: Default::default(),
            saving_indicator: Default::default(),
            autosaving: false,
            applied_modifiers: Vec::new(),
            experience: Experience::default(),
            orbs: Vec::new(),
            xp_delay: 0,
            last_level_up: 0,
            swing: Default::default(),
            swings_started: 0,
            heights: Default::default(),
            sway: Default::default(),
            visible_hands: [(0, None), (0, None)],
            hands_replaced: [false; 2],
            clicks: Default::default(),
            attack_clicks: 0,
            use_clicks: 0,
            hand_strength: Default::default(),
            settings,
            tutorial,
            tutorial_toasts: Vec::new(),
            narrator_toast: None,
            narration: Default::default(),
            options_from_title: false,
            dragging: false,
            avatar: Default::default(),
            flash: 0,
            rain_sound_time: 0,
            lightning_seen: [0; 2],
            sleeping: None,
            sleep_timer: 0,
            player_beds: Vec::new(),
            sleep_seen: (0, 0),
            dim: Dim::Overworld,
            world_root: PathBuf::new(),
            world_flat: false,
            world_debug: false,
            travelling: None,
            server_trips: Vec::new(),
            level_teleports: Vec::new(),
            levels: Default::default(),
            level_pass: Default::default(),
            portals: Default::default(),
            end_travel: false,
            respawn_config: None,
            respawn_travel: None,
            end_portal_travel: false,
            win: None,
            end: Default::default(),
            block_ticks: Ticks::new(0),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            falling: Vec::new(),
            transient_blocks: Default::default(),
            paintings: Vec::new(),
            painting_variants: prepared.painting_variants,
            item_frames: Vec::new(),
            frame_model: prepared.frame_model,
            glow_frame_model: prepared.glow_frame_model,
            map_frame_model: prepared.map_frame_model,
            glow_map_frame_model: prepared.glow_map_frame_model,
            armor_stands: Vec::new(),
            cushions: Vec::new(),
            others: Vec::new(),
            armor_stand_assets: prepared.armor_stand_assets,
            waypoint_styles: prepared.waypoint_styles,
            waypoints: Default::default(),
            locator_camera: None,
            post_effects: Vec::new(),
            spectated_post_effect: None,
            experience_display: Default::default(),
            boats: Vec::new(),
            leash_boats: Vec::new(),
            boat_models: prepared.boat_models,
            crystal_model: prepared.crystal_model,
            cushion_model: prepared.cushion_model,
            carts: Vec::new(),
            jukebox_sounds: HashMap::new(),
            bolts: Vec::new(),
            maps: maps::Maps::default(),
            fight: dragon::Fight::default(),
            dragon: None,
            crystals: Vec::new(),
            dragon_fireballs: Vec::new(),
            breath_clouds: Vec::new(),
            dragon_model: prepared.dragon_model,
            map_patches: Vec::new(),
            text_displays: Default::default(),
            interactions: Default::default(),
            remote_knots: Default::default(),
            knot_ids: Default::default(),
            text_display_vertices: 0,
            skin_downloads: RefCell::new({
                let mut downloads = horizon_client::skins::Downloader::new(
                    std::env::current_dir()
                        .unwrap_or_default()
                        .join("assets/skins"),
                );
                downloads.set_resource_root(root.clone());
                downloads
            }),
            skin_profiles: RefCell::new(horizon_client::skins::Resolver::new()),
            skin_slots: horizon_client::skins::SkinSlots::new(horizon_client::hud::SKIN_SLOTS),
            trim_slots: Vec::new(),
            cart_models: prepared.cart_models,
            riding: None,
            pending_vehicle: None,
            ender_items: vec![None; 27],
            ender_open: None,
            pending: Default::default(),
            spill: Default::default(),
            recipe_ui: Default::default(),
            mount_menu: Default::default(),
            hook: None,
            remote_hooks: Vec::new(),
            tridents: Vec::new(),
            rockets: Vec::new(),
            ominous_spawners: Vec::new(),
            creaking: Default::default(),
            spawners: Default::default(),
            spawn_scratch: Default::default(),
            trident_model: prepared.trident_model,
            arrow_model: prepared.arrow_model,
            bee_stinger_model: prepared.bee_stinger_model,
            tnts: Vec::new(),
            hoppers: HashMap::new(),
            mod_screen: None,
            item_columns: None,
            creative_screen: prepared.creative_screen,
            control: None,
            agent_control_on: false,
            agent_last: None,
            frame_stats: Default::default(),
            perf: Default::default(),
            pending_screenshot: None,
            pending_bench: None,
            flight: None,
            ui_wait: Default::default(),
            events: Default::default(),
            agent: agent::Agent::default(),
            view_frustum: None,
            entity_culler: None,
            pack_shadow_casters: (false, false),
            pending_uploads: Vec::new(),
            frame_graph: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            recipe_tables: Default::default(),
            panel: Default::default(),
            native_shaders: Default::default(),
            native_lifecycle: Default::default(),
            native_net: Default::default(),
            sky_darken_memo: Default::default(),
            water_plane_memo: Default::default(),
            water_plane_scan: Default::default(),
            persist: Default::default(),
            remote: None,
            lan: None,
            integrated: None,
            level_shapes: Default::default(),
            serving: None,
            world_options: None,
            options_to_world_options: false,
            options_to_game: false,
            command_block_screen: None,
            block_editor: None,
            structure_templates: HashMap::new(),
            test_defs: std::cell::RefCell::new(HashMap::new()),
            test_highlights: HashMap::new(),
            game_tests: game_tests::GameTests::default(),
            friends: None,
            debug_keys: Default::default(),
            debug_charts: Default::default(),
            debug_entries: horizon_client::debug_entries::DebugEntries::load(
                &std::env::current_dir().unwrap_or_default(),
            ),
            debug_options: None,
            memory_meter: Default::default(),
            gpu_utilization: 0.0,
            rendered_mobs: Default::default(),
            spawn_counts: HashMap::new(),
            pause_menu: true,
            pause_focus: None,
            death_focus: None,
            previous_mode: None,
            gamemode_switcher: None,
            clipboard: Default::default(),
            text_selection: Default::default(),
            saved_for_exit: false,
            pending_save: None,
            world_icon: None,
            forced_hit: None,
            tick_hit: None,
            entity_reach: 3.0,
            block_reach: 4.5,
            guest_use: None,
            use_failed: false,
            lan_events: Vec::new(),
            lan_songs: Vec::new(),
            lan_cushion_breaks: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_kinetic_hits: Vec::new(),
            stabbed: HashMap::new(),
            known_movement: Vec3::ZERO,
            kinetic_feedback: HashMap::new(),
            lan_global_events: Vec::new(),
            lid_openers: HashMap::new(),
            lan_acting: None,
            lan_break_pos: None,
            guest_packets: Vec::new(),
            guest_cooldowns: Vec::new(),
            guest_entity_events: Vec::new(),
            guest_equipment_breaks: Vec::new(),
            guest_player_sounds: Vec::new(),
            lan_acting_name: None,
            lan_acting_uuid: None,
            lan_acting_permission: None,
            chat_capture: None,
            admin_feedback: Vec::new(),
            console_broadcast: None,
            server_lists: None,
            world_tables: None,
            cmds: Default::default(),
            ui: chat::UiState::new(),
            combat: Default::default(),
            guest_combat: Default::default(),
            mount: Default::default(),
            stats: Default::default(),
            score: Default::default(),
            fns: Default::default(),
            bosses: Default::default(),
            boss_overlay: Default::default(),
            exec: Default::default(),
            rand_sequences: Default::default(),
            brushing: Default::default(),
            border: Default::default(),
            border_renderer: Default::default(),
            border_state: Default::default(),
            tab_list: Default::default(),
            subtitles: Default::default(),
            adv: Default::default(),
            restore_player_state: true,
            stats_ui: Default::default(),
            adv_ui: Default::default(),
            links_ui: Default::default(),
            camera_type: Default::default(),
            fangs: Default::default(),
            player_model: None,
        })
    }
}

/// CPU-only startup data, owned by one loader generation. No file reads or
/// model baking are needed when the event loop installs this bundle.
struct GameCpuResources {
    painting_variants: paintings::PaintingVariants,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    boat_models: boats::BoatModels,
    cart_models: minecarts::CartModels,
    crystal_model: Option<horizon_client::mobs::Part>,
    cushion_model: Option<horizon_client::mobs::Part>,
    dragon_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    bee_stinger_model: Option<horizon_client::mobs::Part>,
    trident_model: Option<horizon_client::mobs::Part>,
    creative_screen: Creative,
}
impl GameCpuResources {
    fn load(root: &std::path::Path) -> Result<Self> {
        let document = horizon_client::mobs::EntityModelDocument::load(root).ok();
        let model = |key| document.as_ref().and_then(|document| document.model(key));
        Ok(Self {
            painting_variants: paintings::PaintingVariants::load(root),
            frame_model: frames::bake_frame(root, "item_frame"),
            glow_frame_model: frames::bake_frame(root, "glow_item_frame"),
            map_frame_model: frames::bake_frame(root, "item_frame_map"),
            glow_map_frame_model: frames::bake_frame(root, "glow_item_frame_map"),
            armor_stand_assets: armor_stands::ArmorStandAssets::load(root, document.as_ref()),
            waypoint_styles: horizon_client::locator_bar::WaypointStyles::load(root),
            boat_models: boats::BoatModels::load(document.as_ref()),
            cart_models: minecarts::CartModels::load(document.as_ref()),
            crystal_model: model("minecraft:end_crystal#main"),
            cushion_model: model("minecraft:cushion#main"),
            dragon_model: model("minecraft:ender_dragon#main"),
            arrow_model: model("minecraft:arrow#main"),
            bee_stinger_model: model("minecraft:bee_stinger#main"),
            trident_model: model("minecraft:trident#main"),
            creative_screen: Creative::load(root)?,
        })
    }
}

/// The data the background loader produces for `Game::new`.
struct Loaded {
    game_cpu: Option<GameCpuResources>,
    renderer_cpu: Option<horizon_client::gfx::PreparedTextures>,
    root: PathBuf,
    pack_error: Option<String>,
    resources: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pack: PackResources,
    /// The menu bundle, for the synchronous launch; the loading screen receives
    /// it earlier through its own channel.
    gui: Option<UiResources>,
}

/// What the background loader publishes: the menu bundle as soon as it is
/// ready, then the core bundle with a handle to the rest worker.
enum LoaderMsg {
    Ui(UiResources, PathBuf),
    Done(std::result::Result<Loaded, String>),
}

enum PendingWorldEntry {
    Local {
        folder: String,
        name: Option<String>,
        seed: Option<i64>,
        creative: bool,
        create: Option<Box<horizon_client::title::WorldOptions>>,
    },
    Server {
        address: String,
        name: String,
        lan: bool,
        /// A transfer's carried-over server cookies (`TransferState`).
        cookies: Option<horizon_client::net::Cookies>,
    },
}

struct WorldRestLoader {
    progress: Progress,
    handle: JoinHandle<std::result::Result<(), String>>,
}

impl WorldRestLoader {
    fn ready(&self) -> Result<bool> {
        // No time limit: the title stays usable while this loads, and a slow
        // disk or a debug build can take minutes. A failure ends the thread
        // and is reported by `finish`.
        Ok(self.handle.is_finished())
    }

    fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| "world resource loader panicked")?
            .map_err(Into::into)
    }
}

impl Game<'_> {
    fn defer_world_entry(&mut self, entry: PendingWorldEntry) -> Result<bool> {
        self.poll_world_rest()?;
        if self.rest_loader.is_none() && self.world_loader.is_none() {
            return Ok(false);
        }
        self.perf.world_opening();
        if self.title.is_none() {
            self.title = Some(
                Title::new(
                    std::env::current_dir()?.join("saves"),
                    &self.options.resources,
                )
                .with_user(|| self.options.player_name()),
            );
        }
        self.pending_entry = Some(entry);
        log::info!("Horizon: world entry waiting for world resources");
        Ok(true)
    }

    fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_integrated_start()?;
        self.poll_world_rest()?;
        self.poll_world_creation();
        if let Some(loader) = &self.world_loader {
            if !loader.ready() {
                return Ok(());
            }
            let loader = self
                .world_loader
                .take()
                .ok_or("missing world initialization worker")?;
            // FileFixerProgressScreen's Cancel: the upgrade stopped, back to the title.
            let cancelled = loader.cancelled;
            let upgrade_canceled = loader.upgrade.is_canceled();
            let prepared = loader.finish();
            if !cancelled && !(upgrade_canceled && prepared.is_err()) {
                self.finish_open_world(prepared?)?;
            }
        }
        if self.rest_loader.is_some() {
            return Ok(());
        }
        let on_world_screens = self.title.as_ref().is_some_and(|title| {
            !matches!(
                title.mode,
                horizon_client::title::Mode::Main | horizon_client::title::Mode::Multiplayer(_)
            )
        });
        world_loading::preload_world_data(
            on_world_screens,
            &self.options.resources,
            self.pack.assets.registry,
        );
        if std::mem::take(&mut self.pending_pack_reload) {
            self.reload_packs();
        }
        match self.pending_entry.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                cookies,
                lan,
            }) => {
                self.connect_server_with_options(&address, &name, cookies, lan)?;
            }
            None => {}
        }
        Ok(())
    }

    /// `CreateWorldScreen.openCreateWorldScreen`: read the world generation
    /// data the create screen describes, on a worker, then open the screen
    /// (`title.preparing`).
    fn poll_world_creation(&mut self) {
        let Some(title) = self.title.as_ref() else {
            return;
        };
        if !title.preparing() {
            return;
        }
        if self.creation_job.is_none() {
            let dir = self.options.resources.join("worldgen");
            let registry = self.pack.assets.registry;
            // The chosen packs' worldgen over the game's own:
            // `applyNewPackConfig` reloads with the new pack config.
            let overrides = title
                .creation_packs()
                .map(|(dir, packs)| {
                    world_loading::creation_worldgen_overrides(
                        dir.as_deref(),
                        &self.options.resources,
                        &packs,
                        self.res.biomes(),
                    )
                })
                .unwrap_or_default();
            self.creation_job = std::thread::Builder::new()
                .name("world-creation".into())
                .spawn(move || {
                    horizon_client::vanillagen::shared_data_with(&dir, &overrides, registry)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
                .ok();
            return;
        }
        if !self
            .creation_job
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
        {
            return;
        }
        let outcome = self.creation_job.take().and_then(|job| job.join().ok());
        if let Some(title) = self.title.as_mut() {
            match outcome {
                Some(Ok(())) => title.finish_preparing(),
                // `applyNewPackConfig`'s failure: back out of the read with
                // the message, the Create screen's options intact.
                outcome => {
                    let e = outcome
                        .and_then(|r| r.err())
                        .unwrap_or_else(|| "the read never ran".to_owned());
                    log::warn!("Horizon: preparing world creation failed: {e}");
                    // Clear the flag first: `failed` sets the error screen
                    // with Create as its back target, which
                    // `finish_preparing` would replace with a fresh world.
                    title.finish_preparing();
                    title.failed(format!(
                        "{}: {e}",
                        horizon_client::lang::get_or(
                            "createWorld.preparing",
                            "Preparing for world creation..."
                        )
                    ));
                }
            }
        }
    }

    fn poll_world_rest(&mut self) -> Result<()> {
        if let Some(loader) = &self.rest_loader
            && loader.ready()?
        {
            self.rest_loader
                .take()
                .ok_or("missing world loader")?
                .finish()?;
        }
        Ok(())
    }

    // ---- the world's data packs' content tables ---------------------------

    /// The content tables the world plays with: its data packs' over the
    /// game's own when it has packs, `Resources.rest`'s own otherwise.
    fn items(&self) -> &Items {
        match &self.world_tables {
            Some(t) => &t.items,
            None => self.res.items(),
        }
    }
    fn recipes(&self) -> &Recipes {
        match &self.world_tables {
            Some(t) => &t.recipes,
            None => self.res.recipes(),
        }
    }
    fn stations(&self) -> &horizon_client::stations::StationData {
        match &self.world_tables {
            Some(t) => &t.stations,
            None => self.res.stations(),
        }
    }
    /// The world's packs' content files, none without packs.
    /// The biome table the world plays with, `None` before a world opens.
    fn packed_biomes(&self) -> Option<Arc<Biomes>> {
        let table = world_loading::packed_biomes(
            &self.world_root,
            &self.options.resources,
            self.res.biomes(),
            &self.level.enabled_features,
        );
        (!Arc::ptr_eq(&table, self.res.biomes())).then_some(table)
    }
    fn pack_files(&self) -> horizon_world::loot::PackOverlay<'_> {
        pack_files(&self.world_tables)
    }
    /// Load the world's data packs' content tables (`None`: the world has no
    /// packs, and `Resources.rest` stays in use). This re-reads the tables,
    /// so it runs once per world open and on `/reload`.
    fn build_world_tables(&self) -> Option<WorldTables> {
        let files = functions::content_overlay(
            &self.world_root,
            &self.options.resources,
            self.cmds.vanilla_disabled,
            &self.level.enabled_features,
        );
        if files.is_empty() {
            return None;
        }
        let overlay = horizon_world::loot::PackOverlay { files: &files };
        let root = &self.options.resources;
        let items = match Items::load(root, self.pack.assets.registry, self.res.facts(), &overlay) {
            Ok(items) => items,
            Err(e) => {
                log::warn!("Horizon: the world's data packs are not applied: {e}");
                return None;
            }
        };
        let recipes = match Recipes::load(root, &items, &overlay) {
            Ok(recipes) => recipes,
            Err(e) => {
                log::warn!("Horizon: the world's data packs are not applied: {e}");
                return None;
            }
        };
        let stations = horizon_client::stations::StationData::load(root, &items, &overlay);
        Some(WorldTables {
            files,
            items,
            recipes,
            stations,
        })
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    pub(super) fn reload_world_tables(&mut self) {
        if self.remote.is_some() {
            return;
        }
        self.world_tables = self.build_world_tables();
        horizon_client::structure_blocks::forget_loot_data();
        // PlayerList.reloadResources: the tags and recipes go to the players
        // and their advancements reload. The synchronized registries (the
        // dialogs, and the commands' ids of them) stay the hub's.
        let kept = self.lan.as_ref().map(|host| host.hub.lan.shared.sync());
        match kept {
            Some(kept) => {
                match lan::reload_sync_data(&self.options.resources, &self.pack_files(), &kept) {
                    Ok(sync) => {
                        if let Some(host) = self.lan.as_mut() {
                            host.hub.reload_resources(sync);
                        }
                    }
                    Err(e) => {
                        log::warn!("Horizon: the players are not sent the reloaded data: {e}")
                    }
                }
            }
            None => lan::forget_sync_data(),
        }
        let overlay = pack_files(&self.world_tables);
        self.adv.tree = Arc::new(horizon_client::advancements::Tree::load_with(
            &self.options.resources.join("data"),
            &overlay,
        ));
        self.adv.reset_tree();
    }

    fn join_world_rest(&mut self) -> Result<()> {
        if self.rest_loader.is_some() {
            let waited = Instant::now();
            log::info!("Horizon: waiting for world resources");
            while self.rest_loader.is_some() {
                self.poll_world_rest()?;
                if self.rest_loader.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            log::info!(
                "Horizon: world resources joined after {} ms",
                waited.elapsed().as_millis()
            );
        }
        if self.res.rest.get().is_none() {
            return Err("world resources are unavailable".into());
        }
        Ok(())
    }
}

/// `key.hotbar.1` to `key.hotbar.9` in a menu.
const HOTBAR_DIGITS: [KeyCode; 9] = [
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
/// The other keys `InputConstants.Key.getNumericKeyValue` gives a digit.
const NUMERIC_KEYS: [KeyCode; 11] = [
    KeyCode::Digit0,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
];

const STAGE_LINES: [&str; 10] = [
    "Loading menu",
    "Loading assets",
    "Loading block facts",
    "Loading biomes",
    "Loading pack tables",
    "Loading collision",
    "Loading items",
    "Loading recipes",
    "Loading world resources",
    "World resources ready",
];

/// The background loader's checkpoint, read by the loading screen each frame.
#[derive(Clone)]
struct Progress {
    value: Arc<AtomicU32>,
    lines: &'static [&'static str],
}
impl Progress {
    fn new() -> Self {
        Self::with(&STAGE_LINES)
    }
    fn with(lines: &'static [&'static str]) -> Self {
        Self {
            value: Arc::new(AtomicU32::new(0)),
            lines,
        }
    }
    fn set(&self, stage: u32) {
        self.value.store(stage, Ordering::Relaxed);
    }
    fn fraction(&self) -> f32 {
        self.value.load(Ordering::Relaxed) as f32 / (self.lines.len() - 1) as f32
    }
    fn line(&self) -> &'static str {
        self.lines[(self.value.load(Ordering::Relaxed) as usize).min(self.lines.len() - 1)]
    }
}

/// Harness and dedicated runs keep the fully-synchronous startup so their
/// output stays byte-identical; only an ordinary launch shows the window first.
fn synchronous_startup(options: &Options) -> bool {
    // A headless game has no window for the loading screen.
    options.headless
        || options.frames.is_some()
        || options.world_frames.is_some()
        || options.benchmark.is_some()
        || options.control.is_some()
        || !options.execution_role.is_client()
        || std::env::var_os("HORIZON_DISPENSE_FIXTURE").is_some()
        || std::env::var_os("HORIZON_SCULK_FIXTURE").is_some()
        || std::env::var_os("HORIZON_COMMAND_SCRIPT").is_some()
        || std::env::var_os("HORIZON_NET_REPORT").is_some()
}

/// The local world a client's launch opens at once (`--world`,
/// `--quickPlaySingleplayer`), whose generator data can load with the game's.
fn launch_world(options: &Options) -> Option<&str> {
    (options.execution_role.is_client() && options.server.is_none() && !options.test_terrain)
        .then_some(options.world.as_deref())
        .flatten()
}

/// A step of the resource load: on its own scoped thread, or, when the
/// settings allow a single worker, run where it is joined.
enum Step<'scope, T> {
    Thread(std::thread::ScopedJoinHandle<'scope, T>),
    Later(Box<dyn FnOnce() -> T + Send + 'scope>),
}
impl<'scope, T: Send + 'scope> Step<'scope, T> {
    fn spawn(
        scope: &'scope std::thread::Scope<'scope, '_>,
        parallel: bool,
        f: impl FnOnce() -> T + Send + 'scope,
    ) -> Self {
        if parallel {
            Self::Thread(scope.spawn(f))
        } else {
            Self::Later(Box::new(f))
        }
    }
    fn join(self) -> T {
        match self {
            Self::Thread(handle) => handle.join().unwrap(),
            Self::Later(f) => f(),
        }
    }
}

/// The world bundle's core tables from one resource root, before
/// `load_resources` binds them into the pack tables and `WorldCore`.
struct CoreTables {
    assets: Assets,
    render: RenderAssets,
    item_renderer: horizon_client::item_render::ItemRenderer,
    visibility: horizon_world::block_render::FaceVisibility,
    gameplay: GameplayTables,
    biomes: Biomes,
    environment: Environment,
    collision: BlockCollisionRegistry<'static>,
}
/// The block facts and the tables read from them.
struct GameplayTables {
    behaviour_json: serde_json::Value,
    facts: BlockFacts,
    gameplay_projection: Vec<horizon_client::blocks::GameplayProjection>,
    light: LightTables,
    motion: MotionResources,
}

/// [`CoreTables`] at `root`. Every table reads the block registry; past it,
/// the block models and sprites (then the item models, which share their
/// parsed models), the face visibility, the collisions, the biomes and the
/// environment need no block facts and load beside them. The outer error
/// fails the load; the inner one is the packs' (the registry or the block
/// models), which `load_resources` rolls back from.
fn load_core(
    root: &std::path::Path,
    headless: bool,
    parallel: bool,
    progress: &Progress,
) -> std::result::Result<std::result::Result<CoreTables, String>, String> {
    let assets = match Assets::load(root) {
        Ok(assets) => assets,
        Err(e) => return Ok(Err(e.to_string())),
    };
    progress.set(1);
    let registry = assets.registry;
    let (render, gameplay, visibility, biomes, environment, collision) =
        std::thread::scope(|scope| {
            let assets = &assets;
            let render = Step::spawn(scope, parallel, move || {
                // A server reads no block sprites' pixels, which only the
                // renderer draws (32 MB).
                let render = if headless {
                    assets.reload_for_server(root)
                } else {
                    assets.reload(root)
                }
                .map_err(|e| e.to_string())?;
                let item_renderer = horizon_client::item_render::ItemRenderer::load_with_library(
                    root,
                    render.model_library.clone(),
                )
                .map_err(|e| e.to_string());
                Ok((render, item_renderer))
            });
            // Nor the face visibility the mesher and the block entity
            // renderer cull with (19 MB).
            let visibility = Step::spawn(scope, parallel, || {
                if headless {
                    return Ok(horizon_world::block_render::FaceVisibility::empty());
                }
                models::load_visibility(root, registry).map_err(|e| e.to_string())
            });
            let biomes = Step::spawn(scope, parallel, || {
                Biomes::load(root).map_err(|e| e.to_string())
            });
            let environment = Step::spawn(scope, parallel, || {
                Environment::load(root).map_err(|e| e.to_string())
            });
            let collision = Step::spawn(scope, parallel, || {
                collisions::load(root, registry).map_err(|e| e.to_string())
            });
            let gameplay = gameplay_tables(root, assets, parallel, progress);
            (
                render.join(),
                gameplay,
                visibility.join(),
                biomes.join(),
                environment.join(),
                collision.join(),
            )
        });
    // The packs' failure first: it rolls back whatever else failed.
    let (render, item_renderer) = match render {
        Ok(render) => render,
        Err(e) => return Ok(Err(e)),
    };
    let gameplay = gameplay?;
    progress.set(3);
    Ok(Ok(CoreTables {
        gameplay,
        biomes: biomes?,
        environment: environment?,
        visibility: visibility?,
        item_renderer: item_renderer?,
        collision: collision?,
        assets,
        render,
    }))
}
/// The block facts, then the light tables beside the motion tables, which
/// both read them.
fn gameplay_tables(
    root: &std::path::Path,
    assets: &Assets,
    parallel: bool,
    progress: &Progress,
) -> std::result::Result<GameplayTables, String> {
    let behaviour_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json")).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())?;
    let (facts, gameplay_projection) =
        BlockFacts::load_with_projection(root, assets.registry, &behaviour_json, parallel)
            .map_err(|e| e.to_string())?;
    horizon_client::block_contact::set_mod_blocks(
        facts
            .states
            .iter()
            .any(|s| !s.name.starts_with("minecraft:")),
    );
    progress.set(2);
    let (light, motion) = std::thread::scope(|scope| {
        let light = Step::spawn(scope, parallel, || {
            LightTables::load(root, assets.registry, &facts).map_err(|e| e.to_string())
        });
        let motion =
            MotionResources::load_with_facts(root, assets, &facts).map_err(|e| e.to_string());
        (light.join(), motion)
    });
    Ok(GameplayTables {
        light: light?,
        motion: motion?,
        behaviour_json,
        facts,
        gameplay_projection,
    })
}

/// The synchronous launch loads both phases; an ordinary launch returns the
/// core bundle while the rest worker keeps loading. `launch_world`: the world
/// the launch opens, whose data is preloaded meanwhile.
#[allow(clippy::too_many_arguments)]
fn load_resources(
    mut root: PathBuf,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    headless: bool,
    progress: &Progress,
    publish: &mut dyn FnMut(UiResources, &std::path::Path),
) -> Result<Loaded> {
    let threads = settings.horizon.worker_threads();
    horizon_client::sprites::set_texture_budget(settings.horizon.texture_budget_mib);
    horizon_client::sprites::set_labpbr_maps(shader_packs::picks_native_pack(settings));
    // Stage A: the menu bundle (atlas and panorama). With a loading screen it is
    // published at once so the title screen can appear while the world bundle
    // loads; the synchronous launch instead carries it in `Loaded`.
    let mut pack_error = None;
    // A dedicated server builds only the font (UiResources::server), as the
    // integrated server's game does: the full atlas and panorama are 144 MB.
    let load_ui = |root: &std::path::Path| {
        if headless {
            UiResources::server(root)
        } else {
            UiResources::load(root, vanilla_root)
        }
    };
    // The menu bundle loads on this thread while workers build the world
    // bundle's core tables from the same root (`load_core`). A pack failure
    // in either starts both again with the game's own resources
    // (Minecraft.rollbackResourcePacks).
    let parallel = threads > 1;
    let mut offer = |ui: UiResources, root: &std::path::Path| {
        if menu {
            horizon_client::telemetry::mark_ui_ready();
            log::info!(
                "Horizon: UI bundle ready after {} ms",
                started.elapsed().as_millis()
            );
            publish(ui, root);
            None
        } else {
            Some(ui)
        }
    };
    let (core, loaded_gui) = loop {
        let (ui, core) = std::thread::scope(|scope| {
            let core = Step::spawn(scope, parallel, || {
                load_core(&root, headless, parallel, progress)
            });
            let ui = load_ui(&root).map(|ui| offer(ui, &root));
            (ui, core.join())
        });
        let packs = root != vanilla_root;
        let failure = match (ui, core) {
            (Ok(gui), Ok(Ok(core))) => break (core, gui),
            (Err(e), _) if packs => e.to_string(),
            (_, Ok(Err(e))) if packs => e,
            (_, Ok(Err(e))) => {
                return Err(format!(
                    "{e}\nPrepare resources with the launcher, or: horizon-prepare --prepare"
                )
                .into());
            }
            (_, Err(e)) => return Err(e.into()),
            (Err(e), Ok(Ok(_))) => return Err(e),
        };
        log::warn!(
            "Horizon: resource packs failed to load ({failure}); using the default resources"
        );
        pack_error = Some(failure);
        root = vanilla_root.to_path_buf();
    };
    log::info!("load core tables: {:.2}s", started.elapsed().as_secs_f32());
    let CoreTables {
        assets,
        render,
        item_renderer,
        visibility,
        gameplay:
            GameplayTables {
                behaviour_json,
                facts,
                gameplay_projection,
                light,
                motion,
            },
        biomes,
        environment,
        collision,
    } = core;
    let assets = Arc::new(assets);
    let render = Arc::new(render);
    let biomes = Arc::new(biomes);
    if let Some(folder) = launch_world {
        world_loading::preload_launch_world(folder, &root, assets.registry, biomes.clone());
    }
    let tables = Arc::new(MeshTables::new(&assets, &render, &facts, visibility)?);
    let pack = PackResources::assemble(
        &root,
        assets.clone(),
        render,
        tables,
        item_renderer,
        biomes.clone(),
        Arc::new(light),
    );
    log::info!("load pack tables: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(4);
    if !menu {
        horizon_client::ui::load_metadata(&root);
        horizon_client::lang::set(horizon_client::lang::Language::load(
            &[&root],
            &settings.language,
        ));
    }
    progress.set(5);
    let resources = Resources {
        core: WorldCore {
            facts,
            biomes: biomes.clone(),
            environment,
            collision,
            motion,
            threads,
        },
        rest: std::sync::OnceLock::new(),
    };
    let resources = Arc::new(resources);
    if menu {
        horizon_client::telemetry::mark_core_ready();
        log::info!(
            "Horizon: core-ready after {} ms",
            started.elapsed().as_millis()
        );
    }
    // Finish fallible CPU preparation before starting the tail, so an error
    // cannot detach a tail worker that races the next pack-loader generation.
    let prepared_at = Instant::now();
    let game_cpu = menu.then(|| GameCpuResources::load(&root)).transpose()?;
    let renderer_cpu = menu.then(|| prepare_renderer_textures(&pack));
    if menu {
        log::info!(
            "Horizon: handoff CPU resources prepared on worker in {} ms",
            prepared_at.elapsed().as_millis()
        );
    }
    // The block entity renderer's face visibility: the meshes' own, whose
    // shapes already hold their faces, not a second load of the same tables.
    let visibility = pack.context.tables.visibility.clone();
    let rest_loader = if menu {
        let resources = resources.clone();
        let root = root.clone();
        let rest_progress = progress.clone();
        let progress = progress.clone();
        let handle = std::thread::Builder::new()
            .name("world-rest-loader".into())
            .spawn(move || {
                let rest = load_world_rest(
                    &root,
                    &resources,
                    &assets,
                    &behaviour_json,
                    &gameplay_projection,
                    visibility,
                    &progress,
                    started,
                    true,
                )
                .map_err(|error| error.to_string())?;
                resources
                    .rest
                    .set(rest)
                    .map_err(|_| "world resources already loaded".to_owned())?;
                progress.set(9);
                horizon_client::telemetry::mark_rest_ready();
                log::info!(
                    "Horizon: rest-ready after {} ms",
                    started.elapsed().as_millis()
                );
                Ok(())
            })?;
        Some(WorldRestLoader {
            progress: rest_progress,
            handle,
        })
    } else {
        let rest = load_world_rest(
            &root,
            &resources,
            &assets,
            &behaviour_json,
            &gameplay_projection,
            visibility,
            progress,
            started,
            !headless,
        )?;
        resources
            .rest
            .set(rest)
            .map_err(|_| "world resources already loaded")?;
        None
    };
    Ok(Loaded {
        game_cpu,
        renderer_cpu,
        root,
        pack_error,
        resources,
        rest_loader,
        pack,
        gui: loaded_gui,
    })
}

fn load_world_rest(
    root: &std::path::Path,
    resources: &Resources,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[horizon_client::blocks::GameplayProjection],
    visibility: horizon_world::block_render::FaceVisibility,
    progress: &Progress,
    started: Instant,
    // False on a dedicated server, which draws nothing: no block entity
    // renderer or animate tables (client-side display ticks).
    draws: bool,
) -> Result<WorldRest> {
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry = assets.registry;
    let overlay = &horizon_world::loot::PackOverlay::NONE;
    progress.set(6);
    // Independent tables load together, within the worker budget the settings
    // allow. Only the recipes and the stations read another table: the items.
    let load_items = || -> Result<_> {
        let items = Items::load(root, registry, facts, overlay)?;
        log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
        progress.set(7);
        let recipes = Recipes::load(root, &items, overlay)?;
        let stations = horizon_client::stations::StationData::load(root, &items, overlay);
        Ok((items, recipes, stations))
    };
    let load_behaviours =
        || Behaviours::load_with_gameplay(registry, root, behaviour_json, gameplay_projection);
    let load_block_entity_render = || {
        if !draws {
            return Ok(Default::default());
        }
        horizon_client::block_entity_render::BlockEntityRenderer::load_with_visibility(
            root, registry, visibility,
        )
    };
    let load_animate = || -> Result<_> {
        let animate = if draws {
            horizon_client::animate_tick::AnimateTables::load(root, registry, biomes)?
        } else {
            Default::default()
        };
        Ok((animate, horizon_client::fog::FogTables::load(root, biomes)))
    };
    let load_mobs = || -> Result<_> {
        let mob_types: HashMap<_, _> = MobTypes::load(root)?
            .types
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect();
        let mob_data = horizon_client::mobs::MobData::load(root, facts);
        horizon_client::mobs::trial_spawner::load_registry(root);
        Ok((mob_types, mob_data))
    };
    let load_transformers = || -> Result<_> {
        let transformers = horizon_client::block_transform::Transformers::load(root, registry)?;
        let models = (
            perspective::player_type(root),
            player_arms(root, "minecraft:player#main"),
            player_arms(root, "minecraft:player_slim#main"),
            horizon_client::mobs::load_model(root, "minecraft:spin_attack#main"),
        );
        Ok((transformers, models))
    };
    // The errors cross threads as text.
    fn text<T, E: std::fmt::Display>(
        result: std::result::Result<T, E>,
    ) -> std::result::Result<T, String> {
        result.map_err(|e| e.to_string())
    }
    let (items, behaviours, block_entity_render, animate, mobs, transformers) = if threads > 1 {
        std::thread::scope(|scope| {
            let items = scope.spawn(|| text(load_items()));
            let behaviours = scope.spawn(|| text(load_behaviours()));
            let block_entity_render = scope.spawn(|| text(load_block_entity_render()));
            let animate = scope.spawn(|| text(load_animate()));
            let mobs = scope.spawn(|| text(load_mobs()));
            let transformers = text(load_transformers());
            (
                items.join().unwrap(),
                behaviours.join().unwrap(),
                block_entity_render.join().unwrap(),
                animate.join().unwrap(),
                mobs.join().unwrap(),
                transformers,
            )
        })
    } else {
        (
            text(load_items()),
            text(load_behaviours()),
            text(load_block_entity_render()),
            text(load_animate()),
            text(load_mobs()),
            text(load_transformers()),
        )
    };
    let (items, recipes, stations) = items?;
    let (animate, fog) = animate?;
    let (mob_types, mob_data) = mobs?;
    let (transformers, (player_type, player_arm, player_arm_slim, spin_attack)) = transformers?;
    log::info!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    Ok(WorldRest {
        stations,
        items,
        recipes,
        transformers,
        providers: Providers::load(root),
        block_entity_render: block_entity_render?,
        animate,
        fog,
        music: horizon_client::music::MusicTables::load(root),
        ambient: horizon_client::ambience::AmbientTables::load(root),
        nether_environment: Environment::load_dimension(root, "the_nether", false)?,
        end_environment: Environment::load_dimension(root, "the_end", false)?,
        behaviours: behaviours?,
        mob_types,
        mob_data,
        player_type,
        player_arm,
        player_arm_slim,
        spin_attack,
    })
}

/// CPU mip chains for the block sprites and world border.
fn prepare_renderer_textures(pack: &PackResources) -> horizon_client::gfx::PreparedTextures {
    let assets = &pack.render;
    horizon_client::gfx::PreparedTextures::new(
        &assets.pixels,
        assets.sprite_size,
        assets.texture_layers,
        &assets.sprite_mips,
        (&pack.forcefield.2, pack.forcefield.0),
    )
    .with_pbr(
        &assets.pbr_normals,
        &assets.pbr_specular,
        assets.pbr_size,
        assets.texture_layers,
    )
}

/// Upload pack textures and create the renderer on the window's device.
fn build_renderer(
    window: Arc<Window>,
    offscreen: bool,
    pack: &PackResources,
    gui: &UiResources,
    previous: Option<&Renderer>,
    prepared: Option<horizon_client::gfx::PreparedTextures>,
    api: horizon_client::graphics::GraphicsApi,
) -> Result<Renderer> {
    let assets = &pack.render;
    let atlas = &gui.atlas;
    // The new renderer's atlas texture starts from `HudAtlas.pixels`, which
    // holds the textures but never a glyph rasterized at run time. The font's
    // cache still answers with the rects it drew into the previous renderer's
    // texture, so forget them: the glyphs that are drawn again are placed into
    // this texture (as `ui_theme` does when its atlas changes). Without this,
    // Chinese and Japanese menu labels go blank when the loading screen hands
    // over to the game.
    atlas.font.reset_cache();
    // Made here when the worker did not: the renderer's own fallback has no LabPBR maps.
    let prepared = Some(prepared.unwrap_or_else(|| prepare_renderer_textures(pack)));
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        offscreen,
        &assets.pixels,
        assets.sprite_size,
        assets.sprite_minimum_size,
        assets.texture_layers,
        &assets.sprite_mips,
        &assets.animations,
        (&atlas.pixels, atlas.width, atlas.height),
        (&atlas.glint.0, atlas.glint.1, atlas.glint.2),
        (
            &atlas.glint_armor.0,
            atlas.glint_armor.1,
            atlas.glint_armor.2,
        ),
        (&pack.forcefield.2, pack.forcefield.0),
        &gui.panorama,
        prepared,
        previous,
        api,
    ))?;
    renderer.hud.sprite_animations = atlas
        .sprite_animations
        .iter()
        .map(|(rect, animation)| (*rect, animation.clone(), (usize::MAX, 0, 0)))
        .collect();
    renderer
        .hud
        .set_text_shader(&renderer.device, atlas.text_shader.as_deref());
    Ok(renderer)
}

/// The loading screen's renderer: one atlas (the boot screen's until the
/// pack's GUI atlas is ready) over one opaque white block-sprite layer, an
/// empty border texture and the title panorama once the menu bundle is ready.
fn loading_renderer(
    window: Arc<Window>,
    offscreen: bool,
    atlas: Option<&horizon_client::hud::HudAtlas>,
    panorama: Option<&horizon_client::gfx::panorama::Faces>,
    previous: Option<&Renderer>,
    api: horizon_client::graphics::GraphicsApi,
) -> Result<Renderer> {
    static FORCEFIELD: [u8; 4] = [0; 4];
    static EMPTY_HUD: [u8; 4] = [0, 0, 0, 255];
    static EMPTY_GLINT: [u8; 4] = [0; 4];
    let empty_panorama = horizon_client::gfx::panorama::Faces {
        size: 1,
        pixels: [0u8; 4].repeat(6),
    };
    let panorama = panorama.unwrap_or(&empty_panorama);
    let (pixels, width, height) = atlas.map_or((EMPTY_HUD.as_slice(), 1, 1), |a| {
        (a.pixels.as_slice(), a.width, a.height)
    });
    let (glint_pixels, glint_width, glint_height) = atlas
        .map_or((EMPTY_GLINT.as_slice(), 1, 1), |a| {
            (a.glint.0.as_slice(), a.glint.1, a.glint.2)
        });
    // A glyph rasterized into an earlier texture is not in these pixels; the
    // new texture gets it again when the character is drawn (see
    // `build_renderer`).
    if let Some(atlas) = atlas {
        atlas.font.reset_cache();
    }
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        offscreen,
        &[255, 255, 255, 255],
        1,
        1,
        1,
        &[],
        &[],
        (pixels, width, height),
        (glint_pixels, glint_width, glint_height),
        (glint_pixels, glint_width, glint_height),
        (&FORCEFIELD[..], 1),
        panorama,
        None,
        previous,
        api,
    ))?;
    if let Some(atlas) = atlas {
        renderer.hud.sprite_animations = atlas
            .sprite_animations
            .iter()
            .map(|(rect, animation)| (*rect, animation.clone(), (usize::MAX, 0, 0)))
            .collect();
    }
    Ok(renderer)
}

/// A title-shaped frame: black clear and panorama, only the HUD is drawn.
/// The window's `HWND`, for the boot screen's GDI paint (`horizon_threads`):
/// the raw handle read is safe, the paint is not.
#[cfg(target_os = "windows")]
fn hwnd(window: &Window) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match window.window_handle() {
        Ok(raw) => match raw.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        },
        Err(_) => None,
    }
}

fn loading_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: horizon_client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        eye: glam::DVec3::new(0.5, 100.0, 0.5),
        eye_rotation: [0.0, 10.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: horizon_client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        water_plane: None,
        cave_culling: false,
        title: true,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        entity_spans: &[],
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        shadow_only: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        chunk_fade: 0.0,
        post_effects: horizon_client::post_chain::Request::none(),
        menu_blur: None,
    }
}

/// The post-load setup once the resources are ready: `Game::new` and the
/// options, packs, mods, control socket, benchmark and world selection the
/// old `run` did after loading. `None` means a harness or dedicated run
/// finished here instead.
#[allow(clippy::too_many_arguments)]
fn start_game<'a>(
    res: &'a Arc<Resources>,
    pack: PackResources,
    gui: UiResources,
    options: Options,
    pack_error: Option<String>,
    audio: Audio,
    vanilla_root: PathBuf,
    started: Instant,
    rest_loader: Option<WorldRestLoader>,
    settings: Option<Settings>,
    prepared: Option<GameCpuResources>,
) -> Result<Option<Game<'a>>> {
    let handoff = prepared.is_some();
    let mut settings = match settings {
        Some(settings) => settings,
        None => Settings::load(&std::env::current_dir()?.join("options.txt")),
    };
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    let mut game = Game::new(res, pack, gui, options, settings, audio, prepared)?;
    game.rest_loader = rest_loader;
    game.ui.vanilla_root = vanilla_root;
    if let Some(error) = pack_error {
        // Minecraft.clearResourcePacksOnError: the selection is cleared and
        // saved, so the next launch starts without the packs.
        game.settings.resource_packs.clear();
        game.settings.save(&Game::options_path());
        game.open_options_at(true, horizon_client::options_ui::Page::Packs);
        game.ui.options.resource_error = Some(error);
    }
    if game.options.benchmark.is_some() {
        // The same conditions on every machine: no VSync, no frame cap, no
        // pause or idle limit without focus. In memory only, never saved.
        game.settings.vsync = false;
        game.settings.max_fps = 260;
        game.settings.inactivity_fps_limit = "minimized".into();
        game.options.no_pause = true;
    }
    game.apply_settings();
    // Benchmarks measure the game alone.
    if game.options.benchmark.is_none() {
        game.mods.dev = game.options.dev_mods.clone();
        game.start_mods();
    }
    game.start_agent_control()?;
    if let Some(scenario) = game.options.benchmark.clone() {
        let control = game
            .control
            .get_or_insert_with(horizon_client::control::Server::local);
        horizon_client::benchmark::start(
            control.sender(),
            horizon_client::benchmark::Setup {
                scenario,
                started,
                app: horizon_bench::App {
                    name: "horizon-client".into(),
                    version: horizon_client::crash::version(),
                    commit: option_env!("HORIZON_COMMIT").map(str::to_owned),
                    mods: Vec::new(),
                },
                simulation_distance: game.settings.simulation_distance as u32,
                vsync: false,
                fps_cap: None,
                offscreen: game.options.offscreen,
                game_dir: std::env::current_dir()?,
            },
        );
    }
    if let Ok(spec) = std::env::var("HORIZON_DISPENSE_FIXTURE") {
        game.run_component_harness(integrated::ComponentHarness::Dispenser(spec))?;
        return Ok(None);
    }
    if let Ok(spec) = std::env::var("HORIZON_SCULK_FIXTURE") {
        game.run_component_harness(integrated::ComponentHarness::Sculk(spec))?;
        return Ok(None);
    }
    if game.options.execution_role == execution_role::ExecutionRole::DedicatedServer {
        game.run_dedicated()?;
        return Ok(None);
    }
    match game.options.world.clone() {
        _ if game.options.server.is_some() => {
            let (address, name) = (
                game.options.server.clone().unwrap_or_default(),
                game.options.player_name(),
            );
            game.connect_server(&address, &name)?;
            if let Some(report) = std::env::var_os("HORIZON_NET_REPORT") {
                game.run_net_harness(report.as_ref())?;
                return Ok(None);
            }
        }
        Some(folder) => {
            let (seed, creative) = (game.options.seed, game.options.creative);
            if std::env::var_os("HORIZON_COMMAND_SCRIPT").is_some() {
                // Headless, but with the same server and memory connection as play.
                game.open_integrated(&folder, None, Some(seed), creative, None)?;
            } else {
                // Start after the window is handed over, so --world uses the
                // same responsive initialization path as the title screen.
                game.title = Some(
                    Title::new(
                        std::env::current_dir()?.join("saves"),
                        &game.options.resources,
                    )
                    .with_user(|| game.options.player_name()),
                );
                game.pending_entry = Some(PendingWorldEntry::Local {
                    folder,
                    name: None,
                    seed: Some(seed),
                    creative,
                    create: None,
                });
            }
        }
        None if handoff => {}
        None => {
            let mut title = Title::new(
                std::env::current_dir()?.join("saves"),
                &game.options.resources,
            )
            .with_user(|| game.options.player_name());
            title.last_server = game.settings.file.string("lastServer");
            title.skip_multiplayer_warning = game.settings.file.bool("skipMultiplayerWarning");
            // Gui.addInitialScreens: the accessibility onboarding comes first.
            if game.settings.file.bool("onboardAccessibility") {
                title.start_onboarding();
            }
            game.title = Some(title);
        }
    }
    if std::env::var_os("HORIZON_COMMAND_SCRIPT").is_some()
        && std::env::var_os("HORIZON_COMMAND_OUTPUT").is_some()
    {
        game.wait_command_harness()?;
        return Ok(None);
    }
    Ok(Some(game))
}

fn write_report(game: &Game<'_>) -> Result<()> {
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.frames,
            "elapsed_seconds": game.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.streaming.generated_total,
            "meshed_sections": game.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error.as_ref().map(|e| e.to_string()),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// The window system ended the event loop with a failure (on Linux, a compositor dropping the
/// connection after a protocol error) while fullscreen was on: the next start opens windowed, so a
/// fullscreen the system can't show doesn't crash every launch (#2904).
fn leave_fullscreen_after(
    result: &std::result::Result<(), EventLoopError>,
    settings: &mut Settings,
) {
    if matches!(result, Err(EventLoopError::ExitFailure(_))) && settings.fullscreen {
        settings.fullscreen = false;
        settings.save(&Game::options_path());
        log::warn!(
            "the window system closed the game in fullscreen; the next start opens windowed"
        );
    }
}

fn run_event_loop(mut game: Game<'_>) -> Result<()> {
    if game.options.headless {
        // No display to open winit's loop on: the game's own loop.
        game.run_headless();
        // narrator::start ran for it too (options.txt's narrator setting).
        narrator::stop();
        write_report(&game)?;
        return game.error.take().map_or(Ok(()), Err);
    }
    let mut split = split::Split::new(game);
    let result = EventLoop::new()?.run_app(&mut split);
    game = split.host;
    narrator::stop();
    leave_fullscreen_after(&result, &mut game.settings);
    result?;
    horizon_client::discord_presence::stop();
    write_report(&game)?;
    if let Some(e) = game.error.take() {
        return Err(e);
    }
    Ok(())
}

/// The loading screen's state while the worker loads the resources.
struct Loading {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// The boot screen's atlas, until the pack's GUI atlas replaces it.
    boot: horizon_client::hud::HudAtlas,
    gui: Option<UiResources>,
    settings: Settings,
    fullscreen: horizon_client::window::FullscreenState,
    video_modes: horizon_client::window::VideoModeList,
    options: Option<Options>,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
    panorama_rotation: horizon_client::gfx::panorama::Rotation,
    progress: Progress,
    receiver: Receiver<LoaderMsg>,
    loaded: Option<std::result::Result<Loaded, String>>,
    /// The loader's `Done` arrived: its channel closing afterwards is expected.
    delivered: bool,
    handle: Option<JoinHandle<()>>,
    shown: bool,
    menu: startup_menu::StartupMenu,
    error: Option<Box<dyn std::error::Error>>,
}
impl Loading {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen =
            self.settings.fullscreen || self.options.as_ref().is_some_and(|o| o.fullscreen);
        let size = self
            .options
            .as_ref()
            .map_or([1280, 720], |o| o.window_size)
            .map(f64::from);
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon());
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "horizon", "horizon",
        );
        // `--offscreen` renders into a texture: its window would only show white.
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        horizon_client::telemetry::note_display(&window);
        // The renderer takes seconds to build and winit's window sits white
        // until the first frame presents, so paint the boot screen on it and
        // keep repainting it as the console's lines arrive; the wgpu frames
        // redraw the same design when they start. GDI is unsafe:
        // horizon-threads holds it (this crate forbids unsafe code).
        #[cfg(target_os = "windows")]
        let boot_paint_stop = hwnd(&window).map(|hwnd| {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let repaint_stop = stop.clone();
            std::thread::spawn(move || {
                while !repaint_stop.load(std::sync::atomic::Ordering::Relaxed) {
                    horizon_threads::boot_paint::paint(
                        hwnd,
                        horizon_theme::fonts::GEIST_SEMIBOLD,
                        horizon_theme::fonts::GEIST_MONO,
                        &horizon_client::logging::recent_lines(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            });
            stop
        });
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        match loading_renderer(
            window.clone(),
            offscreen,
            Some(&self.boot),
            None,
            None,
            self.settings.horizon.api,
        ) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                #[cfg(target_os = "windows")]
                if let Some(stop) = &boot_paint_stop {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        // The wgpu frames take the window over: stop the boot paint thread
        // before the first one presents.
        #[cfg(target_os = "windows")]
        if let Some(stop) = &boot_paint_stop {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        horizon_client::hud::GUI_SCALE.store(self.settings.gui_scale, Ordering::Relaxed);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, mut event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        follow_keymap(&mut event);
        if let Err(error) = self.menu_event(event_loop, event) {
            self.error = Some(error);
            event_loop.exit();
        }
    }
    /// The menu bundle arrived: rebuild the loading screen with the real atlas
    /// and title panorama.
    fn set_ui(&mut self, gui: UiResources, root: PathBuf) {
        log::info!(
            "Horizon: loading the title artwork after {} ms",
            self.started.elapsed().as_millis()
        );
        self.menu.set_root(root, &self.settings);
        self.gui = Some(gui);
        horizon_client::ui_theme::reset_atlas();
        self.apply_menu_settings();
        let Some(window) = self.window.clone() else {
            return;
        };
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let bundle = self.gui.as_ref().expect("the menu bundle was just set");
        match loading_renderer(
            window,
            offscreen,
            Some(&bundle.atlas),
            Some(bundle.panorama.as_ref()),
            self.renderer.as_ref(),
            self.settings.horizon.api,
        ) {
            Ok(r) => {
                horizon_client::hud::GUI_SCALE.store(self.settings.gui_scale, Ordering::Relaxed);
                self.renderer = Some(r);
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The loop may run once more after exit(): keep the first error, the
        // one that ended loading.
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        if horizon_threads::stop_requests() > 0 {
            // No world is open yet: nothing to save.
            event_loop.exit();
            return;
        }
        if let Err(error) = self.poll_menu_resources() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.options.as_ref().is_some_and(|o| o.offscreen) {
                self.draw();
            } else {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    fn draw(&mut self) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let mut batch = HudBatch::new(size);
        self.draw_menu(&mut batch);
        self.menu.frame_drawn((batch.width, batch.height));
        let mut frame = loading_frame(&batch);
        frame.yaw = self.panorama_rotation.update(
            self.started.elapsed().as_secs_f32(),
            self.settings.file.double("panoramaScrollSpeed") as f32,
        );
        if let Some(renderer) = &mut self.renderer {
            match renderer.render(&frame) {
                Ok(true) if self.gui.is_some() && self.menu.pending.is_none() => {
                    self.menu.perf.title_shown()
                }
                Ok(_) => {}
                Err(e) => log::warn!("Horizon: loading screen frame failed: {e}"),
            }
        }
        if !self.shown {
            self.shown = true;
            log::info!(
                "Horizon: loading screen shown after {} ms",
                self.started.elapsed().as_millis()
            );
        }
    }
    fn finish(&mut self, loaded: Loaded) -> Result<Option<split::Split<'static>>> {
        let handoff_at = Instant::now();
        let mut options = self.options.take().ok_or("loading already finished")?;
        options.resources = loaded.root.clone();
        // StartupMenu owns world selection until it can hand it to Game.
        options.world = None;
        options.server = None;
        let gui = self.gui.take().ok_or("missing menu bundle")?;
        let window = self.window.clone().ok_or("window closed")?;
        // Keep the loader's device and swap in the real textures in one step.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer_at = Instant::now();
        let renderer = build_renderer(
            window.clone(),
            options.offscreen,
            &loaded.pack,
            &gui,
            self.renderer.as_ref(),
            loaded.renderer_cpu,
            self.settings.horizon.api,
        )?;
        log::info!(
            "Horizon: world renderer ready after {} ms",
            self.started.elapsed().as_millis()
        );
        let renderer_ms = renderer_at.elapsed().as_millis();
        // Audio is created on the main thread, after the core is ready.
        let audio = if !options.execution_role.is_client() {
            Audio::silent(&options.resources)
        } else {
            Audio::with_device(&options.resources, &self.settings.sound_device)
        };
        let pack_error = loaded.pack_error.or_else(|| self.pack_error.take());
        // The game borrows its shared resources for the process's lifetime,
        // like the single copy an integrated server would share.
        let res: &'static Arc<Resources> = Box::leak(Box::new(loaded.resources));
        let game_at = Instant::now();
        let Some(mut game) = start_game(
            res,
            loaded.pack,
            gui,
            options,
            pack_error,
            audio,
            self.vanilla_root.clone(),
            self.started,
            loaded.rest_loader,
            Some(self.settings.clone()),
            loaded.game_cpu,
        )?
        else {
            return Ok(None);
        };
        let game_ms = game_at.elapsed().as_millis();
        game.window = Some(window);
        game.renderer = Some(renderer);
        game.world_renderer_ready = true;
        game.fullscreen = self.fullscreen;
        game.panorama_rotation = self.panorama_rotation;
        self.menu.handoff(&mut game);
        game.apply_settings();
        game.previous = Instant::now();
        log::info!(
            "Horizon: startup handoff {} ms (renderer {} ms, start_game {} ms)",
            handoff_at.elapsed().as_millis(),
            renderer_ms,
            game_ms
        );
        Ok(Some(split::Split::new(game)))
    }
}

/// The application: the loading screen until the resources are ready, then
/// the game itself, on the same event loop and window.
enum App {
    Loading(Box<Loading>),
    Ready(Box<split::Split<'static>>),
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Loading(loading) => loading.resumed(event_loop),
            App::Ready(game) => game.resumed(event_loop),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {
            App::Loading(loading) => loading.window_event(event_loop, id, event),
            App::Ready(game) => game.window_event(event_loop, id, event),
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if let App::Ready(game) = self {
            game.device_event(event_loop, id, event);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enum Step {
            Waiting,
            Ready(Box<split::Split<'static>>),
            Done,
            Failed(Box<dyn std::error::Error>),
        }
        let step = match self {
            App::Ready(game) => {
                game.about_to_wait(event_loop);
                return;
            }
            App::Loading(loading) => {
                loading.about_to_wait(event_loop);
                if loading.error.is_some() {
                    event_loop.exit();
                    return;
                }
                match if loading.menu.reload_packs {
                    None
                } else {
                    loading.loaded.take()
                } {
                    None => Step::Waiting,
                    Some(Err(e)) => Step::Failed(e.into()),
                    Some(Ok(loaded)) => match loading.finish(loaded) {
                        Ok(Some(game)) => Step::Ready(Box::new(game)),
                        Ok(None) => Step::Done,
                        Err(e) => Step::Failed(e),
                    },
                }
            }
        };
        match step {
            Step::Waiting => {}
            Step::Ready(game) => *self = App::Ready(game),
            Step::Done => event_loop.exit(),
            Step::Failed(e) => {
                if let App::Loading(loading) = self {
                    loading.error = Some(e);
                }
                event_loop.exit();
            }
        }
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Ready(game) = self {
            game.exiting(event_loop);
        }
    }
}

/// Every launch reads the prepared game files (`horizon-prepare`'s output, whose
/// manifest is `horizon-resources.json`): without them, say where they were
/// looked for and how to get them, before a window or a loader starts.
fn missing_resources(root: &std::path::Path) -> Option<String> {
    if root.join("horizon-resources.json").is_file() {
        return None;
    }
    let shown = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    Some(format!(
        "Horizon's prepared game files are missing: no horizon-resources.json in {}\n\
         Horizon is started from its launcher: download it at {} (Download). It needs your \
         own Minecraft: Java Edition; the launcher signs you in and prepares the game files.\n\
         Developers: horizon-prepare --prepare, or pass --resources <folder>",
        shown.display(),
        horizon_client::no_resources::DOWNLOAD_URL
    ))
}

/// The ordinary launch: load the resources on a worker while the window shows
/// the loading screen, then swap in the game on the main thread.
fn run_loading(
    options: Options,
    settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
) -> Result<()> {
    let progress = Progress::new();
    let (receiver, loader) =
        startup_menu::spawn_loader(&options, &settings, &vanilla_root, started, &progress)?;
    let menu = startup_menu::StartupMenu::new(&options, pack_error.as_deref())?;
    let loading = Loading {
        window: None,
        renderer: None,
        boot: horizon_client::hud::HudAtlas::boot(),
        gui: None,
        settings,
        fullscreen: horizon_client::window::FullscreenState::new(false),
        video_modes: Default::default(),
        options: Some(options),
        vanilla_root,
        pack_error,
        started,
        panorama_rotation: Default::default(),
        progress,
        receiver,
        loaded: None,
        delivered: false,
        handle: Some(loader),
        shown: false,
        menu,
        error: None,
    };
    let mut app = App::Loading(Box::new(loading));
    let result = EventLoop::new()?.run_app(&mut app);
    narrator::stop();
    match &mut app {
        // Ready: splitscreen around P1's game.
        App::Ready(split) => leave_fullscreen_after(&result, &mut split.host.settings),
        App::Loading(loading) => leave_fullscreen_after(&result, &mut loading.settings),
    }
    result?;
    horizon_client::discord_presence::stop();
    match app {
        App::Ready(mut split) => {
            write_report(&split.host)?;
            if let Some(e) = split.host.error.take() {
                return Err(e);
            }
        }
        App::Loading(loading) => {
            if let Some(e) = loading.error {
                return Err(e);
            }
        }
    }
    Ok(())
}

pub use launch_api::{UsageError, exit_code};

pub fn run() -> Result<()> {
    let Some(mut options) = Options::read()? else {
        return Ok(());
    };
    if options.execution_role == execution_role::ExecutionRole::DedicatedServer
        && !dedicated::preflight(&mut options)?
    {
        return Ok(());
    }
    let _ = horizon_client::health::HURT_SERVER.set(native_hooks::hurt_server);
    let _ = horizon_client::block_contact::HOOK.set(native_hooks::block_contact);
    // Before any worker starts: on Linux they go behind the server thread
    // and the render thread.
    horizon_threads::put_helpers_behind();
    if options.execution_role.is_client() {
        // The render thread, like the server thread, ahead of the workers on
        // macOS and Windows. While terrain loads, the worker pools (two with
        // single player's server thread) and other programs outnumber the
        // cores, and a frame that gave its core up (in a GPU submit) waited
        // 10 to 200 ms to get one back.
        horizon_threads::favour_current();
    }
    if options.execution_role.is_client() {
        horizon_threads::catch_stop_signals();
    }
    let started = Instant::now();
    horizon_client::spans::enable(options.spikes.is_some());
    let vanilla_root = options.resources.clone();
    if let Some(error) = missing_resources(&vanilla_root) {
        // The game build started by hand (the release's zip), not by the
        // launcher: a window says where to get it. Scripts and servers get
        // the error.
        if !options.offscreen && !synchronous_startup(&options) {
            log::error!("Horizon: {error}");
            let settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
            return no_resources::show(&vanilla_root, &settings);
        }
        return Err(error.into());
    }
    // Someone is playing: the anonymous ping behind the launcher's "N playing now", and the
    // official server in their list (servers.dat in the game folder, the working directory). Not
    // for a server, a benchmark, a run that quits by itself, a harness's fixture or a run that
    // horizon-ctl drives: those start in a source checkout and wrote the list there.
    if !options.offscreen && !synchronous_startup(&options) {
        horizon_client::heartbeat::start();
        if let Err(e) = horizon_client::multiplayer::add_official_server(&std::env::current_dir()?)
        {
            log::warn!("could not add the official server to servers.dat: {e}");
        }
    }
    mods::register_components(&vanilla_root)?;
    let early_settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    // Minecraft's constructor opens the narrator; a server has none.
    if options.execution_role.is_client() {
        narrator::start(&early_settings);
    }
    // "Playing Horizon" on Discord: never for a server; an automated run only talks to a test's fake Discord.
    if options.dedicated.is_none() {
        let automated = options.benchmark.is_some()
            || options.offscreen
            || options.frames.is_some()
            || options.world_frames.is_some();
        horizon_client::discord_presence::start(early_settings.extras.discord_presence, automated);
    }
    // Snapshots of derived tables live in the game folder, never beside the
    // shared resources.
    Behaviours::set_cache_dir(std::env::current_dir()?.join(".horizon-cache"));
    if !synchronous_startup(&options) {
        return run_loading(options, early_settings, vanilla_root, None, started);
    }
    let mut pack_error = None;
    options.resources = packs::startup_root(&vanilla_root, &early_settings.resource_packs, &[])
        .unwrap_or_else(|e| {
            log::warn!("Horizon: resource packs not applied: {e}");
            pack_error = Some(e.to_string());
            vanilla_root.clone()
        });
    options.resources = mods::content_root(
        &options.resources,
        &early_settings.mods.enabled,
        &options.dev_mods,
    );
    let loaded = load_resources(
        options.resources.clone(),
        launch_world(&options),
        &vanilla_root,
        &early_settings,
        started,
        false,
        options.dedicated.is_some(),
        &Progress::new(),
        &mut |_, _| {},
    )?;
    let gui = loaded.gui.ok_or("missing menu bundle")?;
    options.resources = loaded.root.clone();
    let audio = if !options.execution_role.is_client() || options.headless {
        Audio::silent(&options.resources)
    } else {
        Audio::with_device(&options.resources, &early_settings.sound_device)
    };
    let resources = loaded.resources;
    let game = start_game(
        &resources,
        loaded.pack,
        gui,
        options,
        loaded.pack_error.or(pack_error),
        audio,
        vanilla_root,
        started,
        loaded.rest_loader,
        None,
        None,
    )?;
    if let Some(game) = game {
        run_event_loop(game)?;
    }
    Ok(())
}

/// The right and left arms of a player model layer (`PlayerModel.rightArm`
/// and `leftArm`, with their sleeves).
fn player_arms(root: &std::path::Path, key: &str) -> [Option<horizon_client::mobs::Part>; 2] {
    let model = horizon_client::mobs::load_model(root, key);
    ["right_arm", "left_arm"].map(|name| {
        model
            .as_ref()
            .and_then(|m| m.children.iter().find(|c| c.name == name).cloned())
    })
}

// Player.tick: cap the sleeping fade at 100, then fade out over 10 waking ticks.
fn tick_sleep_timer(timer: u32, sleeping: bool) -> u32 {
    if sleeping {
        (timer + 1).min(100)
    } else if (1..109).contains(&timer) {
        timer + 1
    } else {
        0
    }
}

/// `KeyboardHandler.keyPress` for a key bound to a mouse mapping (key.attack,
/// key.use, key.pickItem): in game, a press and every repeat of the held key
/// reach `KeyMapping.set(key, true)` and `KeyMapping.click` alike, so a held
/// key keeps attacking; a release lets go. `None` behind a screen, which takes
/// the key.
fn keyboard_button(playing: bool, action: KeyAction) -> Option<bool> {
    playing.then_some(action != KeyAction::Release)
}

/// Gameplay owns the keyboard: `Screen::Playing`, no title, no server dialog.
/// Vanilla's `DialogScreen` is a screen, so `KeyMapping.releaseAll` runs.
fn gameplay_has_keyboard(screen: Screen, title: bool, dialog: bool) -> bool {
    screen == Screen::Playing && !title && !dialog
}

/// A dialog drops held keys and mouse buttons it does not see (#4974).
fn drop_held_behind_dialog(
    dialog: bool,
    keys: &mut HashSet<KeyCode>,
    left: &mut bool,
    right: &mut bool,
) {
    if dialog {
        keys.clear();
        *left = false;
        *right = false;
    }
}

/// `AbstractContainerScreen.keyPressed` for a key bound to a mouse mapping: a press or repeat of
/// key.pickItem's key clones the hovered slot; keys bound to key.attack and key.use do nothing
/// there (#5378).
fn container_key_clones(button: MouseButton, action: KeyAction) -> bool {
    button == MouseButton::Middle && action != KeyAction::Release
}

/// `VoxelShape.clip` of a fluid source's shape for a ray that starts in the
/// source's own block `cell`: the face and point it hits, in `pick_fluid`'s
/// `Hit` terms. `FlowingFluid.getShape` is the whole block under the same
/// fluid and `getOwnHeight` (8/9) tall otherwise. A start inside the shape
/// (tested 1e-7 of the way along) hits at the start, on the face opposite
/// `Direction.getApproximateNearest(dir)`; from above it, a ray going down
/// enters through the top while still in this block.
fn clip_start_source(
    eye: [f64; 3],
    dir: [f64; 3],
    reach: f64,
    cell: [i32; 3],
    same_above: bool,
) -> Option<([i32; 3], [f64; 3])> {
    let height = if same_above {
        1.0
    } else {
        f64::from(8.0f32 / 9.0f32)
    };
    let probe_y = eye[1] + dir[1] * reach * 1.0e-7;
    if probe_y - f64::from(cell[1]) < height {
        // Direction.values() order; the first of equal dot products wins.
        let normals: [[i32; 3]; 6] = [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ];
        let mut nearest = normals[2];
        let mut best = f64::NEG_INFINITY;
        for n in normals {
            let dot =
                f64::from(n[0]) * dir[0] + f64::from(n[1]) * dir[1] + f64::from(n[2]) * dir[2];
            if dot > best {
                best = dot;
                nearest = n;
            }
        }
        return Some((nearest.map(|v| -v), eye));
    }
    if dir[1] >= 0.0 {
        return None;
    }
    let t = (f64::from(cell[1]) + height - eye[1]) / dir[1];
    if t > reach {
        return None;
    }
    let point = [
        eye[0] + dir[0] * t,
        eye[1] + dir[1] * t,
        eye[2] + dir[2] * t,
    ];
    (point[0].floor() as i32 == cell[0] && point[2].floor() as i32 == cell[2])
        .then_some(([0, 1, 0], point))
}

fn reset_world_progress(adv: &mut advancements::AdvState, stats: &mut stats::Stats) {
    *adv = Default::default();
    *stats = Default::default();
}

#[cfg(test)]
mod startup_cpu_tests {
    use super::*;

    #[test]
    fn a_folder_without_prepared_files_is_named_before_loading() {
        let root = std::env::temp_dir().join(format!("horizon-unprepared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let missing = missing_resources(&root.join(".local/horizon-client/26.3"))
            .expect("no manifest is reported");
        assert!(missing.contains("horizon-resources.json"), "{missing}");
        assert!(missing.contains("horizon-unprepared-"), "{missing}");
        assert!(missing.contains("launcher"), "{missing}");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("horizon-resources.json"), "{}").unwrap();
        assert_eq!(missing_resources(&root), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn worker_prepares_owned_models_and_inventory_before_handoff() {
        let root = std::env::temp_dir().join(format!("horizon-startup-cpu-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("entity-models.json"),
            r#"{
            "minecraft:end_crystal#main": {}, "minecraft:ender_dragon#main": {},
            "minecraft:arrow#main": {}, "minecraft:trident#main": {},
            "minecraft:armor_stand#main": {}, "minecraft:cushion#main": {}
        }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("creative-tabs.json"),
            r#"[{
            "id":"minecraft:building_blocks", "items":["minecraft:stone"]
        }]"#,
        )
        .unwrap();
        let worker_root = root.clone();
        let prepared = std::thread::spawn(move || {
            GameCpuResources::load(&worker_root).map_err(|error| error.to_string())
        })
        .join()
        .unwrap()
        .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(prepared.crystal_model.is_some());
        assert!(prepared.cushion_model.is_some());
        assert!(prepared.dragon_model.is_some());
        assert!(prepared.arrow_model.is_some());
        assert!(prepared.trident_model.is_some());
        assert!(prepared.armor_stand_assets.main.is_some());
        assert_eq!(
            prepared.creative_screen.tabs[0].items[0].item.as_ref(),
            "minecraft:stone"
        );
        // No documents from the old generation remain available to reload.
        assert!(GameCpuResources::load(&root).is_err());
    }
}

#[cfg(test)]
mod runtime_reset_tests {
    use super::*;
    #[test]
    fn world_entry_invalidates_loaded_progress() {
        let mut adv = advancements::AdvState::default();
        let mut stats = stats::Stats::default();
        adv.loaded = true;
        stats.loaded = true;
        adv.progress
            .criteria
            .insert("minecraft:story/root".into(), Default::default());
        stats.add("mined", "minecraft:stone", 4);
        reset_world_progress(&mut adv, &mut stats);
        assert!(!adv.loaded && !stats.loaded);
        assert!(adv.progress.criteria.is_empty());
        assert!(stats.values.is_empty());
    }
}

#[cfg(test)]
mod entity_model_loading_tests;

/// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners (aiStep, unless a spectator or a
/// passenger): a corner inside a suffocating block sets a 0.1 speed towards the nearest side that is
/// free, on that side's axis; a later corner overwrites an earlier one. Suffocating is a full
/// collision cube that is also a full render cube, as `in_wall` has it.
pub(crate) fn closest_space_nudge(
    p: &PlayerMotion,
    world: &World,
    facts: &BlockFacts,
) -> [Option<f64>; 2] {
    use horizon_core::geometry::Axis;
    let mut out = [None; 2];
    let body = p.bounds();
    let (min_y, max_y) = (body.min(Axis::Y) + 1.0e-7, body.max(Axis::Y) - 1.0e-7);
    let suffocates_at = |x: i32, z: i32| {
        ((min_y.floor() as i32 - 1)..=(max_y.ceil() as i32 + 1)).any(|y| {
            f64::from(y) < max_y && f64::from(y + 1) > min_y && {
                let f = facts.get(world.block(x, y, z));
                f.full_collision && f.solid_render
            }
        })
    };
    let side = f64::from(p.dimensions.width) * 0.35;
    let (px, pz) = (p.position.x, p.position.z);
    for (x, z) in [
        (px - side, pz + side),
        (px - side, pz - side),
        (px + side, pz - side),
        (px + side, pz + side),
    ] {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        if !suffocates_at(bx, bz) {
            continue;
        }
        let (xd, zd) = (x - f64::from(bx), z - f64::from(bz));
        // WEST, EAST, NORTH, SOUTH: (axis, step, distance to that edge).
        let mut closest = 1.797_693_134_862_315_7e308; // Double.MAX_VALUE
        let mut chosen = None;
        for (axis, step, distance) in [(0, -1, xd), (0, 1, 1.0 - xd), (1, -1, zd), (1, 1, 1.0 - zd)]
        {
            let (nx, nz) = if axis == 0 {
                (bx + step, bz)
            } else {
                (bx, bz + step)
            };
            if distance < closest && !suffocates_at(nx, nz) {
                closest = distance;
                chosen = Some((axis, step));
            }
        }
        if let Some((axis, step)) = chosen {
            out[axis] = Some(0.1 * f64::from(step));
        }
    }
    out
}

/// GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown, and only over
/// a level. The F3 overlay stays on when the world closes, so it must not reach the panorama.
fn debug_crosshair_drawn(
    enabled: bool,
    first_person: bool,
    hide_gui: bool,
    panorama: bool,
) -> bool {
    enabled && first_person && !hide_gui && !panorama
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::container_screen::{Button, Clicks, Release, Target};

    /// `NativeScreenView.id` (the `screen_render_post` hook) is a mod's
    /// contract: every `Screen` has its own id, and it does not change.
    /// `Screen::id`'s match makes a new variant give itself one; this list
    /// makes the test cover it too.
    #[test]
    fn screen_ids_are_distinct_and_stable() {
        use std::collections::BTreeSet;
        let all = [
            Screen::Playing,
            Screen::GameModeSwitcher,
            Screen::Paused,
            Screen::Options,
            Screen::Chat,
            Screen::Creative,
            Screen::Inventory,
            Screen::Dead,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
            Screen::Friends,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::ServerLinks,
            Screen::ConfirmLink,
            Screen::DialogWarning,
            Screen::Win,
            Screen::DebugOptions,
        ];
        let ids: Vec<&'static str> = all.iter().map(|screen| screen.id()).collect();
        let unique: BTreeSet<&'static str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "two screens share an id");
        for id in &ids {
            assert!(!id.is_empty(), "an empty screen id");
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{id} is not a snake_case id"
            );
        }
        // The ones a mod can match on in the hook's docs.
        assert_eq!(Screen::Playing.id(), "playing");
        assert_eq!(Screen::Inventory.id(), "inventory");
        assert_eq!(Screen::Win.id(), "win");
    }

    /// The `modifiers` a native mod's input hooks hand over are GLFW's bits
    /// (`GLFW_MOD_SHIFT` 1, `GLFW_MOD_CONTROL` 2, `GLFW_MOD_ALT` 4,
    /// `GLFW_MOD_SUPER` 8), each on its own and all together.
    #[test]
    fn glfw_modifier_bits_are_glfw_modifier_bits() {
        use winit::keyboard::ModifiersState;
        for (state, bits) in [
            (ModifiersState::empty(), 0),
            (ModifiersState::SHIFT, 1),
            (ModifiersState::CONTROL, 2),
            (ModifiersState::ALT, 4),
            (ModifiersState::SUPER, 8),
            (ModifiersState::SHIFT | ModifiersState::ALT, 5),
            (ModifiersState::CONTROL | ModifiersState::SUPER, 10),
            (ModifiersState::SHIFT | ModifiersState::CONTROL, 3),
            (
                ModifiersState::SHIFT
                    | ModifiersState::CONTROL
                    | ModifiersState::ALT
                    | ModifiersState::SUPER,
                15,
            ),
        ] {
            assert_eq!(glfw_modifiers(state), bits, "{state:?}");
        }
        // Every modifier GLFW has a bit for, and no other.
        assert_eq!(glfw_modifiers(ModifiersState::all()), 15);
    }

    /// The `button` a native mod's mouse hooks hand over is GLFW's index (0
    /// left, 1 right, 2 middle, 3 back, 4 forward, else the button's number),
    /// the one `keybinds::mouse_name` names (`key.mouse.<index + 1>` from the
    /// back button on).
    #[test]
    fn glfw_button_indices_are_glfw_indices() {
        for (button, index, name) in [
            (MouseButton::Left, 0, "key.mouse.left"),
            (MouseButton::Right, 1, "key.mouse.right"),
            (MouseButton::Middle, 2, "key.mouse.middle"),
            (MouseButton::Back, 3, "key.mouse.4"),
            (MouseButton::Forward, 4, "key.mouse.5"),
        ] {
            assert_eq!(glfw_button(button), index);
            assert_eq!(horizon_client::keybinds::mouse_name(button), name);
        }
        for n in 5..12u16 {
            assert_eq!(glfw_button(MouseButton::Other(n)), i32::from(n));
            assert_eq!(
                horizon_client::keybinds::mouse_name(MouseButton::Other(n)),
                format!("key.mouse.{}", n + 1)
            );
        }
    }

    /// F3 left on when leaving a world kept the axes on the title screen.
    #[test]
    fn the_debug_crosshair_is_gone_once_the_world_is() {
        assert!(debug_crosshair_drawn(true, true, false, false));
        assert!(!debug_crosshair_drawn(true, true, false, true));
    }

    /// Snuffing a candle or a candle cake draws one smoke per
    /// `getParticleOffsets`: by `candles` for a candle, a block up for a cake.
    #[test]
    fn a_snuffed_candle_and_candle_cake_smoke_at_vanillas_offsets() {
        assert_eq!(
            Game::candle_offsets("minecraft:candle", Some(1)),
            [[8u8, 8, 8]].as_slice()
        );
        assert_eq!(
            Game::candle_offsets("minecraft:red_candle", Some(3)),
            [[8u8, 5, 10], [6, 7, 8], [9, 8, 7]].as_slice()
        );
        assert_eq!(
            Game::candle_offsets("minecraft:candle_cake", None),
            [[8u8, 16, 8]].as_slice()
        );
    }

    /// Held items apply their main-hand and off-hand modifiers to the player (#4118, #4777).
    #[test]
    fn equipment_modifiers_follow_their_slot_group() {
        assert!(equipment_modifier_applies("mainhand", "mainhand"));
        assert!(equipment_modifier_applies("hand", "mainhand"));
        assert!(equipment_modifier_applies("any", "mainhand"));
        assert!(!equipment_modifier_applies("armor", "mainhand"));
        assert!(!equipment_modifier_applies("mainhand", "head"));
        assert!(equipment_modifier_applies("armor", "chest"));
        assert!(!equipment_modifier_applies("hand", "feet"));
        assert!(equipment_modifier_applies("offhand", "offhand"));
        assert!(equipment_modifier_applies("hand", "offhand"));
        assert!(equipment_modifier_applies("any", "offhand"));
        assert!(!equipment_modifier_applies("armor", "offhand"));
        assert!(!equipment_modifier_applies("mainhand", "offhand"));
        assert!(!equipment_modifier_applies("offhand", "mainhand"));
    }
    use horizon_client::keymapping::{KeyAction, SNEAK, SPRINT, ToggleKeys};

    /// A guest's interact packet names its entity and the server keeps it:
    /// `ServerGamePacketListenerImpl.handleInteract` resolves
    /// `packet.entityId()` once and `Player.interactOn` clicks that entity,
    /// while a fresh pick would take the nearest one. Two cows stand one
    /// behind the other; the packet names the far one.
    #[test]
    fn an_interact_packet_clicks_the_entity_it_names_not_the_nearest() {
        // (server id, distance from the guest) in the level's own order.
        let cows = [(17u32, 1.5f64), (23, 2.5)];
        assert_eq!(Game::named_entity(&cows, 23, |c| c.0), Some(1));
        // The near cow is reachable too, when that is the one named.
        assert_eq!(Game::named_entity(&cows, 17, |c| c.0), Some(0));
        // An id the level does not hold clicks nothing (Level.getEntity).
        assert_eq!(Game::named_entity(&cows, 31, |c| c.0), None);
    }

    /// The id path keeps the crosshair pick's filters, so a packet cannot name
    /// a mob the pick would refuse: one in its death animation, the one the
    /// player rides, or the camera entity. Without them a guest could shear,
    /// feed or leash a dying mob that `mob_hit` never returns.
    #[test]
    fn an_interact_packet_cannot_name_a_mob_the_pick_would_refuse() {
        use super::creatures::named_mob_pickable;
        assert!(named_mob_pickable(true, 17, None, None));
        // Dead: `Mob.alive()` is health > 0.
        assert!(!named_mob_pickable(false, 17, None, None));
        // Ridden, and the camera entity (`remote_camera_id`).
        assert!(!named_mob_pickable(true, 17, Some(17), None));
        assert!(!named_mob_pickable(true, 17, None, Some(17)));
        // Another mob being ridden or watched does not block this one.
        assert!(named_mob_pickable(true, 17, Some(23), Some(29)));
    }

    /// A mod screen on a hopper or dispenser menu owns its slots: the stock
    /// grid stands back so `slot_position` reaches the mod's own slots
    /// (`native_screen_slot`) and its `inventory_y` (#5141). Without a mod
    /// screen the stock layout answers exactly as before, which is what keeps
    /// this invisible to players who run no native mods.
    #[test]
    fn a_mod_screen_on_a_machine_menu_takes_the_stock_layout_out() {
        let stock = Some((8.0, 109.0));
        // A mod screen is open: the stock grid does not answer.
        assert_eq!(Game::stock_machine_slot(stock, true), None);
        // No mod screen: the hopper and dispenser layouts are untouched.
        assert_eq!(Game::stock_machine_slot(stock, false), stock);
        // Other menus have no stock layout to stand back from.
        assert_eq!(Game::stock_machine_slot(None, true), None);
        assert_eq!(Game::stock_machine_slot(None, false), None);
    }

    /// A falling block is drawn moved by its column's model offset, as
    /// `ModelBlockRenderer.tesselateBlock` does with `BlockState.getOffset`
    /// of `BlockPos.containing(x, maxY, z)`: a pointed dripstone that breaks
    /// off stays where it hung instead of jumping to the block's centre (#4102).
    #[test]
    fn a_falling_block_keeps_its_model_offset() {
        let at = Vec3::new(3.5, 70.25, -1.5);
        let falling = FallingBlock {
            id: 0,
            state: BlockStateId(7),
            start_pos: BlockPos::new(3, 72, -2),
            position: at,
            previous: Vec3::new(3.5, 71.0, -1.5),
            velocity: Vec3::ZERO,
            age: 5,
            no_gravity: false,
            drop_item: true,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            damage: horizon_client::falling_block::Damage::default(),
            saved: None,
        };
        // OffsetType.XZ: one offset per column (Mth.getSeed(x, 0, z)).
        let offset = |state: BlockStateId, x: i32, z: i32| {
            assert_eq!(state, BlockStateId(7));
            ((x, z) == (3, -2)).then_some([0.125, 0.0, -0.0625])
        };
        let origin = falling.model_origin(0.5, offset);
        assert_eq!(origin, glam::DVec3::new(3.125, 70.625, -2.0625));
        // Blocks without a random offset are drawn centred on the entity.
        let origin = falling.model_origin(1.0, |_, _, _| None);
        assert_eq!(origin, glam::DVec3::new(3.0, 70.25, -2.0));
    }

    /// `FallingBlockRenderer.shouldRender` emits nothing for a falling block whose
    /// own state is the state of the cell it is in (a `NoGravity` block, or one
    /// that has just landed), where Horizon drew a z-fighting copy; the same block
    /// draws its model again once that cell is cleared.
    #[test]
    fn a_falling_block_in_its_own_state_emits_nothing() {
        let state = BlockStateId(7);
        assert!(!falling_block_visible(state, state));
        assert!(falling_block_visible(state, BlockStateId(0)));
    }

    /// A key bound to key.attack keeps attacking while it is held: each
    /// repeat clicks like the press (#3243).
    #[test]
    fn a_held_key_bound_to_a_button_repeats_its_clicks() {
        assert_eq!(keyboard_button(true, KeyAction::Press), Some(true));
        assert_eq!(keyboard_button(true, KeyAction::Repeat), Some(true));
        assert_eq!(keyboard_button(true, KeyAction::Release), Some(false));
        for action in [KeyAction::Press, KeyAction::Repeat, KeyAction::Release] {
            assert_eq!(keyboard_button(false, action), None);
        }
    }

    /// A server dialog is a screen: held keys and the use button are released
    /// (`KeyMapping.releaseAll`), and toggled sneak comes back when it closes (#4974).
    #[test]
    fn a_dialog_releases_held_keys_like_a_screen() {
        assert!(gameplay_has_keyboard(Screen::Playing, false, false));
        assert!(!gameplay_has_keyboard(Screen::Playing, false, true));
        assert!(!gameplay_has_keyboard(Screen::Playing, true, false));
        assert!(!gameplay_has_keyboard(Screen::Chat, false, false));

        let mut keys = HashSet::from([KeyCode::KeyW]);
        let mut left = true;
        let mut right = true;
        drop_held_behind_dialog(false, &mut keys, &mut left, &mut right);
        assert!(keys.contains(&KeyCode::KeyW) && left && right);
        drop_held_behind_dialog(true, &mut keys, &mut left, &mut right);
        assert!(keys.is_empty() && !left && !right);

        let options = [true, false, false, false];
        let mut toggles = ToggleKeys::default();
        toggles.key(SNEAK, KeyAction::Press, false, true);
        toggles.key(SNEAK, KeyAction::Release, false, true);
        assert!(toggles.is_down(SNEAK));
        toggles.screen_opened(options);
        assert!(!toggles.is_down(SNEAK));
        toggles.screen_closed(options);
        assert!(toggles.is_down(SNEAK));
    }
    /// A key bound to key.pickItem clones over a container screen, as the
    /// middle button does; a key bound to attack or use does nothing there (#5378).
    #[test]
    fn a_key_bound_to_pick_block_clones_behind_a_container_screen() {
        assert!(container_key_clones(MouseButton::Middle, KeyAction::Press));
        assert!(container_key_clones(MouseButton::Middle, KeyAction::Repeat));
        assert!(!container_key_clones(
            MouseButton::Middle,
            KeyAction::Release
        ));
        for button in [MouseButton::Left, MouseButton::Right] {
            assert!(!container_key_clones(button, KeyAction::Press));
        }
    }
    #[test]
    fn sleep_fade_lasts_ten_ticks_after_waking() {
        let alpha = |timer| match hud_overlays::sleep_overlay(320, 240, timer).first() {
            Some(hud_layout::GuiDraw::Fill { color, .. }) => color >> 24,
            None => 0,
            _ => panic!("sleep overlay must be a full-screen fill"),
        };
        assert_eq!(tick_sleep_timer(0, false), 0);
        let mut timer = 0;
        for _ in 0..50 {
            timer = tick_sleep_timer(timer, true);
        }
        assert_eq!(alpha(timer as i32), 110);
        for _ in 0..100 {
            timer = tick_sleep_timer(timer, true);
        }
        assert_eq!(timer, 100);
        assert_eq!(alpha(timer as i32), 220);
        for _ in 0..5 {
            timer = tick_sleep_timer(timer, false);
        }
        assert_eq!(alpha(timer as i32), 110);
        for _ in 0..5 {
            timer = tick_sleep_timer(timer, false);
        }
        assert_eq!(timer, 0);
        assert!(hud_overlays::sleep_overlay(320, 240, timer as i32).is_empty());
    }

    #[test]
    fn world_rest_poll_does_not_wait_for_a_blocked_worker() {
        let (release, wait) = std::sync::mpsc::channel();
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(move || {
                wait.recv().unwrap();
                Ok(())
            }),
        };
        assert!(!loader.ready().unwrap());
        release.send(()).unwrap();
        loader.finish().unwrap();
    }

    #[test]
    fn world_rest_poll_waits_for_a_slow_worker() {
        let (release, wait) = std::sync::mpsc::channel();
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(move || {
                wait.recv().unwrap();
                Ok(())
            }),
        };
        // However long it takes, a worker still loading is not an error.
        assert!(!loader.ready().unwrap());
        release.send(()).unwrap();
        while !loader.ready().unwrap() {
            std::thread::sleep(Duration::from_millis(1));
        }
        loader.finish().unwrap();
    }

    #[test]
    fn world_rest_worker_failure_reaches_the_client() {
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(|| Err("missing world table".to_owned())),
        };
        assert_eq!(
            loader.finish().unwrap_err().to_string(),
            "missing world table"
        );
    }

    #[test]
    fn world_rest_worker_panic_reaches_the_client() {
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(|| panic!("broken loader")),
        };
        assert_eq!(
            loader.finish().unwrap_err().to_string(),
            "world resource loader panicked"
        );
    }

    /// An integrated server thread shares the loaded resources with the client.
    #[test]
    fn shared_resources_can_cross_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Resources>();
        shared::<PackResources>();
    }

    #[test]
    fn selecting_creative_tabs_does_not_drop_the_carried_stack_on_release() {
        let mut creative = Creative::from_json(&serde_json::json!({
            "schema": 2,
            "tabs": [
                {"type": "CATEGORY", "items": ["{count:1,id:\"minecraft:stone\"}"]},
                {"type": "INVENTORY"}
            ]
        }))
        .unwrap();
        for button in [Button::Left, Button::Right] {
            let mut clicks = Clicks::default();
            for tab in [1, 1, 0, 1] {
                select_creative_tab(&mut creative, &mut clicks, tab);
                assert_eq!(creative.selected, tab);
                assert_eq!(creative.tab().inventory, tab == 1);
                assert_eq!(
                    clicks.release(
                        &Target::<usize>::Outside,
                        button,
                        false,
                        false,
                        false,
                        false
                    ),
                    Release::Nothing
                );
            }
        }
    }

    /// Eyes under water: the ray hits the source the eyes are in, at the eyes
    /// (#3787), whichever way it looks.
    #[test]
    fn a_ray_starting_inside_a_source_hits_it_at_the_start() {
        let eye = [0.5, 65.62, 0.5];
        let down = [0.0, -1.0, 0.0];
        assert_eq!(
            clip_start_source(eye, down, 4.5, [0, 65, 0], true),
            Some(([0, 1, 0], eye))
        );
        // Without water above, 0.62 is still below the surface (8/9).
        assert_eq!(
            clip_start_source(eye, [0.0, 1.0, 0.0], 4.5, [0, 65, 0], false),
            Some(([0, -1, 0], eye))
        );
        // Looking ahead, slightly down: SOUTH's dot product is largest.
        let ahead = [0.0, -0.2f64, 0.98f64.sqrt()];
        assert_eq!(
            clip_start_source(eye, ahead, 4.5, [0, 65, 0], true),
            Some(([0, 0, -1], eye))
        );
    }

    /// Eyes above a surface source's 8/9: a ray going down enters it through
    /// the top inside the block; level or rising rays miss it.
    #[test]
    fn a_ray_starting_above_a_source_surface_enters_through_the_top() {
        let eye = [0.5, 65.95, 0.5];
        let hit = clip_start_source(eye, [0.0, -1.0, 0.0], 4.5, [0, 65, 0], false);
        let (face, point) = hit.unwrap();
        assert_eq!(face, [0, 1, 0]);
        assert!((point[1] - (65.0 + f64::from(8.0f32 / 9.0f32))).abs() < 1e-12);
        assert_eq!(
            clip_start_source(eye, [1.0, 0.0, 0.0], 4.5, [0, 65, 0], false),
            None
        );
        // Leaves the block sideways before it comes down to the surface.
        let shallow = [0.995f64.sqrt(), -0.0707, 0.0];
        assert_eq!(
            clip_start_source(eye, shallow, 4.5, [0, 65, 0], false),
            None
        );
    }

    #[test]
    fn stack_drop_requires_ctrl_even_with_sprint_latched() {
        for sprint in [false, true] {
            let mut toggles = ToggleKeys::default();
            if sprint {
                toggles.key(SPRINT, KeyAction::Press, false, true);
                toggles.key(SPRINT, KeyAction::Release, false, true);
            }
            assert_eq!(toggles.is_down(SPRINT), sprint);

            for held in 0..4 {
                let mut modifiers = winit::keyboard::ModifiersState::empty();
                let mut physical_keys = HashSet::from([KeyCode::ShiftLeft, KeyCode::KeyQ]);
                let sides = [KeyCode::ControlLeft, KeyCode::ControlRight];
                for (bit, key) in sides.into_iter().enumerate() {
                    if held & (1 << bit) != 0 {
                        horizon_client_input::modifiers::set_key(
                            &mut modifiers,
                            &mut physical_keys,
                            key,
                            true,
                        );
                    }
                }
                assert_eq!(drop_whole_stack(modifiers), held != 0);
                for key in sides {
                    horizon_client_input::modifiers::set_key(
                        &mut modifiers,
                        &mut physical_keys,
                        key,
                        false,
                    );
                }
                assert!(!drop_whole_stack(modifiers));
                assert_eq!(toggles.is_down(SPRINT), sprint);
            }
        }
    }

    #[test]
    fn quit_shortcut_blocks_window_close_when_option_disabled() {
        use winit::keyboard::ModifiersState;

        let mut keys = HashSet::new();
        let alt = ModifiersState::ALT;
        let super_key = ModifiersState::SUPER;

        let quit_shortcuts_enabled = true;
        let quit_shortcuts_disabled = false;

        keys.insert(if cfg!(target_os = "macos") {
            KeyCode::KeyQ
        } else {
            KeyCode::F4
        });
        let mods = if cfg!(target_os = "macos") {
            super_key
        } else {
            alt
        };

        let should_ignore =
            |quit_shortcuts: bool, modifiers: ModifiersState, keys: &HashSet<KeyCode>| {
                !quit_shortcuts && horizon_client::input::is_quit_shortcut_down(modifiers, keys)
            };

        // When enabled, close requests from shortcuts proceed normally.
        assert!(!should_ignore(quit_shortcuts_enabled, mods, &keys));

        // When disabled and the quit shortcut is held, close requests are ignored.
        assert!(should_ignore(quit_shortcuts_disabled, mods, &keys));

        // When disabled but shortcut NOT held (e.g. 'X' button clicked), close request proceeds.
        assert!(!should_ignore(
            quit_shortcuts_disabled,
            ModifiersState::empty(),
            &HashSet::new()
        ));
    }

    #[test]
    fn integrated_server_save_persists_owner_player_data() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-integrated-player-save-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // Initial world creation writes `level.dat` before the player spawns.
        let mut level = Level {
            name: "Creative".into(),
            creative: true,
            game_mode: 1,
            ..Level::default()
        };
        level.write(&dir).unwrap();

        // The integrated server stores the owner's state in `level` and saves the world.
        level.position = Some([128.5, 72.0, -64.25]);
        level.inventory = vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)];
        write_level_for_role(
            &mut level,
            &dir,
            execution_role::ExecutionRole::IntegratedServer,
        )
        .unwrap();

        let reopened = Level::read(&dir).unwrap();
        assert_eq!(reopened.position, Some([128.5, 72.0, -64.25]));
        assert_eq!(
            reopened.inventory,
            vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)]
        );

        // A dedicated server save updates world metadata without overwriting the player file.
        let mut dedicated = Level {
            player_uuid: level.player_uuid,
            day_time: 6000,
            ..Level::default()
        };
        write_level_for_role(
            &mut dedicated,
            &dir,
            execution_role::ExecutionRole::DedicatedServer,
        )
        .unwrap();

        let reopened = Level::read(&dir).unwrap();
        assert_eq!(reopened.day_time, 6000);
        assert_eq!(reopened.position, Some([128.5, 72.0, -64.25]));
        assert_eq!(
            reopened.inventory,
            vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn write_level_for_role(
    level: &mut Level,
    dir: &std::path::Path,
    role: execution_role::ExecutionRole,
) -> Result<()> {
    if role.writes_singleplayer_player() {
        level.write(dir)
    } else {
        level.write_without_player(dir)
    }
}
