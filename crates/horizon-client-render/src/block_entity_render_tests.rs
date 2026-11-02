use crate::{
    block_entity_render::{
        BlockAnim, BlockEntityRenderer, BlockEntityShapes, CacheOptions, ChestPart, Kind,
        ModelPart, SIGN_TEXT_KEEP, Scene, StaticSectionRevision, classify,
    },
    chunk::Chunk,
    hud::Rect,
    mesher::Vertex,
    signs::{SignData, SignText},
    world::World,
};
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_world::registry::BlockStateId;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::Arc,
};

fn unit_pose() -> [f32; 9] {
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
}

fn quad() -> [[f32; 5]; 4] {
    [
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [16.0, 0.0, 0.0, 1.0, 0.0],
        [16.0, 16.0, 0.0, 1.0, 1.0],
        [0.0, 16.0, 0.0, 0.0, 1.0],
    ]
}

fn horizontal_quad() -> [[f32; 5]; 4] {
    [
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [16.0, 0.0, 0.0, 1.0, 0.0],
        [16.0, 0.0, 16.0, 1.0, 1.0],
        [0.0, 0.0, 16.0, 0.0, 1.0],
    ]
}

fn part(y: f32) -> ModelPart {
    ModelPart {
        pose: [0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: vec![quad()],
        children: Vec::new(),
    }
}

fn renderer_with_shapes(
    models: HashMap<&'static str, ModelPart>,
    kinds: Vec<Kind>,
    pot_patterns: HashMap<String, String>,
) -> BlockEntityRenderer {
    let vanilla = Arc::new(BlockEntityShapes::new(models, HashMap::new()));
    BlockEntityRenderer {
        vanilla: vanilla.clone(),
        shapes: std::sync::RwLock::new(vanilla),
        kinds,
        pot_patterns,
        ..Default::default()
    }
}

fn shulker_renderer() -> BlockEntityRenderer {
    let model = ModelPart {
        pose: unit_pose(),
        polygons: Vec::new(),
        children: vec![("base".into(), part(0.0)), ("lid".into(), part(24.0))],
    };
    renderer_with_shapes(
        HashMap::from([("shulker_box", model)]),
        vec![Kind::Shulker {
            texture: "entity/shulker/shulker".into(),
            facing: "up",
        }],
        HashMap::new(),
    )
}

fn bell_renderer() -> BlockEntityRenderer {
    let model = ModelPart {
        pose: unit_pose(),
        polygons: Vec::new(),
        children: vec![("bell_body".into(), part(0.0))],
    };
    renderer_with_shapes(
        HashMap::from([("bell", model)]),
        vec![Kind::Bell],
        HashMap::new(),
    )
}

fn chest_model(offset: f32) -> ModelPart {
    let chest_part = |y| ModelPart {
        pose: [0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: vec![quad(), horizontal_quad()],
        children: Vec::new(),
    };
    ModelPart {
        pose: unit_pose(),
        polygons: Vec::new(),
        children: vec![
            ("base".into(), chest_part(offset)),
            ("lid".into(), chest_part(offset + 8.0)),
            ("lock".into(), chest_part(offset + 8.0)),
        ],
    }
}

fn chest_renderer_with(kinds: Vec<Kind>) -> BlockEntityRenderer {
    let models = HashMap::from([
        ("chest", chest_model(0.0)),
        ("double_chest_left", chest_model(1.0)),
        ("double_chest_right", chest_model(2.0)),
    ]);
    // Shape construction precomputes closed-chest geometry so the fixture covers
    // the cached world path as well as the direct model path.
    renderer_with_shapes(models, kinds, HashMap::new())
}

fn chest_renderer() -> BlockEntityRenderer {
    chest_renderer_with(vec![Kind::Chest {
        block: "chest".into(),
        texture: "entity/chest/normal",
        part: ChestPart::Single,
        y_rot: 90.0,
        double_with: None,
    }])
}

fn classified_chest(name: &str, facing: &str, part: &str) -> Kind {
    classify(name, &|property| match property {
        "facing" => Some(facing.to_owned()),
        "type" => Some(part.to_owned()),
        _ => None,
    })
}

fn decorated_pot_renderer() -> BlockEntityRenderer {
    let sides = ModelPart {
        pose: unit_pose(),
        polygons: Vec::new(),
        children: ["back", "left", "right", "front"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name.into(), part(i as f32)))
            .collect(),
    };
    renderer_with_shapes(
        HashMap::from([
            ("decorated_pot_base", part(0.0)),
            ("decorated_pot_sides", sides),
        ]),
        vec![Kind::DecoratedPot { y_rot: 0.0 }],
        ["back", "left", "right", "front"]
            .into_iter()
            .map(|side| {
                (
                    format!("{side}_pattern"),
                    format!("entity/decorated_pot/{side}_pattern"),
                )
            })
            .collect(),
    )
}

fn banner_renderer() -> BlockEntityRenderer {
    let banner_model = |name: &str| ModelPart {
        pose: unit_pose(),
        polygons: Vec::new(),
        children: vec![(name.into(), part(0.0))],
    };
    let standing = classify("minecraft:red_banner", &|property| match property {
        "rotation" => Some("0".into()),
        _ => None,
    });
    let wall = classify("minecraft:red_wall_banner", &|property| match property {
        "facing" => Some("north".into()),
        _ => None,
    });
    renderer_with_shapes(
        HashMap::from([
            ("standing_banner", banner_model("pole")),
            ("standing_banner_flag", banner_model("flag")),
            ("wall_banner", banner_model("pole")),
            ("wall_banner_flag", banner_model("flag")),
        ]),
        vec![standing, wall],
        HashMap::new(),
    )
}

fn copper_statue_renderer() -> BlockEntityRenderer {
    let model = |copies: usize| ModelPart {
        pose: unit_pose(),
        polygons: vec![quad(); copies],
        children: Vec::new(),
    };
    renderer_with_shapes(
        HashMap::from([
            ("copper_golem", model(1)),
            ("copper_golem_sitting", model(2)),
        ]),
        vec![
            Kind::CopperGolem {
                texture: "entity/copper_golem/copper_golem",
                pose: "standing",
                facing: "north",
            },
            Kind::CopperGolem {
                texture: "entity/copper_golem/copper_golem",
                pose: "sitting",
                facing: "north",
            },
        ],
        HashMap::new(),
    )
}

fn sprites(texture: &str) -> HashMap<String, Rect> {
    HashMap::from([(
        texture.to_owned(),
        Rect {
            u0: 0.125,
            v0: 0.25,
            u1: 0.375,
            v1: 0.5,
            w: 16,
            h: 16,
        },
    )])
}

fn sprite_atlas(textures: &[&str]) -> HashMap<String, Rect> {
    let count = textures.len() as f32;
    textures
        .iter()
        .enumerate()
        .map(|(index, texture)| {
            let u0 = index as f32 / count;
            let u1 = (index + 1) as f32 / count;
            (
                (*texture).to_owned(),
                Rect {
                    u0,
                    v0: 0.0,
                    u1,
                    v1: 1.0,
                    w: 16,
                    h: 16,
                },
            )
        })
        .collect()
}

fn render_direct(
    renderer: &BlockEntityRenderer,
    scene: &Scene,
    blocks: &[([i32; 3], u32)],
) -> Vec<Vertex> {
    let mut vertices = Vec::new();
    for &(pos, state) in blocks {
        let origin = (DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
            - scene.camera)
            .as_vec3();
        renderer.render_block(
            scene,
            pos,
            state,
            Mat4::from_translation(origin),
            scene.world.light(pos[0], pos[1], pos[2]),
            &mut vertices,
        );
    }
    vertices
}

fn assert_same_vertices(actual: &[Vertex], expected: &[Vertex]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (Vec3::from_array(actual.pos) - Vec3::from_array(expected.pos)).length() < 1e-5,
            "positions differ: {:?} != {:?}",
            actual.pos,
            expected.pos
        );
        assert_eq!(
            (
                actual.uv,
                actual.layer,
                actual.color,
                actual.light,
                actual.normal,
                actual.glint_uv,
            ),
            (
                expected.uv,
                expected.layer,
                expected.color,
                expected.light,
                expected.normal,
                expected.glint_uv,
            )
        );
    }
}

