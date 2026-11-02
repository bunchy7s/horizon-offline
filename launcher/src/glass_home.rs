//! Home's head in the glass look (the mockup's `HomeView`, `launcher-views.tsx`): the instance's
//! edition, its icon and name, how long it was played, then Play with its menu, the version, the
//! instance's mods and its folders, and joining a friend on the right. Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use gpui::deferred;
use horizon_theme::glass::{fill, ink, radius};
use std::time::Instant;

/// Play, the version and the big icons are this tall (`h-14`).
const TALL: f32 = 56.;

/// Which of Home's menus is open.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum HomeMenu {
    #[default]
    None,
    Play,
    Folder,
}

impl Launcher {
    /// The head: what Home shows above its tabs. `tag`: the Horizon version it plays (empty for a
    /// Java instance); `prepared`: whether that version's files are ready.
    pub(super) fn glass_home_head(
        &mut self,
        tag: &str,
        prepared: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let java = self.selected_instance().and_then(|i| i.java.clone());
        // How long this game has run, for "Playing for 5 min now".
        match (&self.client, self.client_since) {
            (Some(_), None) => self.client_since = Some(Instant::now()),
            (None, Some(_)) => self.client_since = None,
            _ => {}
        }
        let left =
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(self.edition_tabs(java.is_some()))
                .child(self.home_title())
                .child(self.played_text(cx))
                .child(
                    div()
                        .mt(px(32.))
                        .flex()
                        // One line, unlike the mockup's `flex-wrap`: at 1280 px the folder's ⋯
                        // would wrap the row away from under the pointer, which hides it again.
                        .items_center()
                        .gap(px(10.))
                        .child(self.play_group(tag, prepared || java.is_some(), cx))
                        .children(java.is_none().then(|| self.glass_picker(cx)))
                        .child(ui::big_icon("home-mods", "puzzle", &self.theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.home_tab = home_tabs::HomeTab::Mods;
                                cx.notify();
                            }),
                        ))
                        .children(self.folder_group(java.is_none(), cx)),
                );
        div()
            .flex()
            .items_end()
            .justify_between()
            .gap(px(32.))
            .pt(px(64.))
            .child(left)
            .child(self.glass_join_friend(tag, cx))
    }

    /// The edition tabs (`Tabs`, size `sm`): the instance's own lit. An instance's edition is
    /// chosen when it is made, so the other one is shown but can't be picked.
    fn edition_tabs(&mut self, java: bool) -> Div {
        let grass = self.vanilla_icon().and_then(|path| {
            self.icon_image("mark:grass".into(), 18., false, move || {
                std::fs::read(&path).ok()
            })
        });
        let tab = |label: &'static str, icon: AnyElement, open: bool| {
            let d = div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h(px(32.))
                .px(px(12.))
                .rounded(px(radius::CONTROL))
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(13.))
                .line_height(px(18.))
                .child(icon)
                .child(tr(label));
            if open {
                d.bg(white(fill::ACTIVE))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white(ink::FULL))
            } else {
                d.text_color(white(ink::OFF))
            }
        };
        let java_icon = match grass {
            Some(image) => gpui::img(image)
                .flex_none()
                .size(px(18.))
                .rounded(px(5.))
                .into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(tab(
                "Rust Edition",
                ui::mark_tile(18., self.look.accent_soft).into_any_element(),
                !java,
            ))
            .child(tab("Java Edition", java_icon, java))
    }

    /// The instance's icon and its name in Bricolage, 64 px at this width
    /// (`clamp(2.75rem, 5vw, 4rem)`).
    fn home_title(&mut self) -> Div {
        let Some(instance) = self.selected_instance().cloned() else {
            return div();
        };
        div()
            .mt(px(20.))
            .flex()
            .items_center()
            .gap(px(16.))
            .min_w_0()
            .child(self.glass_instance_icon(&instance, 56.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(64.))
                    .line_height(px(64.))
                    .text_color(white(ink::FULL))
                    .child(instance.name),
            )
    }

    /// "You've played for 41 h 06 min! Last played today.", or how long it has run now; then
    /// the players counts the launcher knows (not in the mockup).
    fn played_text(&self, cx: &mut Context<Self>) -> Div {
        let line = match (self.client_since, self.selected_instance()) {
            (Some(since), _) => trf(
                "Playing for {time} now. Have fun!",
                &[("time", &instances::play_time(since.elapsed()))],
            ),
            (None, Some(i)) => match i.last_played {
                Some(when) => trf(
                    "You've played for {time}! Last played {when}.",
                    &[
                        ("time", &instances::play_time(i.played)),
                        ("when", &worlds_page::ago(when)),
                    ],
                ),
                None => tr("You haven't played this one yet. Press Play to start.").into(),
            },
            (None, None) => String::new(),
        };
        let link = |id: &'static str, label: String| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.text_color(white(ink::FULL)))
                .child(label)
        };
        let playing = self.playing.map(|count| {
            let label = format!(
                "· {}",
                trf("{count} playing now", &[("count", &thousands(count))])
            );
            let url = telemetry::stats_url(&self.links.website);
            link("playing-now", label).on_click(move |_: &ClickEvent, _, cx| {
                if let Some(url) = &url {
                    cx.open_url(url);
                }
            })
        });
        let official = self.official_players.map(|count| {
            link(
                "official-players",
                format!("· {} on the official SMP", thousands(count)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Servers);
                cx.notify();
            }))
        });
        ui::text(15., FontWeight::NORMAL, ink::STRONG)
            .mt(px(16.))
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .child(line)
            .children(playing)
            .children(official)
    }

    /// Play and its ⋯ menu (`PlayGroup`): Stop while the game runs, Update when the launcher's
    /// version has a newer one, the progress while preparing, updating or starting.
    fn play_group(&mut self, tag: &str, ready: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let accent = ui::color(self.look.accent);
        let running = self.client.is_some();
        let follows = self
            .selected_instance()
            .is_some_and(|i| i.version.is_none());
        let update = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| follows && !tag.is_empty())
            .map(|r| r.tag.clone());
        let installing = update
            .as_ref()
            .and_then(|u| self.installing.as_ref().filter(|(t, _)| t == u))
            .map(|(_, p)| *p);
        let busy = match (installing, self.preparing, &self.starting_client) {
            (Some(p), _, _) => Some((tr("Updating"), Some(p))),
            (_, true, _) => Some((
                tr("Preparing"),
                horizon_launcher::mojang::progress().map(|(d, n)| d as f32 / n.max(1) as f32),
            )),
            (_, _, Some(_)) => Some((tr("Starting"), None)),
            _ => None,
        };
        let open = self.home_menu == HomeMenu::Play;
        let more = div()
            .id("play-more")
            .absolute()
            .right(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.))
            .rounded(px(radius::CONTROL))
            .cursor_pointer()
            .when(open, |d| d.bg(gpui::black().opacity(0.2)))
            .hover(|s| s.bg(gpui::black().opacity(0.2)))
            .child(ui::icon("more-horizontal", 20., white(ink::FULL)))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if !this.closed_by_this_press(event) {
                    this.home_menu = match this.home_menu {
                        HomeMenu::Play => HomeMenu::None,
                        _ => HomeMenu::Play,
                    };
                }
                cx.notify();
            }));
        let base = div()
            .relative()
            .flex()
            .items_center()
            .w(px(200.))
            .h(px(TALL))
            .overflow_hidden()
            .rounded(px(radius::LARGE));
        let body = |label: SharedString, icon: &'static str| {
            div()
                .flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .pr(px(40.))
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(ui::icon(icon, 20., white(ink::FULL)))
                .child(label)
        };
        let group = match busy {
            Some((label, progress)) => base
                .bg(accent.opacity(0.35))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(gpui::relative(progress.unwrap_or(0.).clamp(0., 1.)))
                        .bg(accent),
                )
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_1()
                        .justify_center()
                        .pr(px(40.))
                        .font_family(horizon_theme::glass::FONT)
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(match progress {
                            Some(p) => format!("{label} {:.0}%", p * 100.),
                            None => format!("{label}…"),
                        }),
                ),
            None if running => base.bg(white(0.14)).child(
                body(tr("Stop").into(), "stop")
                    .id("play")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(client) = &this.client {
                            client.kill();
                        }
                        cx.notify();
                    })),
            ),
            None => {
                let (label, icon) = match &update {
                    Some(_) => (tr("Update"), "download-04"),
                    None => (tr("Play"), "play"),
                };
                let press = play_press(ready, self.owner, false);
                let main = body(label.into(), icon).id("play").cursor_pointer();
                let main = match (&update, press) {
                    (Some(_), _) => main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    })),
                    (None, PlayPress::Play) => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picker_open = false;
                            this.play();
                            cx.notify();
                        }))
                    }
                    (None, PlayPress::PrepareThenPlay) => {
                        main.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picker_open = false;
                            this.play_when_prepared = this.selected().cloned();
                            this.prepare(backend::MinecraftFrom::Install);
                            cx.notify();
                        }))
                    }
                    (None, PlayPress::Nothing) => main.opacity(0.5).cursor_default(),
                };
                base.bg(accent)
                    .text_color(ui::color(self.look.on_accent))
                    .hover(|s| s.opacity(0.92))
                    .child(main)
            }
        };
        let mut wrap = div()
            .relative()
            .child(keyboard(group.id("play-group"), &t).child(more));
        if open {
            let locked = running || busy.is_some();
            let mut items: Vec<(&'static str, SharedString, String, bool, PlayItem)> = Vec::new();
            if !tag.is_empty() {
                let version = backend::game_version(tag);
                items.push((
                    "play-prepare",
                    tr(if backend::is_prepared(tag) {
                        "Prepare again"
                    } else {
                        "Prepare"
                    })
                    .into(),
                    trf(
                        "Read {version}'s files from your own copy",
                        &[("version", &version)],
                    ),
                    locked,
                    PlayItem::Prepare,
                ));
            }
            if update.is_some() {
                items.push((
                    "play-skip",
                    tr("Play without updating").into(),
                    trf(
                        "Stay on {version}",
                        &[(
                            "version",
                            &horizon_launcher::nightly::label(tag).unwrap_or(tag.to_string()),
                        )],
                    ),
                    locked || !ready,
                    PlayItem::Play,
                ));
            }
            let id = self
                .selected_instance()
                .map(|i| (i.id.clone(), i.name.clone()));
            if let Some((id, _)) = &id {
                let has = self.has_shortcut(id);
                items.push((
                    "play-shortcut",
                    tr(if has {
                        "Remove the desktop shortcut"
                    } else {
                        "Create a desktop shortcut"
                    })
                    .into(),
                    tr("Starts this instance in one click").into(),
                    false,
                    PlayItem::Shortcut,
                ));
                items.push((
                    "play-command",
                    tr("Copy the launch command").into(),
                    launch_command(id),
                    false,
                    PlayItem::Command,
                ));
            }
            let mut menu = ui::popover(290.)
                .id("play-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.home_menu = HomeMenu::None;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for (key, label, note, locked, what) in items {
                let id = id.clone();
                let item = ui::menu_item(key, label, note, locked, &t);
                menu = menu.child(if locked {
                    item
                } else {
                    item.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_menu = HomeMenu::None;
                        match what {
                            PlayItem::Prepare => this.prepare(backend::MinecraftFrom::Install),
                            PlayItem::Play => this.play(),
                            PlayItem::Shortcut => {
                                if let Some((id, name)) = id.clone() {
                                    this.toggle_shortcut(id, name);
                                }
                            }
                            PlayItem::Command => {
                                if let Some((id, _)) = &id {
                                    copy_to_clipboard(
                                        cx,
                                        gpui::ClipboardItem::new_string(launch_command(id)),
                                    );
                                }
                            }
                        }
                        cx.notify();
                    }))
                });
            }
            wrap = wrap.child(
                deferred(div().absolute().top(px(TALL + 8.)).left_0().child(menu)).with_priority(1),
            );
        }
        wrap
    }

    /// The version (`VersionPicker`): "Version" over the one played, a chevron that turns while
    /// its list is open; the list ticks the one played and ends with Manage versions.
    fn glass_picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let soft = self.look.accent_soft;
        let label = self.selected().map_or_else(
            || tr("No version").into(),
            |tag| {
                horizon_launcher::nightly::label(tag)
                    .or_else(|| build_import::label(tag))
                    .unwrap_or_else(|| tag.clone())
            },
        );
        let open = self.picker_open;
        let chevron = ui::icon("arrow-down-01", 18., white(ink::NOTE)).when(open, |s| {
            s.with_transformation(gpui::Transformation::rotate(gpui::radians(
                std::f32::consts::PI,
            )))
        });
        let button = keyboard(
            div()
                .id("picker")
                .flex()
                .items_center()
                .justify_between()
                .gap(px(24.))
                .h(px(TALL))
                .min_w(px(220.))
                .px(px(16.))
                .rounded(px(radius::LARGE))
                .bg(white(fill::BUTTON))
                .hover(|s| s.bg(white(fill::ACTIVE)))
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            ui::text(12., FontWeight::NORMAL, ink::QUIET)
                                .line_height(px(18.))
                                .child(tr("Version")),
                        )
                        .child(
                            ui::text(15., FontWeight::MEDIUM, ink::FULL)
                                .line_height(px(22.))
                                .child(label),
                        ),
                )
                .child(chevron)
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if !this.closed_by_this_press(event) {
                        this.picker_open = !this.picker_open;
                    }
                    cx.notify();
                })),
            &t,
        );
        let mut wrap = div().relative().child(button);
        if open {
            let follows = self.launcher_version().cloned();
            let mut menu = ui::popover(300.)
                .id("picker-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.picker_open = false;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for tag in &self.installed {
                let chosen = self.selected() == Some(tag);
                let latest = follows.as_ref() == Some(tag);
                let name = horizon_launcher::nightly::label(tag)
                    .or_else(|| build_import::label(tag))
                    .unwrap_or_else(|| tag.clone());
                let note = if latest {
                    tr("Newest · follows updates").to_string()
                } else {
                    tag.clone()
                };
                let pick = tag.clone();
                menu = menu.child(
                    ui::menu_item(format!("pick-{tag}"), name.into(), note, false, &t)
                        .when(chosen, |d| {
                            d.child(ui::icon("tick-02", 17., ui::color(soft)).relative())
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pin_version((!latest).then(|| pick.clone()));
                            this.picker_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(
                ui::menu_item(
                    "manage",
                    tr("Manage versions").into(),
                    tr("Install, remove and pick versions").into(),
                    false,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.go(Page::Versions);
                    this.picker_open = false;
                    cx.notify();
                })),
            );
            wrap = wrap.child(
                deferred(div().absolute().top(px(TALL + 8.)).left_0().child(menu)).with_priority(1),
            );
        }
        wrap
    }

    /// The folder button and its ⋯ (`FolderGroup`), which shows under the pointer or while its
    /// menu is open: the instance's folder, its screenshots, and (Horizon) the Minecraft folder.
    fn folder_group(&self, horizon: bool, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?;
        let home = instance.home.clone();
        let open = self.home_menu == HomeMenu::Folder;
        let opener = home.clone();
        let more = div()
            .id("folder-more")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(40.))
            .w(px(36.))
            .mr(px(8.))
            .rounded(px(radius::CONTROL))
            .cursor_pointer()
            .when(open, |d| {
                d.w(px(36.))
                    .mr(px(8.))
                    .opacity(1.)
                    .bg(white(fill::SELECTED))
            })
            .group_hover("folder-group", |s| s.w(px(36.)).mr(px(8.)).opacity(1.))
            .hover(|s| s.bg(white(fill::SELECTED)))
            .child(ui::icon("more-horizontal", 20., white(ink::STRONG)))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if !this.closed_by_this_press(event) {
                    this.home_menu = match this.home_menu {
                        HomeMenu::Folder => HomeMenu::None,
                        _ => HomeMenu::Folder,
                    };
                }
                cx.notify();
            }));
        let group = div()
            .id("folder-group")
            .flex()
            .items_center()
            .h(px(TALL))
            .rounded(px(radius::LARGE))
            .bg(white(fill::BUTTON))
            .hover(|s| s.bg(white(fill::ACTIVE)))
            .child(
                div()
                    .id("home-folder")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(TALL))
                    .cursor_pointer()
                    .child(ui::icon("folder-01", 21., white(ink::STRONG)))
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&opener);
                        cx.open_with_system(&opener);
                    }),
            )
            .when(open || self.folder_hovered, |d| d.child(more))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.folder_hovered = *hovered;
                cx.notify();
            }));
        let mut wrap = div().relative().child(keyboard(group, &t));
        if open {
            let shots = home.join("screenshots");
            let count = std::fs::read_dir(&shots).map_or(0, |d| d.count());
            let mut items = vec![
                (
                    "folder-instance",
                    tr("Open the instance folder"),
                    short(&home),
                    Some(home.clone()),
                ),
                (
                    "folder-shots",
                    tr("Open the screenshots folder"),
                    if count == 1 {
                        tr("1 screenshot").into()
                    } else {
                        trf("{count} screenshots", &[("count", &count.to_string())])
                    },
                    Some(shots),
                ),
            ];
            if horizon {
                items.push((
                    "folder-minecraft",
                    tr("Choose the Minecraft folder"),
                    self.settings
                        .minecraft_dir
                        .as_deref()
                        .map_or_else(|| tr("Not found").into(), short),
                    None,
                ));
            }
            let mut menu = ui::popover(290.)
                .id("folder-menu")
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.home_menu = HomeMenu::None;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for (key, label, note, path) in items {
                menu = menu.child(ui::menu_item(key, label.into(), note, false, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_menu = HomeMenu::None;
                        match &path {
                            Some(p) => {
                                let _ = std::fs::create_dir_all(p);
                                cx.open_with_system(p);
                            }
                            None => this.choose_folder(cx),
                        }
                        cx.notify();
                    }),
                ));
            }
            wrap = wrap.child(
                deferred(div().absolute().top(px(TALL + 8.)).left_0().child(menu)).with_priority(1),
            );
        }
        Some(wrap)
    }

    /// The invite code field (`JoinFriend`): 340 wide, the icon inside on the left, Join inside
    /// on the right.
    fn glass_join_friend(&self, tag: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.friend_focused;
        let field = div()
            .id("friend-code")
            .track_focus(&self.friend_focus)
            .flex()
            .items_center()
            .size_full()
            .pl(px(48.))
            .pr(px(84.))
            .rounded(px(radius::LARGE))
            .bg(white(if focused { 0.09 } else { 0.06 }))
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(14.))
            .text_color(white(ink::FULL))
            .cursor_text()
            .overflow_hidden()
            .child(line_edit::content(
                &self.friend_code,
                &t,
                focused,
                false,
                false,
                "Paste a friend's invite code",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friend_code, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.join_friend(),
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.friend_code);
        let valid = horizon_friends::invite::parse(self.friend_code.text()).is_some();
        let can = valid && (tag.is_empty() || self.can_start(tag));
        let join = ui::quiet("join-friend", tr("Join"), 40., &t);
        let join = if can {
            join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_friend();
                cx.notify();
            }))
        } else {
            join.opacity(0.4).cursor_default()
        };
        div()
            .relative()
            .flex_none()
            .w(px(340.))
            .h(px(TALL))
            .child(field)
            .child(
                div()
                    .absolute()
                    .left(px(16.))
                    .top(px((TALL - 19.) / 2.))
                    .child(ui::icon("user-add-01", 19., white(ink::NOTE))),
            )
            .child(div().absolute().right(px(8.)).top(px(8.)).child(join))
    }
}

/// What a Play menu row does.
#[derive(Clone, Copy)]
enum PlayItem {
    Prepare,
    Play,
    Shortcut,
    Command,
}

/// The command that plays instance `id` without the window, as its desktop shortcut runs it.
fn launch_command(id: &str) -> String {
    format!("horizon-launcher play --instance {id}")
}
