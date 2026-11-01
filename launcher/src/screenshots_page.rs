//! The selected instance's screenshots on Home (`horizon_launcher::screenshots`): the newest as
//! thumbnails, all of them on request, and a viewer (← and → step through) with Copy, Open,
//! Show in folder and Delete (to the trash, after a confirmation).
use super::*;
use gpui::{ClipboardItem, Image, ImageFormat};
use horizon_launcher::screenshots::{self, Shot};
use std::path::PathBuf;

/// Thumbnails shown before "Show all".
const FIRST: usize = 8;
const THUMB_W: f32 = 196.;
const THUMB_H: f32 = THUMB_W * 9. / 16.;

#[derive(Default)]
pub(super) struct Screenshots {
    home: Option<PathBuf>,
    list: Vec<Shot>,
    fresh: bool,
    all: bool,
    /// The picture open in the viewer, by its place in `list`.
    viewing: Option<usize>,
    confirm_delete: bool,
    /// What the last action did, or why it failed.
    note: Option<Result<String, String>>,
}

impl Screenshots {
    /// Read again on the next frame: the game may have taken new ones.
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

impl Launcher {
    fn load_screenshots(&mut self, home: PathBuf) {
        if self.screenshots.home.as_ref() != Some(&home) {
            self.screenshots = Screenshots {
                home: Some(home.clone()),
                ..Screenshots::default()
            };
        }
        if !self.screenshots.fresh {
            // One directory listing: cheap enough while drawing.
            self.screenshots.list = screenshots::list(&home);
            self.screenshots.fresh = true;
            let last = self.screenshots.list.len().checked_sub(1);
            self.screenshots.viewing = self
                .screenshots
                .viewing
                .and_then(|i| last.map(|l| i.min(l)));
        }
    }

    /// Escape closes the viewer. Whether it did.
    pub(super) fn screenshot_escape(&mut self) -> bool {
        let open = self.screenshots.viewing.take().is_some();
        self.screenshots.confirm_delete = false;
        open
    }

    /// ← and → in the viewer: the newer and the older picture. Whether it moved.
    pub(super) fn screenshot_step(&mut self, older: bool) -> bool {
        let Some(i) = self.screenshots.viewing else {
            return false;
        };
        let next = match older {
            true => (i + 1 < self.screenshots.list.len()).then_some(i + 1),
            false => i.checked_sub(1),
        };
        if let Some(n) = next {
            self.screenshots.viewing = Some(n);
            self.screenshots.confirm_delete = false;
            self.screenshots.note = None;
        }
        true
    }

    fn copy_screenshot(&mut self, shot: &Shot, cx: &mut Context<Self>) {
        let format = match shot.path.extension().and_then(|x| x.to_str()) {
            Some(x) if x.eq_ignore_ascii_case("png") => ImageFormat::Png,
            _ => ImageFormat::Jpeg,
        };
        self.screenshots.note = Some(match std::fs::read(&shot.path) {
            Ok(bytes) => {
                copy_to_clipboard(
                    cx,
                    ClipboardItem::new_image(&Image::from_bytes(format, bytes)),
                );
                Ok("Copied to the clipboard.".into())
            }
            Err(e) => Err(format!("Could not read {}: {e}", shot.name)),
        });
    }

    fn delete_screenshot(&mut self, shot: &Shot) {
        self.screenshots.confirm_delete = false;
        self.screenshots.note =
            Some(screenshots::delete(shot).map(|()| format!("{} moved to the trash.", shot.name)));
        self.screenshots.stale();
    }

