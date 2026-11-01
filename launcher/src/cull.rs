//! Variable-height lists in the page's scroll area. Only nearby rows build elements;
//! unseen rows use estimated heights and consecutive hidden rows share one spacer.
use gpui::{
    div, point, px, size, Bounds, Div, IntoElement, ParentElement, Pixels, ScrollHandle, Styled,
};
use std::{cell::RefCell, rc::Rc};

const ESTIMATED_HEIGHT: f32 = 96.;
/// Until the enclosing scroll area completes its first layout.
const INITIAL_VIEW_HEIGHT: f32 = 800.;

#[derive(Clone, Default)]
pub struct Cull(Rc<RefCell<Drawn>>);

#[derive(Default)]
struct Drawn {
    heights: Vec<Pixels>,
    top: Pixels,
    offset: Pixels,
    width: Pixels,
}

#[derive(Debug)]
enum Span {
    Row(usize),
    /// Hidden rows, this tall together.
    Space(Pixels),
}

impl Cull {
    /// Invalidate measurements when filtering, ordering or collection identity changes.
    pub fn reset(&self) {
        *self.0.borrow_mut() = Drawn::default();
    }

    /// Render nearby rows plus those retained by `keep` (such as active confirmations).
    /// `gap` is the bottom margin already supplied by each row.
    pub fn column<E: IntoElement>(
        &self,
        handle: &ScrollHandle,
        rows: usize,
        gap: f32,
        keep: impl Fn(usize) -> bool,
        mut row: impl FnMut(usize) -> E,
    ) -> Div {
        let max = handle.max_offset().y;
        if max > px(0.)
            && handle.offset().y <= -max + px(0.5)
            && self.0.borrow().heights.len() == rows
        {
            // Measuring taller rows at the end must keep the final row in view. Appending
            // results deliberately leaves the current viewport in place instead.
            handle.scroll_to_bottom();
        }
        let spans = self
            .0
            .borrow_mut()
            .plan(handle.bounds(), handle.offset().y, rows, gap, keep);
        let children = spans
            .iter()
            .map(|span| match span {
                // The wrapper includes the row's margin in its measured height.
                Span::Row(index) => div().flex().flex_col().flex_none().child(row(*index)),
                Span::Space(height) => div().flex_none().h(*height),
            })
            .collect::<Vec<_>>();
        let state = self.0.clone();
        let scroll = handle.clone();
        div()
            .flex()
            .flex_col()
            .children(children)
            .on_children_prepainted(move |bounds, window, _| {
                if state
                    .borrow_mut()
                    .record(&spans, &bounds, scroll.bounds(), scroll.offset().y)
                {
                    window.refresh();
                }
            })
    }
}

impl Drawn {
    fn plan(
        &mut self,
        mut view: Bounds<Pixels>,
        offset: Pixels,
        rows: usize,
        gap: f32,
        keep: impl Fn(usize) -> bool,
    ) -> Vec<Span> {
        let estimate = px(ESTIMATED_HEIGHT + gap);
        if self.width != view.size.width {
            // A resize changes wrapping, including in rows currently off screen.
            self.heights.fill(estimate);
        }
        self.heights.resize(rows, estimate);
        if view.size.height <= px(0.) {
            view.size.height = px(INITIAL_VIEW_HEIGHT);
        }
        let mut top = self.top + offset - self.offset;
        let mut spans = Vec::new();
        for (index, height) in self.heights.iter().copied().enumerate() {
            let bounds = Bounds::new(point(view.left(), top), size(view.size.width, height));
            if !away(bounds, view) || keep(index) {
                spans.push(Span::Row(index));
            } else if let Some(Span::Space(total)) = spans.last_mut() {
                *total += height;
            } else {
                spans.push(Span::Space(height));
            }
            top += height;
        }
        spans
    }

    /// A moved header or scrollbar jump can expose a spacer: request another frame when
    /// measurements or the estimated visible range need correcting.
    fn record(
        &mut self,
        spans: &[Span],
        bounds: &[Bounds<Pixels>],
        view: Bounds<Pixels>,
        offset: Pixels,
    ) -> bool {
        let mut changed = false;
        for (span, bounds) in spans.iter().zip(bounds) {
            match span {
                Span::Row(index) => {
                    let height = &mut self.heights[*index];
                    changed |= (*height - bounds.size.height).abs() > px(0.5);
                    *height = bounds.size.height;
                }
                Span::Space(_) => changed |= !away(*bounds, view),
            }
        }
        self.top = bounds.first().map_or(view.top(), Bounds::top);
        self.offset = offset;
        self.width = view.size.width;
        changed
    }
}