fn assert_vertices_move(first: &[Vertex], second: &[Vertex]) {
    assert_eq!(first.len(), second.len());
    assert!(
        first
            .iter()
            .zip(second)
            .any(|(a, b)| (Vec3::from_array(a.pos) - Vec3::from_array(b.pos)).length() > 1e-4),
        "animated frames unexpectedly have identical geometry"
    );
}

fn assert_vertices_differ(first: &[Vertex], second: &[Vertex]) {
    assert_eq!(first.len(), second.len());
    assert!(
        first.iter().zip(second).any(|(a, b)| {
            (Vec3::from_array(a.pos) - Vec3::from_array(b.pos)).length() > 1e-4
                || a.uv != b.uv
                || a.color != b.color
                || a.light != b.light
        }),
        "expected at least one vertex attribute to change"
    );
}

#[test]
fn batched_callbacks_preserve_entity_ranges_and_allow_immediate_culling() {
    let renderer = shulker_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprites("entity/shulker/shulker");
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(3.25, 64.5, -4.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    // Reverse local-position order to check that cache indexing does not
    // change the order of emitted entities.
    let blocks = [([1, 64, 0], 0), ([0, 64, 0], 0)];
    let options = CacheOptions {
        shulker_boxes: true,
        ..Default::default()
    };
    let mut expected = Vec::new();
    renderer.render(&scene, blocks.iter(), &mut expected);

    let before = RefCell::new(Vec::new());
    let after = RefCell::new(Vec::new());
    let mut actual = Vec::new();
    renderer.render_with_cache_options_and_callbacks(
        &scene,
        blocks.iter(),
        options,
        0,
        None,
        &mut actual,
        |pos, state, start| before.borrow_mut().push((pos, state, start)),
        |pos, state, start, out| after.borrow_mut().push((pos, state, start, out.len())),
    );
    assert_same_vertices(&actual, &expected);
    assert_eq!(
        *before.borrow(),
        vec![(blocks[0].0, 0, 0), (blocks[1].0, 0, 8)]
    );
    assert_eq!(
        *after.borrow(),
        vec![(blocks[0].0, 0, 0, 8), (blocks[1].0, 0, 8, 16)]
    );
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 2);

    // The culler may discard the first entity before the next one is drawn.
    // The second callback must then see the compacted output's new start.
    let starts = RefCell::new(Vec::new());
    let ranges = RefCell::new(Vec::new());
    actual.clear();
    renderer.render_with_cache_options_and_callbacks(
        &scene,
        blocks.iter(),
        options,
        0,
        None,
        &mut actual,
        |_, _, start| starts.borrow_mut().push(start),
        |pos, _, start, out| {
            ranges.borrow_mut().push((start, out.len()));
            if pos == blocks[0].0 {
                out.truncate(start);
            }
        },
    );
    assert_eq!(*starts.borrow(), vec![0, 0]);
    assert_eq!(*ranges.borrow(), vec![(0, 8), (0, 8)]);
    let mut second_only = Vec::new();
    renderer.render(&scene, std::iter::once(&blocks[1]), &mut second_only);
    assert_same_vertices(&actual, &second_only);
}

