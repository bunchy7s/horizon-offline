//! The glass look's plates (`.hzl-glass` and `.hzl-float` in the mockup): the scene behind,
//! blurred and saturated, under a little white and the accent's dark violet, with round corners.
//!
//! GPUI has no backdrop filter, but nothing sits under a plate except the scene (`scene.rs`), so
//! each plate paints the matching piece of the scene's blurred copy ([`scene::Backdrop`]) with its
//! own corners, its tint mixed in. The pieces are small images (a texel per 4 logical pixels: a
//! 24 px blur has nothing finer), kept while the pages that drew them may still be on screen.
use crate::scene::{Backdrop, Model};
use gpui::{canvas, div, prelude::*, px, Bounds, Corners, Div, Pixels, RenderImage, Size, Window};
use horizon_theme::glass::Glass;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Logical pixels per texel of a plate's piece of backdrop.
const PIECE: f32 = 4.;

/// A plate's kind: `.hzl-glass` (panels, the sidebar, dialogs) or `.hzl-float` (menus and
/// popovers: twice the blur, darker).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Glass,
    Float,
}

/// A piece's place and size, its kind, the backdrop's generation, and its shape: 0 for a plate,
/// else the device scale (×100) of a [`fillet`](tabbed)'s piece.
type Key = (i32, i32, i32, i32, Kind, u32, u32);

struct Pieces {
    backdrop: Option<Backdrop>,
    look: Glass,
    /// Bumped when the backdrop or the look changes: older pieces are thrown away.
    generation: u32,
    /// Bumped each time the launcher's pages are built: a piece last painted two builds ago is
    /// on no frame any more (GPUI Fast copies a view's last frame, never an older one).
    build: u64,
    pieces: HashMap<Key, (Arc<RenderImage>, u64)>,
    retired: Vec<Arc<RenderImage>>,
}

thread_local! {
    static PIECES: RefCell<Pieces> = RefCell::new(Pieces {
        backdrop: None,
        look: Glass::default_look(),
        generation: 0,
        build: 0,
        pieces: HashMap::new(),
        retired: Vec::new(),
    });
}

/// The scene was built again (a new window size): plates take their pieces from this one.
pub fn set_backdrop(model: &Model) {
    PIECES.with_borrow_mut(|p| {
        p.backdrop = Some(Backdrop::new(model, p.look.blur as f64));
        p.generation += 1;
    });
}

/// The launcher's pages are being built: pieces nothing drew in the last build are freed (call
/// at the start of the launcher's render).
pub fn begin_build(window: &mut Window) {
    let retired = PIECES.with_borrow_mut(|p| {
        p.build += 1;
        let (build, generation) = (p.build, p.generation);
        let stale: Vec<Key> = p
            .pieces
            .iter()
            .filter(|(k, (_, used))| k.5 != generation || *used + 1 < build)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            if let Some((image, _)) = p.pieces.remove(&k) {
                p.retired.push(image);
            }
        }
        std::mem::take(&mut p.retired)
    });
    for image in retired {
        let _ = window.drop_image(image);
    }
}