/// One viewport of overdraw above and below the visible area.
fn away(row: Bounds<Pixels>, view: Bounds<Pixels>) -> bool {
    row.bottom() < view.top() - view.size.height || row.top() > view.bottom() + view.size.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(top: f32, height: f32, width: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(width), px(height)))
    }

    fn rendered(spans: &[Span]) -> Vec<usize> {
        spans
            .iter()
            .filter_map(|span| match span {
                Span::Row(index) => Some(*index),
                Span::Space(_) => None,
            })
            .collect()
    }

    #[test]
    fn ten_thousand_rows_build_only_nearby_elements_on_the_first_frame() {
        let list = Cull::default();
        let mut count = 0;
        list.column(
            &ScrollHandle::new(),
            10_000,
            8.,
            |_| false,
            |_| {
                count += 1;
                div().h(px(96.)).mb(px(8.))
            },
        );
        assert!(count <= 17, "built {count} rows before the first layout");
        let spans = list
            .0
            .borrow_mut()
            .plan(at(0., 800., 600.), px(0.), 10_000, 8., |_| false);
        assert!(spans.len() <= count + 1, "hidden rows share one spacer");
    }

    #[test]
    fn large_scroll_jumps_reach_the_last_rows_and_return_to_the_first() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 10_000, 8., |_| false);
        state.width = view.size.width;
        let end = state.plan(view, px(-1_039_500.), 10_000, 8., |_| false);
        assert!(rendered(&end).contains(&9_999));
        assert!(rendered(&end).len() < 20);
        assert_eq!(
            rendered(&state.plan(view, px(0.), 10_000, 8., |_| false))[0],
            0
        );
    }

    #[test]
    fn resizing_and_appending_keep_render_work_bounded_and_retained_rows_available() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 5_000, 8., |_| false);
        state.width = view.size.width;
        state.heights[100] = px(900.);
        let spans = state.plan(at(0., 500., 300.), px(0.), 10_000, 8., |index| {
            index == 9_000
        });
        assert_eq!(state.heights[100], px(104.));
        assert!(rendered(&spans).contains(&9_000));
        assert!(spans.len() < 20);
        assert_eq!(state.heights.len(), 10_000);
    }

    #[test]
    fn measurements_include_wrapping_and_reveal_spacers_after_headers_move() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 100, 8., |_| false);
        let spans = vec![Span::Row(0), Span::Space(px(10_296.))];
        assert!(state.record(
            &spans,
            &[at(0., 240., 600.), at(240., 10_296., 600.)],
            view,
            px(0.)
        ));
        assert_eq!(state.heights[0], px(240.));
        assert!(rendered(&state.plan(view, px(0.), 100, 8., |_| false)).contains(&1));
        let list = Cull::default();
        list.0.borrow_mut().heights = vec![px(400.); 100];
        list.reset();
        assert!(list.0.borrow().heights.is_empty());
    }
}

#[cfg(all(test, target_os = "windows"))]
mod desktop_tests {
    use super::*;
    use gpui::{prelude::*, AppContext, Context, Render, Window, WindowBounds, WindowOptions};
    use std::time::Duration;

    struct ListWindow {
        list: Cull,
        scroll: ScrollHandle,
        rendered: Vec<usize>,
    }

    impl Render for ListWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.rendered.clear();
            let rows = self.list.column(
                &self.scroll,
                10_000,
                8.,
                |_| false,
                |index| {
                    self.rendered.push(index);
                    div()
                        .h(px(if index % 2 == 0 { 72. } else { 180. }))
                        .mb(px(8.))
                        .child(format!("Row {index}"))
                },
            );
            div()
                .id("viewport")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(div().pt(px(320.)).child(rows))
        }
    }

    #[test]
    #[ignore = "requires an interactive desktop and graphics adapter"]
    fn desktop_list_renders_the_viewport_after_scrollbar_jumps() {
        gpui_platform::application().run(|cx| {
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(600.), px(500.)),
                        ))),
                        ..WindowOptions::default()
                    },
                    |_, cx| {
                        cx.new(|_| ListWindow {
                            list: Cull::default(),
                            scroll: ScrollHandle::new(),
                            rendered: Vec::new(),
                        })
                    },
                )
                .unwrap();
            cx.spawn(async move |cx| {
                for step in 0..3 {
                    cx.background_executor()
                        .timer(Duration::from_millis(300))
                        .await;
                    window
                        .update(cx, |view, _, cx| {
                            assert!(
                                view.rendered.len() < 40,
                                "rendered {} rows",
                                view.rendered.len()
                            );
                            let wanted = if step == 1 { 9_999 } else { 0 };
                            assert!(
                                view.rendered.contains(&wanted),
                                "row {wanted} missing: {:?}",
                                view.rendered
                            );
                            let measured = view.list.0.borrow();
                            let mut top = measured.top;
                            for (index, height) in measured.heights.iter().enumerate() {
                                if top < view.scroll.bounds().bottom()
                                    && top + *height > view.scroll.bounds().top()
                                {
                                    assert!(
                                        view.rendered.contains(&index),
                                        "visible row {index} was a spacer"
                                    );
                                }
                                top += *height;
                            }
                            if step == 1 {
                                assert!(
                                    top <= view.scroll.bounds().bottom() + px(1.),
                                    "the last row stayed below the viewport by {:?}",
                                    top - view.scroll.bounds().bottom()
                                );
                            }
                            drop(measured);
                            let offset = if step == 0 {
                                -view.scroll.max_offset().y
                            } else {
                                px(0.)
                            };
                            view.scroll.set_offset(point(px(0.), offset));
                            cx.notify();
                        })
                        .unwrap();
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
