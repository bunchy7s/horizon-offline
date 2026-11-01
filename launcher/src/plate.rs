//! Shapes whose corners are cut in pixel steps, like the sky's art (docs/design-system.md, "Night").
//! The fill and the stroke are rectangles that don't overlap, so a translucent plate keeps one
//! opacity, and the stroke follows the steps.
use crate::theme::night::{self, Night};
use gpui::{div, prelude::*, px, rgba, Div, Rgba};

/// `c` at opacity `a`.
pub fn alpha(c: u32, a: f32) -> Rgba {
    rgba(c << 8 | (a * 255.).round() as u32)
}

/// A plate: `card` at its opacity, each corner cut in two [`night::PIXEL_CORNER`] steps, with its
/// 1 px `stroke` along the edge. Its content goes in as children, as on any div.
pub fn plate(n: &Night) -> Div {
    cut(
        night::PIXEL_CORNER,
        2,
        Some(alpha(n.card, n.card_alpha)),
        Some((alpha(n.stroke, n.stroke_alpha), 1.)),
    )
}

/// A box of any size whose corners are cut in `steps` steps of `step` px: `fill` inside, and a
/// `stroke` of the given colour and width along the edge.
pub fn cut(step: f32, steps: usize, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>) -> Div {
    let fill = fill
        .into_iter()
        .flat_map(|c| fill_rects(step, steps).into_iter().map(move |r| r.div(c)));
    let stroke = stroke.into_iter().flat_map(|(c, w)| {
        stroke_rects(step, steps, w)
            .into_iter()
            .map(move |r| r.div(c))
    });
    div().relative().children(fill.chain(stroke))
}

/// A distance from the box's start (left, top) or end (right, bottom) edge.
#[derive(Clone, Copy, Debug)]
enum Edge {
    Start(f32),
    End(f32),
}

/// A rectangle between two vertical and two horizontal edges, for a box whose size isn't known yet.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: (Edge, Edge),
    y: (Edge, Edge),
}

impl Rect {
    /// Measured from the corner that `right` and `bottom` name: `x`, `y` from it, `w` × `h`.
    fn at(right: bool, bottom: bool, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let span = |far: bool, a: f32, len: f32| {
            if far {
                (Edge::End(a + len), Edge::End(a))
            } else {
                (Edge::Start(a), Edge::Start(a + len))
            }
        };
        Rect {
            x: span(right, x, w),
            y: span(bottom, y, h),
        }
    }

    /// A band `h` tall, `y` from the top (or bottom), `inset` from both sides.
    fn across(inset: f32, bottom: bool, y: f32, h: f32) -> Rect {
        let mut r = Rect::at(false, bottom, 0., y, 0., h);
        r.x = (Edge::Start(inset), Edge::End(inset));
        r
    }

    /// A band `w` wide on the left (or right) edge, `inset` from the top and the bottom.
    fn down(inset: f32, right: bool, w: f32) -> Rect {
        let mut r = Rect::at(right, false, 0., 0., w, 0.);
        r.y = (Edge::Start(inset), Edge::End(inset));
        r
    }

    fn div(self, c: Rgba) -> Div {
        let d = div().absolute().bg(c);
        let d = match self.x {
            (Edge::Start(a), Edge::Start(b)) => d.left(px(a)).w(px(b - a)),
            (Edge::Start(a), Edge::End(b)) => d.left(px(a)).right(px(b)),
            (Edge::End(a), Edge::End(b)) => d.right(px(b)).w(px(a - b)),
            (Edge::End(_), Edge::Start(_)) => unreachable!("rectangles run start to end"),
        };
        match self.y {
            (Edge::Start(a), Edge::Start(b)) => d.top(px(a)).h(px(b - a)),
            (Edge::Start(a), Edge::End(b)) => d.top(px(a)).bottom(px(b)),
            (Edge::End(a), Edge::End(b)) => d.bottom(px(b)).h(px(a - b)),
            (Edge::End(_), Edge::Start(_)) => unreachable!("rectangles run start to end"),
        }
    }
}

