//! GPU renderer for streamed terrain sections, sky, block outline and HUD.
//! Section origins are supplied relative to the camera each frame so large
//! world coordinates keep full precision.
use crate::{
    Result,
    environment::Sky,
    graphics::GraphicsApi,
    hud::{HudBatch, HudRenderer},
    mesher::{Part as Quads, SectionMesh, Sides, TerrainVertex, Vertex},
    section_mask::SectionMask,
    sprites::SpriteAnimation,
};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};
use wgpu::util::DeviceExt;
use winit::window::Window;

// RenderPipelines.CRUMBLING: dst * src + src * dst, alpha replaced.
const CRUMBLING_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Dst,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
/// `TerrainVertex` for `vs_terrain`: position, UV, layer, light, colour.
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Uint16x2,2=>Uint16,4=>Uint8x2,3=>Unorm8x4];

/// entity.wgsl after the OverlayTexture constants it reads.
fn entity_shader() -> String {
    crate::overlay::wgsl_constants() + include_str!("entity.wgsl")
}

#[cfg(test)]
mod crumbling_tests;
mod entity_batch;
mod entity_outline;
#[cfg(test)]
pub(crate) mod test_gpu;
mod weather;
use entity_batch::EntityBatch;
pub use entity_batch::{
    DISSOLVE_OVERLAY, DRAGON_RAYS_OVERLAY, ENERGY_SWIRL_OVERLAY, OIT_ENTITY, SHADOW_OVERLAY,
    TEXT_POLYGON_OFFSET_OVERLAY, TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY,
    TRANSLUCENT_PARTICLE_OVERLAY, WATER_MASK_OVERLAY, mark_oit,
};
pub use view::{View, ViewSet};

/// Window.getAppropriateLineWidth, in framebuffer pixels.
pub fn outline_width(width: u32) -> f32 {
    2.5f32.max(width as f32 / 1920.0 * 2.5)
}

/// ScreenSize, stroke width and ProjectionType.PERSPECTIVE layering.
pub fn outline_params(width: u32, height: u32) -> [f32; 4] {
    [
        width as f32,
        height.max(1) as f32,
        outline_width(width),
        1.0 - 1.0 / 4096.0,
    ]
}

/// Camera-relative segment endpoints; no shape expansion in vanilla 26.3.
pub fn outline_vertices(edges: &[[f64; 6]], camera: glam::DVec3) -> Vec<[f32; 6]> {
    edges
        .iter()
        .map(|e| std::array::from_fn(|i| (e[i] - camera.to_array()[i % 3]) as f32))
        .collect()
}

/// Whether the renderer should initialize its GPU occlusion pass.
///
/// Software adapters spend more time running the culling pass than drawing
/// the sections it removes. Keep the pass available for explicit A/B checks,
/// while leaving hardware adapters on the existing default.
fn gpu_culling_requested(info: &wgpu::AdapterInfo, env: Option<&str>) -> bool {
    env != Some("0") && (env.is_some() || info.device_type != wgpu::DeviceType::Cpu)
}

/// Whether Translucent Batching is offered (`graphics::offer_translucent_batching`): where
/// indirect multi-draws (`multi_draw`) take its batches, but not OpenGL's, which loop over
/// their entries on the CPU (and without them every section is a call of its own anyway);
/// nor where vanilla frames draw flat (`flat`'s `min_sections` below `usize::MAX`: Metal,
/// Intel Gen11/Gen12's Vulkan driver, `HORIZON_FLAT_DRAWS=1`), as flat draws read
/// translucent quads in the section arena.
fn translucent_batchable(
    info: &wgpu::AdapterInfo,
    multi_draw: bool,
    flat_min_sections: Option<usize>,
) -> bool {
    multi_draw
        && info.backend != wgpu::Backend::Gl
        && flat_min_sections.is_none_or(|min| min == usize::MAX)
}

/// Whether translucent quads have an arena of their own: with Translucent Batching
/// (`batching`), unless a shader pack's frames can draw terrain flat (as for `flat_frame`),
/// which reads them in the section arena through its tables: where flat draws exist and
/// are on (`flat_draws`), for a `shaders/` pack's terrain programs (`terrain`) or a shadow
/// pass (`shadow`: a `shaders/` pack's, or the `[shadow]` of one of Horizon's own).
fn translucent_split(batching: bool, flat_draws: bool, terrain: bool, shadow: bool) -> bool {
    batching && !(flat_draws && (terrain || shadow))
}

mod animate;
mod arena;
pub mod compositor;
mod cull;
mod end_sky;
mod flat;
mod frame_gen;
mod hdr;
mod hdr_icc;
mod hdr_stats;
pub use hdr::Calibration as HdrCalibration;
pub mod block_light;
pub mod lightning;
mod lod;
mod native_material;
mod native_pack;
mod native_post;
pub mod native_render;
mod native_shadow;
mod native_sky;
mod occlusion;
mod oit;
mod oit_features;
mod pack_atlas;
mod pack_center_depth;
pub mod pack_draw;
mod pack_entities;
mod pack_hand;
mod pack_images;
mod pack_post;
mod pack_shadow;
mod pack_sky;
mod pack_textures;
pub(crate) mod pack_timing;
pub mod pack_uniforms;
mod pack_weather;
mod page_runs;
pub mod panorama;
mod readback;
mod resident;
mod reuse;
mod stars;
pub mod submit;
mod sunrise_fan;
mod timing;
pub mod translucency;
pub(crate) mod upload;
mod view;
mod wgsl_prefix;
mod world_post;

/// A section's instance: its camera-relative origin and, in `w`, the step of
/// its fixed-point UVs (`terrain.wgsl`'s `terrain_uv`).
/// The world pass again after occlusion culling's test: it loads both
/// targets (phase 2 draws over phase 1, reading the test's results).
/// `LevelRenderer.addSkyPass`: fluids and camera effects block the sky pass.
fn sky_open(frame: &Frame) -> bool {
    !frame.sky_blocked
}

/// The Overworld-style sky (disc, sunset, stars, void) draws this frame.
fn sky_visible(frame: &Frame) -> bool {
    !frame.title && frame.skybox && sky_open(frame)
}

/// The world pass's part that draws a shader pack's solid terrain: the world's colour and
/// depth loaded, and its `surface` target, cleared on the first of these passes.
fn surface_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    surface: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    clear: bool,
) -> wgpu::RenderPass<'e> {
    let keep = |load| wgpu::Operations {
        load,
        store: wgpu::StoreOp::Store,
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Horizon world, pack terrain"),
        color_attachments: &[
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: keep(wgpu::LoadOp::Load),
            }),
            Some(wgpu::RenderPassColorAttachment {
                view: surface,
                resolve_target: None,
                depth_slice: None,
                ops: keep(if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                }),
            }),
        ],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}
fn world_pass_after_culling<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    keep_depth: bool,
    timing: Option<&mut pack_timing::PassTiming>,
    name: &str,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Horizon world, after culling"),
        timestamp_writes: timing.and_then(|t| t.writes(name)),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                // A shader pack reads it as depthtex0.
                store: if keep_depth {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}
/// A section's instance: its camera-relative origin and its UV step, negated
/// past its fade code while it fades in over far terrain (`lod::instance`).
fn section_instance(origin: [f32; 3], fade: u32, visibility: f32) -> [f32; 4] {
    let step = 1.0 / crate::mesher::UV_STEPS;
    [
        origin[0],
        origin[1],
        origin[2],
        if fade == 0 {
            // The step is a power of two, so its mantissa is free: it holds the vanilla
            // chunk fade-in's `1 - visibility` in 23 bits (terrain.wgsl's `instance_step` and
            // `instance_visibility` read it back). A whole section is the bare step.
            let faded = ((1.0 - visibility) * 8_388_608.0)
                .round()
                .clamp(0.0, 8_388_607.0) as u32;
            f32::from_bits(step.to_bits() | faded)
        } else {
            -(fade as f32 + step)
        },
    ]
}
/// `SectionRenderDispatcher.RenderSection.getVisibility`: how far through its fade-in a section
/// uploaded `elapsed_ms` ago is.
fn chunk_visibility(elapsed_ms: u64, fade_ms: u64) -> f32 {
    if elapsed_ms >= fade_ms {
        1.0
    } else {
        elapsed_ms as f32 / fade_ms as f32
    }
}
/// `Util.toMillis`: `Mth.floor(seconds * 1000.0)`.
fn fade_millis(seconds: f32) -> u64 {
    (f64::from(seconds) * 1000.0).floor().max(0.0) as u64
}
/// `LevelRenderer.CHUNK_VISIBILITY_THRESHOLD`: a section's block entities wait for it.
const CHUNK_VISIBILITY_THRESHOLD: f32 = 0.3;
/// A new value for `Renderer::block_entity_revision`.
fn next_block_entity_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    sky_color: [f32; 4],
    misc: [f32; 4],
    inverse_view: [[f32; 4]; 4],
    sunrise: [f32; 4],
    sun: [f32; 4],
    /// x: sky fog end, y: cloud fog end, z: dark disc (camera below the horizon),
    /// w: the glint strength (`GlintAlpha`).
    fog_ends: [f32; 4],
    /// Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png.
    end_portal_sprites: [[f32; 4]; 2],
    /// x: vanilla's `GameTime` uniform; y: nether cardinal lighting; z: how many
    /// `dynamic_lights` are set; w: the glint scroll clock (`Glint::scroll_ticks`).
    time: [f32; 4],
    /// Atlas rect of a destroy stage (entity.wgsl takes only its size: every stage's is the same,
    /// and each crack carries its own stage's origin).
    crumbling_sprite: [f32; 4],
    /// Framebuffer size, selection stroke width, perspective layering scale.
    outline_params: [f32; 4],
    /// Horizon Extras' dynamic lights: camera-relative position and level (`dynamic_light`).
    dynamic_lights: [[f32; 4]; crate::dynamic_light::MAX],
    colored_lights: [crate::dynamic_light::Colored; crate::dynamic_light::MAX],
    colored_count: [u32; 4],
    emissive_layers: [[u32; 4]; crate::native_emissive::MAX],
    emissive_count: [u32; 4],
    /// The sunrise fan's triangles as the sky shader tests them (`sunrise_fan::normals`).
    sunrise_fan: [[f32; 4]; 2 * sunrise_fan::TRIANGLES],
    /// x: `Options.textureFiltering` (0 none, 1 RGSS, 2 anisotropic); y: the sprite arrays' width.
    filtering: [f32; 4],
    /// HDR output's tier windows, low to very bright: where the lift starts, where it is
    /// full (`hdr::Hdr::windows`); zero without HDR.
    hdr_windows: [[f32; 4]; 2],
}
pub type SectionKey = (i32, i32, i32);
/// The water mirror's targets: the world as a camera mirrored about the water plane sees
/// it, at half the world's resolution (the material stage's `reflection`).
struct MirrorTarget {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}
struct SectionGpu {
    /// Arena page and first vertex of its solid quads, and after them its
    /// translucent quads unless those have an arena of their own; (0, 0),
    /// unused, when that leaves it no quads in `arena`.
    page: u32,
    start: u64,
    /// The page and first vertex of its translucent quads: in
    /// `translucent_arena` when there is one, else after the solid quads.
    /// Unused without translucent quads.
    translucent_at: (u32, u64),
    solid: u32,
    /// Where the solid quads' buckets by facing side end (`mesher::Sides`).
    sides: Sides,
    translucent: u32,
    sort: translucency::SortState,
    sorted_indices: Option<(u32, u64)>,
    /// Its id for flat draws (`flat::Flat::add`), when they are on.
    flat_id: Option<u32>,
    /// When it began to fade in over the far terrain it replaces, which
    /// stays whole behind it until then: a section new where far tiles are
    /// drawn. Meshed again, it stays as it was.
    fading: Option<std::time::Instant>,
    /// When it began to fade out, its column gone over far terrain that
    /// stays behind it (`Renderer::leave_section`); released once it has.
    leaving: Option<std::time::Instant>,
    /// Animated sprite layers present in this section.
    animated_layers: Box<[u16]>,
}
/// Extracts the animated sprite layers present in a section mesh. Meshing
/// records the distinct layers, so uploading does not rescan every vertex.
pub(crate) fn collect_animated_layers(
    mesh: &SectionMesh,
    is_animated: impl Fn(u16) -> bool,
) -> Box<[u16]> {
    mesh.sprite_layers
        .iter()
        .copied()
        .filter(|&layer| is_animated(layer))
        .collect()
}

impl SectionGpu {
    /// Its fade code this frame (`lod::fade_code`), 0 once whole. `retiring`:
    /// its column has left the render distance, far terrain is coming in
    /// under it (`lod::HELD`).
    fn fade(&self, now: std::time::Instant, retiring: bool) -> u32 {
        match self.leaving {
            Some(since) => lod::fade_out_code(since, 0, now),
            None if retiring => lod::HELD,
            None => self
                .fading
                .and_then(|since| lod::fade_code(Some(since), 0, now))
                .unwrap_or(0),
        }
    }
    /// Its vertices in `arena`, and in `translucent_arena` when `split` (there
    /// is one): (page, first vertex, vertices) of each.
    fn vertex_ranges(&self, split: bool) -> (Option<(u32, u64, u64)>, Option<(u32, u64, u64)>) {
        if !split {
            let vertices = u64::from(self.solid + self.translucent) * 4;
            return (Some((self.page, self.start, vertices)), None);
        }
        let (page, start) = self.translucent_at;
        (
            (self.solid > 0).then(|| (self.page, self.start, u64::from(self.solid) * 4)),
            (self.translucent > 0).then(|| (page, start, u64::from(self.translucent) * 4)),
        )
    }
}
/// Which of a mesh's ranges `move_off_page` moves: what it holds in the section
/// arena (`split`: its solid quads alone, `SectionGpu::vertex_ranges`), its
/// translucent quads in `translucent_arena`, or its sorted indices.
#[derive(Clone, Copy)]
enum Part {
    Section { split: bool },
    Translucent,
    Indices,
}
impl Part {
    /// (page, first vertex or index, count), as the release paths free it.
    fn of(self, s: &SectionGpu) -> Option<(u32, u64, u64)> {
        match self {
            Self::Section { split } => s.vertex_ranges(split).0,
            Self::Translucent => s.vertex_ranges(true).1,
            Self::Indices => s
                .sorted_indices
                .map(|(page, at)| (page, at, u64::from(s.translucent) * 6)),
        }
    }
    /// Points the mesh where its range moved.
    fn set(self, s: &mut SectionGpu, to: (u32, u64)) {
        match self {
            Self::Section { split } => {
                (s.page, s.start) = to;
                if !split {
                    // Its translucent quads came along, after the solid ones.
                    s.translucent_at = (to.0, to.1 + u64::from(s.solid) * 4);
                }
            }
            Self::Translucent => s.translucent_at = to,
            Self::Indices => s.sorted_indices = Some(to),
        }
    }
}
/// `Renderer::compact_arena`'s and `shrink_translucent`'s moves: what the meshes
/// hold on `page` of `arena` (`part`) goes to its other pages in video memory,
/// until the next range would pass `budget` bytes (it keeps what is left; the
/// first range moves whatever its size, so none keeps the page closed), the
/// copies into `encoder` and, in the section arena, the ranges' flat draw ids
/// (`flat`) along. False when the other pages have no room left.
fn move_off_page<'a>(
    arena: &mut arena::Arena,
    encoder: &mut wgpu::CommandEncoder,
    mut flat: Option<(&mut flat::Flat, &wgpu::Device, &mut upload::Uploads)>,
    meshes: impl Iterator<Item = &'a mut SectionGpu>,
    page: u32,
    part: Part,
    budget: &mut u64,
) -> bool {
    let mut moved = false;
    for s in meshes {
        // Nothing of a mesh here: split, one without solid quads (its (0, 0) is no
        // range of page 0); in the other arenas, one without translucent quads.
        let Some((_, start, vertices)) = part.of(s).filter(|r| r.0 == page) else {
            continue;
        };
        let bytes = vertices * arena.stride();
        if moved && bytes > *budget {
            break;
        }
        let Some(to) = arena.alloc_moved(vertices) else {
            return false;
        };
        arena.copy(encoder, (page, start), to, vertices);
        arena.release(page, start, vertices);
        if let (Some((flat, device, uploads)), Some(id)) = (&mut flat, s.flat_id) {
            flat.remove(id);
            s.flat_id = Some(flat.add(device, encoder, uploads, to.0, to.1, vertices));
        }
        part.set(s, to);
        *budget = budget.saturating_sub(bytes);
        moved = true;
    }
    true
}
/// Starts moving a page of a large arena far larger than its ranges in use
/// (`Arena::oversized`) into a new page of the size they get (`Arena::open_page`),
/// for `Renderer::shrink_translucent`: the page closes and `move_off_page` empties
/// it. None: no page to shrink. False, and nothing changes: no video memory for
/// the new page, or a range there without its mesh in `meshes`, which would keep
/// the page from ever emptying.
fn begin_shrink<'a>(
    arena: &mut arena::Arena,
    device: &wgpu::Device,
    meshes: impl Iterator<Item = &'a mut SectionGpu>,
    part: Part,
) -> Option<bool> {
    let (page, capacity) = arena.oversized()?;
    let held: u64 = meshes
        .filter_map(|s| part.of(s).filter(|r| r.0 == page))
        .map(|(.., n)| arena::Arena::span(n))
        .sum();
    debug_assert_eq!(held, arena.used(page), "a range in use without its mesh");
    if held != arena.used(page) || !arena.open_page(device, capacity) {
        return Some(false);
    }
    arena.close(page);
    Some(true)
}
/// Line segments the hit outline can hold.
const MAX_OUTLINE_EDGES: u64 = 256;
/// Staging buffer size for section meshes: a frame of uploads while
/// exploring fits in a few.
const SECTION_UPLOAD_CHUNK: u64 = 4 << 20;
/// Section copies are submitted early past this many staged bytes, which
/// bounds the staging memory frames that are not drawn can hold.
const SECTION_COPY_LIMIT: u64 = 64 << 20;
/// What compaction copies in a frame, `compact_arena` first and
/// `shrink_translucent` with what it left, and a range more where a call's
/// first is larger (`move_off_page`): a page empties over a few frames, with
/// no hitch.
const COMPACT_BUDGET: u64 = 32 << 20;
/// A large translucent page starts to shrink (`Renderer::shrink_translucent`)
/// this long after terrain last arrived, and `SHRINK_RETRY` after a try found
/// no room.
const SHRINK_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);
const SHRINK_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
/// Not before then may a large translucent page start to shrink.
#[derive(Default)]
struct ShrinkGate(Option<Instant>);
impl ShrinkGate {
    /// Not before `wait` from `now`, nor before an earlier wait is over.
    fn wait(&mut self, now: Instant, wait: std::time::Duration) {
        self.0 = self.0.max(Some(now + wait));
    }
    /// Whether one may start now: the waits are over, and `compact_arena` copied
    /// nothing this frame (it `left` the whole budget).
    fn may_start(&self, now: Instant, left: u64) -> bool {
        left == COMPACT_BUDGET && self.0.is_none_or(|at| now >= at)
    }
}
pub struct Frame<'a> {
    pub camera: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_degrees: f32,
    /// The camera entity's eye and view rotation (yaw, pitch): the camera's own except in a
    /// third-person view.
    pub eye: DVec3,
    pub eye_rotation: [f32; 2],
    /// The camera is the entity's eye: no third-person view.
    pub first_person: bool,
    /// The `GameRenderer.bobHurt`/`bobView` pose multiplied into the
    /// projection (`crate::camera::bob_matrix`).
    pub bob: Mat4,
    /// The spinning effect multiplied into the level's projection only
    /// (`crate::camera::spinning_effect_matrix`), not the hand's.
    pub spin: Mat4,
    pub sky: Sky,
    /// Block edits and chunk membership that can change the shader pack's shadow map.
    pub shadow_scene_revision: (u64, u64),
    /// Opt-in reuse of the shader pack's last shadow map when the key still matches
    /// (`horizon-graphics.txt` `shadowCache`; off by default).
    pub shadow_cache: bool,
    /// Render distance in blocks (fog end).
    pub distance: f32,
    /// Full-detail terrain distance in blocks; distant LODs bypass the cave graph.
    pub detail_distance: f32,
    /// `Camera.depthFar`: the far plane (`crate::camera::depth_far`).
    pub depth_far: f32,
    pub underwater: bool,
    /// The y of the horizontal water plane around the camera, in world blocks, while one
    /// of Horizon's own packs reads the water mirror (`Game::water_plane`,
    /// `Renderer::wants_water_plane`).
    pub water_plane: Option<f32>,
    /// Skip sections the camera cannot see through open faces (vanilla's
    /// occlusion graph; off in spectator-like views inside blocks).
    pub cave_culling: bool,
    /// Title screen: only the dedicated panorama cube map and the HUD.
    pub title: bool,
    /// Lava, powder snow, blindness or darkness blocks the sky pass.
    pub sky_blocked: bool,
    /// Dimensions without a skybox (the Nether) clear to the fog colour.
    pub skybox: bool,
    /// The End: clear to black and draw the resident sky box behind the world.
    pub end_sky: bool,
    /// End sky atlas bounds (u0, v0, u1, v1); absent when the sprite is missing.
    pub end_sky_sprite: Option<[f32; 4]>,
    /// Environmental fog start/end (Nether 10..96), if any.
    pub environment_fog: Option<(f32, f32)>,
    pub water_fog: [f32; 3],
    /// `FogRenderer.setupFog` result; replaces the underwater/environment fields when set.
    pub fog: Option<crate::fog::FogData>,
    /// Camera eye below the horizon height and not under water (`shouldRenderDarkDisc`).
    pub dark_disc: bool,
    /// The hit block outline as line segments (from, to) in world space.
    pub outline: Vec<[f64; 6]>,
    /// `BlockOutlineRenderState.isTranslucent`: draw after translucent terrain.
    pub outline_after_terrain: bool,
    /// `BlockOutlineRenderState.highContrast`: a thick black outline under a bright one.
    pub outline_high_contrast: bool,
    /// `lines` with a colour and pixel width (`gizmos::camera_relative`):
    /// the fishing line and debug gizmos, depth tested, then gizmos drawn
    /// over everything.
    pub debug_lines: &'a [[f32; 8]],
    pub debug_lines_on_top: &'a [[f32; 8]],
    /// `DebugCrosshairRenderer`'s axes, camera relative like `debug_lines`:
    /// drawn after the hand at its projection, without the bob or fog.
    pub debug_crosshair: &'a [[f32; 8]],
    pub hud: &'a HudBatch,
    /// `GlintAlpha` (accessibility Glint Strength); the entity shader uses it
    /// for held/world items, independent of what the HUD batch last drew.
    pub glint_alpha: f32,
    /// The glint scroll clock (`Glint::scroll_ticks`): entity.wgsl's
    /// `ARMOR_ENTITY_GLINT_TEXTURING` phase.
    pub glint_ticks: f32,
    /// CardinalLighting.Type.NETHER, otherwise DEFAULT.
    pub nether_lighting: bool,
    /// Camera-relative entity geometry textured from the shared atlas.
    pub entities: &'a [Vertex],
    /// Which entity `entities`' vertices belong to: (first vertex, entity id), each run up
    /// to the next; id 0 for none. Frame generation follows these entities' own motion.
    pub entity_motion: &'a [(u32, u64)],
    /// The glowing entities' silhouettes in their team colours (`OutlineBufferSource`),
    /// outlined through walls after the hand.
    pub glowing: &'a [Vertex],
    /// Runs of `entities` by entity, while a shader pack draws them (`pack_draw::EntitySpan`).
    pub entity_spans: &'a [pack_draw::EntitySpan],
    pub weather: &'a [crate::weather::Vertex],
    pub weather_time: crate::weather::Time,
    /// Horizon Extras' dynamic lights, nearest first (`dynamic_light::select`); empty when off.
    pub dynamic_lights: &'a [[f32; 4]],
    pub colored_lights: &'a [crate::dynamic_light::Colored],
    pub emissive_layers: &'a [[u32; 4]],
    /// Cloud mesh inputs; geometry and GPU storage are retained by the renderer.
    pub clouds: Option<crate::clouds::Input<'a>>,
    /// Retained lightning bolts, drawn before the other additive entities.
    pub lightning: lightning::Input<'a>,
    /// Additive translucent geometry after lightning (dragon rays).
    pub additive: &'a [Vertex],
    /// Name tags over everything, without depth (`TEXT_SEE_THROUGH`).
    pub see_through: &'a [Vertex],
    /// Opaque entity quads only a native pack's shadow pass draws (the local player in first
    /// person).
    pub shadow_only: &'a [Vertex],
    /// Camera-relative sun and moon, or the animated End flash (additive,
    /// behind everything and after the resident End sky box).
    pub celestial: &'a [Vertex],
    /// The stars, drawn from their static buffer after the sun and moon:
    /// the sky's rotation (camera-relative world from star space) and the
    /// star brightness.
    pub stars: Option<(Mat4, f32)>,
    /// First-person hand and held item, drawn after clearing depth.
    pub hand: &'a [Vertex],
    /// `CameraRenderState.hudFov`: the hand's own field of view
    /// (`GameRenderer.render3dHud`).
    pub hand_fov_degrees: f32,
    /// Full-screen camera effects drawn over the world and hand, under the HUD.
    pub screen: &'a [crate::screen_effects::ScreenQuad],
    /// What a shader pack's uniforms need from the game.
    pub pack: pack_uniforms::PackInputs,
    /// The world border wall, when near enough to be drawn.
    pub border: Option<BorderDraw>,
    pub game_time: f32,
    /// `GlobalSettingsUniform` GameTime: (game time % 24000 + partial tick) / 24000.
    pub shader_game_time: f32,
    /// Atlas rects (u0, v0, width, height) of end_sky.png and end_portal.png
    /// (`rendertype_end_portal`'s Sampler0 and Sampler1).
    pub end_portal_sprites: [[f32; 4]; 2],
    /// A destroy-stage sprite: projected UVs repeat inside a rect of this size, from the
    /// origin each crack vertex carries (`breaking::stage_light`).
    pub crumbling_sprite: [f32; 4],
    pub ticks: u64,
    /// The darkness effect on the lightmap: the brightness it takes off the
    /// gamma option and the pulsing scale subtracted from the light.
    pub darkness: (f32, f32),
    /// Night vision (or conduit water vision) intensity on the lightmap.
    pub night_vision: f32,
    /// Torch flicker added to the block light factor of 1.4.
    pub block_flicker: f32,
    /// `lightmap.fsh` BossOverlayWorldDarkeningFactor (a wither's bar).
    pub boss_darken: f32,
    /// `Screen.extractBlurredBackground`: blur the frame by this radius
    /// (`menuBackgroundBlurriness`) before the screen is drawn over it.
    pub menu_blur: Option<f32>,
    /// `Options.chunkSectionFadeInTime`, in seconds: how long a section takes to fade in once
    /// uploaded (0 draws it whole at once).
    pub chunk_fade: f32,
    /// `GameRenderer.requestedPostEffects`: the chains that run over the world.
    pub post_effects: crate::post_chain::Request<'a>,
}
/// One frame of the world border wall (`WorldBorderRenderer.render`).
#[derive(Clone, Debug)]
pub struct BorderDraw {
    /// The cached wall buffer; uploaded only when `rebuilt`.
    pub vertices: [crate::border_render::BorderVertex; 16],
    pub rebuilt: bool,
    pub transforms: crate::border_render::BorderTransforms,
    /// Walls to draw (indices of four-vertex quads), nearest first.
    pub sides: Vec<usize>,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BorderUniform {
    color: [f32; 4],
    offset: [f32; 4],
}
/// A frame to save as a PNG: F2's screenshot, an agent's `game_screenshot`,
/// or the icon the Select World list shows.
pub struct ScreenshotRequest {
    pub path: PathBuf,
    /// `GameRenderer.takeAutoScreenshot`: centre-crop the frame to a square
    /// and average it down to this size. `None` saves the frame as drawn.
    pub size: Option<[u32; 2]>,
    /// F2's: drawn at the game's resolution (Resolution Scale above 100%) times
    /// `Renderer::screenshot_quality`, not the window's.
    pub scaled: bool,
}
pub struct Renderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// The window's swapchain; `None` when rendering offscreen (`--offscreen`). Shared
    /// with frame generation's present thread while it runs.
    surface: Option<Arc<wgpu::Surface<'static>>>,
    /// The surface `release_surface` set aside for the renderer that replaces
    /// this one on the same device: reused, not created again, as a new
    /// surface for the window can come back lost (Wayland on Mesa).
    spare_surface: std::sync::Mutex<Option<wgpu::Surface<'static>>>,
    /// The offscreen colour target, sized like `config`.
    offscreen: Option<wgpu::Texture>,
    /// Offscreen submissions still on the GPU: at most two frames run ahead.
    in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Splitscreen: the caller waits for the GPU once every player has submitted
    /// (`wait_in_flight`), so one player's work overlaps the GPU drawing another's.
    pub paced_elsewhere: bool,
    /// Which steps of the first frame on this device still go to the log
    /// (see `startup_step`).
    first_frame: FirstFrame,
    gpu_timer: Option<timing::GpuTimer>,
    /// GPU frame times (ms) read back since the caller last drained them.
    pub gpu_times: Vec<f32>,
    /// The last lightmap written, RGBA rows of sky level by block level
    /// (F3+4 shows it).
    pub lightmap_pixels: [u8; 16 * 16 * 4],
    /// Time the last `render` spent blocked on the GPU or the display
    /// (acquiring, presenting, offscreen pacing, the submit thread holding the queue),
    /// excluded from CPU time.
    pub last_wait: std::time::Duration,
    /// CPU time of the last `render`, waits excluded.
    pub last_encode: std::time::Duration,
    /// What the system says about the window's display, watched only while HDR output
    /// is on; before `window`, so it's dropped first.
    display: DisplayWatch,
    /// The watch was just made: the settings' Auto values take what it reported.
    display_fresh: bool,
    window: Arc<Window>,
    pub device: wgpu::Device,
    /// The shader modules, layouts and pipelines kept for the next renderer on
    /// this device ([`reuse`]).
    reuse: Arc<reuse::Reuse>,
    /// Frames are finished and submitted on its submit thread ([`submit`]).
    pub queue: submit::Queue,
    /// The last frame went to the submit thread; whether it had no window surface.
    /// The next frame takes it in ([`Self::land_frame`]).
    deferred: Option<bool>,
    /// Set by the device's lost callback. wgpu reports nothing else when an
    /// operation fails on a lost device (no error scope sees it).
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    pub config: wgpu::SurfaceConfiguration,
    solid: wgpu::RenderPipeline,
    /// `solid` split in two passes for the world pass (terrain.wgsl's `opaque_quad`).
    solid_opaque: wgpu::RenderPipeline,
    solid_cutout: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    /// Flat section draws (`flat`), where multi-draw is emulated.
    flat: Option<flat::Flat>,
    /// `flat` draws this frame's sections (off: the per-section draws).
    flat_on: bool,
    /// Occlusion culling of solid section draws, where the GPU's instance
    /// counts reach a multi-draw's entries (Vulkan, Metal).
    cull: Option<cull::Cull>,
    cull_on: bool,
    /// Solid quads facing away from the eye are left out (`mesher::Sides`).
    sides_on: bool,
    sky: wgpu::RenderPipeline,
    dark_disc: wgpu::RenderPipeline,
    /// Clears the world pass's depth to the far plane so the first-person hand draws in the
    /// same pass (`hand_in_world`), as the hand's own pass would clear it.
    depth_reset: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    lines_secondary: wgpu::RenderPipeline,
    /// Gizmo lines (`RenderTypes.linesTranslucentNoDepthWrite`) and the
    /// always-on-top ones.
    debug_lines: wgpu::RenderPipeline,
    debug_lines_on_top: wgpu::RenderPipeline,
    debug_line_buffer: wgpu::Buffer,
    debug_line_capacity: u64,
    /// The terrain resources with the debug crosshair's projection
    /// (`crosshair_globals`).
    crosshair_group: wgpu::BindGroup,
    crosshair_globals: wgpu::Buffer,
    /// The water mirror: its solid terrain pipeline, its camera's globals and their group,
    /// and its targets once drawn.
    solid_mirror: wgpu::RenderPipeline,
    mirror_globals: wgpu::Buffer,
    mirror_group: wgpu::BindGroup,
    mirror: Option<MirrorTarget>,
    entity_pipeline: wgpu::RenderPipeline,
    hdr_mark_pipeline: wgpu::RenderPipeline,
    hdr_hand_unmark_pipeline: wgpu::RenderPipeline,
    hdr_mark_lightning_pipeline: wgpu::RenderPipeline,
    weather: weather::Weather,
    crumbling_pipeline: wgpu::RenderPipeline,
    shadow_blob_pipeline: wgpu::RenderPipeline,
    translucent_item_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.WATER_MASK`: depth writes only, no colour.
    water_mask_pipeline: wgpu::RenderPipeline,
    cloud_pipeline: wgpu::RenderPipeline,
    cloud_mesh: crate::clouds::Mesh,
    cloud_buffer: wgpu::Buffer,
    cloud_capacity: u64,
    cloud_uniform: wgpu::Buffer,
    cloud_group: wgpu::BindGroup,
    additive_pipeline: wgpu::RenderPipeline,
    dragon_rays_pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.ENERGY_SWIRL`: additive, unlit, the sprite repeated within itself.
    energy_swirl_pipeline: wgpu::RenderPipeline,
    lightning_pipeline: wgpu::RenderPipeline,
    lightning: lightning::Renderer,
    see_through_pipeline: wgpu::RenderPipeline,
    text_polygon_offset_pipeline: wgpu::RenderPipeline,
    celestial_pipeline: wgpu::RenderPipeline,
    celestial_buffer: wgpu::Buffer,
    end_sky: end_sky::EndSky,
    stars: stars::Stars,
    /// The glowing entities' outline (`LevelRenderer.entityOutlineTarget`).
    entity_outline: entity_outline::EntityOutline,
    panorama: panorama::Panorama,
    /// `RenderPipelines.WORLD_BORDER` and its buffers.
    border_pipeline: wgpu::RenderPipeline,
    border_group: wgpu::BindGroup,
    border_uniform: wgpu::Buffer,
    border_vertices: wgpu::Buffer,
    /// Screen effects: alpha-blended, then vignette (`dst * (1 - src)`)
    /// pipelines, each for every sampling (index `blend * 4 + sampling`).
    screen_pipelines: [wgpu::RenderPipeline; 16],
    screen_buffer: wgpu::Buffer,
    screen_capacity: u64,
    entity_group: wgpu::BindGroup,
    /// The layout of `entity_group`, for the native shadow pass.
    entity_bindings: wgpu::BindGroupLayout,
    /// The entity resources with the hand's projection (`hand_globals`).
    hand_group: wgpu::BindGroup,
    hand_globals: wgpu::Buffer,
    entity_buffer: wgpu::Buffer,
    entity_capacity: u64,
    entity_batch: EntityBatch,
    group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    /// The resource packs' LabPBR maps (normals, specular), for native shader packs.
    pbr_maps: Option<[wgpu::Texture; 2]>,
    lightmap: wgpu::Texture,
    /// The block sprites, `sprite_split` layers per array (the first is
    /// `sprites` in terrain.wgsl, the others `sprites_1`...).
    sprite_textures: Vec<wgpu::Texture>,
    /// Block sprite layers per array on this device ([`layer_split`]).
    sprite_split: u32,
    /// A 1-layer array bound where a pack needs fewer arrays.
    sprite_unused: wgpu::TextureView,
    mip_levels: u32,
    /// The mipmap levels sampled (the Mipmap Levels option), at most `mip_levels - 1`.
    mipmaps: u32,
    terrain_layout: wgpu::BindGroupLayout,
    /// terrain.wgsl's `SPRITE_ARRAYS` and `SPRITE_SPLIT`, for a shader pack's terrain.
    terrain_sprites: [f64; 2],
    sprite_sampler: wgpu::Sampler,
    /// `Options.textureFiltering`: 0 none, 1 RGSS, 2 anisotropic.
    texture_filtering: u32,
    /// The anisotropic sampler's clamp (`Options.maxAnisotropyValue`); 1 unless anisotropic.
    max_anisotropy: u16,
    lightmap_view: wgpu::TextureView,
    lightmap_sampler: wgpu::Sampler,
    animations: animate::SpriteAnimator,
    active_animated_layers: Vec<u64>,
    uploads: upload::Uploads,
    /// Section meshes are staged through their own belt into
    /// `section_copies`, which the next frame records its commands after:
    /// `Queue::write_buffer` would make a staging buffer per write.
    section_uploads: upload::Uploads,
    section_copies: Option<wgpu::CommandEncoder>,
    /// Bytes staged in `section_copies`.
    section_copy_bytes: u64,
    /// When a large translucent page may start to shrink (`shrink_translucent`).
    shrink_gate: ShrinkGate,
    /// Whether `lightmap` holds `lightmap_pixels` yet.
    lightmap_uploaded: bool,
    sprite_minimum_size: u32,
    indices: wgpu::Buffer,
    index_quads: u32,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    outline: wgpu::Buffer,
    depth: wgpu::TextureView,
    sections: resident::Resident<SectionGpu>,
    lods: HashMap<crate::lod::TileKey, lod::Tile>,
    /// The chunk columns the resident tiles leave out (`Lod::take_cutout`):
    /// their sections are drawn even past the render distance, so terrain
    /// leaving it never opens a hole before the tiles fill it in.
    lod_cutout: crate::fast_hash::FxHashSet<(i32, i32)>,
    /// Removed sections of `lod_cutout` columns, released once tiles cover them.
    lod_retiring: crate::fast_hash::FxHashSet<SectionKey>,
    /// Removed sections fading out over far terrain (`leave_section`).
    lod_leaving: Vec<(SectionKey, std::time::Instant)>,
    /// Finished tile meshes `upload_lods` has yet to upload.
    lod_pending: Option<lod::Pending>,
    detail_columns: crate::fast_hash::FxHashMap<(i32, i32), SectionMask>,
    /// Bumped whenever a column becomes or stops being complete.
    detail_revision: u64,
    arena: arena::Arena,
    /// Translucent quads' own arena, with large pages, while the layout is split
    /// (`translucent_split`): the back-to-front list then stays in one page (and
    /// one sorted-index page) and draws in a few multi-draws rather than one per
    /// few sections.
    translucent_arena: Option<arena::Arena>,
    /// Translucent Batching (`Graphics::translucent_batching`, read when a renderer
    /// is made), where it is offered (`graphics::offer_translucent_batching`).
    translucent_batching: bool,
    /// Every section and far tile was dropped to change the layout since the game
    /// last asked (`take_dropped_sections`).
    sections_dropped: bool,
    sorted_indices: arena::Arena,
    /// Buffers of released arena pages, freed a few frames later on a thread of their own.
    graveyard: arena::Graveyard<wgpu::Buffer>,
    translucent_sorter: translucency::Sorter,
    translucency_cursor: usize,
    /// `DrawIndexedIndirectArgs` for this frame's solid then translucent draws.
    indirect: wgpu::Buffer,
    indirect_capacity: u64,
    /// Indirect draws may set `first_instance` (the section origin).
    pub multi_draw: bool,
    /// Per-frame scratch, kept to avoid allocating every frame.
    visible: Vec<resident::Visible>,
    visible_lods: Vec<lod::Visible>,
    /// The sections a shader pack's shadow camera sees (`pack_post::ShadowView`).
    shadow_visible: Vec<resident::Visible>,
    translucent_draws: Vec<(u64, usize, [u32; 5], (u32, Option<u32>))>,
    origins: Vec<[f32; 4]>,
    /// `DrawIndexedIndirectArgs` words: index count, instance count, first
    /// index, base vertex, first instance.
    draws: Vec<[u32; 5]>,
    /// Vertex arena page and optional sorted index arena page of each draw:
    /// with sorted indices (a translucent draw), a page of `translucent_arena`
    /// when there is one.
    draw_pages: Vec<(u32, Option<u32>)>,
    /// Solid draws left to occlusion culling's phase 2, while the list is
    /// built, and their pages.
    phase2_draws: Vec<[u32; 5]>,
    phase2_pages: Vec<(u32, Option<u32>)>,
    /// Each section's solid draws, to group them by arena page.
    solid_runs: page_runs::PageRuns,
    phase2_runs: page_runs::PageRuns,
    /// The shadow camera's draws by page, in bands of distance from the light.
    shadow_runs: page_runs::PageRuns,
    /// Block-entity blocks per meshed section (world position, state id).
    block_entities: crate::fast_hash::FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    /// When each section was first uploaded (`RenderSection.uploadedTime`): kept across meshing
    /// it again, gone with its column. Sections that fade in over far terrain are not in it.
    section_uploaded: crate::fast_hash::FxHashMap<SectionKey, std::time::Instant>,
    /// When `section_uploaded` last gained a section: once the fade time has passed since,
    /// every section is whole and frames skip the per-section lookups.
    newest_upload: Option<std::time::Instant>,
    /// When a section last began to fade in over far terrain (`SectionGpu::fading`), for the
    /// same shortcut.
    newest_lod_fade: Option<std::time::Instant>,
    /// Changes whenever `block_entities` does (unique across renderers).
    block_entity_revision: u64,
    /// The light sources of each section and their revision, for `block_light`.
    light_blocks: crate::fast_hash::FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    light_revision: u64,
    /// VisGraph face connectivity per meshed section (cave culling).
    occlusion: occlusion::Occlusion,
    /// The column `occlusion`, `sections` and `detail_columns` count in: the
    /// level's (`clear_sections`).
    layout: crate::chunk::ColumnLayout,
    /// Resolution scale, FXAA and TAA (Horizon settings).
    world_post: world_post::Post,
    /// Frame generation (Horizon setting).
    frame_gen: frame_gen::FrameGen,
    /// The entities' texture atlas (`entity_group`'s), for frame generation's motion.
    entity_atlas: wgpu::TextureView,
    /// HDR output (Horizon setting): `Some` while the swapchain is HDR.
    hdr: Option<hdr::Hdr>,
    /// Which pixels of each block sprite may shine in HDR output (terrain.wgsl's
    /// `emission_thresholds`), measured when HDR output turns on.
    emission_stats: hdr_stats::EmissionStats,
    /// The HDR output the window's surface can present.
    hdr_output: Option<hdr::Output>,
    /// The marker file of an HDR output that has not presented a frame yet
    /// (`graphics::HDR_MARKER`).
    hdr_marker: Option<std::path::PathBuf>,
    /// The shader pack's post-processing, and the dimension (legacy id) it was loaded for.
    pack_post: Option<(i32, Option<pack_post::PackPost>)>,
    /// The post passes of one of Horizon's own shader packs; never beside `pack_post`'s.
    native: Option<native_pack::NativePack>,
    /// Time the shader pack's passes on the GPU (F3, or `HORIZON_SHADER_PACK_TIMING`).
    pass_timing: bool,
    /// Times Horizon's own passes on the GPU, as `pass_timing` does (`own_pass_times`).
    own_timing: Option<pack_timing::PassTiming>,
    /// The entity vertices Horizon draws itself while a shader pack draws the others.
    pack_entity_rest: Vec<Vertex>,
    /// Set 1 of the cloud pipeline, for a shader pack's `gbuffers_clouds`.
    cloud_bindings: wgpu::BindGroupLayout,
    /// Rain and snow handed to a shader pack's `gbuffers_weather`, once a pack draws them.
    pack_weather: Option<pack_weather::PackWeather>,
    /// The block sprites as one atlas, while a shader pack is loaded.
    pack_atlas: Option<pack_atlas::PackAtlas>,
    /// The pack's terrain program reads flat draws' vertices (`pack_terrain_inputs`).
    pack_terrain_flat: bool,
    pub hud: HudRenderer,
    /// Vanilla's menu background blur, made on first use.
    menu_blur: Option<crate::menu_blur::MenuBlur>,
    /// `post_effect` chains, made on first use.
    post_effects: Option<crate::post_chain::PostEffects>,
    /// Where a screenshot's frame is drawn when the swapchain cannot be read back.
    readback: readback::Readback,
    pub info: wgpu::AdapterInfo,
    pub suspended: bool,
    /// Consecutive frames whose surface acquisition failed validation. The
    /// first gets the swapchain a fresh configuration; later ones reattach
    /// the window, as a lost surface does.
    acquire_validation_failures: u32,
    /// A surface from `recreate_surface` whose configuration failed. wgpu
    /// panics when such a surface is acquired from (it never had one that
    /// worked), so frames configure it again first and skip until it takes.
    surface_unconfigured: bool,
    pub drawn_sections: usize,
    /// Last frame's section draws (face ranges and translucent sections),
    /// and the draw calls the backend issued for them.
    pub section_draws: usize,
    pub section_draw_calls: usize,
    pub uploaded_bytes: u64,
    started: Instant,
    /// The next frame to write to a PNG path (control socket, F2, world icon).
    pub screenshot: Option<ScreenshotRequest>,
    /// Outcome of the last screenshot: frame size, or the error.
    pub screenshot_result: Option<std::result::Result<[u32; 2], String>>,
    /// Screenshot Quality (Horizon setting): a `scaled` screenshot's frame is this many
    /// times the game's resolution (1, 2, 4 or 8).
    pub screenshot_quality: u32,
    /// How many times the pending scaled screenshot ran out of video memory: each
    /// halves its size, down to the window's.
    screenshot_halvings: u32,
    /// Brightness option (lightmap gamma), 0..1.
    pub gamma: f32,
    /// Optional full-light lightmap (Horizon Extras).
    pub fullbright: bool,
    /// The Improved transparency option (`GameRenderer.useImprovedTransparency`).
    pub improved_transparency: bool,
    /// Its passes and targets, made when first drawn with.
    oit: Option<oit::Oit>,
    /// Advance animated block textures (Horizon setting; vanilla always does).
    pub animated_textures: bool,
    present_modes: Vec<wgpu::PresentMode>,
}
/// Every layer's mip levels, layer major (`first` is the first layer's
/// index into `settings`), made by `MipmapGenerator`.
/// Block sprite layers per texture array at most: the
/// `max_texture_array_layers` of most GPUs. Mods add sprites past it into more
/// arrays (terrain.wgsl's `sprite`); a GPU with fewer layers per array gets a
/// smaller split ([`layer_split`]).
pub(crate) const SPRITE_SPLIT: u32 = 2048;
/// A smaller split from `HORIZON_SPRITE_SPLIT` (dev only: vanilla's sprites
/// drawn from several arrays must look the same).
fn requested_split() -> Option<u32> {
    std::env::var("HORIZON_SPRITE_SPLIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| (1..=SPRITE_SPLIT).contains(&n))
}
/// The split the CPU mip chains are cut by before the adapter is known.
fn sprite_split() -> u32 {
    requested_split().unwrap_or(SPRITE_SPLIT)
}
/// The block sprite layers per texture array on an adapter that holds
/// `max_layers` layers per array: [`SPRITE_SPLIT`], `requested`
/// (`HORIZON_SPRITE_SPLIT`) or `max_layers`, whichever is smallest, with a full
/// array still fitting once [`array_layers`] pads it for OpenGL. Errors when
/// `layers` sprites need more than the [`SPRITE_ARRAYS`] arrays terrain.wgsl binds.
fn layer_split(
    max_layers: u32,
    gl: bool,
    requested: Option<u32>,
    layers: u32,
) -> std::result::Result<u32, String> {
    let mut split = requested.unwrap_or(SPRITE_SPLIT).min(max_layers).max(1);
    if split > 1 && array_layers(split, gl) > max_layers {
        split -= 1;
    }
    if layers > split * SPRITE_ARRAYS {
        return Err(format!(
            "GPU supports {max_layers} texture layers per array; {layers} block textures need \
             more than the {SPRITE_ARRAYS} arrays of {split} Horizon draws from"
        ));
    }
    Ok(split)
}
/// Sprite arrays terrain.wgsl binds (`sprites`, `sprites_1`...).
const SPRITE_ARRAYS: u32 = 8;
/// The layers to give a square texture of `count` layers viewed as a 2D
/// array. wgpu's OpenGL backend picks a texture's target from its layer
/// count when it is created: one layer makes a 2D texture and a multiple of
/// six a cube map, neither of which can be viewed as an array.
pub(crate) fn array_layers(count: u32, gl: bool) -> u32 {
    match count {
        _ if !gl => count,
        0 | 1 => 2,
        n if n % 6 == 0 => n + 1,
        n => n,
    }
}
/// The views of terrain.wgsl's sprite arrays, the unused ones a 1-layer
/// stand-in.
fn sprite_views(
    textures: &[wgpu::Texture],
    unused: &wgpu::TextureView,
    mips: Option<u32>,
) -> Vec<wgpu::TextureView> {
    (0..SPRITE_ARRAYS as usize)
        .map(|i| match textures.get(i) {
            Some(t) => t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                mip_level_count: mips,
                ..Default::default()
            }),
            None => unused.clone(),
        })
        .collect()
}
/// The terrain sprites' sampler: LevelRenderer.chunkLayerSampler's linear one (clamped to the
/// edge) in every Texture Filtering mode, with `anisotropy` samples when it is above 1 (wgpu
/// then needs a linear mipmap filter too).
fn sprite_sampler_descriptor(anisotropy: u16) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("Block sprites (filtered)"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}
/// `Options.maxAnisotropyValue` for a `maxAnisotropyBit`: `1 << bit`, at most wgpu's 16.
fn sampler_anisotropy(bit: i32) -> u16 {
    1u16 << bit.clamp(0, 4)
}
/// terrain.wgsl's group 0.
#[allow(clippy::too_many_arguments)]
fn terrain_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    sprites: &[wgpu::TextureView],
    sprite_sampler: &wgpu::Sampler,
    lightmap: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    emission_thresholds: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            view(1, &sprites[0]),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sprite_sampler),
            },
            view(3, lightmap),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
            view(5, &sprites[1]),
            view(6, &sprites[2]),
            view(7, &sprites[3]),
            view(8, &sprites[4]),
            view(9, &sprites[5]),
            view(10, &sprites[6]),
            view(11, &sprites[7]),
            view(12, emission_thresholds),
        ],
    })
}
/// CPU texture uploads prepared by the resource worker. Consumed by renderer
/// construction so the temporary mip chains are released after GPU upload.
pub struct PreparedTextures {
    sprites: Vec<(Vec<u8>, u32)>,
    /// The sprite layers per array `sprites` is cut by.
    split: u32,
    forcefield: (Vec<u8>, u32),
    /// The LabPBR maps' mip chains (normals, specular), their levels, layers and size.
    pbr: Option<(Vec<u8>, Vec<u8>, u32, u32, u32)>,
}
/// Every level of `layers` square RGBA layers of `size`, each the mean of four texels of the
/// one above, layer major: the LabPBR maps' mips (their channels are data, not colour).
fn box_mip_chain(pixels: &[u8], size: u32, layers: u32) -> Vec<u8> {
    let layer_bytes = (size * size * 4) as usize;
    let mut out = Vec::new();
    for layer in pixels.chunks_exact(layer_bytes).take(layers as usize) {
        let mut level = layer.to_vec();
        let mut side = size as usize;
        out.extend_from_slice(&level);
        while side > 1 {
            let half = side / 2;
            let mut next = Vec::with_capacity(half * half * 4);
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            u32::from(level[((2 * y + dy) * side + 2 * x + dx) * 4 + c])
                        };
                        next.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8);
                    }
                }
            }
            out.extend_from_slice(&next);
            level = next;
            side = half;
        }
    }
    out
}
impl PreparedTextures {
    pub fn new(
        pixels: &[u8],
        size: u32,
        layers: u32,
        settings: &[crate::mipmap::Settings],
        forcefield: (&[u8], u32),
    ) -> Self {
        Self::with_split(pixels, size, layers, settings, forcefield, sprite_split())
    }

