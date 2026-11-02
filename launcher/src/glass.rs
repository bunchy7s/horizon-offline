//! The glass look's plates (`.hzl-glass` and `.hzl-float` in the mockup): the scene behind,
//! blurred and saturated, under a little white and the accent's dark violet, with round corners.
//!
//! GPUI has no backdrop filter, but nothing sits under a plate except the scene (`scene.rs`), so
//! each plate paints the matching piece of the scene's blurred copy ([`scene::Backdrop`]) with its
//! own corners, its tint mixed in. The pieces are small images (a texel per 4 logical pixels: a
//! 24 px blur has nothing finer), kept while the pages that drew them may still be on screen.
use crate::scene::{Backdrop, Model};
use gpui::{canvas, div, prelude::*, px, Bounds, Corners, Div, Pixels, RenderImage, Window};
use horizon_theme::glass::Glass;
use std::cell::RefCell;
use std::collections::HashMap;
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

type Key = (i32, i32, i32, i32, Kind, u32);

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
        move |bounds, (), window, _| paint(bounds, kind, radius, window),
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

fn paint(bounds: Bounds<Pixels>, kind: Kind, radius: f32, window: &mut Window) {
    let image = PIECES.with_borrow_mut(|p| {
        let backdrop = p.backdrop.as_ref()?;
        let key = (
            f32::from(bounds.origin.x).round() as i32,
            f32::from(bounds.origin.y).round() as i32,
            f32::from(bounds.size.width).round() as i32,
            f32::from(bounds.size.height).round() as i32,
            kind,
            p.generation,
        );
        if key.2 <= 0 || key.3 <= 0 {
            return None;
        }
        let build = p.build;
        if let Some((image, used)) = p.pieces.get_mut(&key) {
            *used = build;
            return Some(image.clone());
        }
        let image = piece(backdrop, &p.look, kind, key);
        p.pieces.insert(key, (image.clone(), build));
        Some(image)
    });
    if let Some(image) = image {
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
        let _ = window.paint_image(bounds, outer, Corners::all(px(radius)), image, 0, false);
    }
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
