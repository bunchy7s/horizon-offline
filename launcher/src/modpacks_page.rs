//! Imported modpacks, on the Mods page's Modpacks tab: import a pack, read its coverage report,
//! port what this computer can and play it as its own profile (`horizon_launcher::modpacks`).
use super::*;
use horizon_launcher::browser::horizon::pack as horizon_pack;
use horizon_launcher::modpacks::{self, Imported};
use horizon_port_pack::report::Status;
use std::path::PathBuf;

/// The report's filter: every row, or one status.
pub(super) const FILTERS: [(Option<Status>, &str); 5] = [
    (None, "All"),
    (Some(Status::Ported), "Ported"),
    (Some(Status::Partial), "Partial"),
    (Some(Status::NotYet), "Not yet"),
    (Some(Status::NotNeeded), "Not needed"),
];

/// Lines of the pipeline's log the page shows while it works.
const LOG_SHOWN: usize = 6;

impl Launcher {
    fn open_pack(&self) -> Option<&Imported> {
        let slug = self.pack_open.as_ref()?;
        self.packs.iter().find(|p| &p.slug == slug)
    }

    /// Runs a pipeline step on a thread; its lines and its end arrive as messages.
    pub(super) fn pack_job(
        &mut self,
        job: impl FnOnce(&dyn Fn(String)) -> Result<Imported, String> + Send + 'static,
    ) {
        if self.pack_busy {
            return;
        }
        self.pack_busy = true;
        self.pack_log.clear();
        self.pack_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |line: String| {
                let _ = lines.send(Msg::PackLine(line));
            };
            let result = job(&log);
            let _ = tx.send(Msg::PackDone(result.map(Box::new)));
        });
    }

    /// A finished step: the pack's new state, opened.
    pub(super) fn pack_done(&mut self, result: Result<Box<Imported>, String>) {
        self.pack_busy = false;
        match result {
            Ok(pack) => {
                self.mods.report_list.reset();
                self.pack_open = Some(pack.slug.clone());
                self.packs.retain(|p| p.slug != pack.slug);
                self.packs.push(*pack);
                self.packs.sort_by_key(|p| p.title().to_ascii_lowercase());
            }
            Err(e) => self.pack_error = Some(e),
        }
    }

    fn choose_pack(&mut self, folder: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.pack_job(move |log| modpacks::import(&path, log));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Ports the open pack; `from` is a folder of the player's jars, chosen when `ask`.
    fn port_pack(&mut self, ask: bool, cx: &mut Context<Self>) {
        let Some(work) = self.open_pack().map(|p| p.work.clone()) else {
            return;
        };
        let resources = self
            .selected()
            .filter(|tag| backend::is_prepared(tag))
            .map(|tag| backend::resources(tag));
        if !ask {
            self.pack_job(move |log| modpacks::port(&work, resources.as_deref(), None, log));
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use these jars".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(from) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.pack_job(move |log| {
                            modpacks::port(&work, resources.as_deref(), Some(&from), log)
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn play_pack(&mut self, work: PathBuf) {
        // The pack's instance plays its own version, if it has one.
        let pack = self.instances.list.iter().find(|i| i.work == work);
        let Some(tag) = pack
            .and_then(|i| self.version_of(i))
            .or_else(|| self.launcher_version())
            .cloned()
        else {
            return;
        };
        if !self.can_start(&tag) {
            return;
        }
        self.starting_pack = Some(work);
        self.start_client(tag);
    }

    /// Reads the imported packs, once (the sidebar and the Modpacks tab both show them).
    pub(super) fn load_packs(&mut self) {
        if self.packs_loaded.is_none() {
            self.mods.report_list.reset();
            self.packs = modpacks::list();
            self.registry_packs = horizon_pack::list();
            self.reload_instances();
            if self.pack_open.is_none() {
                self.pack_open = self.packs.first().map(|p| p.slug.clone());
            }
            self.packs_loaded = Some(Instant::now());
        }
    }

    /// The imported packs, above the Modpacks tab's search.
    pub(super) fn imported_packs(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.load_packs();
        let busy = self.pack_busy;
        let pick = |id: &'static str, label: &'static str, folder: bool| {
            let b = button(id, label, Kind::Secondary, &t);
            if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_pack(folder, cx);
                    cx.notify();
                }))
            }
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(pick("import-file", "Import a pack file…", false))
            .child(pick("import-folder", "Import an instance folder…", true))
            .child(text(theme::CAPTION).w_full().text_color(t.muted).child(
                ".mrpack (Modrinth), .zip (CurseForge), or a Prism, MultiMC or CurseForge instance. \
                 Horizon ports what it can on this computer, from your own downloads, and plays \
                 the pack as its own profile. Nothing is uploaded or shared.",
            ));
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(section_title("Your modpacks", "archive", &t))
            .child(actions);
        if busy || !self.pack_log.is_empty() {
            page = page.child(self.pack_log_plate(&t));
        }
        if let Some(e) = &self.pack_error {
            page = page.child(notice(e.clone(), &t));
        }
        if !self.registry_packs.is_empty() {
            page = page.child(self.registry_pack_rows(cx));
        }
        if self.packs.is_empty() && !self.registry_packs.is_empty() {
            return page;
        }
        if self.packs.is_empty() {
            return page.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child("No modpack imported yet: import one, or find one below."),
            );
        }
        if self.packs.len() > 1 {
            let mut list = div().flex().flex_col();
            for (i, p) in self.packs.iter().enumerate() {
                let open = self.pack_open.as_ref() == Some(&p.slug);
                let slug = p.slug.clone();
                let right = if open {
                    caps("Open").text_color(t.accent).into_any_element()
                } else {
                    button(format!("open-{slug}"), "Open", Kind::Ghost, &t)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pack_open = Some(slug.clone());
                            this.mods.report_list.reset();
                            cx.notify();
                        }))
                        .into_any_element()
                };
                list = list.child(index_row(
                    i + 1,
                    p.title().to_string(),
                    Some(p.summary().into_any_element()),
                    Some(right),
                    &t,
                ));
            }
            page = page.child(list);
        }
        match self.pack_detail(cx) {
            Some(detail) => page.child(detail),
            None => page,
        }
    }

    /// The modpacks installed from Horizon's registry: each plays its own profile.
    fn registry_pack_rows(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut list = div().flex().flex_col();
        for (i, p) in self.registry_packs.iter().enumerate() {
            let (play, remove) = (p.work.clone(), p.work.clone());
            let id = p.id.clone();
            let key = format!("horizon:{id}");
            let confirming = self.confirm_remove.as_deref() == Some(key.as_str());
            let right = div()
                .flex()
                .gap(px(8.))
                .child(
                    button(format!("play-pack-{id}"), "Play", Kind::Primary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_pack(play.clone());
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(
                        format!("remove-pack-{id}"),
                        if confirming {
                            "Remove for good"
                        } else {
                            "Remove"
                        },
                        if confirming {
                            Kind::Secondary
                        } else {
                            Kind::Ghost
                        },
                        &t,
                    )
                    .when(confirming, |b| b.text_color(t.danger))
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if this.confirm_remove.as_ref() == Some(&key) {
                                match horizon_pack::remove(&remove) {
                                    Ok(()) => this.packs_loaded = None,
                                    Err(e) => this.pack_error = Some(e),
                                }
                                this.confirm_remove = None;
                            } else {
                                this.confirm_remove = Some(key.clone());
                            }
                            cx.notify();
                        },
                    )),
                );
            let detail = format!(
                "{} from Horizon's registry, its own profile: {}",
                p.version,
                p.mods.join(", ")
            );
            list = list.child(index_row(
                i + 1,
                p.name.clone(),
                Some(
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(detail)
                        .into_any_element(),
                ),
                Some(right.into_any_element()),
                &t,
            ));
        }
        list
    }

    fn pack_log_plate(&self, t: &Theme) -> Div {
        let start = self.pack_log.len().saturating_sub(LOG_SHOWN);
        plate(t)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(16.))
            .child(
                caps(if self.pack_busy { "Working" } else { "Done" })
                    .text_color(if self.pack_busy { t.accent } else { t.muted }),
            )
            .children(self.pack_log.iter().skip(start).map(|line| {
                text(theme::CAPTION)
                    .font_family(theme::MONO)
                    .text_color(t.ink_soft)
                    .truncate()
                    .child(line.clone())
            }))
    }

    fn pack_detail(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let p = self.open_pack()?;
        let mut counts = div().flex().gap(px(12.));
        for (status, label) in FILTERS.iter().skip(1) {
            let status = status.expect("the first filter is All");
            let color = match status {
                Status::Ported => t.success,
                Status::Partial => t.accent,
                _ => t.ink,
            };
            counts = counts.child(
                plate(&t)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(4.))
                    .p(px(16.))
                    .child(
                        text(theme::DISPLAY)
                            .text_color(color)
                            .child(p.count(status).to_string()),
                    )
                    .child(caps(*label).text_color(t.muted)),
            );
        }
        // Prepared, nothing preparing or running (`play_pack` checks the same).
        let can_play = self.can_start_selected();
        let mut buttons = div().flex().flex_wrap().items_center().gap(px(8.));
        let play_label = if p.mods.is_empty() {
            "Play".to_string()
        } else {
            format!("Play with {} mods", p.mods.len())
        };
        let mut play = button("pack-play", play_label, Kind::Primary, &t);
        if p.mods.is_empty() || !can_play || self.pack_busy {
            play = disabled(play);
        } else {
            let work = p.work.clone();
            play = play.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_pack(work.clone());
                cx.notify();
            }));
        }
        let port_label = format!("Port {} mods", p.portable);
        let mut port = button("pack-port", port_label, Kind::Secondary, &t);
        let mut from = button("pack-from", "Use jars from my instance…", Kind::Ghost, &t);
        if self.pack_busy || p.portable == 0 {
            port = disabled(port);
            from = disabled(from);
        } else {
            port = port.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.port_pack(false, cx);
                cx.notify();
            }));
            from = from.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.port_pack(true, cx);
                cx.notify();
            }));
        }
        let confirming = self.confirm_remove.as_deref() == Some(p.slug.as_str());
        let slug = p.slug.clone();
        let work = p.work.clone();
        let remove = button(
            "pack-remove",
            if confirming {
                "Remove for good"
            } else {
                "Remove"
            },
            if confirming {
                Kind::Secondary
            } else {
                Kind::Ghost
            },
            &t,
        )
        .when(confirming, |b| b.text_color(t.danger))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if this.confirm_remove.as_ref() == Some(&slug) {
                match modpacks::remove(&work) {
                    Ok(()) => {
                        this.packs.retain(|p| p.slug != slug);
                        this.pack_open = this.packs.first().map(|p| p.slug.clone());
                        this.mods.report_list.reset();
                    }
                    Err(e) => this.pack_error = Some(e),
                }
                this.confirm_remove = None;
            } else {
                this.confirm_remove = Some(slug.clone());
            }
            cx.notify();
        }));
        buttons = buttons.child(play).child(port).child(from).child(remove);
        let mut notes = vec![format!(
            "Port gets the jars of the {} mods this computer can port today (content with data \
             and assets only) from the pack's own links, Modrinth, or your folder, and carries \
             them over. The ports stay here, in {}.",
            p.portable,
            short(&p.work)
        )];
        if let Some(jars) = p.jars() {
            notes.push(jars);
        }
        for line in &p.left_off {
            notes.push(format!("Left off: {line}."));
        }
        if !can_play {
            notes.push("Playing needs a prepared Horizon version (Play page).".into());
        }
        let mut rows: Vec<&horizon_port_pack::report::Row> = p
            .rows
            .iter()
            .filter(|r| self.pack_filter.is_none_or(|s| r.status == s))
            .collect();
        rows.sort_by(|a, b| (a.status, &a.key).cmp(&(b.status, &b.key)));
        let filter = segments("pack-filter", &FILTERS, self.pack_filter, &t, {
            let this = cx.entity().downgrade();
            move |value, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.pack_filter = value;
                    this.mods.report_list.reset();
                    cx.notify();
                });
            }
        });
        let row = |i: usize| {
            let r = rows[i];
            let color = match r.status {
                Status::Ported => t.success,
                Status::Partial => t.accent,
                Status::NotYet => t.ink_soft,
                Status::NotNeeded => t.muted,
            };
            let name = if r.name.is_empty() || r.name == r.key {
                r.key.clone()
            } else {
                format!("{}  ({})", r.name, r.key)
            };
            let detail = format!("{} · {} · {}: {}", r.class, r.tier, r.license, r.reason);
            index_row(
                i + 1,
                name,
                Some(detail.into_any_element()),
                Some(
                    caps(r.status.label())
                        .flex_none()
                        .text_color(color)
                        .into_any_element(),
                ),
                &t,
            )
        };
        let list = self.mods.report_list.column(
            &self.content_scroll.handle,
            rows.len(),
            8.,
            |_| false,
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            text(theme::TITLE)
                                .text_color(t.ink)
                                .child(p.title().to_string()),
                        )
                        .child(text(theme::UI).text_color(t.muted).child(p.summary())),
                )
                .child(counts)
                .child(buttons)
                .children(self.client_error.clone().map(|e| notice(e, &t)))
                .children(notes.into_iter().map(|n| {
                    text(theme::CAPTION)
                        .max_w(px(720.))
                        .text_color(t.muted)
                        .child(n)
                }))
                .child(div().flex().child(filter))
                .child(list),
        )
    }
}
