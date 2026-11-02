//! The Mods page's project page, search filters and project icons (`horizon_launcher::browser`:
//! `details` for the data, `images` for the icons and gallery). Drawn only with the launcher's
//! shared components, so a restyle of those restyles this.
//!
//! - A result's title opens its page inside the launcher: icon, author, downloads, followers,
//!   licence, how well it runs and Install, then Description (Markdown), Gallery, Versions and
//!   the project's links.
//! - Filters: sort, loader, game version, environment and categories, with the values the
//!   source lists for the kind.
//! - Icons and gallery images come from the source's image servers at run time, cached on disk;
//!   a placeholder stands in until one arrives.
use super::mods_page::{Checking, Event};
use super::*;
use gpui::img;
use horizon_launcher::browser::{
    self,
    details::{Details, Environment, Filters, Sort, Tags},
    images, java, Kind as PackKind, Project, SourceId, Support, Version,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Category and tag names from Modrinth / CurseForge, present in code so the i18n
/// tables can provide translations for them without failing the unused strings test.
const CATEGORY_NAMES: &[&str] = &[
    "adventure",
    "audio",
    "blocks",
    "combat",
    "core shaders",
    "cursed",
    "decoration",
    "economy",
    "entities",
    "environment",
    "equipment",
    "food",
    "fonts",
    "game mechanics",
    "gui",
    "items",
    "locale",
    "magic",
    "management",
    "minigame",
    "mobs",
    "modded",
    "models",
    "optimization",
    "realistic",
    "simplistic",
    "social",
    "storage",
    "technology",
    "themed",
    "transportation",
    "tweaks",
    "utility",
    "vanilla like",
    "worldgen",
];

/// A project page's tabs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Description,
    Gallery,
    Versions,
}

const TABS: [(Tab, &str); 3] = [
    (Tab::Description, "Description"),
    (Tab::Gallery, "Gallery"),
    (Tab::Versions, "Versions"),
];

/// The open project page.
pub(super) struct Opened {
    pub(super) project: Project,
    tab: Tab,
    details: Option<Result<Arc<Details>, String>>,
    versions: Option<Result<Arc<[ProjectVersion]>, String>>,
    version_list: cull::Cull,
    checked_versions: HashMap<String, horizon_port_pack::check::Verdict>,
}

/// Cached verdicts are read with the version request, never while drawing a frame.
#[derive(Clone)]
pub(super) struct ProjectVersion {
    version: Version,
    verdict: Option<horizon_port_pack::check::Verdict>,
}

/// The project page, filters and images' share of the Mods page's state.
#[derive(Default)]
pub(super) struct Browse {
    pub(super) open: Option<Opened>,
    pub(super) filters: Filters,
    pub(super) show_filters: bool,
    tags: HashMap<(SourceId, PackKind), Tags>,
    tags_asked: HashSet<(SourceId, PackKind)>,
    /// Images on disk, by URL and size; `None` while they load or when they failed.
    images: HashMap<(String, u32), Option<PathBuf>>,
    image_queue: Option<std::sync::mpsc::Sender<(String, u32)>>,
}

/// What the background sends this module.
pub(super) enum Loaded {
    Details((SourceId, String), Result<Details, String>),
    Versions((SourceId, String), Result<Vec<ProjectVersion>, String>),
    Checked(
        (SourceId, String),
        String,
        horizon_port_pack::check::Verdict,
    ),
    Image((String, u32), Option<PathBuf>),
    Tags((SourceId, PackKind), Result<Tags, String>),
}

/// Images fetched at once.
const IMAGE_WORKERS: usize = 4;

