//! What the window does while the game runs (`backend::WhilePlaying`): stay, minimise, or hide
//! (macOS), and come back to the front when the game closes, with how the run ended.
use super::*;
use backend::WhilePlaying;
use gpui::AnyWindowHandle;

pub(super) struct Aside {
    /// The launcher's window.
    window: AnyWindowHandle,
    /// What the window did when the game started, to undo when it closes.
    stepped: Option<WhilePlaying>,
}

impl Aside {
    pub(super) fn new(window: AnyWindowHandle) -> Self {
        Aside {
            window,
            stepped: None,
        }
    }
}

impl Launcher {
    /// The game started: get out of its way as the player asked. Read from `settings.json`, so
    /// `horizon-launcher settings while-playing …` counts at once.
    pub(super) fn step_aside(&mut self, cx: &mut Context<Self>) {
        // Keep a choice whose save failed available for Retry instead of replacing it from disk.
        if self.settings_save_error.is_none() {
            self.settings.while_playing = Settings::load().while_playing;
        }
        let how = self.settings.while_playing.here();
        if how == WhilePlaying::Stay {
            return;
        }
        let window = self.aside.window;
        self.aside.stepped = Some(how);
        cx.defer(move |cx| match how {
            WhilePlaying::Hide => cx.hide(),
            _ => {
                let _ = window.update(cx, |_, window, _| window.minimize_window());
            }
        });
    }

    /// The game closed: back to the front, where its play time, errors and log are.
    pub(super) fn come_back(&mut self, cx: &mut Context<Self>) {
        if self.aside.stepped.take().is_none() {
            return;
        }
        let window = self.aside.window;
        cx.defer(move |cx| {
            cx.activate(true);
            let _ = window.update(cx, |_, window, _| window.activate_window());
        });
    }

    /// Settings' row for it.
    pub(super) fn while_playing_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut options = vec![
            (WhilePlaying::Stay, "Stay"),
            (WhilePlaying::Minimize, "Minimise"),
        ];
        if cfg!(target_os = "macos") {
            options.push((WhilePlaying::Hide, "Hide"));
        }
        let current = self.settings.while_playing.here();
        let picker = segments("while-playing", &options, current, &t, {
            let this = cx.entity().downgrade();
            move |choice, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.settings.while_playing = choice;
                    this.save_settings();
                    cx.notify();
                });
            }
        });
        let said = match current {
            WhilePlaying::Stay => "The launcher stays open while you play.",
            WhilePlaying::Minimize => {
                "The launcher goes down when the game starts, and comes back when it closes."
            }
            WhilePlaying::Hide => {
                "The launcher hides when the game starts, and comes back when it closes."
            }
        };
        index_row(
            n,
            tr("While playing"),
            Some(div().child(tr(said)).into_any_element()),
            Some(picker.into_any_element()),
            &t,
        )
    }
}