    fn with_split(
        pixels: &[u8],
        size: u32,
        layers: u32,
        settings: &[crate::mipmap::Settings],
        forcefield: (&[u8], u32),
        split: u32,
    ) -> Self {
        let layer_bytes = (size * size * 4) as usize;
        Self {
            sprites: (0..layers.div_ceil(split).max(1))
                .map(|array| {
                    let first = array * split;
                    let count = layers.saturating_sub(first).clamp(1, split);
                    let start = (first as usize * layer_bytes).min(pixels.len());
                    mip_chain(&pixels[start..], size, settings, first, count)
                })
                .collect(),
            split,
            forcefield: mip_chain(forcefield.0, forcefield.1, &[], 0, 1),
            pbr: None,
        }
    }

    /// With the resource packs' LabPBR `normals` and `specular` maps (`layers` layers of
    /// `size`, as `RenderAssets` holds them; nothing when empty).
    pub fn with_pbr(mut self, normals: &[u8], specular: &[u8], size: u32, layers: u32) -> Self {
        if !normals.is_empty() {
            self.pbr = Some((
                box_mip_chain(normals, size, layers),
                box_mip_chain(specular, size, layers),
                size.trailing_zeros() + 1,
                layers,
                size,
            ));
        }
        self
    }

    /// The same mip chains cut by `split` layers per array: chains made before
    /// the adapter was known, for a GPU with fewer layers per array.
    fn resplit(mut self, layers: u32, split: u32) -> Self {
        if self.split == split || layers <= split.min(self.split) {
            self.split = split;
            return self;
        }
        let levels = self.sprites[0].1;
        let all: Vec<u8> = self
            .sprites
            .drain(..)
            .flat_map(|(bytes, _)| bytes)
            .collect();
        // Every layer's mip levels are contiguous (layer major).
        let layer_bytes = all.len() / layers as usize;
        self.sprites = all
            .chunks(layer_bytes * split as usize)
            .map(|chunk| (chunk.to_vec(), levels))
            .collect();
        self.split = split;
        self
    }
}