/// The fill: a strip per step at the top and the bottom, each `step` further in, and the middle.
fn fill_rects(step: f32, steps: usize) -> Vec<Rect> {
    let deep = steps as f32 * step;
    let mut rects = Vec::new();
    for i in 0..steps {
        for bottom in [false, true] {
            rects.push(Rect::across(
                deep - i as f32 * step,
                bottom,
                i as f32 * step,
                step,
            ));
        }
    }
    rects.push(Rect {
        x: (Edge::Start(0.), Edge::End(0.)),
        y: (Edge::Start(deep), Edge::End(deep)),
    });
    rects
}

/// The stroke, `w` wide on the fill's outermost pixels: the four edges, and a riser and a tread
/// per step at each corner.
fn stroke_rects(step: f32, steps: usize, w: f32) -> Vec<Rect> {
    let deep = steps as f32 * step;
    let mut rects = Vec::new();
    for far in [false, true] {
        rects.push(Rect::across(deep, far, 0., w));
        rects.push(Rect::down(deep + w, far, w));
    }
    for right in [false, true] {
        for bottom in [false, true] {
            for i in 0..steps {
                let (i, x) = (i as f32, deep - i as f32 * step);
                rects.push(Rect::at(right, bottom, x, i * step + w, w, step));
                rects.push(Rect::at(right, bottom, x - step, (i + 1.) * step, step, w));
            }
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels `rects` cover in a `w` × `h` box, counting overlaps.
    fn raster(rects: &[Rect], w: usize, h: usize) -> Vec<Vec<u8>> {
        let at = |e: Edge, size: usize| match e {
            Edge::Start(a) => a as usize,
            Edge::End(a) => size - a as usize,
        };
        let mut grid = vec![vec![0u8; w]; h];
        for r in rects {
            for row in grid.iter_mut().take(at(r.y.1, h)).skip(at(r.y.0, h)) {
                for cell in row.iter_mut().take(at(r.x.1, w)).skip(at(r.x.0, w)) {
                    *cell += 1;
                }
            }
        }
        grid
    }

    #[test]
    fn fill_and_stroke_never_overlap_themselves() {
        for (step, steps, w) in [(4., 2, 1.), (4., 1, 1.), (4., 1, 2.), (2., 2, 1.)] {
            let fill = raster(&fill_rects(step, steps), 40, 30);
            let stroke = raster(&stroke_rects(step, steps, w), 40, 30);
            assert!(
                fill.iter().flatten().all(|&c| c <= 1),
                "fill {step}×{steps}"
            );
            assert!(
                stroke.iter().flatten().all(|&c| c <= 1),
                "stroke {step}×{steps} w{w}"
            );
        }
    }

    #[test]
    fn the_corners_are_cut_in_steps() {
        let fill = raster(&fill_rects(4., 2), 40, 30);
        let row = |y: usize| fill[y].iter().position(|&c| c == 1);
        assert_eq!(
            [row(0), row(4), row(8), row(29)],
            [Some(8), Some(4), Some(0), Some(8)]
        );
        assert_eq!(fill[0].iter().rposition(|&c| c == 1), Some(31));
    }

    #[test]
    fn the_stroke_is_the_fills_outer_edge() {
        // A 1 px stroke covers exactly the fill's pixels that touch the outside (or the box's edge)
        // on a side or a corner, and nothing outside the fill.
        for (step, steps) in [(4., 2), (4., 1), (2., 2)] {
            let (w, h) = (40, 30);
            let fill = raster(&fill_rects(step, steps), w, h);
            let stroke = raster(&stroke_rects(step, steps, 1.), w, h);
            let inside = |x: i32, y: i32| {
                x >= 0
                    && y >= 0
                    && (x as usize) < w
                    && (y as usize) < h
                    && fill[y as usize][x as usize] == 1
            };
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let edge = inside(x, y)
                        && [
                            (-1, 0),
                            (1, 0),
                            (0, -1),
                            (0, 1),
                            (-1, -1),
                            (1, -1),
                            (-1, 1),
                            (1, 1),
                        ]
                        .iter()
                        .any(|(dx, dy)| !inside(x + dx, y + dy));
                    assert_eq!(
                        stroke[y as usize][x as usize] == 1,
                        edge,
                        "{step}×{steps} at ({x}, {y})"
                    );
                }
            }
        }
    }
}
