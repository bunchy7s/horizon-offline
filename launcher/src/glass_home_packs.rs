//! Home's Packs tab in the glass look (the mockup's `Packs`, `launcher-views.tsx`): one `ROW`
//! per resource or shader pack, its picture, "Resource pack · on" and a `Toggle` that turns it on
//! or off. Add pack… and Open folder head the list; packs dropped on it are added; Remove
//! (clicked twice) shows under the pointer. Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use glass_home_lists::{danger_button, danger_line, picture};
use gpui::ExternalPaths;
use horizon_launcher::packs::{self, Kind as PackKind, Pack};
use horizon_theme::glass::{fill, ink, radius};

impl Launcher {
    /// Home's Packs tab: the selected instance's resource packs, then its shader packs.
    pub(super) fn glass_packs(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let instance = self.selected_instance()?;
        let (home, name) = (instance.home.clone(), instance.name.clone());
        self.load_packs_list(home.clone());
        let resource = self.game_packs.resource.clone();
        let shader = self.game_packs.shader.clone();
        let count = resource.len() + shader.len();
        // The whole tab takes dropped packs, lit as a row under the pointer while one is over it.
        let mut body = div()
            .id("packs")
            .flex()
            .flex_col()
            .rounded(px(radius::ROW))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(white(fill::ROW_HOVER)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_packs(paths.paths().to_vec());
                // Not the window's import as well.
                cx.stop_propagation();
                cx.notify();
            }));
        let add = || {
            cx.listener(
                |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                    this.choose_packs(cx);
                    cx.notify();
                },
            )
        };
        if count == 0 {
            return Some(
                body.child(
                    ui::empty()
                        .flex()
                        .gap(px(4.))
                        .child(tr(
                            "No resource packs. Find them on the Mods page under Modrinth.",
                        ))
                        .child(ui::link("packs-add", tr("Add pack…")).on_click(add())),
                ),
            );
        }
        let note = match count {
            1 => trf("1 pack in {name}", &[("name", &name)]),
            _ => trf(
                "{count} packs in {name}",
                &[("count", &count.to_string()), ("name", &name)],
            ),
        };
        let folder = home.join(PackKind::Resource.folder());
        body = body.child(
            ui::list_head(note).child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::quiet("packs-folder", tr("Open folder"), 36., &t).on_click(
                            move |_, _, cx| {
                                let _ = std::fs::create_dir_all(&folder);
                                cx.open_with_system(&folder);
                            },
                        ),
                    )
                    .child(
                        ui::quiet_icon("packs-add", Some("plus-sign"), tr("Add pack…"), 36., &t)
                            .on_click(add()),
                    ),
            ),
        );
        match self.game_packs.note.clone() {
            Some(Ok(m)) => body = body.child(ui::list_head(m)),
            Some(Err(e)) => body = body.child(danger_line(e)),
            None => {}
        }
        if self.client.is_some() {
            body = body.child(ui::list_head(tr(
                "The game is running: change packs in its own menus, or here once it is closed.",
            )));
        }
        for (pack, icon) in resource {
            let row = self.glass_pack_row(pack, icon.map(gpui::ImageSource::from), cx);
            body = body.child(row);
        }
        for pack in shader {
            let row = self.glass_pack_row(pack, None, cx);
            body = body.child(row);
        }
        Some(body)
    }

    fn glass_pack_row(
        &mut self,
        pack: Pack,
        icon: Option<gpui::ImageSource>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let kind = match pack.kind {
            PackKind::Resource => "resource",
            PackKind::Shader => "shader",
        };
        let key = SharedString::from(format!("pack-{kind}-{}", pack.file));
        let hovered = self.list_hover.as_ref() == Some(&key);
        let confirm = (pack.kind, pack.file.clone());
        let confirming = self.game_packs.confirm_remove.as_ref() == Some(&confirm);
        let running = self.client.is_some();
        let hover = self.list_hover_listener(key.clone(), cx);
        let mut note = vec![match (pack.kind, pack.enabled) {
            (PackKind::Resource, true) => tr("Resource pack · on"),
            (PackKind::Resource, false) => tr("Resource pack · off"),
            (PackKind::Shader, true) => tr("Shader pack · on"),
            (PackKind::Shader, false) => tr("Shader pack · off"),
        }
        .to_string()];
        if !pack.description.is_empty() {
            note.push(pack.description.clone());
        }
        let remove = (!running && (hovered || confirming)).then(|| {
            if confirming {
                let gone = pack.clone();
                danger_button(
                    format!("{key}-trash").into(),
                    tr("Click again to remove").into(),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let gone = gone.clone();
                    this.pack_action(move |home| {
                        packs::remove(home, &gone)
                            .map(|()| trf("{name} moved to the trash.", &[("name", &gone.title)]))
                    });
                    cx.notify();
                }))
            } else {
                ui::icon_button(
                    SharedString::from(format!("{key}-remove")),
                    "delete-02",
                    36.,
                    17.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.game_packs.confirm_remove = Some(confirm.clone());
                    cx.notify();
                }))
            }
        });
        let toggle = ui::toggle(
            SharedString::from(format!("{key}-toggle")),
            pack.enabled,
            self.look.accent,
            self.look.on_accent,
            &t,
        );
        let toggle = if running {
            toggle.opacity(0.4).cursor_default()
        } else {
            let flip = pack.clone();
            toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let flip = flip.clone();
                this.pack_action(move |home| {
                    packs::set_enabled(home, &flip, !flip.enabled).map(|()| match flip.enabled {
                        true => trf("{name} is off.", &[("name", &flip.title)]),
                        false => trf("{name} is on.", &[("name", &flip.title)]),
                    })
                });
                cx.notify();
            }))
        };
        ui::list_row(key, hovered || confirming, hover, &t)
            .child(picture(icon, "package-01"))
            .child(ui::list_text(pack.title.clone(), note.join(" · ")))
            .children(remove.map(|r| div().relative().child(r)))
            .child(toggle.relative())
    }
}