#[test]
fn static_cache_qualifies_local_positions_by_section_and_drops_old_entries() {
    let renderer = shulker_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprites("entity/shulker/shulker");
    let anim = |_: [i32; 3]| BlockAnim::default();
    let mut scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let options = CacheOptions {
        shulker_boxes: true,
        ..Default::default()
    };
    let same_local_index = [([0, 64, 0], 0), ([0, 80, 0], 0)];
    let mut both = Vec::new();
    renderer.render_with_cache_options(&scene, same_local_index.iter(), options, &mut both);
    assert_same_vertices(&both, &render_direct(&renderer, &scene, &same_local_index));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 2);

    scene.game_time = SIGN_TEXT_KEEP + 1;
    let one = [same_local_index[0]];
    let mut retained = Vec::new();
    renderer.render_with_cache_options(&scene, one.iter(), options, &mut retained);
    assert_same_vertices(&retained, &render_direct(&renderer, &scene, &one));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
}

#[test]
fn gpu_static_cache_uses_section_local_chests_and_keeps_visibility_separate() {
    let renderer = chest_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprites("entity/chest/normal");
    let opaque_sprites = HashSet::new();
    let anim = |_: [i32; 3]| BlockAnim::default();
    let openness = Cell::new(0.0);
    let lid = |pos: [i32; 3]| {
        if pos == [100, 70, -32] {
            openness.get()
        } else {
            0.0
        }
    };
    let mut scene = Scene {
        camera: DVec3::new(99.25, 70.5, -31.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &opaque_sprites,
        atlas_revision: 1,
        nether: false,
        lid: &lid,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([100, 70, -32], 0)];
    let other_block = [([116, 70, -32], 0)];
    let candidates = [blocks[0], other_block[0]];
    let first_section_membership_revision = Cell::new(1);
    let membership_revision = Cell::new(1);
    let options = CacheOptions {
        chests: true,
        ..Default::default()
    };
    let render_static = |scene: &Scene,
                         visible: &[([i32; 3], u32)],
                         candidates: &[([i32; 3], u32)]| {
        let mut dynamic = Vec::new();
        let (static_sections, visible_sections, stats) = renderer.render_with_gpu_cache_options(
            scene,
            visible.iter(),
            candidates.iter(),
            options,
            membership_revision.get(),
            |pos| {
                Some(StaticSectionRevision {
                    mesh: 1,
                    block_entities: if pos[0] == 6 {
                        first_section_membership_revision.get()
                    } else {
                        1
                    },
                })
            },
            &mut dynamic,
        );
        (dynamic, static_sections, visible_sections, stats)
    };
    let (dynamic, first, first_visible, first_stats) = render_static(&scene, &blocks, &candidates);
    assert!(dynamic.is_empty());
    assert_eq!(first.len(), 2);
    assert_eq!(first_visible.len(), 1);
    assert_eq!(first_stats.snapshot_rebuilds, 2);
    let camera_relative = first[0]
        .vertices
        .iter()
        .map(|vertex| Vertex {
            pos: std::array::from_fn(|i| {
                vertex.pos[i] + (f64::from(first[0].origin[i]) - scene.camera.to_array()[i]) as f32
            }),
            ..*vertex
        })
        .collect::<Vec<_>>();
    assert_same_vertices(&camera_relative, &render_direct(&renderer, &scene, &blocks));

    let (_, same, _, same_stats) = render_static(&scene, &blocks, &candidates);
    assert!(Arc::ptr_eq(&same, &first));
    assert_eq!(same_stats.snapshot_rebuilds, 0);

    // Camera translation only updates a per-section transform uniform.
    scene.camera = DVec3::new(99.75, 70.9, -31.2);
    let (_, within_block, _, within_stats) = render_static(&scene, &blocks, &candidates);
    assert!(Arc::ptr_eq(&within_block, &first));
    assert_eq!(within_stats.snapshot_rebuilds, 0);

    // Crossing a block boundary does not rebuild section-local
    // vertex data. The transform remains camera-relative in f64.
    scene.camera.x += 1.0;
    let (_, moved, _, moved_stats) = render_static(&scene, &blocks, &candidates);
    assert!(Arc::ptr_eq(&moved, &first));
    assert_eq!(moved_stats.snapshot_rebuilds, 0);
    let camera_relative = moved[0]
        .vertices
        .iter()
        .map(|vertex| Vertex {
            pos: std::array::from_fn(|i| {
                vertex.pos[i] + (f64::from(moved[0].origin[i]) - scene.camera.to_array()[i]) as f32
            }),
            ..*vertex
        })
        .collect::<Vec<_>>();
    assert_same_vertices(&camera_relative, &render_direct(&renderer, &scene, &blocks));

    // Frustum changes only the visible section list, not the cached snapshot.
    let (_, hidden, hidden_visible, hidden_stats) = render_static(&scene, &[], &candidates);
    assert_eq!(hidden_visible.len(), 0);
    assert!(Arc::ptr_eq(&hidden, &first));
    assert_eq!(hidden_stats.snapshot_rebuilds, 0);

    // Turning to a different prewarmed section changes which buffer draws but
    // leaves both section snapshots untouched.
    let (_, rotated, rotated_visible, rotated_stats) =
        render_static(&scene, &other_block, &candidates);
    assert_eq!(rotated_visible.len(), 1);
    assert!(Arc::ptr_eq(&rotated, &first));
    assert_eq!(rotated_stats.snapshot_rebuilds, 0);

    // Removing the last block entity in one section invalidates only that
    // section, even though the next candidate scan has no entry for it.
    first_section_membership_revision.set(2);
    membership_revision.set(2);
    let (_, after_removal, remaining_visible, removal_stats) =
        render_static(&scene, &other_block, &other_block);
    assert_eq!(after_removal.len(), 1);
    assert_eq!(remaining_visible, rotated_visible);
    assert_eq!(removal_stats.snapshot_rebuilds, 0);
    openness.set(0.3);
    let (opening_dynamic, opening_static, opening_visible, opening_stats) =
        render_static(&scene, &blocks, &candidates);
    assert!(!opening_dynamic.is_empty());
    assert!(opening_visible.is_empty());
    assert_eq!(opening_static.len(), 1);
    // The opened chest's section had already been removed above. It stays on
    // the dynamic path, so opening it does not rebuild the remaining section.
    assert_eq!(opening_stats.snapshot_rebuilds, 0);
    assert_same_vertices(&opening_dynamic, &render_direct(&renderer, &scene, &blocks));

    // A resource reload invalidates cached UV geometry even when it reuses the
    // same atlas coordinates.
    scene.atlas_revision = 2;
    let (_, reloaded, _, reload_stats) = render_static(&scene, &blocks, &candidates);
    assert_eq!(reload_stats.snapshot_rebuilds, 1);
    assert!(!Arc::ptr_eq(&reloaded, &opening_static));

    // A resource-pack sprite with fractional alpha must use the ordinary
    // blended entity path instead of the no-blend cutout snapshot.
    let translucent_sprites = HashSet::from(["entity/chest/normal".to_owned()]);
    scene.translucent_sprites = &translucent_sprites;
    scene.atlas_revision = 3;
    let (translucent_dynamic, translucent_static, _, _) =
        render_static(&scene, &other_block, &candidates);
    assert!(!translucent_dynamic.is_empty());
    assert!(translucent_static.is_empty());
}

#[test]
fn sign_text_cache_can_be_disabled_and_reenabled_without_changing_output() {
    let renderer = BlockEntityRenderer {
        kinds: vec![Kind::Sign {
            hanging: false,
            wall: false,
            angle: 0.0,
        }],
        ..Default::default()
    };
    let world = World::new(BlockStateId(0), 1);
    let sprites = HashMap::new();
    let data = SignData {
        front: SignText {
            lines: ["cached".into(), "text".into(), String::new(), String::new()],
            ..Default::default()
        },
        ..Default::default()
    };
    let sign_data = &data;
    let sign = move |_: [i32; 3]| Some(sign_data);
    let glyph = |_: char| {
        (
            Some(Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 0.0625,
                v1: 0.0625,
                w: 8,
                h: 8,
            }),
            6.0,
        )
    };
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &|_| BlockAnim::default(),
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &sign,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &glyph,
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let enabled = CacheOptions {
        signs: true,
        ..Default::default()
    };
    let disabled = CacheOptions::default();
    let mut cached = Vec::new();
    renderer.render_with_cache_options(&scene, blocks.iter(), enabled, &mut cached);
    assert_same_vertices(&cached, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.sign_texts.lock().unwrap().signs.len(), 1);

    let mut uncached = Vec::new();
    renderer.render_with_cache_options(&scene, blocks.iter(), disabled, &mut uncached);
    assert_same_vertices(&uncached, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.sign_texts.lock().unwrap().signs.len(), 0);

    let mut recached = Vec::new();
    renderer.render_with_cache_options(&scene, blocks.iter(), enabled, &mut recached);
    assert_same_vertices(&recached, &uncached);
    assert_eq!(renderer.sign_texts.lock().unwrap().signs.len(), 1);
}

