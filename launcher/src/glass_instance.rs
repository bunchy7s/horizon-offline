//! The instance dialog in the glass look (the mockup's `InstanceModal`, horizon-site
//! `components/launcher/instance-modal.tsx`): one plate round at 28 over the dimmed page. On the
//! left, what the instance will be (its icon, name and what it plays) and its kind; on the right,
//! what it plays, its name and group, and its icon; Cancel and Create at the foot. Dark
//! appearance only: the light look keeps `instance_dialog`.
use super::*;
use crate::glass_ui::{self as ui, white};
use horizon_launcher::instances::{self, Icon, Instance};
use horizon_launcher::loaders::Kind as Loader;
use horizon_theme::glass::{fill, ink, radius};
use new_instance::{NewKind, VersionList};
use servers_page::Field;
use std::sync::Arc;

/// The loaders Prism Launcher offers, in the mockup's order. Those `loaders::Kind` can install
/// are shown; the others join when the launcher learns them.
const LOADERS: [&str; 5] = ["fabric", "quilt", "neoforge", "forge", "liteloader"];
/// Logos offered as icons (the mockup's `LOGO_ICONS`, but Minecraft's, which is Mojang's).
const LOGO_ICONS: [&str; 4] = ["neoforge", "fabric", "quilt", "forge"];
/// The colours offered (`ICON_COLORS`).
const COLOURS: [u32; 8] = [
    0x6b4bff, 0x3b82f6, 0x14b8a6, 0x22c55e, 0xeab308, 0xf97316, 0xef4444, 0x8b8a99,
];

/// The loaders a new modded instance can use now.
pub(super) fn loaders() -> Vec<Loader> {
    LOADERS
        .iter()
        .filter_map(|id| Loader::parse(id).ok())
        .collect()
}

/// What an existing instance plays.
fn kind_of(i: &Instance) -> NewKind {
    match &i.java {
        None => NewKind::Horizon,
        Some(j) if j.loader.is_some() => NewKind::Modded,
        Some(_) => NewKind::Vanilla,
    }
}

/// A choice card (`CHOICE`): round at 16, white at 12 % when chosen, else 4 %, 7 % under the
/// pointer.
pub(super) fn choice(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    keyboard(
        div()
            .id(id.into())
            .flex()
            .rounded(px(radius::POPOVER))
            .cursor_pointer()
            .bg(white(if on { fill::ACTIVE } else { 0.04 }))
            .when(!on, |d| d.hover(|s| s.bg(white(0.07)))),
        t,
    )
}

/// A choice's two lines: its title at 15 px, semibold, its note at 13 px.
pub(super) fn choice_text(title: impl Into<SharedString>, note: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            ui::text(15., FontWeight::SEMIBOLD, ink::FULL)
                .truncate()
                .child(title.into()),
        )
        .child(
            ui::text(13., FontWeight::NORMAL, ink::QUIET)
                .line_height(px(18.))
                .child(note.into()),
        )
}

/// A `Section`: its title at 13 px, white at 60 %, anything `aside` at its right, then `body`.
pub(super) fn section(title: SharedString, aside: Option<Div>, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap(px(16.))
                .mb(px(12.))
                .child(ui::text(13., FontWeight::MEDIUM, ink::NOTE).child(title))
                .children(aside),
        )
        .child(body)
}

/// A list's row of choices: round at 12, white at 11 % when chosen, 5 % under the pointer.
pub(super) fn list_choice(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    keyboard(
        div()
            .id(id.into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .rounded(px(radius::ROW))
            .cursor_pointer()
            .when(on, |d| d.bg(white(0.11)))
            .when(!on, |d| d.hover(|s| s.bg(white(fill::HOVER)))),
        t,
    )
}

/// The tick at a list row's end, the accent's soft shade, in a 20 px column.
pub(super) fn tick(on: bool, soft: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .w(px(20.))
        .children(on.then(|| ui::icon("tick-02", 18., ui::color(soft))))
}

/// A list of rows on white at 4 %, round at 16, scrolling past `max` px.
pub(super) fn list_box(id: &'static str, max: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(px(max))
        .overflow_y_scroll()
        .p(px(6.))
        .rounded(px(radius::POPOVER))
        .bg(white(0.04))
}

/// A tile with a name's initial on `colour` (`ColorTile`), round at 26 %.
pub(super) fn letter_tile(colour: Hsla, name: &str, size: f32) -> Div {
    let letter = (size * 0.46).round();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px((size * 0.26).round()))
        .bg(colour)
        .font_family(if letter >= 40. {
            horizon_theme::glass::DISPLAY_FONT
        } else {
            horizon_theme::glass::HEADING_FONT
        })
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(letter))
        .line_height(px(letter))
        .text_color(gpui::white())
        .child(instances_page::initial(name))
}

