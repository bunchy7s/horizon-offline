//! `GuiGraphics.renderItem` drawn into the HUD batch: the item's layers are
//! submitted under the GUI item pose (`translate(x + 8, y + 8)`,
//! `scale(16, -16, 16)`), lit with `Lighting.Entry.ITEMS_3D` or `ITEMS_FLAT`
//! (`minecraft_mix_light` on the transformed normals), back faces culled,
//! drawn far to near, and clipped to the item's 16x16 slot unless the model
//! is oversized in the GUI (`GuiItemAtlas` draws each item into a scissored
//! 16x16 slot; oversized items get a picture-in-picture of their bounds).
use super::{Foil, Glint, ItemQuad, ItemRenderState, ItemRenderer, SpecialHost};
use crate::hud::{HudBatch, Rect};
use glam::{EulerRot, Mat4, Vec2, Vec3};
use std::{collections::HashMap, f32::consts::PI, sync::Arc};

mod depth;

/// `Lighting.DIFFUSE_LIGHT_0` and `DIFFUSE_LIGHT_1`.
fn diffuse_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, 1.0, -0.7).normalize(),
        Vec3::new(-0.2, 1.0, 0.7).normalize(),
    )
}
/// `Lighting.Entry.ITEMS_FLAT`: `rotationY(-PI/8).rotateX(3PI/4)`.
pub fn items_flat_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_rotation_y(-PI / 8.0) * Mat4::from_rotation_x(PI * 3.0 / 4.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}
/// JOML `rotateYXZ(y, x, z)`.
fn rotate_yxz(y: f32, x: f32, z: f32) -> Mat4 {
    Mat4::from_euler(EulerRot::YXZ, y, x, z)
}
/// `Lighting.Entry.ITEMS_3D`.
pub fn items_3d_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
        * rotate_yxz(1.082_104_1, 3.237_585_8, 0.0)
        * rotate_yxz(-PI / 8.0, PI * 3.0 / 4.0, 0.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}

/// A GUI-space vertex: position, atlas UV.
#[derive(Clone, Copy, Debug)]
struct Point {
    p: Vec2,
    uv: [f32; 2],
    glint_uv: [f32; 2],
}
fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point {
        p: a.p + (b.p - a.p) * t,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
        glint_uv: [
            a.glint_uv[0] + (b.glint_uv[0] - a.glint_uv[0]) * t,
            a.glint_uv[1] + (b.glint_uv[1] - a.glint_uv[1]) * t,
        ],
    }
}
/// Sutherland-Hodgman against one half plane `dot(p, n) <= limit`.
fn clip_edge(poly: &[Point], axis: usize, limit: f32, keep_below: bool) -> Vec<Point> {
    let inside = |p: &Point| {
        let v = p.p[axis];
        if keep_below { v <= limit } else { v >= limit }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (limit - a.p[axis]) / (b.p[axis] - a.p[axis]);
            out.push(lerp(a, b, t));
        }
    }
    out
}
/// A triangle clipped to `[x0, x1] x [y0, y1]`, as a fan.
fn clip_triangle(tri: [Point; 3], rect: [f32; 4]) -> Vec<Point> {
    let mut poly = tri.to_vec();
    for (axis, limit, below) in [
        (0, rect[0], false),
        (0, rect[2], true),
        (1, rect[1], false),
        (1, rect[3], true),
    ] {
        if poly.is_empty() {
            break;
        }
        poly = clip_edge(&poly, axis, limit, below);
    }
    poly
}