impl Launcher {
    pub(super) fn mods_loaded(&mut self, loaded: Loaded) {
        let b = &mut self.mods.browse;
        fn open<'a>(b: &'a mut Browse, key: &(SourceId, String)) -> Option<&'a mut Opened> {
            b.open
                .as_mut()
                .filter(|o| (o.project.source, &o.project.id) == (key.0, &key.1))
        }
        match loaded {
            Loaded::Details(key, d) => {
                if let Some(o) = open(b, &key) {
                    o.details = Some(d.map(Arc::new));
                }
            }
            Loaded::Versions(key, v) => {
                if let Some(o) = open(b, &key) {
                    o.versions = Some(v.map(Arc::from));
                    o.apply_checks();
                    o.version_list.reset();
                }
            }
            Loaded::Checked(key, version, verdict) => {
                if let Some(o) = open(b, &key) {
                    o.checked_versions.insert(version, verdict);
                    o.apply_checks();
                }
            }
            Loaded::Image(key, path) => {
                // A refresh that failed (offline) keeps the cached image.
                if path.is_some() || !matches!(b.images.get(&key), Some(Some(_))) {
                    b.images.insert(key, path);
                }
            }
            Loaded::Tags(key, Ok(tags)) => {
                b.tags.insert(key, tags);
            }
            Loaded::Tags(_, Err(_)) => {}
        }
    }

    /// Opens a project's page and asks its source for the rest of it.
    pub(super) fn mods_open(&mut self, project: Project) {
        let key = (project.source, project.id.clone());
        let tx = self.mods.tx.clone();
        let p = project.clone();
        std::thread::spawn(move || {
            let Some(source) = browser::source(p.source) else {
                return;
            };
            let d = source.details(&p);
            let _ = tx.send(Event::Loaded(Loaded::Details(key.clone(), d)));
            let v = source.versions(&p).map(|versions| {
                let home = backend::game_home();
                versions
                    .into_iter()
                    .map(|version| {
                        let verdict = version.file().and_then(|file| {
                            (p.kind == PackKind::Mod)
                                .then(|| java::cached(&home, file))
                                .flatten()
                                .map(|check| check.verdict)
                        });
                        ProjectVersion { version, verdict }
                    })
                    .collect()
            });
            let _ = tx.send(Event::Loaded(Loaded::Versions(key, v)));
        });
        self.mods.browse.open = Some(Opened {
            project,
            tab: Tab::Description,
            details: None,
            versions: None,
            version_list: cull::Cull::default(),
            checked_versions: HashMap::new(),
        });
    }

    /// The image of `url` at `size` when it is on disk; else asks for it once. One cached longer
    /// than `images::FRESH` is shown and asked for again, with its validators.
    pub(super) fn mods_image(&mut self, url: &str, size: u32) -> Option<PathBuf> {
        let key = (url.to_owned(), size);
        if let Some(state) = self.mods.browse.images.get(&key) {
            return state.clone();
        }
        // The launcher's data folder, shared by every instance.
        let home = backend::data_dir();
        if let Some(p) = images::cached(&home, url, size) {
            self.mods.browse.images.insert(key.clone(), Some(p.clone()));
            if images::stale(&home, url, size) {
                let _ = self.mods_image_queue().send(key);
            }
            return Some(p);
        }
        self.mods.browse.images.insert(key.clone(), None);
        if images::refuse(url).is_some() {
            return None;
        }
        let _ = self.mods_image_queue().send(key);
        None
    }

    /// The image workers' queue, started the first time an image is asked for.
    fn mods_image_queue(&mut self) -> std::sync::mpsc::Sender<(String, u32)> {
        let tx = self.mods.tx.clone();
        self.mods
            .browse
            .image_queue
            .get_or_insert_with(|| {
                let (send, receive) = std::sync::mpsc::channel::<(String, u32)>();
                let receive = std::sync::Arc::new(std::sync::Mutex::new(receive));
                let home = backend::data_dir();
                for _ in 0..IMAGE_WORKERS {
                    let (receive, tx, home) = (receive.clone(), tx.clone(), home.clone());
                    std::thread::spawn(move || loop {
                        let Ok(next) = receive.lock().map(|r| r.recv()) else {
                            break;
                        };
                        let Ok((url, size)) = next else {
                            break;
                        };
                        let path = images::fetch(&home, &url, size).ok();
                        if tx
                            .send(Event::Loaded(Loaded::Image((url, size), path)))
                            .is_err()
                        {
                            break;
                        }
                    });
                }
                send
            })
            .clone()
    }

    /// A project's icon, `side` pixels: its image, or its first letter on a plate until then.
    pub(super) fn mods_icon(&mut self, p: &Project, side: f32) -> AnyElement {
        let t = self.theme;
        let path = p
            .icon
            .clone()
            .and_then(|url| self.mods_image(&url, images::ICON));
        blocks::slot(path.map(gpui::ImageSource::from), &p.title, side, &t).into_any_element()
    }

    /// The values the filters can take for the current kind and source, asked for once.
    fn mods_tags(&mut self) -> Option<&Tags> {
        let key = (self.mods.source, self.mods.kind);
        if self.mods.browse.tags_asked.insert(key) {
            let tx = self.mods.tx.clone();
            std::thread::spawn(move || {
                let tags = browser::source(key.0)
                    .ok_or_else(|| "not set up".to_string())
                    .and_then(|s| s.tags(key.1));
                let _ = tx.send(Event::Loaded(Loaded::Tags(key, tags)));
            });
        }
        self.mods.browse.tags.get(&key)
    }

    /// Sets the filters and searches again.
    fn mods_filter(&mut self, change: impl FnOnce(&mut Filters)) {
        change(&mut self.mods.browse.filters);
        self.mods_search(0);
    }

    /// The filter bar: sort always; loader, version, environment and categories when shown.
    pub(super) fn mods_filters(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let this = cx.entity().downgrade();
        let f = self.mods.browse.filters.clone();
        let sorts: Vec<(Option<Sort>, &'static str)> = std::iter::once((None, "Best match"))
            .chain(Sort::ALL.iter().map(|s| (Some(*s), s.label())))
            .collect();
        let sort = segments("mods-sort", &sorts, f.sort, &t, {
            let this = this.clone();
            move |value, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.mods_filter(|f| f.sort = value);
                    cx.notify();
                });
            }
        });
        let shown = self.mods.browse.show_filters;
        let active = f.categories.len()
            + f.loader.is_some() as usize
            + f.game_version.is_some() as usize
            + f.environment.is_some() as usize;
        let toggle = button(
            "mods-filters",
            if active > 0 {
                trf("Filters ({count})", &[("count", &active.to_string())])
            } else {
                tr("Filters").to_string()
            },
            if shown { Kind::Secondary } else { Kind::Ghost },
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.mods.browse.show_filters = !this.mods.browse.show_filters;
            cx.notify();
        }));
        let mut bar = div().flex().flex_col().gap(px(12.)).child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(12.))
                .child(sort)
                .child(toggle),
        );
        if !shown {
            return bar;
        }
        let Some(tags) = self.mods_tags().cloned() else {
            return bar.child(caps(tr("Loading filters")).text_color(t.muted));
        };
        // A chip: on or off, one value of a filter.
        let chip = |id: String, label: String, on: bool| {
            button(
                id,
                label,
                if on { Kind::Secondary } else { Kind::Ghost },
                &t,
            )
        };
        let row = |title: &'static str| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(4.))
                .child(caps(tr(title)).text_color(t.muted).w(px(120.)))
        };
        let mut loaders = row("Loader");
        for l in &tags.loaders {
            let on = f.loader.as_deref() == Some(l.as_str());
            let value = l.clone();
            loaders = loaders.child(chip(format!("mods-loader-{l}"), l.clone(), on).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let v = value.clone();
                    this.mods_filter(|f| f.loader = if on { None } else { Some(v) });
                    cx.notify();
                }),
            ));
        }
        let mut versions = row("Game version");
        if let Some(path) = self.vanilla_icon() {
            versions = versions.child(img(path).size(px(20.)).flex_none());
        }
        for v in tags.game_versions.iter().take(12) {
            let on = f.game_version.as_deref() == Some(v.as_str());
            let value = v.clone();
            versions = versions.child(chip(format!("mods-version-{v}"), v.clone(), on).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let v = value.clone();
                    this.mods_filter(|f| f.game_version = if on { None } else { Some(v) });
                    cx.notify();
                }),
            ));
        }
        let mut environment = row("Environment");
        for e in Environment::ALL {
            let on = f.environment == Some(e);
            environment = environment.child(
                chip(format!("mods-env-{}", e.label()), e.label().into(), on).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_filter(|f| f.environment = if on { None } else { Some(e) });
                        cx.notify();
                    }),
                ),
            );
        }
        let mut categories = row("Categories");
        for c in &tags.categories {
            let on = f.categories.contains(&c.name);
            let value = c.name.clone();
            categories = categories.child(
                chip(format!("mods-cat-{}", c.name), c.name.replace('-', " "), on).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let v = value.clone();
                        this.mods_filter(|f| {
                            if !f.categories.contains(&v) {
                                f.categories.push(v);
                            } else {
                                f.categories.retain(|x| *x != v);
                            }
                        });
                        cx.notify();
                    }),
                ),
            );
        }
        let _ = CATEGORY_NAMES;
        let clear = button("mods-clear", "Clear filters", Kind::Ghost, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.mods_filter(|f| *f = Filters::default());
                cx.notify();
            },
        ));
        bar = bar
            .child(loaders)
            .child(versions)
            .child(environment)
            .child(categories);
        if active > 0 {
            bar = bar.child(div().flex().child(clear));
        }
        bar
    }

    /// Vanilla's mark, drawn from the player's prepared game files when there are some.
    pub(super) fn vanilla_icon(&self) -> Option<PathBuf> {
        let tag = self.selected().filter(|tag| backend::is_prepared(tag))?;
        images::grass_block(&backend::game_home(), &backend::resources(tag))
    }

    /// The open project's page.
    pub(super) fn mods_project_page(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let o = self.mods.browse.open.as_ref()?;
        let p = o.project.clone();
        let tab = o.tab;
        let support = self.mods.support(&p);
        let color = match support {
            Support::Works => t.success,
            Support::Partial(_) => t.warn,
            Support::No(_) => t.muted,
        };
        let back = button("mods-back", "Back to results", Kind::Ghost, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.mods.browse.open = None;
                cx.notify();
            },
        ));
        let details = o.details.clone();
        let versions = o.versions.clone();
        let version_list = o.version_list.clone();
        let mut meta = vec![trf(
            "{count} downloads",
            &[("count", &thousands(p.downloads))],
        )];
        if let Some(Ok(d)) = &details {
            meta.push(trf(
                "{count} followers",
                &[("count", &thousands(d.followers))],
            ));
            if let Some(day) = d.updated.get(..10) {
                meta.push(trf("updated {date}", &[("date", day)]));
            }
        }
        if !p.license.is_empty() {
            meta.push(p.license.clone());
        }
        meta.push(trf("from {source}", &[("source", p.source.label())]));
        let authors = match &details {
            Some(Ok(d)) if !d.authors.is_empty() => d.authors.join(", "),
            _ => p.author.clone(),
        };
        let icon = self.mods_icon(&p, 72.);
        let busy = self.mods.busy.as_ref().map(|b| b.0.clone());
        let installed = self.mods.entry(&p).is_some();
        let checking = self.mods.checking(&p);
        let mut action = div().flex().items_center().gap(px(8.)).child(
            caps(if checking {
                tr("Checking")
            } else {
                tr(support.label())
            })
            .flex_none()
            .text_color(color),
        );
        if busy.as_deref() == Some(p.id.as_str()) {
            action = action.child(caps(tr("Installing")).text_color(t.accent));
        } else if installed {
            action = action.child(caps(tr("Installed")).text_color(t.muted));
        } else if support.installable() && !checking {
            let label = if p.kind == PackKind::Modpack {
                "Import"
            } else {
                "Install"
            };
            let mut install = button("mods-page-install", label, Kind::Primary, &t);
            if busy.is_some() {
                install = disabled(install);
            } else {
                let p = p.clone();
                install = install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_install(p.source, p.id.clone(), Some(p.clone()));
                    cx.notify();
                }));
            }
            action = action.child(install);
        }
        let head = div()
            .flex()
            .items_center()
            .gap(px(20.))
            .child(icon)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(text(theme::TITLE).text_color(t.ink).child(p.title.clone()))
                    .child(
                        text(theme::UI)
                            .text_color(t.muted)
                            .child(if authors.is_empty() {
                                p.summary.clone()
                            } else {
                                format!(
                                    "{} · {}",
                                    trf("by {author}", &[("author", &authors)]),
                                    p.summary
                                )
                            }),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(meta.join(" · ")),
                    ),
            )
            .child(action);
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(div().flex().child(back))
            .child(head);
        // Registry projects are rated on the site, signed in with GitHub (one rating per account).
        if p.source == SourceId::Horizon {
            let url = browser::horizon::rate_url(&p.id);
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .children(super::mods_page::stars(&p.id, &t).map(|s| text(theme::UI).child(s)))
                    .child(
                        button("mods-rate", "Sign in with GitHub to rate", Kind::Ghost, &t)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            );
        }
        if let Some(why) = support.reason() {
            page = page.child(
                text(theme::UI)
                    .text_color(t.ink_soft)
                    .child(why.to_string()),
            );
        }
        if let Some(Checking::Done(c)) = self.mods.checks.get(&(p.source, p.id.clone())) {
            let lines = |title: &str, list: &[String]| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(caps(tr(title)).text_color(t.muted))
                    .children(
                        list.iter()
                            .map(|l| text(theme::CAPTION).text_color(t.ink_soft).child(l.clone())),
                    )
            };
            let mut what = div().flex().gap(px(32.));
            if !c.ports.is_empty() {
                what = what.child(lines("Ports", &c.ports));
            }
            if !c.skipped.is_empty() {
                what = what.child(lines("Doesn't port", &c.skipped));
            }
            page = page.child(what);
        }
        if let Some(Ok(d)) = &details {
            let mut links = div().flex().flex_wrap().gap(px(8.));
            let page_url = p.page.clone();
            if !page_url.is_empty() {
                links = links.child(
                    button(
                        "mods-link-page",
                        trf("{source} page", &[("source", p.source.label())]),
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| cx.open_url(&page_url)),
                );
            }
            for (i, l) in d.links.iter().enumerate() {
                let url = l.url.clone();
                links = links.child(
                    button(format!("mods-link-{i}"), l.label.clone(), Kind::Ghost, &t)
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            page = page.child(links);
        }
        let this = cx.entity().downgrade();
        page = page.child(div().flex().child(segments("mods-tab", &TABS, tab, &t, {
            move |value, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if let Some(o) = &mut this.mods.browse.open {
                        o.tab = value;
                    }
                    cx.notify();
                });
            }
        })));
        let body = match tab {
            Tab::Description => match &details {
                None => centered(tr("Loading…"), &t),
                Some(Err(e)) => notice(e.clone(), &t),
                Some(Ok(d)) => div().max_w(px(760.)).child(markdown::render(&d.body, &t)),
            },
            Tab::Gallery => match &details {
                None => centered(tr("Loading…"), &t),
                Some(Err(e)) => notice(e.clone(), &t),
                Some(Ok(d)) if d.gallery.is_empty() => centered(tr("No gallery images."), &t),
                Some(Ok(d)) => {
                    let mut grid = div().flex().flex_wrap().gap(px(16.));
                    for g in &d.gallery {
                        let shot = match self.mods_image(&g.url, images::GALLERY) {
                            Some(path) => img(path).w(px(360.)).h(px(203.)).into_any_element(),
                            None => plate(&t).w(px(360.)).h(px(203.)).into_any_element(),
                        };
                        grid = grid.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .w(px(360.))
                                .child(plate(&t).overflow_hidden().child(shot))
                                .when(!g.title.is_empty(), |d| {
                                    d.child(
                                        text(theme::CAPTION)
                                            .text_color(t.ink_soft)
                                            .child(g.title.clone()),
                                    )
                                }),
                        );
                    }
                    grid
                }
            },
            Tab::Versions => match &versions {
                None => centered(tr("Loading…"), &t),
                Some(Err(e)) => notice(e.clone(), &t),
                Some(Ok(list)) => version_list.column(
                    &self.content_scroll.handle,
                    list.len().min(50),
                    8.,
                    |_| false,
                    |i| {
                        let v = &list[i].version;
                        let file = v.file();
                        let mut detail = vec![
                            v.published.get(..10).unwrap_or_default().to_string(),
                            v.loaders.join(", "),
                        ];
                        let games: Vec<&str> = v
                            .game_versions
                            .iter()
                            .rev()
                            .take(4)
                            .map(String::as_str)
                            .collect();
                        detail.push(games.join(", "));
                        if let Some(f) = file {
                            detail.push(format!("{} · {:.1} MB", f.name, f.size as f64 / 1e6));
                        }
                        index_row(
                            i + 1,
                            v.number.clone(),
                            Some(detail.join(" · ").into_any_element()),
                            list[i].verdict.as_ref().map(|verdict| {
                                caps(tr(verdict.label()))
                                    .text_color(t.muted)
                                    .into_any_element()
                            }),
                            &t,
                        )
                    },
                ),
            },
        };
        Some(page.child(body))
    }
}

impl Opened {
    /// A check can finish before or after the version request; retain either event order.
    fn apply_checks(&mut self) {
        let Some(Ok(versions)) = &mut self.versions else {
            return;
        };
        for row in Arc::make_mut(versions) {
            if let Some(verdict) = self.checked_versions.get(&row.version.id) {
                row.verdict = Some(*verdict);
            }
        }
    }
}