/// An image `size` px square, round at 26 % (`LogoIcon`).
pub(super) fn tile_image(image: Arc<gpui::RenderImage>, size: f32) -> AnyElement {
    gpui::img(image)
        .flex_none()
        .size(px(size))
        .rounded(px((size * 0.26).round()))
        .into_any_element()
}

/// An icon choice (52 px, round at 15): white at 16 % when chosen, 7 % under the pointer.
fn icon_choice(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    keyboard(
        div()
            .id(id.into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(52.))
            .rounded(px(15.))
            .cursor_pointer()
            .when(on, |d| d.bg(white(0.16)))
            .when(!on, |d| d.hover(|s| s.bg(white(0.07)))),
        t,
    )
}

impl Launcher {
    pub(super) fn glass_instance_dialog(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let d = self.instances.dialog.as_ref()?;
        let busy = d.busy;
        let editing = d
            .editing
            .as_ref()
            .and_then(|id| self.instances.list.iter().find(|i| &i.id == id))
            .cloned();
        let kind = editing.as_ref().map_or(d.java.kind, kind_of);
        let aside = self.instance_aside(editing.as_ref(), kind, cx);
        let main = self.instance_main(editing.as_ref(), kind, cx);
        let close = cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            if !busy {
                this.instances.dialog = None;
            }
            cx.notify();
        });
        let card = crate::glass::plate(radius::MODAL)
            .id("instance-card")
            .occlude()
            .flex()
            .w_full()
            .max_w(px(1180.))
            .h_full()
            .max_h(px(820.))
            .child(aside)
            .child(main);
        Some(
            div()
                .id("instance-dialog")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .occlude()
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(ui::color(horizon_theme::glass::SCRIM).opacity(0.5))
                        .on_mouse_down(gpui::MouseButton::Left, close),
                )
                .child(card),
        )
    }

    /// The loader a modded instance plays (or will).
    pub(super) fn draft_loader(&self, editing: Option<&Instance>) -> Loader {
        match editing {
            Some(i) => i
                .java
                .as_ref()
                .and_then(|j| j.loader.as_ref())
                .map_or(Loader::Fabric, |l| l.kind),
            None => self
                .instances
                .dialog
                .as_ref()
                .map_or(Loader::Fabric, |d| d.java.loader),
        }
    }

    /// The Minecraft version a Java instance plays (or will), once Mojang's list says.
    pub(super) fn draft_minecraft(&self, editing: Option<&Instance>) -> Option<String> {
        if let Some(j) = editing.and_then(|i| i.java.as_ref()) {
            return Some(j.minecraft.clone());
        }
        let versions = match &self.instances.versions {
            VersionList::Ready(v) => Some(v),
            _ => None,
        };
        self.instances.dialog.as_ref()?.java.minecraft(versions)
    }

    /// The Horizon version an instance plays: the one it keeps, else the launcher's.
    fn draft_horizon(&self) -> Option<String> {
        let pin = self.instances.dialog.as_ref()?.pin.clone();
        pin.or_else(|| self.launcher_version().cloned())
    }

    /// The mark an instance shows when nothing is chosen: Horizon's, the grass block, its
    /// loader's logo.
    fn auto_mark(&mut self, kind: NewKind, loader: Loader, size: f32) -> AnyElement {
        let mark = match kind {
            NewKind::Horizon => {
                return ui::mark_tile(size, self.look.accent_soft).into_any_element()
            }
            NewKind::Vanilla => self.vanilla_icon().and_then(|grass| {
                self.icon_image("mark:grass".into(), size, false, || {
                    std::fs::read(grass).ok()
                })
            }),
            NewKind::Modded => self.loader_logo(loader.id(), size),
        };
        match mark {
            Some(image) if kind == NewKind::Vanilla => {
                instance_icons::centred(image, size, self.scale).into_any_element()
            }
            Some(image) => tile_image(image, size),
            None => {
                let name = match kind {
                    NewKind::Vanilla => "Vanilla",
                    _ => loader.name(),
                };
                letter_tile(white(fill::BUTTON), name, size).into_any_element()
            }
        }
    }

    /// The icon the instance will show, `size` px.
    fn draft_icon(
        &mut self,
        editing: Option<&Instance>,
        kind: NewKind,
        name: &str,
        size: f32,
    ) -> AnyElement {
        let Some(d) = self.instances.dialog.as_ref() else {
            return div().into_any_element();
        };
        let d_logo = d.logo;
        let (icon, picture) = (
            d.icon.clone(),
            d.picture
                .as_ref()
                .map(|p| p.as_ref().map(|c| c.png.clone())),
        );
        let image = match picture {
            Some(Some(png)) => {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                png.hash(&mut h);
                self.icon_image(format!("chosen:{:x}", h.finish()), size, false, || {
                    Some(png)
                })
            }
            Some(None) => None,
            None => editing.and_then(|i| self.instance_picture(i, size)),
        };
        if let Some(image) = image {
            // A loader's logo is a tile, round as the others; a picture keeps its own shape.
            return match d_logo {
                Some(_) => tile_image(image, size),
                None => instance_icons::centred(image, size, self.scale).into_any_element(),
            };
        }
        match icon {
            Some(Icon::Colour(hex)) => {
                let rgb = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
                letter_tile(ui::color(rgb), name, size).into_any_element()
            }
            Some(Icon::Texture(texture)) => match self.texture_image(&texture, size) {
                Some(image) => tile_image(image, size),
                None => letter_tile(ui::color(self.look.accent), name, size).into_any_element(),
            },
            None => {
                let loader = self.draft_loader(editing);
                self.auto_mark(kind, loader, size)
            }
        }
    }

    /// The left column: the instance as it will be, and its kind.
    fn instance_aside(
        &mut self,
        editing: Option<&Instance>,
        kind: NewKind,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let typed = self.servers.value(Field::InstanceName).trim().to_string();
        let name = if typed.is_empty() {
            tr("New instance").to_string()
        } else {
            typed
        };
        let loader = self.draft_loader(editing);
        let minecraft = trf(
            "Minecraft {version}",
            &[(
                "version",
                &self.draft_minecraft(editing).unwrap_or_else(|| "…".into()),
            )],
        );
        let mut summary = match kind {
            NewKind::Horizon => match self.draft_horizon() {
                Some(tag) => format!("Horizon · {}", version_label(&tag)),
                None => "Horizon".into(),
            },
            NewKind::Vanilla => format!("{} · {minecraft}", tr("Vanilla")),
            NewKind::Modded => format!("{} · {minecraft}", loader.name()),
        };
        let group = self.servers.value(Field::InstanceGroup).trim().to_string();
        if !group.is_empty() {
            summary = format!("{summary} · {group}");
        }
        let icon = self.draft_icon(editing, kind, &name, 88.);
        let note = match kind {
            NewKind::Horizon => {
                tr("Its own worlds and packs. Your options and player name come along.")
            }
            _ => tr("Play downloads the game from Mojang the first time, with the Java it needs."),
        };
        // An instance keeps its kind: editing shows only its own.
        let kinds: Vec<NewKind> = match editing {
            Some(_) => vec![kind],
            None => vec![NewKind::Horizon, NewKind::Vanilla, NewKind::Modded],
        };
        let names: Vec<&str> = loaders().iter().map(|l| l.name()).collect();
        let modded = match names.split_last() {
            Some((last, [])) => last.to_string(),
            Some((last, rest)) => trf(
                "{first} or {last}",
                &[("first", &rest.join(", ")), ("last", last)],
            ),
            None => String::new(),
        };
        let mut cards = Vec::new();
        for k in kinds {
            let (id, title, note) = match k {
                NewKind::Horizon => (
                    "horizon",
                    tr("Horizon"),
                    tr("Minecraft rebuilt in Rust, with Horizon mods").to_string(),
                ),
                NewKind::Vanilla => (
                    "vanilla",
                    tr("Vanilla"),
                    tr("Minecraft as Mojang ships it").to_string(),
                ),
                NewKind::Modded => ("modded", tr("Modded"), modded.clone()),
            };
            let tile = match k {
                NewKind::Modded => div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(42.))
                    .rounded(px(11.))
                    .bg(white(fill::BUTTON))
                    .child(ui::icon("puzzle", 21., white(0.85)))
                    .into_any_element(),
                _ => self.auto_mark(k, loader, 42.),
            };
            let card = choice(format!("new-kind-{id}"), k == kind, &t)
                .items_center()
                .gap(px(14.))
                .p(px(12.))
                .child(tile)
                .child(choice_text(title, note));
            cards.push(if editing.is_some() {
                card.cursor_default()
            } else {
                card.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.pick_kind(k, cx);
                    cx.notify();
                }))
            });
        }
        // A modpack becomes an instance by being imported, on the Mods page.
        let import = (editing.is_none() && !self.instances.dialog.as_ref().is_some_and(|d| d.busy))
            .then(|| {
                keyboard(
                    div()
                        .id("instance-import")
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(40.))
                        .px(px(8.))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .font_family(horizon_theme::glass::FONT)
                        .text_size(px(14.))
                        .text_color(white(ink::BODY))
                        .hover(|s| s.bg(white(fill::ROW_HOVER)).text_color(white(ink::FULL)))
                        .child(ui::icon("package-01", 18., white(ink::BODY)))
                        .child(tr("Import a modpack…")),
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.instances.dialog = None;
                    this.show_modpack(None);
                    cx.notify();
                }))
            });
        div()
            .id("instance-aside")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(340.))
            .h_full()
            .p(px(28.))
            .overflow_y_scroll()
            .rounded_l(px(radius::MODAL))
            .bg(white(0.035))
            .child(
                ui::text(13., FontWeight::MEDIUM, ink::QUIET).child(tr(if editing.is_some() {
                    "Edit instance"
                } else {
                    "New instance"
                })),
            )
            .child(div().mt(px(24.)).child(icon))
            .child(
                div()
                    .mt(px(20.))
                    .truncate()
                    .font_family(horizon_theme::glass::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(30.))
                    .line_height(px(32.))
                    .text_color(white(ink::FULL))
                    .child(name),
            )
            .child(
                ui::text(14., FontWeight::NORMAL, ink::MUTED)
                    .mt(px(8.))
                    .child(summary),
            )
            .child(
                ui::text(13., FontWeight::NORMAL, ink::FAINT)
                    .mt(px(16.))
                    .line_height(px(20.))
                    .child(note),
            )
            .child(
                div()
                    .mt(px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(cards),
            )
            .child(div().flex_1().min_h(px(32.)))
            .children(import)
    }
}