#[test]
fn shulker_cache_tracks_settled_poses_and_does_not_freeze_opening_animation() {
    let renderer = shulker_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprites("entity/shulker/shulker");
    let pose = Cell::new((0.0, 0.0));
    let anim = |_: [i32; 3]| BlockAnim {
        shulker: pose.get(),
        ..Default::default()
    };
    let scene = Scene {
        camera: DVec3::new(2.0, 64.0, -3.0),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let options = CacheOptions {
        shulker_boxes: true,
        ..Default::default()
    };
    let render_cached = |scene: &Scene| {
        let mut vertices = Vec::new();
        renderer.render_with_cache_options(scene, blocks.iter(), options, &mut vertices);
        vertices
    };
    let render_uncached = |scene: &Scene| {
        let mut vertices = Vec::new();
        renderer.render(scene, blocks.iter(), &mut vertices);
        vertices
    };

    let closed = render_cached(&scene);
    assert_same_vertices(&closed, &render_uncached(&scene));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    pose.set((0.25, 0.0));
    let opening_early = render_cached(&scene);
    assert_same_vertices(&opening_early, &render_uncached(&scene));
    pose.set((0.75, 0.0));
    let opening_late = render_cached(&scene);
    assert_vertices_move(&closed, &opening_early);
    assert_vertices_move(&opening_early, &opening_late);
    assert_same_vertices(&opening_late, &render_uncached(&scene));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 0);

    pose.set((1.0, 1.0));
    let open = render_cached(&scene);
    assert_vertices_move(&opening_late, &open);
    assert_same_vertices(&open, &render_uncached(&scene));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
    assert_same_vertices(&open, &render_cached(&scene));
}