/// RGBA vertex colour of a lit quad: `minecraft_mix_light` on the tint.
pub fn lit_color(q: &ItemQuad, lights: (Vec3, Vec3)) -> u32 {
    let light = crate::block_entity_render::diffuse(q.normal, lights);
    let [a, r, g, b] = q.color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * light) as u8;
    crate::hud::rgba(c(r), c(g), c(b), a)
}
fn emit(batch: &mut HudBatch, fan: &[Point], color: impl Fn(Vec2) -> u32, glint: Option<&Glint>) {
    for i in 1..fan.len().saturating_sub(1) {
        let (a, b, c) = (fan[0], fan[i], fan[i + 1]);
        let corners = [a, b, c, c];
        if let Some(glint) = glint {
            let start = batch.vertices.len();
            batch.quad_glint(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| [v.uv[0] + 2.0, v.uv[1]]),
                corners.map(|v| glint.uv(v.glint_uv)),
                u32::MAX,
            );
            batch.blend_since(start, crate::hud::Blend::Additive);
        } else {
            batch.quad(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| v.uv),
                color(a.p),
            );
        }
    }
}

/// Submit, light, cull, sort and clip one item's quads into `batch` at GUI
/// position `(x, y)`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    time: f32,
    glint_speed: f32,
    glint_alpha: f32,
) {
    if state.is_empty() {
        return;
    }
    // A static item drawn where it was drawn before gives the same vertices:
    // take them instead of submitting, sorting and clipping its quads again.
    let key = DrawnKey::of(state, renderer, sprites, x, y);
    if let Some(key) = &key
        && let Some(drawn) = renderer
            .gui_drawn
            .lock()
            .ok()
            .and_then(|d| d.get(key).cloned())
    {
        batch.vertices.extend_from_slice(&drawn);
        return;
    }
    let start = batch.vertices.len();
    draw_now(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        time,
        glint_speed,
        glint_alpha,
        None,
    );
    if let Some(key) = key
        && let Ok(mut drawn) = renderer.gui_drawn.lock()
    {
        // GUI positions and items are few; a long scroll through every tab stays bounded.
        if drawn.len() >= DRAWN_LIMIT {
            drawn.clear();
        }
        drawn.insert(key, batch.vertices[start..].into());
    }
}

/// [`draw`] clipped to `scissor` (`[x0, y0, x1, y1]`) as well as the item's slot:
/// a list's scissor cuts the item of a half-visible row exactly. Not cached, the
/// scissor is not part of the cache key.
#[allow(clippy::too_many_arguments)]
pub fn draw_scissored(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    scissor: [f32; 4],
) {
    if state.is_empty() {
        return;
    }
    draw_now(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        0.0,
        0.0,
        0.0,
        Some(scissor),
    );
}

/// Entries of `ItemRenderer::gui_drawn` before it starts over.
const DRAWN_LIMIT: usize = 8192;

/// What a GUI item's vertices depend on: its layers' baked models (kept for the
/// renderer's life, so their addresses name them), tints and transforms, the
/// atlas, and the exact position. None for items whose drawing changes with time
/// or the host: a glint, or a special model renderer.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct DrawnKey {
    renderer: usize,
    sprites: (usize, usize),
    at: [u32; 2],
    oversized: bool,
    layers: Vec<(usize, Vec<i32>, [u32; 16], bool)>,
}
impl DrawnKey {
    fn of(
        state: &ItemRenderState,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        x: f32,
        y: f32,
    ) -> Option<Self> {
        if state.animated {
            return None;
        }
        let layers = state
            .layers
            .iter()
            .map(|layer| match &layer.geometry {
                super::LayerGeometry::Quads(baked) if layer.foil == super::FoilType::None => {
                    Some((
                        Arc::as_ptr(baked) as usize,
                        layer.tints.clone(),
                        layer.matrix.to_cols_array().map(f32::to_bits),
                        layer.uses_block_light,
                    ))
                }
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            renderer: std::ptr::from_ref(renderer) as usize,
            sprites: (std::ptr::from_ref(sprites) as usize, sprites.len()),
            at: [x.to_bits(), y.to_bits()],
            oversized: state.oversized_in_gui,
            layers,
        })
    }
}

