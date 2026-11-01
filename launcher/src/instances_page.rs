//! Instances in the window (`horizon_launcher::instances`): the sidebar's list, by group, and the
//! dialog that makes one (empty, or a copy of the selected one) or changes its name, group and
//! icon.
use super::*;
use gpui::RenderImage;
use horizon_launcher::instances::{self, Icon, Instance, Kind as InstanceKind};
use servers_page::Field;
use std::collections::HashMap;
use std::sync::Arc;

/// Textures offered as icons, when the player's prepared files hold them: block faces and items
/// that read well at sidebar size.
const TEXTURES: [&str; 24] = [
    "grass_block_side",
    "oak_planks",
    "oak_log",
    "cobblestone",
    "bricks",
    "bookshelf",
    "crafting_table_front",
    "tnt_side",
    "sand",
    "snow",
    "ice",
    "obsidian",
    "netherrack",
    "end_stone",
    "glowstone",
    "diamond_ore",
    "diamond",
    "emerald",
    "apple",
    "ender_pearl",
    "iron_pickaxe",
    "diamond_sword",
    "bow",
    "book",
];

/// Colours offered as icons: Horizon's accent and a few that sit beside it in both themes.
const COLOURS: [&str; 8] = [
    "#5b3fd6", "#3f7fd6", "#2f9e8f", "#4f9e2f", "#c9a227", "#d9732f", "#c4405a", "#7d7591",
];

/// The dialog: a new instance (`editing` None) or one being changed. Its name and group are the
/// text fields `Field::InstanceName` and `Field::InstanceGroup`.
pub(super) struct InstanceDialog {
    editing: Option<String>,
    /// A new instance copies the selected one's worlds, options and packs.
    copy: bool,
    icon: Option<Icon>,
    /// A picture chosen since it opened (None in it: the instance's is removed on Save).
    pub(super) picture: Option<Option<instance_icons::Chosen>>,
    pub(super) error: Option<String>,
    busy: bool,
    /// Horizon, or the Java game it plays.
    pub(super) java: new_instance::JavaChoice,
}

/// The instances and their icons as images, kept between frames.
#[derive(Default)]
pub(super) struct Instances {
    pub(super) list: Vec<Instance>,
    pub(super) dialog: Option<InstanceDialog>,
    /// The instance whose Delete is waiting for its confirmation, and why the last one failed.
    confirm_delete: Option<String>,
    delete_error: Option<String>,
    /// Textures, pictures and marks as images, by name and size in device pixels.
    pub(super) icons: HashMap<(String, u32), Option<Arc<RenderImage>>>,
    /// Mojang's versions, for a new Java instance.
    pub(super) versions: new_instance::VersionList,
}

impl Launcher {
    pub(super) fn reload_instances(&mut self) {
        self.instances.list = instances::list();
        // A picture may have come or gone with the change.
        self.instances
            .icons
            .retain(|(key, _), _| !key.starts_with("picture:"));
    }

    /// The instance Play starts.
    pub(super) fn selected_instance(&self) -> Option<&Instance> {
        let id = self
            .settings
            .instance
            .as_deref()
            .unwrap_or(instances::DEFAULT);
        let list = &self.instances.list;
        list.iter().find(|i| i.id == id).or(list.first())
    }

    /// The installed version `instance` plays: its own, else the launcher's.
    pub(super) fn version_of(&self, instance: &Instance) -> Option<&String> {
        match &instance.version {
            Some(own) => self.installed.iter().find(|t| *t == own),
            None => self.launcher_version(),
        }
    }

    /// Keeps the selected instance on `tag`, or lets it follow the launcher's version.
    pub(super) fn pin_version(&mut self, tag: Option<String>) {
        let Some(id) = self.selected_instance().map(|i| i.id.clone()) else {
            return;
        };
        match instances::edit(&id, |m| m.version = tag) {
            Ok(_) => self.reload_instances(),
            Err(e) => self.client_error = Some(e),
        }
    }

    pub(super) fn select_instance(&mut self, instance: &Instance) {
        // A native build's warning names the mods of the instance it was opened for.
        self.native_confirm = None;
        self.native_update = None;
        self.settings.instance =
            (instance.kind != InstanceKind::Default).then(|| instance.id.clone());
        self.settings.save();
    }