#[test]
fn shulker_cache_covers_all_colors_and_facing_directions() {
    let colors = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let facings = ["up", "down", "north", "south", "west", "east"];
    let kinds = colors
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let name = format!("minecraft:{color}_shulker_box");
            let facing = facings[index % facings.len()];
            classify(&name, &|property| {
                (property == "facing").then(|| facing.to_owned())
            })
        })
        .collect();
    let mut renderer = shulker_renderer();
    renderer.kinds = kinds;
    let textures: Vec<_> = colors
        .iter()
        .map(|color| format!("entity/shulker/shulker_{color}"))
        .collect();
    let texture_refs: Vec<_> = textures.iter().map(String::as_str).collect();
    let sprites = sprite_atlas(&texture_refs);
    let world = World::new(BlockStateId(0), 1);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let options = CacheOptions {
        shulker_boxes: true,
        ..Default::default()
    };
    let pos = [0, 64, 0];
    let mut previous: Option<Vec<Vertex>> = None;
    for state in 0..colors.len() as u32 {
        let blocks = [(pos, state)];
        let mut cached = Vec::new();
        renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut cached);
        assert_same_vertices(&cached, &render_direct(&renderer, &scene, &blocks));
        let sprite = sprites.get(&textures[state as usize]).unwrap();
        assert_eq!(cached[0].uv, [sprite.u0, sprite.v0]);
        if let Some(previous) = &previous {
            assert_vertices_differ(previous, &cached);
        }
        previous = Some(cached);
        assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
    }
}

#[test]
fn cached_bells_keep_shaking_until_the_animation_ends() {
    let renderer = bell_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprites("entity/bell/bell_body");
    let ticks = Cell::new(2);
    let shaking = Cell::new(true);
    let anim = |_: [i32; 3]| BlockAnim {
        bell_ticks: ticks.get(),
        bell_shake: shaking.get().then_some("north"),
        ..Default::default()
    };
    let scene = Scene {
        camera: DVec3::new(2.0, 64.0, -3.0),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 0.25,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let options = CacheOptions {
        bells: true,
        ..Default::default()
    };
    let render = || {
        let mut vertices = Vec::new();
        renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut vertices);
        vertices
    };

    let first_shake = render();
    ticks.set(7);
    let second_shake = render();
    assert_vertices_move(&first_shake, &second_shake);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 0);
    assert_same_vertices(&second_shake, &{
        let mut expected = Vec::new();
        renderer.render(&scene, blocks.iter(), &mut expected);
        expected
    });

    shaking.set(false);
    let settled = render();
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
    shaking.set(true);
    ticks.set(11);
    let resumed_shake = render();
    assert_vertices_move(&settled, &resumed_shake);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 0);
}

