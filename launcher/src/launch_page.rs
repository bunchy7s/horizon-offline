//! Launch… on Home's Instance row: what runs around the selected instance's game
//! (`horizon_launcher::launch`): a command before it, a wrapper, and environment variables.
use super::servers_page::Field;
use super::*;
use horizon_launcher::launch;

pub(super) struct LaunchPanel {
    /// The instance it is for: picking another closes it.
    instance: String,
    /// What Save did, or why not.
    saved: Option<Result<String, String>>,
    /// What this computer has for the Linux switches, looked at when the panel opens.
    found: launch::Found,
}

/// A Linux switch: which one, its label, and what it does.
#[derive(Clone, Copy)]
enum Toggle {
    GameMode,
    MangoHud,
    Gpu,
}

impl Launcher {
    pub(super) fn toggle_launch(&mut self) {
        if self.launch.take().is_some() {
            return;
        }
        let Some(i) = self.selected_instance().cloned() else {
            return;
        };
        self.servers
            .text(Field::PreLaunch)
            .set(i.launch.pre_launch.clone());
        self.servers
            .text(Field::Wrapper)
            .set(i.launch.wrapper.clone());
        self.servers
            .text(Field::LaunchEnv)
            .set(launch::env_line(&i.launch.env));
        self.servers
            .text(Field::PostExit)
            .set(i.launch.post_exit.clone());
        self.launch = Some(LaunchPanel {
            instance: i.id,
            saved: None,
            found: launch::Found::here(),
        });
    }

    pub(super) fn save_launch(&mut self) {
        let Some(id) = self.launch.as_ref().map(|p| p.instance.clone()) else {
            return;
        };
        let pre = self.servers.value(Field::PreLaunch).trim().to_string();
        let wrapper = self.servers.value(Field::Wrapper).trim().to_string();
        let post_exit = self.servers.value(Field::PostExit).trim().to_string();
        let saved = launch::parse_env(self.servers.value(Field::LaunchEnv)).and_then(|env| {
            instances::edit(&id, |m| {
                m.launch = launch::Launch {
                    pre_launch: pre,
                    wrapper,
                    env,
                    post_exit,
                    ..m.launch.clone()
                }
            })
        });
        let saved = saved.map(|i| {
            let said = match i.launch.summary() {
                Some(what) => format!("Saved: the next Play {what}."),
                None => "Saved: nothing runs around the game.".to_string(),
            };
            self.reload_instances();
            said
        });
        if let Some(p) = &mut self.launch {
            p.saved = Some(saved);
        }
    }

    /// Flips a Linux switch, saved at once.
    fn flip(&mut self, toggle: Toggle) {
        let Some(panel) = &mut self.launch else {
            return;
        };
        let saved = instances::edit(&panel.instance, |m| {
            let on = match toggle {
                Toggle::GameMode => &mut m.launch.gamemode,
                Toggle::MangoHud => &mut m.launch.mangohud,
                Toggle::Gpu => &mut m.launch.dedicated_gpu,
            };
            *on = !*on;
        });
        panel.saved = Some(saved.map(|i| match i.launch.summary() {
            Some(what) => format!("Saved: the next Play {what}."),
            None => "Saved: nothing runs around the game.".to_string(),
        }));
        self.reload_instances();
    }

    /// The Linux switches this computer can use: GameMode and MangoHud when installed, the
    /// dedicated GPU with two cards.
    fn linux_switches(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let found = self.launch.as_ref()?.found;
        let now = self.selected_instance()?.launch.clone();
        let gpu_about = match found.nvidia {
            true => "NVIDIA render offload: __NV_PRIME_RENDER_OFFLOAD and its Vulkan layer.",
            false => "Mesa's DRI_PRIME=1, so Vulkan picks the other card.",
        };
        let mut list = div().flex().flex_col().gap(px(10.));
        for (toggle, shown, on, label, about) in [
            (
                Toggle::GameMode,
                found.gamemode,
                now.gamemode,
                "GameMode",
                "Feral's gamemoderun: CPU and scheduler tuned while you play.",
            ),
            (
                Toggle::MangoHud,
                found.mangohud,
                now.mangohud,
                "MangoHud",
                "The frame time, CPU and GPU overlay.",
            ),
            (
                Toggle::Gpu,
                found.two_gpus,
                now.dedicated_gpu,
                "Dedicated graphics card",
                gpu_about,
            ),
        ] {
            if !shown {
                continue;
            }
            list = list.child(
                div()
                    .id(SharedString::from(format!("launch-switch-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .child(switch(
                        SharedString::from(format!("launch-switch-{label}-s")),
                        on,
                        &t,
                    ))
                    .child(text(theme::LABEL).text_color(t.ink_soft).child(label))
                    .child(text(theme::CAPTION).text_color(t.muted).child(about))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.flip(toggle);
                        cx.notify();
                    })),
            );
        }
        found.any().then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(caps("On this computer").text_color(t.muted))
                .child(list)
        })
    }

    /// The panel under the Instance row, while open for the selected instance.
    pub(super) fn launch_panel(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let selected = self.selected_instance().map(|i| i.id.clone());
        if self.launch.as_ref().map(|p| &p.instance) != selected.as_ref() {
            self.launch = None;
        }
        let saved = self.launch.as_ref()?.saved.clone();
        let row = |this: &mut Self,
                   cx: &mut Context<Self>,
                   label: &'static str,
                   field: Field,
                   placeholder: &'static str,
                   about: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(caps(label).text_color(t.muted))
                .child(this.field(field, placeholder, 520., cx))
                .child(text(theme::CAPTION).text_color(t.muted).child(about))
        };
        let pre = row(
            self,
            cx,
            "Before the game",
            Field::PreLaunch,
            "For example ./backup.sh",
            "A shell command, run in the instance's folder. The game starts only if it succeeds.",
        );
        let wrapper = row(
            self,
            cx,
            "Run the game through",
            Field::Wrapper,
            "For example nice -n 5",
            "A program and its arguments; the game's command line goes after them.",
        );
        let env = row(
            self,
            cx,
            "Environment variables",
            Field::LaunchEnv,
            "For example NAME=value OTHER=\"two words\"",
            "For the game and the commands; INST_NAME, INST_DIR and the other instance variables are set too.",
        );
        let post = row(
            self,
            cx,
            "After the game",
            Field::PostExit,
            "For example ./backup.sh",
            "A shell command, run in the instance's folder once the game has exited.",
        );
        let save = button("launch-save", "Save", Kind::Secondary, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.save_launch();
                cx.notify();
            },
        ));
        let result = saved.map(|s| match s {
            Ok(said) => text(theme::CAPTION).text_color(t.muted).child(said),
            Err(e) => text(theme::CAPTION).text_color(t.danger).child(e),
        });
        Some(
            plate(&t)
                .flex()
                .flex_col()
                .gap(px(14.))
                .px(px(14.))
                .py(px(12.))
                .ml(px(40.))
                .child(caps("When it starts").text_color(t.muted))
                .child(pre)
                .child(wrapper)
                .child(env)
                .child(post)
                .children(self.linux_switches(cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(save)
                        .children(result),
                ),
        )
    }
}
