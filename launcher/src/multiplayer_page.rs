//! The selected instance's server list on Home (`horizon_launcher::server_list`, the game's own
//! `servers.dat`): each server with the picture the game last saw, Join (the game starts straight
//! into it), Remove after a confirmation, and a line to add one. The official server is put on top
//! once, as the game does (`server_list::add_official`).
use super::*;
use gpui::{Image, ImageFormat};
use horizon_launcher::server_list::{self, Server};
use servers_page::Field;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Multiplayer {
    /// The game folder the list is of, and the list as last read with each picture decoded.
    home: Option<PathBuf>,
    list: Vec<(Server, Option<Arc<Image>>)>,
    fresh: bool,
    adding: bool,
    confirm_remove: Option<usize>,
    error: Option<String>,
}

impl Multiplayer {
    /// Read again on the next frame: the game may have changed its list.
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

impl Launcher {
    fn load_servers(&mut self, home: PathBuf) {
        if self.multiplayer.home.as_ref() != Some(&home) {
            self.multiplayer = Multiplayer {
                home: Some(home.clone()),
                ..Multiplayer::default()
            };
        }
        if self.multiplayer.fresh {
            return;
        }
        // A list that can't be read is the game's to report: the page shows it empty.
        let _ = server_list::add_official(&home);
        // A few servers and their 64×64 pictures: small enough to read while drawing.
        self.multiplayer.list = server_list::List::load(&home)
            .servers()
            .into_iter()
            .map(|s| {
                let icon = s
                    .icon
                    .clone()
                    .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                (s, icon)
            })
            .collect();
        self.multiplayer.fresh = true;
    }

    fn edit_servers(&mut self, edit: impl FnOnce(&mut server_list::List) -> Result<(), String>) {
        let Some(home) = self.multiplayer.home.clone() else {
            return;
        };
        let mut list = server_list::List::load(&home);
        self.multiplayer.error = edit(&mut list).and_then(|()| list.save(&home)).err();
        self.multiplayer.confirm_remove = None;
        self.multiplayer.stale();
    }

    /// Enter in the address field, or Add.
    pub(super) fn add_server(&mut self) {
        let name = self.servers.value(Field::ServerName).to_string();
        let address = self.servers.value(Field::ServerAddress).to_string();
        self.edit_servers(|list| list.add(&name, &address));
        if self.multiplayer.error.is_none() {
            self.servers.text(Field::ServerName).set(String::new());
            self.servers.text(Field::ServerAddress).set(String::new());
            self.multiplayer.adding = false;
        }
    }

    /// Home's Multiplayer section, for the selected instance.
    pub(super) fn multiplayer_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_servers(home);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(4.))
            .child(section_title("Multiplayer", "server", &t))
            .child(
                button("server-add", "Add server", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.multiplayer.adding = !this.multiplayer.adding;
                        this.multiplayer.error = None;
                        if this.multiplayer.adding {
                            this.servers.focus_soon = Some(Field::ServerAddress);
                        }
                        cx.notify();
                    },
                )),
            );
        let mut section = div().flex().flex_col().gap(px(8.)).child(header);
        if let Some(e) = self.multiplayer.error.clone() {
            section = section.child(notice(e, &t));
        }
        if self.multiplayer.adding {
            section = section.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(10.))
                    .child(self.field(Field::ServerName, "Minecraft Server", 220., cx))
                    .child(self.field(Field::ServerAddress, "play.example.com", 300., cx))
                    .child(button("server-save", "Add", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.add_server();
                            cx.notify();
                        }),
                    )),
            );
        }
        let list = self.multiplayer.list.clone();
        if list.is_empty() && !self.multiplayer.adding {
            section = section.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child("No servers yet: add one here or in the game's Multiplayer screen."),
            );
        }
        for (i, (server, icon)) in list.into_iter().enumerate() {
            section = section.child(self.server_row(i, server, icon, cx));
        }
        Some(section)
    }

    fn server_row(
        &mut self,
        i: usize,
        server: Server,
        icon: Option<Arc<Image>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let picture = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(48.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well);
        let picture = match icon {
            Some(image) => picture.child(gpui::img(image).size(px(44.))),
            None => picture.child(blocks::icon("server", 22., t.muted)),
        };
        let mut actions = div().flex().items_center().gap(px(6.));
        if self.multiplayer.confirm_remove == Some(i) {
            actions = actions
                .child(
                    text(theme::CAPTION)
                        .text_color(t.danger)
                        .child("Remove it from the list?"),
                )
                .child(
                    button(
                        SharedString::from(format!("server-keep-{i}")),
                        "Keep",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.multiplayer.confirm_remove = None;
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        SharedString::from(format!("server-remove-{i}")),
                        "Remove",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.edit_servers(|list| list.remove(i).map(|_| ()));
                            cx.notify();
                        },
                    )),
                );
        } else {
            let ip = server.ip.clone();
            let busy = !self.can_join_selected();
            let join = button(
                SharedString::from(format!("server-join-{i}")),
                "Join",
                Kind::Secondary,
                &t,
            );
            let join = if busy { disabled(join) } else { join };
            actions = actions
                .child(
                    join.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.quick_play("--server", ip.clone());
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        SharedString::from(format!("server-delete-{i}")),
                        "Remove",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.multiplayer.confirm_remove = Some(i);
                            cx.notify();
                        },
                    )),
                );
        }
        plate(&t)
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(10.))
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(15., FontWeight::MEDIUM)
                            .text_color(t.ink)
                            .truncate()
                            .child(server.name.clone()),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .truncate()
                            .child(server.ip.clone()),
                    ),
            )
            .child(actions)
    }
}