#[test]
fn cached_closed_chests_match_the_direct_model_and_rebuild_when_opening() {
    let renderer = chest_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprite_atlas(&["entity/chest/normal"]);
    let opening = Cell::new(0.0);
    let lid = |_: [i32; 3]| opening.get();
    let anim = |_: [i32; 3]| BlockAnim::default();
    let mut scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &lid,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let options = CacheOptions {
        chests: true,
        ..Default::default()
    };
    let render_cached = |scene: &Scene| {
        let mut vertices = Vec::new();
        renderer.render_with_cache_options(scene, blocks.iter(), options, &mut vertices);
        vertices
    };

    let closed = render_cached(&scene);
    assert_same_vertices(&closed, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
    assert_same_vertices(&closed, &render_cached(&scene));

    opening.set(0.65);
    let opening_frame = render_cached(&scene);
    assert_same_vertices(&opening_frame, &render_direct(&renderer, &scene, &blocks));
    assert_vertices_move(&closed, &opening_frame);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 0);

    opening.set(0.0);
    let closed_again = render_cached(&scene);
    assert_same_vertices(&closed_again, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
    scene.nether = true;
    let nether_closed = render_cached(&scene);
    assert_same_vertices(&nether_closed, &render_direct(&renderer, &scene, &blocks));
    assert_vertices_differ(&closed_again, &nether_closed);
}

#[test]
fn closed_chest_cache_covers_all_materials_and_atlas_reloads() {
    let variants = [
        ("minecraft:chest", "entity/chest/normal"),
        ("minecraft:trapped_chest", "entity/chest/trapped"),
        ("minecraft:ender_chest", "entity/chest/ender"),
        ("minecraft:copper_chest", "entity/chest/copper"),
        (
            "minecraft:exposed_copper_chest",
            "entity/chest/copper_exposed",
        ),
        (
            "minecraft:weathered_copper_chest",
            "entity/chest/copper_weathered",
        ),
        (
            "minecraft:oxidized_copper_chest",
            "entity/chest/copper_oxidized",
        ),
        ("minecraft:waxed_copper_chest", "entity/chest/copper"),
        (
            "minecraft:waxed_exposed_copper_chest",
            "entity/chest/copper_exposed",
        ),
        (
            "minecraft:waxed_weathered_copper_chest",
            "entity/chest/copper_weathered",
        ),
        (
            "minecraft:waxed_oxidized_copper_chest",
            "entity/chest/copper_oxidized",
        ),
    ];
    let kinds = variants
        .iter()
        .map(|(name, _)| classified_chest(name, "east", "single"))
        .collect();
    let renderer = chest_renderer_with(kinds);
    let textures: Vec<_> = variants.iter().map(|(_, texture)| *texture).collect();
    let sprites = sprite_atlas(&textures);
    let world = World::new(BlockStateId(99), 1);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let mut scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let pos = [0, 64, 0];
    let options = CacheOptions {
        chests: true,
        ..Default::default()
    };
    let mut previous_texture = None;
    let mut previous_uv = None;
    for (state, (_, texture)) in variants.iter().enumerate() {
        let blocks = [(pos, state as u32)];
        let mut cached = Vec::new();
        renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut cached);
        assert_same_vertices(&cached, &render_direct(&renderer, &scene, &blocks));
        let sprite = sprites.get(*texture).unwrap();
        assert_eq!(cached[0].uv, [sprite.u0, sprite.v0]);
        assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
        if previous_texture != Some(*texture) {
            assert_ne!(previous_uv, Some(cached[0].uv));
        }
        previous_texture = Some(*texture);
        previous_uv = Some(cached[0].uv);
    }

    // A resource reload may move the same sprite without changing block state.
    // Cached UVs must follow the new atlas rectangle.
    let changed = HashMap::from([(
        "entity/chest/copper_oxidized".to_owned(),
        Rect {
            u0: 0.75,
            v0: 0.5,
            u1: 1.0,
            v1: 0.75,
            w: 16,
            h: 16,
        },
    )]);
    let state = variants.len() as u32 - 1;
    let blocks = [(pos, state)];
    let old_uv = previous_uv.unwrap();
    scene.sprites = &changed;
    let mut after_reload = Vec::new();
    renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut after_reload);
    assert_same_vertices(&after_reload, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(after_reload[0].uv, [0.75, 0.5]);
    assert_ne!(old_uv, after_reload[0].uv);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
}

#[test]
fn double_chest_cache_bypasses_both_halves_while_either_lid_is_open() {
    let renderer = chest_renderer_with(vec![
        classified_chest("minecraft:chest", "north", "left"),
        classified_chest("minecraft:chest", "north", "right"),
        classified_chest("minecraft:trapped_chest", "north", "right"),
        classified_chest("minecraft:chest", "south", "right"),
    ]);
    let mut world = World::new(BlockStateId(99), 1);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    world.set(0, 64, 0, BlockStateId(0)).unwrap();
    world.set(1, 64, 0, BlockStateId(1)).unwrap();
    let sprites = sprite_atlas(&[
        "entity/chest/normal_left",
        "entity/chest/normal_right",
        "entity/chest/trapped_right",
    ]);
    let openings = Cell::new([0.0, 0.0]);
    let lid = |pos: [i32; 3]| openings.get()[usize::from(pos[0] != 0)];
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &lid,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let pair = [([0, 64, 0], 0), ([1, 64, 0], 1)];
    let options = CacheOptions {
        chests: true,
        ..Default::default()
    };
    let mut closed = Vec::new();
    renderer.render_with_cache_options(&scene, pair.iter(), options, &mut closed);
    assert_same_vertices(&closed, &render_direct(&renderer, &scene, &pair));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 2);

    openings.set([0.0, 0.6]);
    let left = [pair[0]];
    let mut opening_neighbor = Vec::new();
    renderer.render_with_cache_options(&scene, left.iter(), options, &mut opening_neighbor);
    assert_same_vertices(&opening_neighbor, &render_direct(&renderer, &scene, &left));
    assert_vertices_move(&closed[..24], &opening_neighbor);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    openings.set([0.0, 0.0]);
    let mut closed_again = Vec::new();
    renderer.render_with_cache_options(&scene, pair.iter(), options, &mut closed_again);
    assert_same_vertices(&closed_again, &render_direct(&renderer, &scene, &pair));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 2);

    let left_kind = renderer.chest_openness_of(
        pair[0].0,
        pair[0].1,
        &|pos| if pos[0] == 1 { 2 } else { 99 },
        &|pos| if pos[0] == 0 { 0.0 } else { 0.8 },
    );
    assert_eq!(left_kind, (0.0, None), "a trapped chest is not its partner");
    let wrong_facing = renderer.chest_openness_of(
        pair[0].0,
        pair[0].1,
        &|pos| if pos[0] == 1 { 3 } else { 99 },
        &|pos| if pos[0] == 0 { 0.0 } else { 0.8 },
    );
    assert_eq!(wrong_facing, (0.0, None), "mismatched facings do not pair");
}

