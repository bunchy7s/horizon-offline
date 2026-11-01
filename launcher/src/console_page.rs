//! Home's Console: the game's output of this run as it comes, with a search, Follow (keep the
//! newest line in view), Copy, and Share log (`horizon_launcher::game_log`): an upload to mclo.gs
//! that happens only after the player confirms, without their home folder or names.
use super::servers_page::Field;
use super::*;
use gpui::{ClipboardItem, ScrollHandle};
use horizon_launcher::game_log::{self, Shared};

pub(super) struct Console {
    open: bool,
    follow: bool,
    scroll: ScrollHandle,
    /// Lines and search at the last draw: a change means scroll down when following.
    seen: (usize, String),
    confirm_share: bool,
    sharing: bool,
    shared: Option<Result<Shared, String>>,
}

impl Default for Console {
    fn default() -> Self {
        Console {
            open: std::env::var("HORIZON_LAUNCHER_CONSOLE").is_ok(),
            follow: true,
            scroll: ScrollHandle::new(),
            seen: (0, String::new()),
            confirm_share: false,
            sharing: false,
            shared: None,
        }
    }
}

impl Launcher {
    /// What Share log sends: this run's lines, else the selected instance's latest.log.
    fn log_to_share(&self) -> Result<(String, Vec<std::path::PathBuf>), String> {
        let home = self
            .client_home
            .clone()
            .or_else(|| self.selected_instance().map(|i| i.home.clone()))
            .unwrap_or_else(backend::game_home);
        let text = match self.prepare_log.is_empty() {
            false => {
                self.prepare_log
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            }
            true => {
                let path = home.join("logs/latest.log");
                std::fs::read_to_string(&path).map_err(|_| "There is no log to share yet.")?
            }
        };
        Ok((text, vec![home, backend::game_home()]))
    }

    fn share_log(&mut self, cx: &mut Context<Self>) {
        let shared = match self.log_to_share() {
            Ok(log) => log,
            Err(e) => {
                self.console.shared = Some(Err(e));
                return;
            }
        };
        self.console.confirm_share = false;
        self.console.sharing = true;
        let task = cx.background_executor().spawn(async move {
            let (text, homes) = shared;
            let homes: Vec<&std::path::Path> = homes.iter().map(|h| h.as_path()).collect();
            let names = game_log::names(&homes);
            game_log::share(&game_log::private(
                &text,
                dirs::home_dir().as_deref(),
                &names,
            ))
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.console.sharing = false;
                this.console.shared = Some(done);
                cx.notify();
            });
        })
        .detach();
    }

    fn unshare_log(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(shared)) = self.console.shared.clone() else {
            return;
        };
        self.console.sharing = true;
        let task = cx
            .background_executor()
            .spawn(async move { game_log::unshare(&shared) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                this.console.sharing = false;
                this.console.shared = match done {
                    Ok(()) => None,
                    Err(e) => Some(Err(e)),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Home's Console section, once the game has said something (or is running).
    pub(super) fn console_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        if self.prepare_log.is_empty() && self.client.is_none() {
            return None;
        }
        let open = self.console.open;
        let toggle = button(
            "console-toggle",
            if open { "Hide" } else { "Show" },
            Kind::Ghost,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.console.open = !this.console.open;
            cx.notify();
        }));
        let count = self.prepare_log.len();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_title("Console", "terminal", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(match self.client {
                                Some(_) => format!("{count} lines, the game is running"),
                                None => format!("{count} lines from the last run"),
                            }),
                    )
                    .child(toggle),
            );
        let mut section = div().flex().flex_col().gap(px(10.)).child(header);
        if !open {
            return Some(section);
        }
        let search = self.servers.value(Field::ConsoleSearch).trim().to_string();
        let lines: Vec<String> = self
            .prepare_log
            .iter()
            .filter(|l| game_log::matches(l, &search))
            .cloned()
            .collect();
        let now = (count, search);
        if self.console.follow && now != self.console.seen {
            self.console.scroll.scroll_to_bottom();
        }
        self.console.seen = now;
        let follow = div()
            .id("console-follow")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("console-follow-switch", self.console.follow, &t))
            .child(text(theme::LABEL).text_color(t.ink_soft).child("Follow"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.console.follow = !this.console.follow;
                this.console.seen.0 = usize::MAX;
                cx.notify();
            }));
        let copied = lines.join("\n");
        let tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(self.field(Field::ConsoleSearch, "Search the log", 260., cx))
            .child(follow)
            .child(div().flex_1())
            .child(
                button("console-copy", "Copy", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        copy_to_clipboard(cx, ClipboardItem::new_string(copied.clone()));
                        this.console.shared = None;
                        cx.notify();
                    },
                )),
            )
            .child({
                let b = button("console-share", "Share log…", Kind::Ghost, &t);
                match self.console.sharing {
                    true => disabled(b),
                    false => b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.console.confirm_share = true;
                        cx.notify();
                    })),
                }
            })
            .child({
                let logs = self
                    .client_home
                    .clone()
                    .or_else(|| self.selected_instance().map(|i| i.home.clone()))
                    .unwrap_or_else(backend::game_home)
                    .join("logs");
                button("console-folder", "Open logs folder", Kind::Ghost, &t).on_click(
                    move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&logs);
                        cx.open_with_system(&logs);
                    },
                )
            });
        section = section.child(tools);
        if self.console.confirm_share {
            section = section.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .child(div().flex_1().min_w_0().child(
                        text(theme::CAPTION).text_color(t.ink_soft).child(
                            "Upload this log to mclo.gs, a paste service for Minecraft logs? \
                             Anyone with the link can read it, until it is deleted or unread \
                             for 90 days. Your home folder, user name and account names are \
                             taken out first.",
                        ),
                    ))
                    .child(
                        button("console-share-no", "Cancel", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.console.confirm_share = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("console-share-yes", "Upload", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.share_log(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        match &self.console.shared {
            _ if self.console.sharing => {
                section = section.child(text(theme::CAPTION).text_color(t.muted).child("Sending…"))
            }
            Some(Ok(shared)) => {
                let (url, open_url) = (shared.url.clone(), shared.url.clone());
                section = section.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            text(theme::CAPTION)
                                .text_color(t.ink_soft)
                                .child(format!("Shared: {url}")),
                        )
                        .child(
                            button("console-link-copy", "Copy link", Kind::Ghost, &t).on_click(
                                move |_, _, cx| {
                                    copy_to_clipboard(cx, ClipboardItem::new_string(url.clone()))
                                },
                            ),
                        )
                        .child(
                            button("console-link-open", "Open", Kind::Ghost, &t)
                                .on_click(move |_, _, cx| cx.open_url(&open_url)),
                        )
                        .child(
                            button("console-unshare", "Delete it", Kind::Ghost, &t).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.unshare_log(cx);
                                    cx.notify();
                                }),
                            ),
                        ),
                );
            }
            Some(Err(e)) => section = section.child(notice(e.clone(), &t)),
            None => {}
        }
        let shown = lines.len().saturating_sub(LOG_LINES);
        let body = div()
            .id("console-lines")
            .track_scroll(&self.console.scroll)
            .h(px(320.))
            .overflow_y_scroll()
            .p(px(10.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .children(lines.into_iter().skip(shown).map(|line| {
                let color = if line.contains("/ERROR]") || line.contains("ERROR:") {
                    t.danger
                } else if line.contains("/WARN]") {
                    t.warn
                } else {
                    t.ink_soft
                };
                div().text_color(color).child(line)
            }));
        Some(section.child(body))
    }
}
