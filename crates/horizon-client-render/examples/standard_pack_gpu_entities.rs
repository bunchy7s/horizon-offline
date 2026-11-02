//! Original asset-free standard-pack entity fixture. Run with throwaway HORIZON_HOME,
//! HORIZON_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md).
//! `cargo run -p horizon-client-render --example standard_pack_gpu_entities -- <output-dir>`
use client::{
    gfx::{
        Frame, Renderer, ScreenshotRequest,
        entity_skin::{SkinFrame, SkinJob, SkinPart, SkinVertex},
        native_render,
        pack_draw::EntitySpan,
    },
    graphics::GraphicsApi,
    hud::HudBatch,
};
use horizon_client_render as client;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

static NEEDS_SPANS: AtomicBool = AtomicBool::new(false);
static OBSERVED: AtomicUsize = AtomicUsize::new(0);
struct Observer;
impl native_render::RenderHooks for Observer {
    fn render_needs(&self, event: &mut native_render::RenderNeedsEvent<'_>) {
        event.needs.entity_spans |= NEEDS_SPANS.load(Ordering::Relaxed);
    }
    fn world_drawn(&self, event: &mut native_render::WorldDrawnEvent<'_, '_>) {
        if NEEDS_SPANS.load(Ordering::Relaxed) {
            assert_eq!(event.world.entities.len(), 12);
            assert_eq!(
                event.world.spans.iter().map(|s| s.id).collect::<Vec<_>>(),
                [77, 88, 99]
            );
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn pack(renderer: &mut Renderer, folder: &Path, valid: bool) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    let vertex = "#version 120\nvarying vec2 uv; void main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}\n";
    let fragment = "#version 120\nuniform int entityId; uniform int blockEntityId; uniform vec4 entityColor; void main(){gl_FragData[0]=vec4(float(entityId)/255.0,entityColor.a,float(blockEntityId)/255.0,1.0);}\n";
    for program in [
        "gbuffers_entities",
        "gbuffers_block",
        "gbuffers_entities_translucent",
    ] {
        std::fs::write(shaders.join(format!("{program}.vsh")), vertex).unwrap();
        std::fs::write(
            shaders.join(format!("{program}.fsh")),
            if valid {
                fragment
            } else {
                "#version 120\nvoid main(){ invalid shader; }"
            },
        )
        .unwrap();
    }
    std::fs::write(shaders.join("final.vsh"), vertex).unwrap();
    std::fs::write(shaders.join("final.fsh"), "#version 120\nvarying vec2 uv; uniform sampler2D colortex0; void main(){gl_FragColor=texture2D(colortex0,uv);}\n").unwrap();
    std::fs::write(
        shaders.join("shaders.properties"),
        "oldLighting=false\nshadow.enabled=false\n",
    )
    .unwrap();
    let pack = horizon_shaders::Pack::open(folder).unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    assert_eq!(post.notes.is_empty(), valid, "{:?}", post.notes);
    let (_, notes) = renderer.set_shader_pack(0, Some(post), &[]).unwrap();
    assert!(
        !valid || notes.iter().all(|n| !n.contains("skipped")),
        "{notes:?}"
    );
}

fn read(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    bytes.truncate(info.buffer_size());
    bytes
}

fn fixture(window: Arc<Window>) -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).ok_or("output directory required")?;
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;
    native_render::install(Box::new([&Observer])).map_err(|_| "observer already installed")?;
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        16,
        16,
        1,
        &[],
        &[],
        (&white, 16, 16),
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        None,
        GraphicsApi::Vulkan,
        false,
        true,
    ))?;
    renderer.resize(320, 180);
    assert!(
        renderer.skins_entities(),
        "fixture requires compute skinning"
    );
    let unmodified_tagged = renderer.skins_tagged();
    pack(&mut renderer, &out.join("valid-pack"), true);
    assert!(
        !renderer.skins_tagged(),
        "eye/tagged layers keep the CPU path with a pack"
    );

    let spans = [77, 88, 99]
        .into_iter()
        .enumerate()
        .map(|(i, id)| EntitySpan {
            start: i as u32 * 4,
            stage: horizon_shaders::render_stage("ENTITIES"),
            id,
            hidden: i == 2,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut skin = SkinFrame {
        reset: true,
        vertices: 12,
        spans: spans.clone(),
        parts: vec![SkinPart {
            matrix: glam::Mat4::IDENTITY.to_cols_array(),
            normal: glam::Mat3::IDENTITY.to_cols_array(),
        }],
        ..Default::default()
    };
    for (i, x) in [-1.8, 0.3, -0.7].into_iter().enumerate() {
        for (pos, uv) in [
            [x, -0.8, 5.],
            [x, 0.8, 5.],
            [x + 1.4, 0.8, 5.],
            [x + 1.4, -0.8, 5.],
        ]
        .into_iter()
        .zip([[0., 0.], [0., 1.], [1., 1.], [1., 0.]])
        {
            skin.new_model.push(SkinVertex {
                pos,
                uv,
                axis: 5,
                part: 0,
            });
        }
        skin.jobs.push(SkinJob {
            first: i as u32 * 4,
            dst: i as u32 * 4,
            count: 4,
            rect: [0., 0., 1., 1.],
            color: u32::MAX,
            light: 0xf0f0,
            overlay: if i == 0 { 1 } else { 5 },
            ..Default::default()
        });
    }
    let cpu = client::gfx::entity_skin::expand(&skin.new_model, &skin);
    let hud = HudBatch::new([320, 180]);
    let mut frame = fixture_frame(&hud);
    frame.title = false;
    frame.camera = glam::DVec3::ZERO;
    frame.eye = glam::DVec3::ZERO;
    frame.pitch = 0.;
    frame.entities = &cpu;
    frame.entity_spans = &spans;
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("cpu.png"),
        size: None,
        scaled: false,
    });
    renderer.render(&frame)?;
    let expected = read(&out.join("cpu.png"));
    let matches = expected
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] == [77, 77, 0])
        .count();
    assert!(
        matches > 100,
        "original entity-ID/hurt output, got {matches} pixels"
    );
    let white = expected
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] == [88, 192, 0])
        .count();
    assert!(
        white > 100,
        "original entity-ID/white-flash output, got {white} pixels"
    );
    assert_eq!(
        expected
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] == 99)
            .count(),
        0,
        "hidden caster is not camera-visible"
    );
    assert!(
        renderer.skins_entities(),
        "compiled standard-pack stages must accept GPU posing"
    );
    frame.entities = &[];
    frame.entity_spans = &[];
    frame.skin = Some(&skin);
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("gpu.png"),
        size: None,
        scaled: false,
    });
    renderer.render(&frame)?;
    assert_eq!(
        read(&out.join("gpu.png")),
        expected,
        "CPU and GPU standard-pack pixels"
    );

    NEEDS_SPANS.store(true, Ordering::Relaxed);
    assert!(
        !renderer.skins_entities(),
        "native CPU-stream observer keeps its fallback"
    );
    frame.skin = None;
    frame.entities = &cpu;
    frame.entity_spans = &spans;
    renderer.render(&frame)?;
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 1);
    NEEDS_SPANS.store(false, Ordering::Relaxed);
    assert!(renderer.skins_entities());
    pack(&mut renderer, &out.join("failed-pack"), false);
    assert!(
        !renderer.skins_entities(),
        "failed entity programs retain CPU posing"
    );
    renderer.drop_shader_packs();
    assert!(
        renderer.skins_entities(),
        "ordinary path returns when the pack is dropped"
    );
    assert_eq!(renderer.skins_tagged(), unmodified_tagged);
    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "cpu_gpu_pixels_equal": true, "entity_id_hurt_pixels": matches, "hidden_subject_pixels": 0,
                "entity_id_white_flash_pixels": white,
                "graphics_adapter": client::crash::system_details("fixture").into_iter()
                    .find(|(name, _)| name == "Graphics Adapter").map(|(_, detail)| detail),
            "native_observer_frames": OBSERVED.load(Ordering::Relaxed), "player_game_started": false,
            "fixture_only_not_gameplay_or_fps_proof": true
        }))?,
    )?;
    Ok(())
}

struct App;
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = fixture(window) {
            eprintln!("fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
fn main() {
    for key in [
        "HORIZON_HOME",
        "HORIZON_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        assert!(
            std::env::var_os(key).is_some(),
            "set throwaway {key} (AGENTS.md)"
        );
    }
    EventLoop::new().unwrap().run_app(&mut App).unwrap();
}

fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        eye: glam::DVec3::new(0.5, 100.0, 0.5),
        eye_rotation: [0.0, 10.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
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
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_spans: &[],
        skin: None,
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
        post_effects: client::post_chain::Request::none(),
        menu_blur: None,
    }
}