fn mip_chain(
    pixels: &[u8],
    size: u32,
    settings: &[crate::mipmap::Settings],
    first: u32,
    layers: u32,
) -> (Vec<u8>, u32) {
    let levels = size.trailing_zeros() + 1;
    let layer_bytes = (size * size * 4) as usize;
    // Exactly the chain's size: cached animation frames count their capacity.
    let chain_bytes: usize = (0..levels).map(|l| layer_bytes >> (2 * l)).sum();
    let mut out = Vec::with_capacity(chain_bytes * layers as usize);
    for layer in 0..layers as usize {
        let base = crate::mipmap::Image {
            width: size,
            height: size,
            pixels: pixels[layer * layer_bytes..(layer + 1) * layer_bytes]
                .as_chunks::<4>()
                .0
                .to_vec(),
        };
        let s = settings
            .get(first as usize + layer)
            .copied()
            .unwrap_or_default();
        let has_transparent = s.has_transparent || base.pixels.iter().any(|p| p[3] == 0);
        for image in crate::mipmap::generate(
            base,
            levels - 1,
            crate::mipmap::Settings {
                has_transparent,
                ..s
            },
        ) {
            out.extend(image.pixels.iter().flatten());
        }
    }
    debug_assert_eq!(out.len(), chain_bytes * layers as usize);
    (out, levels)
}
/// The wgpu instance options, without wgpu's validation of indirect draws
/// in release builds. That validation (meant for untrusted WebGPU content)
/// copies every indirect draw through a compute pass before the render pass
/// and costs about a tenth of `render`'s CPU time on Metal. The only indirect
/// draws are the section draws `render` writes from resident sections, in
/// bounds by construction: index counts within `indices`, vertices within
/// the section's arena range, first instances below the origins written that
/// frame. Debug builds keep validating them; `WGPU_VALIDATION_INDIRECT_CALL=1`
/// turns it back on. DirectX 12 keeps it too: there wgpu-core's validation
/// pass is also what passes each draw's first instance to shaders reading
/// `instance_index`, and DirectX 12 is the fallback, where working matters
/// more than CPU time.
/// `allow_nonconformant` offers the adapters wgpu hides for reporting no Vulkan
/// conformance version: Mesa's hasvk, which drives Ivy Bridge and Haswell,
/// reports 0.0.0.0 while it works. `open_device` asks for them only on a second
/// try at a backend that had no adapter at all.
fn instance_descriptor(
    backends: wgpu::Backends,
    window: Option<&Arc<Window>>,
    allow_nonconformant: bool,
) -> wgpu::InstanceDescriptor {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if let Some(window) = window {
        descriptor = descriptor.with_display_handle(Box::new(WindowDisplay(window.clone())));
    }
    descriptor.backends = backends.with_env();
    let dx12 = cfg!(windows) && descriptor.backends.contains(wgpu::Backends::DX12);
    if !cfg!(debug_assertions) && !dx12 {
        descriptor
            .flags
            .remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    }
    let mut descriptor = descriptor.with_env();
    if allow_nonconformant {
        descriptor
            .flags
            .insert(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER);
    }
    descriptor
}
/// The window's display connection, so the OpenGL backend creates its EGL
/// display from the X11 or Wayland connection the window lives on. Without
/// one, wgpu-hal falls back to `EGL_MESA_platform_surfaceless`, whose configs
/// cannot present to a window; the backend then reports the surface as
/// incompatible and the game has no OpenGL to fall back to (`gl not
/// compatible with provided surface`).
#[derive(Debug, Clone)]
struct WindowDisplay(Arc<Window>);
impl wgpu::rwh::HasDisplayHandle for WindowDisplay {
    fn display_handle(
        &self,
    ) -> std::result::Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        wgpu::rwh::HasDisplayHandle::display_handle(&*self.0)
    }
}
/// The backends to open a device on, each tried in turn until one gives a
/// device. `api` is the player's choice (`GraphicsApi::with_env` applied);
/// `os` is `std::env::consts::OS`; `wgpu_backend`: whether `WGPU_BACKEND` is set,
/// which picks the backends itself. Auto tries Vulkan, then DirectX 12 on Windows,
/// then OpenGL (Windows and Linux) for GPUs with neither driver, each when the one
/// before has no surface, adapter or device. A native crash in the driver can't be
/// caught in the game; the launcher switches the setting to DirectX 12 after one at
/// the renderer's start (#746). macOS has Metal only.
fn backend_order(api: GraphicsApi, os: &str, wgpu_backend: bool) -> Vec<wgpu::Backends> {
    use wgpu::Backends;
    if os == "macos" || wgpu_backend {
        return vec![Backends::default()];
    }
    let windows = os == "windows";
    match api {
        GraphicsApi::Auto if windows => vec![Backends::VULKAN, Backends::DX12, Backends::GL],
        GraphicsApi::Auto => vec![Backends::VULKAN, Backends::GL],
        GraphicsApi::Dx12 if windows => vec![Backends::DX12],
        GraphicsApi::Vulkan | GraphicsApi::Dx12 => vec![Backends::VULKAN],
        GraphicsApi::OpenGl => vec![Backends::GL],
    }
}
fn backends_name(backends: wgpu::Backends) -> &'static str {
    match backends {
        wgpu::Backends::VULKAN => "Vulkan",
        wgpu::Backends::DX12 => "DirectX 12",
        wgpu::Backends::GL => "OpenGL",
        _ => "graphics",
    }
}
/// Whether an adapter renders on the CPU (WARP, llvmpipe).
fn is_software(kind: wgpu::DeviceType) -> bool {
    kind == wgpu::DeviceType::Cpu
}
/// A device and what it came from.
struct Opened {
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    device_lost: Arc<std::sync::atomic::AtomicBool>,
}
/// Logs the adapter and records it for telemetry and crash reports.
fn describe_adapter(adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    log::info!(
        "Horizon renderer: {} ({:?}, {:?})",
        info.name,
        info.backend,
        info.device_type
    );
    crate::telemetry::set_gpu(crate::telemetry::Gpu {
        name: info.name.clone(),
        vendor: horizon_telemetry::system::gpu_vendor(&info.name, info.vendor).to_owned(),
        kind: crate::telemetry::gpu_kind(&format!("{:?}", info.device_type)).to_owned(),
        driver: format!("{} {}", info.driver, info.driver_info)
            .trim()
            .to_owned(),
        backend: format!("{:?}", info.backend),
    });
    crate::crash::set_detail(
        "Graphics Adapter",
        format!(
            "{} ({:?}, {:?}), driver {} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        ),
    );
}
/// Whether an adapter's texture arrays hold the block textures Horizon puts
/// in one array.
fn check_layers(adapter: &wgpu::Adapter, layers: u32) -> Result<u32> {
    Ok(layer_split(
        adapter.limits().max_texture_array_layers,
        adapter.get_info().backend == wgpu::Backend::Gl,
        requested_split(),
        layers,
    )?)
}
/// Whether the swapchain can take a window of `width` x `height`: each side from 1 to the
/// device's `max_texture_dimension_2d` (`max`), else `Surface::configure` panics. 0 is a
/// minimized window. Past `max`: Windows lets a program size a window up to 65535 pixels
/// (`SetWindowPos`), a 984x65496 client area, when an RTX 5080 takes 32768. Vanilla's
/// `Window.onFramebufferResize` likewise keeps the last size for one that is not positive.
fn drawable_size(width: u32, height: u32, max: u32) -> bool {
    (1..=max).contains(&width) && (1..=max).contains(&height)
}
/// The swapchain format: a non-sRGB target, because vanilla blends and tints in gamma
/// space, and 8-bit, because wgpu gives `Rgba16Float` the linear `ExtendedSrgb` color
/// space: the compositor would read these gamma-encoded values as linear light and wash
/// every colour out.
fn surface_format(
    formats: &[wgpu::TextureFormat],
    default: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
    .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
    .unwrap_or(default)
}
/// Opens a device on the first of `backend_order`'s backends that gives one.
/// Offscreen rendering needs no surface, so it can use a GPU the window's
/// display cannot present from (a headless compositor).
async fn open_device(
    window: &Arc<Window>,
    offscreen: bool,
    api: GraphicsApi,
    layers: u32,
) -> Result<Opened> {
    // A software adapter (WARP, llvmpipe) is a last resort: a later backend may
    // have the real GPU (#3978: a Fermi GPU with no Vulkan and no DX12 driver
    // still has OpenGL).
    match open_device_with(window, offscreen, api, layers, false).await {
        Ok(opened) => Ok(opened),
        Err(first) => match open_device_with(window, offscreen, api, layers, true).await {
            Ok(opened) => {
                log::warn!(
                    "Horizon renderer: no GPU adapter could be used ({first}); \
                     rendering on the CPU, which is slow"
                );
                Ok(opened)
            }
            Err(_) => Err(first),
        },
    }
}
/// `open_device`, taking an adapter of the CPU type (software rendering) only
/// when `allow_cpu`.
async fn open_device_with(
    window: &Arc<Window>,
    offscreen: bool,
    api: GraphicsApi,
    layers: u32,
    allow_cpu: bool,
) -> Result<Opened> {
    let order = backend_order(
        api.with_env(),
        std::env::consts::OS,
        std::env::var_os("WGPU_BACKEND").is_some(),
    );
    let mut failures = Vec::new();
    for (attempt, backends) in order.iter().copied().enumerate() {
        let name = backends_name(backends);
        if attempt > 0 {
            log::warn!("Horizon renderer: trying {name} ({})", failures.join("; "));
        }
        match open_with_retry(backends, window, offscreen, layers, allow_cpu).await {
            Ok(opened) => return Ok(opened),
            Err(e) => failures.push(e),
        }
    }
    Err(format!("{NO_GPU_DRIVER} ({})", failures.join("; ")).into())
}
/// Why a backend gave no device: no adapter at all, which a second try with the
/// adapters wgpu hides can still find, or a failure after an adapter.
enum NoDevice {
    NoAdapter(String),
    Failed(String),
}
/// Opens a device on one backend, and on a backend whose adapters wgpu all hid
/// for reporting no Vulkan conformance version tries it again with them shown:
/// Mesa's hasvk drives Ivy Bridge and Haswell, works, and reports 0.0.0.0, so
/// without this the player is told they have no graphics driver while Vulkan
/// runs (a `gl not compatible with provided surface` fallback, or nothing, is
/// all they get). Returns why the backend gave no device.
async fn open_with_retry(
    backends: wgpu::Backends,
    window: &Arc<Window>,
    offscreen: bool,
    layers: u32,
    allow_cpu: bool,
) -> std::result::Result<Opened, String> {
    match open_backend(backends, window, offscreen, false, layers, allow_cpu).await {
        Err(NoDevice::NoAdapter(hidden)) => {
            log::warn!(
                "Horizon renderer: {} has no conformant adapter ({hidden}); asking again for the \
                 ones wgpu hides as non-conformant",
                backends_name(backends)
            );
            match open_backend(backends, window, offscreen, true, layers, allow_cpu).await {
                Ok(opened) => {
                    log::warn!(
                        "Horizon renderer: using {} although its driver reports no Vulkan \
                         conformance version",
                        opened.adapter.get_info().name
                    );
                    Ok(opened)
                }
                Err(NoDevice::NoAdapter(e)) | Err(NoDevice::Failed(e)) => Err(e),
            }
        }
        Err(NoDevice::Failed(e)) => Err(e),
        Ok(opened) => Ok(opened),
    }
}
/// Opens a device on `backends` alone: its surface, adapter and device, or what
/// stopped it. `allow_nonconformant` is `open_with_retry`'s second try; a
/// software adapter is taken only when `allow_cpu`.
async fn open_backend(
    backends: wgpu::Backends,
    window: &Arc<Window>,
    offscreen: bool,
    allow_nonconformant: bool,
    layers: u32,
    allow_cpu: bool,
) -> std::result::Result<Opened, NoDevice> {
    let name = backends_name(backends);
    let instance = wgpu::Instance::new(instance_descriptor(
        backends,
        Some(window),
        allow_nonconformant,
    ));
    let surface = if offscreen {
        None
    } else {
        match instance.create_surface(window.clone()) {
            Ok(surface) => Some(surface),
            Err(e) => return Err(NoDevice::Failed(format!("{name} surface: {e}"))),
        }
    };
    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface.as_ref(),
            force_fallback_adapter: false,
            ..Default::default()
        })
        .await
    {
        Ok(adapter) => adapter,
        Err(e) => return Err(NoDevice::NoAdapter(format!("{name}: {e}"))),
    };
    // NoAdapter, so `open_with_retry` asks again: with the non-conformant
    // adapters shown, wgpu ranks a real GPU (hasvk on Ivy Bridge or Haswell)
    // above a software one. The second try's error is final either way.
    if !allow_cpu && is_software(adapter.get_info().device_type) {
        return Err(NoDevice::NoAdapter(format!(
            "{name}: only a software adapter ({})",
            adapter.get_info().name
        )));
    }
    describe_adapter(&adapter);
    let info = adapter.get_info();
    let limits = adapter.limits();
    if let Err(e) = check_layers(&adapter, layers) {
        return Err(NoDevice::Failed(format!("{name}: {e}")));
    }
    // Mappable primary buffers: section pages that no longer fit in
    // video memory go to system memory. Shader packs' 16-bit normalized buffers (render
    // targets with the adapter's own format features) and R11F_G11F_B10F: a float
    // stand-in neither clamps nor drops NaN. Subgroups: shader packs' compute programs
    // (`MC_GL_KHR_shader_subgroup_*`); storage buffers in vertex shaders, clearing and
    // atomics on images: their buffers and images.
    let features = timing::GpuTimer::features(&adapter)
        | cull::Cull::features(&adapter)
        | oit::features(&adapter)
        | (adapter.features()
            & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                | wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
                | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::RG11B10UFLOAT_RENDERABLE
                | wgpu::Features::SUBGROUP
                | wgpu::Features::VERTEX_WRITABLE_STORAGE
                | wgpu::Features::CLEAR_TEXTURE
                | wgpu::Features::TEXTURE_ATOMIC));
    let driver = format!("{} {}", info.driver, info.driver_info);
    startup_step(format_args!(
        "requesting the device (driver {}) with features [{features}] and the adapter's \
         limits (buffers up to {} bytes, storage bindings up to {} bytes, {} texture layers)",
        match driver.trim() {
            "" => "unnamed",
            driver => driver,
        },
        limits.max_buffer_size,
        limits.max_storage_buffer_binding_size,
        limits.max_texture_array_layers,
    ));
    let desc = wgpu::DeviceDescriptor {
        label: Some("Horizon device"),
        required_features: features,
        required_limits: limits,
        // Video memory in blocks of 8-64 MiB instead of 128-256 MiB (Vulkan and DirectX 12):
        // with the larger blocks a few small buffers kept whole blocks the arena's pages had
        // left, and over half the video memory reserved was unused (1,344 MiB for 500).
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    };
    #[cfg(not(target_os = "macos"))]
    let opened = match info.backend {
        wgpu::Backend::Vulkan => horizon_hdr::open_vulkan_device(&adapter, &desc),
        _ => None,
    };
    #[cfg(target_os = "macos")]
    let opened = None;
    let opened = match opened {
        Some(opened) => opened,
        None => adapter.request_device(&desc).await,
    };
    match opened {
        Ok((device, queue)) => {
            let device_lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let lost = device_lost.clone();
            device.set_device_lost_callback(move |reason, message| {
                // A device destroyed on purpose was not lost.
                if reason == wgpu::DeviceLostReason::Destroyed {
                    return;
                }
                log::warn!(
                    "Horizon renderer: the graphics device was lost ({reason:?}: {message})"
                );
                lost.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            Ok(Opened {
                instance,
                surface,
                adapter,
                device,
                queue,
                device_lost,
            })
        }
        Err(e) => Err(NoDevice::Failed(format!("{name} device: {e}"))),
    }
}
/// What to tell a player whose system offers no usable graphics adapter.
/// Vulkan is loaded at run time (the game links no graphics library), so a
/// missing driver shows here rather than as a missing-DLL dialog.
#[cfg(target_os = "windows")]
const NO_GPU_DRIVER: &str = "No graphics driver with Vulkan, DirectX 12 or OpenGL found: Horizon needs \
     a GPU driver with one of them. Install the latest driver for your NVIDIA, AMD or Intel GPU and \
     start the game again";
#[cfg(target_os = "macos")]
const NO_GPU_DRIVER: &str = "No Metal graphics device found";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const NO_GPU_DRIVER: &str = "No Vulkan or OpenGL graphics driver found: Horizon needs the Vulkan loader \
     (libvulkan.so.1) and a Vulkan driver for your GPU (Mesa's, or NVIDIA's), or OpenGL through EGL";
/// Logs a step of the renderer's start, for a native crash in the graphics
/// driver: it leaves no panic or crash report, so the log's last line is the
/// only sign of where the game stopped. The log file is written unbuffered;
/// the flush is for loggers that buffer.
fn startup_step(step: std::fmt::Arguments) {
    log::info!("Horizon renderer: {step}");
    log::logger().flush();
}
/// How far the first frame on a new device got, for `startup_step`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstFrame {
    Acquire,
    Submit,
    Drawn,
}
fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
impl Renderer {
    /// Sections with GPU buffers, for soak tests watching residency.
    pub fn resident_sections(&self) -> usize {
        self.sections.len()
    }
    /// Quads of the far terrain tiles with GPU buffers.
    pub fn lod_quads(&self) -> u64 {
        self.lods
            .values()
            .map(|tile| u64::from(tile.mesh.solid + tile.mesh.translucent))
            .sum()
    }
    /// Tiles and quads of the far terrain tiles with GPU buffers, by level.
    pub fn lod_levels(&self) -> Vec<[u64; 2]> {
        let mut levels = Vec::new();
        for (key, tile) in &self.lods {
            let level = usize::from(key.level);
            if levels.len() <= level {
                levels.resize(level + 1, [0, 0]);
            }
            levels[level][0] += 1;
            levels[level][1] += u64::from(tile.mesh.solid + tile.mesh.translucent);
        }
        levels
    }
    /// Bytes of the section vertex arenas' buffers, and of the vertices
    /// resident sections hold in them.
    pub fn section_vertex_bytes(&self) -> [u64; 2] {
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let vertices: u64 = self
            .sections
            .values()
            .chain(self.lods.values().map(|tile| &tile.mesh))
            .map(|s| u64::from(s.solid + s.translucent) * 4)
            .sum();
        let translucent = self.translucent_arena.as_ref();
        let arenas = self.arena.bytes() + translucent.map_or(0, arena::Arena::bytes);
        [arenas, vertices * stride]
    }
}
impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        offscreen: bool,
        sprite_pixels: &[u8],
        sprite_size: u32,
        sprite_minimum_size: u32,
        layers: u32,
        sprite_mips: &[crate::mipmap::Settings],
        animations: &[SpriteAnimation],
        hud_atlas: (&[u8], u32, u32),
        glint: (&[u8], u32, u32),
        glint_armor: (&[u8], u32, u32),
        forcefield: (&[u8], u32),
        panorama_faces: Arc<panorama::Faces>,
        prepared: Option<PreparedTextures>,
        previous: Option<&Self>,
        api: GraphicsApi,
        translucent_batching: bool,
        shader_pack: bool,
    ) -> Result<Self> {
        let Opened {
            instance,
            surface,
            adapter,
            device,
            queue,
            device_lost,
        } = match previous {
            // Reload resources on the same device; replacing live Vulkan
            // devices can invalidate the driver's swapchain state on the same
            // window.
            Some(renderer) => {
                describe_adapter(&renderer.adapter);
                check_layers(&renderer.adapter, layers)?;
                Opened {
                    surface: if offscreen {
                        None
                    } else {
                        let spare = renderer
                            .spare_surface
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        Some(match spare {
                            Some(surface) => surface,
                            None => renderer.instance.create_surface(window.clone())?,
                        })
                    },
                    instance: renderer.instance.clone(),
                    adapter: renderer.adapter.clone(),
                    device: renderer.device.clone(),
                    queue: renderer.queue.clone(),
                    device_lost: renderer.device_lost.clone(),
                }
            }
            None => {
                let _step = crate::spans::startup::step("renderer.open_device");
                open_device(&window, offscreen, api, layers).await?
            }
        };
        // Computed once: the sprite arrays, the shaders' constants and the
        // shader pack atlas all cut the sprites by it.
        let split = check_layers(&adapter, layers)?;
        if split < sprite_split() {
            log::info!("Horizon: {split} block textures per array (the GPU's limit)");
        }
        let info = adapter.get_info();
        crate::graphics::suggest_reduced_far_detail(crate::graphics::lod_capped(&info));
        let size = window.inner_size();
        let max_side = device.limits().max_texture_dimension_2d;
        let (config, present_modes, hdr_output) = if let Some(surface) = &surface {
            let mut config = surface
                .get_default_config(
                    &adapter,
                    size.width.clamp(1, max_side),
                    size.height.clamp(1, max_side),
                )
                .ok_or("surface has no supported configuration")?;
            let caps = surface.get_capabilities(&adapter);
            config.format = surface_format(&caps.formats, config.format);
            config.view_formats = vec![config.format.remove_srgb_suffix()];
            // Screenshots copy the presented frame back from the swapchain, frame
            // generation's present thread copies its frames in.
            if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                config.usage |= wgpu::TextureUsages::COPY_SRC;
            }
            if caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
                config.usage |= wgpu::TextureUsages::COPY_DST;
            }
            config.present_mode = previous
                .map(|renderer| renderer.config.present_mode)
                .filter(|mode| caps.present_modes.contains(mode))
                .unwrap_or_else(|| {
                    if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                        wgpu::PresentMode::Mailbox
                    } else {
                        wgpu::PresentMode::Fifo
                    }
                });
            if previous.is_none() {
                startup_step(format_args!(
                    "device ready; configuring the surface ({}x{} {:?}, {:?})",
                    config.width, config.height, config.format, config.present_mode
                ));
            }
            surface.configure(&device, &config);
            let hdr_output = hdr::output(&caps);
            crate::graphics::offer_hdr(hdr_output.is_some());
            if previous.is_none() {
                log::info!(
                    "Horizon renderer: HDR output {}",
                    hdr_output.map_or("not offered by this display", hdr::Output::name)
                );
            }
            (config, caps.present_modes.clone(), hdr_output)
        } else {
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                format,
                width: size.width.clamp(1, max_side),
                height: size.height.clamp(1, max_side),
                present_mode: wgpu::PresentMode::Immediate,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![format],
                color_space: wgpu::SurfaceColorSpace::default(),
            };
            (config, Vec::new(), None)
        };
        let offscreen = surface
            .is_none()
            .then(|| Self::offscreen_target(&device, &config));
        if previous.is_none() {
            startup_step(format_args!("creating the pipelines"));
        }
        // A renderer on the same device takes the previous one's shader
        // modules, layouts and pipelines where they come out the same.
        let reuse = previous.map_or_else(Default::default, |r| r.reuse.clone());
        let generation = reuse.begin();
        let kept = reuse.on(&device);
        let pipelines_step = crate::spans::startup::step("renderer.pipelines_textures");
        let terrain_step = crate::spans::startup::step("renderer.terrain_pipelines");
        let gpu_timer = timing::GpuTimer::new(&device, &queue);
        let shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let sprite_array = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        };
        let sprite_arrays = layers.div_ceil(split).max(1);
        if sprite_arrays > 1 {
            log::info!("Horizon: {layers} block textures in {sprite_arrays} arrays");
        }
        let sprite_constants = [
            ("SPRITE_ARRAYS", f64::from(sprite_arrays)),
            ("SPRITE_SPLIT", f64::from(split)),
        ];
        let bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon terrain bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(
                    1 + 1,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(5, vf, sprite_array),
                entry(6, vf, sprite_array),
                entry(7, vf, sprite_array),
                entry(8, vf, sprite_array),
                entry(9, vf, sprite_array),
                entry(10, vf, sprite_array),
                entry(11, vf, sprite_array),
                entry(
                    12,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        });
        let layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon terrain layout"),
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let instance_attributes = wgpu::vertex_attr_array![5=>Float32x4];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let alpha_blend = wgpu::BlendState::ALPHA_BLENDING;
        let debug_line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let debug_line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &debug_line_attributes,
        })];
        let pipeline_in = |layout: &wgpu::PipelineLayout,
                           label: &str,
                           vertex: &str,
                           fragment: &str,
                           buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                           topology: wgpu::PrimitiveTopology,
                           cull: Option<wgpu::Face>,
                           depth_write: bool,
                           depth_test: bool,
                           blend: Option<wgpu::BlendState>| {
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &sprite_constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = |label: &str,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        cull: Option<wgpu::Face>,
                        depth_write: bool,
                        depth_test: bool,
                        blend: Option<wgpu::BlendState>| {
            pipeline_in(
                &layout,
                label,
                vertex,
                fragment,
                buffers,
                topology,
                cull,
                depth_write,
                depth_test,
                blend,
            )
        };
        let tri = wgpu::PrimitiveTopology::TriangleList;
        let solid = pipeline(
            "Terrain solid",
            "vs_terrain",
            "fs_solid",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            None,
        );
        // `solid` in two passes (terrain.wgsl's `opaque_quad`, `mesher::Part`), for the
        // world pass without a shader pack: no cutout first, through a shader that never
        // discards, then the rest.
        let solid_opaque = pipeline(
            "Terrain solid opaque",
            "vs_terrain_opaque",
            "fs_opaque",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            None,
        );
        let solid_cutout = pipeline(
            "Terrain solid cutout",
            "vs_terrain_cutout",
            "fs_solid",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            None,
        );
        // The water mirror's camera sees the world from the other side of the plane, which
        // flips the winding: its solid terrain culls the front.
        let solid_mirror = pipeline(
            "Terrain solid (mirrored)",
            "vs_terrain",
            "fs_solid_mirror",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Front),
            true,
            true,
            None,
        );
        let translucent = pipeline(
            "Terrain translucent",
            "vs_terrain",
            "fs_translucent",
            &terrain_buffers,
            tri,
            Some(wgpu::Face::Back),
            true,
            true,
            Some(alpha_blend),
        );
        let sky = pipeline(
            "Sky",
            "vs_sky",
            "fs_sky",
            &[],
            tri,
            None,
            false,
            false,
            None,
        );
        let dark_disc = pipeline(
            "Dark disc",
            "vs_sky",
            "fs_dark_disc",
            &[],
            tri,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        let depth_reset = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Hand depth reset"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_depth_reset"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &sprite_constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let lines = pipeline(
            "Block outline",
            "vs_line",
            "fs_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            true,
            true,
            Some(alpha_blend),
        );
        // RenderTypes.secondaryBlockOutline: the black stroke under a high contrast outline.
        let lines_secondary = pipeline(
            "Block outline secondary",
            "vs_line",
            "fs_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        let debug_lines = pipeline(
            "Debug lines",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            true,
            Some(alpha_blend),
        );
        let debug_lines_on_top = pipeline(
            "Debug lines on top",
            "vs_debug_line",
            "fs_debug_line",
            &debug_line_buffers,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
            false,
            false,
            Some(alpha_blend),
        );
        drop(terrain_step);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("First-person hand globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let crosshair_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug crosshair globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mirror_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water mirror globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prepared = match prepared {
            Some(prepared) => prepared.resplit(layers, split),
            None => PreparedTextures::with_split(
                sprite_pixels,
                sprite_size,
                layers,
                sprite_mips,
                forcefield,
                split,
            ),
        };
        let mut mip_levels = 1;
        let gl = adapter.get_info().backend == wgpu::Backend::Gl;
        // The resource packs' LabPBR maps, for native shader packs: one array each, cut to
        // the layers the GPU has (`block_normal_map` gives the default past them).
        let pbr_maps = prepared
            .pbr
            .map(|(normals, specular, levels, layers, size)| {
                let kept = layers.min(device.limits().max_texture_array_layers);
                let count = array_layers(kept, gl);
                [("LabPBR normals", normals), ("LabPBR specular", specular)].map(
                    |(label, mut data)| {
                        // The extra layers are empty.
                        let layer_bytes = data.len() / layers as usize;
                        data.resize(layer_bytes * count as usize, 0);
                        device.create_texture_with_data(
                            &queue,
                            &wgpu::TextureDescriptor {
                                label: Some(label),
                                size: wgpu::Extent3d {
                                    width: size,
                                    height: size,
                                    depth_or_array_layers: count,
                                },
                                mip_level_count: levels,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            },
                            wgpu::util::TextureDataOrder::LayerMajor,
                            &data,
                        )
                    },
                )
            });
        let sprite_textures: Vec<wgpu::Texture> = prepared
            .sprites
            .into_iter()
            .enumerate()
            .map(|(array, (mut mips, levels))| {
                let first = array as u32 * split;
                let used = layers.saturating_sub(first).clamp(1, split);
                let count = array_layers(used, gl);
                // The extra layers are empty.
                mips.resize(mips.len() / used as usize * count as usize, 0);
                mip_levels = levels;
                device.create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some("Block sprites"),
                        size: wgpu::Extent3d {
                            width: sprite_size,
                            height: sprite_size,
                            depth_or_array_layers: count,
                        },
                        mip_level_count: levels,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            // Shader packs' atlas copies them (pack_atlas.rs).
                            | wgpu::TextureUsages::COPY_SRC
                            | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &mips,
                )
            })
            .collect();
        let animations = animate::SpriteAnimator::new(
            &device,
            animations,
            sprite_size,
            sprite_mips,
            &sprite_textures,
            split,
            animate::SpriteAnimator::compute(&adapter, &device),
        );
        let active_layer_words = animations.active_layer_words();
        let sprite_unused = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Block sprites (unused)"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: array_layers(1, gl),
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let sprite_views = sprite_views(&sprite_textures, &sprite_unused, None);
        let sprite_sampler = device.create_sampler(&sprite_sampler_descriptor(1));
        let lightmap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Lightmap"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lightmap_view = lightmap.create_view(&Default::default());
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Lightmap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let emission_stats =
            hdr_stats::EmissionStats::new(&device, split, sprite_textures.len() as u32);
        let group = terrain_group(
            &device,
            "Horizon terrain resources",
            &bindings,
            &globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let crosshair_group = terrain_group(
            &device,
            "Debug crosshair resources",
            &bindings,
            &crosshair_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let mirror_group = terrain_group(
            &device,
            "Water mirror resources",
            &bindings,
            &mirror_globals,
            &sprite_views,
            &sprite_sampler,
            &lightmap_view,
            &lightmap_sampler,
            &emission_stats.view,
        );
        let index_quads = 1 << 16;
        let indices = Self::quad_indices(&device, index_quads);
        let instance_capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section origins"),
            size: instance_capacity * 16,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let outline = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline"),
            size: 32 * 2 * MAX_OUTLINE_EDGES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let debug_line_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Debug lines"),
            size: 32 * 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (atlas_pixels, atlas_width, atlas_height) = hud_atlas;
        let limit = device.limits().max_texture_dimension_2d;
        if atlas_width.max(atlas_height) > limit {
            return Err(format!(
                "the shared atlas is {atlas_width}x{atlas_height}; this GPU allows {limit}x{limit}"
            )
            .into());
        }
        let atlas = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Shared atlas"),
                size: wgpu::Extent3d {
                    width: atlas_width,
                    height: atlas_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                // COPY_SRC: a shader pack's weather copies the rain and snow sprites out.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            atlas_pixels,
        );
        let atlas_view = atlas.create_view(&Default::default());
        // vanilla's `GlintSampler`: enchanted_glint_item as its own texture,
        // repeated per pixel (`clamp: false`), linear without mipmaps
        // (`blur: true`).
        let (glint_pixels, glint_width, glint_height) = glint;
        let glint_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Enchanted glint"),
                size: wgpu::Extent3d {
                    width: glint_width,
                    height: glint_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            glint_pixels,
        );
        let glint_view = glint_texture.create_view(&Default::default());
        let glint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Glint"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // ARMOR_ENTITY_GLINT_TEXTURING's sampler is vanilla's GlintSampler: the
        // same repeat and linear filtering as the item glint.
        let (glint_armor_pixels, glint_armor_width, glint_armor_height) = glint_armor;
        let glint_armor_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Enchanted armor glint"),
                size: wgpu::Extent3d {
                    width: glint_armor_width,
                    height: glint_armor_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            glint_armor_pixels,
        );
        let glint_armor_view = glint_armor_texture.create_view(&Default::default());
        let mut hud = HudRenderer::new(
            &device,
            config.format.remove_srgb_suffix(),
            &atlas_view,
            &glint_view,
            &glint_sampler,
        )?;
        hud.atlas_texture = Some(atlas.clone());
        let entity_step = crate::spans::startup::step("renderer.entity_pipelines");
        let entity_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon entities"),
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let entity_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Entity bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    5,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    6,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    7,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        });
        let weather = weather::Weather::new(
            &device,
            config.format.remove_srgb_suffix(),
            &entity_bindings,
            &entity_shader,
        );
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas"),
            ..Default::default()
        });
        let entity_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Entity resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&glint_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&glint_armor_view),
                },
            ],
        });
        let hand_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("First-person hand resources"),
            layout: &entity_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: hand_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&glint_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&glint_armor_view),
                },
            ],
        });
        let entity_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Entity layout"),
            bind_group_layouts: &[Some(&entity_bindings)],
            ..Default::default()
        });
        let cloud_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud bindings"),
            entries: &[entry(
                0,
                wgpu::ShaderStages::VERTEX,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )],
        });
        let cloud_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud transforms and colors"),
            size: std::mem::size_of::<crate::clouds::Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cloud_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cloud resources"),
            layout: &cloud_bindings,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cloud_uniform.as_entire_binding(),
            }],
        });
        let cloud_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud layout"),
            bind_group_layouts: &[Some(&entity_bindings), Some(&cloud_bindings)],
            ..Default::default()
        });
        let cloud_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cloud vertices"),
            size: 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let entity_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &entity_attributes,
        })];
        #[allow(clippy::too_many_arguments)]
        let entity_pipeline_with = |label,
                                    cull_mode,
                                    depth_write,
                                    compare,
                                    bias,
                                    blend,
                                    clouds,
                                    fragment: &str,
                                    write_mask| {
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(if clouds {
                    &cloud_layout
                } else {
                    &entity_layout
                }),
                vertex: wgpu::VertexState {
                    module: &entity_shader,
                    entry_point: Some(if clouds { "vs_cloud" } else { "vs_entity" }),
                    compilation_options: Default::default(),
                    buffers: &entity_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &entity_shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let entity_pipeline = |label, cull_mode, depth_write, compare, bias, blend, clouds| {
            entity_pipeline_with(
                label,
                cull_mode,
                depth_write,
                compare,
                bias,
                blend,
                clouds,
                "fs_entity",
                wgpu::ColorWrites::ALL,
            )
        };
        // RenderPipelines.WATER_MASK: the boat's water patch writes depth only.
        let water_mask_pipeline = entity_pipeline_with(
            "Water mask",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
            "fs_water_mask",
            wgpu::ColorWrites::empty(),
        );
        let crumbling_pipeline = entity_pipeline(
            "Breaking cracks",
            Some(wgpu::Face::Back),
            false,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::DepthBiasState {
                constant: 10,
                slope_scale: 1.0,
                clamp: 0.0,
            },
            CRUMBLING_BLEND,
            false,
        );
        // RenderPipelines.ENTITY_SHADOW: translucent, back-face culled, depth
        // test without writes. The quads lie exactly on the block tops like
        // vanilla's, facing +Y so looking up from below culls them, but
        // terrain and shadows are separate draw paths, so their fp32 depth
        // interpolations disagree by a few dozen ulps of the fragment depth
        // and the shadow's rim z-fights with the block top (speckle that
        // re-rolls as the camera drifts a subpixel). A constant bias of 128
        // ulps covers that spread at any resolution while staying far below
        // the 0.001 a model's feet sit higher, so shadows never bleed onto
        // the feet.
        let shadow_blob_pipeline = entity_pipeline_with(
            "Entity shadows",
            Some(wgpu::Face::Back),
            false,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::DepthBiasState {
                constant: 128,
                slope_scale: 0.015,
                clamp: 0.0,
            },
            alpha_blend,
            false,
            "fs_entity_shadow",
            wgpu::ColorWrites::ALL,
        );
        // RenderPipelines.ITEM_TRANSLUCENT: blended, depth tested. Drawn before
        // translucent terrain, it cannot write depth: the water behind a
        // glass item would fail the test and vanish (vanilla sorts these in
        // its order independent transparency pass instead).
        let translucent_item_pipeline = entity_pipeline(
            "Translucent items",
            None,
            false,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
        );
        let cloud_pipeline = entity_pipeline(
            "Clouds",
            Some(wgpu::Face::Back),
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            true,
        );
        // RenderPipelines.TEXT_SEE_THROUGH: translucent, no depth test or
        // writes, culled like every world text pipeline (TEXT_SNIPPET keeps
        // RenderPipeline.Builder's default): a name tag or text display hides
        // its back.
        let see_through_pipeline = entity_pipeline(
            "See-through text",
            Some(wgpu::Face::Back),
            false,
            wgpu::CompareFunction::Always,
            Default::default(),
            alpha_blend,
            false,
        );
        // RenderPipelines.TEXT_POLYGON_OFFSET: a text display's text and
        // background win against the surface they lie on (bias 1, 10), from
        // the front only.
        let text_polygon_offset_pipeline = entity_pipeline(
            "Polygon offset text",
            Some(wgpu::Face::Back),
            true,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::DepthBiasState {
                constant: 10,
                slope_scale: 1.0,
                clamp: 0.0,
            },
            alpha_blend,
            false,
        );
        let entity_pipeline = entity_pipeline(
            "Entities",
            None,
            true,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            alpha_blend,
            false,
        );
        // RenderPipelines.ENERGY_SWIRL: BlendFunction.ADDITIVE (one, one), no culling; no depth
        // writes either: vanilla sorts the quads far to near, so every face passes the test.
        let one_one = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let energy_swirl_pipeline = entity_pipeline_with(
            "Energy swirls",
            None,
            false,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            wgpu::BlendState {
                color: one_one,
                alpha: one_one,
            },
            false,
            "fs_energy_swirl",
            wgpu::ColorWrites::ALL,
        );
        // HDR output's mark on light-giving particles (entity.wgsl fs_hdr_mark).
        let hdr_mark_pipeline = entity_pipeline_with(
            "HDR particle mark",
            None,
            false,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            wgpu::BlendState::REPLACE,
            false,
            "fs_hdr_mark",
            wgpu::ColorWrites::ALPHA,
        );
        let hdr_hand_unmark_pipeline = entity_pipeline_with(
            "HDR hand unmark",
            None,
            false,
            wgpu::CompareFunction::GreaterEqual,
            Default::default(),
            wgpu::BlendState::REPLACE,
            false,
            "fs_hdr_hand_unmark",
            wgpu::ColorWrites::ALPHA,
        );
        // Attribute 7: 6 is the entity vertex buffer's glint UV.
        let lightning_attributes = wgpu::vertex_attr_array![7=>Float32x3];
        let lightning_vertex = [
            entity_vertex[0].clone(),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &lightning_attributes,
            }),
        ];
        // BlendFunction.LIGHTNING: src alpha, one; no depth writes.
        let additive_descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("Additive entities"),
            layout: Some(&entity_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: &entity_vertex,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_celestial"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let additive_pipeline = kept.create_render_pipeline(&additive_descriptor);
        // `RenderPipelines.DRAGON_RAYS` uses DepthStencilState.DEFAULT:
        // unlike eyes, a ray hides the faces and rays behind it.
        let dragon_rays_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dragon death rays"),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..additive_descriptor.primitive
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                depth_write_enabled: Some(true),
                ..additive_descriptor.depth_stencil.clone().unwrap()
            }),
            ..additive_descriptor.clone()
        });
        // HDR output's mark on lightning bolts: fs_hdr_mark over the bolts' own vertex layout,
        // as hdr_mark_pipeline marks entities and particles (only alpha written).
        let mark_targets = [Some(wgpu::ColorTargetState {
            format: config.format.remove_srgb_suffix(),
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALPHA,
        })];
        let hdr_mark_lightning_pipeline =
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("HDR lightning mark"),
                vertex: wgpu::VertexState {
                    entry_point: Some("vs_lightning"),
                    buffers: &lightning_vertex,
                    ..additive_descriptor.vertex.clone()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &entity_shader,
                    entry_point: Some("fs_hdr_mark"),
                    compilation_options: Default::default(),
                    targets: &mark_targets,
                }),
                ..additive_descriptor.clone()
            });
        let lightning_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lightning"),
            vertex: wgpu::VertexState {
                entry_point: Some("vs_lightning"),
                buffers: &lightning_vertex,
                ..additive_descriptor.vertex.clone()
            },
            ..additive_descriptor
        });
        let celestial_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Celestial bodies"),
            layout: Some(&entity_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: &entity_vertex,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_sky_celestial"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    // RenderPipelines.CELESTIAL: additive (src alpha, one), with
                    // the colour premultiplied in fs_sky_celestial. Its alpha is
                    // how much a texel adds: HDR output (hdr.wgsl) lets the sun's
                    // and moon's discs shine; nothing else reads the frame's alpha.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        drop(entity_step);
        // WorldBorderRenderer: misc/forcefield.png as a MipmappedTexture
        // (repeat, linear minification with mipmaps, nearest magnification).
        let forcefield_size = forcefield.1;
        let (forcefield_mips, forcefield_levels) = prepared.forcefield;
        let forcefield_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("Forcefield"),
                size: wgpu::Extent3d {
                    width: forcefield_size,
                    height: forcefield_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: forcefield_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &forcefield_mips,
        );
        let forcefield_view = forcefield_texture.create_view(&Default::default());
        let forcefield_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Forcefield"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let border_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border"),
            size: std::mem::size_of::<BorderUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("World border vertices"),
            size: 16 * std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let border_bindings = kept.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("World border bindings"),
            entries: &[
                entry(
                    0,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    vf,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    vf,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    vf,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let border_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("World border resources"),
            layout: &border_bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&forcefield_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&forcefield_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: border_uniform.as_entire_binding(),
                },
            ],
        });
        let border_layout = kept.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World border layout"),
            bind_group_layouts: &[Some(&border_bindings)],
            ..Default::default()
        });
        let border_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon world border"),
            source: wgpu::ShaderSource::Wgsl(include_str!("border.wgsl").into()),
        });
        let border_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2];
        // RenderPipelines.WORLD_BORDER: BlendFunction.OVERLAY, no culling,
        // depth writes with a polygon offset of 3 towards the camera.
        let border_pipeline = kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("World border"),
            layout: Some(&border_layout),
            vertex: wgpu::VertexState {
                module: &border_shader,
                entry_point: Some("vs_border"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &border_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 3,
                    slope_scale: 3.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &border_shader,
                entry_point: Some("fs_border"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format.remove_srgb_suffix(),
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let screen_shader = kept.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon screen effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("screen.wgsl").into()),
        });
        let screen_attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Float32x4,3=>Float32x4];
        let screen_vertex = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &screen_attributes,
        })];
        let vignette_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        // One per blend and `.png.mcmeta` sampling (screen_effects::SAMPLE_*):
        // the shader is specialised, so a pixel pays for none of the others.
        let screen_pipelines: [wgpu::RenderPipeline; 16] = std::array::from_fn(|i| {
            let blur = i as u32 & crate::screen_effects::SAMPLE_BLUR != 0;
            let (entry, blend) = match (i & 4 == 0, blur) {
                (true, false) => ("fs_screen", alpha_blend),
                (true, true) => ("fs_screen_blur", alpha_blend),
                (false, false) => ("fs_vignette", vignette_blend),
                (false, true) => ("fs_vignette_blur", vignette_blend),
            };
            let clamp = i as u32 & crate::screen_effects::SAMPLE_CLAMP != 0;
            let constants = [("CLAMP", f64::from(u8::from(clamp)))];
            let compilation_options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            kept.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Screen effects"),
                layout: Some(&entity_layout),
                vertex: wgpu::VertexState {
                    module: &screen_shader,
                    entry_point: Some("vs_screen"),
                    compilation_options: compilation_options.clone(),
                    buffers: &screen_vertex,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &screen_shader,
                    entry_point: Some(entry),
                    compilation_options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format.remove_srgb_suffix(),
                        blend: Some(blend),
                        // 8 and up: colour only, for a tint of the world in HDR output.
                        write_mask: if i & 8 == 0 {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::COLOR
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let screen_capacity = 64;
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screen effect vertices"),
            size: screen_capacity
                * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let celestial_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Celestial vertices"),
            size: 2048 * 4 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_capacity = 1 << 14;
        let entity_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Entity vertices"),
            size: entity_capacity * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flat_wanted = flat::wanted(&adapter, &device);
        // Metal draws solid terrain in two passes (`split_solid`): meshes put all
        // their opaque quads first, so each pass draws its part in a run or two.
        crate::mesher::group_parts(adapter.get_info().backend == wgpu::Backend::Metal);
        // Readable as storage where flat draws run: a shader pack's terrain program reads them.
        let arena = arena::Arena::new(
            &device,
            std::mem::size_of::<TerrainVertex>() as u64,
            1 << 20,
            flat_wanted,
        );
        // Sections as one index list per frame where multi-draw is emulated.
        let flat = flat_wanted.then(|| {
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &TERRAIN_ATTRIBUTES,
            })];
            let info = adapter.get_info();
            let min_sections = if flat::is_broken_intel_vulkan(&info)
                || std::env::var("HORIZON_FLAT_DRAWS").as_deref() == Ok("1")
            {
                0
            } else if flat::for_vanilla(&info) {
                flat::MIN_SECTIONS
            } else {
                // Only a shader pack's terrain and shadow pass draw flat here.
                usize::MAX
            };
            flat::Flat::new(
                &device,
                &bindings,
                arena.page_vertices(),
                min_sections,
                |layout, label, vertex, fragment, blend, cull| {
                    pipeline_in(
                        layout, label, vertex, fragment, &buffers, tri, cull, true, true, blend,
                    )
                },
            )
        });
        let multi_draw = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
        let batchable = translucent_batchable(
            &adapter.get_info(),
            multi_draw,
            flat.as_ref().map(|f| f.min_sections),
        );
        crate::graphics::offer_translucent_batching(batchable);
        let translucent_batching = batchable && translucent_batching;
        // A new renderer runs no shader pack: the game loads the picked one on its
        // first world frame (`shader_pack`: the picked pack draws flat, a `shaders/`
        // pack or one of Horizon's own with `[shadow]`), and the frames before it, the
        // level loading screen's among them, keep the layout the sections upload in
        // (`draw_frame`). It starts as that pack's, a `shaders/` pack taken to draw
        // terrain: the pack loading keeps it, and only one whose terrain and shadow
        // pass draw none meshes the sections again.
        let (sorted_indices, translucent_arena) = arena::Arena::translucent_arenas(
            &device,
            std::mem::size_of::<TerrainVertex>() as u64,
            flat.is_some(),
            translucent_split(translucent_batching, flat.is_some(), shader_pack, false),
        );
        // Without indirect draws (OpenGL before 4.0 without indirect execution, or
        // before 4.2 + GL_ARB_shader_draw_parameters without indirect first instance)
        // even creating an indirect buffer fails; one draw per section reads
        // `draws` on the CPU instead.
        let indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section draws"),
            size: 1024 * 20,
            usage: if multi_draw {
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST
            } else {
                wgpu::BufferUsages::COPY_DST
            },
            mapped_at_creation: false,
        });
        // HORIZON_GPU_CULL=0 turns occlusion culling off, `all` culls every
        // frame however few its draws and little it hides, `shader` tests in
        // the shader as on Metal (A/B checks).
        let cull_env = std::env::var("HORIZON_GPU_CULL");
        let adapter_info = adapter.get_info();
        let cull = cull::Cull::supported(&adapter, &device)
            .filter(|_| gpu_culling_requested(&adapter_info, cull_env.as_deref().ok()))
            .map(|depth_test| match cull_env.as_deref() {
                Ok("shader") => cull::DepthTest::InShader,
                _ => depth_test,
            })
            .map(|depth_test| {
                let mut cull =
                    cull::Cull::new(&device, config.format.remove_srgb_suffix(), depth_test);
                cull.always = cull_env.as_deref() == Ok("all");
                if !cull.always && adapter_info.backend == wgpu::Backend::Metal {
                    cull.min_sections = cull::TILED_MIN_SECTIONS;
                }
                cull
            });
        let depth = Self::depth(&device, &config);
        let world_post = world_post::Post::new(&device, config.format.remove_srgb_suffix());
        let uploads = upload::Uploads::new(&device);
        let section_uploads = upload::Uploads::with_chunk(&device, SECTION_UPLOAD_CHUNK);
        let stars = stars::Stars::new(&device, config.format.remove_srgb_suffix());
        let entity_outline = entity_outline::EntityOutline::new(
            &device,
            (&entity_shader, &entity_layout, &entity_vertex),
            config.format.remove_srgb_suffix(),
        );
        let panorama =
            panorama::Panorama::new(&device, config.format.remove_srgb_suffix(), panorama_faces);
        drop(pipelines_step);
        reuse.finish(generation);
        if previous.is_some() {
            log::info!(
                "Horizon renderer: rebuilt on the same device ({} shader modules, layouts and pipelines kept)",
                reuse.len()
            );
        }
        if previous.is_none() {
            startup_step(format_args!(
                "pipelines ready (multi-draw {}, occlusion culling {}, GPU timer {})",
                on_off(multi_draw),
                on_off(cull.is_some()),
                on_off(gpu_timer.is_some()),
            ));
        }
        Ok(Self {
            display: DisplayWatch::default(),
            display_fresh: false,
            instance,
            adapter,
            surface: surface.map(Arc::new),
            spare_surface: std::sync::Mutex::new(None),
            offscreen,
            in_flight: std::collections::VecDeque::new(),
            paced_elsewhere: false,
            first_frame: if previous.is_none() {
                FirstFrame::Acquire
            } else {
                FirstFrame::Drawn
            },
            gpu_timer,
            gpu_times: Vec::new(),
            lightmap_pixels: [0; 16 * 16 * 4],
            last_wait: std::time::Duration::ZERO,
            last_encode: std::time::Duration::ZERO,
            window,
            device,
            reuse,
            queue: submit::Queue::new(queue, submit::wanted()),
            deferred: None,
            device_lost,
            config,
            solid,
            solid_opaque,
            solid_cutout,
            translucent,
            flat_on: flat.is_some(),
            cull_on: cull.is_some(),
            sides_on: true,
            cull,
            flat,
            sky,
            dark_disc,
            depth_reset,
            lines,
            lines_secondary,
            debug_lines,
            debug_lines_on_top,
            debug_line_buffer,
            debug_line_capacity: 4096,
            crosshair_group,
            crosshair_globals,
            solid_mirror,
            mirror_globals,
            mirror_group,
            mirror: None,
            entity_pipeline,
            hdr_mark_pipeline,
            hdr_hand_unmark_pipeline,
            hdr_mark_lightning_pipeline,
            weather,
            crumbling_pipeline,
            shadow_blob_pipeline,
            translucent_item_pipeline,
            water_mask_pipeline,
            cloud_pipeline,
            cloud_mesh: Default::default(),
            cloud_buffer,
            cloud_capacity: 4,
            cloud_uniform,
            cloud_group,
            additive_pipeline,
            dragon_rays_pipeline,
            energy_swirl_pipeline,
            lightning_pipeline,
            lightning: lightning::Renderer::new(),
            see_through_pipeline,
            text_polygon_offset_pipeline,
            celestial_pipeline,
            celestial_buffer,
            end_sky: end_sky::EndSky::default(),
            stars,
            entity_outline,
            panorama,
            border_pipeline,
            border_group,
            border_uniform,
            border_vertices,
            screen_pipelines,
            screen_buffer,
            screen_capacity,
            entity_group,
            entity_bindings,
            hand_group,
            hand_globals,
            entity_buffer,
            entity_capacity,
            entity_batch: EntityBatch::default(),
            group,
            globals,
            pbr_maps,
            lightmap,
            sprite_textures,
            sprite_unused,
            mip_levels,
            mipmaps: mip_levels - 1,
            terrain_layout: bindings,
            terrain_sprites: sprite_constants.map(|(_, v)| v),
            sprite_split: split,
            sprite_sampler,
            texture_filtering: 0,
            max_anisotropy: 1,
            lightmap_view,
            lightmap_sampler,
            animations,
            active_animated_layers: vec![0; active_layer_words],
            uploads,
            section_uploads,
            section_copies: None,
            section_copy_bytes: 0,
            shrink_gate: ShrinkGate::default(),
            lightmap_uploaded: false,
            sprite_minimum_size,
            indices,
            index_quads,
            instances,
            instance_capacity,
            outline,
            depth,
            sections: resident::Resident::default(),
            lods: HashMap::new(),
            lod_cutout: Default::default(),
            lod_retiring: Default::default(),
            lod_leaving: Vec::new(),
            lod_pending: None,
            detail_columns: Default::default(),
            detail_revision: 0,
            arena,
            translucent_arena,
            translucent_batching,
            sections_dropped: false,
            sorted_indices,
            graveyard: Default::default(),
            translucent_sorter: translucency::Sorter::default(),
            translucency_cursor: 0,
            indirect,
            indirect_capacity: 1024,
            multi_draw,
            visible: Vec::new(),
            visible_lods: Vec::new(),
            shadow_visible: Vec::new(),
            translucent_draws: Vec::new(),
            origins: Vec::new(),
            draws: Vec::new(),
            draw_pages: Vec::new(),
            phase2_draws: Vec::new(),
            phase2_pages: Vec::new(),
            solid_runs: page_runs::PageRuns::default(),
            phase2_runs: page_runs::PageRuns::default(),
            shadow_runs: page_runs::PageRuns::default(),
            block_entities: Default::default(),
            section_uploaded: Default::default(),
            newest_upload: None,
            newest_lod_fade: None,
            block_entity_revision: next_block_entity_revision(),
            light_blocks: Default::default(),
            light_revision: 0,
            occlusion: occlusion::Occlusion::new(crate::chunk::MIN_Y, crate::chunk::HEIGHT / 16),
            layout: crate::chunk::ColumnLayout::VANILLA,
            world_post,
            frame_gen: frame_gen::FrameGen::new(),
            entity_atlas: atlas_view.clone(),
            hdr: None,
            emission_stats,
            hdr_output,
            hdr_marker: None,
            pack_post: None,
            native: None,
            pass_timing: pack_timing::PassTiming::logging(),
            own_timing: None,
            pack_entity_rest: Vec::new(),
            cloud_bindings,
            pack_weather: None,
            pack_atlas: None,
            pack_terrain_flat: false,
            hud,
            menu_blur: None,
            post_effects: None,
            readback: readback::Readback::default(),
            info,
            suspended: !drawable_size(size.width, size.height, max_side),
            acquire_validation_failures: 0,
            surface_unconfigured: false,
            drawn_sections: 0,
            section_draws: 0,
            section_draw_calls: 0,
            uploaded_bytes: 0,
            started: Instant::now(),
            screenshot: None,
            screenshot_result: None,
            screenshot_quality: 1,
            screenshot_halvings: 0,
            gamma: 0.5,
            improved_transparency: false,
            oit: None,
            fullbright: false,
            animated_textures: true,
            present_modes,
        })
    }
    /// The first-person hand's quads, which follow `first` quads of the entity buffer.
    fn draw_hand(&self, pass: &mut wgpu::RenderPass<'_>, quads: u32, first: u32) {
        pass.set_pipeline(&self.entity_pipeline);
        pass.set_bind_group(0, &self.hand_group, &[]);
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
        pass.draw_indexed(0..quads * 6, (first * 4) as i32, 0..1);
        if self.hdr.is_some() {
            // Drawn into the world: the light sources it holds are marked as the world's are.
            pass.set_pipeline(&self.hdr_mark_pipeline);
            pass.draw_indexed(0..quads * 6, (first * 4) as i32, 0..1);
        }
    }
    /// LevelRenderer.submitBlockOutline: the buffer holds `edges` per stroke, the black
    /// secondary stroke first when high contrast is on.
    fn draw_outline(&self, pass: &mut wgpu::RenderPass<'_>, edges: u32, high_contrast: bool) {
        pass.set_vertex_buffer(0, self.outline.slice(..));
        if high_contrast {
            pass.set_pipeline(&self.lines_secondary);
            pass.draw(0..4, 0..edges);
        }
        pass.set_pipeline(&self.lines);
        let first = if high_contrast { edges } else { 0 };
        pass.draw(0..4, first..first + edges);
    }
    fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
        let indices: Vec<u32> = (0..quads)
            .flat_map(|q| {
                let b = q * 4;
                [b, b + 1, b + 2, b, b + 2, b + 3]
            })
            .collect();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
    fn offscreen_target(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen frame"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &config.view_formats,
        })
    }
    /// Hand the window surface over to the renderer that replaces this one on
    /// the same device (it takes `spare_surface`).
    pub fn release_surface(&mut self) {
        self.frame_gen.stop();
        self.queue.wait();
        *self
            .spare_surface
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            self.surface.take().and_then(|s| Arc::try_unwrap(s).ok());
    }
    /// Reattach a window after surface loss or a failed renderer replacement.
    pub fn recreate_surface(&mut self) -> Result<()> {
        self.queue.wait();
        if self.offscreen.is_some() {
            return Ok(());
        }
        // The surface set aside for a replacement that failed, else a new one
        // (Wayland allows only one tearing-control object per window surface,
        // so the old one goes first).
        let spare = self
            .spare_surface
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.frame_gen.stop();
        self.surface = None;
        let surface = match spare {
            Some(surface) => surface,
            None => self.instance.create_surface(self.window.clone())?,
        };
        let configured = Self::configure(
            &self.device,
            &surface,
            &self.surface_config(),
            self.hdr.as_ref(),
            &self.device_lost,
        );
        if let Err(e) = &configured {
            log::warn!(
                "Horizon renderer: the new window surface did not configure ({e}); retrying"
            );
        }
        self.surface_unconfigured = configured.is_err();
        self.surface = Some(Arc::new(surface));
        Ok(())
    }
    /// The world's depth, which occlusion culling's test and post effects may read.
    fn depth(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon depth"),
                size: wgpu::Extent3d {
                    width: config.width,
                    height: config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                // TEXTURE_BINDING: a post effect may read the depth buffer.
                // COPY_SRC: improved transparency and a native shader pack's
                // `opaque_depth` copy it.
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    /// VSync on presents with Fifo, capped to the display's refresh rate in a
    /// window and in fullscreen alike; Horizon's present-mode setting applies
    /// only with VSync off (`window::chosen_present_mode`).
    pub fn set_present_mode(&mut self, vsync: bool, mode: crate::graphics::PresentMode) {
        use crate::graphics::PresentMode as P;
        use crate::window::{PresentMode, chosen_present_mode};
        let supported: Vec<PresentMode> = self
            .present_modes
            .iter()
            .filter_map(|m| match m {
                wgpu::PresentMode::Fifo => Some(PresentMode::Fifo),
                wgpu::PresentMode::Mailbox => Some(PresentMode::Mailbox),
                wgpu::PresentMode::Immediate => Some(PresentMode::Immediate),
                _ => None,
            })
            .collect();
        let setting = match mode {
            P::Auto => None,
            P::Fifo => Some(PresentMode::Fifo),
            P::Mailbox => Some(PresentMode::Mailbox),
            P::Immediate => Some(PresentMode::Immediate),
        };
        let mode = match chosen_present_mode(&supported, vsync, setting) {
            Some(PresentMode::Mailbox) => wgpu::PresentMode::Mailbox,
            Some(PresentMode::Immediate) => wgpu::PresentMode::Immediate,
            _ => wgpu::PresentMode::Fifo,
        };
        if self.config.present_mode != mode && self.surface.is_some() {
            self.config.present_mode = mode;
            self.configure_surface();
        }
    }
    /// Follows the window to `width` x `height`. A size the swapchain can't take
    /// ([`drawable_size`]) keeps the last one and draws nothing until the next.
    pub fn resize(&mut self, width: u32, height: u32) {
        let max = self.device.limits().max_texture_dimension_2d;
        self.suspended = !drawable_size(width, height, max);
        if self.suspended {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if self.surface.is_some() {
            self.configure_surface();
        } else {
            self.offscreen = Some(Self::offscreen_target(&self.device, &self.config));
        }
        self.depth = Self::depth(&self.device, &self.config);
    }
    /// Whether sections are drawn as one index list per frame (`flat`).
    pub fn flat_draws(&self) -> bool {
        self.flat_on
    }
    /// Switches between flat and per-section draws, for A/B checks.
    pub fn set_flat_draws(&mut self, on: bool) -> Result<()> {
        if on && self.flat.is_none() {
            return Err("flat section draws are off on this device".into());
        }
        self.flat_on = on;
        Ok(())
    }
    /// Whether one of Horizon's own packs reads the water mirror, which needs the frame's
    /// `water_plane`.
    pub fn wants_water_plane(&self) -> bool {
        self.native.as_ref().is_some_and(|n| n.frame.reflects)
    }
    /// Whether solid quads facing away from the eye are left out.
    pub fn section_sides(&self) -> bool {
        self.sides_on
    }
    /// Switches leaving out solid quads facing away, for A/B checks.
    pub fn set_section_sides(&mut self, on: bool) {
        self.sides_on = on;
    }
    /// Whether solid sections are occlusion culled (`cull`).
    pub fn section_cull(&self) -> bool {
        self.cull_on
    }
    /// Switches occlusion culling, for A/B checks.
    pub fn set_section_cull(&mut self, on: bool) -> Result<()> {
        if on && self.cull.is_none() {
            return Err("occlusion culling is off on this device".into());
        }
        self.cull_on = on;
        Ok(())
    }
    /// Of the boxes occlusion culling's last test read back drew, how many
    /// it did not see and how many it drew, and the share of solid indices
    /// it hid.
    pub fn culled_sections(&self) -> Option<([usize; 2], f32)> {
        self.cull
            .as_ref()
            .filter(|_| self.cull_on)
            .map(|c| c.hidden)
    }
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }
    /// The running native pack's `[[particle_lights]]`.
    pub fn particle_lights(&self) -> Option<&[horizon_shader_api::manifest::ParticleLight]> {
        self.native.as_ref().map(|n| n.particle_lights.as_slice())
    }
    /// Whether one of Horizon's own packs with `[shadow]` draws, whose shadow pass takes the
    /// frame's `shadow_only` quads.
    pub fn native_shadow(&self) -> bool {
        self.native.as_ref().is_some_and(|n| n.shadow.is_some())
    }
    /// What a shader pack's shadow pass draws of the entities this frame: (entities or block
    /// entities other than the player, so those Horizon's culling hides from the camera are
    /// kept for it; the local player, so its model is built in first person too). (false,
    /// false) without a pack or its shadow pass.
    pub fn pack_shadow_casters(&self) -> (bool, bool) {
        let Some((_, Some(post))) = &self.pack_post else {
            return (false, false);
        };
        let c = post.shadow_casters();
        ((c.entities && !c.only_player) || c.blocks, c.entities)
    }

    pub fn has_section(&self, key: SectionKey) -> bool {
        self.sections.contains(key)
    }
    /// Bring the cave culling graph up to date for `camera` without waiting:
    /// a search this starts runs on its worker while the frame is prepared,
    /// and `render` takes it if it has finished.
    pub fn start_occlusion(&mut self, camera: DVec3, distance: f32) {
        self.occlusion.update(camera, (distance / 16.0) as i32 + 2);
    }
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        self.occlusion.set(key, mesh.visibility);
        let changed = if mesh.block_entities.is_empty() {
            self.block_entities.remove(&key).is_some()
        } else {
            let old = self.block_entities.insert(key, mesh.block_entities.clone());
            old.as_deref() != Some(&mesh.block_entities[..])
        };
        if changed {
            self.block_entity_revision = next_block_entity_revision();
        }
        let lights_changed = if mesh.lights.is_empty() {
            self.light_blocks.remove(&key).is_some()
        } else {
            let old = self.light_blocks.insert(key, mesh.lights.clone());
            old.as_deref() != Some(&mesh.lights[..])
        };
        if lights_changed {
            self.light_revision += 1;
        }
        if !self.lod_retiring.is_empty() {
            self.lod_retiring.remove(&key);
        }
        if !mesh.translucent.is_empty() && !self.sections.contains(key) {
            // New translucent terrain, not an edit's remesh: large pages wait for it.
            self.shrink_gate.wait(Instant::now(), SHRINK_SETTLE);
        }
        let fading = match self.sections.find(key) {
            Some(old) => old.fading,
            None if !self.lods.is_empty() => {
                let now = std::time::Instant::now();
                self.newest_lod_fade = Some(now);
                Some(now)
            }
            None => None,
        };
        if fading.is_none() && !self.section_uploaded.contains_key(&key) {
            let now = std::time::Instant::now();
            self.section_uploaded.insert(key, now);
            self.newest_upload = Some(now);
        }
        self.release_section(key);
        let section = self
            .upload_mesh(mesh)
            .map(|section| SectionGpu { fading, ..section });
        self.set_detail(
            key,
            section.is_some() || (mesh.solid.is_empty() && mesh.translucent.is_empty()),
        );
        if let Some(section) = section {
            self.sections.insert_with_bounds(key, section, mesh.bounds);
        }
    }
    fn upload_mesh(&mut self, mesh: &SectionMesh) -> Option<SectionGpu> {
        let solid = (mesh.solid.len() / 4) as u32;
        let translucent = (mesh.translucent.len() / 4) as u32;
        if solid + translucent == 0 {
            return None;
        }
        let max = solid.max(translucent);
        if max > self.index_quads {
            self.index_quads = max.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        // The solid quads, and the translucent ones after them unless they have
        // an arena of their own. With video and system memory both full, the
        // section is not drawn.
        let split = self.translucent_arena.is_some();
        let main = if split {
            mesh.solid.len()
        } else {
            mesh.solid.len() + mesh.translucent.len()
        } as u64;
        let (page, start) = if main > 0 {
            self.arena.alloc(&self.device, encoder, main)?
        } else {
            (0, 0)
        };
        let release_main = |arena: &mut arena::Arena| {
            if main > 0 {
                arena.release(page, start, main);
            }
        };
        let translucent_vertices = mesh.translucent.len() as u64;
        let translucent_at = match &mut self.translucent_arena {
            Some(arena) if translucent > 0 => {
                match arena.alloc(&self.device, encoder, translucent_vertices) {
                    Some(at) => at,
                    None => {
                        release_main(&mut self.arena);
                        return None;
                    }
                }
            }
            _ => (page, start + u64::from(solid) * 4),
        };
        let sorted_indices = if translucent > 0 {
            let indices =
                self.sorted_indices
                    .alloc(&self.device, encoder, u64::from(translucent) * 6);
            if indices.is_none() {
                release_main(&mut self.arena);
                if let Some(arena) = &mut self.translucent_arena {
                    arena.release(translucent_at.0, translucent_at.1, translucent_vertices);
                }
                return None;
            }
            indices
        } else {
            None
        };
        // Flat draws name the blocks of its range in `arena`: split, its solid
        // quads alone (no frame draws flat then: `translucent_split`).
        let flat_id = self.flat.as_mut().filter(|_| main > 0).map(|f| {
            f.add(
                &self.device,
                encoder,
                &mut self.section_uploads,
                page,
                start,
                main,
            )
        });
        let stride = std::mem::size_of::<TerrainVertex>() as u64;
        let translucent_arena = self.translucent_arena.as_ref().unwrap_or(&self.arena);
        for (part, arena, (part_page, first)) in [
            (&mesh.solid, &self.arena, (page, start)),
            (&mesh.translucent, translucent_arena, translucent_at),
        ] {
            if !part.is_empty() {
                let bytes: &[u8] = bytemuck::cast_slice(part);
                self.uploaded_bytes += bytes.len() as u64;
                self.section_copy_bytes += bytes.len() as u64;
                self.section_uploads.buffer(
                    encoder,
                    arena.buffer(part_page),
                    first * stride,
                    bytes,
                );
            }
        }
        if self.section_copy_bytes >= SECTION_COPY_LIMIT
            && let Some(copies) = self.take_section_copies()
        {
            self.queue.submit([copies]);
        }
        let animated_layers =
            collect_animated_layers(mesh, |layer| self.animations.is_animated(layer));
        Some(SectionGpu {
            page,
            start,
            translucent_at,
            solid,
            sides: if mesh.sides.quads() == solid {
                mesh.sides
            } else {
                Sides::unsorted(solid)
            },
            translucent,
            sort: translucency::SortState::new(
                mesh.translucent
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| translucency::quad_center(q[0].pos, q[2].pos))
                    .collect(),
            ),
            sorted_indices,
            flat_id,
            fading: None,
            leaving: None,
            animated_layers,
        })
    }
    /// The section copies recorded since the last submission, closed.
    fn take_section_copies(&mut self) -> Option<wgpu::CommandBuffer> {
        let encoder = self.section_copies.take()?;
        self.section_copy_bytes = 0;
        self.section_uploads.finish(&encoder);
        Some(encoder.finish())
    }
    /// Draws `count` section draws from `first` with the arenas and section
    /// origins bound: one indirect multi-draw, or a loop without it.
    /// Consecutive draws in one arena page share a vertex buffer binding,
    /// keeping the list's order (translucent sections go back to front).
    /// `indirect` holds the list's draws from `base`. Leaves the quad
    /// indices bound, as the draws after it expect.
    fn draw_sections(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indirect: &wgpu::Buffer,
        base: u32,
        first: u32,
        count: u32,
    ) {
        let (draws, pages) = (&self.draws[..], &self.draw_pages[..]);
        let (mut i, end) = (first as usize, (first + count) as usize);
        while i < end {
            let (page, index_page) = pages[i];
            let run = pages[i..end]
                .iter()
                .take_while(|&&p| p == (page, index_page))
                .count();
            // Translucent draws, and only they, have sorted indices; split, their large
            // pages are bound up to their last range in use (`Arena::binding`).
            let (vertices, indices) = match (&self.translucent_arena, index_page) {
                (Some(arena), Some(p)) => (arena.binding(page), self.sorted_indices.binding(p)),
                (_, p) => (
                    self.arena.buffer(page).slice(..),
                    p.map_or(&self.indices, |p| self.sorted_indices.buffer(p))
                        .slice(..),
                ),
            };
            pass.set_vertex_buffer(0, vertices);
            pass.set_index_buffer(indices, wgpu::IndexFormat::Uint32);
            if self.multi_draw {
                let offset = (i - base as usize) as u64 * 20;
                pass.multi_draw_indexed_indirect(indirect, offset, run as u32);
            } else {
                for d in &draws[i..i + run] {
                    pass.draw_indexed(d[2]..d[2] + d[0], d[3] as i32, d[4]..d[4] + 1);
                }
            }
            i += run;
        }
        if pages[first as usize..end]
            .iter()
            .any(|(_, sorted)| sorted.is_some())
        {
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        }
    }
    fn release_section(&mut self, key: SectionKey) {
        if let Some(s) = self.sections.remove(key) {
            self.release_mesh(s);
        }
    }
    fn release_mesh(&mut self, s: SectionGpu) {
        if let (Some(flat), Some(id)) = (&mut self.flat, s.flat_id) {
            flat.remove(id);
        }
        if let Some((page, start)) = s.sorted_indices {
            self.sorted_indices
                .release(page, start, u64::from(s.translucent) * 6);
        }
        let (main, translucent) = s.vertex_ranges(self.translucent_arena.is_some());
        if let Some((page, start, vertices)) = main {
            self.arena.release(page, start, vertices);
        }
        if let (Some(arena), Some((page, start, vertices))) =
            (&mut self.translucent_arena, translucent)
        {
            arena.release(page, start, vertices);
        }
    }
    /// Video memory the arena keeps because fragmentation holds its pages
    /// partly used (a render distance that went down, far tiles that were
    /// replaced): moves the meshes off the sparsest page, a few MiB a frame,
    /// and `Arena::trim` returns the page once it is empty. Frames with the
    /// arena in system memory (video memory full) are slow, so this is what
    /// keeps the terrain in video memory. The section arena's: the large pages
    /// of the translucent arena and the sorted indices are `shrink_translucent`'s.
    /// Returns what is left of the frame's `COMPACT_BUDGET`.
    fn compact_arena(&mut self) -> u64 {
        let mut budget = COMPACT_BUDGET;
        if self.arena.closing().is_none()
            && let Some(page) = self.arena.sparsest()
        {
            self.arena.close(page);
        }
        let Some(page) = self.arena.closing() else {
            return budget;
        };
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        let meshes = self
            .sections
            .values_mut()
            .chain(self.lods.values_mut().flat_map(|tile| tile.meshes_mut()));
        let flat = self
            .flat
            .as_mut()
            .map(|f| (f, &self.device, &mut self.section_uploads));
        let split = self.translucent_arena.is_some();
        let (arena, part) = (&mut self.arena, Part::Section { split });
        if !move_off_page(arena, encoder, flat, meshes, page, part, &mut budget) {
            // The other pages have no room after all.
            arena.reopen();
        }
        // With every mesh moved off, the page stays closed so this frame's
        // uploads can't refill it: `trim` returns it and clears `closing`.
        budget
    }
    /// Shrinks the large pages of the translucent arena and the sorted indices
    /// once terrain has settled (`set_terrain_loading`): one far larger than what
    /// it holds moves into a new page of their size (`begin_shrink`), through
    /// `move_off_page` with what `compact_arena` left of the frame's budget
    /// (`left`), and `trim` returns it once empty. One page at a time, vertices
    /// first; only starts wait, a move under way finishes even if terrain comes
    /// again (stopped, the page would stay closed while the new one fills).
    fn shrink_translucent(&mut self, left: u64) {
        let Some(vertices) = self.translucent_arena.as_mut() else {
            return; // split off: nothing to shrink
        };
        let now = Instant::now();
        let mut arenas = [
            (vertices, Part::Translucent),
            (&mut self.sorted_indices, Part::Indices),
        ];
        if arenas.iter().all(|(arena, _)| arena.closing().is_none()) {
            if !self.shrink_gate.may_start(now, left) {
                return;
            }
            // The next start in a second, or `SHRINK_RETRY` after a try found no room.
            let mut wait = SHRINK_SETTLE;
            for (arena, part) in &mut arenas {
                let meshes = self
                    .sections
                    .values_mut()
                    .chain(self.lods.values_mut().flat_map(|tile| tile.meshes_mut()));
                match begin_shrink(arena, &self.device, meshes, *part) {
                    Some(true) => break,
                    Some(false) => wait = SHRINK_RETRY, // the other arena may still go
                    None => {}
                }
            }
            self.shrink_gate.wait(now, wait);
        }
        let Some((arena, part, page)) = arenas.into_iter().find_map(|(arena, part)| {
            let page = arena.closing()?;
            Some((arena, part, page))
        }) else {
            return;
        };
        let encoder = self.section_copies.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Section uploads"),
                })
        });
        let meshes = self
            .sections
            .values_mut()
            .chain(self.lods.values_mut().flat_map(|tile| tile.meshes_mut()));
        // As many quads a frame either way: a quad's indices are a quarter of its bytes.
        let cap = match part {
            Part::Indices => COMPACT_BUDGET / 4,
            _ => COMPACT_BUDGET,
        };
        let mut budget = left.min(cap);
        if !move_off_page(arena, encoder, None, meshes, page, part, &mut budget) {
            // New ranges took the room: both pages stay, drawn, until a later try.
            arena.reopen();
            self.shrink_gate.wait(now, SHRINK_RETRY);
        }
    }
    /// Block-entity blocks of every meshed section (see `SectionMesh::block_entities`).
    pub fn block_entities(&self) -> impl Iterator<Item = &([i32; 3], u32)> {
        self.block_entities.values().flatten()
    }
    /// `block_entities` of the sections far enough into their fade-in to draw them
    /// (`LevelExtractor.extractVisibleBlockEntities`: `getVisibility(now, fade) >= 0.3`).
    pub fn visible_block_entities(
        &self,
        fade_seconds: f32,
    ) -> impl Iterator<Item = &([i32; 3], u32)> {
        let fade = fade_millis(fade_seconds);
        let now = std::time::Instant::now();
        let all = !self.sections_fading_in(fade, now);
        self.block_entities
            .iter()
            .filter(move |(key, _)| {
                all || chunk_visibility(self.section_elapsed_ms(**key, now), fade)
                    >= CHUNK_VISIBILITY_THRESHOLD
            })
            .flat_map(|(_, blocks)| blocks)
    }
    /// Whether a section may still be fading in over `fade_ms` (vanilla's chunk fade): one was
    /// uploaded more recently than that. Otherwise every section is whole.
    fn sections_fading_in(&self, fade_ms: u64, now: std::time::Instant) -> bool {
        fade_ms > 0
            && self
                .newest_upload
                .is_some_and(|t| (now.saturating_duration_since(t).as_millis() as u64) < fade_ms)
    }
    /// Whether a section may be fading in or out over far terrain (`SectionGpu::fade`):
    /// otherwise every section's code is 0.
    fn sections_fading_over_lods(&self, now: std::time::Instant) -> bool {
        !self.lod_retiring.is_empty()
            || !self.lod_leaving.is_empty()
            || self
                .newest_lod_fade
                .is_some_and(|t| now.saturating_duration_since(t) < lod::FADE)
    }
    /// Milliseconds since `key` was uploaded; a section not in `section_uploaded` is old.
    fn section_elapsed_ms(&self, key: SectionKey, now: std::time::Instant) -> u64 {
        self.section_uploaded.get(&key).map_or(u64::MAX, |t| {
            now.saturating_duration_since(*t).as_millis() as u64
        })
    }
    /// A value that changes whenever `block_entities` gives something else.
    pub fn block_entity_revision(&self) -> u64 {
        self.block_entity_revision
    }
    /// Drops every section, for a level whose chunks hold `layout`.
    pub fn clear_sections(&mut self, layout: crate::chunk::ColumnLayout) {
        self.block_entities.clear();
        self.section_uploaded.clear();
        self.newest_upload = None;
        self.newest_lod_fade = None;
        self.block_entity_revision = next_block_entity_revision();
        self.sections.clear();
        self.lods.clear();
        self.lod_cutout.clear();
        self.lod_retiring.clear();
        self.lod_leaving.clear();
        self.lod_pending = None;
        self.detail_columns.clear();
        self.detail_revision += 1;
        // A dimension change drops every page at once: freed later, off this thread.
        self.graveyard.add(self.arena.clear());
        if let Some(arena) = &mut self.translucent_arena {
            self.graveyard.add(arena.clear());
        }
        if let Some(flat) = &mut self.flat {
            flat.clear();
        }
        self.graveyard.add(self.sorted_indices.clear());
        self.translucency_cursor = 0;
        if layout == self.layout {
            self.occlusion.clear();
        } else {
            self.layout = layout;
            self.occlusion = occlusion::Occlusion::new(layout.min_y(), layout.sections() as i32);
            self.sections.set_min_y(layout.min_y());
        }
    }
    /// Lays the sections out as `translucent_split` says for the shader pack that
    /// runs, before a world frame draws them. A change drops every section and far
    /// tile, as a new renderer has none, and changes the sorted indices' arena with
    /// them (large pages beside the translucent arena, else pages flat draws bind):
    /// the game meshes them again (`take_dropped_sections`).
    fn follow_translucent_layout(&mut self) {
        let pack = self.pack_post.as_ref().and_then(|p| p.1.as_ref());
        let split = translucent_split(
            self.translucent_batching,
            self.flat.is_some() && self.flat_on,
            pack.is_some_and(pack_post::PackPost::has_terrain),
            // One of Horizon's own packs with `[shadow]` draws from the same lists (`shadow_view`).
            pack.is_some_and(pack_post::PackPost::has_shadow) || self.native_shadow(),
        );
        if split == self.translucent_arena.is_some() {
            return;
        }
        self.sections_dropped |=
            self.sections.len() > 0 || !self.lods.is_empty() || self.lod_pending.is_some();
        self.clear_sections(self.layout);
        (self.sorted_indices, self.translucent_arena) = arena::Arena::translucent_arenas(
            &self.device,
            std::mem::size_of::<TerrainVertex>() as u64,
            self.flat.is_some(),
            split,
        );
    }
    /// Whether every section and far tile was dropped since the last call, to be
    /// laid out again (`follow_translucent_layout`): the game meshes them again,
    /// as for a new renderer.
    pub fn take_dropped_sections(&mut self) -> bool {
        std::mem::take(&mut self.sections_dropped)
    }
    /// Whether terrain is on its way (the view not all in, events, chunks or
    /// uploads still waiting, far tiles sampled): a large translucent page shrinks
    /// once that, new translucent sections (`upload`) and far tiles to upload
    /// (`upload_lods`) have been over for `SHRINK_SETTLE`, not at the low point
    /// between a view's columns leaving and the next one's arriving, after which
    /// it would grow back by whole copies.
    pub fn set_terrain_loading(&mut self, loading: bool) {
        if loading {
            self.shrink_gate.wait(Instant::now(), SHRINK_SETTLE);
        }
    }
    /// A column whose every section has its mesh (`lod_detail_columns`).
    fn complete_column(&self) -> SectionMask {
        SectionMask::below(self.layout.sections())
    }
    pub fn lod_detail_columns(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let complete = self.complete_column();
        self.detail_columns
            .iter()
            .filter_map(move |(&column, &mask)| (mask == complete).then_some(column))
    }
    /// Changes whenever `lod_detail_columns` does.
    pub fn lod_detail_revision(&self) -> u64 {
        self.detail_revision
    }
    /// Whether the section at `key` has its mesh, for `lod_detail_columns`.
    fn set_detail(&mut self, key: SectionKey, meshed: bool) {
        let column = (key.0, key.2);
        let mask = self
            .detail_columns
            .get(&column)
            .copied()
            .unwrap_or_default();
        let mut next = mask;
        if meshed {
            next.insert(key.1 as usize);
        } else {
            next.remove(key.1 as usize);
        }
        let complete = self.complete_column();
        if (mask == complete) != (next == complete) {
            self.detail_revision += 1;
        }
        if next.is_empty() {
            self.detail_columns.remove(&column);
        } else {
            self.detail_columns.insert(column, next);
        }
    }
    /// Every section of these columns goes: one lookup per column where the
    /// tables are by column, and the meshes' ranges freed in one batch. A
    /// teleport removes a few thousand columns in one frame.
    /// A section whose column far terrain now draws fades out over it, chunk by
    /// chunk as it came, and is released once it has (`release_left`): taken
    /// away at once, the column's detail dropped to the far terrain's in one
    /// frame.
    fn leave_section(&mut self, key: SectionKey) {
        let now = std::time::Instant::now();
        if let Some(section) = self.sections.find_mut(key)
            && section.leaving.is_none()
        {
            section.leaving = Some(now);
            self.lod_leaving.push((key, now));
        }
    }
    /// Releases the sections that have faded out (`leave_section`).
    fn release_left(&mut self, now: std::time::Instant) {
        let mut done = Vec::new();
        self.lod_leaving.retain(|&(key, since)| {
            let left = now.saturating_duration_since(since) >= lod::FADE;
            if left {
                done.push((key, since));
            }
            !left
        });
        for (key, since) in done {
            // Meshed again since (the player came back): it stays.
            if self
                .sections
                .find(key)
                .is_some_and(|s| s.leaving == Some(since))
            {
                self.release_section(key);
            }
        }
    }
    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        let mut released = Vec::new();
        let mut block_entities = false;
        for (x, z) in columns {
            if self.detail_columns.remove(&(x, z)) == Some(self.complete_column()) {
                self.detail_revision += 1;
            }
            self.occlusion.remove_column(x, z);
            let retiring = !self.lod_cutout.is_empty() && self.lod_cutout.contains(&(x, z));
            for sy in 0..self.layout.sections() as i32 {
                let key = (x, sy, z);
                block_entities |= self.block_entities.remove(&key).is_some();
                if self.light_blocks.remove(&key).is_some() {
                    self.light_revision += 1;
                }
                self.section_uploaded.remove(&key);
                if retiring {
                    // Distant tiles still leave its column out: drawn until they don't.
                    self.lod_retiring.insert(key);
                } else if !self.lods.is_empty() && self.sections.contains(key) {
                    // Far terrain draws the column: it fades out over it.
                    self.leave_section(key);
                } else if let Some(section) = self.sections.remove(key) {
                    released.push(section);
                }
            }
        }
        if block_entities {
            self.block_entity_revision = next_block_entity_revision();
        }
        if let Some(flat) = &mut self.flat {
            for id in released.iter().filter_map(|s| s.flat_id) {
                flat.remove(id);
            }
        }
        self.sorted_indices
            .release_all(released.iter().filter_map(|s| {
                let (page, start) = s.sorted_indices?;
                Some((page, start, u64::from(s.translucent) * 6))
            }));
        let split = self.translucent_arena.is_some();
        self.arena
            .release_all(released.iter().filter_map(|s| s.vertex_ranges(split).0));
        if let Some(arena) = &mut self.translucent_arena {
            arena.release_all(released.iter().filter_map(|s| s.vertex_ranges(true).1));
        }
    }
    /// Horizon's resolution scale (world pixels per window pixel) and FXAA.
    /// Configures `surface` and, while HDR output is on, sends its HDR metadata
    /// again: a new swapchain starts without it. Errs when `surface` was left
    /// unconfigured (a validation error, or a lost device, which wgpu reports
    /// to no error scope).
    fn configure(
        device: &wgpu::Device,
        surface: &wgpu::Surface<'_>,
        config: &wgpu::SurfaceConfiguration,
        hdr: Option<&hdr::Hdr>,
        device_lost: &std::sync::atomic::AtomicBool,
    ) -> std::result::Result<(), String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        surface.configure(device, config);
        let memory = pollster::block_on(memory.pop());
        if let Some(e) = memory.or(pollster::block_on(validation.pop())) {
            return Err(e.to_string());
        }
        if device_lost.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("the graphics device was lost".into());
        }
        if let Some(hdr) = hdr {
            horizon_hdr::set_metadata(
                device,
                surface,
                hdr.paper_white,
                hdr.declared_peak(),
                hdr.black,
            );
        }
        Ok(())
    }
    /// Configures the window surface again. One that fails is still the one
    /// wgpu configured before, which acquisition reports as a validation error.
    fn configure_surface(&mut self) {
        self.frame_gen.stop();
        // The submit thread may still present to the old swapchain.
        self.queue.wait();
        if let Some(surface) = &self.surface
            && let Err(e) = Self::configure(
                &self.device,
                surface,
                &self.surface_config(),
                self.hdr.as_ref(),
                &self.device_lost,
            )
        {
            log::warn!("Horizon renderer: the window surface did not configure ({e})");
        }
    }
    /// HDR calibration's test picture (`crate::hdr_calibration`) while it runs, else None.
    pub fn set_hdr_calibration(&mut self, calibration: Option<HdrCalibration>) {
        let Some(hdr) = &mut self.hdr else {
            return;
        };
        if hdr.calibration == calibration {
            return;
        }
        let declared = hdr.declared_peak();
        hdr.calibration = calibration;
        if hdr.declared_peak() != declared {
            self.send_hdr_metadata();
        }
    }
    fn send_hdr_metadata(&self) {
        if let (Some(surface), Some(hdr)) = (&self.surface, &self.hdr) {
            horizon_hdr::set_metadata(
                &self.device,
                surface,
                hdr.paper_white,
                hdr.declared_peak(),
                hdr.black,
            );
        }
    }
    /// The swapchain's configuration: `config` (what every pipeline draws in), or
    /// HDR10 or scRGB while HDR output is on.
    fn surface_config(&self) -> wgpu::SurfaceConfiguration {
        let mut config = self.config.clone();
        if let Some(hdr) = &self.hdr {
            config.format = hdr.output.format();
            config.view_formats = vec![hdr.output.format()];
            config.color_space = hdr.output.color_space();
            config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        config
    }
    /// The F3 screen's HDR line while the setting is on: the swapchain HDR presents to with
    /// its paper white and peak, or that the surface offers none.
    /// Whether HDR output is on: the setting, where the display offers it.
    pub fn hdr_on(&self) -> bool {
        self.hdr.is_some()
    }
    pub fn hdr_line(&self) -> String {
        match &self.hdr {
            Some(hdr) => format!(
                "HDR: {}, {}/{} nits",
                hdr.output.name(),
                hdr.paper_white.round(),
                hdr.peak.round()
            ),
            None => "HDR: not offered by this display".into(),
        }
    }
    /// HDR output (Horizon setting): HDR10 or scRGB at `paper_white` nits, the world's
    /// highlights expanded up to `peak` nits. Off where the surface can't present it.
    pub fn set_hdr(
        &mut self,
        on: bool,
        paper_white: f32,
        peak: f32,
        black: f32,
        ui: f32,
        marker: &std::path::Path,
    ) {
        let output = self.hdr_output.filter(|_| on && self.surface.is_some());
        if output.is_some() != self.hdr.is_some() {
            // If the game dies before HDR output shows a frame, the marker is still there at
            // the next start, which then comes up with HDR off.
            if output.is_some() {
                let _ = std::fs::write(marker, "HDR output was being switched on\n");
                self.hdr_marker = Some(marker.to_owned());
            } else if let Some(marker) = self.hdr_marker.take() {
                let _ = std::fs::remove_file(marker);
            }
            // Only HDR output asks the system about the display, so with HDR off the
            // game never waits on the compositor for it.
            self.display = if output.is_some() {
                let (watch, now) =
                    DisplayWatch::new(&self.window, self.surface.as_deref(), &self.adapter);
                crate::graphics::report(now);
                self.display_fresh = true;
                watch
            } else {
                DisplayWatch::default()
            };
            let format = self.config.format.remove_srgb_suffix();
            self.hdr =
                output.map(|output| hdr::Hdr::new(&self.device, &self.queue, format, output));
            self.configure_surface();
            match output {
                Some(output) => log::info!(
                    "Horizon renderer: HDR on: {} ({:?}), {paper_white}/{peak} nits",
                    output.name(),
                    output.format()
                ),
                None => log::info!("Horizon renderer: HDR off"),
            }
            self.measure_emission();
        }
        if let Some(hdr) = &mut self.hdr {
            hdr.ui = ui;
        }
        if let Some(hdr) = &mut self.hdr
            && (hdr.paper_white, hdr.peak, hdr.black) != (paper_white, peak, black)
        {
            hdr.paper_white = paper_white;
            hdr.peak = peak;
            hdr.black = black;
            self.send_hdr_metadata();
        }
    }
    /// Measures which pixels of each block sprite may shine (`hdr_stats`), at the current
    /// masks. Only with HDR output on: without it the frame's alpha is never read.
    pub fn measure_emission(&mut self) {
        if let Some(hdr) = &self.hdr {
            let views = sprite_views(&self.sprite_textures, &self.sprite_unused, None);
            self.emission_stats
                .measure(&self.device, &self.queue, &views, hdr.masks);
        }
    }
    /// Whether what the system reports for the display changed (another output, the
    /// player recalibrating the desktop): the settings' Auto values follow it then, so
    /// apply them again.
    pub fn display_changed(&mut self) -> bool {
        flush_display_log();
        if std::mem::take(&mut self.display_fresh) {
            return true;
        }
        let Some(now) = self.display.changed(self.surface.as_deref(), &self.adapter) else {
            return false;
        };
        crate::graphics::report(now);
        true
    }
    /// The share of real frames frame generation shows a made frame before (1 while it
    /// keeps up), `None` while it isn't running.
    pub fn frame_generation_share(&self) -> Option<f32> {
        self.frame_gen.made_share
    }
    /// Frame generation on or off (FSR 3.1's frame interpolation).
    pub fn set_frame_generation(&mut self, on: bool) {
        if !on {
            self.frame_gen.stop();
        }
        self.frame_gen.on = on;
    }
    /// `upscale`: output pixels per rendered pixel, for TAA to rebuild (1: native).
    pub fn set_world_post(&mut self, scale: f32, antialias: bool, taa: bool, upscale: f32) {
        let taa = taa && self.world_post.supports_taa();
        self.world_post.scale = scale.clamp(0.25, 2.0);
        self.world_post.antialias = antialias && !taa;
        self.world_post.taa = taa;
        self.world_post.upscale = if taa { upscale.clamp(1.0, 4.0) } else { 1.0 };
    }
    /// Runs `post` over the world from now on (`None`: no pack), for the dimension `dimension`;
    /// `block_ids` gives each block sprite layer's `mc_Entity` (id, fluid) pair.
    /// Returns the programs that run and the notes of loading them.
    pub fn set_shader_pack(
        &mut self,
        dimension: i32,
        pack: Option<horizon_shaders::Post>,
        block_ids: &[i32],
    ) -> Option<(Vec<String>, Vec<String>)> {
        if pack.is_none() {
            self.pack_atlas = None;
        }
        let instance = wgpu::vertex_attr_array![5=>Float32x4];
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TERRAIN_ATTRIBUTES,
        };
        let instances = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance,
        };
        let buffers = [Some(vertex), Some(instances)];
        let flat = self.pack_terrain_flat;
        let post = pack.map(|post| {
            let world = pack_post::World {
                atlas: self.pack_atlas.as_ref().map(|a| (a.view.clone(), a.size())),
                lightmap: self.lightmap_view.clone(),
                terrain: self.pack_atlas.as_ref().map(|_| pack_post::Geometry {
                    buffers: if flat { &buffers[..1] } else { &buffers },
                    table: self.flat.as_ref().filter(|_| flat).map(|f| &f.draw_layout),
                    depth_write: true,
                    atlas: None,
                    ids: None,
                    cull: true,
                }),
                block_ids,
                celestial: self
                    .hud
                    .atlas_texture
                    .as_ref()
                    .map(|t| t.create_view(&Default::default())),
                clouds: Some(&self.cloud_bindings),
                weather: true,
                hand: true,
            };
            pack_post::PackPost::new(
                &self.device,
                &self.queue,
                &self.adapter,
                post,
                self.config.format.remove_srgb_suffix(),
                world,
            )
        });
        let report = post.as_ref().map(|p| {
            let programs = p.programs().into_iter().map(str::to_owned).collect();
            (programs, p.notes.clone())
        });
        self.pack_post = Some((dimension, post));
        if self.pack_post.as_ref().is_some_and(|p| p.1.is_some()) {
            self.native = None;
        }
        report
    }
    /// Drops the running shader pack, of either kind.
    pub fn drop_shader_packs(&mut self) {
        self.pack_post = None;
        self.native = None;
        self.pack_atlas = None;
    }
    /// Runs the sky, material and post stages of one of Horizon's own shader packs from now
    /// on (`None`: no pack), with its options' values; replaces a `shaders/` pack. Returns the
    /// stages' labels; on an error the pack that ran keeps running.
    pub fn set_native_shader_pack(
        &mut self,
        pack: Option<(
            horizon_shader_api::Report,
            Vec<(String, f64)>,
            Vec<(u32, u32)>,
            Vec<(u32, u32)>,
            block_light::LightTable,
        )>,
    ) -> std::result::Result<Vec<String>, String> {
        let Some((report, constants, sprite_tags, sprite_materials, lights)) = pack else {
            self.native = None;
            return Ok(Vec::new());
        };
        let terrain = native_material::TerrainShape {
            globals: &self.terrain_layout,
            flat: self.flat.as_ref().map(|f| &f.draw_layout),
            sprites: self.terrain_sprites,
            format: self.config.format.remove_srgb_suffix(),
            sprite_tags: &sprite_tags,
            sprite_materials: &sprite_materials,
            entities: &self.entity_bindings,
            weather_groups: [&self.weather.entity_layout, &self.weather.time_layout],
            pbr: self.pbr_maps.as_ref(),
        };
        let (pack, labels) = native_pack::NativePack::new(
            &self.device,
            &self.queue,
            &terrain,
            report,
            &constants,
            lights,
        )?;
        self.native = pack.draws().then_some(pack);
        if self.native.is_some() {
            self.pack_post = None;
            self.pack_atlas = None;
        }
        Ok(labels)
    }
    /// What a shader pack's gbuffers programs read from Horizon's geometry: the sky's, and
    /// terrain's in the pack's block atlas (made here when missing; none when it does not fit).
    pub fn pack_inputs(&mut self) -> horizon_shaders::GameInputs {
        let terrain = self.pack_terrain_inputs();
        // A geometry stage reads flat draws' vertices itself, through their index buffer.
        let terrain_geometry = terrain
            .clone()
            .filter(|_| self.pack_terrain_flat)
            .map(|mut inputs| {
                inputs.declarations.push_str(
                    "layout(set = 5, binding = 0) readonly buffer HzIndices { uint hz_indices[]; };\n",
                );
                horizon_shaders::GeometryInputs {
                    inputs,
                    pull: pack_atlas::PULL.to_owned(),
                    index: "hz_indices[@]".to_owned(),
                }
            });
        horizon_shaders::GameInputs {
            terrain,
            terrain_geometry,
            sky: Some(pack_sky::inputs()),
            celestial: Some(pack_sky::celestial_inputs()),
            entities: Some(pack_entities::inputs()),
            clouds: Some(pack_entities::cloud_inputs()),
            weather: Some(pack_weather::inputs()),
            hand: Some(pack_hand::inputs()),
        }
    }

    fn pack_terrain_inputs(&mut self) -> Option<horizon_shaders::GbuffersInputs> {
        if self.pack_atlas.is_none() {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let layers = self
                .sprite_textures
                .iter()
                .map(|t| t.depth_or_array_layers())
                .sum();
            self.pack_atlas = pack_atlas::PackAtlas::new(
                &self.device,
                &mut encoder,
                &self.sprite_textures,
                self.sprite_split,
                layers,
                self.sprite_textures[0].width(),
            );
            self.queue.submit([encoder.finish()]);
        }
        // Flat draws whenever the device has them (`render` keeps them on with a pack).
        self.pack_terrain_flat = self.flat.is_some() && self.flat_on;
        Some(
            self.pack_atlas
                .as_ref()?
                .terrain_inputs(self.pack_terrain_flat),
        )
    }
    /// Times the shader pack's passes on the GPU while `on` (the F3 overlay is open), or always
    /// with `HORIZON_SHADER_PACK_TIMING`.
    pub fn set_pass_timing(&mut self, on: bool) {
        self.pass_timing = on || pack_timing::PassTiming::logging();
    }
    /// The running shader pack's timer, of either kind.
    fn pass_timer<'a>(
        pack: &'a mut Option<(i32, Option<pack_post::PackPost>)>,
        native: &'a mut Option<native_pack::NativePack>,
    ) -> Option<&'a mut Option<pack_timing::PassTiming>> {
        match (pack, native) {
            (Some((_, Some(post))), _) => Some(&mut post.timing),
            (_, Some(native)) => Some(&mut native.timing),
            _ => None,
        }
    }
    /// Each pass of the running shader pack with its median GPU time in milliseconds, while
    /// they are timed (`set_pass_timing`).
    pub fn shader_pass_times(&self) -> Vec<(String, f32)> {
        let timer = match (&self.pack_post, &self.native) {
            (Some((_, Some(post))), _) => post.timing.as_ref(),
            (_, Some(native)) => native.timing.as_ref(),
            _ => None,
        };
        timer
            .map(pack_timing::PassTiming::medians)
            .unwrap_or_default()
    }
    /// Each of Horizon's own passes with its median GPU time in milliseconds, while they are
    /// timed (`set_pass_timing`).
    pub fn own_pass_times(&self) -> Vec<(String, f32)> {
        self.own_timing
            .as_ref()
            .map(pack_timing::PassTiming::medians)
            .unwrap_or_default()
    }
    /// The macros of the GL extensions this device gives shader packs: subgroups, when it
    /// has them.
    pub fn define_pack_macros(&self, macros: &mut horizon_shaders::Macros) {
        if self.device.features().contains(wgpu::Features::SUBGROUP) {
            for ext in [
                "basic",
                "vote",
                "arithmetic",
                "ballot",
                "shuffle",
                "shuffle_relative",
            ] {
                macros.define(format!("MC_GL_KHR_shader_subgroup_{ext}"), "");
            }
        }
    }

    /// The dimension the shader pack was last set for.
    pub fn shader_pack_dimension(&self) -> Option<i32> {
        self.pack_post.as_ref().map(|p| p.0)
    }
    /// The Texture Filtering and Max Anisotropy options (`filtering`: 0 none, 1 RGSS, 2
    /// anisotropic; `bit`: `maxAnisotropyBit`). All take LevelRenderer's chunk layer sampler
    /// (linear, clamped, `bit` anisotropy when anisotropic) and terrain.wgsl's sampleNearest,
    /// or sampleRGSS for RGSS.
    pub fn set_texture_filtering(&mut self, filtering: i32, bit: i32) {
        let filtering = filtering.clamp(0, 2) as u32;
        let anisotropy = if filtering == 2 {
            sampler_anisotropy(bit)
        } else {
            1
        };
        if filtering == self.texture_filtering && anisotropy == self.max_anisotropy {
            return;
        }
        self.texture_filtering = filtering;
        self.max_anisotropy = anisotropy;
        self.sprite_sampler = self
            .device
            .create_sampler(&sprite_sampler_descriptor(anisotropy));
        self.rebuild_terrain_group();
    }
    fn rebuild_terrain_group(&mut self) {
        let views = sprite_views(
            &self.sprite_textures,
            &self.sprite_unused,
            Some(self.mipmaps + 1),
        );
        self.group = terrain_group(
            &self.device,
            "Horizon terrain resources",
            &self.terrain_layout,
            &self.globals,
            &views,
            &self.sprite_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
    }
    /// The Mipmap Levels option: block textures sample at most `levels`
    /// reduced copies (0: none).
    pub fn set_mipmap_levels(&mut self, levels: u32) {
        let levels = crate::sprites::sprite_mipmap_limit(
            levels,
            self.sprite_textures[0].width(),
            self.sprite_minimum_size,
            self.mip_levels,
        );
        if levels == self.mipmaps {
            return;
        }
        self.mipmaps = levels;
        self.rebuild_terrain_group();
    }
    /// The views `render` draws `frame` from.
    pub fn views(&self, frame: &Frame) -> ViewSet {
        ViewSet::single(self.screen_view(
            frame.fov_degrees,
            frame.yaw,
            frame.pitch,
            frame.depth_far,
        ))
    }
    /// The view frustum `render` will cull with, for culling geometry built
    /// before the frame (entities, block entities) the same way.
    pub fn camera_frustum(
        &self,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        bob: Mat4,
        depth_far: f32,
    ) -> Frustum {
        let views = ViewSet::single(self.screen_view(fov_degrees, yaw, pitch, depth_far));
        Frustum {
            planes: frustum(views.cull().view_proj(bob)),
        }
    }
    fn screen_view(&self, fov_degrees: f32, yaw: f32, pitch: f32, far: f32) -> View {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        View::screen(yaw, pitch, fov_degrees, aspect, far)
    }
    fn update_lightmap(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        sky: &Sky,
        darkness: f32,
        gamma: f32,
        night_vision: f32,
        flicker: f32,
        boss_darken: f32,
    ) {
        // lightmap.fsh: per (block, sky) level.
        let brightness = |level: f32| level / (4.0 - 3.0 * level);
        let mut pixels = [0u8; 16 * 16 * 4];
        for s in 0..16 {
            for b in 0..16 {
                let block_level = b as f32 / 15.0;
                let sky_level = s as f32 / 15.0;
                let block_brightness = brightness(block_level) * (1.4 + flicker);
                let sky_brightness = brightness(sky_level) * sky.sky_light_factor;
                // NightVisionColor defaults to 0x999999.
                let mut color =
                    Vec3::from(sky.ambient_light_color).max(Vec3::splat(0.6 * night_vision));
                color += Vec3::from(sky.sky_light_color) * sky_brightness;
                let parabolic = (2.0 * block_level - 1.0).powi(2);
                let block_color = Vec3::from(sky.block_light_tint).lerp(Vec3::ONE, 0.9 * parabolic);
                color += block_color * block_brightness;
                // lightmap.fsh: a boss bar mixes the light toward 70/60/60%.
                color = color.lerp(color * Vec3::new(0.7, 0.6, 0.6), boss_darken);
                color -= Vec3::splat(darkness);
                color = color.clamp(Vec3::ZERO, Vec3::ONE);
                let max = color.max_element().max(1e-6);
                let inverted = 1.0 - max;
                let scaled = 1.0 - inverted.powi(4);
                let not_gamma = color * (scaled / max);
                color = color.lerp(not_gamma, gamma);
                let i = (s * 16 + b) * 4;
                pixels[i] = (color.x * 255.0) as u8;
                pixels[i + 1] = (color.y * 255.0) as u8;
                pixels[i + 2] = (color.z * 255.0) as u8;
                pixels[i + 3] = 255;
            }
        }
        if self.fullbright {
            pixels.fill(255);
        }
        // The lightmap changes with the time of day, flicker and effects,
        // not every frame: upload only what differs from the texture.
        if pixels == self.lightmap_pixels && self.lightmap_uploaded {
            return;
        }
        self.uploads.texture(
            encoder,
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightmap,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            16,
            16,
            &pixels,
        );
        self.lightmap_pixels = pixels;
        self.lightmap_uploaded = true;
    }
    fn animate(&mut self, encoder: &mut wgpu::CommandEncoder, tick: u64, cull_inactive: bool) {
        let active = cull_inactive.then_some(&self.active_animated_layers[..]);
        self.animations.animate(
            encoder,
            &mut self.uploads,
            tick,
            active,
            self.own_timing.as_mut(),
        );
        if let Some(atlas) = &self.pack_atlas {
            atlas.copy(
                encoder,
                &self.sprite_textures,
                self.animations.changed().iter().copied(),
            );
        }
    }
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        // A lost device cannot accept uploads either. Leave recovery to the
        // owner instead of submitting atlas patches to the invalid device.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        let scaled = self.screenshot_size();
        let rendered = match scaled {
            Some(size) => self.draw_scaled_frame(frame, size),
            None => self.draw_frame(frame, None),
        };
        // A frame that was not drawn still submits the section copies and
        // atlas patches (glyphs, skins, maps), which the caller already took.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        if !matches!(rendered, Ok(true)) && !frame.hud.patches.is_empty() {
            let section_copies = self.section_copies.take();
            let has_section_copies = section_copies.is_some();
            let mut encoder = section_copies.unwrap_or_else(|| {
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Horizon atlas patches"),
                    })
            });
            self.hud
                .upload_patches(&mut self.uploads, &mut encoder, &frame.hud.patches);
            self.uploads.finish(&encoder);
            if has_section_copies {
                self.section_copy_bytes = 0;
                self.section_uploads.finish(&encoder);
            }
            self.queue.submit([encoder.finish()]);
        } else if let Some(copies) = self.take_section_copies() {
            self.queue.submit([copies]);
        }
        // A scaled screenshot's frame never reached the window.
        rendered.map(|drawn| drawn && scaled.is_none())
    }
    /// Whether the device's loss callback has fired. A surface reconfigure
    /// alone cannot restore resources belonging to a lost device.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The frame size of the pending `scaled` screenshot when it is larger than the
    /// window: the window's times the Resolution Scale (when above 100%) and
    /// `screenshot_quality`, cut down to what one texture and one readback buffer hold,
    /// and halved for each time it ran out of video memory.
    fn screenshot_size(&self) -> Option<[u32; 2]> {
        let request = self.screenshot.as_ref()?;
        if !request.scaled || request.size.is_some() || self.suspended {
            return None;
        }
        let window = [self.config.width, self.config.height];
        let limits = self.device.limits();
        let factor = scaled_screenshot_factor(
            window,
            self.world_post.scale,
            self.screenshot_quality,
            limits.max_texture_dimension_2d,
            limits.max_buffer_size,
        ) / 2f32.powi(self.screenshot_halvings as i32);
        (factor > 1.0).then(|| window.map(|side| (side as f32 * factor) as u32))
    }
    /// Draws one frame at `size` into a texture of its own, for the pending screenshot:
    /// every pass sizes itself by `config`, which says `size` for this frame, the HUD
    /// keeps its layout at a larger scale, and the world is drawn at `size` (a Resolution
    /// Scale below 100% stays as low). The window keeps its last frame.
    fn draw_scaled_frame(&mut self, frame: &Frame, size: [u32; 2]) -> Result<bool> {
        let window = [self.config.width, self.config.height];
        let mut config = self.config.clone();
        [config.width, config.height] = size;
        config.usage |= wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        // Out of video memory (an invalid texture's view is a validation error):
        // tried again at half the size next frame; the window keeps its last one.
        let checks = GpuChecks::push(&self.device);
        let target = Self::offscreen_target(&self.device, &config);
        let depth = Self::depth(&self.device, &config);
        if let Some(e) = checks.pop() {
            log::warn!(
                "Horizon renderer: no memory for a {}x{} screenshot ({e}); trying half the size",
                size[0],
                size[1]
            );
            self.screenshot_halvings += 1;
            return Ok(false);
        }
        let scale = self.world_post.scale;
        self.world_post.scale = scale / scale.max(1.0);
        self.hud.zoom = size[0] as f32 / window[0] as f32;
        [self.config.width, self.config.height] = size;
        let surface = self.surface.take();
        // HDR presents to the swapchain; this frame goes to a file.
        let hdr = self.hdr.take();
        let offscreen = self.offscreen.replace(target);
        let window_depth = std::mem::replace(&mut self.depth, depth);
        // The frame's own targets at `size` can run out of memory too.
        let drawn = self.draw_frame(frame, Some(GpuChecks::push(&self.device)));
        self.depth = window_depth;
        self.offscreen = offscreen;
        self.hdr = hdr;
        self.surface = surface;
        [self.config.width, self.config.height] = window;
        self.hud.zoom = 1.0;
        self.world_post.scale = scale;
        drawn
    }
    /// `checks`: the error scopes of a scaled screenshot's frame, checked before its PNG is written.
    fn draw_frame(&mut self, frame: &Frame, checks: Option<GpuChecks>) -> Result<bool> {
        // Its steps as spans (HORIZON_SPIKES): each lap names the step that just ended.
        let mut laps = crate::spans::laps();
        let landing = Instant::now();
        self.land_frame();
        // Waiting for the previous frame (offscreen: at most two on the GPU), not work.
        let landed = landing.elapsed();
        self.queue.take_waited();
        // World frames alone follow the shader pack: the game loads the picked one
        // before it draws one. Title frames draw no terrain and keep the layout
        // through a pack dropped (Shader Options) or not loaded yet (`Renderer::new`),
        // so the next world's sections, which the level loading screen uploads, stay.
        if !frame.title {
            self.follow_translucent_layout();
        }
        let released = self.arena.trim();
        self.graveyard.add(released);
        let left = self.compact_arena();
        if let Some(arena) = &mut self.translucent_arena {
            self.graveyard.add(arena.trim());
        }
        self.graveyard.add(self.sorted_indices.trim());
        arena::free_later(self.graveyard.advance());
        self.shrink_translucent(left);
        if let Some(flat) = &mut self.flat {
            flat.trim(|page| self.arena.has_page(page));
        }
        if self.suspended {
            return Ok(false);
        }
        let started = Instant::now();
        if self.first_frame == FirstFrame::Acquire {
            // Once: a minimized window acquires nothing for a while.
            startup_step(format_args!("acquiring the first frame"));
            self.first_frame = FirstFrame::Submit;
        }
        if self.surface_unconfigured
            && let Some(surface) = &self.surface
        {
            // Acquiring from it would panic: configure it again, or skip the
            // frame. A lost device never configures it.
            if self.device_lost.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("the graphics device was lost".into());
            }
            self.frame_gen.stop();
            if Self::configure(
                &self.device,
                surface,
                &self.surface_config(),
                self.hdr.as_ref(),
                &self.device_lost,
            )
            .is_err()
            {
                return Ok(false);
            }
            self.surface_unconfigured = false;
        }
        laps.lap("r_prepare");
        // Frame generation: its present thread acquires and presents, this frame draws
        // into frames of its own. Not over menus that blur the frame, a shader pack's
        // own post passes, HDR or the title screen.
        let fg = self.frame_gen.on
            && !self.frame_gen.paused
            && self.hdr.is_none()
            && !frame.title
            && frame.menu_blur.is_none()
            && self.pack_post.as_ref().is_none_or(|p| p.1.is_none())
            && self.native.as_ref().is_none_or(|n| n.post.is_none())
            && (self.surface.is_none()
                || self.config.usage.contains(wgpu::TextureUsages::COPY_DST));
        if !fg {
            self.frame_gen.stop();
        }
        // Waits for the submit thread since the last frame's present (this frame's uploads).
        let queued = self.queue.take_waited();
        let acquire = Instant::now();
        let fg_frames = fg
            .then(|| {
                self.frame_gen.frames(
                    &self.device,
                    &self.queue,
                    self.surface.as_ref(),
                    self.offscreen.as_ref(),
                    &self.window,
                    &self.config,
                )
            })
            .flatten();
        // The present thread failed: this frame acquires as without frame generation.
        let fg = fg_frames.is_some();
        let mut fg_made = false;
        let acquired = if fg {
            None
        } else {
            self.surface.as_ref().map(|s| s.get_current_texture())
        };
        self.last_wait = landed + queued + acquire.elapsed();
        // Inside the acquire's time already.
        self.queue.take_waited();
        let (surface_frame, reconfigure) = match acquired {
            None => (None, false),
            Some(wgpu::CurrentSurfaceTexture::Success(f)) => (Some(f), false),
            Some(wgpu::CurrentSurfaceTexture::Suboptimal(f)) => (Some(f), true),
            Some(wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded) => {
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Outdated) => {
                self.configure_surface();
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Lost) => {
                self.recreate_surface()?;
                return Ok(false);
            }
            Some(wgpu::CurrentSurfaceTexture::Validation) => {
                // Acquire caught a validation error: the swapchain cannot be
                // used as configured (wgpu reports a configuration raced with
                // a submission this way). Give it a fresh configuration like
                // an Outdated one, and once that has not helped, reattach the
                // window like a Lost one — a fullscreen loading screen then
                // skips frames instead of failing every one of them.
                if self.acquire_validation_failures == 0 {
                    self.configure_surface();
                } else {
                    self.recreate_surface()?;
                }
                self.acquire_validation_failures =
                    self.acquire_validation_failures.saturating_add(1);
                return Ok(false);
            }
        };
        if surface_frame.is_some() {
            self.acquire_validation_failures = 0;
            self.frame_gen.paused = false;
        }
        let size = [self.config.width, self.config.height];
        let target_texture = match (&surface_frame, &mut self.hdr) {
            _ if let Some((real, _)) = &fg_frames => real.clone(),
            (Some(_), Some(hdr)) => hdr.frame(&self.device, size),
            // A screenshot of a swapchain without COPY_SRC (OpenGL) draws
            // into a texture that has it, then onto the swapchain.
            (Some(f), None) => self
                .readback
                .target(&self.device, &f.texture, self.screenshot.is_some())
                .unwrap_or_else(|| f.texture.clone()),
            (None, _) => self.offscreen.clone().ok_or("no offscreen target")?,
        };
        // One command buffer for the frame, after the section copies staged
        // since the last one: Metal commits every command buffer wgpu-core
        // makes (one per run of copies and two per pass) on its own.
        let section_copies = self.section_copies.take();
        let has_section_copies = section_copies.is_some();
        let mut encoder = section_copies.unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Horizon frame"),
                })
        });
        // Horizon's own passes are timed like a shader pack's, from the frame's first pass.
        if self.own_timing.is_some() != self.pass_timing {
            self.own_timing = self
                .pass_timing
                .then(|| pack_timing::PassTiming::new(&self.device, &self.queue))
                .flatten();
        }
        if let Some(t) = &mut self.own_timing {
            t.begin(&self.device, &self.queue);
        }
        if self.animated_textures {
            self.hud
                .animate_sprites(&mut self.uploads, &mut encoder, frame.ticks);
        }
        self.update_lightmap(
            &mut encoder,
            &frame.sky,
            frame.darkness.1,
            (self.gamma - frame.darkness.0).max(0.0),
            frame.night_vision,
            frame.block_flicker,
            frame.boss_darken,
        );
        let views = self.views(frame);
        let pose = frame.bob * frame.spin;
        let view_proj = views.primary().view_proj(pose);
        laps.lap("r_acquire");
        // TAA runs where the world post pass does, not under a shader pack's own.
        let taa = !frame.title
            && self.pack_post.as_ref().is_none_or(|p| p.1.is_none())
            && self.native.as_ref().is_none_or(|n| n.post.is_none());
        let jittered = self.world_post.jitter(
            [self.config.width, self.config.height],
            taa,
            view_proj,
            frame.camera,
        ) * view_proj;
        if frame.title {
            self.panorama.prepare(
                &self.device,
                &self.queue,
                self.config.width,
                self.config.height,
                frame.yaw,
                frame.pitch,
            );
        } else {
            self.panorama.release();
        }
        let view = views.primary().view;
        // The hand's bob and view (`hand_globals`).
        let hand_view = frame.bob * view;
        let sun_angle = frame.sky.sun_angle.to_radians();
        // Celestial bodies rotate around the X axis; angle 0 = noon.
        let sun = Vec3::new(0.0, sun_angle.cos(), -sun_angle.sin());
        let (fog_color, fog) = if let Some(f) = frame.fog {
            (
                f.color,
                [
                    f.environmental_start,
                    f.environmental_end,
                    f.render_distance_start,
                    f.render_distance_end,
                ],
            )
        } else if frame.underwater {
            (
                [
                    frame.water_fog[0],
                    frame.water_fog[1],
                    frame.water_fog[2],
                    1.0,
                ],
                [-8.0, 96.0, -8.0, 96.0],
            )
        } else {
            let d = frame.distance;
            (
                [
                    frame.sky.fog_color[0],
                    frame.sky.fog_color[1],
                    frame.sky.fog_color[2],
                    1.0,
                ],
                // FogRenderer: render-distance fog starts at max(d - 16, 0)... d.
                {
                    let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
                    [start, end, (d - d.min(64.0) * 0.25).max(0.0), d]
                },
            )
        };
        let globals = Globals {
            view_proj: jittered.to_cols_array_2d(),
            fog_color,
            fog,
            sky_color: [
                frame.sky.sky_color[0],
                frame.sky.sky_color[1],
                frame.sky.sky_color[2],
                1.0,
            ],
            misc: [
                1.0 - frame.sky.sky_light_factor,
                frame.game_time,
                views.primary().aspect,
                views.primary().tan_half_fov,
            ],
            inverse_view: view.inverse().to_cols_array_2d(),
            sunrise: frame.sky.sunrise_color,
            sun: [sun.x, sun.y, sun.z, frame.sky.star_brightness],
            fog_ends: [
                frame.fog.map_or(frame.distance.min(512.0), |f| f.sky_end),
                frame.fog.map_or(2048.0, |f| f.cloud_end),
                if frame.dark_disc { 1.0 } else { 0.0 },
                frame.glint_alpha,
            ],
            end_portal_sprites: frame.end_portal_sprites,
            crumbling_sprite: frame.crumbling_sprite,
            time: [
                frame.shader_game_time,
                f32::from(frame.nether_lighting),
                frame.dynamic_lights.len().min(crate::dynamic_light::MAX) as f32,
                frame.glint_ticks,
            ],
            outline_params: outline_params(self.config.width, self.config.height),
            dynamic_lights: {
                let mut lights = [[0.0; 4]; crate::dynamic_light::MAX];
                for (to, from) in lights.iter_mut().zip(frame.dynamic_lights) {
                    *to = *from;
                }
                lights
            },
            colored_lights: {
                let mut lights = [[[0.0; 4]; 2]; crate::dynamic_light::MAX];
                for (to, from) in lights.iter_mut().zip(frame.colored_lights) {
                    *to = *from;
                }
                lights
            },
            colored_count: [
                frame.colored_lights.len().min(crate::dynamic_light::MAX) as u32,
                0,
                0,
                0,
            ],
            emissive_layers: {
                let mut layers = [[0; 4]; crate::native_emissive::MAX];
                for (to, from) in layers.iter_mut().zip(frame.emissive_layers) {
                    *to = *from;
                }
                layers
            },
            emissive_count: [
                frame.emissive_layers.len().min(crate::native_emissive::MAX) as u32,
                0,
                0,
                0,
            ],
            sunrise_fan: sunrise_fan::normals(frame.sky.sunrise_color[3], sun),
            filtering: [
                self.texture_filtering as f32,
                self.sprite_textures[0].width() as f32,
                0.0,
                0.0,
            ],
            hdr_windows: self.hdr.as_ref().map_or([[0.0; 4]; 2], hdr::Hdr::windows),
        };
        self.uploads
            .buffer(&mut encoder, &self.globals, 0, bytemuck::bytes_of(&globals));
        laps.lap("r_targets");
        // GameRenderer.render3dHud: the hand's projection at hudFov, with the
        // bob in its pose rather than the projection (same product).
        let hand_projection = Mat4::from_cols_array(&crate::camera::hand_projection(
            frame.hand_fov_degrees,
            self.config.width as f32,
            self.config.height.max(1) as f32,
            frame.depth_far,
        ));
        let hand_globals = Globals {
            view_proj: (hand_projection * frame.bob * view).to_cols_array_2d(),
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.hand_globals,
            0,
            bytemuck::bytes_of(&hand_globals),
        );
        // GameRenderer.render3dHud: DebugCrosshairRenderer draws with the
        // hand's projection but no bob, and FogMode.NONE (no fog colour).
        let crosshair_globals = Globals {
            view_proj: (hand_projection * view).to_cols_array_2d(),
            fog_color: [0.0; 4],
            ..globals
        };
        self.uploads.buffer(
            &mut encoder,
            &self.crosshair_globals,
            0,
            bytemuck::bytes_of(&crosshair_globals),
        );
        // Frustum culling and camera-relative origins.
        let cull_proj = views.cull().view_proj(pose);
        let view = Frustum {
            planes: frustum(cull_proj),
        };
        let camera = frame.camera;
        let camera_section = camera_section(camera, self.layout.min_y());
        // The water mirror draws the same lists: they take in what its camera sees too
        // (above the screen's top, the far shore, the trees against the sky).
        let mirror_plane = frame
            .water_plane
            .filter(|_| !frame.title && self.wants_water_plane());
        let mirror = mirror_plane.map(|plane| Frustum {
            planes: frustum(mirror_view_projection(
                cull_proj,
                plane - frame.camera.y as f32,
            )),
        });
        // Solid quads facing away from the eye are left out: the bob moves
        // the eye off `camera`, so it is found from the projection. With the mirror, those
        // facing its eye are kept too: it sits straight below, at this camera-relative y.
        let eye = projection_center(cull_proj).filter(|_| self.sides_on);
        let mirror_low = mirror_plane
            .zip(eye)
            .map(|(plane, eye)| 2.0 * (plane - frame.camera.y as f32) - eye.y);
        let culling = !frame.title && frame.cave_culling;
        if culling {
            crate::spans::time("occlusion", || {
                self.start_occlusion(camera, frame.detail_distance)
            });
        }
        let reachable = &self.occlusion;
        let cutout = &self.lod_cutout;
        // Sections fading out (`leave_section`) are past the render distance:
        // drawn there until they have, or they popped out at once.
        let leaving: crate::fast_hash::FxHashSet<(i32, i32)> = self
            .lod_leaving
            .iter()
            .map(|&((x, _, z), _)| (x, z))
            .collect();
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        crate::spans::time("visibility", || {
            self.sections.visible(
                camera,
                (frame.detail_distance / 16.0) as i32,
                &Frusta(view, mirror),
                |k| !culling || reachable.contains(k),
                |x, z| {
                    (!cutout.is_empty() && cutout.contains(&(x, z)))
                        || (!leaving.is_empty() && leaving.contains(&(x, z)))
                },
                &mut visible,
            )
        });
        let now = std::time::Instant::now();
        let mut visible_lods = std::mem::take(&mut self.visible_lods);
        visible_lods.clear();
        if frame.title {
            visible.clear();
        } else {
            visible_lods.extend(
                self.lods
                    .iter()
                    .filter_map(|(&key, tile)| tile.visible(key, camera, &view, now)),
            );
            visible_lods.sort_unstable_by_key(|v| (v.distance, v.key));
        }
        // A shader pack's shadow pass draws the sections in its camera's box, all of them (the
        // occlusion graph follows the eye, not the light), within shadowDistanceRenderMul.
        let shadow_view = self
            .pack_post
            .as_ref()
            .and_then(|p| p.1.as_ref())
            .filter(|_| !frame.title)
            .and_then(|p| p.shadow_view(frame))
            .or_else(|| {
                // One of Horizon's own packs with `[shadow]` uses the same lists.
                let native = self.native.as_ref().filter(|_| !frame.title)?;
                native.shadow.as_ref()?;
                native.frame.shadow_view(frame)
            });
        let mut shadow_visible = std::mem::take(&mut self.shadow_visible);
        shadow_visible.clear();
        if let Some(v) = &shadow_view {
            let radius = (frame.detail_distance / 16.0) as i32;
            let radius = v
                .distance
                .map_or(radius, |d| radius.min((d / 16.0).ceil() as i32));
            let light = Frustum {
                planes: frustum(v.cull),
            };
            crate::spans::time("shadow visibility", || {
                // Within the unculled distance every section, then those in the light's box.
                if let Some(d) = v.unculled {
                    let all = Frustum {
                        planes: [glam::Vec4::W; 6],
                    };
                    let near = radius.min((d / 16.0).ceil() as i32);
                    self.sections.visible(
                        camera,
                        near,
                        &all,
                        |_| true,
                        |_, _| false,
                        &mut shadow_visible,
                    );
                }
                let mut boxed = Vec::new();
                self.sections
                    .visible(camera, radius, &light, |_| true, |_, _| false, &mut boxed);
                let seen: std::collections::HashSet<_> =
                    shadow_visible.iter().map(|s| s.key).collect();
                shadow_visible.extend(boxed.into_iter().filter(|s| !seen.contains(&s.key)));
            });
            // Nearest the light first, so the depth test rejects what lies under it.
            let light = v.light;
            shadow_visible.sort_unstable_by(|a, b| {
                let height = |s: &resident::Visible| glam::Vec3::from(s.origin).dot(light);
                height(b).total_cmp(&height(a))
            });
        }
        if self.animated_textures {
            self.active_animated_layers.fill(0);
            for v in &visible {
                let s = self.sections.get(v.slot);
                for &layer in &*s.animated_layers {
                    let layer = layer as usize;
                    self.active_animated_layers[layer / u64::BITS as usize] |=
                        1_u64 << (layer % u64::BITS as usize);
                }
            }
            for v in &visible_lods {
                if let Some(tile) = self.lods.get(&v.key) {
                    for &layer in &*tile.drawn().animated_layers {
                        let layer = layer as usize;
                        self.active_animated_layers[layer / u64::BITS as usize] |=
                            1_u64 << (layer % u64::BITS as usize);
                    }
                    for &layer in &*tile.mesh.animated_layers {
                        let layer = layer as usize;
                        self.active_animated_layers[layer / u64::BITS as usize] |=
                            1_u64 << (layer % u64::BITS as usize);
                    }
                }
            }
            for v in &shadow_visible {
                let s = self.sections.get(v.slot);
                for &layer in &*s.animated_layers {
                    let layer = layer as usize;
                    self.active_animated_layers[layer / u64::BITS as usize] |=
                        1_u64 << (layer % u64::BITS as usize);
                }
            }
            // Visibility is collected before terrain draw commands are recorded,
            // so upload the current mask here and keep animation updates ahead
            // of every draw in this command buffer.
            self.animate(&mut encoder, frame.ticks, !frame.title);
        }
        self.drawn_sections = visible.len() + visible_lods.len();
        let instances = self.drawn_sections + shadow_visible.len();
        if instances as u64 > self.instance_capacity {
            self.instance_capacity = (instances as u64).next_power_of_two();
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Section origins"),
                size: self.instance_capacity * 16,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
        self.origins.clear();
        let (sections, retiring) = (&self.sections, &self.lod_retiring);
        laps.lap("r_cameras");
        // `LevelRenderer.extractSectionDrawGroups`: each section's `getVisibility`. Once
        // nothing is fading (the usual frame), every section is whole without a lookup.
        let fade_ms = fade_millis(frame.chunk_fade);
        let fading_in = self.sections_fading_in(fade_ms, now);
        let fading_over_lods = self.sections_fading_over_lods(now);
        let fade_code = move |section: &SectionGpu, key: &SectionKey| {
            if fading_over_lods {
                section.fade(now, retiring.contains(key))
            } else {
                0
            }
        };
        let uploaded = &self.section_uploaded;
        let visibility = move |key: SectionKey| {
            if !fading_in {
                return 1.0;
            }
            uploaded.get(&key).map_or(1.0, |t| {
                chunk_visibility(
                    now.saturating_duration_since(*t).as_millis() as u64,
                    fade_ms,
                )
            })
        };
        self.origins.extend(
            visible
                .iter()
                .map(|v| {
                    let code = fade_code(sections.get(v.slot), &v.key);
                    section_instance(v.origin, code, visibility(v.key))
                })
                .chain(visible_lods.iter().map(|v| lod::instance(v.origin, v.fade)))
                .chain(
                    shadow_visible
                        .iter()
                        .map(|v| section_instance(v.origin, 0, visibility(v.key))),
                ),
        );
        if !self.origins.is_empty() {
            self.uploads.buffer(
                &mut encoder,
                &self.instances,
                0,
                bytemuck::cast_slice(&self.origins),
            );
        }
        // Solid sections front to back, then translucent ones back to front,
        // as one list of indirect draws over the section arena.
        self.draws.clear();
        self.draw_pages.clear();
        // Occlusion culling leaves the sections its last result did not see
        // to phase 2, after the others, with instance counts of 0. Whether
        // the frame has draws enough goes by the last one's.
        // A shader pack drawing terrain draws all of it: occlusion culling reads the depth of
        // Horizon's own terrain draw.
        let pack_terrain = !frame.title
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_terrain());
        let mut cull = self
            .cull
            .as_mut()
            .filter(|c| {
                self.cull_on
                    && !frame.title
                    && !pack_terrain
                    && self.multi_draw
                    // Occlusion is found from the camera, not the mirror's eye.
                    && mirror.is_none()
                    && visible.len() >= c.min_sections
            })
            .and_then(|cull| cull.begin(&self.device).then_some(cull));
        self.phase2_draws.clear();
        self.phase2_pages.clear();
        self.solid_runs.clear();
        self.phase2_runs.clear();
        // On Metal, phase 1 goes the opaque quads of every list, then the cutout
        // ones (`Part`), each solid pass drawing its own: `[..opaque_draws]`,
        // then the rest. A fading list's quads all draw in the cutout pass.
        let split_solid = self.info.backend == wgpu::Backend::Metal;
        let parts = |fading: bool| -> &'static [(u8, Quads)] {
            match (split_solid, fading) {
                (false, _) => &[(0, Quads::All)],
                (true, false) => &[(0, Quads::Opaque), (1, Quads::Cutout)],
                (true, true) => &[(1, Quads::All)],
            }
        };
        let mut opaque_draws = 0;
        for (i, v) in visible.iter().enumerate() {
            let s = self.sections.get(v.slot);
            let first_phase = cull.as_ref().is_none_or(|c| c.first(v.slot));
            let (draws, pages) = if first_phase {
                (&mut self.draws, &mut self.draw_pages)
            } else {
                (&mut self.phase2_draws, &mut self.phase2_pages)
            };
            let instances = u32::from(first_phase);
            let mut indices = 0;
            // Phase 2 draws every quad, in both passes.
            let parts = if first_phase {
                parts(fade_code(s, &v.key) != 0)
            } else {
                &[(0, Quads::All)]
            };
            let whole = draws.len();
            for &(group, part) in parts {
                let start = draws.len();
                facing_runs_from(&s.sides, eye, mirror_low, v.origin, part, |first, quads| {
                    indices += quads * 6;
                    let base = s.start as u32 + first * 4;
                    draws.push([quads * 6, instances, 0, base, i as u32]);
                    pages.push((s.page, None));
                });
                let len = draws.len() - start;
                if first_phase {
                    if group == 0 {
                        opaque_draws += len;
                    }
                    self.solid_runs
                        .push_in(group, v.distance, s.page, start, len, [0; 3]);
                }
            }
            if !first_phase {
                self.phase2_runs.push(
                    v.distance,
                    s.page,
                    whole,
                    draws.len() - whole,
                    [i as u32, v.slot, indices],
                );
            }
            if let Some(c) = &mut cull
                && first_phase
            {
                c.drawn(i, v.slot, indices);
            }
        }
        // Phase 2's draws are tested where grouping left them.
        let runs = self
            .phase2_runs
            .regroup(&mut self.phase2_draws, &mut self.phase2_pages, 0);
        if let Some(c) = &mut cull {
            for r in runs {
                let [i, slot, indices] = r.tag;
                c.deferred(i as usize, slot, r.start as usize, r.len as usize, indices);
            }
        }
        // Distant terrain tiles are never occlusion tested: phase 1.
        for (i, v) in visible_lods.iter().enumerate() {
            let s = self.lods[&v.key].drawn();
            let instance = (visible.len() + i) as u32;
            for &(group, part) in parts(v.fade != 0) {
                let start = self.draws.len();
                facing_runs_from(&s.sides, eye, mirror_low, v.origin, part, |first, quads| {
                    let base = s.start as u32 + first * 4;
                    self.draws.push([quads * 6, 1, 0, base, instance]);
                    self.draw_pages.push((s.page, None));
                });
                let len = self.draws.len() - start;
                if group == 0 {
                    opaque_draws += len;
                }
                self.solid_runs
                    .push_in(group, v.distance, s.page, start, len, [0; 3]);
            }
        }
        self.solid_runs
            .regroup(&mut self.draws, &mut self.draw_pages, 0);
        let phase1_draws = self.draws.len() as u32;
        // Draws `[..opaque_end]` hold opaque quads, `[cutout_start..phase1_draws]` cutout ones.
        let (opaque_end, cutout_start) = if split_solid {
            (opaque_draws, opaque_draws)
        } else {
            (phase1_draws as usize, 0)
        };
        self.draws.extend_from_slice(&self.phase2_draws);
        self.draw_pages.extend_from_slice(&self.phase2_pages);
        let culling = cull.is_some();
        let solid_draws = self.draws.len() as u32;
        laps.lap("r_section_groups");
        // LevelRenderer: nearby sections each frame, plus a rotating eighth
        // (at least 15) of the visible list. Cached views avoid needless sorts.
        self.translucent_draws.clear();
        let count = visible.len() + visible_lods.len();
        let budget = (count / 8).max(15).min(count);
        self.translucency_cursor %= count.max(1);
        for (i, v) in visible.iter().enumerate().rev() {
            let s = self.sections.get_mut(v.slot);
            if let Some((index_page, index_start)) = s.sorted_indices {
                let nearby = (v.key.0 - camera_section.0).abs() <= 2
                    && (v.key.1 - camera_section.1).abs() <= 2
                    && (v.key.2 - camera_section.2).abs() <= 2;
                let scheduled = (i + count - self.translucency_cursor) % count < budget;
                let origin = [
                    v.key.0 * 16,
                    self.layout.min_y() + v.key.1 * 16,
                    v.key.2 * 16,
                ];
                if (!s.sort.is_sorted() || nearby || scheduled)
                    && let Some(indices) =
                        s.sort
                            .update(camera.to_array(), origin, &mut self.translucent_sorter)
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let (page, first) = s.translucent_at;
                self.translucent_draws.push((
                    v.distance,
                    i,
                    [
                        s.translucent * 6,
                        1,
                        index_start as u32,
                        first as u32,
                        i as u32,
                    ],
                    (page, Some(index_page)),
                ));
            }
        }
        // Sections went back to front (`visible` is sorted by distance, then
        // key): tiles' water only needs sorting in when there is some.
        let section_translucent = self.translucent_draws.len();
        for (i, v) in visible_lods.iter().enumerate() {
            let instance = visible.len() + i;
            let tile = self.lods.get_mut(&v.key).expect("visible resident tile");
            let s = tile.drawn_mut();
            if let Some((index_page, index_start)) = s.sorted_indices {
                let scheduled = (instance + count - self.translucency_cursor) % count < budget;
                if (!s.sort.is_sorted() || scheduled)
                    && let Some(indices) = s.sort.update_lod(
                        camera.to_array(),
                        v.key.origin(),
                        &mut self.translucent_sorter,
                    )
                {
                    self.uploads.buffer(
                        &mut encoder,
                        self.sorted_indices.buffer(index_page),
                        index_start * 4,
                        bytemuck::cast_slice(indices),
                    );
                }
                let (page, first) = s.translucent_at;
                self.translucent_draws.push((
                    v.distance,
                    instance,
                    [
                        s.translucent * 6,
                        1,
                        index_start as u32,
                        first as u32,
                        instance as u32,
                    ],
                    (page, Some(index_page)),
                ));
            }
        }
        // Near water and distant water share one back-to-front draw order.
        if self.translucent_draws.len() > section_translucent {
            // Distant water goes after the near chunks', and replaced tiles'
            // after all: each is a hair behind the one before, so depth leaves
            // it only where that one has none. Back to front together, a far
            // tile's water under a near chunk's (one fading in, or brought
            // into view by a longer render distance) blended twice and looked
            // opaque. The cost: far water just behind near water shows no
            // surface of its own there.
            let tier = |instance: usize| match instance.checked_sub(visible.len()) {
                None => 0,
                Some(tile) if visible_lods[tile].old => 2,
                Some(_) => 1,
            };
            self.translucent_draws
                .sort_unstable_by_key(|&(distance, instance, ..)| {
                    (
                        tier(instance),
                        std::cmp::Reverse(distance),
                        std::cmp::Reverse(instance),
                    )
                });
        }
        for &(_, _, draw, pages) in &self.translucent_draws {
            self.draws.push(draw);
            self.draw_pages.push(pages);
        }
        self.translucency_cursor += budget;
        let translucent_draws = self.draws.len() as u32 - solid_draws;
        self.section_draws = self.draws.len();
        // The shadow camera's sections after the camera's: quads that can face the light, then
        // translucent ones (in their last sorted order; depth decides there).
        let shadow_first = self.draws.len() as u32;
        let mut shadow_translucent = Vec::new();
        self.shadow_runs.clear();
        if let Some(view) = &shadow_view {
            let base = self.drawn_sections;
            let toward = view.light * 1.0e5;
            // How far below the nearest section to the light (squared, as bits, as `PageRuns`
            // takes distances): each band's draws go by page, a call per page and band.
            let height = |v: &resident::Visible| glam::Vec3::from(v.origin).dot(view.light);
            let top = shadow_visible.first().map_or(0.0, height);
            for (k, v) in shadow_visible.iter().enumerate() {
                let s = self.sections.get_mut(v.slot);
                let instance = (base + k) as u32;
                let depth = f64::from(top - height(v)).max(0.0);
                let distance = (depth * depth).to_bits();
                // A section the camera never saw has no sorted indices yet: sorted once now.
                if let Some((index_page, index_start)) = s.sorted_indices
                    && !s.sort.is_sorted()
                {
                    let origin = [
                        v.key.0 * 16,
                        self.layout.min_y() + v.key.1 * 16,
                        v.key.2 * 16,
                    ];
                    if let Some(indices) =
                        s.sort
                            .update(camera.to_array(), origin, &mut self.translucent_sorter)
                    {
                        self.uploads.buffer(
                            &mut encoder,
                            self.sorted_indices.buffer(index_page),
                            index_start * 4,
                            bytemuck::cast_slice(indices),
                        );
                    }
                }
                let before = self.draws.len();
                // Every face of a section within the unculled distance.
                let unculled = view.unculled.is_some_and(|d| {
                    let o = glam::Vec3::from(v.origin) + 8.0;
                    o.x.abs().max(o.z.abs()) <= d + 8.0
                });
                let toward = (!unculled).then_some(toward);
                facing_runs(&s.sides, toward, v.origin, |first, quads| {
                    let start = s.start as u32 + first * 4;
                    self.draws.push([quads * 6, 1, 0, start, instance]);
                    self.draw_pages.push((s.page, None));
                });
                let len = self.draws.len() - before;
                self.shadow_runs.push(distance, s.page, before, len, [0; 3]);
                if let Some((index_page, index_start)) = s.sorted_indices {
                    let (page, start) = s.translucent_at;
                    shadow_translucent.push((
                        distance,
                        [
                            s.translucent * 6,
                            1,
                            index_start as u32,
                            start as u32,
                            instance,
                        ],
                        (page, Some(index_page)),
                    ));
                }
            }
        }
        let shadow_solid = self.draws.len() as u32 - shadow_first;
        self.shadow_runs
            .regroup(&mut self.draws, &mut self.draw_pages, shadow_first as usize);
        self.shadow_runs.clear();
        for (distance, draw, pages) in shadow_translucent {
            self.shadow_runs
                .push(distance, pages.0, self.draws.len(), 1, [0; 3]);
            self.draws.push(draw);
            self.draw_pages.push(pages);
        }
        self.shadow_runs.regroup(
            &mut self.draws,
            &mut self.draw_pages,
            (shadow_first + shadow_solid) as usize,
        );
        let shadow_translucent = self.draws.len() as u32 - shadow_first - shadow_solid;
        laps.lap("r_nearby");
        // Metal replays a multi-draw as one draw call per entry.
        self.section_draw_calls = if self.multi_draw && self.info.backend != wgpu::Backend::Metal {
            (0..self.section_draws)
                .filter(|&i| {
                    i == 0
                        || i == phase1_draws as usize
                        || i == solid_draws as usize
                        || self.draw_pages[i] != self.draw_pages[i - 1]
                })
                .count()
        } else {
            self.section_draws
        };
        let debug_depth = frame.debug_lines.len() as u32;
        let debug_top = frame.debug_lines_on_top.len() as u32;
        let crosshair = frame.debug_crosshair.len() as u32;
        if debug_depth + debug_top + crosshair > 0 {
            let needed = u64::from(debug_depth + debug_top + crosshair);
            if needed > self.debug_line_capacity {
                self.debug_line_capacity = needed.next_power_of_two();
                self.debug_line_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Debug lines"),
                    size: 32 * self.debug_line_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let all: Vec<[f32; 8]> = frame
                .debug_lines
                .iter()
                .chain(frame.debug_lines_on_top)
                .chain(frame.debug_crosshair)
                .copied()
                .collect();
            self.uploads.buffer(
                &mut encoder,
                &self.debug_line_buffer,
                0,
                bytemuck::cast_slice(&all),
            );
        }
        self.weather.prepare(
            &self.device,
            &mut encoder,
            &mut self.uploads,
            frame.weather,
            frame.weather_time,
        );
        let outline_edges = frame.outline.len().min(MAX_OUTLINE_EDGES as usize);
        if outline_edges > 0 {
            // The same per-line colour and pixel width layout as the gizmo lines. A high
            // contrast outline is two strokes: the black one first, the bright one after it.
            let edges = outline_vertices(&frame.outline[..outline_edges], camera);
            let width = outline_width(self.config.width);
            let stroke = |color: u32, width: f32| {
                edges.iter().map(move |e| {
                    let mut line = [0.0; 8];
                    line[..6].copy_from_slice(e);
                    line[6] = f32::from_bits(color);
                    line[7] = width;
                    line
                })
            };
            let lines: Vec<[f32; 8]> = if frame.outline_high_contrast {
                stroke(0xff00_0000, 7.0)
                    .chain(stroke(0xff57_ffe1, width))
                    .collect()
            } else {
                stroke(0x6600_0000, width).collect()
            };
            self.uploads
                .buffer(&mut encoder, &self.outline, 0, bytemuck::cast_slice(&lines));
        }
        // While a shader pack draws entities, the runs of `entity_spans` its programs draw are
        // left out of Horizon's own entity draw.
        let mut entities = frame.entities;
        if !frame.title
            && let Some((_, Some(post))) = &mut self.pack_post
            && post.has_entities()
        {
            post.prepare_entities(
                &self.device,
                &self.queue,
                frame.entities,
                frame.entity_spans,
                &mut self.pack_entity_rest,
            );
            entities = &self.pack_entity_rest;
        }
        // GameRenderer.useImprovedTransparency. A shader pack keeps its own
        // path.
        let oit_on = self.improved_transparency
            && !frame.title
            && self.pack_post.as_ref().is_none_or(|p| p.1.is_none())
            && self.native.is_none()
            && oit::supported(&self.device);
        let [
            entity_quads,
            text_quads,
            crumbling_quads,
            translucent_crumbling_quads,
            water_mask_quads,
            swirl_quads,
            shadow_quads,
            translucent_particle_quads,
            translucent_item_quads,
            oit_entity_quads,
        ] = self.entity_batch.update(
            entities,
            frame.additive,
            frame.hand,
            frame.see_through,
            oit_on,
        );
        self.entity_batch.append_shadow_only(frame.shadow_only);
        let weather_quads = frame.weather.len() as u32 / 4;
        let world_quads = entity_quads
            + text_quads
            + crumbling_quads
            + translucent_crumbling_quads
            + water_mask_quads
            + swirl_quads
            + shadow_quads
            + translucent_particle_quads
            + translucent_item_quads
            + oit_entity_quads;
        laps.lap("r_draw_lists");
        // Where the tail of materials the classic and the improved passes
        // both draw ends: the shadows, particles and items count back from it.
        let base_quads = world_quads - oit_entity_quads;
        let native_clouds = self.native.as_ref().is_none_or(|n| n.vanilla_clouds);
        let cloud_quads = if let Some(input) = frame.clouds.as_ref().filter(|_| native_clouds) {
            let (rebuilt, uniform) =
                crate::spans::time("clouds", || self.cloud_mesh.prepare(input));
            let vertices = self.cloud_mesh.vertices.len() as u64;
            if rebuilt && vertices > 0 {
                if vertices > self.cloud_capacity {
                    self.cloud_capacity = vertices.next_power_of_two();
                    self.cloud_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Cloud vertices"),
                        size: self.cloud_capacity * std::mem::size_of::<Vertex>() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
                self.uploads.buffer(
                    &mut encoder,
                    &self.cloud_buffer,
                    0,
                    bytemuck::cast_slice(&self.cloud_mesh.vertices),
                );
            }
            self.uploads.buffer(
                &mut encoder,
                &self.cloud_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
            (vertices / 4) as u32
        } else {
            0
        };
        self.lightning.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            &frame.lightning,
            camera,
        );
        // Frame generation's made frame draws its own hand, halfway, after the others.
        let made_hand_vertex = self.entity_batch.combined.len() as i32;
        if fg {
            self.frame_gen
                .halfway_hand(frame.hand, hand_view, &mut self.entity_batch.combined);
            self.frame_gen.track_entities(
                &self.device,
                &self.entity_atlas,
                frame.entities,
                frame.entity_motion,
                frame.camera,
            );
        }
        let all_entities = &self.entity_batch.combined;
        let hand_quads = (frame.hand.len() / 4) as u32;
        let additive_quads = (frame.additive.len() / 4) as u32 - self.entity_batch.moved_additive;
        let see_through_quads = (frame.see_through.len() / 4) as u32;
        let shadow_only_quads = (frame.shadow_only.len() / 4) as u32;
        let entity_vertices = all_entities.len() as u64;
        if entity_vertices > self.entity_capacity {
            self.entity_capacity = entity_vertices.next_power_of_two();
            self.entity_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Entity vertices"),
                size: self.entity_capacity * std::mem::size_of::<Vertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if entity_vertices > 0 {
            self.uploads.buffer(
                &mut encoder,
                &self.entity_buffer,
                0,
                bytemuck::cast_slice(all_entities),
            );
        }
        self.entity_outline.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.glowing,
            [self.config.width, self.config.height],
        );
        if let Some((rotation, brightness)) = frame.stars {
            self.stars.prepare(
                &mut self.uploads,
                &mut encoder,
                jittered * rotation,
                brightness,
            );
        }
        let celestial_vertices = frame.celestial.len().min(2048 * 4);
        if frame.end_sky && !frame.title {
            self.end_sky.prepare(&self.device, frame.end_sky_sprite);
        }
        if celestial_vertices > 0 {
            self.uploads.buffer(
                &mut encoder,
                &self.celestial_buffer,
                0,
                bytemuck::cast_slice(&frame.celestial[..celestial_vertices]),
            );
        }
        let max_quads =
            (world_quads + additive_quads + hand_quads + see_through_quads + shadow_only_quads)
                .max(cloud_quads)
                .max(if frame.lightning.bolts.is_empty() {
                    0
                } else {
                    lightning::QUADS
                })
                .max(weather_quads)
                .max(frame.glowing.len() as u32 / 4);
        if max_quads > self.index_quads {
            self.index_quads = max_quads.next_power_of_two();
            self.indices = Self::quad_indices(&self.device, self.index_quads);
        }
        if let Some(b) = &frame.border {
            if b.rebuilt {
                self.uploads.buffer(
                    &mut encoder,
                    &self.border_vertices,
                    0,
                    bytemuck::cast_slice(&b.vertices),
                );
            }
            let t = &b.transforms;
            let uniform = BorderUniform {
                color: t.color,
                offset: [
                    t.model_offset[0],
                    t.model_offset[1],
                    t.model_offset[2],
                    t.texture_offset,
                ],
            };
            self.uploads.buffer(
                &mut encoder,
                &self.border_uniform,
                0,
                bytemuck::bytes_of(&uniform),
            );
        }
        laps.lap("r_materials");
        // Every upload before the frame's first pass: a pass ends wgpu-core's
        // command buffer, so copies between passes would need one more.
        if !frame.screen.is_empty() {
            let vertices: Vec<crate::screen_effects::ScreenVertex> =
                frame.screen.iter().flat_map(|q| q.vertices).collect();
            if vertices.len() as u64 > self.screen_capacity {
                self.screen_capacity = (vertices.len() as u64).next_power_of_two();
                self.screen_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Screen effect vertices"),
                    size: self.screen_capacity
                        * std::mem::size_of::<crate::screen_effects::ScreenVertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.screen_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );
        }
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.hud,
            [self.config.width, self.config.height],
        );
        if culling && let Some(cull) = &mut self.cull {
            cull.upload(
                &self.device,
                &mut self.uploads,
                &mut encoder,
                &self.draws[phase1_draws as usize..solid_draws as usize],
                &self.instances,
                &self.globals,
            );
        }
        let flat_frame = match &mut self.flat {
            Some(flat)
                if self.flat_on
                    && !self.draws.is_empty()
                    && (self.drawn_sections >= flat.min_sections
                        || pack_terrain
                        || shadow_view.is_some()) =>
            {
                // They read translucent quads in the section arena (`translucent_split`).
                debug_assert!(self.translucent_arena.is_none(), "flat draws, split");
                let sections = &self.sections;
                let lods = &self.lods;
                let sorted = &self.sorted_indices;
                let phase2 = self.cull.as_ref().filter(|_| culling).map(|c| &c.phase2);
                crate::spans::time("flatten", || {
                    flat.prepare(
                        &self.device,
                        &mut encoder,
                        &mut self.uploads,
                        &self.draws,
                        &self.draw_pages,
                        phase1_draws as usize,
                        solid_draws as usize,
                        [shadow_first, shadow_first + shadow_solid].map(|d| d as usize),
                        [opaque_end, cutout_start],
                        phase2,
                        visible
                            .iter()
                            .chain(&shadow_visible)
                            .filter_map(|v| {
                                let section = sections.get(v.slot);
                                let code = fade_code(section, &v.key);
                                Some((
                                    section.flat_id?,
                                    section_instance(v.origin, code, visibility(v.key)),
                                ))
                            })
                            .chain(visible_lods.iter().filter_map(|v| {
                                let id = lods[&v.key].drawn().flat_id?;
                                Some((id, lod::instance(v.origin, v.fade)))
                            })),
                        |page| sorted.buffer(page).clone(),
                        self.own_timing.as_mut(),
                    )
                })
            }
            _ => false,
        };
        if flat_frame {
            self.section_draw_calls = self.flat.as_ref().map_or(0, |f| f.shadow_calls[0]);
        } else if self.multi_draw && !self.draws.is_empty() {
            if self.draws.len() as u64 > self.indirect_capacity {
                self.indirect_capacity = (self.draws.len() as u64).next_power_of_two();
                self.indirect = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Section draws"),
                    size: self.indirect_capacity * 20,
                    usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                &mut encoder,
                &self.indirect,
                0,
                bytemuck::cast_slice(&self.draws),
            );
        }
        let view_target = target_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        if let Some(timer) = &mut self.gpu_timer {
            timer.begin(&mut encoder);
        }
        // The world's targets: the screen, or the scaled target of `post`.
        let pack_post = !frame.title && self.pack_post.as_ref().is_some_and(|p| p.1.is_some());
        // One of Horizon's own packs reads the same world target.
        let native_post = !frame.title && self.native.as_ref().is_some_and(|n| n.post.is_some());
        if let Some(timer) = Self::pass_timer(&mut self.pack_post, &mut self.native)
            && timer.is_some() != self.pass_timing
        {
            *timer = self
                .pass_timing
                .then(|| pack_timing::PassTiming::new(&self.device, &self.queue))
                .flatten();
        }
        let pack_reads_world = pack_post || native_post;
        laps.lap("r_uploads");
        // Phase 2 of occlusion culling draws over the depth, and a shader pack or TAA reads it.
        let keep_depth = pack_reads_world
            || (taa && self.world_post.taa)
            || oit_on
            || native_render::render_needs().world_depth
            || fg;
        // The pack's program reads the vertices this frame's draws bind.
        let pack_draws_terrain = pack_terrain && flat_frame == self.pack_terrain_flat;
        // The shadow camera's lists, drawn by the pack's shadow programs.
        let pack_draws_shadow = shadow_view.is_some() && flat_frame == self.pack_terrain_flat;
        let pack_draws_sky = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_sky());
        let pack_draws_celestial = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_celestial());
        let pack_draws_water = pack_draws_terrain
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_water());
        let pack_draws_clouds = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_clouds());
        let pack_draws_hand = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_hand());
        let pack_draws_weather = pack_post
            && self
                .pack_post
                .as_ref()
                .and_then(|p| p.1.as_ref())
                .is_some_and(|p| p.has_weather());
        let scaled = self.world_post.prepare(
            &self.device,
            [self.config.width, self.config.height],
            pack_reads_world || fg,
        );
        let (world_view, world_depth) = match &scaled {
            Some((color, depth)) => (color, depth),
            None => (&view_target, &self.depth),
        };
        if culling && let Some(cull) = &mut self.cull {
            cull.bind_depth(&self.device, world_depth);
        }
        // The hand draws at the end of the world pass, after a depth reset, when nothing runs
        // between them (a scaled world's post pass, HDR's world copy, a native mod's
        // `native_world_drawn`): one render pass fewer, ~10 µs of wgpu's encoding a frame.
        let hand_in_world = (frame.hand.len() / 4) > 0
            && !pack_draws_hand
            && scaled.is_none()
            && self.hdr.is_none()
            // Frame generation draws the hand over the real and the made frame after the world.
            && fg_frames.is_none()
            && !native_render::world_drawn_hooked();
        if oit_on {
            let size = self
                .world_post
                .size()
                .filter(|_| scaled.is_some())
                .unwrap_or([self.config.width, self.config.height]);
            let (device, layout, sprites) =
                (&self.device, &self.terrain_layout, self.terrain_sprites);
            let clouds = [0, 1].map(|i| self.cloud_pipeline.get_bind_group_layout(i));
            let format = self.config.format.remove_srgb_suffix();
            let flat = self
                .flat
                .as_ref()
                .map(|f| f.translucent.get_bind_group_layout(1));
            let features = [
                self.entity_pipeline.get_bind_group_layout(0),
                self.weather.pipeline().get_bind_group_layout(1),
                self.border_pipeline.get_bind_group_layout(0),
            ];
            self.oit
                .get_or_insert_with(|| {
                    oit::Oit::new(
                        device,
                        layout,
                        flat.as_ref(),
                        [&features[0], &features[1], &features[2]],
                        [&clouds[0], &clouds[1]],
                        sprites,
                        format,
                    )
                })
                .prepare(
                    &self.device,
                    &self.queue,
                    size,
                    &views.primary().projection,
                    [water_mask_quads > 0, cloud_quads > 0],
                );
        }
        let oit = self.oit.as_ref().filter(|_| oit_on);
        let cull = self.cull.as_ref().filter(|_| culling && solid_draws > 0);
        let pack_sky_clear = pack_draws_sky
            .then(|| self.pack_post.as_ref().and_then(|p| p.1.as_ref()))
            .flatten()
            .and_then(|p| p.sky_clear());
        let clear = if let Some([r, g, b, a]) = pack_sky_clear {
            wgpu::Color {
                r: f64::from(r),
                g: f64::from(g),
                b: f64::from(b),
                a: f64::from(a),
            }
        } else if frame.title || (frame.end_sky && !frame.underwater && sky_open(frame)) {
            wgpu::Color::BLACK
        } else {
            wgpu::Color {
                r: f64::from(fog_color[0]),
                g: f64::from(fog_color[1]),
                b: f64::from(fog_color[2]),
                a: 1.0,
            }
        };
        let sky_visible = sky_open(frame);
        laps.lap("r_world_pass");
        // One of Horizon's own packs: this frame's inputs; in a timed frame its sky draws in a
        // pass of its own, so that it has a time of its own.
        let mut native_sky_pass = false;
        if !frame.title
            && let Some(native) = &mut self.native
        {
            let size = self
                .world_post
                .size()
                .filter(|_| scaled.is_some())
                .unwrap_or([self.config.width, self.config.height]);
            native.frame.write(&self.queue, frame, size);
            native.frame.block_light.update(
                &self.queue,
                frame.camera,
                &self.light_blocks,
                self.light_revision,
            );
            if let Some(t) = &mut native.timing {
                t.begin(&self.device, &self.queue);
                native_sky_pass =
                    native.sky.is_some() && t.recording() && sky_visible && frame.skybox;
            }
        }
        // One of Horizon's own packs with `[shadow]`: the solid terrain the light sees, from the
        // shadow camera's lists, into the pack's shadow map (a pack without one gets its 1×1 map
        // cleared to "far" once).
        if !frame.title
            && let Some(native) = &mut self.native
        {
            native.frame.shadow_map.clear_blank(&mut encoder);
        }
        if !frame.title
            && shadow_view.is_some()
            && let Some(native) = &self.native
            && let Some(shadow) = &native.shadow
        {
            let mut pass = native.frame.shadow_map.begin(
                &mut encoder,
                "Native shader pack shadow",
                self.own_timing.as_mut(),
            );
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(horizon_shader_api::stages::GROUP, &shadow.group, &[]);
            if flat_frame
                && let Some(flat) = &self.flat
                && let Some(pipeline) = &shadow.flat
            {
                pass.set_pipeline(pipeline);
                let [first, translucent] = flat.shadow_calls;
                flat.draw(&mut pass, &flat.calls[first..translucent], |p| {
                    self.arena.buffer(p)
                });
            } else if !flat_frame {
                pass.set_pipeline(&shadow.solid);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(&mut pass, &self.indirect, 0, shadow_first, shadow_solid);
            }
            drop(pass);
            let mut pass = native.frame.shadow_map.begin_translucent(
                &mut encoder,
                "Native shader pack translucent shadow",
                self.own_timing.as_mut(),
            );
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(horizon_shader_api::stages::GROUP, &shadow.group, &[]);
            if flat_frame
                && let Some(flat) = &self.flat
                && let Some(pipeline) = &shadow.flat_translucent
            {
                pass.set_pipeline(pipeline);
                let [_, translucent] = flat.shadow_calls;
                flat.draw(&mut pass, &flat.calls[translucent..], |p| {
                    self.arena.buffer(p)
                });
            } else if !flat_frame {
                pass.set_pipeline(&shadow.translucent);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(
                    &mut pass,
                    &self.indirect,
                    0,
                    shadow_first + shadow_solid,
                    shadow_translucent,
                );
            }
            drop(pass);
            if entity_quads > 0 || shadow_only_quads > 0 {
                let mut pass = native
                    .frame
                    .shadow_map
                    .begin_entities(&mut encoder, "Native shader pack entity shadow");
                pass.set_pipeline(&shadow.entity);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(horizon_shader_api::stages::GROUP, &shadow.group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                // At the end of the buffer (`EntityBatch::append_shadow_only`).
                let first = entity_vertices as u32 - shadow_only_quads * 4;
                pass.draw_indexed(0..shadow_only_quads * 6, first as i32, 0..1);
            }
        }
        if native_sky_pass
            && let Some(native) = &mut self.native
            && let Some(sky) = &native.sky
        {
            if let Some(t) = &mut native.timing {
                let format = self.config.format.remove_srgb_suffix();
                t.mark(&self.device, &mut encoder, (world_view, format));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&sky.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: world_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: native.timing.as_mut().and_then(|t| t.writes(&sky.label)),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            sky.draw(&mut pass, &native.frame);
        }
        laps.lap("r_pack_inputs");
        // The water mirror, for one of Horizon's own packs that reads it: the world as a
        // camera mirrored about the water plane sees it, into a half-resolution target the
        // material stage samples as `reflection`.
        let mut reflection = None;
        if let Some(plane) = mirror_plane {
            let world = self
                .world_post
                .size()
                .filter(|_| scaled.is_some())
                .unwrap_or([self.config.width, self.config.height]);
            let size = native_post::reflection_size(world);
            if self.mirror.as_ref().is_none_or(|m| m.size != size) {
                let target = |label: &'static str, format, usage| {
                    self.device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some(label),
                            size: wgpu::Extent3d {
                                width: size[0],
                                height: size[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                };
                self.mirror = Some(MirrorTarget {
                    size,
                    color: target(
                        "Water mirror",
                        self.config.format.remove_srgb_suffix(),
                        wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                    ),
                    depth: target(
                        "Water mirror depth",
                        wgpu::TextureFormat::Depth32Float,
                        wgpu::TextureUsages::RENDER_ATTACHMENT,
                    ),
                });
            }
            if let Some(mirror) = &self.mirror {
                // Mirrored about y = `plane` (camera-relative).
                let plane = plane - frame.camera.y as f32;
                let inverse_view = mirror_inverse_view(globals.inverse_view, plane);
                // fog_ends.w: the plane, below which fs_solid_mirror cuts the world away
                // (only entity.wgsl reads the glint strength there, and no entity draws here).
                let mut fog_ends = globals.fog_ends;
                fog_ends[3] = plane;
                let mirror_globals = Globals {
                    view_proj: mirror_view_projection(view_proj, plane).to_cols_array_2d(),
                    inverse_view,
                    fog_ends,
                    ..globals
                };
                self.uploads.buffer(
                    &mut encoder,
                    &self.mirror_globals,
                    0,
                    bytemuck::bytes_of(&mirror_globals),
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Water mirror"),
                    timestamp_writes: self
                        .own_timing
                        .as_mut()
                        .and_then(|t| t.writes("Water mirror")),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mirror.color,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &mirror.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.mirror_group, &[]);
                if sky_visible && frame.skybox {
                    pass.set_pipeline(&self.sky);
                    pass.draw(0..3, 0..1);
                }
                // The solid terrain: occlusion culling is off with a mirror, so every draw
                // is phase 1's.
                if flat_frame && let Some(flat) = &self.flat {
                    pass.set_pipeline(&flat.solid_mirror);
                    let vertices = |p| self.arena.buffer(p);
                    flat.draw(&mut pass, &flat.calls[..flat.solid_calls], vertices);
                } else {
                    pass.set_pipeline(&self.solid_mirror);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    self.draw_sections(&mut pass, &self.indirect, 0, 0, solid_draws);
                }
                drop(pass);
                reflection = Some(mirror.color.clone());
            }
        }
        if let Some(native) = &mut self.native {
            native
                .frame
                .set_reflection(&self.device, reflection.as_ref());
        }
        // The material stage's copies of the world before translucent terrain, the size of
        // this frame's targets.
        if let Some(native) = &mut self.native
            && native.material.is_some()
            && !frame.title
        {
            let color = scaled.as_ref().map(|(c, _)| c.texture());
            native
                .frame
                .fit_opaque(&self.device, world_depth.texture(), color);
        }
        if let Some(native) = self.native.as_mut().filter(|_| !frame.title) {
            let writes = native.material.as_ref().is_some_and(|m| m.writes_surface);
            let depth = world_depth.texture();
            let size = if writes {
                [depth.width(), depth.height()]
            } else {
                [1, 1]
            };
            native.frame.fit_surface(&self.device, size);
        }
        laps.lap("r_mirror");
        // One of Horizon's own packs with a material stage: terrain draws with its pipelines,
        // which read the pack's frame at group 3.
        let material = self
            .native
            .as_ref()
            .filter(|_| !frame.title)
            .and_then(|n| Some((n.material.as_ref()?, &n.frame.group)));
        let solid_pipeline = material.map_or(&self.solid, |(m, _)| &m.solid);
        // Two solid passes only where a discard costs the GPU its hidden surface
        // removal: Apple's tiled GPUs, behind Metal. Elsewhere the second pass
        // would double the solid draw calls and vertex work for nothing.
        let split_solid = self.info.backend == wgpu::Backend::Metal;
        let translucent_pipeline = material.map_or(&self.translucent, |(m, _)| &m.translucent);
        let pack_flat = material.and_then(|(m, _)| m.flat.as_ref());
        // A material stage's translucent pass reads a copy of the depth the solid terrain
        // left: it is stored across the pass that ends before the copy.
        let keep_depth = keep_depth || material.is_some();
        let pack_frame = |pass: &mut wgpu::RenderPass<'_>| {
            if let Some((_, group)) = material {
                pass.set_bind_group(horizon_shader_api::stages::GROUP, group, &[]);
            }
        };
        // A pack's solid terrain that calls `set_surface` also writes the surface target, in
        // passes of its own between the sky and everything else.
        let surface = self
            .native
            .as_ref()
            .filter(|n| {
                !frame.title
                    && !pack_draws_terrain
                    && n.material.as_ref().is_some_and(|m| m.writes_surface)
            })
            .map(|n| n.frame.surface_view.clone());
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: world_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if native_sky_pass {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(clear)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon world"),
                timestamp_writes: self.own_timing.as_mut().and_then(|t| t.writes("World")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: if cull.is_some() || keep_depth {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            pack_frame(&mut pass);
            if frame.title || (sky_visible && (frame.skybox || frame.end_sky)) {
                if !frame.title && frame.skybox && !pack_draws_sky && !native_sky_pass {
                    match self
                        .native
                        .as_ref()
                        .and_then(|n| Some((n.sky.as_ref()?, &n.frame)))
                    {
                        Some((sky, native)) => sky.draw(&mut pass, native),
                        None => {
                            pass.set_pipeline(&self.sky);
                            pass.draw(0..3, 0..1);
                        }
                    }
                }
                if frame.title {
                    self.panorama.draw(&mut pass);
                    pass.set_bind_group(0, &self.group, &[]);
                } else if celestial_vertices > 0 || frame.end_sky {
                    pass.set_pipeline(&self.celestial_pipeline);
                    pass.set_bind_group(0, &self.entity_group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    if frame.end_sky {
                        self.end_sky.draw(&mut pass);
                    }
                    if celestial_vertices > 0 && !pack_draws_celestial {
                        // The Overworld's bodies start with the sun's quad
                        // (`Game::celestial_geometry`): a pack sky drawing its own skips it.
                        let own_sun = !frame.end_sky
                            && self
                                .native
                                .as_ref()
                                .is_some_and(|n| n.sky.is_some() && !n.vanilla_sun);
                        let first = if own_sun { 6 } else { 0 };
                        pass.set_vertex_buffer(0, self.celestial_buffer.slice(..));
                        pass.draw_indexed(first..(celestial_vertices as u32 / 4) * 6, 0, 0..1);
                    }
                    pass.set_bind_group(0, &self.group, &[]);
                }
                let own_stars = self
                    .native
                    .as_ref()
                    .is_some_and(|n| n.sky.is_some() && !n.vanilla_stars);
                if !frame.title && frame.stars.is_some() && !pack_draws_sky && !own_stars {
                    self.stars.draw(&mut pass);
                    pass.set_bind_group(0, &self.group, &[]);
                }
                if !frame.title && frame.dark_disc && !pack_draws_sky {
                    pass.set_pipeline(&self.dark_disc);
                    pass.draw(0..3, 0..1);
                }
            }
            let terrain_pass =
                |encoder: &mut wgpu::CommandEncoder, surface: &wgpu::TextureView, clear: bool| {
                    // Its lifetime forgotten: a closure cannot tie it to `encoder`'s.
                    let mut pass = surface_pass(encoder, world_view, surface, world_depth, clear)
                        .forget_lifetime();
                    pass.set_bind_group(0, &self.group, &[]);
                    pack_frame(&mut pass);
                    pass
                };
            // The occlusion test's pipeline draws into the world's colour alone.
            let plain_pass = |encoder: &mut wgpu::CommandEncoder| {
                let mut pass =
                    world_pass_after_culling(encoder, world_view, world_depth, true, None, "")
                        .forget_lifetime();
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
                pass
            };
            if let Some(surface) = &surface {
                drop(pass);
                pass = terrain_pass(&mut encoder, surface, true);
            }
            match self
                .flat
                .as_ref()
                .filter(|_| flat_frame && !pack_draws_terrain)
            {
                _ if pack_draws_terrain => {
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                Some(flat) => {
                    let vertices = |p| self.arena.buffer(p);
                    // Without a pack, on Metal, the opaque pass, then the cutout one
                    // (terrain.wgsl's `opaque_quad`), each over phase 1's lists with
                    // quads for it, and over all of phase 2's.
                    let [opaque, cutout] = flat.pass_calls;
                    let passes: &[(&wgpu::RenderPipeline, std::ops::Range<usize>)] = match pack_flat
                    {
                        Some(p) => &[(&p.0, 0..flat.phase1_calls)],
                        None if split_solid => &[
                            (&flat.solid_opaque, 0..opaque),
                            (&flat.solid_cutout, cutout..flat.phase1_calls),
                        ],
                        None => &[(&flat.solid, 0..flat.phase1_calls)],
                    };
                    let draw = |pass: &mut wgpu::RenderPass<'_>| {
                        for (pipeline, calls) in passes {
                            pass.set_pipeline(pipeline);
                            flat.draw(pass, &flat.calls[calls.clone()], vertices);
                        }
                    };
                    match cull {
                        // As below: phase 1, the test, then phase 2's draws
                        // that the test saw, copied after it.
                        Some(cull) => {
                            draw(&mut pass);
                            if surface.is_some() {
                                drop(pass);
                                pass = plain_pass(&mut encoder);
                            }
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder, self.own_timing.as_mut());
                            flat.flatten_phase2(&mut encoder, self.own_timing.as_mut());
                            pass = match &surface {
                                Some(surface) => terrain_pass(&mut encoder, surface, false),
                                None => world_pass_after_culling(
                                    &mut encoder,
                                    world_view,
                                    world_depth,
                                    keep_depth,
                                    self.own_timing.as_mut(),
                                    "World after culling",
                                ),
                            };
                            pass.set_bind_group(0, &self.group, &[]);
                            pack_frame(&mut pass);
                            for (pipeline, _) in passes {
                                pass.set_pipeline(pipeline);
                                flat.draw_phase2(&mut pass, vertices);
                            }
                        }
                        // Without culling, phase 1 is all the solid calls.
                        None => draw(&mut pass),
                    }
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                None => {
                    // Without a native material, on Metal, the opaque pass, then the
                    // cutout one (terrain.wgsl's `opaque_quad`), each over phase 1's
                    // lists with quads for it, and over all of phase 2's.
                    let (opaque, cutout) = (opaque_end as u32, cutout_start as u32);
                    let passes: &[(&wgpu::RenderPipeline, std::ops::Range<u32>)] = match material {
                        None if split_solid => &[
                            (&self.solid_opaque, 0..opaque),
                            (&self.solid_cutout, cutout..phase1_draws),
                        ],
                        _ => &[(solid_pipeline, 0..phase1_draws)],
                    };
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    match cull {
                        Some(cull) => {
                            // Phase 1 and the boxes against its depth (in
                            // its pass, or one of their own on Metal), then
                            // what they found in sight, in a pass that loads
                            // both targets (it reads the test's results).
                            for (pipeline, draws) in passes {
                                pass.set_pipeline(pipeline);
                                self.draw_sections(
                                    &mut pass,
                                    &self.indirect,
                                    0,
                                    draws.start,
                                    draws.len() as u32,
                                );
                            }
                            if surface.is_some() {
                                drop(pass);
                                pass = plain_pass(&mut encoder);
                            }
                            cull.test(&mut pass);
                            drop(pass);
                            cull.test_pass(&mut encoder, self.own_timing.as_mut());
                            pass = match &surface {
                                Some(surface) => terrain_pass(&mut encoder, surface, false),
                                None => world_pass_after_culling(
                                    &mut encoder,
                                    world_view,
                                    world_depth,
                                    keep_depth,
                                    self.own_timing.as_mut(),
                                    "World after culling",
                                ),
                            };
                            pass.set_bind_group(0, &self.group, &[]);
                            pack_frame(&mut pass);
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            for (pipeline, _) in passes {
                                pass.set_pipeline(pipeline);
                                self.draw_sections(
                                    &mut pass,
                                    &cull.phase2,
                                    phase1_draws,
                                    phase1_draws,
                                    solid_draws - phase1_draws,
                                );
                            }
                        }
                        // Without culling, phase 1 is all the solid draws.
                        None => {
                            for (pipeline, draws) in passes {
                                pass.set_pipeline(pipeline);
                                self.draw_sections(
                                    &mut pass,
                                    &self.indirect,
                                    0,
                                    draws.start,
                                    draws.len() as u32,
                                );
                            }
                        }
                    }
                }
            }
            if surface.is_some() {
                drop(pass);
                pass = plain_pass(&mut encoder);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            if entity_quads > 0 {
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                if self.hdr.is_some() {
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if text_quads > 0 && !oit_on {
                pass.set_pipeline(&self.text_polygon_offset_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(0..text_quads * 6, (entity_quads * 4) as i32, 0..1);
                pass.set_bind_group(0, &self.group, &[]);
            }
            if translucent_item_quads > 0 && !oit_on {
                pass.set_pipeline(&self.translucent_item_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_item_quads * 6,
                    ((base_quads - translucent_item_quads) * 4) as i32,
                    0..1,
                );
                if self.hdr.is_some() {
                    // A glowing item in the world (Item sheet quads), as entities are.
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(
                        0..translucent_item_quads * 6,
                        ((base_quads - translucent_item_quads) * 4) as i32,
                        0..1,
                    );
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if crumbling_quads > 0 && !oit_on {
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..crumbling_quads * 6,
                    ((entity_quads + text_quads) * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if shadow_quads > 0 && !oit_on {
                pass.set_pipeline(&self.shadow_blob_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..shadow_quads * 6,
                    ((base_quads
                        - translucent_item_quads
                        - translucent_particle_quads
                        - shadow_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            // BoatRenderer's water patch (RenderTypes.waterMask): depth only,
            // before translucent terrain, so no water shows inside the boat.
            if water_mask_quads > 0 && !oit_on {
                pass.set_pipeline(&self.water_mask_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..water_mask_quads * 6,
                    ((entity_quads + text_quads + crumbling_quads + translucent_crumbling_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if !frame.lightning.bolts.is_empty() && !oit_on {
                pass.set_pipeline(&self.lightning_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                self.lightning.draw(&mut pass, &frame.lightning);
                if self.hdr.is_some() {
                    pass.set_pipeline(&self.hdr_mark_lightning_pipeline);
                    self.lightning.draw(&mut pass, &frame.lightning);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if swirl_quads > 0 && !oit_on {
                pass.set_pipeline(&self.energy_swirl_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..swirl_quads * 6,
                    ((entity_quads
                        + text_quads
                        + crumbling_quads
                        + translucent_crumbling_quads
                        + water_mask_quads)
                        * 4) as i32,
                    0..1,
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            if additive_quads > 0 && !oit_on {
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                let mut first = 0;
                for run in frame.additive.as_chunks::<4>().0.chunk_by(|a, b| {
                    (a[0].layer & 0xffff == DRAGON_RAYS_OVERLAY)
                        == (b[0].layer & 0xffff == DRAGON_RAYS_OVERLAY)
                }) {
                    let rays = run[0][0].layer & 0xffff == DRAGON_RAYS_OVERLAY;
                    pass.set_pipeline(if rays {
                        &self.dragon_rays_pipeline
                    } else {
                        &self.additive_pipeline
                    });
                    let end = first + run.len() as u32;
                    pass.draw_indexed(first * 6..end * 6, (world_quads * 4) as i32, 0..1);
                    first = end;
                }
                if self.hdr.is_some() {
                    // The dragon's eyes and death rays, marked as entities are.
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(0..additive_quads * 6, (world_quads * 4) as i32, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            if outline_edges > 0 && !frame.outline_after_terrain {
                self.draw_outline(&mut pass, outline_edges as u32, frame.outline_high_contrast);
            }
            if debug_depth > 0 {
                pass.set_pipeline(&self.debug_lines);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, 0..debug_depth);
            }
            if debug_top > 0 {
                pass.set_pipeline(&self.debug_lines_on_top);
                pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
                pass.draw(0..4, debug_depth..debug_depth + debug_top);
            }
            // One of Horizon's own packs reads the world before translucent terrain
            // (`opaque_depth`, `opaque_color`): the pass ends, the targets are copied, and a
            // pass that loads both carries on.
            if material.is_some()
                && let Some(native) = &self.native
            {
                drop(pass);
                let depth = world_depth.texture();
                encoder.copy_texture_to_texture(
                    depth.as_image_copy(),
                    native.frame.opaque_depth.as_image_copy(),
                    depth.size(),
                );
                if let Some((color, _)) = &scaled {
                    let color = color.texture();
                    encoder.copy_texture_to_texture(
                        color.as_image_copy(),
                        native.frame.opaque_color.as_image_copy(),
                        color.size(),
                    );
                }
                pass = world_pass_after_culling(
                    &mut encoder,
                    world_view,
                    world_depth,
                    keep_depth,
                    self.own_timing.as_mut(),
                    "World after copy",
                );
                pass.set_bind_group(0, &self.group, &[]);
                pack_frame(&mut pass);
            }
            match self.flat.as_ref().filter(|_| flat_frame) {
                // The pack draws translucent terrain after its deferred programs.
                _ if pack_draws_water => {}
                // LevelRenderer.executeOit in place of the classic blend; what
                // follows draws over its composite in a pass of its own.
                flat if let Some(oit) = oit => {
                    drop(pass);
                    // LevelRenderer.executeOitWaterMask and CloudRenderer.renderOit.
                    let mask = |pass: &mut wgpu::RenderPass<'_>| {
                        pass.set_pipeline(&self.water_mask_pipeline);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        pass.draw_indexed(
                            0..water_mask_quads * 6,
                            ((entity_quads
                                + text_quads
                                + crumbling_quads
                                + translucent_crumbling_quads)
                                * 4) as i32,
                            0..1,
                        );
                    };
                    let clouds =
                        |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
                            pass.set_pipeline(pipeline);
                            pass.set_bind_group(0, &self.entity_group, &[]);
                            pass.set_bind_group(1, &self.cloud_group, &[]);
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
                            pass.draw_indexed(0..cloud_quads * 6, 0, 0..1);
                        };
                    // "Features, World Border, Weather": what follows the opaque
                    // world in the classic pass, one stage at a time.
                    let features = |pass: &mut wgpu::RenderPass<'_>, stage: oit::Stage| {
                        let f = oit.features();
                        let s = stage as usize;
                        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        // Quads of a material, `first` quads into the buffer.
                        let quads = |pass: &mut wgpu::RenderPass<'_>,
                                     pipeline: &wgpu::RenderPipeline,
                                     count: u32,
                                     first: u32| {
                            if count > 0 {
                                pass.set_pipeline(pipeline);
                                pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                            }
                        };
                        quads(pass, &f.text[s], text_quads, entity_quads);
                        quads(
                            pass,
                            &f.crumbling[s],
                            crumbling_quads,
                            entity_quads + text_quads,
                        );
                        quads(
                            pass,
                            &f.crumbling[s],
                            translucent_crumbling_quads,
                            entity_quads + text_quads + crumbling_quads,
                        );
                        quads(
                            pass,
                            &f.swirl[s],
                            swirl_quads,
                            entity_quads
                                + text_quads
                                + crumbling_quads
                                + translucent_crumbling_quads
                                + water_mask_quads,
                        );
                        quads(
                            pass,
                            &f.shadow[s],
                            shadow_quads,
                            base_quads
                                - translucent_item_quads
                                - translucent_particle_quads
                                - shadow_quads,
                        );
                        quads(
                            pass,
                            &f.entity[s],
                            translucent_particle_quads,
                            base_quads - translucent_item_quads - translucent_particle_quads,
                        );
                        quads(
                            pass,
                            &f.entity[s],
                            translucent_item_quads,
                            base_quads - translucent_item_quads,
                        );
                        // Translucent, emissive, eyes and breeze wind models:
                        // the same fragment, their light and shading are in
                        // the vertices.
                        quads(pass, &f.entity[s], oit_entity_quads, base_quads);
                        if !frame.lightning.bolts.is_empty() {
                            pass.set_pipeline(&f.lightning[s]);
                            self.lightning.draw(pass, &frame.lightning);
                            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        }
                        quads(pass, &f.additive[s], additive_quads, world_quads);
                        self.weather
                            .draw_with(pass, &f.weather[s], &self.entity_group);
                        if let Some(b) = frame.border.as_ref().filter(|_| !frame.title) {
                            pass.set_pipeline(&f.border[s]);
                            pass.set_bind_group(0, &self.border_group, &[]);
                            pass.set_vertex_buffer(0, self.border_vertices.slice(..));
                            for &side in &b.sides {
                                let first = side as u32 * 6;
                                pass.draw_indexed(first..first + 6, 0, 0..1);
                            }
                        }
                    };
                    let extras = oit::Extras {
                        water_mask: (water_mask_quads > 0).then_some(&mask as _),
                        clouds: (cloud_quads > 0).then_some(&clouds as _),
                        features: Some(&features),
                    };
                    // The draw closure below borrows all of `self`, so the timer is taken out.
                    let mut own_timing = self.own_timing.take();
                    oit.run(
                        &mut encoder,
                        world_view,
                        world_depth,
                        &|pass, stage| {
                            pass.set_bind_group(0, &self.group, &[]);
                            // Flat section draws take the flat pipelines.
                            if let Some((flat, pipeline)) = flat.zip(oit.flat(stage)) {
                                pass.set_pipeline(pipeline);
                                flat.draw(pass, flat.translucent_calls(), |p| self.arena.buffer(p));
                                return;
                            }
                            pass.set_pipeline(oit.terrain(stage));
                            pass.set_index_buffer(
                                self.indices.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            self.draw_sections(
                                pass,
                                &self.indirect,
                                0,
                                solid_draws,
                                translucent_draws,
                            );
                        },
                        &extras,
                        own_timing.as_mut(),
                    );
                    self.own_timing = own_timing;
                    pass = world_pass_after_culling(
                        &mut encoder,
                        world_view,
                        world_depth,
                        true,
                        self.own_timing.as_mut(),
                        "World after OIT",
                    );
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    // HDR output's mark on what shines in the stages, as the classic
                    // pass marks it after drawing it: eyes, the dragon's eyes and rays,
                    // firework sparks and lightning.
                    if self.hdr.is_some() {
                        pass.set_pipeline(&self.hdr_mark_pipeline);
                        pass.set_bind_group(0, &self.entity_group, &[]);
                        pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                        for (quads, first) in [
                            (oit_entity_quads, base_quads),
                            (additive_quads, world_quads),
                            (translucent_item_quads, base_quads - translucent_item_quads),
                            (
                                translucent_particle_quads,
                                base_quads - translucent_item_quads - translucent_particle_quads,
                            ),
                        ] {
                            if quads > 0 {
                                pass.draw_indexed(0..quads * 6, (first * 4) as i32, 0..1);
                            }
                        }
                        if !frame.lightning.bolts.is_empty() {
                            pass.set_pipeline(&self.hdr_mark_lightning_pipeline);
                            self.lightning.draw(&mut pass, &frame.lightning);
                        }
                        pass.set_bind_group(0, &self.group, &[]);
                    }
                }
                Some(flat) => {
                    pass.set_pipeline(pack_flat.map_or(&flat.translucent, |p| &p.1));
                    flat.draw(&mut pass, flat.translucent_calls(), |p| {
                        self.arena.buffer(p)
                    });
                    pass.set_bind_group(0, &self.group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                None => {
                    pass.set_pipeline(translucent_pipeline);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    self.draw_sections(
                        &mut pass,
                        &self.indirect,
                        0,
                        solid_draws,
                        translucent_draws,
                    );
                }
            }
            // LevelRenderer.executeTranslucentAfterTerrain: vanilla's
            // translucent particle layer draws after translucent terrain —
            // the quads write depth, so before it they would hide the glass
            // or water behind them.
            if translucent_particle_quads > 0 && !oit_on {
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_particle_quads * 6,
                    ((base_quads - translucent_item_quads - translucent_particle_quads) * 4) as i32,
                    0..1,
                );
                if self.hdr.is_some() {
                    // Fireworks: their sparks are translucent-layer particles.
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(
                        0..translucent_particle_quads * 6,
                        ((base_quads - translucent_item_quads - translucent_particle_quads) * 4)
                            as i32,
                        0..1,
                    );
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // LevelRenderer.executeClassicTransparency: translucent targets'
            // outlines follow terrain, so its blending cannot wash them out.
            if outline_edges > 0 && frame.outline_after_terrain {
                self.draw_outline(&mut pass, outline_edges as u32, frame.outline_high_contrast);
            }
            if translucent_crumbling_quads > 0 && !oit_on {
                pass.set_pipeline(&self.crumbling_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..translucent_crumbling_quads * 6,
                    ((entity_quads + text_quads + crumbling_quads) * 4) as i32,
                    0..1,
                );
            }
            if cloud_quads > 0 && !pack_draws_clouds && !oit_on {
                pass.set_pipeline(&self.cloud_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_bind_group(1, &self.cloud_group, &[]);
                pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
                pass.draw_indexed(0..cloud_quads * 6, 0, 0..1);
            }
            // LevelRenderer.executeClassicTransparency: weather after translucent
            // terrain and clouds, so the water behind the rain is drawn first.
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            if !pack_draws_weather && !oit_on {
                let native_weather = self
                    .native
                    .as_ref()
                    .filter(|_| !frame.title)
                    .and_then(|n| Some((n.weather.as_ref()?, &n.frame.group)));
                self.weather
                    .draw(&mut pass, &self.entity_group, native_weather);
                pass.set_bind_group(0, &self.group, &[]);
            }
            // LevelRenderer: the world border after translucent terrain and weather.
            if let Some(b) = frame.border.as_ref().filter(|_| !frame.title && !oit_on) {
                pass.set_pipeline(&self.border_pipeline);
                pass.set_bind_group(0, &self.border_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.border_vertices.slice(..));
                for &side in &b.sides {
                    let first = side as u32 * 6;
                    pass.draw_indexed(first..first + 6, 0, 0..1);
                }
                pass.set_bind_group(0, &self.group, &[]);
            }
            // LevelRenderer.executeSeeThrough, after the rest of the world.
            if see_through_quads > 0 {
                pass.set_pipeline(&self.see_through_pipeline);
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                pass.draw_indexed(
                    0..see_through_quads * 6,
                    ((world_quads + additive_quads + hand_quads) * 4) as i32,
                    0..1,
                );
            }
            if hand_in_world {
                pass.set_pipeline(&self.depth_reset);
                pass.set_bind_group(0, &self.group, &[]);
                pass.draw(0..3, 0..1);
                self.draw_hand(&mut pass, hand_quads, world_quads + additive_quads);
            }
        }
        if let Some((color, depth)) = &scaled
            && pack_post
            && let Some((dimension, Some(mut post))) = self.pack_post.take()
        {
            let size = self.world_post.size().unwrap_or([1, 1]);
            let weather = match &self.hud.atlas_texture {
                Some(atlas) if pack_draws_weather => self
                    .pack_weather
                    .get_or_insert_with(|| pack_weather::PackWeather::new(&self.device))
                    .prepare(
                        &self.device,
                        &mut encoder,
                        &mut self.uploads,
                        atlas,
                        frame.weather,
                        frame.weather_time,
                        &self.indices,
                    ),
                _ => None,
            };
            post.set_weather(weather);
            let flat = self.flat.as_ref().filter(|_| flat_frame);
            let pages: Vec<(u32, wgpu::Buffer)> = match flat {
                Some(_) if pack_draws_terrain || pack_draws_shadow => self
                    .arena
                    .page_buffers()
                    .map(|(p, b)| (p, b.clone()))
                    .collect(),
                _ => Vec::new(),
            };
            let mut terrain =
                |rp: &mut wgpu::RenderPass<'_>,
                 list: pack_post::TerrainList,
                 page: &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
                 geometry: Option<(u32, &wgpu::BindGroupLayout)>| {
                    use pack_post::TerrainList as L;
                    let shadow = matches!(list, L::ShadowSolid | L::ShadowTranslucent);
                    if !(if shadow {
                        pack_draws_shadow
                    } else {
                        pack_draws_terrain
                    }) {
                        return;
                    }
                    match flat {
                        Some(flat) => {
                            let [shadow, shadow_translucent] = flat.shadow_calls;
                            let calls = match list {
                                L::Solid => &flat.calls[..flat.solid_calls],
                                L::Translucent => flat.translucent_calls(),
                                L::ShadowSolid => &flat.calls[shadow..shadow_translucent],
                                L::ShadowTranslucent => &flat.calls[shadow_translucent..],
                            };
                            match geometry {
                                Some((instances, layout)) => {
                                    let indices =
                                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some("Shader pack geometry indices"),
                                            layout,
                                            entries: &[wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: flat.indices.as_entire_binding(),
                                            }],
                                        });
                                    rp.set_bind_group(5, &indices, &[]);
                                    let vertices = |p| self.arena.buffer(p);
                                    flat.draw_geometry(rp, calls, vertices, page, instances);
                                }
                                None => flat.draw_with(rp, calls, |p| self.arena.buffer(p), page),
                            }
                        }
                        // A geometry stage reads flat draws' indices; without them it draws nothing.
                        None if geometry.is_some() => {}
                        None => {
                            rp.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                            rp.set_vertex_buffer(1, self.instances.slice(..));
                            let (first, count) = match list {
                                L::Solid => (0, solid_draws),
                                L::Translucent => (solid_draws, translucent_draws),
                                L::ShadowSolid => (shadow_first, shadow_solid),
                                L::ShadowTranslucent => {
                                    (shadow_first + shadow_solid, shadow_translucent)
                                }
                            };
                            self.draw_sections(rp, &self.indirect, 0, first, count);
                        }
                    }
                };
            post.set_clouds(pack_draws_clouds.then(|| pack_post::Clouds {
                vertices: self.cloud_buffer.clone(),
                group: self.cloud_group.clone(),
                indices: self.indices.clone(),
                quads: cloud_quads,
            }));
            post.run(
                &self.device,
                &self.queue,
                &mut encoder,
                color,
                depth,
                &view_target,
                frame,
                &frame.pack,
                size,
                [self.config.width, self.config.height],
                &pages,
                &mut terrain,
            );
            self.pack_post = Some((dimension, Some(post)));
        } else if let Some((color, depth)) = &scaled
            && native_post
            && let Some(native) = &mut self.native
            && let Some(post) = &mut native.post
        {
            let size = self.world_post.size().unwrap_or([1, 1]);
            let history_reset = native.frame.take_history_reset();
            post.run(
                &self.device,
                &mut encoder,
                (color, depth, &native.frame.surface_view),
                &view_target,
                size,
                history_reset,
                native.timing.as_mut(),
            );
        } else if let (Some((_, generated)), Some((_, depth))) = (&fg_frames, &scaled) {
            let world = self
                .frame_gen
                .world(&self.device, &self.world_post, &self.config);
            self.world_post
                .finish(&self.queue, &mut encoder, &world, self.own_timing.as_mut());
            let camera = frame_gen::Camera {
                jittered,
                view_proj,
                depth: depth.clone(),
                render_size: self.world_post.size().unwrap_or(size),
                jitter: self.world_post.jitter_pixels(),
                reproject: self.frame_gen.camera_motion(view_proj, frame.camera),
                near: NEAR_PLANE,
                far: frame.depth_far,
                fov_y: frame.fov_degrees.to_radians(),
            };
            let generated_view = generated.create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.config.format.remove_srgb_suffix()),
                ..Default::default()
            });
            fg_made = self.frame_gen.generate(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.world_post,
                &view_target,
                &generated_view,
                &camera,
            );
            if fg_made {
                let overlay = Overlay {
                    hand_quads: hand_quads * u32::from(!pack_draws_hand),
                    first_hand_vertex: made_hand_vertex,
                    crosshair,
                    first_crosshair: debug_depth + debug_top,
                    world_depth: false,
                    real: false,
                };
                self.draw_overlay(&mut encoder, frame, &generated_view, generated, &overlay);
            }
        } else if scaled.is_some() {
            self.world_post.finish(
                &self.queue,
                &mut encoder,
                &view_target,
                self.own_timing.as_mut(),
            );
        }
        if !frame.title {
            let (depth, depth_size) = match &scaled {
                Some((_, depth)) => (depth, self.world_post.size().unwrap_or(size)),
                None => (&self.depth, size),
            };
            native_render::native_world_drawn(&mut native_render::WorldDrawn {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut encoder,
                color: &target_texture,
                color_view: &view_target,
                color_format: self.config.format.remove_srgb_suffix(),
                size,
                depth,
                depth_size,
                view_proj,
                jittered,
                camera: frame.camera,
                entities: frame.entities,
                spans: frame.entity_spans,
                atlas: self.hud.atlas_texture.as_ref(),
                quad_indices: &self.indices,
            });
        }
        let overlay = Overlay {
            hand_quads: hand_quads * u32::from(!pack_draws_hand && !hand_in_world),
            first_hand_vertex: ((world_quads + additive_quads) * 4) as i32,
            crosshair,
            first_crosshair: debug_depth + debug_top,
            world_depth: scaled.is_none(),
            real: true,
        };
        let readback =
            self.draw_overlay(&mut encoder, frame, &view_target, &target_texture, &overlay);
        if culling && let Some(cull) = &mut self.cull {
            cull.end(&self.device, &mut encoder);
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.end(
                &self.device,
                &mut encoder,
                (&view_target, self.config.format.remove_srgb_suffix()),
            );
        }
        laps.lap("r_passes");
        // Under `HORIZON_FRAMEGEN_DEBUG=generated` a screenshot is the made frame.
        let shown_texture = match &fg_frames {
            Some((_, generated)) if fg_made && frame_gen::generated_only() => generated,
            _ => &target_texture,
        };
        let readback = readback.or_else(|| {
            self.screenshot
                .take()
                .and_then(|request| self.take_readback(request, &mut encoder, shown_texture))
        });
        self.uploads.finish(&encoder);
        if has_section_copies {
            self.section_copy_bytes = 0;
            self.section_uploads.finish(&encoder);
        }
        if let Some(f) = &surface_frame {
            self.readback
                .present(&self.device, &mut encoder, &f.texture);
        }
        if let (Some(hdr), Some(f)) = (&self.hdr, &surface_frame) {
            hdr.present(
                &self.queue,
                &mut encoder,
                &f.texture,
                !frame.title,
                self.own_timing.as_mut(),
            );
        }
        // With HDR on, a full-frame screenshot gets an HDR10 copy beside it.
        let hdr_readback = readback
            .as_ref()
            .filter(|(request, _)| request.size.is_none())
            .zip(self.hdr.as_ref())
            .and_then(|(_, hdr)| {
                let texture = hdr.capture(&self.device, &self.queue, &mut encoder, !frame.title)?;
                let size = [self.config.width, self.config.height];
                Some(copy_frame(&self.device, &mut encoder, &texture, size))
            });
        let first_frame = self.first_frame == FirstFrame::Submit;
        // Finished, submitted and presented on the submit thread when nothing after the
        // submission needs this one: screenshots' readbacks, the error scopes (one per
        // thread), the first frame's startup log, VSync's frame callback, a reconfigure.
        // Frame generation presents its own frames (`frame_gen.present` below).
        let (encoder, surface_frame, after) = if readback.is_none()
            && fg_frames.is_none()
            && hdr_readback.is_none()
            && checks.is_none()
            && !first_frame
            && !reconfigure
            && self.hdr_marker.is_none()
            && (surface_frame.is_none() || self.config.present_mode != wgpu::PresentMode::Fifo)
        {
            let offscreen = surface_frame.is_none();
            // Culling reads its results back as soon as the frame is submitted, as here.
            let after = self
                .cull
                .as_mut()
                .and_then(|c| c.submitted())
                .map(|f| Box::new(f) as submit::AfterSubmit);
            let job = submit::Job {
                encoder,
                surface: surface_frame,
                after,
            };
            match self.queue.submit_later(job) {
                Ok(()) => {
                    self.deferred = Some(offscreen);
                    // The frame's waits for the previous one on the submit thread.
                    self.last_wait += self.queue.take_waited();
                    self.last_encode = started.elapsed().saturating_sub(self.last_wait);
                    self.visible = visible;
                    self.visible_lods = visible_lods;
                    self.shadow_visible = shadow_visible;
                    return Ok(true);
                }
                Err(back) => (back.encoder, back.surface, back.after),
            }
        } else {
            (encoder, surface_frame, None)
        };
        let commands = crate::spans::time("encode", || encoder.finish());
        if first_frame {
            startup_step(format_args!("submitting the first frame"));
            self.first_frame = FirstFrame::Drawn;
        }
        let submission = crate::spans::time("submit", || self.queue.submit([commands]));
        if let Some(read) = after {
            read();
        } else if let Some(read) = self.cull.as_mut().and_then(|c| c.submitted()) {
            read();
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.submitted(&self.queue);
            timer.collect(&self.device, &mut self.gpu_times);
        }
        if let Some(Some(timer)) = Self::pass_timer(&mut self.pack_post, &mut self.native) {
            timer.submitted(&self.queue);
        }
        if let Some(timer) = &mut self.own_timing {
            timer.submitted(&self.queue);
        }
        if let Some((_, Some(post))) = &mut self.pack_post {
            post.submitted();
        }
        let failed = checks.and_then(GpuChecks::pop);
        if let Some((request, buffer)) = readback {
            // Out of memory on the way: the PNG would be black; taken again at half the size.
            if let Some(e) = failed {
                log::warn!(
                    "Horizon renderer: a {}x{} screenshot failed ({e}); trying half the size",
                    self.config.width,
                    self.config.height
                );
                self.screenshot_halvings += 1;
                self.screenshot = Some(request);
            } else {
                self.screenshot_halvings = 0;
                if let Some(hdr_buffer) = hdr_readback {
                    let hdr_path = hdr_screenshot_path(&request.path);
                    match self.write_hdr_screenshot(&hdr_path, &hdr_buffer) {
                        Ok(()) => log::info!("Saved HDR screenshot to {}", hdr_path.display()),
                        Err(e) => log::warn!(
                            "Horizon renderer: HDR screenshot {}: {e}",
                            hdr_path.display()
                        ),
                    }
                }
                self.screenshot_result = Some(
                    self.write_screenshot(&request, &buffer)
                        .map_err(|e| e.to_string()),
                );
            }
        }
        // The frame's own waits for the submit thread (its queue writes and passes).
        self.last_wait += self.queue.take_waited();
        let present = Instant::now();
        if let Some(surface_frame) = surface_frame {
            // On Wayland this ties RedrawRequested to the compositor's frame
            // callbacks, one per refresh. Only VSync (Fifo) waits for the
            // display: with it off, vanilla's window draws uncapped.
            if self.config.present_mode == wgpu::PresentMode::Fifo {
                self.window.pre_present_notify();
            }
            if first_frame {
                startup_step(format_args!("presenting the first frame"));
            }
            self.queue.present(surface_frame);
            if self.hdr.is_some()
                && let Some(marker) = self.hdr_marker.take()
            {
                let _ = std::fs::remove_file(marker);
                log::info!("Horizon renderer: HDR output presented its first frame");
            }
            if reconfigure {
                self.configure_surface();
            }
        } else if let Some((real, generated)) = fg_frames {
            self.frame_gen.present(real, generated, fg_made);
        }
        if self.surface.is_none() {
            // No swapchain paces offscreen frames: keep two on the GPU at most.
            self.in_flight.push_back(submission);
            if !self.paced_elsewhere {
                self.wait_in_flight();
            }
        }
        self.last_wait += present.elapsed();
        // Inside the present's time already.
        self.queue.take_waited();
        self.last_encode = started.elapsed().saturating_sub(self.last_wait);
        let _ = self.started;
        self.visible = visible;
        self.visible_lods = visible_lods;
        self.shadow_visible = shadow_visible;
        Ok(true)
    }
}
impl Renderer {
    /// What draws over the world, on the real frame and frame generation's made one:
    /// the hand, entity outlines, post effects, screen effects, the debug crosshair and
    /// the HUD. Returns the world icon's readback, taken on the real frame.
    fn draw_overlay(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        view_target: &wgpu::TextureView,
        target_texture: &wgpu::Texture,
        overlay: &Overlay,
    ) -> Option<(ScreenshotRequest, wgpu::Buffer)> {
        // Horizon's own pass timings (#5887) time the real frame's overlay only: a made frame's
        // would count the same passes twice.
        let mut timing = if overlay.real {
            self.own_timing.take()
        } else {
            None
        };
        if let Some(hdr) = &self.hdr {
            hdr.keep_world(encoder, timing.as_mut());
        }
        if overlay.hand_quads > 0 {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("First-person hand"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Hand")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        // HDR output's hand marks test against it (below).
                        store: if self.hdr.is_some() {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.entity_pipeline);
            pass.set_bind_group(0, &self.hand_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(0..overlay.hand_quads * 6, overlay.first_hand_vertex, 0..1);
            drop(pass);
            if let Some(hdr) = &self.hdr {
                hdr.mark_hand(encoder, timing.as_mut());
                if let Some(view) = hdr.hand_light_view() {
                    // The light sources in the hand may shine, the rest of it stays at the
                    // world's brightness: nothing the world kept here shows through it.
                    let attachments = [Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })];
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HDR hand marks"),
                        timestamp_writes: timing.as_mut().and_then(|t| t.writes("HDR hand marks")),
                        color_attachments: &attachments,
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_bind_group(0, &self.hand_group, &[]);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
                    let quads = 0..overlay.hand_quads * 6;
                    pass.set_pipeline(&self.hdr_hand_unmark_pipeline);
                    pass.draw_indexed(quads.clone(), overlay.first_hand_vertex, 0..1);
                    pass.set_pipeline(&self.hdr_mark_pipeline);
                    pass.draw_indexed(quads, overlay.first_hand_vertex, 0..1);
                }
            }
        }
        // GameRenderer.render: LevelRenderer.blitEntityOutline after the level and the hand.
        self.entity_outline.draw(
            encoder,
            view_target,
            &self.entity_group,
            &self.indices,
            timing.as_mut(),
        );
        // The world icon grabs here: after the level and the hand, before
        // the effects and the HUD draw over them (`tryTakeScreenshotIfNeeded`
        // runs right after `renderLevel`).
        let readback = self
            .screenshot
            .take_if(|request| overlay.real && request.size.is_some())
            .and_then(|request| self.take_readback(request, encoder, target_texture));
        // GameRenderer.render: applyPostEffects, after the level and before the GUI.
        if !frame.title
            && !frame.post_effects.effects.is_empty()
            && target_texture
                .usage()
                .contains(wgpu::TextureUsages::COPY_SRC)
        {
            let device = &self.device;
            self.post_effects
                .get_or_insert_with(|| crate::post_chain::PostEffects::new(device))
                .apply(
                    &self.device,
                    &self.queue,
                    encoder,
                    target_texture,
                    view_target,
                    [self.config.width, self.config.height],
                    // The scaled world pass drew its own depth buffer.
                    overlay.world_depth.then_some(&self.depth),
                    &frame.post_effects,
                    timing.as_mut(),
                );
        }
        if !frame.screen.is_empty() {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Screen effects"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Screen effects")),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.screen_buffer.slice(..));
            for (i, quad) in frame.screen.iter().enumerate() {
                let blend = match quad.blend {
                    crate::screen_effects::Blend::Alpha => 0,
                    crate::screen_effects::Blend::Vignette => 4,
                };
                let tint = if quad.world_tint && self.hdr.is_some() {
                    8
                } else {
                    0
                };
                pass.set_pipeline(
                    &self.screen_pipelines[tint + blend + (quad.sampling & 3) as usize],
                );
                pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
            }
            drop(pass);
            // A tint leaves the frame's alpha and goes over the kept world too: light sources
            // under water still show the world, and still shine.
            if let Some(view) = self.hdr.as_ref().and_then(|h| h.hand_light_view())
                && frame.screen.iter().any(|quad| quad.world_tint)
            {
                let attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Screen tints over the kept world"),
                    timestamp_writes: None,
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_bind_group(0, &self.entity_group, &[]);
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, self.screen_buffer.slice(..));
                for (i, quad) in frame.screen.iter().enumerate() {
                    if quad.world_tint {
                        pass.set_pipeline(&self.screen_pipelines[8 + (quad.sampling & 3) as usize]);
                        pass.draw_indexed(0..6, (i * 4) as i32, 0..1);
                    }
                }
            }
        }
        // GameRenderer.render3dHud: the debug crosshair after the hand and
        // the screen effects.
        if overlay.crosshair > 0 {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: view_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d crosshair"),
                timestamp_writes: timing.as_mut().and_then(|t| t.writes("Crosshair")),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.debug_lines_on_top);
            pass.set_bind_group(0, &self.crosshair_group, &[]);
            pass.set_vertex_buffer(0, self.debug_line_buffer.slice(..));
            let first = overlay.first_crosshair;
            pass.draw(0..4, first..first + overlay.crosshair);
        }
        let menu_blur = frame
            .menu_blur
            .filter(|_| self.config.usage.contains(wgpu::TextureUsages::COPY_SRC));
        let blur_at = menu_blur.map_or(0, |_| frame.hud.blur_at);
        self.hud.draw_range(
            encoder,
            view_target,
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            0..blur_at,
            timing.as_mut(),
        );
        if let Some(radius) = menu_blur {
            let format = self.config.format;
            let device = &self.device;
            self.menu_blur
                .get_or_insert_with(|| crate::menu_blur::MenuBlur::new(device, format))
                .apply(
                    &self.device,
                    &self.queue,
                    encoder,
                    target_texture,
                    view_target,
                    [self.config.width, self.config.height],
                    radius,
                    timing.as_mut(),
                );
        } else if let Some(blur) = &mut self.menu_blur {
            blur.release();
        }
        self.hud.draw_range(
            encoder,
            view_target,
            &self.depth,
            frame.hud,
            [self.config.width, self.config.height],
            blur_at..frame.hud.vertices.len() as u32,
            timing.as_mut(),
        );
        if overlay.real {
            self.own_timing = timing;
        }
        readback
    }
}
/// Out-of-memory and validation error scopes, popped together: the first error either caught.
struct GpuChecks {
    memory: wgpu::ErrorScopeGuard,
    validation: wgpu::ErrorScopeGuard,
}
impl GpuChecks {
    fn push(device: &wgpu::Device) -> Self {
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        Self { memory, validation }
    }
    fn pop(self) -> Option<wgpu::Error> {
        let validation = pollster::block_on(self.validation.pop());
        let memory = pollster::block_on(self.memory.pop());
        memory.or(validation)
    }
}
/// How many times the window's size a scaled screenshot is: the Resolution Scale
/// `world_scale` when above 1, times `quality`, as far as a side fits in
/// `max_side` pixels and the readback in `max_buffer` bytes.
fn scaled_screenshot_factor(
    [width, height]: [u32; 2],
    world_scale: f32,
    quality: u32,
    max_side: u32,
    max_buffer: u64,
) -> f32 {
    let wanted = world_scale.max(1.0) * quality.max(1) as f32;
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let side = f64::from(max_side) / w.max(h);
    // padded_row(w * f) * h * f <= (4 * w * f + 256) * h * f <= max_buffer.
    let (a, b) = (4.0 * w * h, 256.0 * h);
    let buffer = ((b * b + 4.0 * a * max_buffer as f64).sqrt() - b) / (2.0 * a);
    wanted.min(side.min(buffer) as f32)
}
/// Bytes per row of a `width`-pixel frame read back at 4 bytes a pixel.
fn padded_row(width: u32) -> u32 {
    (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}
/// Encodes the copy of `texture`'s `size` pixels into a mappable buffer.
pub(crate) fn copy_frame(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    [width, height]: [u32; 2],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Horizon screenshot"),
        size: u64::from(padded_row(width)) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row(width)),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    buffer
}
impl Renderer {
    /// Take in the frame the submit thread submitted: start its readbacks and, offscreen,
    /// keep at most two frames ahead of the GPU, as a frame submitted in place does.
    fn land_frame(&mut self) {
        let Some(offscreen) = self.deferred.take() else {
            return;
        };
        let submission = self.queue.landed();
        if let Some(timer) = &mut self.gpu_timer {
            timer.submitted(&self.queue);
            timer.collect(&self.device, &mut self.gpu_times);
        }
        if let Some(Some(timer)) = Self::pass_timer(&mut self.pack_post, &mut self.native) {
            timer.submitted(&self.queue);
        }
        if offscreen && let Some(submission) = submission {
            self.in_flight.push_back(submission);
            if !self.paced_elsewhere {
                self.wait_frames();
            }
        }
    }
    /// No swapchain paces offscreen frames: keep two on the GPU at most.
    pub fn wait_in_flight(&mut self) {
        self.land_frame();
        self.wait_frames();
    }
    fn wait_frames(&mut self) {
        while self.in_flight.len() > 2 {
            let index = self.in_flight.pop_front();
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: index,
                timeout: Some(std::time::Duration::from_secs(5)),
            });
        }
    }
    /// Take a screenshot request: queue the frame's copy, or report why the
    /// frame cannot be read back.
    fn take_readback(
        &mut self,
        request: ScreenshotRequest,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Option<(ScreenshotRequest, wgpu::Buffer)> {
        // The copy and PNG writer both assume 8-bit pixels; a float surface would make
        // the row stride too small for the copy and abort in validation.
        if self.config.format.block_copy_size(None) == Some(4) {
            let size = [self.config.width, self.config.height];
            Some((request, copy_frame(&self.device, encoder, texture, size)))
        } else {
            self.screenshot_result = Some(Err(format!(
                "screenshots need a 32-bit surface format, not {:?}",
                self.config.format
            )));
            None
        }
    }
    /// The pixels `copy_frame` read back into `buffer`, 4 bytes each, row after row.
    fn read_frame(&self, buffer: &wgpu::Buffer) -> Result<Vec<[u8; 4]>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = sender.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        receiver
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let (width, height, row) = (
            self.config.width as usize,
            self.config.height as usize,
            padded_row(self.config.width) as usize,
        );
        let mut pixels = Vec::with_capacity(width * height);
        {
            let data = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
            for y in 0..height {
                pixels.extend_from_slice(data[y * row..y * row + width * 4].as_chunks::<4>().0);
            }
        }
        buffer.unmap();
        Ok(pixels)
    }
    fn write_screenshot(
        &self,
        request: &ScreenshotRequest,
        buffer: &wgpu::Buffer,
    ) -> Result<[u32; 2]> {
        let bgra = matches!(
            self.config.format.remove_srgb_suffix(),
            wgpu::TextureFormat::Bgra8Unorm
        );
        let mut pixels = Vec::with_capacity((self.config.width * self.config.height * 4) as usize);
        for px in self.read_frame(buffer)? {
            let (r, b) = if bgra { (px[2], px[0]) } else { (px[0], px[2]) };
            pixels.extend_from_slice(&[r, px[1], b, 255]);
        }
        // NativeImage.resizeSubRectTo on takeAutoScreenshot's grab: the icon
        // is the frame's centre square averaged down to its size.
        let (width, height, pixels) = match request.size {
            Some(size) => (
                size[0],
                size[1],
                square_downscale(&pixels, self.config.width, self.config.height, size),
            ),
            None => (self.config.width, self.config.height, pixels),
        };
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(&request.path)?),
            width,
            height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        Ok([width, height])
    }
    /// The HDR10 copy (`Hdr::capture`) as a 16-bit PNG tagged BT.2020 with PQ (cICP, PNG's
    /// third edition), with the peak and paper white as its content light levels (cLLI), and
    /// an ICC profile for the programs that don't read cICP (`hdr_icc`).
    fn write_hdr_screenshot(&self, path: &std::path::Path, buffer: &wgpu::Buffer) -> Result<()> {
        let Some(hdr) = &self.hdr else {
            return Ok(());
        };
        let pixels = hdr10_to_rgb16(&self.read_frame(buffer)?);
        let mut info = png::Info::with_size(self.config.width, self.config.height);
        info.color_type = png::ColorType::Rgb;
        info.bit_depth = png::BitDepth::Sixteen;
        info.icc_profile = Some(hdr_icc::profile().into());
        let encoder =
            png::Encoder::with_info(std::io::BufWriter::new(std::fs::File::create(path)?), info)?;
        let mut writer = encoder.write_header()?;
        // BT.2020 primaries, PQ, RGB, full range.
        writer.write_chunk(png::chunk::cICP, &[9, 16, 0, 1])?;
        let nits = |n: f32| ((n * 10_000.0) as u32).to_be_bytes();
        let peak = hdr.peak.max(hdr.paper_white);
        writer.write_chunk(
            png::chunk::cLLI,
            &[nits(peak), nits(hdr.paper_white.min(peak))].concat(),
        )?;
        writer.write_image_data(&pixels)?;
        Ok(())
    }
}
/// `screenshots/2026-10-28_12.00.00.png` -> `screenshots/2026-10-28_12.00.00-hdr.png`.
fn hdr_screenshot_path(path: &std::path::Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}-hdr.png"))
}
/// `Rgb10a2Unorm` pixels as 16-bit RGB, big-endian as PNG stores it: each 10-bit value
/// with its top bits repeated below, so 0 stays 0 and 1023 becomes 65535.
fn hdr10_to_rgb16(pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 6);
    for &px in pixels {
        let v = u32::from_le_bytes(px);
        for shift in [0, 10, 20] {
            let c = (v >> shift & 0x3ff) as u16;
            out.extend_from_slice(&(c << 6 | c >> 4).to_be_bytes());
        }
    }
    out
}
/// Centre-crop the frame to a square and average it down to `size`.
pub(crate) fn square_downscale(pixels: &[u8], width: u32, height: u32, size: [u32; 2]) -> Vec<u8> {
    let (side, cx, cy) = if width > height {
        (height, (width - height) / 2, 0)
    } else {
        (width, 0, (height - width) / 2)
    };
    let [tw, th] = size;
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = cy + ty * side / th;
        let y1 = cy + (ty + 1) * side / th;
        for tx in 0..tw {
            let x0 = cx + tx * side / tw;
            let x1 = cx + (tx + 1) * side / tw;
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = ((y * width + x) * 4) as usize;
                    r += u32::from(pixels[p]);
                    g += u32::from(pixels[p + 1]);
                    b += u32::from(pixels[p + 2]);
                }
            }
            let count = (x1 - x0) * (y1 - y0);
            let o = ((ty * tw + tx) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[
                (r / count) as u8,
                (g / count) as u8,
                (b / count) as u8,
                255,
            ]);
        }
    }
    out
}
/// The union of two frusta (the camera's and its water mirror's), for `Resident::visible`.
#[derive(Clone, Copy, Debug)]
pub struct Frusta(pub Frustum, pub Option<Frustum>);
/// What `Resident::visible` culls with: a frustum or a union of them.
pub trait Cull {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool;
    fn contains(&self, min: Vec3, max: Vec3) -> bool;
}
impl Cull for Frustum {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::intersects(self, min, max)
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::contains(self, min, max)
    }
}
impl Cull for Frusta {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.0.intersects(min, max) || self.1.is_some_and(|f| f.intersects(min, max))
    }
    /// Entirely inside one of them is entirely inside the union.
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.0.contains(min, max) || self.1.is_some_and(|f| f.contains(min, max))
    }
}
/// Camera-relative clip planes (`Frustum`).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}
impl Frustum {
    /// Whether the camera-relative box `min..max` is at least partly inside.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        !self.planes.iter().any(|p| {
            let v = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            p.truncate().dot(v) + p.w < 0.0
        })
    }
    /// Whether the camera-relative box `min..max` is entirely inside. Then
    /// `intersects` accepts every box within it: each plane's distance only
    /// grows from this box's nearest corner to the inner box's farthest, and
    /// rounding keeps that order.
    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let v = Vec3::new(
                if p.x >= 0.0 { min.x } else { max.x },
                if p.y >= 0.0 { min.y } else { max.y },
                if p.z >= 0.0 { min.z } else { max.z },
            );
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}
/// Vanilla's `Projection`: reversed depth (near maps to 1, far to 0, tests
/// are GREATER_THAN_OR_EQUAL), which keeps far faces a hair apart, like
/// fire_side's 0.01 pixel in front of its block, from z-fighting.
/// The section the camera is in.
fn camera_section(camera: DVec3, min_y: i32) -> SectionKey {
    (
        (camera.x.floor() as i32) >> 4,
        (camera.y.floor() as i32 - min_y) >> 4,
        (camera.z.floor() as i32) >> 4,
    )
}

/// The near plane of the world's projection, in blocks.
const NEAR_PLANE: f32 = 0.05;

/// Counts and offsets `Renderer::draw_overlay` draws from this frame's buffers.
struct Overlay {
    /// Zero when a shader pack draws the hand.
    hand_quads: u32,
    first_hand_vertex: i32,
    crosshair: u32,
    first_crosshair: u32,
    /// The world drew into `depth` (post effects may read it), not a scaled target's.
    world_depth: bool,
    /// The real frame, not frame generation's made one: the world icon grabs it.
    real: bool,
}

fn world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_degrees.to_radians(), aspect, far, NEAR_PLANE)
}

/// `view_proj` mirrored about the horizontal plane at the camera-relative height `plane`:
/// with F the mirror (y flips and moves to 2 * plane), `view_proj * F` negates column 1
/// and moves column 3 by 2 * plane times column 1.
///
/// `mirror_inverse_view` is the inverse's other side: F * `inverse_view`.
fn mirror_view_projection(view_proj: Mat4, plane: f32) -> Mat4 {
    let mut m = view_proj.to_cols_array_2d();
    let col1 = m[1];
    m[1] = col1.map(|v| -v);
    m[3] = std::array::from_fn(|i| m[3][i] + 2.0 * plane * col1[i]);
    Mat4::from_cols_array_2d(&m)
}

/// The mirror's `inverse_view` (the sky's rays): what the view sees, mirrored about the
/// horizontal plane at the camera-relative height `plane`, so F * `inverse_view`.
fn mirror_inverse_view(inverse_view: [[f32; 4]; 4], plane: f32) -> [[f32; 4]; 4] {
    inverse_view.map(|[x, y, z, w]| [x, 2.0 * plane * w - y, z, w])
}
/// Where a perspective `view_proj` sees from, in its own (camera-relative)
/// space: the point whose clip x, y and w are all 0. `None` for a
/// projection without one (orthographic).
fn projection_center(view_proj: Mat4) -> Option<Vec3> {
    let m = view_proj.as_dmat4();
    let rows = [m.row(0), m.row(1), m.row(3)];
    let a = glam::DMat3::from_cols(
        DVec3::new(rows[0].x, rows[1].x, rows[2].x),
        DVec3::new(rows[0].y, rows[1].y, rows[2].y),
        DVec3::new(rows[0].z, rows[1].z, rows[2].z),
    );
    let det = a.determinant();
    if !det.is_normal() {
        return None;
    }
    let b = DVec3::new(rows[0].w, rows[1].w, rows[2].w);
    Some((a.inverse() * -b).as_vec3())
}
/// Calls `draw(first, quads)` for each run of a mesh's solid quads that can
/// face `eye` (`projection_center`), the mesh at the camera-relative
/// `origin`; all of them when there is no eye.
fn facing_runs(sides: &Sides, eye: Option<Vec3>, origin: [f32; 3], draw: impl FnMut(u32, u32)) {
    facing_runs_from(sides, eye, None, origin, Quads::All, draw)
}
/// `facing_runs` for the eye and, with `low`, every eye straight below it down to that
/// camera-relative height (the water mirror's eye).
fn facing_runs_from(
    sides: &Sides,
    eye: Option<Vec3>,
    low: Option<f32>,
    origin: [f32; 3],
    part: Quads,
    draw: impl FnMut(u32, u32),
) {
    match eye {
        Some(eye) => {
            let eye = eye - Vec3::from(origin);
            let low = low.map_or(eye.y, |y| (y - origin[1]).min(eye.y));
            sides.visible_from(eye.to_array(), low, part, draw)
        }
        None => sides.every(part, draw),
    }
}
fn frustum(m: Mat4) -> [Vec4; 6] {
    let r = |i| m.row(i);
    let planes = [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(2),
        r(3) - r(2),
    ];
    planes.map(|p| p / p.truncate().length())
}

#[cfg(test)]
mod tests {
    fn adapter(device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "test adapter".into(),
            vendor: 0,
            device: 0,
            device_type,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        }
    }

    #[test]
    fn software_occlusion_culling_is_opt_in() {
        let cpu = adapter(wgpu::DeviceType::Cpu);
        let discrete = adapter(wgpu::DeviceType::DiscreteGpu);
        assert!(!super::gpu_culling_requested(&cpu, None));
        assert!(super::gpu_culling_requested(&discrete, None));
        assert!(super::gpu_culling_requested(&cpu, Some("all")));
        assert!(!super::gpu_culling_requested(&discrete, Some("0")));
    }

    /// Translucent Batching is offered with indirect multi-draws that aren't OpenGL's,
    /// and only where frames without a shader pack never draw flat.
    #[test]
    fn translucent_batching_is_offered_where_vanilla_frames_never_draw_flat() {
        let vulkan = adapter(wgpu::DeviceType::DiscreteGpu);
        let mut gl = adapter(wgpu::DeviceType::DiscreteGpu);
        gl.backend = wgpu::Backend::Gl;
        let batchable = super::translucent_batchable;
        // No flat draws, or flat draws for a shader pack's terrain alone.
        assert!(batchable(&vulkan, true, None));
        assert!(batchable(&vulkan, true, Some(usize::MAX)));
        // Vanilla frames drawn flat past MIN_SECTIONS (Metal), or all of them.
        assert!(!batchable(&vulkan, true, Some(super::flat::MIN_SECTIONS)));
        assert!(!batchable(&vulkan, true, Some(0)));
        assert!(!batchable(&vulkan, false, None));
        assert!(!batchable(&gl, true, None));
        assert!(!batchable(&gl, true, Some(usize::MAX)));
    }

    /// The pack-flat rule: with Translucent Batching translucent quads have an arena of
    /// their own, unless a shader pack's terrain programs or its shadow pass alone can
    /// draw flat (a pack picked and not loaded yet counts as drawing terrain).
    #[test]
    fn translucent_quads_stay_in_the_section_arena_while_a_pack_can_draw_flat() {
        // (batching, flat draws, the pack's terrain, its shadow pass)
        let split = super::translucent_split;
        assert!(split(true, true, false, false));
        assert!(!split(true, true, true, false));
        assert!(!split(true, true, false, true));
        assert!(!split(true, true, true, true));
        // Without flat draws, or with them off, the pack draws per section, from either.
        assert!(split(true, false, true, true));
        assert!(!split(false, true, false, false));
        assert!(!split(false, false, false, false));
    }

    /// No large translucent page starts to shrink until `SHRINK_SETTLE` after terrain
    /// last arrived, nor in a frame `compact_arena` copied in; terrain arriving after
    /// a try that found no room does not cut its `SHRINK_RETRY` short. Only starts
    /// wait: a move under way finishes, or its page would stay closed while the new
    /// one fills.
    #[test]
    fn shrinking_starts_once_terrain_has_settled() {
        use super::{COMPACT_BUDGET, SHRINK_RETRY, SHRINK_SETTLE, ShrinkGate};
        use std::time::{Duration, Instant};
        let (now, ms) = (Instant::now(), Duration::from_millis(1));
        let mut gate = ShrinkGate::default();
        assert!(gate.may_start(now, COMPACT_BUDGET));
        assert!(!gate.may_start(now, COMPACT_BUDGET - 1));
        gate.wait(now, SHRINK_SETTLE);
        assert!(!gate.may_start(now + SHRINK_SETTLE - ms, COMPACT_BUDGET));
        assert!(gate.may_start(now + SHRINK_SETTLE, COMPACT_BUDGET));
        gate.wait(now, SHRINK_RETRY);
        gate.wait(now + ms, SHRINK_SETTLE);
        assert!(!gate.may_start(now + SHRINK_RETRY - ms, COMPACT_BUDGET));
        assert!(gate.may_start(now + SHRINK_RETRY, COMPACT_BUDGET));
    }

    #[test]
    fn the_mirror_sees_each_point_where_its_reflection_is() {
        use glam::{Mat4, Vec3, Vec4};
        let view_proj = super::world_projection(70.0, 1.5, 512.0)
            * Mat4::look_to_rh(Vec3::ZERO, Vec3::new(0.3, -0.4, -1.0), Vec3::Y);
        let plane = -3.25;
        let mirror = super::mirror_view_projection(view_proj, plane);
        for p in [Vec3::new(1.0, 2.0, -9.0), Vec3::new(-4.0, -1.5, -20.0)] {
            let reflected = Vec3::new(p.x, 2.0 * plane - p.y, p.z);
            let a = mirror * Vec4::from((p, 1.0));
            let b = view_proj * Vec4::from((reflected, 1.0));
            assert!((a - b).abs().max_element() < 1e-4, "{a} {b}");
        }
    }
    #[test]
    fn the_mirrors_sky_rays_are_the_cameras_reflected() {
        use glam::{Mat4, Vec3, Vec4};
        // A camera looking down: its mirror looks up, by as much.
        let forward = Vec3::new(0.3, -0.64, -1.0).normalize();
        let view = Mat4::look_to_rh(Vec3::ZERO, forward, Vec3::Y);
        let mirrored = super::mirror_inverse_view(view.inverse().to_cols_array_2d(), -3.25);
        let ray = Mat4::from_cols_array_2d(&mirrored) * Vec4::new(0.0, 0.0, -1.0, 0.0);
        let reflected = Vec3::new(forward.x, -forward.y, forward.z);
        assert!((ray.truncate() - reflected).length() < 1e-5, "{ray}");
        // The view's origin, the camera, goes to the mirror's eye.
        let eye = Mat4::from_cols_array_2d(&mirrored) * Vec4::W;
        assert!(
            (eye.truncate() - Vec3::new(0.0, -6.5, 0.0)).length() < 1e-5,
            "{eye}"
        );
    }
    #[test]
    fn scaled_screenshots_follow_resolution_scale_and_quality_within_limits() {
        use super::scaled_screenshot_factor as factor;
        let big = u64::MAX / 2;
        // The window's size at 100% and 1x; Resolution Scale below 100% stays the window's.
        assert_eq!(factor([1920, 1080], 1.0, 1, 16384, big), 1.0);
        assert_eq!(factor([1920, 1080], 0.5, 1, 16384, big), 1.0);
        // The game's resolution, times the quality.
        assert_eq!(factor([1920, 1080], 2.0, 1, 16384, big), 2.0);
        assert_eq!(factor([1920, 1080], 1.5, 4, 16384, big), 6.0);
        assert_eq!(factor([1920, 1080], 0.5, 8, 16384, big), 8.0);
        // Cut to the largest texture side: 3840 * 4.266 = 16384.
        let f = factor([3840, 2160], 1.0, 8, 16384, big);
        assert!(
            (3840.0 * f) as u32 <= 16384 && (3840.0 * f) as u32 > 16300,
            "{f}"
        );
        // Cut to the largest buffer: 256 MiB holds about 3.8x of 1080p's readback.
        let max = 256 << 20;
        let f = factor([1920, 1080], 1.0, 8, 16384, max);
        let [w, h] = [1920, 1080].map(|s| (s as f32 * f) as u32);
        assert!(u64::from(super::padded_row(w)) * u64::from(h) <= max, "{f}");
        assert!(f > 3.7, "{f}");
    }
    #[test]
    fn drawable_size_skips_minimized_and_oversized_windows() {
        use super::drawable_size;
        assert!(drawable_size(1280, 720, 8192));
        assert!(drawable_size(1, 1, 8192));
        assert!(drawable_size(8192, 8192, 8192));
        // Minimized.
        assert!(!drawable_size(0, 720, 8192));
        assert!(!drawable_size(1280, 0, 8192));
        assert!(!drawable_size(0, 0, 8192));
        // One past the device's largest texture.
        assert!(!drawable_size(8193, 720, 8192));
        assert!(!drawable_size(1280, 8193, 8192));
        // The crash: a window `SetWindowPos` made 65535 pixels tall, on an RTX 5080 (32768).
        assert!(!drawable_size(984, 65496, 32768));
        assert!(!drawable_size(984, u32::MAX, 32768));
    }

    #[test]
    fn a_reported_black_level_rounds_down() {
        use super::hundredths_of_a_nit as h;
        assert_eq!(h(0.005), 0);
        assert_eq!(h(0.05), 5);
        assert_eq!(h(5.0), 100);
        assert_eq!(h(f32::NAN), 0);
    }
    #[test]
    fn os_luminances_become_auto_values() {
        use super::os_reported;
        use crate::graphics::Reported;
        let info = |max, white| wgpu::DisplayHdrInfo {
            luminance: Some(wgpu::DisplayLuminance {
                max_nits: max,
                sdr_white_nits: white,
                ..Default::default()
            }),
            ..Default::default()
        };
        // Windows' SDR content brightness slider runs 80..480 nits.
        assert_eq!(
            os_reported(&info(Some(998.4), Some(240.0))),
            Some(Reported {
                paper_white: 240,
                peak: 998,
                black: 0
            })
        );
        assert_eq!(
            os_reported(&info(Some(120.0), Some(1000.0))),
            Some(Reported {
                paper_white: 480,
                peak: 480,
                black: 0
            })
        );
        // White above the calibrated peak (a Samsung QD-OLED's 380, the slider at 456).
        assert_eq!(
            os_reported(&info(Some(380.0), Some(456.0))),
            Some(Reported {
                paper_white: 456,
                peak: 456,
                black: 0
            })
        );
        assert_eq!(os_reported(&info(Some(1000.0), None)), None);
        assert_eq!(os_reported(&info(Some(f32::NAN), Some(80.0))), None);
        assert_eq!(os_reported(&wgpu::DisplayHdrInfo::default()), None);
    }
    #[test]
    fn surface_is_8_bit_and_not_srgb() {
        use super::surface_format;
        use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba16Float};
        // RADV's Wayland surfaces list the linear fp16 format first; choosing it makes the
        // compositor read gamma-encoded values as linear light.
        assert_eq!(
            surface_format(&[Rgba16Float, Bgra8UnormSrgb, Bgra8Unorm], Bgra8UnormSrgb),
            Bgra8Unorm
        );
        assert_eq!(
            surface_format(&[Bgra8UnormSrgb, Rgba8Unorm], Bgra8UnormSrgb),
            Rgba8Unorm
        );
        assert_eq!(surface_format(&[Rgba16Float], Bgra8UnormSrgb), Rgba16Float);
    }
    /// A Wayland surface on a compositor with colour management, as wgpu lists it (Mesa 26.2
    /// ANV on an Intel HD 620 under weston 16): wide formats first. The pre-#1075 choice,
    /// the first non-sRGB format, took `Rgba16Float` and the window looked washed out, as if
    /// under a white overlay (#1007); XWayland lists no such format, which is why it looked fine.
    #[test]
    fn a_colour_managed_wayland_surface_still_gets_8_bit() {
        use super::surface_format;
        use wgpu::TextureFormat::*;
        let wayland = [
            Rgba16Float,
            Rgba16Unorm,
            Rgb10a2Unorm,
            Bgra8UnormSrgb,
            Bgra8Unorm,
            Rgba8UnormSrgb,
            Rgba8Unorm,
        ];
        assert_eq!(surface_format(&wayland, Bgra8UnormSrgb), Bgra8Unorm);
        // Without Bgra8: Rgba8, still not a 16- or 10-bit format.
        let without_bgra = [
            Rgba16Float,
            Rgba16Unorm,
            Rgb10a2Unorm,
            Rgba8UnormSrgb,
            Rgba8Unorm,
        ];
        assert_eq!(surface_format(&without_bgra, Rgba8UnormSrgb), Rgba8Unorm);
    }
    #[test]
    fn special_material_batches_preserve_quad_order_and_offsets() {
        let quad = |layer, id| {
            [super::Vertex {
                pos: [0.0; 3],
                uv: [0.0; 2],
                layer,
                color: 0xffff_ffff,
                light: id,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            }; 4]
        };
        let entities: Vec<_> = [
            (0, 0),
            (14, 9),
            (4, 1),
            (11, 2),
            (15, 10),
            (7, 3),
            (35, 11),
            (4, 4),
            (12, 7),
        ]
        .into_iter()
        .flat_map(|(layer, id)| quad(layer, id))
        .collect();
        let mut batch = super::EntityBatch::default();
        let counts = batch.update(&entities, &quad(3, 5), &quad(0, 6), &quad(0, 8), false);
        let vertices = &batch.combined;
        assert_eq!(counts, [4, 1, 1, 1, 0, 0, 1, 1, 0, 0]);
        assert_eq!(
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .map(|q| q[0].light)
                .collect::<Vec<_>>(),
            [0, 1, 3, 4, 11, 2, 7, 9, 10, 5, 6, 8]
        );
    }
    #[test]
    fn software_adapters_are_last_resort() {
        use super::is_software;
        assert!(is_software(wgpu::DeviceType::Cpu));
        assert!(!is_software(wgpu::DeviceType::DiscreteGpu));
        assert!(!is_software(wgpu::DeviceType::IntegratedGpu));
        assert!(!is_software(wgpu::DeviceType::VirtualGpu));
    }
    #[test]
    fn auto_tries_vulkan_then_directx_12_then_opengl() {
        use super::{GraphicsApi, backend_order};
        use wgpu::Backends;
        let order = |api, os| backend_order(api, os, false);
        assert_eq!(
            order(GraphicsApi::Auto, "windows"),
            [Backends::VULKAN, Backends::DX12, Backends::GL]
        );
        assert_eq!(order(GraphicsApi::Vulkan, "windows"), [Backends::VULKAN]);
        assert_eq!(order(GraphicsApi::Dx12, "windows"), [Backends::DX12]);
        assert_eq!(order(GraphicsApi::OpenGl, "windows"), [Backends::GL]);
        assert_eq!(
            order(GraphicsApi::Auto, "linux"),
            [Backends::VULKAN, Backends::GL]
        );
        assert_eq!(order(GraphicsApi::Dx12, "linux"), [Backends::VULKAN]);
        assert_eq!(order(GraphicsApi::OpenGl, "linux"), [Backends::GL]);
        // WGPU_BACKEND picks the backends itself; macOS has Metal only.
        for api in GraphicsApi::ALL {
            assert_eq!(backend_order(api, "windows", true), [Backends::default()]);
            assert_eq!(order(api, "macos"), [Backends::default()]);
        }
    }
    /// The second try at a backend shows the adapters wgpu hides for reporting
    /// no Vulkan conformance version (Mesa's hasvk, which drives Ivy Bridge and
    /// Haswell, reports 0.0.0.0 and works).
    #[test]
    fn the_second_try_at_a_backend_shows_nonconformant_adapters() {
        let shown = |allow| {
            super::instance_descriptor(wgpu::Backends::VULKAN, None, allow)
                .flags
                .contains(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER)
        };
        assert!(shown(true));
        // The first try keeps wgpu's own filtering, unless `with_env` turns the
        // flag on from the environment (WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER).
        if std::env::var_os("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER").is_none() {
            assert!(!shown(false));
        }
    }
    /// Every WGSL shader file translates to HLSL as wgpu's DirectX 12 backend
    /// writes it for FXC (shader model 5.1), entry point by entry point.
    #[test]
    fn shaders_translate_to_hlsl_for_directx_12_and_glsl_for_opengl() {
        fn wgsl_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    wgsl_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "wgsl") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        wgsl_files(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
            &mut files,
        );
        assert!(files.len() >= 10, "{files:?}");
        // Improved transparency's files are appended to terrain.wgsl and entity.wgsl.
        let mut sources: Vec<(std::path::PathBuf, String)> = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "oit.wgsl" {
                // The shared maths: it needs the file it is appended to.
                continue;
            } else if name == "oit_world.wgsl" {
                let base = include_str!("terrain.wgsl").to_owned() + include_str!("oit.wgsl");
                sources.push((path, base + &text));
            } else if name == "oit_features.wgsl" {
                let base = super::entity_shader() + include_str!("oit.wgsl");
                sources.push((path, base + &text));
            } else if name == "oit_border.wgsl" {
                let base = include_str!("border.wgsl").to_owned() + include_str!("oit.wgsl");
                sources.push((path, base + &text));
            } else if name == "oit_clouds.wgsl" {
                let base = super::entity_shader() + include_str!("oit.wgsl");
                sources.push((path, base + &text));
            } else {
                sources.push((path, text));
            }
        }
        let mut glsl_checked = 0;
        for (path, text) in sources {
            // entity.wgsl comes after the overlay's constants (entity_shader).
            let module = naga::front::wgsl::parse_str(&text)
                .or_else(|_| {
                    naga::front::wgsl::parse_str(&(crate::overlay::wgsl_constants() + &text))
                })
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            for entry in &module.entry_points {
                let (module, info) = naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((entry.stage, &entry.name)),
                    &Default::default(),
                )
                .unwrap();
                let mut hlsl = String::new();
                naga::back::hlsl::Writer::new(
                    &mut hlsl,
                    &naga::back::hlsl::Options::default(),
                    &naga::back::hlsl::PipelineOptions {
                        entry_point: Some((entry.stage, entry.name.clone())),
                    },
                )
                .write(&module, &info, None)
                .unwrap_or_else(|e| panic!("{} {}: {e}", path.display(), entry.name));
                assert!(!hlsl.is_empty());
                // OpenGL below 4.3 lacks ARB_arrays_of_arrays (#6267): no nested array may reach it.
                // Compute and storage-buffer shaders (culling, frame generation) never run on GL.
                if entry.stage == naga::ShaderStage::Compute
                    || text.contains("var<storage")
                    || text.contains("var<workgroup>")
                {
                    continue;
                }
                for version in [330, 400] {
                    let mut glsl = String::new();
                    let glsl_result = naga::back::glsl::Writer::new(
                        &mut glsl,
                        &module,
                        &info,
                        &naga::back::glsl::Options {
                            version: naga::back::glsl::Version::Desktop(version),
                            ..Default::default()
                        },
                        &naga::back::glsl::PipelineOptions {
                            shader_stage: entry.stage,
                            entry_point: entry.name.clone(),
                            multiview: None,
                        },
                        Default::default(),
                    )
                    .and_then(|mut w| w.write());
                    // Some shaders use features GLSL cannot express (depth textureLoad): GL does
                    // not run them. The ones it does run must translate, checked below.
                    if glsl_result.is_err() {
                        continue;
                    }
                    glsl_checked += 1;
                    assert!(
                        !glsl.contains("ARB_arrays_of_arrays"),
                        "{} {} needs ARB_arrays_of_arrays on GLSL {version}",
                        path.display(),
                        entry.name
                    );
                }
            }
        }
        assert!(glsl_checked >= 10, "{glsl_checked} GLSL translations");
    }
    #[test]
    fn gl_sprite_arrays_are_neither_2d_textures_nor_cube_maps() {
        for (count, gl) in [(1, 2), (2, 2), (5, 5), (6, 7), (12, 13), (2048, 2048)] {
            assert_eq!(super::array_layers(count, true), gl);
            assert_eq!(super::array_layers(count, false), count);
        }
    }
    #[test]
    fn entity_shader_validates_with_its_overlay_constants() {
        let module = naga::front::wgsl::parse_str(&super::entity_shader()).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    fn chunk_fade_is_vanillas_get_visibility() {
        use super::{chunk_visibility, fade_millis};
        assert_eq!(fade_millis(0.75), 750);
        assert_eq!(fade_millis(0.0), 0);
        assert_eq!(chunk_visibility(0, 750), 0.0_f32);
        assert_eq!(chunk_visibility(375, 750), 0.5);
        assert_eq!(chunk_visibility(750, 750), 1.0);
        assert_eq!(chunk_visibility(0, 0), 1.0, "Off: whole at once");
        assert_eq!(chunk_visibility(u64::MAX, 750), 1.0);
    }
    #[test]
    fn section_instance_carries_the_fade_in_the_step() {
        use super::section_instance;
        let step = 1.0 / crate::mesher::UV_STEPS;
        // Fully visible (the option Off) is exactly the record from before the fade.
        assert_eq!(
            section_instance([1.0, 2.0, 3.0], 0, 1.0),
            [1.0, 2.0, 3.0, step]
        );
        for visibility in [0.0_f32, 0.25, 0.5, 0.999] {
            let w = section_instance([0.0; 3], 0, visibility)[3];
            let bits = w.to_bits();
            assert_eq!(f32::from_bits(bits & 0xff80_0000), step);
            let read = 1.0 - (bits & 0x7f_ffff) as f32 / 8_388_608.0;
            assert!((read - visibility).abs() < 1e-6, "{visibility}: {read}");
        }
        // A section fading in over far terrain keeps its dither code unchanged.
        assert_eq!(section_instance([0.0; 3], 7, 0.5)[3], -(7.0 + step));
    }
    #[test]
    fn terrain_attributes_follow_the_terrain_vertex() {
        use super::TerrainVertex;
        use std::mem::offset_of;
        let offsets = super::TERRAIN_ATTRIBUTES.map(|a| (a.shader_location, a.offset as usize));
        assert_eq!(
            offsets,
            [
                (0, offset_of!(TerrainVertex, pos)),
                (1, offset_of!(TerrainVertex, uv)),
                (2, offset_of!(TerrainVertex, layer)),
                (4, offset_of!(TerrainVertex, light)),
                (3, offset_of!(TerrainVertex, color)),
            ]
        );
        assert_eq!(std::mem::size_of::<TerrainVertex>(), 24);
    }
    #[test]
    fn cracks_multiply_instead_of_covering_the_block() {
        let blend = super::CRUMBLING_BLEND;
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::Src);
        assert_eq!(blend.alpha.src_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::Zero);
    }
    #[test]
    fn hdr_screenshots_keep_every_10_bit_step() {
        // Rgb10a2Unorm: red in the low 10 bits, then green, then blue.
        let px = |r: u32, g: u32, b: u32| (r | g << 10 | b << 20 | 3 << 30).to_le_bytes();
        let out = super::hdr10_to_rgb16(&[px(0, 1023, 512)]);
        assert_eq!(out, [0, 0, 0xff, 0xff, 0x80, 0x20]);
        assert_eq!(
            super::hdr_screenshot_path(std::path::Path::new("screenshots/2026-10-28_12.00.00.png")),
            std::path::Path::new("screenshots/2026-10-28_12.00.00-hdr.png")
        );
    }
    #[test]
    fn texture_filtering_samplers() {
        use super::{sampler_anisotropy, sprite_sampler_descriptor};
        // LevelRenderer.chunkLayerSampler: linear both ways, clamped, in every mode.
        for aniso in [1, 8] {
            let d = sprite_sampler_descriptor(aniso);
            assert_eq!(d.mag_filter, wgpu::FilterMode::Linear);
            assert_eq!(d.min_filter, wgpu::FilterMode::Linear);
            assert_eq!(d.mipmap_filter, wgpu::MipmapFilterMode::Linear);
            assert_eq!(d.address_mode_v, wgpu::AddressMode::ClampToEdge);
            assert_eq!(d.anisotropy_clamp, aniso);
        }
        assert_eq!(
            [1, 2, 3].map(sampler_anisotropy),
            [2, 4, 8],
            "Options.maxAnisotropyValue is 1 << bit"
        );
    }
    #[test]
    fn distant_tiles_repeat_their_sprite_under_the_clamped_sampler() {
        // terrain.wgsl's tile_uv, evaluated by naga as a constant expression.
        let source = include_str!("terrain.wgsl");
        let body = source.split("fn tile_uv(").nth(1).unwrap();
        let body = body.split("return ").nth(1).unwrap();
        let expr = &body[..body.find(';').unwrap()];
        let tile_uv = |u: f32, v: f32| -> [f32; 2] {
            let uv = format!("vec2<f32>({u:?}, {v:?})");
            let wgsl = format!("const UV: vec2<f32> = {};", expr.replace("uv", &uv));
            let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
            let (_, constant) = module.constants.iter().next().unwrap();
            let naga::Expression::Compose { components, .. } =
                &module.global_expressions[constant.init]
            else {
                panic!("{wgsl}");
            };
            components
                .iter()
                .map(|&c| match module.global_expressions[c] {
                    naga::Expression::Literal(naga::Literal::F32(x)) => x,
                    ref e => panic!("{e:?}"),
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        };
        // A section's sprite UVs, edges included, stay where the clamped sampler reads them.
        for (u, v) in [(0.0, 1.0), (0.25, 0.75), (1.0, 0.0)] {
            assert_eq!(tile_uv(u, v), [u, v]);
        }
        // A merged face's UV counts blocks: each takes the sprite from its own corner.
        assert_eq!(tile_uv(1.25, 2.0), [0.25, 1.0]);
        assert_eq!(tile_uv(1023.5, 3.75), [0.5, 0.75]);
    }
    #[test]
    fn shader_globals_match_the_rust_layout() {
        // terrain.wgsl and entity.wgsl bind the same buffer: same fields, same order.
        let fields = |source: &str| -> Vec<String> {
            let body = source.split("struct Globals {").nth(1).unwrap();
            body[..body.find('}').unwrap()]
                .lines()
                .filter_map(|l| l.trim().split_once(':').map(|(name, _)| name.to_string()))
                .filter(|name| !name.starts_with("//"))
                .collect()
        };
        let terrain = fields(include_str!("terrain.wgsl"));
        assert_eq!(terrain, fields(include_str!("entity.wgsl")));
        let rust = fields(include_str!("gfx.rs"));
        assert_eq!(terrain, rust);
    }
    #[test]
    fn selection_outline_uses_vanilla_pixel_width() {
        for width in [640, 1280, 1920] {
            assert_eq!(super::outline_width(width), 2.5);
        }
        assert_eq!(super::outline_width(2560), 2560.0 / 1920.0 * 2.5);
        assert_eq!(super::outline_width(3840), 5.0);
    }
    #[test]
    fn the_projection_center_follows_the_view_bob() {
        use glam::{Mat4, Vec3};
        let projection = super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let view = crate::camera::view_matrix(30.0, 20.0);
        let eye = super::projection_center(projection * view).unwrap();
        assert!(eye.length() < 1e-5, "{eye}");
        // A pose translated by the bob: the eye is where the pose's inverse
        // takes the view's origin.
        let bob = Mat4::from_rotation_z(0.02) * Mat4::from_translation(Vec3::new(0.03, -0.08, 0.0));
        let expected = (bob * view).inverse().transform_point3(Vec3::ZERO);
        let eye = super::projection_center(projection * bob * view).unwrap();
        assert!(eye.distance(expected) < 1e-5, "{eye} {expected}");
        assert!(expected.length() > 0.05);
        assert!(
            super::projection_center(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.0, 1.0))
                .is_none()
        );
    }
    #[test]
    fn indirect_draws_are_validated_only_in_debug_builds_and_on_directx_12() {
        let validated = |backends| {
            super::instance_descriptor(backends, None, false)
                .flags
                .contains(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
        };
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none()
            && std::env::var_os("WGPU_BACKEND").is_none()
        {
            assert_eq!(validated(wgpu::Backends::default()), cfg!(debug_assertions));
            assert_eq!(validated(wgpu::Backends::VULKAN), cfg!(debug_assertions));
            assert_eq!(
                validated(wgpu::Backends::DX12),
                cfg!(debug_assertions) || cfg!(windows)
            );
        }
    }
    #[test]
    fn far_faces_a_hair_apart_keep_their_depth_order() {
        let m = super::world_projection(70.0, 16.0 / 9.0, 768.0);
        let depth = |distance: f32| {
            let clip = m * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            clip.z / clip.w
        };
        // fire_side at 150 blocks, 0.01 pixel in front of the block it burns on.
        let (block, fire) = (depth(150.0), depth(150.0 - 0.01 / 16.0));
        assert!(fire > block, "{fire} {block}");
        assert!((0.0..=1.0).contains(&block));
        assert!((depth(0.05) - 1.0).abs() < 1e-6);
    }
    #[test]
    fn selection_outline_has_vanilla_perspective_layering() {
        assert_eq!(
            super::outline_params(1280, 720),
            [1280.0, 720.0, 2.5, 0.99975586]
        );
        assert_eq!(
            super::outline_params(3840, 0),
            [3840.0, 1.0, 5.0, 0.99975586]
        );
    }
    #[test]
    fn selection_outline_preserves_shape_edges_without_inflation() {
        let edges = [[2.0, 97.5, 3.0, 3.0, 97.5, 3.0]];
        assert_eq!(
            super::outline_vertices(&edges, glam::DVec3::new(0.5, 98.62, 0.5)),
            vec![[
                1.5,
                (97.5 - 98.62) as f32,
                2.5,
                2.5,
                (97.5 - 98.62) as f32,
                2.5
            ]]
        );
        assert!(super::outline_vertices(&[], glam::DVec3::ZERO).is_empty());
    }
}

#[cfg(test)]
mod sprite_upload_tests {
    use super::*;
    #[test]
    fn real_rail_mips_do_not_change_with_larger_array_layers() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/rail.png")
            .exists()
        {
            return;
        }
        let mut sprites = crate::sprites::Sprites::default();
        let layers: Vec<_> = [
            "rail",
            "powered_rail",
            "powered_rail_on",
            "detector_rail",
            "activator_rail",
        ]
        .map(|name| {
            sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap()
        })
        .into();
        sprites.finish();
        assert_eq!(sprites.size, 16);
        let native: Vec<_> = layers
            .iter()
            .map(|&layer| {
                let start = layer as usize * 16 * 16 * 4;
                mip_chain(
                    &sprites.pixels[start..start + 16 * 16 * 4],
                    16,
                    &sprites.mips,
                    layer,
                    1,
                )
                .0
            })
            .collect();
        sprites.insert(&root, "minecraft:block/oak_sign").unwrap();
        sprites.finish();
        assert_eq!(sprites.size, 32);
        for (&layer, expected) in layers.iter().zip(native) {
            let start = layer as usize * 32 * 32 * 4;
            let (actual, _) = mip_chain(
                &sprites.pixels[start..start + 32 * 32 * 4],
                32,
                &sprites.mips,
                layer,
                1,
            );
            assert_eq!(&actual[32 * 32 * 4..], expected, "rail layer {layer}");
        }
    }
    #[test]
    fn prepared_texture_uploads_preserve_layer_order_and_settings() {
        let size = 4;
        let layers = 5;
        let pixels: Vec<u8> = (0..size * size * layers)
            .flat_map(|i| [i as u8, (i * 3) as u8, 99, if i % 3 == 0 { 0 } else { 255 }])
            .collect();
        let settings: Vec<_> = (0..layers)
            .map(|i| crate::mipmap::Settings {
                strategy: if i % 2 == 0 {
                    crate::mipmap::Strategy::Mean
                } else {
                    crate::mipmap::Strategy::StrictCutout
                },
                ..Default::default()
            })
            .collect();
        let border = [20, 30, 40, 128].repeat(16);
        let expected = mip_chain(&pixels, size, &settings, 0, layers);
        // A partial final array and an exact boundary exercise the global
        // layer offset used to select each sprite's mipmap settings.
        for split in [1, 2, 5, 8] {
            let prepared = PreparedTextures::with_split(
                &pixels,
                size,
                layers,
                &settings,
                (&border, size),
                split,
            );
            assert_eq!(prepared.sprites.len(), layers.div_ceil(split) as usize);
            let actual: Vec<_> = prepared
                .sprites
                .iter()
                .flat_map(|(bytes, levels)| {
                    assert_eq!(*levels, expected.1);
                    bytes.iter().copied()
                })
                .collect();
            assert_eq!(actual, expected.0);
            assert_eq!(prepared.forcefield, mip_chain(&border, size, &[], 0, 1));
            // Chains cut before the adapter was known, cut again for it.
            for to in [1, 2, 3, 5, 8] {
                let direct = PreparedTextures::with_split(
                    &pixels,
                    size,
                    layers,
                    &settings,
                    (&border, size),
                    to,
                );
                let resplit = PreparedTextures::with_split(
                    &pixels,
                    size,
                    layers,
                    &settings,
                    (&border, size),
                    split,
                )
                .resplit(layers, to);
                assert_eq!(resplit.split, to);
                assert_eq!(resplit.sprites, direct.sprites, "{split} -> {to}");
            }
        }
    }

    #[test]
    fn sprite_split_follows_the_adapters_layer_limit() {
        // The Discord report: 512 layers per array, 1291 block textures (3 arrays).
        assert_eq!(layer_split(512, false, None, 1291), Ok(512));
        assert_eq!(layer_split(512, true, None, 1291), Ok(512));
        assert_eq!(layer_split(2048, false, None, 1291), Ok(SPRITE_SPLIT));
        assert_eq!(layer_split(8192, false, None, 1291), Ok(SPRITE_SPLIT));
        // HORIZON_SPRITE_SPLIT only lowers it.
        assert_eq!(layer_split(2048, false, Some(512), 1291), Ok(512));
        assert_eq!(layer_split(256, false, Some(512), 1291), Ok(256));
        // OpenGL pads a multiple of 6 by a layer: a full array must still fit.
        assert_eq!(layer_split(600, true, None, 1291), Ok(599));
        assert_eq!(layer_split(600, false, None, 1291), Ok(600));
        assert_eq!(array_layers(599, true), 599);
        // Past the arrays terrain.wgsl binds.
        assert_eq!(layer_split(256, false, None, 256 * SPRITE_ARRAYS), Ok(256));
        let err = layer_split(128, false, None, 1291).unwrap_err();
        assert!(err.contains("128 texture layers per array"), "{err}");
        assert!(layer_split(512, false, None, 512 * SPRITE_ARRAYS + 1).is_err());
    }

    #[test]
    fn mip_chain_allocates_exactly_its_size() {
        for size in [16, 64] {
            let animation = crate::sprites::test_animation(size, 0);
            let (chain, _) = mip_chain(animation.pixels(0), size, &[], 0, 1);
            assert_eq!(chain.capacity(), chain.len());
        }
    }

    #[test]
    fn collect_animated_layers_masks_opaque_layer() {
        use crate::mesher::{self, SectionMesh, TerrainVertex};
        let opaque_vertex =
            TerrainVertex::new([0.0, 0.0, 0.0], [0.0, 0.0], 7 | mesher::OPAQUE_LAYER, 0, 0);
        let translucent_vertex = TerrainVertex::new([0.0, 0.0, 0.0], [0.0, 0.0], 12, 0, 0);
        let mut mesh = SectionMesh {
            solid: vec![opaque_vertex; 4],
            translucent: vec![translucent_vertex; 4],
            ..Default::default()
        };
        mesh.note_sprite_layer(7 | mesher::OPAQUE_LAYER);
        mesh.note_sprite_layer(12);
        mesh.finish_sprite_layers();

        let animated = super::collect_animated_layers(&mesh, |layer| layer == 7 || layer == 12);
        assert_eq!(&*animated, &[7, 12]);
    }
}

