//! Original asset-free post-output regressions, using only the public shader format.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use std::io::{Cursor, Write};
use wgpu::TextureFormat as T;

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";

#[derive(Clone, Copy, Debug)]
enum Coverage {
    Full,
    Discard,
    Vertex,
    Scale,
}

fn pack(
    coverage: Coverage,
    mrt: bool,
    keep_main: bool,
    history: Option<bool>,
    shadow: bool,
) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    let targets = match (shadow, mrt) {
        (true, true) => "0,1",
        (true, false) => "0",
        (false, true) => "2,5",
        (false, false) => "2",
    };
    let fragment = |first: &str, second: &str, discard: bool| {
        format!(
            "#version 120\n/* RENDERTARGETS:{targets} */\nvarying vec2 uv;\nvoid main() {{ {} gl_FragData[0]=vec4({first}); {} }}\n",
            if discard { "if(uv.x<0.5) discard;" } else { "" },
            if mrt {
                format!("gl_FragData[1]=vec4({second});")
            } else {
                String::new()
            },
        )
    };
    let mut properties = if shadow {
        "shadow.enabled=true\n".to_owned()
    } else {
        String::new()
    };
    if let Some(clear) = history {
        add("composite.vsh", VERTEX);
        add("composite.fsh", &fragment("0.25,0,0,1", "", false));
        add("composite1.vsh", VERTEX);
        add("composite1.fsh", &fragment("0,0.5,0,1", "", true));
        add("composite2.vsh", VERTEX);
        add(
            "composite2.fsh",
            "#version 120\n/* RENDERTARGETS:3 */\nuniform sampler2D colortex2;\nvarying vec2 uv;\nvoid main() { gl_FragData[0]=texture2D(colortex2,uv); }\n",
        );
        properties.push_str("flip.composite2.colortex2=true\n");
        add(
            "final.fsh",
            &format!(
                "#version 120\nuniform sampler2D colortex2;\nuniform sampler2D colortex3;\nvarying vec2 uv;\nconst bool colortex2Clear={clear};\nconst vec4 colortex2ClearColor=vec4(0,0.375,0.5,1);\nvoid main() {{ gl_FragColor=vec4(texture2D(colortex2,uv).r,texture2D(colortex3,uv).g,texture2D(colortex3,uv).b,1); }}\n"
            ),
        );
    } else {
        // B gets red, A gets blue, then the partial pass writes B again. Outside writes
        // must retain B's red, independently of the current A's blue.
        for (name, first, second) in [
            ("composite", "0.25,0,0,1", "0,0.375,0,1"),
            ("composite1", "0,0,0.25,1", "0.25,0,0.25,1"),
            ("composite2", "0,0.5,0,1", "0,0.75,0,1"),
        ] {
            let partial = name == "composite2";
            let name = if shadow {
                name.replace("composite", "shadowcomp")
            } else {
                name.to_owned()
            };
            let vertex = if partial && matches!(coverage, Coverage::Vertex) {
                VERTEX.replace(
                    "ftransform();",
                    "ftransform(); gl_Position.x=gl_Position.x*0.5+gl_Position.w*0.5;",
                )
            } else {
                VERTEX.to_owned()
            };
            add(&format!("{name}.vsh"), &vertex);
            add(
                &format!("{name}.fsh"),
                &fragment(
                    first,
                    second,
                    partial && matches!(coverage, Coverage::Discard),
                ),
            );
        }
        if matches!(coverage, Coverage::Scale) {
            properties.push_str("scale.composite2=0.5 0.5 0.0\n");
        }
        if keep_main {
            properties
                .push_str("flip.composite2.colortex2=false\nflip.composite2.colortex5=false\n");
        }
        let final_source = if mrt {
            "#version 120\nuniform sampler2D colortex2;\nuniform sampler2D colortex5;\nvarying vec2 uv;\nvoid main() { gl_FragColor=vec4(texture2D(colortex2,uv).r,texture2D(colortex5,uv).g,texture2D(colortex2,uv).g,1); }\n"
        } else {
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\nvoid main() { gl_FragColor=texture2D(colortex2,uv); }\n"
        };
        let final_source = if shadow {
            final_source
                .replace("colortex2", "shadowcolor0")
                .replace("colortex5", "shadowcolor1")
                .replace(
                    "#version 120",
                    "#version 120\nconst int shadowMapResolution=16;",
                )
        } else {
            final_source.to_owned()
        };
        add("final.fsh", &final_source);
    }
    add("final.vsh", VERTEX);
    add("shaders.properties", &properties);
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-destination-contract",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert_eq!(
        post.programs.len(),
        if shadow { 1 } else { 4 },
        "{:?}",
        post.notes
    );
    assert_eq!(
        post.shadow_comp.len(),
        if shadow { 3 } else { 0 },
        "{:?}",
        post.notes
    );
    post
}

struct Fixture {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Fixture {
    fn new() -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("Post destination test skipped: no GPU adapter");
            return None;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device for destination contract");
        eprintln!("Post destination adapter: {:?}", adapter.get_info());
        Some(Self {
            adapter,
            device,
            queue,
        })
    }

