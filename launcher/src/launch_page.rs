//! Launch… on Home's Instance row: what runs around the selected instance's game
//! (`horizon_launcher::launch`): a command before it, a wrapper, and environment variables.
use super::servers_page::Field;
use super::*;
use gpui::{
    canvas, relative, Bounds, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point,
};
use horizon_launcher::launch;

pub(super) struct LaunchPanel {
    /// The instance it is for: picking another closes it.
    instance: String,
    /// What Save did: Ok with the summary of what runs (None when nothing runs), or Err with reason.
    saved: Option<Result<Option<String>, String>>,
    /// What this computer has for the Linux switches, looked at when the panel opens.
    found: launch::Found,
}

/// What saving the memory slider did, kept to show under the slider in the active language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MemorySaved {
    /// Saved to the instance: the limit in MB, or None for no limit.
    Ok(Option<u32>),
    /// What was typed was not a valid memory limit.
    Invalid,
    /// Saving failed on disk.
    Error(String),
}

/// The memory row on Home. What it shows comes from the selected instance, except during a drag.
#[derive(Default)]
pub(super) struct MemorySlider {
    /// The computer's memory in MiB, read once: the slider's top.
    total: Option<u32>,
    area: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// The value being dragged to; inside, none is no limit.
    dragging: Option<Option<u32>>,
    /// What the last change did, or why not.
    saved: Option<MemorySaved>,
    /// The instance whose value is being typed: saved to it when the box loses the keyboard, even
    /// if that's because another instance was picked.
    typing: Option<String>,
    /// The instance `saved` is about.
    shown: Option<String>,
}