/// What the system says about the window's display, kept up to date: on Wayland, the
/// compositor's preferred image description of the surface; elsewhere what wgpu reads
/// from the OS (on Windows, DXGI's luminances and the SDR white level).
#[derive(Default)]
struct DisplayWatch {
    #[cfg(target_os = "linux")]
    wayland: Option<horizon_hdr::wayland::Watch>,
    /// Without Wayland's event, what wgpu last read and when to read it again: the OS
    /// sends no event for a moved window or a changed SDR brightness slider.
    polled: Option<(Option<crate::graphics::Reported>, std::time::Instant)>,
}
/// How often [`DisplayWatch`] reads the OS's values again where no event says they changed.
const DISPLAY_POLL: std::time::Duration = std::time::Duration::from_secs(2);
impl DisplayWatch {
    /// Starts watching, with what the system reports now.
    fn new(
        window: &Window,
        surface: Option<&wgpu::Surface>,
        adapter: &wgpu::Adapter,
    ) -> (Self, Option<crate::graphics::Reported>) {
        #[cfg(target_os = "linux")]
        if let Some((wayland, now)) = horizon_hdr::wayland::Watch::new(window) {
            let watch = Self {
                wayland: Some(wayland),
                polled: None,
            };
            return (watch, now.map(reported));
        }
        let _ = window;
        let now = surface.and_then(|surface| os_reported(&surface.display_hdr_info(adapter)));
        log_os_reported(now);
        let watch = Self {
            #[cfg(target_os = "linux")]
            wayland: None,
            polled: Some((now, std::time::Instant::now() + DISPLAY_POLL)),
        };
        (watch, now)
    }
    /// What the system reports, if that changed since the last call.
    fn changed(
        &mut self,
        surface: Option<&wgpu::Surface>,
        adapter: &wgpu::Adapter,
    ) -> Option<Option<crate::graphics::Reported>> {
        #[cfg(target_os = "linux")]
        if let Some(wayland) = &mut self.wayland {
            return wayland.changed().map(|now| now.map(reported));
        }
        let (last, next) = self.polled.as_mut()?;
        let time = std::time::Instant::now();
        if time < *next {
            return None;
        }
        *next = time + DISPLAY_POLL;
        let now = surface.and_then(|surface| os_reported(&surface.display_hdr_info(adapter)));
        (now != *last).then(|| {
            log_os_reported(now);
            *last = now;
            now
        })
    }
}