    /// A texture from the selected version's prepared files, its first frame, drawn as pixels.
    fn texture_image(&mut self, name: &str, size: f32) -> Option<Arc<RenderImage>> {
        let tag = self.selected().filter(|t| backend::is_prepared(t))?.clone();
        let scale = self.scale;
        self.instances
            .icons
            .entry((name.to_string(), (size * scale) as u32))
            .or_insert_with(|| {
                let path = Icon::Texture(name.into()).texture(&backend::resources(&tag))?;
                let png = image::open(path).ok()?.to_rgba8();
                let w = png.width() as usize;
                // An animated texture is its frames one under another: the first is square.
                let texels: Vec<[u8; 4]> = png.pixels().take(w * w).map(|p| p.0).collect();
                Some(art::bake(&texels, w, w, size / w as f32, scale))
            })
            .clone()
    }

    /// An instance's icon, `size` points square (`instance_icons.rs` says which).
    pub(super) fn instance_icon(&mut self, instance: &Instance, size: f32) -> AnyElement {
        let t = self.theme;
        if let Some(picture) = self.instance_picture(instance, size) {
            return instance_icons::centred(picture, size, self.scale).into_any_element();
        }
        let framed = |d: Div| {
            d.flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(size))
                .border_2()
                .border_color(t.outline)
        };
        match &instance.icon {
            Some(Icon::Colour(hex)) => {
                let rgb = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
                return framed(div().bg(gpui::rgb(rgb)))
                    .child(
                        blocks::pixel((size * 0.42).round(), FontWeight::BOLD)
                            .text_color(gpui::white())
                            .child(initial(&instance.name)),
                    )
                    .into_any_element();
            }
            Some(Icon::Texture(name)) => {
                if let Some(image) = self.texture_image(name, size - 8.) {
                    return framed(div().bg(t.well))
                        .child(gpui::img(image).size(px(size - 8.)))
                        .into_any_element();
                }
            }
            None => {
                if let Some(mark) = self.instance_mark(instance, size) {
                    return instance_icons::centred(mark, size, self.scale).into_any_element();
                }
            }
        }
        blocks::slot(None, &instance.name, size, &t).into_any_element()
    }

    /// The sidebar's instances: those without a group, then each group under its name.
    pub(super) fn instance_items(
        &mut self,
        heading: impl Fn(String) -> Div,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = self.theme;
        let selected = self.selected_instance().map(|i| i.id.clone());
        let on_packs = self.page == Page::Mods && self.mods.on_modpacks();
        let mut out = Vec::new();
        let mut group = None;
        for i in self.instances.list.clone() {
            if i.group != group {
                group = i.group.clone();
                if let Some(g) = &group {
                    out.push(heading(g.to_uppercase()).into_any_element());
                }
            }
            let tag = self.version_of(&i).cloned();
            let prepared = tag.as_deref().is_some_and(backend::is_prepared);
            let (open, sub) = match i.kind {
                InstanceKind::Default | InstanceKind::Own => (
                    self.page == Page::Play && selected.as_ref() == Some(&i.id),
                    match (&i.java, &tag, &i.version) {
                        (Some(java), _, _) => java_line(java),
                        (None, None, Some(own)) => {
                            trf("{version} is not installed", &[("version", own)])
                        }
                        (None, None, None) => tr("No version installed").into(),
                        (None, Some(_), _) if !prepared => tr("Needs preparing").into(),
                        (None, Some(tag), _) if self.native_build_of(tag).is_some() => {
                            let build = self.native_build_of(tag).unwrap_or_default();
                            trf(
                                "Native build · {version}",
                                &[("version", &build.horizon_short())],
                            )
                        }
                        (None, Some(tag), Some(_)) => format!("{} · {tag}", tr("Ready")),
                        (None, Some(_), None) => tr("Ready").into(),
                    },
                ),
                InstanceKind::Port => {
                    let slug = &i.id["port:".len()..];
                    let summary = self.packs.iter().find(|p| p.slug == slug);
                    (
                        on_packs && self.pack_open.as_deref() == Some(slug),
                        summary.map_or_else(|| tr("Modpack").into(), |p| p.summary()),
                    )
                }
                InstanceKind::Pack => (false, tr("From Horizon's registry").into()),
            };
            // Java instances have nothing of Horizon's to prepare: no state square.
            let state = (matches!(i.kind, InstanceKind::Default | InstanceKind::Own)
                && i.java.is_none())
            .then_some(if prepared { t.success } else { t.warn });
            let icon = self.instance_icon(&i, 36.);
            let clicked = i.clone();
            out.push(
                shell::row(format!("side-instance-{}", i.id), open, &t)
                    .child(div().relative().child(icon))
                    .child(shell::row_text(i.name.clone(), sub, state, &t))
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
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// Opens the dialog for a new instance, or for changing `editing`.
    pub(super) fn open_instance_dialog(&mut self, editing: Option<Instance>) {
        let (name, group) = editing
            .as_ref()
            .map(|i| (i.name.clone(), i.group.clone().unwrap_or_default()))
            .unwrap_or_default();
        self.servers.text(Field::InstanceName).set(name);
        self.servers.text(Field::InstanceGroup).set(group);
        self.servers.focus_soon = Some(Field::InstanceName);
        self.instances.dialog = Some(InstanceDialog {
            icon: editing.as_ref().and_then(|i| i.icon.clone()),
            picture: None,
            editing: editing.map(|i| i.id),
            copy: false,
            error: None,
            busy: false,
            java: Default::default(),
        });
        self.servers.text(Field::InstanceVersion).set(String::new());
    }

    /// Escape closes the dialog. Whether it did.
    pub(super) fn instance_escape(&mut self) -> bool {
        if self.instances.dialog.as_ref().is_some_and(|d| !d.busy) {
            self.instances.dialog = None;
            return true;
        }
        false
    }

    /// Creates or changes the instance as the dialog says. A copy can be gigabytes of worlds, so
    /// the work runs in the background.
    pub(super) fn save_instance(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_instance().map(|i| i.id.clone());
        let name = self.servers.value(Field::InstanceName).trim().to_string();
        let group = Some(self.servers.value(Field::InstanceGroup).to_string());
        let Some(d) = self.instances.dialog.as_mut().filter(|d| !d.busy) else {
            return;
        };
        let icon = d.icon.clone();
        let picture = d
            .picture
            .as_ref()
            .map(|p| p.as_ref().map(|c| c.png.clone()));
        let editing = d.editing.clone();
        let copy = selected.filter(|_| d.copy);
        // A new Java instance: what it plays, and its name when none is typed.
        let java = (editing.is_none() && d.java.kind != new_instance::NewKind::Horizon)
            .then(|| d.java.clone());
        let minecraft = java.as_ref().map(|j| {
            j.minecraft(match &self.instances.versions {
                new_instance::VersionList::Ready(v) => Some(v),
                _ => None,
            })
        });
        let Some(d) = self.instances.dialog.as_mut() else {
            return;
        };
        let (java, name) = match (java, minecraft) {
            (Some(_), Some(None)) => {
                d.error = Some(tr("Mojang's version list isn't here yet.").into());
                return;
            }
            (Some(j), Some(Some(mc))) => {
                let name = if name.is_empty() {
                    j.default_name(&mc)
                } else {
                    name
                };
                (Some((j, mc)), name)
            }
            _ => (None, name),
        };
        d.busy = true;
        d.error = None;
        let task = cx.background_executor().spawn(async move {
            let id = match (editing, java) {
                (Some(id), _) => id,
                (None, Some((j, mc))) => new_instance::create_java(&j, &name, &mc)?.id,
                (None, None) => instances::create(&name, copy.as_deref())?.id,
            };
            let instance = instances::edit(&id, |m| {
                m.name = Some(name);
                m.group = group;
                m.icon = icon;
            })?;
            if let Some(png) = picture {
                instances::set_picture(
                    &instance.home,
                    png.as_deref(),
                    &horizon_launcher::desktop_seams::to_trash,
                )?;
            }
            Ok(instance)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(instance) => {
                        this.instances.dialog = None;
                        this.select_instance(&instance);
                        this.reload_instances();
                        if matches!(instance.kind, InstanceKind::Default | InstanceKind::Own) {
                            this.go(Page::Play);
                        }
                    }
                    Err(e) => {
                        if let Some(d) = &mut this.instances.dialog {
                            d.busy = false;
                            d.error = Some(e);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn instance_dialog(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let d = self.instances.dialog.as_ref()?;
        let editing_id = d.editing.clone();
        let chosen = d
            .picture
            .as_ref()
            .map(|p| p.as_ref().map(|c| c.preview.clone()));
        let (editing, copy, icon, error, busy, kind) = (
            d.editing.is_some(),
            d.copy,
            d.icon.clone(),
            d.error.clone(),
            d.busy,
            d.java.kind,
        );
        let java = !editing && kind != new_instance::NewKind::Horizon;
        let n = t.night;
        let from = self
            .selected_instance()
            .map_or_else(|| "Horizon".into(), |i| i.name.clone());
        let row = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    shell::text(theme::night::CAPTION, shell::c(n.muted))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr(label).to_string()),
                )
                .child(control)
        };
        let mut body = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(20.))
            .p(px(28.))
            .child(
                shell::text(theme::night::TITLE, shell::c(n.ink)).child(
                    tr(if editing {
                        "Edit instance"
                    } else {
                        "New instance"
                    })
                    .to_string(),
                ),
            );
        if !editing {
            body = body.child(self.kind_tabs(cx));
        }
        if java {
            body = body.child(self.java_choice_rows(cx));
        }
        body = body.child(row(
            "Name",
            self.field(Field::InstanceName, tr("My instance"), 400., cx)
                .into_any_element(),
        ));
        if !editing && !java {
            let weak = cx.entity().downgrade();
            body = body.child(row(
                "Start from",
                segments(
                    "instance-from",
                    &[(false, "Empty"), (true, "A copy of the selected one")],
                    copy,
                    &t,
                    move |copy, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if let Some(d) = &mut this.instances.dialog {
                                d.copy = copy;
                            }
                            cx.notify();
                        });
                    },
                )
                .into_any_element(),
            ));
            body = body.child(text(theme::CAPTION).text_color(t.muted).child(if copy {
                trf(
                    "{from}'s worlds, options and packs are copied; {from} stays as it is.",
                    &[("from", &from)],
                )
            } else {
                tr("Its own worlds and packs; your options and player name come along.").to_string()
            }));
        }
        body = body.child(row(
            "Group",
            self.field(Field::InstanceGroup, tr("No group"), 400., cx)
                .into_any_element(),
        ));
        let mut tiles: Vec<AnyElement> = Vec::new();
        let tile = |id: String, chosen: bool| {
            div()
                .id(SharedString::from(id))
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.))
                .border_2()
                .border_color(if chosen { t.accent } else { t.rule })
                .cursor_pointer()
                .hover(move |s| s.border_color(t.outline))
        };
        let instance =
            editing_id.and_then(|id| self.instances.list.iter().find(|i| i.id == id).cloned());
        // The picture it will show: one just chosen, else its own unless that is being removed.
        let picture = match chosen {
            Some(chosen) => chosen,
            None => instance
                .as_ref()
                .and_then(|i| self.instance_picture(i, 28.)),
        };
        let pictured = picture.is_some();
        if let Some(image) = picture {
            tiles.push(
                tile("instance-icon-picture".into(), true)
                    .child(instance_icons::centred(image, 28., self.scale))
                    .into_any_element(),
            );
        }
        // Nothing chosen: the instance's mark (its loader's, Horizon's), when it has one.
        let mark = instance.as_ref().and_then(|i| self.instance_mark(i, 28.));
        tiles.push(
            tile("instance-icon-none".into(), icon.is_none() && !pictured)
                .child(match mark {
                    Some(mark) => instance_icons::centred(mark, 28., self.scale).into_any_element(),
                    None => text(theme::MICRO)
                        .text_color(t.muted)
                        .child(tr("NONE"))
                        .into_any_element(),
                })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(d) = &mut this.instances.dialog {
                        d.icon = None;
                        d.picture = Some(None);
                    }
                    cx.notify();
                }))
                .into_any_element(),
        );
        for name in TEXTURES {
            let Some(image) = self.texture_image(name, 28.) else {
                continue;
            };
            let chosen = !pictured && icon == Some(Icon::Texture(name.into()));
            tiles.push(
                tile(format!("instance-icon-{name}"), chosen)
                    .bg(t.well)
                    .child(gpui::img(image).size(px(28.)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(d) = &mut this.instances.dialog {
                            d.icon = Some(Icon::Texture(name.into()));
                            d.picture = Some(None);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        for hex in COLOURS {
            let chosen = !pictured && icon == Some(Icon::Colour(hex.into()));
            let rgb = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
            tiles.push(
                tile(format!("instance-icon-{hex}"), chosen)
                    .child(div().size(px(28.)).bg(gpui::rgb(rgb)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(d) = &mut this.instances.dialog {
                            d.icon = Some(Icon::Colour(hex.into()));
                            d.picture = Some(None);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let choose = shell::action("instance-picture", shell::Action::Quiet, &t)
            .child(shell::label(tr("Choose image…").to_string()))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.choose_instance_picture(cx);
            }));
        // Rows of eleven: a wrapping row is measured as one line, and the plate would end above
        // the buttons.
        let (mut tiles, mut rows) = (tiles.into_iter().peekable(), Vec::new());
        while tiles.peek().is_some() {
            rows.push(div().flex().gap(px(6.)).children(tiles.by_ref().take(11)));
        }
        body = body.child(row(
            "Icon",
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(rows)
                .child(div().flex().child(choose))
                .into_any_element(),
        ));
        if self.selected().is_none_or(|t| !backend::is_prepared(t)) {
            body = body.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(tr("Block and item icons come from your prepared game files: prepare a version to see them.")),
            );
        }
        if let Some(e) = error {
            body = body.child(text(theme::CAPTION).text_color(t.danger).child(e));
        }
        let cancel = shell::action("instance-cancel", shell::Action::Quiet, &t)
            .child(shell::label(tr("Cancel").to_string()));
        let label = match (editing, busy, copy) {
            (_, true, true) if !java => "Copying…",
            (true, false, _) => "Save",
            _ => "Create",
        };
        let save = shell::action("instance-save", shell::Action::Main, &t)
            .h(px(theme::CONTROL + 6.))
            .text_size(px(theme::night::UI.px))
            .child(shell::label(tr(label).to_string()));
        // A modpack becomes an instance by being imported, on the Mods page.
        let import = (!editing && !busy).then(|| {
            shell::action("instance-import", shell::Action::Quiet, &t)
                .child(shell::label(tr("Import a modpack…").to_string()))
                .mr_auto()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.instances.dialog = None;
                    this.show_modpack(None);
                    cx.notify();
                }))
        });
        let foot = div()
            .relative()
            .flex()
            .justify_end()
            .gap(px(8.))
            .px(px(28.))
            .py(px(16.))
            .border_t_1()
            .border_color(plate::alpha(n.stroke, n.rule_alpha))
            .items_center()
            .children(import);
        let foot = if busy {
            foot.child(disabled(cancel)).child(disabled(save))
        } else {
            foot.child(cancel.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.instances.dialog = None;
                cx.notify();
            })))
            .child(save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.save_instance(cx);
                cx.notify();
            })))
        };
        // Opaque, as the sign-in sheet: it sits over the page's content.
        let card = plate::cut(
            theme::night::PIXEL_CORNER,
            2,
            Some(plate::alpha(n.card, 0.98)),
            Some((plate::alpha(n.stroke, 2. * n.stroke_alpha), 1.)),
        )
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .max_w(px(600.))
        .child(body)
        .child(foot);
        Some(
            div()
                .id("instance-dialog")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .px(px(32.))
                .py(px(24.))
                // A Java kind's list can make it taller than the window.
                .overflow_y_scroll()
                .bg(plate::alpha(n.page, 0.8))
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .occlude()
                .child(card),
        )
    }

    /// The Play page's row for the selected instance: its folder, and changing, copying or
    /// exporting it.
    pub(super) fn instance_row(&mut self, n: usize, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?.clone();
        let (home, edit, name) = (
            instance.home.clone(),
            instance.clone(),
            instance.name.clone(),
        );
        // Under the name, wrapping: six actions don't fit beside it at the window's narrowest.
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .mt(px(6.))
            .ml(px(-14.))
            .child(
                button("instance-edit", "Edit…", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.open_instance_dialog(Some(edit.clone()));
                        cx.notify();
                    },
                )),
            )
            .child(
                button("instance-copy", "Copy…", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.open_instance_dialog(None);
                        this.servers
                            .text(Field::InstanceName)
                            .set(format!("{name} copy"));
                        if let Some(d) = &mut this.instances.dialog {
                            d.copy = true;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                button(
                    "instance-export",
                    if self.export.is_some() {
                        "Close export"
                    } else {
                        "Export…"
                    },
                    Kind::Ghost,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_export();
                    cx.notify();
                })),
            )
            .child(
                button(
                    "instance-launch",
                    if self.launch.is_some() {
                        "Close launch"
                    } else {
                        "Launch…"
                    },
                    Kind::Ghost,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_launch();
                    cx.notify();
                })),
            )
            .child(self.shortcut_button(&instance, cx))
            .child(
                button("instance-folder", "Open folder", Kind::Ghost, &t).on_click(
                    move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&home);
                        cx.open_with_system(&home);
                    },
                ),
            )
            .children(self.delete_button(&instance, cx));
        Some(index_row(
            n,
            tr("Instance"),
            Some(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .child(instance.name.clone())
                            .child(
                                text(theme::CAPTION)
                                    .text_color(t.muted)
                                    .child(played_line(&instance)),
                            ),
                    )
                    .children(self.shortcut_note(&instance.id))
                    .child(actions)
                    .children(self.delete_confirmation(&instance, cx))
                    .into_any_element(),
            ),
            None,
            &t,
        ))
    }
}