#[test]
fn decorated_pot_cache_tracks_sherds_and_bypasses_wobble_animation() {
    let renderer = decorated_pot_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprite_atlas(&[
        "entity/decorated_pot/decorated_pot_base",
        "entity/decorated_pot/decorated_pot_side",
        "entity/decorated_pot/back_pattern",
        "entity/decorated_pot/left_pattern",
        "entity/decorated_pot/right_pattern",
        "entity/decorated_pot/front_pattern",
    ]);
    let animation = RefCell::new(BlockAnim::default());
    let anim = |_: [i32; 3]| animation.borrow().clone();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let options = CacheOptions {
        decorated_pots: true,
        ..Default::default()
    };
    let render_cached = || {
        let mut vertices = Vec::new();
        renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut vertices);
        vertices
    };

    let plain = render_cached();
    assert_same_vertices(&plain, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    animation.borrow_mut().sherds = [
        Some("back_pattern".into()),
        Some("left_pattern".into()),
        Some("right_pattern".into()),
        Some("front_pattern".into()),
    ];
    let patterned = render_cached();
    assert_same_vertices(&patterned, &render_direct(&renderer, &scene, &blocks));
    assert_vertices_differ(&plain, &patterned);
    for (side, pattern) in ["back", "left", "right", "front"].iter().enumerate() {
        let sprite = sprites
            .get(&format!("entity/decorated_pot/{pattern}_pattern"))
            .unwrap();
        assert_eq!(
            patterned[(side + 1) * 4].uv,
            [sprite.u0, sprite.v0],
            "the {pattern} sherd uses its own sprite"
        );
        assert_ne!(plain[(side + 1) * 4].uv, patterned[(side + 1) * 4].uv);
    }
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    animation.borrow_mut().wobble = Some((true, 0.5));
    let wobbling = render_cached();
    assert_same_vertices(&wobbling, &render_direct(&renderer, &scene, &blocks));
    assert_vertices_move(&patterned, &wobbling);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 0);
}

#[test]
fn banner_cache_keeps_the_pole_static_while_the_flag_and_patterns_update() {
    let renderer = banner_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprite_atlas(&[
        "entity/banner/banner_base",
        "entity/banner/base",
        "entity/banner/pattern",
        "entity/banner/alternate_pattern",
    ]);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let patterns = RefCell::new(vec![("entity/banner/pattern".to_owned(), 0x123456)]);
    let banner_patterns = |_: [i32; 3]| patterns.borrow().clone();
    let mut scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &banner_patterns,
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let blocks = [([0, 64, 0], 0)];
    let options = CacheOptions {
        banners: true,
        ..Default::default()
    };
    let render_cached = |scene: &Scene| {
        let mut vertices = Vec::new();
        renderer.render_with_cache_options(scene, blocks.iter(), options, &mut vertices);
        vertices
    };

    let first = render_cached(&scene);
    assert_eq!(
        first.len(),
        16,
        "pole, flag, base pattern, and overlay pattern"
    );
    assert_same_vertices(&first, &render_direct(&renderer, &scene, &blocks));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    scene.game_time = 25;
    let next_frame = render_cached(&scene);
    assert_same_vertices(&next_frame, &render_direct(&renderer, &scene, &blocks));
    assert_same_vertices(&first[..4], &next_frame[..4]);
    let first_pattern = sprites.get("entity/banner/pattern").unwrap();
    assert_eq!(next_frame[12].uv, [first_pattern.u0, first_pattern.v0]);
    assert_vertices_move(&first[4..], &next_frame[4..]);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    patterns.borrow_mut()[0] = ("entity/banner/alternate_pattern".to_owned(), 0xABCDEF);
    let new_pattern = render_cached(&scene);
    assert_same_vertices(&new_pattern, &render_direct(&renderer, &scene, &blocks));
    assert_same_vertices(&next_frame[..4], &new_pattern[..4]);
    let alternate = sprites.get("entity/banner/alternate_pattern").unwrap();
    assert_eq!(new_pattern[12].uv, [alternate.u0, alternate.v0]);
    assert_vertices_differ(&next_frame[4..], &new_pattern[4..]);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
}

#[test]
fn cached_wall_banner_poles_use_the_wall_model() {
    let renderer = banner_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprite_atlas(&["entity/banner/banner_base", "entity/banner/base"]);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let pos = [0, 64, 0];
    let options = CacheOptions {
        banners: true,
        ..Default::default()
    };
    let standing = [(pos, 0)];
    let wall = [(pos, 1)];
    let mut cached_standing = Vec::new();
    renderer.render_with_cache_options(&scene, standing.iter(), options, &mut cached_standing);
    assert_same_vertices(
        &cached_standing,
        &render_direct(&renderer, &scene, &standing),
    );
    let mut cached_wall = Vec::new();
    renderer.render_with_cache_options(&scene, wall.iter(), options, &mut cached_wall);
    assert_same_vertices(&cached_wall, &render_direct(&renderer, &scene, &wall));
    assert_vertices_differ(&cached_standing[..4], &cached_wall[..4]);
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
}

