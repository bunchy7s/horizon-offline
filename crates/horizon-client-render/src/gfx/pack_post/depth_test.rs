//! Original depth-buffer GPU regressions from public format documentation; no pack assets.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use wgpu::{TextureFormat as T, util::DeviceExt};

const SIZE: [u32; 2] = [16, 16];

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Gpu {
    fn new() -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
            ..Default::default()
        }))
        .expect("GPU device required: setup failure is not a contract pass");
        eprintln!("DEPTH_FIXTURE_ADAPTER {:?}", adapter.get_info());
        Some(Self {
            adapter,
            device,
            queue,
        })
    }
    fn texture(&self, format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original depth fixture input"),
                size: wgpu::Extent3d {
                    width: SIZE[0],
                    height: SIZE[1],
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
    fn load(
        &self,
        files: Vec<(String, String)>,
        world: World<'_>,
        inputs: horizon_shaders::GameInputs,
    ) -> PackPost {
        use std::io::{Cursor, Write};
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in files {
            zip.start_file(
                format!("shaders/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        let pack = horizon_shaders::Pack::from_zip_bytes(
            "original-depth-fixture",
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
        let mut macros = horizon_shaders::Macros::standard();
        let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
        let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        assert!(
            post.programs.iter().any(|p| p.name == "final"),
            "{:?}",
            post.notes
        );
        let built = PackPost::new(
            &self.device,
            &self.queue,
            &self.adapter,
            post,
            T::Rgba8Unorm,
            world,
        );
        assert!(
            built.passes.iter().any(|p| p.group == Group::Final),
            "{:?}",
            built.notes
        );
        built
    }
    fn clear(&self, color: &wgpu::TextureView, depth: &wgpu::TextureView, reverse_depth: f32) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Independent original fixture clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0625,
                        g: 0.0625,
                        b: 0.0625,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(reverse_depth),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        self.queue.submit([encoder.finish()]);
    }
    fn render(
        &self,
        post: &mut PackPost,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame: &Frame<'_>,
        terrain: &mut TerrainDraw<'_>,
    ) -> Vec<[u8; 4]> {
        let screen = self.texture(T::Rgba8Unorm);
        let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Original fixture readback"),
            size: u64::from(stride * SIZE[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        post.run(
            &self.device,
            &self.queue,
            &mut encoder,
            color,
            depth,
            &screen,
            frame,
            &Default::default(),
            SIZE,
            SIZE,
            &[],
            terrain,
        );
        encoder.copy_texture_to_buffer(
            screen.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
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
                row[..SIZE[0] as usize * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
            })
            .collect()
    }
}

fn all_pixels(pixels: &[[u8; 4]], expected: [u8; 4], name: &str) {
    let wrong = pixels
        .iter()
        .filter(|p| p.iter().zip(expected).any(|(a, b)| a.abs_diff(b) > 1))
        .count();
    eprintln!(
        "DEPTH_FIXTURE_RESULT {name} expected={expected:?} actual_first={:?} wrong={wrong}/{}",
        pixels[0],
        pixels.len()
    );
    assert_eq!(wrong, 0, "{name}: independent expected pixels {expected:?}");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Fragment,
    Deferred,
    Attached,
    ComputeOnly,
    EarlierCompute,
}
#[derive(Clone, Copy, Debug)]
enum Scene {
    World,
    Terrain { hand: bool, water: bool },
    WaterOnly,
}

const VERTEX: &str =
    "#version 120\nvarying vec2 uv;void main(){uv=gl_MultiTexCoord0.xy;gl_Position=ftransform();}";

fn shaders(sampler: &str, stage: Stage, scene: Scene) -> Vec<(String, String)> {
    let names = if sampler == "all" {
        vec!["depthtex0", "depthtex1", "depthtex2"]
    } else {
        vec![sampler]
    };
    let declarations = names
        .iter()
        .map(|s| format!("uniform sampler2D {s};\n"))
        .collect::<String>();
    let sample = |s| {
        if matches!(stage, Stage::Fragment | Stage::Deferred) {
            format!("texture2D({s},uv).r")
        } else {
            format!("texelFetch({s},p,0).r")
        }
    };
    let result = if sampler == "all" {
        format!(
            "vec4({},{},{},1.0)",
            sample(names[0]),
            sample(names[1]),
            sample(names[2])
        )
    } else {
        format!("vec4({},0.25,0.5,1.0)", sample(names[0]))
    };
    let mut files = vec![
        ("final.vsh".into(), VERTEX.into()),
        (
            "shaders.properties".into(),
            "blend.gbuffers_terrain=off\nblend.gbuffers_water=off\nblend.gbuffers_hand=off\n"
                .into(),
        ),
    ];
    if stage == Stage::Fragment {
        files.push((
            "final.fsh".into(),
            format!(
                "#version 120\n{declarations}varying vec2 uv;void main(){{gl_FragColor={result};}}"
            ),
        ));
    } else if stage == Stage::Deferred {
        files.push(("final.fsh".into(), "#version 120\nuniform sampler2D colortex1;varying vec2 uv;void main(){gl_FragColor=texture2D(colortex1,uv);}".into()));
        files.push(("deferred.vsh".into(), VERTEX.into()));
        files.push(("deferred.fsh".into(), format!("#version 120\n/* DRAWBUFFERS:1 */\n{declarations}varying vec2 uv;void main(){{gl_FragData[0]={result};}}")));
    } else {
        files.push(("final.fsh".into(),"#version 120\nuniform sampler2D colortex1;varying vec2 uv;void main(){gl_FragColor=texture2D(colortex1,uv);}".into()));
        files.push(("composite.csh".into(),format!("#version 430\nlayout(local_size_x=1,local_size_y=1)in;const ivec3 workGroups=ivec3(16,16,1);layout(rgba8) writeonly uniform image2D colorimg1;\n{declarations}\nvoid main(){{ivec2 p=ivec2(gl_GlobalInvocationID.xy);imageStore(colorimg1,p,{result});}}")));
        match stage {
            Stage::Attached => {
                files.push(("composite.vsh".into(),VERTEX.into()));
                files.push(("composite.fsh".into(),"#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0);}".into()));
            }
            Stage::EarlierCompute => files.push(("composite_a.csh".into(),"#version 430\nlayout(local_size_x=1)in;const ivec3 workGroups=ivec3(1);layout(rgba8) writeonly uniform image2D colorimg2;void main(){imageStore(colorimg2,ivec2(0),vec4(0.125));}".into())),
            _ => {}
        }
    }
    if !matches!(scene, Scene::World) {
        let hand = matches!(scene, Scene::Terrain { hand: true, .. });
        for (name, clip) in [
            ("gbuffers_terrain", 0.4),
            ("gbuffers_water", -0.2),
            ("gbuffers_hand", -0.6),
        ] {
            if name == "gbuffers_hand" && !hand {
                continue;
            }
            if name == "gbuffers_terrain" && matches!(scene, Scene::WaterOnly) {
                continue;
            }
            files.push((
                format!("{name}.vsh"),
                format!("#version 120\nvoid main(){{gl_Position=vec4(gl_Vertex.xy,{clip},1.0);}}"),
            ));
            files.push((format!("{name}.fsh"),"#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,0.0,0.0,1.0);}".into()));
        }
    }
    files
}

fn check(sampler: &str, stage: Stage, scene: Scene, expected: [u8; 4]) {
    let Some(gpu) = Gpu::new() else {
        eprintln!("DEPTH_FIXTURE_NO_ADAPTER");
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    // A seeded world depth of reverse-Z .3 is GL window depth .7.
    gpu.clear(&color, &depth, 0.3);
    const ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0=>Float32x3];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let terrain = !matches!(scene, Scene::World);
    let hand = matches!(scene, Scene::Terrain { hand: true, .. });
    let water = matches!(scene, Scene::Terrain { water: true, .. } | Scene::WaterOnly);
    let inputs=horizon_shaders::GameInputs {
        terrain:terrain.then(|| horizon_shaders::GbuffersInputs { declarations:"layout(location=0)in vec3 fixture_position;\n".into(),body:"hz_Vertex=vec4(fixture_position,1.0);hz_Color=vec4(1.0);hz_MultiTexCoord0=vec4(0.0);hz_MultiTexCoord1=vec4(240.0);hz_MultiTexCoord2=vec4(240.0);hz_Normal=vec3(0.0,0.0,1.0);\n".into(),attributes:vec![],after:String::new(),alpha_test:None }),
        hand:hand.then(crate::gfx::pack_hand::inputs),..Default::default()
    };
    let mut post = gpu.load(
        shaders(sampler, stage, scene),
        World {
            atlas: None,
            lightmap: color.clone(),
            terrain: terrain.then_some(Geometry {
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                ids: None,
                cull: false,
            }),
            block_ids: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand,
        },
        inputs,
    );
    if terrain {
        assert!(
            post.terrain.is_some() != matches!(scene, Scene::WaterOnly) && post.water.is_some(),
            "{:?}",
            post.notes
        );
    }
    if hand {
        assert!(post.hand.is_some(), "{:?}", post.notes);
    }
    if stage == Stage::Deferred {
        assert!(
            post.passes.iter().any(|p| p.group == Group::Deferred),
            "{:?}",
            post.notes
        );
    } else if stage != Stage::Fragment {
        let count = post
            .passes
            .iter()
            .flat_map(|p| std::iter::once(p).chain(&p.compute))
            .filter(|p| p.name.starts_with("composite"))
            .count();
        let expected = if matches!(stage, Stage::Attached | Stage::EarlierCompute) {
            2
        } else {
            1
        };
        assert_eq!(
            count, expected,
            "all original compute/draw stages must build: {:?}",
            post.notes
        );
    }
    let positions: [[f32; 3]; 6] = [
        [-1., -1., 0.],
        [1., -1., 0.],
        [1., 1., 0.],
        [-1., -1., 0.],
        [1., 1., 0.],
        [-1., 1., 0.],
    ];
    let vertices = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original fixed depth geometry"),
            contents: bytemuck::cast_slice(&positions),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let quad = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| {
        crate::mesher::Vertex {
            pos,
            uv: [0.; 2],
            layer: 0,
            color: u32::MAX,
            light: 0,
            normal: [0, 0, 127, 0],
            glint_uv: [0.; 2],
        }
    });
    let hud = HudBatch::new(SIZE);
    let mut frame = fixture_frame(&hud);
    if hand {
        frame.hand = &quad;
    }
    let actual = gpu.render(&mut post, &color, &depth, &frame, &mut |rp, list, _, _| {
        if list == TerrainList::Solid || (water && list == TerrainList::Translucent) {
            rp.set_vertex_buffer(0, vertices.slice(..));
            rp.draw(0..6, 0..1);
        }
    });
    all_pixels(&actual, expected, &format!("{sampler} {stage:?} {scene:?}"));
}

#[test]
fn hand_is_in_depth1_and_excluded_from_depth2() {
    check(
        "all",
        Stage::Fragment,
        Scene::Terrain {
            hand: true,
            water: true,
        },
        [51, 51, 179, 255],
    );
}
#[test]
fn depth2_only_without_hand_excludes_water() {
    check(
        "depthtex2",
        Stage::Fragment,
        Scene::Terrain {
            hand: false,
            water: true,
        },
        [179, 64, 128, 255],
    );
}
#[test]
fn depth1_only_without_hand_excludes_water_control() {
    check(
        "depthtex1",
        Stage::Fragment,
        Scene::Terrain {
            hand: false,
            water: true,
        },
        [179, 64, 128, 255],
    );
}
#[test]
fn water_only_program_populates_both_opaque_snapshots() {
    check(
        "all",
        Stage::Fragment,
        Scene::WaterOnly,
        [102, 179, 179, 255],
    );
}
#[test]
fn deferred_depth2_only_excludes_hand() {
    check(
        "depthtex2",
        Stage::Deferred,
        Scene::Terrain {
            hand: true,
            water: false,
        },
        [179, 64, 128, 255],
    );
}
#[test]
fn deferred_depth1_contains_hand_control() {
    check(
        "depthtex1",
        Stage::Deferred,
        Scene::Terrain {
            hand: true,
            water: false,
        },
        [51, 64, 128, 255],
    );
}
#[test]
fn direct_fragment_gets_world_depth_control() {
    check(
        "depthtex0",
        Stage::Fragment,
        Scene::World,
        [179, 64, 128, 255],
    );
}
#[test]
fn attached_compute_gets_world_depth() {
    check(
        "depthtex0",
        Stage::Attached,
        Scene::World,
        [179, 64, 128, 255],
    );
}
#[test]
fn compute_only_gets_world_depth_control() {
    check(
        "depthtex0",
        Stage::ComputeOnly,
        Scene::World,
        [179, 64, 128, 255],
    );
}
#[test]
fn earlier_compute_child_gets_world_depth() {
    check(
        "depthtex0",
        Stage::EarlierCompute,
        Scene::World,
        [179, 64, 128, 255],
    );
}
#[test]
fn attached_compute_depth1_without_hand_excludes_water() {
    check(
        "depthtex1",
        Stage::Attached,
        Scene::Terrain {
            hand: false,
            water: true,
        },
        [179, 64, 128, 255],
    );
}
#[test]
fn attached_compute_depth2_without_hand_excludes_water() {
    check(
        "depthtex2",
        Stage::Attached,
        Scene::Terrain {
            hand: false,
            water: true,
        },
        [179, 64, 128, 255],
    );
}
#[test]
fn attached_compute_preserves_distinct_hand_snapshots() {
    check(
        "all",
        Stage::Attached,
        Scene::Terrain {
            hand: true,
            water: true,
        },
        [51, 51, 179, 255],
    );
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
