//! A world's data packs, opened under its row on Home (`horizon_launcher::datapacks`): what is
//! in `datapacks/`, Add (a file picker) or a zip dropped on the world, and Remove (to the trash,
//! after a confirmation).
use super::*;
use gpui::ExternalPaths;
use horizon_launcher::datapacks::{self, DataPack};
use horizon_launcher::worlds::World;
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct WorldPacks {
    /// The world whose packs are open, and its packs as last read.
    open: Option<PathBuf>,
    list: Vec<DataPack>,
    confirm_remove: Option<String>,
    note: Option<Result<String, String>>,
}

impl WorldPacks {
    /// Reads the open world's packs again, after something else changed them.
    pub(super) fn reread(&mut self) {
        if let Some(world) = &self.open {
            self.list = datapacks::list(world);
        }
    }
}

impl Launcher {
    /// Opens or closes the packs of `world` under its row.
    pub(super) fn toggle_world_packs(&mut self, world: &World) {
        if self.world_packs.open.as_ref() == Some(&world.path) {
            self.world_packs = WorldPacks::default();
        } else {
            self.world_packs = WorldPacks {
                open: Some(world.path.clone()),
                list: datapacks::list(&world.path),
                ..WorldPacks::default()
            };
        }
    }

    pub(super) fn world_packs_open(&self, world: &World) -> bool {
        self.world_packs.open.as_ref() == Some(&world.path)
    }

    /// Copies dropped or picked zips into `world`, and opens its packs to show them.
    pub(super) fn add_world_packs(&mut self, world: PathBuf, files: Vec<PathBuf>) {
        let added: Result<Vec<String>, String> = files
            .iter()
            .map(|f| datapacks::add(&world, f).map(|p| p.file))
            .collect();
        self.world_packs = WorldPacks {
            list: datapacks::list(&world),
            note: Some(added.map(|names| {
                format!(
                    "Added {}: the game turns it on when the world opens.",
                    names.join(", ")
                )
            })),
            open: Some(world),
            confirm_remove: None,
        };
    }

    fn choose_world_packs(&mut self, world: PathBuf, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| {
                    this.add_world_packs(world, paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// A world row that takes dropped data packs.
    pub(super) fn world_drop_target(
        &self,
        i: usize,
        world: &World,
        row: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let path = world.path.clone();
        div()
            .id(SharedString::from(format!("world-drop-{i}")))
            .border_2()
            .mx(px(-4.))
            .p(px(2.))
            .border_color(gpui::transparent_black())
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(t.accent))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.add_world_packs(path.clone(), paths.paths().to_vec());
                // Not the window's import as well.
                cx.stop_propagation();
                cx.notify();
            }))
            .child(row)
    }

    /// The open world's packs, under its row.
    pub(super) fn world_packs_panel(
        &mut self,
        world: &World,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if !self.world_packs_open(world) {
            return None;
        }
        let t = self.theme;
        let path = world.path.clone();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .ml(px(62.))
            .mb(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(caps("Data packs").text_color(t.muted))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                button("world-packs-add", "Add data pack…", Kind::Ghost, &t)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.choose_world_packs(path.clone(), cx);
                                        cx.notify();
                                    })),
                            )
                            .child({
                                let dir = world.path.join("datapacks");
                                button("world-packs-folder", "Open folder", Kind::Ghost, &t)
                                    .on_click(move |_, _, cx| {
                                        let _ = std::fs::create_dir_all(&dir);
                                        cx.open_with_system(&dir);
                                    })
                            }),
                    ),
            );
        match &self.world_packs.note {
            Some(Ok(m)) => {
                panel = panel.child(text(theme::CAPTION).text_color(t.muted).child(m.clone()))
            }
            Some(Err(e)) => panel = panel.child(notice(e.clone(), &t)),
            None => {}
        }
        if self.world_packs.list.is_empty() {
            panel = panel.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child("None yet: drop a data pack .zip on the world, or use Add data pack…"),
            );
        }
        for (i, pack) in self.world_packs.list.clone().into_iter().enumerate() {
            let confirming = self.world_packs.confirm_remove.as_ref() == Some(&pack.file);
            let mut actions = div().flex().items_center().gap(px(6.));
            if confirming {
                let (world, gone) = (world.path.clone(), pack.clone());
                actions = actions
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.danger)
                            .child("Move it to the trash?"),
                    )
                    .child(
                        button(
                            SharedString::from(format!("world-pack-keep-{i}")),
                            "Keep",
                            Kind::Ghost,
                            &t,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.world_packs.confirm_remove = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        button(
                            SharedString::from(format!("world-pack-trash-{i}")),
                            "Move to trash",
                            Kind::Secondary,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                let note = datapacks::remove(&world, &gone)
                                    .map(|()| format!("{} moved to the trash.", gone.file));
                                this.world_packs.list = datapacks::list(&world);
                                this.world_packs.note = Some(note);
                                this.world_packs.confirm_remove = None;
                                cx.notify();
                            },
                        )),
                    );
            } else {
                let file = pack.file.clone();
                if !pack.enabled {
                    actions = actions.child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child("Off in the game"),
                    );
                }
                actions = actions.child(
                    button(
                        SharedString::from(format!("world-pack-remove-{i}")),
                        "Remove",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.world_packs.confirm_remove = Some(file.clone());
                            cx.notify();
                        },
                    )),
                );
            }
            panel = panel.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(6.))
                    .child(blocks::icon("package", 16., t.muted))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                text(theme::LABEL)
                                    .text_color(t.ink)
                                    .truncate()
                                    .child(pack.file.clone()),
                            )
                            .child(
                                text(theme::CAPTION)
                                    .text_color(t.muted)
                                    .truncate()
                                    .child(pack.description.clone()),
                            ),
                    )
                    .child(actions),
            );
        }
        Some(panel)
    }
}
