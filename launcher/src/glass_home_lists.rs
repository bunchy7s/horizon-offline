//! Home's Worlds and Multiplayer tabs in the glass look (the mockup's `Worlds` and
//! `Multiplayer`, `launcher-views.tsx`): one `ROW` per world or server. A world's row plays it,
//! its size turning into "Play →" under the pointer; its ⋯ menu holds what the launcher does
//! besides (back up, copy, data packs, folder, delete). A server's row has Join, and Remove
//! (clicked twice) under the pointer. Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use gpui::{deferred, ExternalPaths};
use horizon_launcher::worlds::World;
use horizon_theme::glass::{fill, ink, radius};
use std::path::Path;
use worlds_page::{ago, Action};

/// A listener of the launcher's, boxed so a helper can return it.
pub(super) type Listener<E> = Box<dyn Fn(&E, &mut Window, &mut App)>;

/// The 36 px picture at a row's start (`PixelIcon` at 36), round at `round(36 · 0.26)`.
const PICTURE: f32 = 36.;
const PICTURE_RADIUS: f32 = 9.;

impl Launcher {
    /// Records whether the pointer is over the row `key`; the row then shows its actions.
    pub(super) fn list_hover_listener(
        &self,
        key: SharedString,
        cx: &mut Context<Self>,
    ) -> Listener<bool> {
        Box::new(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.list_hover = Some(key.clone());
            } else if this.list_hover.as_ref() == Some(&key) {
                this.list_hover = None;
            }
            cx.notify();
        }))
    }

    /// Home's Worlds tab: the selected instance's worlds, the last played first.
    pub(super) fn glass_worlds(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_worlds(home.clone(), cx);
        let list = self.worlds.list.clone();
        let mut body = div().flex().flex_col();
        // What the last back up, copy or delete did, where the mockup's toast would say it.
        match self.worlds.done.clone() {
            Some(Ok((message, zip))) => {
                let open = zip.map(|zip| {
                    let dir = zip.parent().map(Path::to_path_buf).unwrap_or(zip);
                    ui::quiet("world-backups", tr("Open backups"), 36., &t)
                        .on_click(move |_, _, cx| cx.open_with_system(&dir))
                });
                body = body.child(ui::list_head(message).children(open));
            }
            Some(Err(e)) => body = body.child(danger_line(e)),
            None => {}
        }
        if list.is_empty() {
            let saves = home.join("saves");
            return Some(
                body.child(
                    ui::empty()
                        .flex()
                        .gap(px(4.))
                        .child(tr(
                            "No worlds yet. Make one in the game and it shows up here.",
                        ))
                        .child(ui::link("worlds-folder", tr("Open folder")).on_click(
                            move |_, _, cx| {
                                let _ = std::fs::create_dir_all(&saves);
                                cx.open_with_system(&saves);
                            },
                        )),
                ),
            );
        }
        // Pictures line up: a world without one gets a plain tile when another has one.
        let pictures = list.iter().any(|w| w.icon.is_some());
        for w in list {
            let row = self.glass_world_row(&w, pictures, cx);
            body = body.child(row).children(self.world_packs_panel(&w, cx));
        }
        Some(body)
    }

    fn glass_world_row(&mut self, w: &World, pictures: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let key = SharedString::from(format!("world-{}", w.folder));
        let hovered = self.list_hover.as_ref() == Some(&key);
        let menu_open = self.list_menu.as_ref().is_some_and(|(k, _)| *k == key);
        let busy = self.worlds.busy.is_some();
        let this_busy = self.worlds.busy.as_ref() == Some(&w.folder);
        let can_play = self.can_start_selected() && !busy;
        let mut facts = vec![tr(w.mode).to_string()];
        facts.extend(w.version.clone());
        facts.push(ago(w.last_played));
        let hover = self.list_hover_listener(key.clone(), cx);
        let mut row = ui::list_row(key.clone(), hovered || menu_open, hover, &t);
        if can_play {
            let folder = w.folder.clone();
            row = row
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.quick_play("--world", folder.clone());
                    cx.notify();
                }));
        }
        // Data packs dropped on a world go into it.
        let path = w.path.clone();
        row = row
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(white(fill::SELECTED)))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.add_world_packs(path.clone(), paths.paths().to_vec());
                cx.stop_propagation();
                cx.notify();
            }));
        if pictures {
            row = row.child(picture(
                w.icon.clone().map(gpui::ImageSource::from),
                "earth",
            ));
        }
        let size = if this_busy {
            ui::text(13., FontWeight::NORMAL, ink::QUIET).child(tr("Working…"))
        } else if hovered && can_play {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(ui::text(13., FontWeight::MEDIUM, ink::FULL).child(tr("Play")))
                .child(ui::icon("arrow-right-01", 15., white(ink::FULL)))
        } else {
            ui::text(13., FontWeight::NORMAL, ink::QUIET).child(megabytes(w.size))
        };
        let more = ui::icon_button(
            SharedString::from(format!("{key}-more")),
            "more-horizontal",
            36.,
            17.,
            radius::CONTROL,
            menu_open,
            &t,
        )
        .on_click(cx.listener({
            let key = key.clone();
            move |this, event: &ClickEvent, _, cx| {
                if !this.closed_by_this_press(event) {
                    this.list_menu = match this.list_menu.take() {
                        Some((k, _)) if k == key => None,
                        _ => Some((key.clone(), false)),
                    };
                }
                cx.stop_propagation();
                cx.notify();
            }
        }));
        // Under the pointer only, as the mockup's row actions: the size keeps the row's end.
        let mut more = div().relative().child(more);
        if menu_open {
            more = more.child(
                deferred(
                    div()
                        .absolute()
                        .top(px(44.))
                        .right_0()
                        .child(self.world_menu(w, cx)),
                )
                .with_priority(1),
            );
        }
        div().child(
            row.child(ui::list_text(w.name.clone(), facts.join(" · ")))
                .child(size.relative())
                .when(hovered || menu_open, |r| r.child(more)),
        )
    }

    /// A world's ⋯ menu: Back up, Copy, its data packs, Open folder, then Delete… (asked twice).
    fn world_menu(&mut self, w: &World, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let confirm = self.list_menu.as_ref().is_some_and(|(_, c)| *c);
        let act = |this: &mut Self, action: Action, w: &World, cx: &mut Context<Self>| {
            this.list_menu = None;
            this.world_action(action, w.clone(), cx);
            cx.notify();
        };
        let (backup, copy, packs, gone) = (w.clone(), w.clone(), w.clone(), w.clone());
        let folder = w.path.clone();
        let packs_label = if self.world_packs_open(w) {
            tr("Hide data packs")
        } else {
            tr("Data packs")
        };
        let delete = ui::menu_row(
            "world-menu-delete",
            "delete-02",
            if confirm {
                tr("Click again to delete")
            } else {
                tr("Delete…")
            },
            true,
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            match this.list_menu.as_mut() {
                Some((_, confirm)) if !*confirm => *confirm = true,
                _ => act(this, Action::Delete, &gone, cx),
            }
            cx.notify();
        }));
        ui::popover(230.)
            .id("world-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.list_menu = None;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }))
            .child(
                ui::menu_row("world-menu-backup", "download-04", tr("Back up"), false, &t)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        act(this, Action::Backup, &backup, cx)
                    })),
            )
            .child(
                ui::menu_row("world-menu-copy", "copy-01", tr("Copy"), false, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        act(this, Action::Copy, &copy, cx)
                    }),
                ),
            )
            .child(
                ui::menu_row("world-menu-packs", "package-01", packs_label, false, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.list_menu = None;
                        this.toggle_world_packs(&packs);
                        cx.notify();
                    }),
                ),
            )
            .child(
                ui::menu_row(
                    "world-menu-folder",
                    "folder-01",
                    tr("Open folder"),
                    false,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.list_menu = None;
                    cx.open_with_system(&folder);
                    cx.notify();
                })),
            )
            .child(div().mt(px(4.)).child(delete))
    }

    /// Home's Multiplayer tab: the selected instance's saved servers (the game's own list), and
    /// a line to add one.
    pub(super) fn glass_multiplayer(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?;
        let (home, name) = (instance.home.clone(), instance.name.clone());
        self.load_servers(home);
        let list = self.multiplayer.list.clone();
        let adding = self.multiplayer.adding;
        let mut body = div().flex().flex_col();
        if list.is_empty() && !adding {
            return Some(
                body.child(
                    ui::empty()
                        .flex()
                        .gap(px(4.))
                        .child(tr("No servers saved in this instance."))
                        .child(ui::link("server-add", tr("Add server")).on_click(toggle_add(cx))),
                ),
            );
        }
        let count = list.len().to_string();
        let note = match list.len() {
            1 => trf("1 server in {name}", &[("name", &name)]),
            _ => trf(
                "{count} servers in {name}",
                &[("count", &count), ("name", &name)],
            ),
        };
        body = body.child(
            ui::list_head(note).child(
                ui::quiet_icon("server-add", Some("plus-sign"), tr("Add server"), 36., &t)
                    .on_click(toggle_add(cx)),
            ),
        );
        if let Some(e) = self.multiplayer.error.clone() {
            body = body.child(danger_line(e));
        }
        if adding {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .pb(px(8.))
                    .child(self.field(
                        servers_page::Field::ServerName,
                        "Minecraft Server",
                        220.,
                        cx,
                    ))
                    .child(self.field(
                        servers_page::Field::ServerAddress,
                        "play.example.com",
                        300.,
                        cx,
                    ))
                    .child(ui::solid("server-save", tr("Add"), 44., 14., &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.add_server();
                            cx.notify();
                        }),
                    )),
            );
        }
        let pictures = list.iter().any(|(_, icon)| icon.is_some());
        for (i, (server, icon)) in list.into_iter().enumerate() {
            body = body.child(self.glass_server_row(i, server, icon, pictures, cx));
        }
        Some(body)
    }

    fn glass_server_row(
        &mut self,
        i: usize,
        server: horizon_launcher::server_list::Server,
        icon: Option<std::sync::Arc<gpui::Image>>,
        pictures: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let key = SharedString::from(format!("server-{i}"));
        let hovered = self.list_hover.as_ref() == Some(&key);
        let confirming = self.multiplayer.confirm_remove == Some(i);
        let hover = self.list_hover_listener(key.clone(), cx);
        let ip = server.ip.clone();
        let join = ui::quiet(
            SharedString::from(format!("{key}-join")),
            tr("Join"),
            36.,
            &t,
        );
        let join = if self.can_join_selected() {
            join.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.quick_play("--server", ip.clone());
                cx.notify();
            }))
        } else {
            join.opacity(0.4).cursor_default()
        };
        let remove = (hovered || confirming).then(|| {
            if confirming {
                danger_button(
                    SharedString::from(format!("{key}-remove")),
                    tr("Click again to remove").into(),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit_servers(|list| list.remove(i).map(|_| ()));
                    cx.notify();
                }))
                .into_any_element()
            } else {
                div()
                    .child(
                        ui::icon_button(
                            SharedString::from(format!("{key}-delete")),
                            "delete-02",
                            36.,
                            17.,
                            radius::CONTROL,
                            false,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.multiplayer.confirm_remove = Some(i);
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element()
            }
        });
        ui::list_row(key, hovered || confirming, hover, &t)
            .when(pictures, |r| {
                r.child(picture(icon.map(gpui::ImageSource::from), "globe-02"))
            })
            .child(ui::list_text(server.name, server.ip))
            .child(join.relative())
            .children(remove.map(|r| div().relative().child(r)))
    }
}

