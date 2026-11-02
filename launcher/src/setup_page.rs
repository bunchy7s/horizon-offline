//! The Windows setup window: `horizon-launcher-setup.exe` installs, `horizon-launcher.exe
//! uninstall` removes. The launcher's own sky and plates (docs/design-system.md, "Night"); the
//! work is `horizon_launcher::installer`, on a thread, reported over a channel.
use crate::plate::plate;
use crate::shell::{action, c, label, text, Action};
use crate::sky::Sky;
use crate::theme::{self, night, Theme};
use crate::titlebar;
use gpui::{
    div, prelude::*, px, size, App, Bounds, ClickEvent, Context, Div, PathPromptOptions, Render,
    Task, Window, WindowBounds, WindowOptions,
};
use horizon_launcher::backend::Settings;
use horizon_launcher::i18n::{self, tr};
use horizon_launcher::installer::{self, Mode, Options, Step};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

enum Stage {
    Welcome,
    Working { step: Step, fraction: f32 },
    Done { left: Vec<String> },
    Failed(String),
}

enum Event {
    Step(Step, f32),
    Done(Result<Vec<String>, String>),
}

pub struct Setup {
    mode: Mode,
    theme: Theme,
    sky: Sky,
    sky_wake: Option<(Instant, Task<()>)>,
    options: Options,
    /// An install is already there: the button says Update.
    update: bool,
    stage: Stage,
    rx: Option<Receiver<Event>>,
}

/// Opens the setup window in place of the launcher's.
pub fn open(mode: Mode, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(520.))),
            titlebar: Some(titlebar::options()),
            app_id: Some("horizon-launcher".into()),
            ..Default::default()
        },
        |window, cx| cx.new(|_| Setup::new(mode, window)),
    )
    .expect("window");
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    cx.activate(true);
}

fn step_text(step: Step) -> String {
    match step {
        Step::Files => tr("Copying files").to_string(),
        Step::Shortcuts => tr("Making shortcuts").to_string(),
        Step::Registry => tr("Registering with Windows").to_string(),
        Step::Removing => tr("Removing files").to_string(),
        Step::Data => tr("Removing the launcher's data").to_string(),
    }
}

impl Setup {
    fn new(mode: Mode, window: &mut Window) -> Self {
        let settings = Settings::load();
        i18n::set(settings.language());
        titlebar::apply_appearance(window, settings.appearance);
        Self {
            mode,
            theme: Theme::for_appearance(settings.appearance, settings.accent, window.appearance()),
            sky: Sky::default(),
            sky_wake: None,
            options: Options::new(),
            update: installer::installed().is_some(),
            stage: Stage::Welcome,
            rx: None,
        }
    }