    /// Home's Screenshots section, for the selected instance; None while it has none.
    pub(super) fn screenshots_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_screenshots(home.clone());
        let list = self.screenshots.list.clone();
        if list.is_empty() {
            return None;
        }
        let shown = if self.screenshots.all {
            list.len()
        } else {
            FIRST
        };
        let folder = screenshots::folder(&home);
        let mut buttons = div().flex().items_center().gap(px(6.));
        if list.len() > FIRST {
            let label = match self.screenshots.all {
                true => "Show fewer".to_string(),
                false => format!("Show all {}", list.len()),
            };
            buttons = buttons.child(button("screenshots-all", label, Kind::Ghost, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.screenshots.all = !this.screenshots.all;
                    cx.notify();
                }),
            ));
        }
        buttons = buttons.child(
            button("screenshots-folder", "Open folder", Kind::Ghost, &t)
                .on_click(move |_, _, cx| cx.open_with_system(&folder)),
        );
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(4.))
            .child(section_title("Screenshots", "image", &t))
            .child(buttons);
        let grid = div().flex().flex_wrap().gap(px(10.)).children(
            list.into_iter().take(shown).enumerate().map(|(i, shot)| {
                div()
                    .id(SharedString::from(format!("screenshot-{i}")))
                    .flex_none()
                    .w(px(THUMB_W))
                    .h(px(THUMB_H))
                    .border_2()
                    .border_color(t.outline)
                    .bg(t.well)
                    .cursor_pointer()
                    .hover(|s| s.border_color(t.accent))
                    .child(
                        gpui::img(shot.path.clone())
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.screenshots.viewing = Some(i);
                        this.screenshots.confirm_delete = false;
                        this.screenshots.note = None;
                        cx.notify();
                    }))
            }),
        );
        let mut section = div().flex().flex_col().gap(px(8.)).child(header);
        if self.screenshots.viewing.is_none() {
            if let Some(Err(e)) = &self.screenshots.note {
                section = section.child(notice(e.clone(), &t));
            }
        }
        Some(section.child(grid))
    }

    /// The viewer over the whole window, while a picture is open.
    pub(super) fn screenshot_viewer(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let i = self.screenshots.viewing?;
        let shot = self.screenshots.list.get(i)?.clone();
        let count = self.screenshots.list.len();
        let taken: chrono::DateTime<chrono::Local> = shot.taken.into();
        let step = |id: &'static str, label: &'static str, older: bool, can: bool| {
            let b = button(id, label, Kind::Ghost, &t);
            if can {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.screenshot_step(older);
                    cx.notify();
                }))
            } else {
                disabled(b)
            }
        };
        let mut actions = div().flex().items_center().gap(px(6.));
        if self.screenshots.confirm_delete {
            let gone = shot.clone();
            actions =
                actions
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.danger)
                            .child("Move it to the trash?"),
                    )
                    .child(button("screenshot-keep", "Keep", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.screenshots.confirm_delete = false;
                            cx.notify();
                        }),
                    ))
                    .child(
                        button("screenshot-trash", "Move to trash", Kind::Secondary, &t).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.delete_screenshot(&gone);
                                cx.notify();
                            }),
                        ),
                    );
        } else {
            let (copy, open, show) = (shot.clone(), shot.path.clone(), shot.path.clone());
            actions =
                actions
                    .child(button("screenshot-copy", "Copy", Kind::Ghost, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.copy_screenshot(&copy, cx);
                            cx.notify();
                        }),
                    ))
                    .child(
                        button("screenshot-open", "Open", Kind::Ghost, &t)
                            .on_click(move |_, _, cx| cx.open_with_system(&open)),
                    )
                    .child(
                        button("screenshot-show", "Show in folder", Kind::Ghost, &t)
                            .on_click(move |_, _, cx| cx.reveal_path(&show)),
                    )
                    .child(
                        button("screenshot-delete", "Delete", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.screenshots.confirm_delete = true;
                                cx.notify();
                            }),
                        ),
                    );
        }
        let note = match &self.screenshots.note {
            Some(Ok(m)) => Some(text(theme::CAPTION).text_color(t.muted).child(m.clone())),
            Some(Err(e)) => Some(text(theme::CAPTION).text_color(t.danger).child(e.clone())),
            None => None,
        };
        let top = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(17., FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .child(taken.format("%Y-%m-%d %H:%M:%S").to_string()),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .truncate()
                            .child(format!("{} · {} of {count}", shot.name, i + 1)),
                    ),
            )
            .child(step("screenshot-newer", "← Newer", false, i > 0))
            .child(step("screenshot-older", "Older →", true, i + 1 < count))
            .child(
                button("screenshot-close", "Close", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.screenshot_escape();
                        cx.notify();
                    },
                )),
            );
        let bottom = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(div().children(note))
            .child(actions);
        Some(
            div()
                .id("screenshot-viewer")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(14.))
                .px(px(32.))
                .py(px(24.))
                .bg(t.page.opacity(0.96))
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .occlude()
                .child(top)
                .child(
                    // The picture fills what the bars leave, whatever its own size.
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .border_2()
                        .border_color(t.outline)
                        .bg(t.well)
                        .child(
                            gpui::img(shot.path.clone())
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .object_fit(gpui::ObjectFit::Contain),
                        ),
                )
                .child(bottom),
        )
    }
}