/// [`draw`] without the cache.
#[allow(clippy::too_many_arguments)]
fn draw_now(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    time: f32,
    glint_speed: f32,
    glint_alpha: f32,
    scissor: Option<[f32; 4]>,
) {
    let pose = Mat4::from_translation(Vec3::new(x + 8.0, y + 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    let mut quads: Vec<ItemQuad> = Vec::new();
    state.submit(renderer, sprites, host, pose, &mut quads);
    let lights = if state.uses_block_light() {
        items_3d_lights()
    } else {
        items_flat_lights()
    };
    let rect = if state.oversized_in_gui {
        [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ]
    } else {
        [x, y, x + 16.0, y + 16.0]
    };
    let rect = match scissor {
        Some([x0, y0, x1, y1]) => [
            rect[0].max(x0),
            rect[1].max(y0),
            rect[2].min(x1),
            rect[3].min(y1),
        ],
        None => rect,
    };
    let glint = sprites
        .contains_key("misc/enchanted_glint_item")
        .then_some(Glint {
            speed: glint_speed,
            alpha: glint_alpha,
            millis: time * 1000.0,
            scale: 8.0,
        });
    batch.glint_alpha = glint_alpha;
    draw_quads(&quads, lights, batch, rect, glint.as_ref());
}

/// Lights, culls, sorts far to near and clips `quads` (GUI positions) to
/// `rect`.
fn draw_quads(
    quads: &[ItemQuad],
    lights: (Vec3, Vec3),
    batch: &mut HudBatch,
    rect: [f32; 4],
    glint: Option<&Glint>,
) {
    for face in depth::sort(quads) {
        let q = &quads[face.quad];
        let points: Vec<_> = face
            .corners
            .iter()
            .map(|v| Point {
                p: v.pos.truncate(),
                uv: v.uv,
                glint_uv: v.glint_uv,
            })
            .collect();
        let color = lit_color(q, lights);
        for i in 1..points.len().saturating_sub(1) {
            let tri = [points[0], points[i], points[i + 1]];
            let fan = clip_triangle(tri, rect);
            if fan.len() < 3 {
                continue;
            }
            emit(batch, &fan, |_| color, None);
            if q.foil != Foil::None {
                emit(batch, &fan, |_| color, glint);
            }
        }
    }
}

/// `PictureInPictureRenderer.prepare` for the loom's 20x40 result rectangle
/// at `(x0, y0)` (`translate(w / 2, h)`, `scale(16, 16, -16)`, one texel per
/// GUI pixel), then `GuiBannerResultRenderer.renderToTexture`'s
/// `translate(0, 0.25, 0)`.
pub fn banner_result_pose(x0: f32, y0: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x0 + 10.0, y0 + 40.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, 16.0, -16.0))
        * Mat4::from_translation(Vec3::new(0.0, 0.25, 0.0))
}

/// The quads of `GuiGraphicsExtractor.bannerPattern` (the loom's result
/// preview) at `(x0, y0)`: base dye and `(texture, dye)` layers.
pub fn banner_result_quads(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    super::special::submit_banner_result(
        renderer,
        banner_result_pose(x0, y0),
        base,
        layers,
        sprites,
        &mut quads,
    );
    quads
}

/// Draws the loom's result preview: the picture in picture is lit with
/// `Lighting.Entry.ITEMS_FLAT` and holds only its 20x40 rectangle.
pub fn draw_banner_result(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) {
    let quads = banner_result_quads(renderer, sprites, x0, y0, base, layers);
    draw_quads(
        &quads,
        items_flat_lights(),
        batch,
        [x0, y0, x0 + 20.0, y0 + 40.0],
        None,
    );
}

/// `Lighting.Entry.ENTITY_IN_UI` (`setupForEntityInInventory`), which the
/// enchanting screen's book is lit with.
fn entity_in_ui_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, -1.0, -1.0).normalize(),
        Vec3::new(-0.2, -1.0, 0.0).normalize(),
    )
}