    /// Draws the window again when the sky next changes (as `Launcher::wake_sky`).
    fn wake_sky(&mut self, after: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + after;
        if self.sky_wake.as_ref().is_some_and(|(when, _)| *when <= at) {
            return;
        }
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                this.sky_wake = None;
                cx.notify();
            });
        });
        self.sky_wake = Some((at, timer));
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.stage = Stage::Working {
            step: if self.mode == Mode::Install {
                Step::Files
            } else {
                Step::Shortcuts
            },
            fraction: 0.02,
        };
        let (mode, options) = (self.mode, self.options.clone());
        std::thread::spawn(move || {
            let report = |step: Step, fraction: f32| {
                let _ = tx.send(Event::Step(step, fraction));
            };
            let result = match mode {
                Mode::Install => installer::install(&options, report).map(|_| Vec::new()),
                Mode::Uninstall => installer::uninstall(options.remove_data, report),
            };
            let _ = tx.send(Event::Done(result));
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            match this.update(cx, |this, cx| this.drain(cx)) {
                Ok(true) => {}
                _ => break,
            }
        })
        .detach();
        cx.notify();
    }

    /// Takes what the worker said; false once it has finished.
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        let (mut changed, mut finished) = (false, false);
        while let Ok(event) = rx.try_recv() {
            changed = true;
            match event {
                Event::Step(step, fraction) => self.stage = Stage::Working { step, fraction },
                Event::Done(Ok(left)) => {
                    self.stage = Stage::Done { left };
                    finished = true;
                }
                Event::Done(Err(e)) => {
                    self.stage = Stage::Failed(e);
                    finished = true;
                }
            }
        }
        if finished {
            self.rx = None;
        }
        if changed {
            cx.notify();
        }
        !finished
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.options.dir = installer::folder_for(&path);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        if let Some(exe) = installer::installed_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
        cx.quit();
    }

    /// A switch with its words (and a quiet line under them).
    fn toggle(
        &self,
        id: &'static str,
        on: bool,
        words: String,
        hint: Option<String>,
        flip: fn(&mut Options),
        cx: &mut Context<Self>,
    ) -> Div {
        let (t, n) = (self.theme, self.theme.night);
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(crate::switch(id, on, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    flip(&mut this.options);
                    cx.notify();
                },
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(text(night::UI, c(n.ink)).child(words))
                    .children(hint.map(|h| text(night::CAPTION, c(n.muted)).child(h))),
            )
    }

    fn folder_row(&self, cx: &mut Context<Self>) -> Div {
        let (t, n) = (self.theme, self.theme.night);
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(night::CAPTION, c(n.muted)).child(tr("Install to").to_string()))
                    .child(
                        text(theme::MICRO, c(n.ink_soft))
                            .font_family(theme::MONO)
                            .truncate()
                            .child(self.options.dir.display().to_string()),
                    ),
            )
            .child(
                action("change", Action::Raised, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                    .child(label(tr("Change…").to_string())),
            )
    }

    fn card(&self, cx: &mut Context<Self>) -> Div {
        let (t, n) = (self.theme, self.theme.night);
        let mut body = div().flex().flex_col().gap(px(16.));
        match &self.stage {
            Stage::Welcome if self.mode == Mode::Install => {
                body = body
                    .child(text(night::UI, c(n.ink_soft)).child(
                        tr("Installs Horizon Launcher for this account. No administrator needed.")
                            .to_string(),
                    ))
                    .child(self.folder_row(cx))
                    .child(self.toggle(
                        "desktop",
                        self.options.desktop,
                        tr("Desktop shortcut").to_string(),
                        None,
                        |o| o.desktop = !o.desktop,
                        cx,
                    ))
                    .child(div().flex().justify_end().child(
                        action("go", Action::Main, &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)))
                            .child(label(if self.update {
                                tr("Update").to_string()
                            } else {
                                tr("Install").to_string()
                            })),
                    ));
            }
            Stage::Welcome => {
                body = body
                    .child(text(night::UI, c(n.ink_soft)).child(
                        tr("Removes Horizon Launcher from this computer. Your worlds and the game's folder stay.")
                            .to_string(),
                    ))
                    .child(self.toggle(
                        "data",
                        self.options.remove_data,
                        tr("Also delete the launcher's data").to_string(),
                        Some(
                            tr("Game builds, servers made in the launcher, settings, skins.")
                                .to_string(),
                        ),
                        |o| o.remove_data = !o.remove_data,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(12.))
                            .child(
                                action("cancel", Action::Raised, &t)
                                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.quit()))
                                    .child(label(tr("Cancel").to_string())),
                            )
                            .child(
                                action("go", Action::Main, &t)
                                    .on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                                    )
                                    .child(label(tr("Uninstall").to_string())),
                            ),
                    );
            }
            Stage::Working { step, fraction } => {
                body = body
                    .child(text(night::UI, c(n.ink)).child(step_text(*step)))
                    .child(crate::progress_bar(Some((0., *fraction)), &t));
            }
            Stage::Done { left } => {
                let said = match self.mode {
                    Mode::Install => tr("Horizon Launcher is installed.").to_string(),
                    Mode::Uninstall => tr("Horizon Launcher was removed.").to_string(),
                };
                body = body.child(text(night::TITLE, c(n.ink)).child(said));
                if !left.is_empty() {
                    body = body.child(
                        text(night::CAPTION, c(n.warn))
                            .child(tr("These could not be removed:").to_string()),
                    );
                    for item in left {
                        body = body.child(
                            text(theme::MICRO, c(n.muted))
                                .font_family(theme::MONO)
                                .child(item.clone()),
                        );
                    }
                }
                // The window's own close control is in the title bar, so the last screen has only
                // the action worth taking; an uninstall just says so and is closed there.
                if self.mode == Mode::Install {
                    body = body.child(
                        div().flex().items_center().justify_end().child(
                            action("launch", Action::Main, &t)
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.launch(cx)),
                                )
                                .child(label(tr("Launch Horizon").to_string())),
                        ),
                    );
                }
            }
            Stage::Failed(error) => {
                body = body
                    .child(
                        text(night::TITLE, c(n.ink)).child(tr("Something went wrong.").to_string()),
                    )
                    .child(text(night::CAPTION, c(n.danger)).child(error.clone()))
                    .child(
                        div().flex().justify_end().child(
                            action("close", Action::Raised, &t)
                                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.quit()))
                                .child(label(tr("Close").to_string())),
                        ),
                    );
            }
        }
        plate(&n)
            .w(px(560.))
            .p(px(32.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(text(night::DISPLAY, c(n.ink)).child("Horizon"))
            .child(body)
    }
}

impl Render for Setup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let viewport = window.viewport_size();
        self.sky.free_replaced(window);
        let (sky, next) = self.sky.render(
            &t.night,
            (viewport.width.into(), viewport.height.into()),
            window.scale_factor(),
            window.is_window_active(),
            Instant::now(),
        );
        if let Some(next) = next {
            self.wake_sky(next, cx);
        }
        let card = self.card(cx);
        div()
            .flex()
            .flex_col()
            .relative()
            .size_full()
            .bg(t.side)
            .child(sky)
            .children(titlebar::render(&t, window))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
            .children(titlebar::resize_handles(window))
    }
}
