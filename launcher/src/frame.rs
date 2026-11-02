//! The glass look's frame (the mockup's `launcher-app.tsx`): the sidebar plate with the
//! instances, the launcher's update, Sculk's card, Discord and the links; the top bar with the
//! pages in a glass tab strip, the player, Microsoft sign-in and Settings; and the menus they open.
//! Dark appearance only for now (`docs/design-system.md`, "Glass").
use super::*;
use crate::glass_ui::{self as ui, white, Tone};
use crate::instances_page::java_line;
use gpui::{anchored, canvas, deferred, point};
use horizon_launcher::account::Player;
use horizon_launcher::instances::{Icon, Instance, Kind as InstanceKind};
use horizon_theme::glass::{fill, ink, radius};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The sidebar's width and its distance from the window's edges (`w-[248px]`, `m-3.5`).
pub const SIDEBAR: f32 = 248.;
pub const MARGIN: f32 = 14.;

/// The ⋮ menu of a sidebar row while open: whose, and whether Delete… was pressed once.
#[derive(Clone, Default)]
pub struct RowMenu {
    pub open: Option<(String, bool)>,
    /// Where each row was drawn last, by instance id: the menu opens beside its row.
    pub rows: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
}

/// Records where the element it sits in is drawn, under `id`.
fn measure(rows: &Rc<RefCell<HashMap<String, Bounds<Pixels>>>>, id: String) -> impl IntoElement {
    let rows = rows.clone();
    canvas(
        move |bounds, _, _| {
            rows.borrow_mut().insert(id, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Launcher {
    /// An instance's icon (`InstanceIcon`), `size` square with no frame: Horizon's mark tile for
    /// Horizon's own instance; else its picture or the block chosen for it, round at 26 %; else
    /// its loader's or Minecraft's mark; else its initial on its colour (the accent when none was
    /// chosen), as `ColorTile` draws it.
    pub(super) fn glass_instance_icon(&mut self, i: &Instance, size: f32) -> AnyElement {
        if i.kind == InstanceKind::Default && i.icon.is_none() {
            return ui::mark_tile(size, self.look.accent_soft).into_any_element();
        }
        if let Some(picture) = self.instance_picture(i, size) {
            return instance_icons::centred(picture, size, self.scale).into_any_element();
        }
        let r = px((size * 0.26).round());
        let mut colour = self.look.accent;
        match &i.icon {
            Some(Icon::Colour(hex)) => {
                colour = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(colour);
            }
            Some(Icon::Texture(name)) => {
                if let Some(image) = self.texture_image(name, size) {
                    return gpui::img(image)
                        .flex_none()
                        .size(px(size))
                        .rounded(r)
                        .into_any_element();
                }
            }
            None => {
                if let Some(mark) = self.instance_mark(i, size) {
                    return instance_icons::centred(mark, size, self.scale).into_any_element();
                }
            }
        }
        // Bricolage at the optical size a browser would pick for the letter.
        let letter = (size * 0.46).round();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded(r)
            .bg(ui::color(colour))
            .font_family(if letter >= 40. {
                horizon_theme::glass::DISPLAY_FONT
            } else {
                horizon_theme::glass::HEADING_FONT
            })
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(letter))
            .line_height(px(letter))
            .text_color(gpui::white())
            .child(instances_page::initial(&i.name))
            .into_any_element()
    }

    /// A sidebar row's state line and its dot (the mockup's `statusOf`), from what the launcher
    /// knows of the instance.
    fn row_state(&self, i: &Instance) -> (Tone, String) {
        let running =
            self.client.is_some() && self.selected_instance().is_some_and(|s| s.id == i.id);
        if running {
            return (Tone::Live, tr("Running").into());
        }
        let starting = self.starting_client.is_some()
            && self.selected_instance().is_some_and(|s| s.id == i.id);
        if starting {
            return (Tone::Busy, tr("Starting").into());
        }
        match i.kind {
            InstanceKind::Default | InstanceKind::Own => {
                let tag = self.version_of(i).cloned();
                let prepared = tag.as_deref().is_some_and(backend::is_prepared);
                match (&i.java, &tag, &i.version) {
                    (Some(java), _, _) => (Tone::Live, java_line(java)),
                    (None, None, Some(own)) => (
                        Tone::Warn,
                        trf("{version} is not installed", &[("version", own)]),
                    ),
                    (None, None, None) => (Tone::Off, tr("No version installed").into()),
                    // Following the launcher's version, which has a newer one waiting.
                    (None, Some(_), None) if self.updates.horizon.is_some() => (
                        Tone::Accent(self.look.accent_soft),
                        tr("Update available").into(),
                    ),
                    (None, Some(_), _) if !prepared => (Tone::Warn, tr("Needs preparing").into()),
                    (None, Some(tag), _) if self.native_build_of(tag).is_some() => {
                        let build = self.native_build_of(tag).unwrap_or_default();
                        (
                            Tone::Live,
                            trf(
                                "Native build · {version}",
                                &[("version", &build.horizon_short())],
                            ),
                        )
                    }
                    (None, Some(tag), Some(_)) => (Tone::Live, format!("{} · {tag}", tr("Ready"))),
                    (None, Some(_), None) => (Tone::Live, tr("Ready").into()),
                }
            }
            InstanceKind::Port => {
                let slug = &i.id["port:".len()..];
                let summary = self.packs.iter().find(|p| p.slug == slug);
                (
                    Tone::Live,
                    summary.map_or_else(|| tr("Modpack").into(), |p| p.summary()),
                )
            }
            InstanceKind::Pack => (Tone::Off, tr("From Horizon's registry").into()),
        }
    }

    /// The instances by group (`sections` in the mockup): those without a group first.
    fn glass_instances(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = self.theme;
        let selected = self.selected_instance().map(|i| i.id.clone());
        let on_packs = self.page == Page::Mods && self.mods.on_modpacks();
        let mut out = Vec::new();
        let mut group = None;
        let mut list = div().flex().flex_col().gap(px(4.));
        let flush = |list: Div, out: &mut Vec<AnyElement>| {
            out.push(list.into_any_element());
            div().flex().flex_col().gap(px(4.))
        };
        for i in self.instances.list.clone() {
            if i.group != group {
                list = flush(list, &mut out);
                group = i.group.clone();
                if let Some(g) = &group {
                    out.push(
                        ui::text(12., FontWeight::MEDIUM, ink::DIM)
                            .px(px(8.))
                            .pt(px(16.))
                            .pb(px(6.))
                            .child(g.clone())
                            .into_any_element(),
                    );
                }
            }
            // As in the mockup, the chosen instance stays marked on every page, unless a ported
            // pack's page is open: that row is the one shown then.
            let current = match i.kind {
                InstanceKind::Default | InstanceKind::Own => {
                    !(on_packs && self.pack_open.is_some()) && selected.as_ref() == Some(&i.id)
                }
                InstanceKind::Port => {
                    on_packs && self.pack_open.as_deref() == Some(&i.id["port:".len()..])
                }
                InstanceKind::Pack => false,
            };
            let state = self.row_state(&i);
            let icon = self.glass_instance_icon(&i, 38.);
            let clicked = i.clone();
            let open = self
                .row_menu
                .open
                .as_ref()
                .is_some_and(|(id, _)| *id == i.id);
            let menu_id = i.id.clone();
            let more = div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(6.))
                .flex()
                .items_center()
                .child(
                    ui::icon_button(
                        format!("side-more-{}", i.id),
                        "more-vertical",
                        32.,
                        18.,
                        radius::SMALL,
                        open,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, event: &ClickEvent, _, cx| {
                            if this.closed_by_this_press(event) {
                                return;
                            }
                            let was = this.row_menu.open.take();
                            if was.is_none_or(|(id, _)| id != menu_id) {
                                this.row_menu.open = Some((menu_id.clone(), false));
                            }
                            cx.notify();
                        },
                    )),
                );
            list = list.child(
                div()
                    .relative()
                    .child(measure(&self.row_menu.rows, i.id.clone()))
                    .child(
                        ui::row(format!("side-instance-{}", i.id), current, 40., &t)
                            .child(div().relative().flex_none().child(icon))
                            .child(ui::row_text(i.name.clone(), state))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                match clicked.kind {
                                    InstanceKind::Default | InstanceKind::Own => {
                                        this.select_instance(&clicked);
                                        this.go(Page::Play);
                                    }
                                    InstanceKind::Port => {
                                        this.show_modpack(Some(clicked.id["port:".len()..].into()))
                                    }
                                    InstanceKind::Pack => this.show_modpack(None),
                                }
                                cx.notify();
                            })),
                    )
                    .child(more),
            );
        }
        out.push(list.into_any_element());
        out
    }

    /// The glass sidebar: a plate 248 wide, 14 from the window's edges.
    pub(super) fn glass_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let soft = self.look.accent_soft;
        self.load_packs();
        let add = ui::icon_button(
            "add-instance",
            "plus-sign",
            36.,
            16.,
            radius::CONTROL,
            false,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.open_instance_dialog(None);
            cx.notify();
        }));
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(8.))
            .child(ui::text(13., FontWeight::MEDIUM, ink::NOTE).child(tr("Instances")))
            .child(add);
        let mut list = div()
            .id("side-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(10.))
            .children(self.glass_instances(cx));

        // The player's hosted servers: not in the mockup (its Servers page lists them), kept as
        // one more section in its pattern.
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        if !servers.is_empty() {
            list = list.child(
                ui::text(12., FontWeight::MEDIUM, ink::DIM)
                    .px(px(8.))
                    .pt(px(16.))
                    .pb(px(6.))
                    .child(tr("Servers")),
            );
        }
        let mut server_rows = div().flex().flex_col().gap(px(4.));
        for e in &servers {
            let up = self.servers.is_up(&e.dir);
            let open = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            server_rows = server_rows.child(
                ui::row(format!("side-server-{}", e.dir.display()), open, 8., &t)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(38.))
                            .rounded(px(radius::CONTROL))
                            .bg(white(fill::BUTTON))
                            .child(ui::icon("computer-terminal-01", 18., white(ink::STRONG))),
                    )
                    .child(ui::row_text(
                        e.name.clone(),
                        if up {
                            (Tone::Live, tr("Running").into())
                        } else {
                            (Tone::Off, tr("Stopped").into())
                        },
                    ))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let list = list.child(server_rows);

        let mut foot = div()
            .flex()
            .flex_col()
            .flex_none()
            .px(px(10.))
            .pt(px(16.))
            .pb(px(12.));
        // The launcher's own update (the mockup's card), while one downloads or waits.
        let card = |title: String, right: Option<String>| {
            div()
                .flex()
                .flex_col()
                .rounded(px(radius::ROW))
                .bg(white(fill::FIELD))
                .p(px(14.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .child(ui::text(14., FontWeight::SEMIBOLD, ink::FULL).child(title))
                        .children(
                            right.map(|r| ui::text(13., FontWeight::NORMAL, ink::NOTE).child(r)),
                        ),
                )
        };
        match &self.self_update {
            SelfUpdate::Downloading(version, f) => {
                foot = foot.child(
                    card(
                        trf("Launcher {version}", &[("version", version)]),
                        Some(format!("{:.0}%", f * 100.)),
                    )
                    .child(div().mt(px(10.)).child(ui::bar(*f, soft))),
                );
            }
            SelfUpdate::Ready(version) => {
                foot = foot.child(
                    card(trf("Launcher {version}", &[("version", version)]), None).child(
                        ui::solid("restart", tr("Restart to update"), 32., 13., &t)
                            .mt(px(12.))
                            .w_full()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.self_update = SelfUpdate::Failed(backend::restart());
                                cx.notify();
                            })),
                    ),
                );
            }
            _ => {}
        }

        // Sculk's card (not in the mockup, kept: a sponsor), where the old sidebar had it: above
        // the links on every page but Servers, which shows the wide banner. Tall where the
        // sidebar has the height for it besides its rows, compact otherwise.
        if self.page != Page::Servers && !self.settings.partner_hidden {
            let rows = self.instances.list.len() + servers.len();
            let free = f32::from(window.viewport_size().height)
                - 2. * MARGIN
                - 52.
                - 56. * rows as f32
                - 190.;
            let mut side = div().relative().mt(px(12.)).child(partner::card(
                "partner-side",
                SIDEBAR - 20.,
                free < partner::TALL_HEIGHT,
                &t,
            ));
            if self.settings.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                side = side.child(
                    partner::hide_button("partner-hide", &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.settings.partner_hidden = true;
                            this.save_settings();
                            cx.notify();
                        },
                    )),
                );
            }
            foot = foot.child(side);
        }

        if !self.links.discord.is_empty() {
            let url = self.links.discord.clone();
            foot = foot.child(keyboard(
                div()
                    .id("discord")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .mt(px(12.))
                    .h(px(40.))
                    .rounded(px(radius::ROW))
                    .bg(ui::color(horizon_theme::glass::DISCORD))
                    .hover(|s| s.bg(ui::color(horizon_theme::glass::DISCORD_HOVER)))
                    .cursor_pointer()
                    .child(ui::icon("discord", 18., gpui::white()))
                    .child(
                        ui::text(14., FontWeight::SEMIBOLD, ink::FULL)
                            .child(tr("Join the Discord")),
                    )
                    .on_click(move |_, _, cx| cx.open_url(&url)),
                &t,
            ));
        }
        let link = |id: &'static str, label: &'static str, icon: &'static str, url: String| {
            let id = SharedString::from(id);
            keyboard(
                track_hover(
                    div()
                        .id(id.clone())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(36.))
                        .px(px(8.))
                        .rounded(px(radius::ROW))
                        .cursor_pointer()
                        .text_color(white(ink::SOFT))
                        .hover(|s| s.text_color(white(ink::FULL)))
                        .child(hover_layer(&id, white(fill::HOVER), radius::ROW, 0.))
                        .child(ui::icon(icon, 17., white(ink::SOFT)).relative())
                        .child(
                            div()
                                .relative()
                                .font_family(horizon_theme::glass::FONT)
                                .text_size(px(14.))
                                .child(label),
                        )
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    id,
                ),
                &t,
            )
        };
        let mut links = div().flex().flex_col().mt(px(8.));
        if !self.links.sponsor.is_empty() {
            links = links.child(link(
                "sponsor",
                tr("Sponsor"),
                "favourite",
                self.links.sponsor.clone(),
            ));
        }
        if !self.links.website.is_empty() {
            links = links.child(link(
                "website",
                tr("Website"),
                "globe-02",
                self.links.website.clone(),
            ));
        }
        foot = foot.child(links).child(
            ui::text(12., FontWeight::NORMAL, ink::DIM)
                .id("launcher-version")
                .px(px(8.))
                .pt(px(8.))
                .child(trf("Launcher {version}", &[("version", backend::VERSION)]))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.version_clicked();
                    cx.notify();
                })),
        );

        crate::glass::plate(radius::PANEL)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR))
            .my(px(MARGIN))
            .ml(px(MARGIN))
            .overflow_hidden()
            .child(header.relative())
            .child(list.relative())
            .child(foot.relative())
    }

    /// The glass top bar: the pages in a glass strip, then the player, Microsoft sign-in and
    /// Settings (`px-8 pt-3.5`).
    pub(super) fn glass_topbar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut tabs = div().flex().items_center().gap(px(4.)).min_w_0();
        for (page, label, _) in Page::ALL {
            if page == Page::Settings {
                continue;
            }
            tabs = tabs.child(
                ui::tab(format!("nav-{label}"), tr(label), self.page == page, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.go(page);
                        cx.notify();
                    }),
                ),
            );
        }
        let strip = crate::glass::plate(radius::LARGE)
            .p(px(4.))
            .min_w_0()
            .flex_none()
            .child(tabs.relative());

        let (name, how) = self.who_plays();
        let face = match self.accounts.player() {
            Player::Microsoft(_) => self.accounts.selected().map(|a| a.uuid.clone()),
            _ => None,
        };
        let open = self.account_menu;
        let chip =
            div()
                .id("account")
                .relative()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(44.))
                .pl(px(8.))
                .pr(px(12.))
                .rounded(px(radius::ROW))
                .bg(white(if open { fill::ACTIVE } else { fill::ROW_HOVER }))
                .when(!open, |d| d.hover(|s| s.bg(white(0.09))))
                .cursor_pointer()
                .child(
                    div().size(px(26.)).rounded(px(7.)).overflow_hidden().child(
                        self.profile_heads
                            .draw(face.as_deref(), 26., self.scale, &t),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .max_w(px(150.))
                        .child(
                            ui::text(14., FontWeight::SEMIBOLD, ink::FULL)
                                .line_height(px(17.))
                                .truncate()
                                .child(name),
                        )
                        .child(
                            ui::text(12., FontWeight::NORMAL, ink::QUIET)
                                .line_height(px(15.))
                                .child(how),
                        ),
                )
                .child(
                    ui::icon("arrow-down-01", 16., white(ink::FAINT))
                        .ml(px(4.))
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            if open { std::f32::consts::PI } else { 0. },
                        ))),
                )
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if this.closed_by_this_press(event) {
                        return;
                    }
                    this.account_menu = !this.account_menu;
                    if this.account_menu {
                        this.accounts = Accounts::load();
                        this.load_profile_heads();
                    }
                    cx.notify();
                }));
        let account = div()
            .relative()
            .child(keyboard(chip, &t))
            .children(self.glass_account_menu(cx));

        // Signing in with Microsoft, while nobody is: the mockup's button.
        let sign_in = (!matches!(self.accounts.player(), Player::Microsoft(_))).then(|| {
            keyboard(
                div()
                    .id("topbar-sign-in")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(44.))
                    .px(px(16.))
                    .rounded(px(radius::ROW))
                    .bg(white(fill::BUTTON))
                    .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                    .cursor_pointer()
                    .child(microsoft::logo(16.))
                    .child(
                        ui::text(14., FontWeight::MEDIUM, ink::FULL)
                            .child(tr("Sign in with Microsoft")),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_sign_in();
                        cx.notify();
                    })),
                &t,
            )
        });
        let settings = ui::icon_button(
            "nav-Settings",
            "sliders-horizontal",
            40.,
            19.,
            radius::CONTROL,
            self.page == Page::Settings,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.go(if this.page == Page::Settings {
                Page::Play
            } else {
                Page::Settings
            });
            cx.notify();
        }));
        // The last native build's log (not in the mockup, kept), as a quiet button.
        let build_log = self.native_log.available.then(|| {
            keyboard(
                div()
                    .id("topbar-native-log")
                    .flex()
                    .items_center()
                    .h(px(36.))
                    .px(px(14.))
                    .rounded(px(radius::CONTROL))
                    .bg(white(fill::BUTTON))
                    .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                    .cursor_pointer()
                    .child(ui::text(14., FontWeight::MEDIUM, ink::FULL).child(tr("Build log")))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.native_log_open();
                        cx.notify();
                    })),
                &t,
            )
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .px(px(32.))
            .pt(px(14.))
            .child(strip)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .children(build_log)
                    .child(account)
                    .children(sign_in)
                    .child(settings),
            )
    }

    /// The menu under the player (`Switch player`): everyone who can play, then Manage accounts.
    fn glass_account_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.account_menu {
            return None;
        }
        let t = self.theme;
        let soft = self.look.accent_soft;
        let now = self.accounts.player();
        let mut menu = ui::popover(250.)
            .id("account-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.account_menu = false;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }));
        for (i, (who, name, how)) in self.players().into_iter().enumerate() {
            let chosen = who == now;
            // The mockup's rows say only "Offline"; the old menu's longer line stays there.
            let how = if who == Player::GameName {
                tr("Offline")
            } else {
                how
            };
            let id = SharedString::from(format!("account-pick-{i}"));
            menu = menu.child(keyboard(
                track_hover(
                    div()
                        .id(id.clone())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .h(px(48.))
                        .px(px(10.))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .child(hover_layer(&id, white(0.07), radius::CONTROL, 0.))
                        .child(
                            div()
                                .relative()
                                .size(px(30.))
                                .rounded(px(8.))
                                .overflow_hidden()
                                .child(self.profile_heads.draw(
                                    match &who {
                                        Player::Microsoft(uuid) => Some(uuid),
                                        _ => None,
                                    },
                                    30.,
                                    self.scale,
                                    &t,
                                )),
                        )
                        .child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    ui::text(14., FontWeight::MEDIUM, ink::FULL)
                                        .truncate()
                                        .child(name),
                                )
                                .child(ui::text(12., FontWeight::NORMAL, ink::FAINT).child(how)),
                        )
                        .when(chosen, |d| {
                            d.child(ui::icon("tick-02", 16., ui::color(soft)).relative())
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(who.clone());
                            cx.notify();
                        })),
                    id,
                ),
                &t,
            ));
        }
        menu = menu.child(div().mt(px(4.)).child(
            ui::menu_row("account-manage", "user", tr("Manage accounts"), false, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.account_menu = false;
                    this.go(Page::Settings);
                    cx.notify();
                }),
            ),
        ));
        Some(deferred(
            div().absolute().top(px(52.)).right_0().child(menu),
        ))
    }

    /// The ⋮ menu of a sidebar row, beside it: Edit…, Duplicate…, Open folder, Export…, the
    /// desktop shortcut, then Delete… (asked twice). Each acts on that instance.
    pub(super) fn row_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (id, confirm) = self.row_menu.open.clone()?;
        let Some(instance) = self.instances.list.iter().find(|i| i.id == id).cloned() else {
            self.row_menu.open = None;
            return None;
        };
        let row = self.row_menu.rows.borrow().get(&id).copied()?;
        let t = self.theme;
        let height = f32::from(window.viewport_size().height);
        // The mockup's `rect.right + 12`, `min(rect.top - 8, innerHeight - 330)`: its ⋮ button
        // ends 6 px inside the row and sits in the row's middle.
        let x = f32::from(row.origin.x + row.size.width) - 6. + 12.;
        let button_top = f32::from(row.origin.y) + (f32::from(row.size.height) - 32.) / 2.;
        let y = (button_top - 8.).min(height - 330.).max(8.);
        let item = |key: &'static str, icon: &'static str, label: String| {
            ui::menu_row(format!("row-menu-{key}"), icon, label, false, &t)
        };
        let pick = |this: &mut Launcher, i: &Instance| {
            this.row_menu.open = None;
            this.select_instance(i);
            this.go(Page::Play);
        };
        let (edit, copy, export, shortcut) = (
            instance.clone(),
            instance.clone(),
            instance.clone(),
            instance.clone(),
        );
        let home = instance.home.clone();
        let shortcut_label = if self.has_shortcut(&instance.id) {
            tr("Remove the desktop shortcut")
        } else {
            tr("Create a desktop shortcut")
        };
        let mut menu = ui::popover(250.)
            .id("row-menu")
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.row_menu.open = None;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }))
            .child(
                item("edit", "pencil-edit-02", tr("Edit…").into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        this.open_instance_dialog(Some(edit.clone()));
                        cx.notify();
                    },
                )),
            )
            .child(
                item("duplicate", "copy-01", tr("Duplicate…").into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        pick(this, &copy);
                        this.start_copy();
                        cx.notify();
                    },
                )),
            )
            .child(
                item("folder", "folder-01", tr("Open folder").into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        let _ = std::fs::create_dir_all(&home);
                        cx.open_with_system(&home);
                        cx.notify();
                    },
                )),
            )
            .child(
                item("export", "upload-04", tr("Export…").into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        pick(this, &export);
                        if this.export.is_none() {
                            this.toggle_export();
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                item("shortcut", "link-01", shortcut_label.into()).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.row_menu.open = None;
                        this.toggle_shortcut(shortcut.id.clone(), shortcut.name.clone());
                        cx.notify();
                    },
                )),
            );
        // Horizon's own instance can't be deleted, nor one while a game runs.
        let can_delete = instance.kind != InstanceKind::Default
            && self.client.is_none()
            && self.starting_client.is_none();
        let gone = instance.id.clone();
        let delete = ui::menu_row(
            "row-menu-delete",
            "delete-02",
            if confirm {
                tr("Click again to delete")
            } else {
                tr("Delete…")
            },
            true,
            &t,
        );
        let delete = if can_delete {
            delete.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some((_, confirm)) = this.row_menu.open.as_mut().filter(|(_, c)| !*c) {
                    *confirm = true;
                } else {
                    this.row_menu.open = None;
                    this.delete_instance(&gone);
                }
                cx.notify();
            }))
        } else {
            delete.opacity(0.4).cursor_default()
        };
        menu = menu.child(div().mt(px(4.)).child(delete));
        Some(
            deferred(anchored().position(point(px(x), px(y))).child(menu))
                .with_priority(1)
                .into_any_element(),
        )
    }
}
