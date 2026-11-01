//! Files dropped anywhere on the window (`horizon_launcher::import`): a shared instance becomes
//! a new instance, a Java modpack goes to the Modpacks import, a resource or shader pack into
//! the selected instance. The Packs section and the world rows take their own drops first.
use super::*;
use gpui::ExternalPaths;
use horizon_launcher::import::{self, Dropped};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Importing {
    /// Shared instances being made.
    busy: usize,
    /// What the last drop did, or why not.
    note: Option<Result<String, String>>,
}

impl Launcher {
    pub(super) fn import_dropped(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        for file in files {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match import::what(&file) {
                Err(e) => self.importing.note = Some(Err(e)),
                Ok(Dropped::Instance) => self.import_instance(file, cx),
                Ok(Dropped::Modpack) => {
                    self.show_modpack(None);
                    self.pack_job(move |log| horizon_launcher::modpacks::import(&file, log));
                    self.importing.note = Some(Ok(format!(
                        "{name} is a Java modpack: importing it on the Modpacks page."
                    )));
                }
                Ok(Dropped::Pack(_)) => {
                    let Some(i) = self.selected_instance().cloned() else {
                        continue;
                    };
                    self.importing.note =
                        Some(horizon_launcher::packs::add(&i.home, &file).map(|p| {
                            format!(
                                "Added {} to {}'s packs: switch it on under Packs.",
                                p.title, i.name
                            )
                        }));
                    self.game_packs.stale();
                }
                Ok(Dropped::DataPack) => {
                    self.importing.note = Some(Err(format!(
                        "{name} is a data pack: drop it on a world on Home, or install it into \
                         one from there."
                    )))
                }
            }
        }
    }

    fn import_instance(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        self.importing.busy += 1;
        self.importing.note = None;
        let task = cx
            .background_executor()
            .spawn(async move { import::import_instance(&file, &|_, _| {}) });
        cx.spawn(async move |this, cx| {
            let made = task.await;
            let _ = this.update(cx, |this, cx| {
                this.importing.busy -= 1;
                this.importing.note = Some(made.map(|i| {
                    this.reload_instances();
                    this.select_instance(&i);
                    this.go(Page::Play);
                    format!("Made the instance {} from the shared file.", i.name)
                }));
                cx.notify();
            });
        })
        .detach();
    }

    /// The root's drop target: every drop the sections don't take.
    pub(super) fn import_drop_target(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            this.import_dropped(paths.paths().to_vec(), cx);
            cx.notify();
        }))
    }

    /// What the last drop did, in a corner over the page until closed.
    pub(super) fn import_note(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let line = match (&self.importing.note, self.importing.busy) {
            (_, n) if n > 0 => text(theme::LABEL)
                .text_color(t.ink_soft)
                .child("Making the instance from the shared file…"),
            (Some(Ok(said)), _) => text(theme::LABEL)
                .text_color(t.ink_soft)
                .child(said.clone()),
            (Some(Err(e)), _) => text(theme::LABEL).text_color(t.danger).child(e.clone()),
            (None, _) => return None,
        };
        Some(
            div()
                .absolute()
                .bottom(px(24.))
                .right(px(24.))
                .w(px(440.))
                .child(
                    plate(&t)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(16.))
                        .py(px(12.))
                        .border_1()
                        .border_color(t.rule)
                        .bg(t.page)
                        .child(blocks::icon("download", 18., t.accent))
                        .child(div().flex_1().min_w_0().child(line))
                        .child(button("import-close", "Close", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.importing.note = None;
                                cx.notify();
                            }),
                        )),
                ),
        )
    }
}