/// `PictureInPictureRenderer.prepare` for the enchanting screen's 38x31 book
/// box at `(x0, y0)` (`translate(w / 2, 17)`, `scale(40, 40, -40)`), then
/// `GuiBookModelRenderer.renderToTexture`'s turns for `open`.
pub fn enchanting_book_pose(x0: f32, y0: f32, open: f32) -> Mat4 {
    let closed = 1.0 - open;
    Mat4::from_translation(Vec3::new(x0 + 19.0, y0 + 17.0, 0.0))
        * Mat4::from_scale(Vec3::new(40.0, 40.0, -40.0))
        * Mat4::from_rotation_y(PI)
        * Mat4::from_rotation_x(25f32.to_radians())
        * Mat4::from_translation(Vec3::new(closed * 0.2, closed * 0.1, closed * 0.25))
        * Mat4::from_rotation_y((-closed * 90.0 - 90.0).to_radians())
        * Mat4::from_rotation_x(PI)
}

/// `GuiBookModelRenderer.renderToTexture`'s page flips for `flip`: each
/// `clamp(frac(flip + offset) * 1.6 - 0.3, 0, 1)`.
pub fn enchanting_book_pages(flip: f32) -> (f32, f32) {
    let page = |offset: f32| {
        let x = flip + offset;
        ((x - x.floor()) * 1.6 - 0.3).clamp(0.0, 1.0)
    };
    (page(0.25), page(0.75))
}