    fn texture(&self, size: [u32; 2], format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original post destination fixture"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    fn build(&self, post: Post) -> PackPost {
        let lightmap = self.texture([1, 1], T::Rgba8Unorm);
        let post = PackPost::new(
            &self.device,
            &self.queue,
            &self.adapter,
            post,
            T::Rgba8Unorm,
            World {
                atlas: None,
                lightmap,
                terrain: None,
                block_ids: &[],
                celestial: None,
                gui_size: [16, 16],
                clouds: None,
                weather: false,
                hand: false,
            },
        );
        assert!(
            post.notes.iter().all(|n| n.starts_with("shadow map ")),
            "{:?}",
            post.notes
        );
        post
    }

    fn render(&self, post: &mut PackPost, size: [u32; 2]) -> Vec<[u8; 4]> {
        let world = self.texture(size, T::Rgba8Unorm);
        let depth = self.texture(size, T::Depth32Float);
        let screen = self.texture(size, T::Rgba8Unorm);
        let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride * size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let hud = HudBatch::new(size);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        post.run(
            &self.device,
            &self.queue,
            &mut encoder,
            &world,
            &depth,
            &screen,
            &fixture_frame(&hud),
            &Default::default(),
            size,
            size,
            &[],
            &mut |_, _, _, _| {},
        );
        encoder.copy_texture_to_buffer(
            screen.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(stride),
                    ..Default::default()
                },
            },
            screen.texture().size(),
        );
        self.queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        read.slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks_exact(stride as usize)
            .flat_map(|row| {
                row[..size[0] as usize * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
            })
            .collect()
    }
}

#[test]
fn partial_writes_retain_the_alternate_with_flip_scale_and_mrt_controls() {
    let Some(gpu) = Fixture::new() else { return };
    let mut failures = Vec::new();
    for (coverage, mrt, keep_main, shadow) in [
        (Coverage::Full, false, false, false),
        (Coverage::Discard, false, false, false),
        (Coverage::Vertex, false, false, false),
        (Coverage::Scale, false, false, false),
        (Coverage::Discard, true, false, false),
        (Coverage::Discard, false, true, false),
        (Coverage::Full, false, false, true),
        (Coverage::Discard, false, false, true),
        (Coverage::Vertex, false, false, true),
        (Coverage::Discard, true, false, true),
    ] {
        let mut post = gpu.build(pack(coverage, mrt, keep_main, None, shadow));
        // The same pack is also exercised after resize; all destination seeds come from
        // real shader passes, never from the load operation under test.
        for size in [[16, 16], [8, 8]] {
            let actual = gpu.render(&mut post, size);
            let wrong = actual
                .iter()
                .enumerate()
                .filter(|(n, pixel)| {
                    let x = *n as u32 % size[0];
                    let y = *n as u32 / size[0];
                    let outside = match coverage {
                        Coverage::Full => false,
                        Coverage::Discard | Coverage::Vertex => x < size[0] / 2,
                        Coverage::Scale => x < size[0] / 2 || y < size[1] / 2,
                    };
                    let expected = match (keep_main, outside, mrt) {
                        (true, _, _) => [0, 0, 64, 255],
                        (false, true, false) => [64, 0, 0, 255],
                        (false, false, false) => [0, 128, 0, 255],
                        (false, true, true) => [64, 96, 0, 255],
                        (false, false, true) => [0, 191, 128, 255],
                    };
                    pixel.iter().zip(expected).any(|(a, b)| a.abs_diff(b) > 1)
                })
                .count();
            eprintln!(
                "Destination {coverage:?} mrt={mrt} keep_main={keep_main} shadow={shadow} size={size:?}: wrong={wrong}/{}",
                actual.len()
            );
            if wrong != 0 {
                failures.push((coverage, mrt, keep_main, shadow, size, wrong));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Destination pixels differ: {failures:?}"
    );
}

#[test]
fn frame_clear_and_retained_history_apply_before_partial_post_writes() {
    let Some(gpu) = Fixture::new() else { return };
    for clear in [true, false] {
        let mut post = gpu.build(pack(Coverage::Full, false, false, Some(clear), false));
        // Every frame changes which physical image is main. The other image is fully
        // seeded red before the partial write, independent of its initial clearing.
        for frame in 0..3 {
            let actual = gpu.render(&mut post, [16, 16]);
            assert!(
                actual.iter().enumerate().all(|(n, p)| {
                    let outside = n % 16 < 8;
                    if frame == 0 && !clear && outside {
                        return true;
                    }
                    let expected = if !outside {
                        [64, 128, 0, 255]
                    } else if clear {
                        [64, 96, 128, 255]
                    } else {
                        [64, 0, 0, 255]
                    };
                    p.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1)
                }),
                "clear={clear} frame={frame}: {:?}",
                &actual[..16]
            );
        }
    }
}

// Frame literal adapted from MIT Horizon f700231 pack_uniforms.rs test API setup.
fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_degrees: 90.0,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
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
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
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
        menu_blur: None,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}

#[path = "clear_budget_tests.rs"]
mod clear_budget_tests;