impl Launcher {
    /// Home's Screenshots tab (the mockup's `Screenshots`): how many, then the pictures in a
    /// grid, 4 across from 1280 px (`xl:grid-cols-4`), else 2, each 16:9, round at 12, its file
    /// name under it. A click opens the viewer.
    pub(super) fn glass_screenshots(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_screenshots(home.clone());
        let list = self.screenshots.list.clone();
        if list.is_empty() {
            return Some(
                div().child(ui::empty().child(tr("No screenshots yet. Press F2 in the game."))),
            );
        }
        let count = match list.len() {
            1 => tr("1 screenshot").to_string(),
            n => trf("{count} screenshots", &[("count", &n.to_string())]),
        };
        let folder = horizon_launcher::screenshots::folder(&home);
        // In place of the mockup's blur toggle (the launcher has none yet): the folder, the same
        // 32 px `ICON_BUTTON`.
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(32.))
            .child(ui::text(13., FontWeight::NORMAL, ink::NOTE).child(count))
            .child(
                ui::icon_button(
                    "screenshots-folder",
                    "folder-01",
                    32.,
                    17.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&folder);
                    cx.open_with_system(&folder);
                }),
            );
        let shown = if self.screenshots.all {
            list.len()
        } else {
            screenshots_page::FIRST
        };
        let more = list.len() > screenshots_page::FIRST;
        let grid = div()
            .grid()
            .grid_cols(if self.window_width >= 1280. { 4 } else { 2 })
            .gap_x(px(12.))
            .gap_y(px(16.))
            .mt(px(12.))
            .children(list.into_iter().take(shown).enumerate().map(|(i, shot)| {
                div()
                    .id(SharedString::from(format!("screenshot-{i}")))
                    .min_w_0()
                    .cursor_pointer()
                    .child(
                        div()
                            .w_full()
                            .aspect_ratio(16. / 9.)
                            .rounded(px(radius::ROW))
                            .overflow_hidden()
                            .bg(white(fill::HOVER))
                            .child(
                                gpui::img(shot.path.clone())
                                    .size_full()
                                    .rounded(px(radius::ROW))
                                    .object_fit(gpui::ObjectFit::Cover),
                            ),
                    )
                    .child(
                        ui::text(12., FontWeight::NORMAL, ink::NOTE)
                            .line_height(px(18.))
                            .pt(px(8.))
                            .truncate()
                            .child(shot.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.screenshots.viewing = Some(i);
                        this.screenshots.confirm_delete = false;
                        this.screenshots.note = None;
                        cx.notify();
                    }))
            }));
        let toggle_all = more.then(|| {
            let label = match self.screenshots.all {
                true => tr("Show fewer").to_string(),
                false => trf(
                    "Show all {count}",
                    &[("count", &self.screenshots.list.len().to_string())],
                ),
            };
            div()
                .mt(px(16.))
                .child(
                    ui::quiet("screenshots-all", label, 36., &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.screenshots.all = !this.screenshots.all;
                            cx.notify();
                        },
                    )),
                )
        });
        let error = match (&self.screenshots.note, self.screenshots.viewing) {
            (Some(Err(e)), None) => Some(
                div().mt(px(8.)).child(
                    ui::text(13., FontWeight::NORMAL, ink::FULL)
                        .text_color(ui::color(horizon_theme::glass::DANGER))
                        .child(e.clone()),
                ),
            ),
            _ => None,
        };
        Some(
            div()
                .px(px(12.))
                .child(head)
                .children(error)
                .child(grid)
                .children(toggle_all),
        )
    }
}