/// `GuiGraphicsExtractor.book` from `EnchantmentScreen.extractBook`: the
/// enchanting table's book, `open` (0 to 1) and turning its pages with
/// `flip`, in the 38x31 box at `(x0, y0)`.
pub fn draw_enchanting_book(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    open: f32,
    flip: f32,
) {
    let (page1, page2) = enchanting_book_pages(flip);
    let mut quads = Vec::new();
    super::special::submit_book(
        renderer,
        enchanting_book_pose(x0, y0, open),
        open,
        page1,
        page2,
        sprites,
        &mut quads,
    );
    draw_quads(
        &quads,
        entity_in_ui_lights(),
        batch,
        [x0, y0, x0 + 38.0, y0 + 31.0],
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchanting_book_pages_turn_with_the_flip() {
        let close = |flip: f32, want: (f32, f32)| {
            let (a, b) = enchanting_book_pages(flip);
            assert!(
                (a - want.0).abs() < 1e-5 && (b - want.1).abs() < 1e-5,
                "{flip}: {a} {b}, want {want:?}"
            );
        };
        close(0.0, (0.1, 0.9));
        // A whole page further, the same pages.
        close(1.0, (0.1, 0.9));
        close(0.5, (0.9, 0.1));
        // The pages rest flat at either end of a turn.
        close(-0.25, (0.0, 0.5));
        close(0.6875, (1.0, 0.4));
    }
    #[test]
    fn a_closed_book_turns_away_and_an_open_one_faces_the_screen() {
        // The box centre maps the model origin; the book's origin moves with
        // `open` (`translate((1 - open) * (0.2, 0.1, 0.25))` before the turn).
        let open = enchanting_book_pose(0.0, 0.0, 1.0).transform_point3(Vec3::ZERO);
        assert!(
            (open - Vec3::new(19.0, 17.0, 0.0)).length() < 1e-4,
            "{open}"
        );
        let closed = enchanting_book_pose(0.0, 0.0, 0.0).transform_point3(Vec3::ZERO);
        assert!((closed - open).length() > 1.0, "{closed}");
    }

    fn face(y0: f32, y1: f32, z: impl Fn(f32) -> f32, color: u32) -> ItemQuad {
        ItemQuad {
            pos: [(0.0, y0), (0.0, y1), (8.0, y1), (8.0, y0)].map(|(x, y)| Vec3::new(x, y, z(y))),
            uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            foil_uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            normal: Vec3::Z,
            color,
            foil: Foil::None,
            light_emission: 0,
            cull_back: false,
            translucent: false,
            oit: false,
        }
    }

    // Sample the emitted triangles in draw order, using opaque test faces.
    fn color_at(batch: &HudBatch, p: Vec2) -> Option<u32> {
        batch
            .vertices
            .as_chunks::<3>()
            .0
            .iter()
            .rev()
            .find_map(|tri| {
                let [a, b, c] = std::array::from_fn(|i| Vec2::from(tri[i].pos));
                let area = (b - a).perp_dot(c - a);
                if area.abs() < 1e-6 {
                    return None;
                }
                let u = (b - p).perp_dot(c - p) / area;
                let v = (c - p).perp_dot(a - p) / area;
                let w = 1.0 - u - v;
                (u >= 0.0 && v >= 0.0 && w >= 0.0).then_some(tri[0].color)
            })
    }

    #[test]
    fn banner_cloth_occludes_the_support_at_the_top_edge() {
        // The cloth is nearer where it overlaps the bar, although its
        // average depth is farther away because it slopes along its height.
        let bar = face(0.0, 1.0, |_| 0.0, 0xFF80_4000);
        let cloth = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let mut batch = HudBatch::new([320, 240]);
        draw_quads(
            &[bar, cloth],
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        assert_eq!(color_at(&batch, Vec2::new(4.0, 0.5)), Some(u32::MAX));
    }

    #[test]
    fn intersecting_faces_change_draw_order_at_their_intersection() {
        let flat = face(0.0, 8.0, |_| 0.0, 0xFFFF_0000);
        let tilted = face(0.0, 8.0, |y| y - 4.0, 0xFF00_00FF);
        for quads in [[flat, tilted], [tilted, flat]] {
            let mut batch = HudBatch::new([320, 240]);
            draw_quads(
                &quads,
                (Vec3::Z, Vec3::Z),
                &mut batch,
                [0.0, 0.0, 16.0, 16.0],
                None,
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 2.0)),
                Some(lit_color(&flat, (Vec3::Z, Vec3::Z)))
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 6.0)),
                Some(lit_color(&tilted, (Vec3::Z, Vec3::Z)))
            );
            // The cut must interpolate the texture along with the position.
            for v in &batch.vertices {
                assert!((v.uv[0] - v.pos[0] / 8.0).abs() < 1e-6);
                assert!((v.uv[1] - v.pos[1] / 8.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn coplanar_pattern_layers_keep_submission_order() {
        let base = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let pattern = ItemQuad {
            color: 0xFF00_00FF,
            ..base
        };
        let quads = [face(0.0, 1.0, |_| 0.0, 0xFF80_4000), base, pattern];
        let mut batch = HudBatch::new([320, 240]);
        draw_quads(
            &quads,
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        for y in [0.5, 6.0] {
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, y)),
                Some(lit_color(&quads[2], (Vec3::Z, Vec3::Z)))
            );
        }
    }

    #[test]
    fn flat_lights_match_joml() {
        // new Matrix4f().rotationY(-PI/8).rotateX(3PI/4).transformDirection(L0).
        let (l0, _) = items_flat_lights();
        let l = Vec3::new(0.2, 1.0, -0.7).normalize();
        let (s, c) = ((3.0 * PI / 4.0).sin(), (3.0 * PI / 4.0).cos());
        let rx = Vec3::new(l.x, c * l.y - s * l.z, s * l.y + c * l.z);
        let (s, c) = ((-PI / 8.0).sin(), (-PI / 8.0).cos());
        let ry = Vec3::new(c * rx.x + s * rx.z, rx.y, -s * rx.x + c * rx.z);
        assert!((l0 - ry).length() < 1e-6);
    }
    #[test]
    fn triangles_clip_to_the_slot() {
        let p = |x: f32, y: f32| Point {
            p: Vec2::new(x, y),
            uv: [x, y],
            glint_uv: [0.0, 0.0],
        };
        let fan = clip_triangle(
            [p(-4.0, 0.0), p(8.0, 0.0), p(8.0, 8.0)],
            [0.0, 0.0, 16.0, 16.0],
        );
        assert!(fan.iter().all(|v| v.p.x >= 0.0));
        // UVs follow the cut affinely.
        assert!(fan.iter().all(|v| (v.uv[0] - v.p.x).abs() < 1e-6));
        assert!(
            clip_triangle(
                [p(20.0, 0.0), p(30.0, 0.0), p(30.0, 5.0)],
                [0.0, 0.0, 16.0, 16.0]
            )
            .is_empty()
        );
    }
}
