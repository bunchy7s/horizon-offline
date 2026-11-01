//! The selected instance's resource packs and shader packs on Home (`horizon_launcher::packs`):
//! each turned on or off with one click, Remove (to the trash, after a confirmation), and
//! Add (a file picker) or a .zip or .hshader dropped anywhere on the section.
use super::*;
use gpui::{ExternalPaths, Image, ImageFormat};
use horizon_launcher::packs::{self, Kind as PackKind, Pack};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Packs {
    home: Option<PathBuf>,
    resource: Vec<(Pack, Option<Arc<Image>>)>,
    shader: Vec<Pack>,
    fresh: bool,
    /// The pack waiting for its removal to be confirmed: its kind and file.
    confirm_remove: Option<(PackKind, String)>,
    /// What the last action did, or why it failed.
    note: Option<Result<String, String>>,
}

impl Packs {
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

impl Launcher {
    fn load_packs_list(&mut self, home: PathBuf) {
        if self.game_packs.home.as_ref() != Some(&home) {
            self.game_packs = Packs {
                home: Some(home.clone()),
                ..Packs::default()
            };
        }
        if self.game_packs.fresh {
            return;
        }
        // A few small reads per pack (pack.mcmeta, pack.png): fine while drawing.
        self.game_packs.resource = packs::list(&home, PackKind::Resource)
            .into_iter()
            .map(|p| {
                let icon = p
                    .icon
                    .clone()
                    .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                (p, icon)
            })
            .collect();
        self.game_packs.shader = packs::list(&home, PackKind::Shader);
        self.game_packs.fresh = true;
    }

    fn pack_action(&mut self, act: impl FnOnce(&std::path::Path) -> Result<String, String>) {
        let Some(home) = self.game_packs.home.clone() else {
            return;
        };
        self.game_packs.note = Some(act(&home));
        self.game_packs.confirm_remove = None;
        self.game_packs.stale();
    }

    /// Copies dropped or picked files into the instance's pack folders.
    fn add_packs(&mut self, files: Vec<PathBuf>) {
        self.pack_action(|home| {
            let mut added = Vec::new();
            for file in &files {
                added.push(packs::add(home, file)?.title);
            }
            Ok(trf(
                "Added {names}: turn it on below.",
                &[("names", &added.join(", "))],
            ))
        });
    }

    fn choose_packs(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Add").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| {
                    this.add_packs(paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// How many resource and shader packs the selected instance has.
    pub(super) fn packs_count(&mut self) -> usize {
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return 0;
        };
        self.load_packs_list(home);
        self.game_packs.resource.len() + self.game_packs.shader.len()
    }

    /// Home's Packs tab, for the selected instance: its buttons (beside the tabs) and its list.
    pub(super) fn packs_section(&mut self, cx: &mut Context<Self>) -> Option<(Div, Stateful<Div>)> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_packs_list(home.clone());
        let actions = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                button("packs-add", "Add pack…", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.choose_packs(cx);
                        cx.notify();
                    },
                )),
            )
            .child(
                button("packs-folder", "Open folder", Kind::Ghost, &t).on_click(move |_, _, cx| {
                    let dir = home.join(PackKind::Resource.folder());
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                }),
            );
        let mut section = div()
            .id("packs")
            .flex()
            .flex_col()
            .gap(px(8.))
            // The drop outline sits just outside the rows, which line up with the other sections.
            .p(px(2.))
            .mx(px(-4.))
            .border_2()
            .border_color(gpui::transparent_black())
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(t.accent).bg(t.well))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_packs(paths.paths().to_vec());
                // Not the window's import as well.
                cx.stop_propagation();
                cx.notify();
            }));
        match &self.game_packs.note {
            Some(Ok(m)) => {
                section = section.child(text(theme::CAPTION).text_color(t.muted).child(m.clone()))
            }
            Some(Err(e)) => section = section.child(notice(e.clone(), &t)),
            None => {}
        }
        if self.client.is_some() {
            section = section.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                "The game is running: change packs in its own menus, or here once it is closed.",
            )));
        }
        let resource = self.game_packs.resource.clone();
        section = section.child(caps(tr("Resource packs")).text_color(t.muted).mt(px(4.)));
        if resource.is_empty() {
            section = section.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                "None yet: drop a .zip here, or find some on the Mods page.",
            )));
        }
        for (i, (pack, icon)) in resource.into_iter().enumerate() {
            section = section.child(self.pack_row(i, pack, icon, cx));
        }
        let shader = self.game_packs.shader.clone();
        section = section.child(caps(tr("Shader packs")).text_color(t.muted).mt(px(8.)));
        if shader.is_empty() {
            section = section.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(tr("None yet: drop a .hshader or a shader pack .zip here.")),
            );
        }
        for (i, pack) in shader.into_iter().enumerate() {
            section = section.child(self.pack_row(i, pack, None, cx));
        }
        Some((actions, section))
    }

    fn pack_row(
        &mut self,
        i: usize,
        pack: Pack,
        icon: Option<Arc<Image>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let id = |what: &str| {
            SharedString::from(format!("pack-{what}-{:?}-{i}", pack.kind).to_lowercase())
        };
        let picture = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(40.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well);
        let picture = match icon {
            Some(image) => picture.child(gpui::img(image).size(px(36.))),
            None => picture.child(blocks::icon(
                if pack.kind == PackKind::Shader {
                    "zap"
                } else {
                    "package"
                },
                18.,
                t.muted,
            )),
        };
        let running = self.client.is_some();
        let mut actions = div().flex().items_center().gap(px(6.));
        let key = (pack.kind, pack.file.clone());
        if self.game_packs.confirm_remove.as_ref() == Some(&key) {
            let gone = pack.clone();
            actions = actions
                .child(
                    text(theme::CAPTION)
                        .text_color(t.danger)
                        .child(tr("Move it to the trash?")),
                )
                .child(
                    button(id("keep"), "Keep", Kind::Ghost, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.game_packs.confirm_remove = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    button(id("trash"), "Move to trash", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let gone = gone.clone();
                            this.pack_action(move |home| {
                                packs::remove(home, &gone).map(|()| {
                                    trf("{name} moved to the trash.", &[("name", &gone.title)])
                                })
                            });
                            cx.notify();
                        }),
                    ),
                );
        } else {
            let toggle = switch(id("toggle"), pack.enabled, &t);
            let flip = pack.clone();
            let toggle = if running {
                disabled(toggle)
            } else {
                toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let flip = flip.clone();
                    this.pack_action(move |home| {
                        packs::set_enabled(home, &flip, !flip.enabled).map(|()| {
                            match flip.enabled {
                                true => trf("{name} is off.", &[("name", &flip.title)]),
                                false => trf("{name} is on.", &[("name", &flip.title)]),
                            }
                        })
                    });
                    cx.notify();
                }))
            };
            let remove = button(id("remove"), "Remove", Kind::Ghost, &t);
            let remove = if running {
                disabled(remove)
            } else {
                remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.game_packs.confirm_remove = Some(key.clone());
                    cx.notify();
                }))
            };
            actions = actions.child(toggle).child(remove);
        }
        let facts = match pack.description.is_empty() {
            true => pack.file.clone(),
            false if pack.title == pack.file => pack.description.clone(),
            false => format!("{} · {}", pack.description, pack.file),
        };
        plate(&t)
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(8.))
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(14., FontWeight::MEDIUM)
                            .text_color(if pack.enabled { t.ink } else { t.muted })
                            .truncate()
                            .child(pack.title.clone()),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .truncate()
                            .child(facts),
                    ),
            )
            .child(actions)
    }
}
