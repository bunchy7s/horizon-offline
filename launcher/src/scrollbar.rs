//! Pointer-operated scrollbars for the launcher's vertical scroll areas.
use crate::theme::Theme;
use gpui::{
    canvas, div, fill, point, prelude::*, px, size, Bounds, Div, HitboxBehavior, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle, Stateful,
};
use std::{cell::Cell, rc::Rc};

/// Space reserved beside scroll content for the track and its padding.
pub const GUTTER: f32 = 16.;

#[derive(Clone, Default)]
pub(crate) struct Scrollbar {
    pub handle: ScrollHandle,
    /// Distance from the top of the thumb to the point grabbed by the pointer.
    grabbed: Rc<Cell<Option<f32>>>,
}

/// Geometry in logical pixels. A minimum thumb size keeps long lists easy to grab.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    thumb: f32,
    travel: f32,
    top: f32,
    max: f32,
}

impl Geometry {
    fn new(height: f32, viewport: f32, max: f32, offset: f32) -> Option<Self> {
        if height <= 0. || viewport <= 0. || max <= 0. {
            return None;
        }
        let thumb = (height * viewport / (viewport + max)).max(32_f32.min(height / 2.));
        let travel = height - thumb;
        Some(Self {
            thumb,
            travel,
            top: (-offset / max).clamp(0., 1.) * travel,
            max,
        })
    }

    fn offset(self, thumb_top: f32) -> f32 {
        if self.travel <= 0. {
            return 0.;
        }
        -(thumb_top / self.travel).clamp(0., 1.) * self.max
    }
}

impl Scrollbar {
    /// Wrap the scroll container, reserving a narrow gutter so the bar never covers controls.
    pub fn wrap(&self, content: Stateful<Div>, theme: &Theme) -> Div {
        let state = self.clone();
        let track_color = theme.well;
        let thumb_color = theme.muted;
        div()
            .relative()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .child(
                content
                    .track_scroll(&self.handle)
                    .mr(px(GUTTER))
                    // Nested scroll areas must not also move the page behind them.
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation()),
            )
            .child(
                canvas(
                    move |bounds, window, _| {
                        let geometry = Geometry::new(
                            bounds.size.height.into(),
                            state.handle.bounds().size.height.into(),
                            state.handle.max_offset().height.into(),
                            state.handle.offset().y.into(),
                        );
                        let hitbox =
                            geometry.map(|_| window.insert_hitbox(bounds, HitboxBehavior::Normal));
                        (state, geometry, hitbox)
                    },
                    move |bounds, (state, geometry, hitbox), window, _| {
                        let Some(g) = geometry else {
                            state.grabbed.set(None);
                            return;
                        };
                        let hitbox = hitbox.unwrap();
                        window.paint_quad(fill(bounds, track_color));
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left() + px(3.), bounds.top() + px(g.top)),
                                size(bounds.size.width - px(6.), px(g.thumb)),
                            ),
                            thumb_color,
                        ));
                        window.on_mouse_event({
                            let state = state.clone();
                            move |ev: &MouseDownEvent, phase, window, cx| {
                                if !phase.bubble()
                                    || ev.button != MouseButton::Left
                                    || !hitbox.is_hovered(window)
                                {
                                    return;
                                }
                                let y = f32::from(ev.position.y - bounds.top());
                                let grab = if (g.top..=g.top + g.thumb).contains(&y) {
                                    y - g.top
                                } else {
                                    // Clicking the track centers the thumb at the pointer.
                                    let grab = g.thumb / 2.;
                                    let mut offset = state.handle.offset();
                                    offset.y = px(g.offset(y - grab));
                                    state.handle.set_offset(offset);
                                    grab
                                };
                                state.grabbed.set(Some(grab));
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                        window.on_mouse_event({
                            let state = state.clone();
                            move |ev: &MouseMoveEvent, phase, window, cx| {
                                if !phase.capture() {
                                    return;
                                }
                                let Some(grab) = state.grabbed.get() else {
                                    return;
                                };
                                if ev.pressed_button != Some(MouseButton::Left) {
                                    state.grabbed.set(None);
                                    return;
                                }
                                let mut offset = state.handle.offset();
                                offset.y =
                                    px(g.offset(f32::from(ev.position.y - bounds.top()) - grab));
                                state.handle.set_offset(offset);
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                        window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                            if phase.capture()
                                && ev.button == MouseButton::Left
                                && state.grabbed.take().is_some()
                            {
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                    },
                )
                .absolute()
                .right(px(2.))
                .top_0()
                .bottom_0()
                .w(px(12.)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::Geometry;

    #[test]
    fn hides_without_overflow() {
        assert!(Geometry::new(200., 200., 0., 0.).is_none());
        assert!(Geometry::new(0., 200., 100., 0.).is_none());
    }

    #[test]
    fn thumb_tracks_wheel_offsets_and_pointer_reaches_both_ends() {
        let g = Geometry::new(200., 200., 600., -300.).unwrap();
        assert_eq!(g.thumb, 50.);
        assert_eq!(g.top, 75.);
        assert_eq!(g.offset(g.top), -300.);
        assert_eq!(g.offset(-100.), 0.);
        assert_eq!(g.offset(1000.), -600.);
    }

    #[test]
    fn long_lists_and_small_viewports_keep_geometry_bounded() {
        let g = Geometry::new(200., 200., 20000., -20000.).unwrap();
        assert_eq!(g.thumb, 32.);
        assert_eq!(g.top, g.travel);
        assert_eq!(g.offset(g.travel), -20000.);
        let tiny = Geometry::new(20., 20., 1000., -500.).unwrap();
        assert_eq!(tiny.thumb, 10.);
        assert_eq!(tiny.offset(10.), -1000.);
    }
}