/// The log of what the system reports about the display: one line per change. The
/// renderer is made anew a few times while the game starts and each asks again, and a
/// slider dragged in the desktop's settings changes the values many times a second, so
/// a change within [`DISPLAY_LOG_QUIET`] of the last line waits until the values have
/// held that long, and only the one they settled on is logged.
struct DisplayLog {
    last: String,
    at: Option<std::time::Instant>,
    pending: Option<(String, std::time::Instant)>,
}
static DISPLAY_LOG: std::sync::Mutex<DisplayLog> = std::sync::Mutex::new(DisplayLog {
    last: String::new(),
    at: None,
    pending: None,
});
const DISPLAY_LOG_QUIET: std::time::Duration = std::time::Duration::from_secs(1);
fn log_display(line: String) {
    let now = std::time::Instant::now();
    let mut log = DISPLAY_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if line == log.last {
        log.pending = None;
    } else if log.at.is_none_or(|at| now - at >= DISPLAY_LOG_QUIET) {
        log::info!("{line}");
        (log.last, log.at, log.pending) = (line, Some(now), None);
    } else {
        log.pending = Some((line, now));
    }
}
/// Logs a change [`log_display`] held back, once the values have held for
/// [`DISPLAY_LOG_QUIET`].
fn flush_display_log() {
    let now = std::time::Instant::now();
    let mut log = DISPLAY_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, since)) = &log.pending
        && now - *since >= DISPLAY_LOG_QUIET
        && let Some((line, _)) = log.pending.take()
    {
        log::info!("{line}");
        (log.last, log.at) = (line, Some(now));
    }
}
/// A reported black level, within the setting's range. Rounded down: OLEDs report a figure
/// just above 0, like KWin's 0.005, which isn't worth lifting the shadows for.
fn hundredths_of_a_nit(nits: f32) -> u32 {
    if nits.is_finite() && nits > 0.0 {
        ((nits * 100.0) as u32).min(100)
    } else {
        0
    }
}
fn log_os_reported(reported: Option<crate::graphics::Reported>) {
    log_display(match reported {
        Some(r) => format!(
            "Horizon renderer: the system reports SDR white at {} nits and a peak of {} nits",
            r.paper_white, r.peak
        ),
        None => "Horizon renderer: the system doesn't report the display's luminance".into(),
    });
}
/// What the OS reports in absolute nits (Windows: DXGI's `MaxLuminance` and the SDR
/// white level the brightness slider sets), which is HDR output's scale there: SDR
/// white is Paper White, the display's peak is Peak Brightness. Both are kept within
/// the settings' ranges. The OS's figures often come from the display's EDID and may
/// run optimistic; the player's own value always wins.
fn os_reported(info: &wgpu::DisplayHdrInfo) -> Option<crate::graphics::Reported> {
    let luminance = info.luminance?;
    let usable = |n: f32| n.is_finite() && n > 0.0;
    let peak = luminance.max_nits.filter(|&n| usable(n))?;
    let white = luminance.sdr_white_nits.filter(|&n| usable(n))?;
    let paper_white = (white.round() as u32).clamp(80, 480);
    Some(crate::graphics::Reported {
        paper_white,
        black: hundredths_of_a_nit(luminance.min_nits.unwrap_or(0.0)),
        // Windows' SDR brightness slider can set white above the reported peak (480 against
        // a calibrated 380): the highlights then stay at white instead of dropping below it.
        peak: (peak.round() as u32).clamp(200, 10_000).max(paper_white),
    })
}

/// What a compositor's luminances mean in HDR output's scale.
///
/// KWin maps content's reference white (203 nits in PQ, BT.2408) onto its own, so the
/// desktop's white is 203 there and the peak is the target's maximum over that ratio.
/// Checked on KWin. Hyprland (0.57) reports the standard 203, and shows HDR windows below
/// the SDR white of its desktop (which it doesn't report), so Auto can't match its desktop
/// there. Other compositors are unchecked. The peak is kept within the settings' lowest and
/// PQ's top, so the metadata never ends below the black level.
#[cfg(target_os = "linux")]
fn reported(l: horizon_hdr::wayland::Luminances) -> crate::graphics::Reported {
    let peak = (l.target_max * 203.0 / l.reference.max(1.0)).round() as u32;
    log_display(format!(
        "Horizon renderer: the compositor shows this window with reference white {} nits, \
         {}..{} nits: peak {peak} nits in HDR output's scale",
        l.reference, l.target_min, l.target_max,
    ));
    crate::graphics::Reported {
        paper_white: 203,
        peak: peak.clamp(200, 10_000),
        black: hundredths_of_a_nit(l.target_min * 203.0 / l.reference.max(1.0)),
    }
}