impl MemorySlider {
    fn total(&mut self) -> u32 {
        *self.total.get_or_insert_with(launch::total_memory_mb)
    }
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
            self.reload_instances();
            i.launch.summary()
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
        panel.saved = Some(saved.map(|i| i.launch.summary()));
        self.reload_instances();
    }

    /// The value under the pointer. Saving waits until the mouse comes up.
    fn pick_memory(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let slider = &mut self.memory_slider;
        let Some(area) = slider.area.get() else {
            return;
        };
        let width: f32 = area.size.width.into();
        if width <= 0. {
            return;
        }
        let across: f32 = (at.x - area.origin.x).into();
        let total = slider.total();
        let picked = launch::memory_at((across / width).clamp(0., 1.), total);
        slider.saved = None;
        slider.dragging = Some(match instance.java {
            Some(_) => picked.or(Some(total)),
            None => picked,
        });
        cx.notify();
    }

    /// A release ends the drag and saves it to the instance, once.
    fn finish_memory(&mut self, cx: &mut Context<Self>) {
        let Some(memory) = self.memory_slider.dragging.take() else {
            return;
        };
        let Some(id) = self.selected_instance().map(|i| i.id.clone()) else {
            return;
        };
        self.save_memory(&id, memory);
        cx.notify();
    }

    fn save_memory(&mut self, id: &str, memory: Option<u32>) {
        let saved = instances::edit(id, |m| match &mut m.java {
            Some(java) => java.memory_mb = memory,
            None => m.launch.set_memory_limit(memory),
        });
        self.memory_slider.saved = Some(match saved {
            Ok(_) => MemorySaved::Ok(memory),
            Err(e) => MemorySaved::Error(e),
        });
        self.reload_instances();
    }

    /// The instance's saved value, as the row shows it.
    fn saved_memory(instance: &instances::Instance) -> Option<u32> {
        match &instance.java {
            Some(java) => Some(java.memory_mb.unwrap_or(instances::DEFAULT_MEMORY_MB)),
            None => instance.launch.memory_limit(),
        }
    }

    /// Saves what's in the box to `id`, if it's a value and a new one.
    fn save_typed_memory(&mut self, id: &str) {
        let typed = self.servers.value(Field::MemoryLimit).to_owned();
        let Ok(memory) = launch::parse_memory(&typed) else {
            self.memory_slider.saved = Some(MemorySaved::Invalid);
            return;
        };
        let same = instances::list()
            .iter()
            .find(|i| i.id == id)
            .is_some_and(|i| Self::saved_memory(i) == memory);
        if !same {
            self.save_memory(id, memory);
        }
    }

    /// Enter in the box.
    pub(super) fn memory_enter(&mut self) {
        if let Some(id) = self.memory_slider.typing.clone() {
            self.save_typed_memory(&id);
        }
    }

    /// The box follows the saved value and the slider except while it has the keyboard, and saves
    /// when it loses it. Read every frame.
    fn sync_memory_box(&mut self, instance: &str, memory: Option<u32>) {
        if self.memory_slider.shown.as_deref() != Some(instance) {
            self.memory_slider.saved = None;
            self.memory_slider.shown = Some(instance.to_owned());
        }
        if self.servers.has_focus(Field::MemoryLimit) {
            self.memory_slider.typing = Some(instance.to_owned());
            return;
        }
        if let Some(id) = self.memory_slider.typing.take() {
            self.save_typed_memory(&id);
            return;
        }
        let shown = memory.map_or_else(String::new, |mb| mb.to_string());
        if self.servers.value(Field::MemoryLimit) != shown {
            self.servers.text(Field::MemoryLimit).set(shown);
        }
    }

    /// The row on Home, for the selected instance: the most memory its game may hold, from nothing
    /// to all of this computer's, kept with the instance.
    pub(super) fn memory_row(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let instance = self.selected_instance()?.clone();
        let java = instance.java.as_ref();
        let total = self.memory_slider.total();
        let area = self.memory_slider.area.clone();
        let memory = self
            .memory_slider
            .dragging
            .unwrap_or_else(|| Self::saved_memory(&instance));
        self.sync_memory_box(&instance.id, memory);
        let typed = launch::parse_memory(self.servers.value(Field::MemoryLimit));
        // While typing, the slider follows a value it can read.
        let memory = match (self.servers.has_focus(Field::MemoryLimit), typed) {
            (true, Ok(typed)) => typed,
            _ => memory,
        };
        let unit = (memory.is_some() || self.servers.has_focus(Field::MemoryLimit))
            .then(|| text(theme::LABEL).text_color(t.ink_soft).child("MB"));
        let input = self.field(Field::MemoryLimit, tr("No limit"), 120., cx);
        let about = match java {
            Some(_) => "How much memory Java may use (-Xmx). Set it too low and the game can crash.",
            None => "The game stays under this. Very low values make it slower, and it crashes only if it can't run at all. Change it while playing with /ramlimit.",
        };
        let track = div()
            .id("launch-memory")
            .relative()
            .w_full()
            .h(px(20.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| {
                        area.set(Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(relative(launch::memory_fraction(memory, total)))
                    .bg(t.face),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.pick_memory(ev.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                // Only a press that began on the track drags it: a button held down elsewhere, for a
                // scrollbar say, can pass over it without changing what's saved.
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.memory_slider.dragging.is_some()
                {
                    this.pick_memory(ev.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            )
            // `on_mouse_up` fires only while the track is hovered: a drag released off it saves too.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_memory(cx)),
            );
        let result = self.memory_slider.saved.as_ref().map(|s| match s {
            MemorySaved::Ok(Some(mb)) => text(theme::CAPTION).text_color(t.muted).child(trf(
                "Saved: the next Play uses at most {mb} MB.",
                &[("mb", &mb.to_string())],
            )),
            MemorySaved::Ok(None) => text(theme::CAPTION)
                .text_color(t.muted)
                .child(tr("Saved: no memory limit on the next Play.")),
            MemorySaved::Invalid => text(theme::CAPTION).text_color(t.danger).child(tr(
                "Type a number of MB like 4096, or 4gb, or leave it empty for no limit.",
            )),
            MemorySaved::Error(e) => text(theme::CAPTION).text_color(t.danger).child(e.clone()),
        });
        Some(
            blocks::panel(&t)
                .flex()
                .flex_col()
                .gap(px(8.))
                .mb(px(8.))
                .px(px(16.))
                .py(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            blocks::pixel(16., FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .child(tr("Maximum memory")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(input)
                                .children(unit),
                        ),
                )
                .child(track)
                .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
                .children(result),
        )
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
                    .child(text(theme::LABEL).text_color(t.ink_soft).child(tr(label)))
                    .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
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
                .child(caps(tr("On this computer")).text_color(t.muted))
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
                .child(caps(tr(label)).text_color(t.muted))
                .child(this.field(field, tr(placeholder), 520., cx))
                .child(text(theme::CAPTION).text_color(t.muted).child(tr(about)))
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
            Ok(Some(what)) => text(theme::CAPTION)
                .text_color(t.muted)
                .child(trf("Saved: the next Play {what}.", &[("what", &what)])),
            Ok(None) => text(theme::CAPTION)
                .text_color(t.muted)
                .child(tr("Saved: nothing runs around the game.")),
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
                .child(caps(tr("When it starts")).text_color(t.muted))
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
