//! The Instance row's desktop shortcut (`horizon_launcher::shortcut`): make one that plays the
//! selected instance, or take it away.
use super::*;
use horizon_launcher::shortcut;

#[derive(Default)]
pub(super) struct Shortcuts {
    /// The instance last looked at, and whether it has a shortcut: read again only when the
    /// selection changes or a button is pressed, not on every frame.
    seen: Option<(String, bool)>,
    /// What the last press did, for that instance.
    note: Option<(String, Result<String, String>)>,
}

impl Launcher {
    fn has_shortcut(&mut self, id: &str) -> bool {
        match &self.shortcuts.seen {
            Some((seen, has)) if seen == id => *has,
            _ => {
                let has = !shortcut::find(id).is_empty();
                self.shortcuts.seen = Some((id.to_string(), has));
                has
            }
        }
    }

    fn toggle_shortcut(&mut self, id: String, name: String) {
        let has = self.has_shortcut(&id);
        let done = match has {
            false => shortcut::make(&backend::update_target(), &id, &name).map(|made| {
                let file = made[0].file_name().unwrap_or_default().to_string_lossy();
                trf(
                    "{file} is on your desktop: it plays {name}.",
                    &[("file", &file), ("name", &name)],
                )
            }),
            true => shortcut::remove(&id)
                .map(|_| trf("{name}'s shortcut is in the trash.", &[("name", &name)])),
        };
        self.shortcuts.seen = None;
        self.shortcuts.note = Some((id, done));
    }

    /// The row's button.
    pub(super) fn shortcut_button(
        &mut self,
        instance: &instances::Instance,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let label = match self.has_shortcut(&instance.id) {
            true => "Remove shortcut",
            false => "Shortcut",
        };
        let (id, name) = (instance.id.clone(), instance.name.clone());
        button("instance-shortcut", label, Kind::Ghost, &t).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.toggle_shortcut(id.clone(), name.clone());
                cx.notify();
            },
        ))
    }

    /// What the last press did, under the row, while that instance is selected.
    pub(super) fn shortcut_note(&self, id: &str) -> Option<Div> {
        let t = self.theme;
        let (_, said) = self.shortcuts.note.as_ref().filter(|(n, _)| n == id)?;
        Some(match said {
            Ok(said) => text(theme::CAPTION).text_color(t.muted).child(said.clone()),
            Err(e) => text(theme::CAPTION).text_color(t.danger).child(e.clone()),
        })
    }
}