impl Launcher {
    /// Delete…, for every instance but Horizon's own (the game's home); not while a game runs.
    fn delete_button(&self, instance: &Instance, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if instance.kind == InstanceKind::Default {
            return None;
        }
        let t = self.theme;
        let delete = button("instance-delete", "Delete…", Kind::Ghost, &t);
        if self.client.is_some() || self.starting_client.is_some() {
            return Some(disabled(delete));
        }
        let id = instance.id.clone();
        Some(
            delete.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.instances.confirm_delete = Some(id.clone());
                this.instances.delete_error = None;
                cx.notify();
            })),
        )
    }

    /// "Move it to the Trash?" under the actions, once Delete… is pressed.
    fn delete_confirmation(&self, instance: &Instance, cx: &mut Context<Self>) -> Option<Div> {
        if self.instances.confirm_delete.as_ref() != Some(&instance.id) {
            return None;
        }
        let t = self.theme;
        let id = instance.id.clone();
        let line = match &self.instances.delete_error {
            Some(e) => e.clone(),
            None => trf(
                "Move {name} to the Trash, with its worlds and settings? You can get it back from there.",
                &[("name", &instance.name)],
            ),
        };
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .mt(px(8.))
                .child(text(theme::CAPTION).text_color(t.danger).child(line))
                .child(
                    button("instance-keep", "Keep", Kind::Ghost, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.instances.confirm_delete = None;
                            this.instances.delete_error = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    button("instance-trash", "Move to Trash", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.delete_instance(&id);
                            cx.notify();
                        }),
                    ),
                ),
        )
    }

    fn delete_instance(&mut self, id: &str) {
        if self.client.is_some() || self.starting_client.is_some() {
            return;
        }
        match instances::delete(id) {
            Ok(_) => {
                self.instances.confirm_delete = None;
                // Play starts Horizon again; the window's copy of the settings says so too.
                if self.settings.instance.as_deref() == Some(id) {
                    self.settings.instance = None;
                    self.settings.save();
                }
                self.reload_instances();
            }
            Err(e) => self.instances.delete_error = Some(e),
        }
    }
}

/// "3 h 05 min in total · last played today", or "Not played yet".
pub(super) fn played_line(instance: &Instance) -> String {
    match instance.last_played {
        Some(when) => trf(
            "{time} in total · last played {when}",
            &[
                ("time", &instances::play_time(instance.played)),
                ("when", &worlds_page::ago(when)),
            ],
        ),
        None => tr("Not played yet").into(),
    }
}

/// A Java instance's line in the sidebar: "Java 1.21.4", and its loader ("· Fabric 0.16.9").
fn java_line(j: &instances::JavaGame) -> String {
    match &j.loader {
        Some(l) => format!("Java {} · {} {}", j.minecraft, l.kind.name(), l.version),
        None => format!("Java {}", j.minecraft),
    }
}

/// The first letter or digit of a name, for an icon without a picture.
fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
}