/// A plate's backdrop with its corners: put it first inside a `relative` element, the content
/// after it.
pub fn backdrop(kind: Kind, radius: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| paint(bounds, kind, Corners::all(px(radius)), window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A `relative` div on a glass plate with `radius` corners.
pub fn plate(radius: f32) -> Div {
    div()
        .relative()
        .rounded(px(radius))
        .child(backdrop(Kind::Glass, radius))
}

/// The same for a menu or popover.
pub fn float(radius: f32) -> Div {
    div()
        .relative()
        .rounded(px(radius))
        .child(backdrop(Kind::Float, radius))
}

/// The piece at `bounds` (a plate's, or with `fillet` set to the device scale, a fillet's), made
/// once and kept.
fn image_at(bounds: Bounds<Pixels>, kind: Kind, fillet: f32) -> Option<Arc<RenderImage>> {
    PIECES.with_borrow_mut(|p| {
        let backdrop = p.backdrop.as_ref()?;
        let key = (
            f32::from(bounds.origin.x).round() as i32,
            f32::from(bounds.origin.y).round() as i32,
            f32::from(bounds.size.width).round() as i32,
            f32::from(bounds.size.height).round() as i32,
            kind,
            p.generation,
            (fillet * 100.).round() as u32,
        );
        if key.2 <= 0 || key.3 <= 0 {
            return None;
        }
        let build = p.build;
        if let Some((image, used)) = p.pieces.get_mut(&key) {
            *used = build;
            return Some(image.clone());
        }
        let image = if fillet > 0. {
            fillet_piece(backdrop, &p.look, kind, key, fillet)
        } else {
            piece(backdrop, &p.look, kind, key)
        };
        p.pieces.insert(key, (image.clone(), build));
        Some(image)
    })
}

fn paint(bounds: Bounds<Pixels>, kind: Kind, corners: Corners<Pixels>, window: &mut Window) {
    if let Some(image) = image_at(bounds, kind, 0.) {
        // The piece's border texels go just outside the plate (see [`piece`]).
        let (w, h) = (
            f32::from(bounds.size.width).round(),
            f32::from(bounds.size.height).round(),
        );
        let (cols, rows) = texels(w, h);
        let (tx, ty) = (w / cols as f32, h / rows as f32);
        let outer = Bounds::new(
            bounds.origin - gpui::point(px(tx), px(ty)),
            bounds.size + gpui::size(px(tx * 2.), px(ty * 2.)),
        );
        let _ = window.paint_image(bounds, outer, corners, image, 0, false);
    }
}

/// The tabbed panel's plate (the mockup's `TabbedPanel`): the tab strip's plate, round at 18 on
/// top, flows into the panel's below it, round at 20 except where the strip sits, with a 16 px
/// inward curve where the strip's right edge meets the panel's top. `strip` is the strip's size,
/// recorded by [`measure`] inside it. Close to the panel's own width, the panel is one plate
/// round at 20, as the mockup does.
pub fn tabbed(strip: Rc<Cell<Option<Size<Pixels>>>>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let tw = f32::from(bounds.size.width).round();
            let th = f32::from(bounds.size.height).round();
            let Some(s) = strip.get() else {
                return paint(bounds, Kind::Glass, Corners::all(px(20.)), window);
            };
            let (w, h) = (f32::from(s.width).round(), f32::from(s.height).round());
            if tw - w < 36. || th - h < 40. {
                return paint(bounds, Kind::Glass, Corners::all(px(20.)), window);
            }
            let o = bounds.origin;
            let at = |x: f32, y: f32, w: f32, h: f32| {
                Bounds::new(o + gpui::point(px(x), px(y)), gpui::size(px(w), px(h)))
            };
            let r = |tl: f32, tr: f32, br: f32, bl: f32| Corners {
                top_left: px(tl),
                top_right: px(tr),
                bottom_right: px(br),
                bottom_left: px(bl),
            };
            paint(at(0., 0., w, h), Kind::Glass, r(18., 18., 0., 0.), window);
            paint(
                at(0., h, tw, th - h),
                Kind::Glass,
                r(0., 20., 20., 20.),
                window,
            );
            let fillet = at(w, h - 16., 16., 16.);
            if let Some(image) = image_at(fillet, Kind::Glass, window.scale_factor()) {
                let _ = window.paint_image(fillet, fillet, Corners::default(), image, 0, false);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Records the size of the element it sits in (absolute, filling it) for [`tabbed`].
pub fn measure(into: Rc<Cell<Option<Size<Pixels>>>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| into.set(Some(bounds.size)),
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The inward curve's piece, a texel per device pixel: the backdrop where it lies outside the
/// circle of radius 16 centred on its top-right corner, clear inside, its edge smoothed over one
/// device pixel. Straight alpha, as GPUI Fast blends sprites.
fn fillet_piece(
    backdrop: &Backdrop,
    look: &Glass,
    kind: Kind,
    key: Key,
    scale: f32,
) -> Arc<RenderImage> {
    let (x0, y0, w) = (key.0 as f32, key.1 as f32, key.2 as f32);
    let n = (w * scale).round().max(1.) as u32;
    let tint = [16, 8, 0].map(|s| ((look.plate >> s) & 255) as f32 / 255.);
    let a = look.plate_alpha;
    let mut buf = image::RgbaImage::new(n, n);
    for (row, line) in buf.rows_mut().enumerate() {
        for (col, p) in line.enumerate() {
            let (lx, ly) = ((col as f32 + 0.5) / scale, (row as f32 + 0.5) / scale);
            let d = ((w - lx).powi(2) + ly.powi(2)).sqrt();
            let alpha = ((d - w) * scale + 0.5).clamp(0., 1.);
            let c = backdrop.at(kind == Kind::Float, (x0 + lx) as f64, (y0 + ly) as f64);
            let mixed = [0, 1, 2].map(|i| ((tint[i] * a + c[i] * (1. - a)) * 255.).round() as u8);
            *p = image::Rgba([mixed[2], mixed[1], mixed[0], (alpha * 255.).round() as u8]);
        }
    }
    Arc::new(RenderImage::new([image::Frame::new(buf)]))
}

/// How many texels a piece `w` × `h` logical pixels has across and down.
fn texels(w: f32, h: f32) -> (u32, u32) {
    (
        (w / PIECE).ceil().max(1.) as u32,
        (h / PIECE).ceil().max(1.) as u32,
    )
}

/// The plate at `key`'s place: the blurred scene under `look`'s tint, one texel per [`PIECE`]
/// logical pixels, and a border of one texel more all round. GPUI samples its atlas linearly,
/// so a stretched image's outer half-texels would blend in its atlas neighbour; with the border
/// they blend in the scene just outside the plate, as a backdrop blur would.
fn piece(backdrop: &Backdrop, look: &Glass, kind: Kind, key: Key) -> Arc<RenderImage> {
    let (x0, y0, w, h) = (key.0 as f32, key.1 as f32, key.2 as f32, key.3 as f32);
    let (cols, rows) = texels(w, h);
    let tint = [16, 8, 0].map(|s| ((look.plate >> s) & 255) as f32 / 255.);
    let a = look.plate_alpha;
    let mut buf = image::RgbaImage::new(cols + 2, rows + 2);
    for (row, line) in buf.rows_mut().enumerate() {
        for (col, p) in line.enumerate() {
            let x = x0 + (col as f32 - 0.5) * w / cols as f32;
            let y = y0 + (row as f32 - 0.5) * h / rows as f32;
            let c = backdrop.at(kind == Kind::Float, x as f64, y as f64);
            let mixed = [0, 1, 2].map(|i| ((tint[i] * a + c[i] * (1. - a)) * 255.).round() as u8);
            // BGRA, opaque.
            *p = image::Rgba([mixed[2], mixed[1], mixed[0], 255]);
        }
    }
    Arc::new(RenderImage::new([image::Frame::new(buf)]))
}
