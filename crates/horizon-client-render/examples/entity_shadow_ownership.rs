//! Original synthetic capability regression; no game resources or player world.
//! Run with fresh HORIZON_HOME, HORIZON_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME,
//! HORIZON_NO_AUDIO=1 and HORIZON_LAUNCHER_NO_SELF_UPDATE=1.
use horizon_client_render::{gfx::Renderer, graphics::GraphicsApi};
use std::{io::Write, sync::Arc};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

const VERTEX: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
const COLOUR: &str = "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n";
// Reading the actual shadow sampler activates the shadow pass, rather than just
// placing a shadow program in the archive and assuming the pass runs.
const SHADOW_READ: &str = "#version 120\nuniform sampler2D shadowtex0;\n\
    void main() { gl_FragColor = vec4(vec3(texture2D(shadowtex0, vec2(0.5)).r), 1.0); }\n";

fn pack(properties: &str, shadow: Option<&str>, reads_shadow: bool) -> horizon_shaders::Pack {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut write = |name: &str, text: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    };
    write("shaders.properties", properties);
    write("final.vsh", VERTEX);
    write("final.fsh", if reads_shadow { SHADOW_READ } else { COLOUR });
    write("gbuffers_entities.vsh", VERTEX);
    write("gbuffers_entities.fsh", COLOUR);
    if let Some(shadow) = shadow {
        write("shadow.vsh", VERTEX);
        write("shadow.fsh", shadow);
    }
    horizon_shaders::Pack::from_zip_bytes(
        "original-shadow-ownership",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap()
}

fn fixture(window: Arc<Window>) -> Result<(), Box<dyn std::error::Error>> {
    let white = [255; 4];
    let panorama = Arc::new(horizon_client_render::gfx::panorama::Faces::colour([0; 4]));
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        1,
        1,
        1,
        &[],
        &[],
        (&white, 1, 1),
        (&white, 1, 1),
        (&white, 1, 1),
        (&white, 1),
        panorama,
        None,
        None,
        GraphicsApi::Vulkan,
        false,
        true,
    ))?;
    assert!(!renderer.pack_entity_shadows(), "no selected pack");
    renderer.set_shader_pack(0, None, &[]);
    assert!(
        !renderer.pack_entity_shadows(),
        "selected pack without a compiled post"
    );

    let cases = [
        ("full", "", Some(COLOUR), true, false, true),
        (
            "player-only",
            "shadowEntities=false\nshadowPlayer=true",
            Some(COLOUR),
            true,
            false,
            false,
        ),
        // Block-entity shadows alone must not suppress a mob's blob.
        (
            "blocks-only",
            "shadowEntities=false",
            Some(COLOUR),
            true,
            false,
            false,
        ),
        (
            "disabled",
            "shadow.enabled=false",
            Some(COLOUR),
            true,
            false,
            false,
        ),
        ("missing-program", "", None, true, false, false),
        (
            "failed-translation",
            "",
            Some("#version 120\nvoid main() { gl_FragColor = deliberately_undefined; }"),
            true,
            false,
            false,
        ),
        ("failed-pipeline", "", Some(COLOUR), true, true, false),
        (
            "unused-shadow-program",
            "",
            Some(COLOUR),
            false,
            false,
            false,
        ),
        (
            "forced-shadow-program",
            "shadow.enabled=true",
            Some(COLOUR),
            false,
            false,
            true,
        ),
    ];
    for dimension in [0, -1, 1] {
        for (name, properties, shadow, reads_shadow, break_pipeline, expected) in cases {
            let pack = pack(properties, shadow, reads_shadow);
            let mut macros = horizon_shaders::Macros::standard();
            renderer.define_pack_macros(&mut macros);
            let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
            let mut post = horizon_shaders::load_post(
                &pack,
                "",
                &properties,
                &macros,
                &renderer.pack_inputs(),
            );
            if name == "failed-translation" {
                assert!(post.shadow_entities.is_none());
                assert!(post.notes.iter().any(|note| note.contains("skipped")));
            } else if name == "missing-program" {
                assert!(post.shadow_entities.is_none());
                assert!(post.notes.iter().any(|note| note.contains("no program")));
            } else if break_pipeline {
                // A translated stage can still fail device pipeline creation. Inject a
                // missing entry point into only the entity caster; terrain/block stages
                // and the settings stay valid. PackPost must report the compiled result.
                post.shadow_entities
                    .as_mut()
                    .unwrap()
                    .program
                    .as_mut()
                    .unwrap()
                    .vertex
                    .entry_points[0]
                    .name = "deliberately_missing_main".into();
            } else {
                assert!(post.notes.is_empty(), "{name}: {:?}", post.notes);
            }
            let (_, notes) = renderer
                .set_shader_pack(dimension, Some(post), &[])
                .unwrap();
            if break_pipeline {
                assert!(
                    notes.iter().any(|note| note.contains("skipped")),
                    "{notes:?}"
                );
            } else if name != "missing-program" && name != "failed-translation" {
                assert!(
                    !notes.iter().any(|note| note.contains("skipped")),
                    "{name}: {notes:?}"
                );
            }
            assert_eq!(
                renderer.pack_entity_shadows(),
                expected,
                "{name}, dimension {dimension}: {notes:?}"
            );
            assert!(!renderer.native_shadow());
            println!("{name}, dimension {dimension}: replaces_mob_blob={expected}");
        }
    }
    renderer.drop_shader_packs();
    assert!(
        !renderer.pack_entity_shadows(),
        "dropped pack restores fallback"
    );
    println!("ENTITY_SHADOW_OWNERSHIP_PASSED: 27 compiled controls and no/pending/dropped pack");
    Ok(())
}

struct App;
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(winit::dpi::LogicalSize::new(32., 32.)),
                )
                .unwrap(),
        );
        if let Err(error) = fixture(window) {
            eprintln!("Shadow ownership fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn main() {
    EventLoop::new().unwrap().run_app(&mut App).unwrap();
}
