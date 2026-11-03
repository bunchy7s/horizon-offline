//! Settings > Account's skin panel for an offline name (`horizon_launcher::skins`): the front
//! of the skin and the cape, choosing either from a PNG, and the model.
use super::*;
use gpui::RenderImage;
use horizon_launcher::skins::{self, Model};
use std::path::PathBuf;
use std::sync::Arc;

/// A baked preview, or None without one.
type Preview = Option<Arc<RenderImage>>;

#[derive(Default)]
pub(super) struct SkinPanel {
    /// The offline name it is for.
    pub(super) name: String,
    /// The two previews, baked at the window's scale (skin, cape), or None without one.
    seen: Option<(f32, Preview, Preview)>,
    /// What the last choice did, or why not.
    note: Option<Result<String, String>>,
}

/// Points per skin texel in the preview.
const TEXEL: f32 = 6.;

impl Launcher {
    pub(super) fn open_skin(&mut self, name: String) {
        self.skin = match self.skin.take() {
            Some(p) if p.name == name => None,
            _ => Some(SkinPanel {
                name,
                ..SkinPanel::default()
            }),
        };
    }

    fn save_look(&mut self, change: impl FnOnce(&mut skins::Look)) -> Result<(), String> {
        let Some(name) = self.skin.as_ref().map(|p| p.name.clone()) else {
            return Ok(());
        };
        let mut accounts = Accounts::load();
        // The name may have been forgotten while a file dialog was open: keep nothing for it.
        if !accounts
            .offline
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&name))
        {
            return Ok(());
        }
        let mut look = accounts.look(&name);
        change(&mut look);
        accounts.looks.insert(name.to_ascii_lowercase(), look);
        accounts.save()?;
        self.accounts = accounts;
        if let Some(p) = &mut self.skin {
            p.seen = None;
        }
        Ok(())
    }

    fn choose_look_file(&mut self, cape: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Choose").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.take_look_file(path, cape);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn take_look_file(&mut self, path: PathBuf, cape: bool) {
        let Some(name) = self.skin.as_ref().map(|p| p.name.clone()) else {
            return;
        };
        // The name may have been forgotten while the file dialog was open.
        if !Accounts::load()
            .offline
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&name))
        {
            return;
        }
        let done = skins::keep(&skins::dir(), &name, &path, cape).and_then(|()| {
            self.save_look(|l| match cape {
                false => l.skin = true,
                true => l.cape = true,
            })
        });
        if let Some(p) = &mut self.skin {
            let said = match cape {
                false => trf("{name}'s skin is kept.", &[("name", &name)]),
                true => trf("{name}'s cape is kept.", &[("name", &name)]),
            };
            p.note = Some(done.map(|()| said));
        }
    }

    fn previews(&mut self) -> (Preview, Preview) {
        let scale = self.scale;
        let Some(panel) = &mut self.skin else {
            return (None, None);
        };
        if let Some((s, skin, cape)) = &panel.seen {
            if *s == scale {
                return (skin.clone(), cape.clone());
            }
        }
        let look = self.accounts.look(&panel.name);
        let (skin_file, cape_file) = skins::files(&skins::dir(), &panel.name);
        let read = |f: &PathBuf| std::fs::read(f).ok();
        let skin = read(&skin_file)
            .filter(|_| look.skin)
            .and_then(|b| skins::read_skin(&b).ok())
            .map(|png| art::bake(&skins::front(&png, look.model), 16, 32, TEXEL, scale));
        let cape = read(&cape_file)
            .filter(|_| look.cape)
            .and_then(|b| skins::read_cape(&b).ok())
            .map(|png| art::bake(&skins::cape_front(&png), 10, 16, TEXEL, scale));
        panel.seen = Some((scale, skin.clone(), cape.clone()));
        (skin, cape)
    }

    /// The panel, under the offline names while open.
    pub(super) fn skin_panel(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let name = self.skin.as_ref()?.name.clone();
        if !self.accounts.offline.iter().any(|n| n == &name) {
            self.skin = None;
            return None;
        }
        let look = self.accounts.look(&name);
        let (skin, cape) = self.previews();
        let frame = |w: f32, h: f32, image: Preview, empty: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(w + 24.))
                .h(px(h + 24.))
                .bg(t.well)
                .border_2()
                .border_color(t.outline)
                .child(match image {
                    Some(i) => gpui::img(i).w(px(w)).h(px(h)).into_any_element(),
                    None => text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(tr(empty))
                        .into_any_element(),
                })
        };
        let weak = cx.entity().downgrade();
        let model = segments(
            "skin-model",
            &[(Model::Classic, "Classic arms"), (Model::Slim, "Slim arms")],
            look.model,
            &t,
            move |m, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    let _ = this.save_look(|l| l.model = m);
                    cx.notify();
                });
            },
        );
        let pick = |id: &'static str, label: &'static str, cape: bool, cx: &mut Context<Self>| {
            button(id, label, Kind::Secondary, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.choose_look_file(cape, cx);
                    cx.notify();
                },
            ))
        };
        let drop = |id: &'static str, label: &'static str, cape: bool, cx: &mut Context<Self>| {
            button(id, label, Kind::Ghost, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let _ = this.save_look(|l| match cape {
                        false => l.skin = false,
                        true => l.cape = false,
                    });
                    cx.notify();
                },
            ))
        };
        let mut skin_buttons =
            div()
                .flex()
                .gap(px(6.))
                .child(pick("skin-choose", "Choose a skin…", false, cx));
        if look.skin {
            skin_buttons = skin_buttons.child(drop("skin-remove", "Default skin", false, cx));
        }
        let mut cape_buttons =
            div()
                .flex()
                .gap(px(6.))
                .child(pick("cape-choose", "Choose a cape…", true, cx));
        if look.cape {
            cape_buttons = cape_buttons.child(drop("cape-remove", "No cape", true, cx));
        }
        let note = self.skin.as_ref().and_then(|p| p.note.clone());
        Some(
            plate(&t)
                .flex()
                .gap(px(20.))
                .px(px(16.))
                .py(px(14.))
                .ml(px(40.))
                .child(frame(16. * TEXEL, 32. * TEXEL, skin, "Default"))
                .child(frame(10. * TEXEL, 16. * TEXEL, cape, "No cape"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(12.))
                        .child(caps(trf("{name}'s skin", &[("name", &name)])).text_color(t.muted))
                        .child(skin_buttons)
                        .child(model)
                        .child(cape_buttons)
                        .children(note.map(|n| match n {
                            Ok(said) => text(theme::CAPTION).text_color(t.muted).child(said),
                            Err(e) => text(theme::CAPTION).text_color(t.danger).child(e),
                        }))
                        .child(text(theme::CAPTION).text_color(t.muted).child(tr(
                            "A 64×64 (or 64×32) skin PNG and a 64×32 cape PNG, kept by the \
                             launcher and given to the game when this name plays. The game \
                             draws the default skins for now; it uses these once it can. A \
                             Microsoft account's skin and cape: coming soon.",
                        ))),
                ),
        )
    }
}