/// Add server: opens or closes the line to add one, its address field focused.
fn toggle_add(cx: &mut Context<Launcher>) -> Listener<ClickEvent> {
    Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
        this.multiplayer.adding = !this.multiplayer.adding;
        this.multiplayer.error = None;
        if this.multiplayer.adding {
            this.servers.focus_soon = Some(servers_page::Field::ServerAddress);
        }
        cx.notify();
    }))
}

/// A row's 36 px picture, or a plain tile with `fallback`'s icon.
pub(super) fn picture(image: Option<gpui::ImageSource>, fallback: &'static str) -> AnyElement {
    match image {
        Some(image) => gpui::img(image)
            .relative()
            .flex_none()
            .size(px(PICTURE))
            .rounded(px(PICTURE_RADIUS))
            .into_any_element(),
        None => div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(PICTURE))
            .rounded(px(PICTURE_RADIUS))
            .bg(white(fill::BUTTON))
            .child(ui::icon(fallback, 17., white(ink::QUIET)))
            .into_any_element(),
    }
}

/// Why an action failed, in the menu's Delete red, over the list.
pub(super) fn danger_line(message: String) -> Div {
    div().px(px(12.)).pb(px(8.)).child(
        ui::text(13., FontWeight::NORMAL, ink::FULL)
            .text_color(ui::color(horizon_theme::glass::DANGER))
            .child(message),
    )
}

/// A destructive button's second step (the menu's "Click again to delete"): its red text on
/// red at 12 % under the pointer.
pub(super) fn danger_button(id: SharedString, label: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(36.))
        .px(px(12.))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(|s| s.bg(gpui::rgba(horizon_theme::glass::DANGER_HOVER << 8 | 0x1f)))
        .child(
            ui::text(13., FontWeight::MEDIUM, ink::FULL)
                .text_color(ui::color(horizon_theme::glass::DANGER))
                .child(label),
        )
}