#[test]
fn copper_statue_cache_rebuilds_when_its_pose_state_changes() {
    let renderer = copper_statue_renderer();
    let world = World::new(BlockStateId(0), 1);
    let sprites = sprite_atlas(&["entity/copper_golem/copper_golem"]);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let options = CacheOptions {
        copper_golem_statues: true,
        ..Default::default()
    };
    let standing = [([0, 64, 0], 0)];
    let sitting = [([0, 64, 0], 1)];
    let mut cached = Vec::new();
    renderer.render_with_cache_options(&scene, standing.iter(), options, &mut cached);
    assert_same_vertices(&cached, &render_direct(&renderer, &scene, &standing));
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);

    cached.clear();
    renderer.render_with_cache_options(&scene, sitting.iter(), options, &mut cached);
    assert_same_vertices(&cached, &render_direct(&renderer, &scene, &sitting));
    assert_eq!(cached.len(), 8, "the sitting model has two fixture quads");
    assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
}

#[test]
fn copper_statue_cache_covers_weathering_waxing_and_all_poses() {
    let variants = [
        ("copper_golem_statue", "entity/copper_golem/copper_golem"),
        (
            "exposed_copper_golem_statue",
            "entity/copper_golem/copper_golem_exposed",
        ),
        (
            "weathered_copper_golem_statue",
            "entity/copper_golem/copper_golem_weathered",
        ),
        (
            "oxidized_copper_golem_statue",
            "entity/copper_golem/copper_golem_oxidized",
        ),
        (
            "waxed_copper_golem_statue",
            "entity/copper_golem/copper_golem",
        ),
        (
            "waxed_exposed_copper_golem_statue",
            "entity/copper_golem/copper_golem_exposed",
        ),
        (
            "waxed_weathered_copper_golem_statue",
            "entity/copper_golem/copper_golem_weathered",
        ),
        (
            "waxed_oxidized_copper_golem_statue",
            "entity/copper_golem/copper_golem_oxidized",
        ),
    ];
    let poses = ["standing", "running", "sitting", "star"];
    let kinds = variants
        .iter()
        .flat_map(|(name, _)| {
            poses.iter().map(move |pose| {
                let name = format!("minecraft:{name}");
                classify(&name, &|property| match property {
                    "facing" => Some("east".into()),
                    "copper_golem_pose" => Some((*pose).into()),
                    _ => None,
                })
            })
        })
        .collect();
    let model = |copies| ModelPart {
        pose: unit_pose(),
        polygons: vec![quad(); copies],
        children: Vec::new(),
    };
    let renderer = renderer_with_shapes(
        HashMap::from([
            ("copper_golem", model(1)),
            ("copper_golem_running", model(2)),
            ("copper_golem_sitting", model(3)),
            ("copper_golem_star", model(4)),
        ]),
        kinds,
        HashMap::new(),
    );
    let texture_strings: Vec<_> = variants.iter().map(|(_, texture)| *texture).collect();
    let sprites = sprite_atlas(&texture_strings);
    let world = World::new(BlockStateId(0), 1);
    let anim = |_: [i32; 3]| BlockAnim::default();
    let scene = Scene {
        camera: DVec3::new(2.25, 64.5, -3.75),
        orientation: Quat::IDENTITY,
        anim: &anim,
        texts: None,
        partial: 1.0,
        game_time: 0,
        max_y: 320,
        world: &world,
        sprites: &sprites,
        emissive: &Default::default(),
        translucent_sprites: &HashSet::new(),
        atlas_revision: 1,
        nether: false,
        lid: &|_| 0.0,
        sign: &|_| None,
        banner: &|_| Vec::new(),
        skull: &|_| None,
        glyph: &|_| (None, 0.0),
        glyphs: 0,
    };
    let options = CacheOptions {
        copper_golem_statues: true,
        ..Default::default()
    };
    let pos = [0, 64, 0];
    let mut previous_texture = None;
    let mut previous_uv = None;
    for (variant, (_, texture)) in variants.iter().enumerate() {
        for (pose, _) in poses.iter().enumerate() {
            let state = (variant * poses.len() + pose) as u32;
            let blocks = [(pos, state)];
            let mut cached = Vec::new();
            renderer.render_with_cache_options(&scene, blocks.iter(), options, &mut cached);
            assert_same_vertices(&cached, &render_direct(&renderer, &scene, &blocks));
            assert_eq!(cached.len(), 4 * (pose + 1));
            let sprite = sprites.get(*texture).unwrap();
            assert_eq!(cached[0].uv, [sprite.u0, sprite.v0]);
            if previous_texture != Some(*texture) {
                assert_ne!(previous_uv, Some(cached[0].uv));
            }
            previous_texture = Some(*texture);
            previous_uv = Some(cached[0].uv);
            assert_eq!(renderer.static_geometry.lock().unwrap().len, 1);
        }
    }
}