/// "Nightly · c9d474d", "Imported · 2026-10-30", or the tag.
fn version_label(tag: &str) -> String {
    horizon_launcher::nightly::label(tag)
        .or_else(|| build_import::label(tag))
        .unwrap_or_else(|| tag.to_string())
}

impl Launcher {
    /// The right column: the title, what the instance plays, its name and group, its icon, and
    /// the foot.
    fn instance_main(
        &mut self,
        editing: Option<&Instance>,
        kind: NewKind,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (busy, copy, error) = (d.busy, d.copy, d.error.clone());
        let title = match (editing, kind) {
            (Some(i), _) => trf("Edit {name}", &[("name", &i.name)]),
            (None, NewKind::Horizon) => tr("A new Horizon instance").into(),
            (None, NewKind::Vanilla) => tr("A new vanilla instance").into(),
            (None, NewKind::Modded) => tr("A new modded instance").into(),
        };
        let sub = match editing {
            Some(_) => tr("Changes apply the next time it starts. Worlds stay where they are."),
            None => tr("Pick what it plays, then give it a name and an icon."),
        };
        // The mockup's `clamp(1.6rem, 2.4vw, 2.1rem)`.
        let size = (self.window_width * 0.024).clamp(25.6, 33.6);
        let head = div()
            .flex()
            .flex_none()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .px(px(36.))
            .pt(px(32.))
            .pb(px(20.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(horizon_theme::glass::DISPLAY_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(size))
                            .line_height(px(size * 1.25))
                            .text_color(white(ink::FULL))
                            .child(title),
                    )
                    .child(
                        ui::text(14., FontWeight::NORMAL, ink::NOTE)
                            .mt(px(6.))
                            .child(sub),
                    ),
            )
            .child(
                ui::icon_button(
                    "instance-close",
                    "cancel-01",
                    36.,
                    18.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .when(!busy, |b| {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.instances.dialog = None;
                        cx.notify();
                    }))
                }),
            );
        let mut body = div()
            .id("instance-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(36.))
            .px(px(36.))
            .pb(px(32.))
            .overflow_y_scroll();
        match (editing, kind) {
            (_, NewKind::Horizon) => {
                body = body.child(self.horizon_versions(cx));
                if editing.is_none() {
                    body = body.child(self.start_from(copy, cx));
                }
            }
            // A Java instance keeps the game it was made for.
            (Some(_), _) => {}
            (None, _) => {
                body = body.child(self.minecraft_versions(cx));
                if kind == NewKind::Modded {
                    body = body.child(self.loader_choice(cx));
                }
            }
        }
        let name = self
            .field(Field::InstanceName, tr("My instance"), 340., cx)
            .flex_1()
            .min_w_0();
        let group = self
            .field(Field::InstanceGroup, tr("No group"), 340., cx)
            .flex_1()
            .min_w_0();
        body = body.child(section(
            tr("Name and group").into(),
            None,
            div().flex().gap(px(10.)).child(name).child(group),
        ));
        body = body.child(self.icon_choice(editing, kind, cx));
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(head)
            .child(body)
            .child(self.instance_foot(editing, copy, busy, error, cx))
    }

    /// The foot: where the instance lives, Cancel and Create.
    fn instance_foot(
        &mut self,
        editing: Option<&Instance>,
        copy: bool,
        busy: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let place = match editing {
            Some(i) => short(&i.home),
            None => trf(
                "Makes a folder in {path}",
                &[("path", &short(&backend::game_home().join("instances")))],
            ),
        };
        let cancel = keyboard(
            div()
                .id("instance-cancel")
                .flex()
                .items_center()
                .h(px(48.))
                .px(px(20.))
                .rounded(px(radius::ROW))
                .cursor_pointer()
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white(ink::BODY))
                .hover(|s| s.bg(white(fill::ROW_HOVER)).text_color(white(ink::FULL)))
                .child(tr("Cancel")),
            &t,
        );
        let label = match (editing.is_some(), busy) {
            (true, _) => tr("Save changes"),
            (false, true) if copy => tr("Copying…"),
            (false, _) => tr("Create instance"),
        };
        let accent = ui::color(self.look.accent);
        let lit = gpui::hsla(accent.h, accent.s, (accent.l * 1.1).min(1.), 1.);
        let save = keyboard(
            div()
                .id("instance-save")
                .flex()
                .items_center()
                .justify_center()
                .h(px(48.))
                .px(px(32.))
                .rounded(px(radius::LARGE))
                .bg(accent)
                .cursor_pointer()
                .hover(move |s| s.bg(lit))
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui::color(self.look.on_accent))
                .child(label),
            &t,
        );
        let buttons = if busy {
            div()
                .flex()
                .gap(px(8.))
                .opacity(0.5)
                .child(cancel)
                .child(save)
        } else {
            div()
                .flex()
                .gap(px(8.))
                .child(cancel.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.instances.dialog = None;
                    cx.notify();
                })))
                .child(save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.save_instance(cx);
                    cx.notify();
                })))
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(36.))
            .py(px(20.))
            .rounded_br(px(radius::MODAL))
            .bg(white(0.03))
            // Why Create failed takes the folder's place, by the buttons.
            .child(match error {
                Some(e) => ui::text(13., FontWeight::NORMAL, ink::FULL)
                    .min_w_0()
                    .text_color(ui::color(horizon_theme::glass::DANGER))
                    .child(e),
                None => ui::text(13., FontWeight::NORMAL, ink::FAINT)
                    .min_w_0()
                    .truncate()
                    .child(place),
            })
            .child(buttons.flex_none())
    }

    /// Horizon's installed versions; the launcher's is the newest, and the one an instance
    /// that keeps none follows.
    fn horizon_versions(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let soft = self.look.accent_soft;
        let follows = self.launcher_version().cloned();
        let chosen = self.draft_horizon();
        let rows: Vec<_> = self
            .installed
            .clone()
            .into_iter()
            .map(|tag| {
                let latest = follows.as_ref() == Some(&tag);
                let on = chosen.as_ref() == Some(&tag);
                let date = horizon_launcher::nightly::date_of(&tag).map(str::to_string);
                let pick = tag.clone();
                list_choice(format!("horizon-version-{tag}"), on, &t)
                    .py(px(12.))
                    .child(choice_text(version_label(&tag), date.unwrap_or_default()).flex_1())
                    .children(latest.then(|| {
                        ui::text(13., FontWeight::NORMAL, ink::FULL)
                            .text_color(ui::color(soft))
                            .child(tr("Newest"))
                    }))
                    .child(tick(on, soft))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(d) = &mut this.instances.dialog {
                            d.pin = (!latest).then(|| pick.clone());
                        }
                        cx.notify();
                    }))
            })
            .collect();
        let list = if rows.is_empty() {
            list_box("horizon-versions", 248.).child(
                ui::text(14., FontWeight::NORMAL, ink::QUIET)
                    .px(px(16.))
                    .py(px(24.))
                    .child(tr("No version installed")),
            )
        } else {
            list_box("horizon-versions", 248.).children(rows)
        };
        section(tr("Version").into(), None, list)
    }

    /// Empty, or a copy of the selected instance.
    fn start_from(&mut self, copy: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let selected = self.selected_instance().cloned();
        let from = selected
            .as_ref()
            .map_or_else(|| "Horizon".into(), |i| i.name.clone());
        let icon = match &selected {
            Some(i) => self.glass_instance_icon(i, 36.),
            None => ui::mark_tile(36., self.look.accent_soft).into_any_element(),
        };
        let card = |on: bool, id: &'static str| {
            choice(id, on, &t)
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(12.))
                .p(px(16.))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(d) = &mut this.instances.dialog {
                        d.copy = id == "instance-from-copy";
                    }
                    cx.notify();
                }))
        };
        let empty = card(!copy, "instance-from-empty")
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .rounded(px(radius::CONTROL))
                    .bg(white(fill::BUTTON))
                    .child(ui::icon("plus-sign", 18., white(ink::BODY))),
            )
            .child(choice_text(tr("Empty"), tr("A fresh game folder")));
        let copied = card(copy, "instance-from-copy")
            .child(icon)
            .child(choice_text(
                trf("A copy of {name}", &[("name", &from)]),
                tr("Its mods, worlds and options"),
            ));
        section(
            tr("Start from").into(),
            None,
            div().flex().gap(px(10.)).child(empty).child(copied),
        )
    }

    /// The icons: the instance's own mark, the loaders' logos and the prepared files' blocks and
    /// items, then the colours and Choose image….
    fn icon_choice(
        &mut self,
        editing: Option<&Instance>,
        kind: NewKind,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (icon, logo) = (d.icon.clone(), d.logo);
        let file_picture = matches!(d.picture, Some(Some(_))) && logo.is_none();
        let own_picture = d.picture.is_none()
            && editing.is_some_and(|i| i.home.join(instances::PICTURE).is_file());
        let pictured = matches!(d.picture, Some(Some(_))) || own_picture;
        let typed = self.servers.value(Field::InstanceName).trim().to_string();
        let name = if typed.is_empty() { "N".into() } else { typed };
        let pick = |icon: Option<Icon>| {
            cx.listener(
                move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                    if let Some(d) = &mut this.instances.dialog {
                        d.icon = icon.clone();
                        d.picture = Some(None);
                        d.logo = None;
                    }
                    cx.notify();
                },
            )
        };
        let mut first: Vec<AnyElement> = Vec::new();
        if file_picture || own_picture {
            let image = self.draft_icon(editing, kind, &name, 40.);
            first.push(
                icon_choice("icon-picture", true, &t)
                    .child(image)
                    .into_any_element(),
            );
        }
        let loader = self.draft_loader(editing);
        let auto = self.auto_mark(kind, loader, 40.);
        first.push(
            icon_choice("icon-auto", !pictured && icon.is_none(), &t)
                .child(auto)
                .on_click(pick(None))
                .into_any_element(),
        );
        // Minecraft's grass block from the player's own files, then the loaders' logos: each
        // becomes the instance's picture.
        let grass = self.vanilla_icon().and_then(|g| std::fs::read(g).ok());
        let mut logos: Vec<(&'static str, Vec<u8>)> = Vec::new();
        if let Some(g) = grass {
            logos.push(("grass", g));
        }
        for id in LOGO_ICONS {
            if let Some(png) = instance_icons::logo_png(id) {
                logos.push((id, png.to_vec()));
            }
        }
        for (id, png) in logos {
            let shown = match id {
                "grass" => self
                    .icon_image("mark:grass".into(), 40., false, || Some(png.clone()))
                    .map(|i| instance_icons::centred(i, 40., self.scale).into_any_element()),
                _ => self.loader_logo(id, 40.).map(|i| tile_image(i, 40.)),
            };
            let Some(shown) = shown else {
                continue;
            };
            first.push(
                icon_choice(format!("icon-logo-{id}"), logo == Some(id), &t)
                    .child(shown)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let made = instance_icons::chosen(png.clone(), this.scale);
                        if let (Some(d), Some(made)) = (&mut this.instances.dialog, made) {
                            d.picture = Some(Some(made));
                            d.icon = None;
                            d.logo = Some(id);
                            d.error = None;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        for texture in instances_page::TEXTURES {
            let Some(image) = self.texture_image(texture, 40.) else {
                continue;
            };
            let on = !pictured && icon == Some(Icon::Texture(texture.into()));
            first.push(
                icon_choice(format!("icon-{texture}"), on, &t)
                    .child(tile_image(image, 40.))
                    .on_click(pick(Some(Icon::Texture(texture.into()))))
                    .into_any_element(),
            );
        }
        let colours = COLOURS.map(|rgb| {
            let hex = format!("#{rgb:06x}");
            let on = !pictured && icon == Some(Icon::Colour(hex.clone()));
            icon_choice(format!("icon-{hex}"), on, &t)
                .child(letter_tile(ui::color(rgb), &name, 40.))
                .on_click(pick(Some(Icon::Colour(hex))))
        });
        let choose = ui::quiet_icon(
            "instance-picture",
            Some("image-01"),
            tr("Choose image…"),
            40.,
            &t,
        )
        .ml(px(6.))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.choose_instance_picture(cx);
        }));
        let mut body = div()
            .flex()
            .flex_col()
            .child(div().flex().flex_wrap().gap(px(6.)).children(first))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .mt(px(6.))
                    .children(colours)
                    .child(choose),
            );
        if self.selected().is_none_or(|t| !backend::is_prepared(t)) {
            body = body.child(
                ui::text(13., FontWeight::NORMAL, ink::FAINT).mt(px(12.)).child(tr(
                    "Block and item icons come from your prepared game files once a version is prepared.",
                )),
            );
        }
        section(tr("Icon").into(), None, body)
    }
}
