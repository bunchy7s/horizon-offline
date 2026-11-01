//! The Mods page: search Horizon's own registry for mods made for Horizon, or Modrinth
//! (CurseForge: coming soon) for Java mods, modpacks and resource packs, see where each comes
//! from and how well it runs (a Java mod's jar is checked first; a Horizon mod says whether it is
//! signed and what it may do), install it with its checksum checked, turn Horizon mods on and
//! off, and update or remove what was installed (`horizon_launcher::browser`). Imported modpacks
//! are listed on its Modpacks tab.
use super::line_edit::{self, Key, LineEdit};
use super::*;
use horizon_launcher::browser::{
    self, horizon, installed, java, java_instance, Kind as PackKind, Project, Query, SourceId,
    Support,
};
use horizon_launcher::modpacks;
use horizon_port_pack::check::Check;
use java_instance::Target;
use std::collections::HashMap;
use std::path::PathBuf;

/// The kinds, as the Mods page's tabs: name and icon.
const KINDS: [(PackKind, &str, &str); 5] = [
    (PackKind::Mod, "Mods", "box"),
    (PackKind::Modpack, "Modpacks", "archive"),
    (PackKind::ResourcePack, "Resource packs", "image"),
    (PackKind::Shader, "Shader packs", "zap"),
    (PackKind::DataPack, "Data packs", "notes"),
];
/// Java mods checked at once.
const CHECKING: usize = 2;
/// The sources to pick from; CurseForge is shown beside them as coming soon.
const SOURCES: [(SourceId, &str); 2] = [
    (SourceId::Modrinth, "Modrinth"),
    (SourceId::Horizon, "Horizon"),
];
const PAGE: usize = 20;

pub(super) enum Event {
    /// A search's answer, for the search it was (stale answers are dropped) at that offset.
    Found(u64, usize, Result<Vec<Project>, String>),
    Progress(String, f32),
    Done(Result<String, String>, bool),
    Updates(HashMap<(SourceId, String), String>),
    /// A Java mod's check: its jar read, or why it couldn't be.
    Checked((SourceId, String), Result<Box<Check>, String>),
    /// What a Horizon registry project says before install: signed or not, what it may do.
    About(HashMap<String, Vec<String>>),
    /// Registry files taken down that the launcher turned off.
    TakenDown(Vec<String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
}

/// Where a Java mod's check is.
pub(super) enum Checking {
    Running,
    Done(Box<Check>),
    Failed(String),
}

pub(super) struct Mods {
    pub(super) kind: PackKind,
    pub(super) source: SourceId,
    text: LineEdit,
    focus: FocusHandle,
    pub(super) focused: bool,
    everything: bool,
    /// Which search is the current one; answers for older ones are dropped.
    search: u64,
    searching: bool,
    searched: bool,
    results: Vec<Project>,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    more: bool,
    error: Option<String>,
    installed: Vec<installed::Entry>,
    loaded: bool,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: Option<PathBuf>,
    /// The selected Java instance's version and loader: its mods and resource packs are the
    /// source's own files for them, put in as they are.
    pub(super) java: Option<Target>,
    /// Its worlds (folder, name), the last played first, and the one data packs go into.
    worlds: Vec<(PathBuf, String)>,
    world: Option<PathBuf>,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    message: Option<Result<String, String>>,
    updates: HashMap<(SourceId, String), String>,
    confirm_remove: Option<String>,
    /// A native mod's Install waiting for the one-time notice (`native_mods::needs_notice`).
    native_notice: Option<(SourceId, String, Option<Project>)>,
    /// Java mods' checks, by source and project.
    pub(super) checks: HashMap<(SourceId, String), Checking>,
    /// Horizon registry projects' trust and capabilities, by id ([`horizon::trust_and_capabilities`]).
    pub(super) about: HashMap<String, Vec<String>>,
    /// The project page, filters and images.
    pub(super) browse: super::mods_project::Browse,
    pub(super) tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Mods {
    pub(super) fn reset_lists(&self) {
        self.results_list.reset();
        self.installed_list.reset();
        self.report_list.reset();
    }

    /// The game folder installs go into: the selected instance's, once the page has read it.
    pub(super) fn home(&self) -> PathBuf {
        self.home.clone().unwrap_or_else(backend::game_home)
    }

    /// Reads the instance's worlds again; the world picked for data packs stays when it is
    /// still there, else the last played.
    fn read_worlds(&mut self) {
        self.worlds = horizon_launcher::worlds::list(&self.home())
            .into_iter()
            .map(|w| (w.path, w.name))
            .collect();
        if !self
            .worlds
            .iter()
            .any(|(p, _)| Some(p) == self.world.as_ref())
        {
            self.world = self.worlds.first().map(|(p, _)| p.clone());
        }
    }

    /// Whether the Modpacks tab is open (the sidebar marks the pack shown there).
    pub(super) fn on_modpacks(&self) -> bool {
        self.kind == PackKind::Modpack
    }

    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = channel();
        let page = std::env::var("HORIZON_LAUNCHER_PAGE");
        let kind = match page.as_deref() {
            Ok("modpacks") => PackKind::Modpack,
            Ok("shaders") => PackKind::Shader,
            Ok("datapacks") => PackKind::DataPack,
            _ => PackKind::Mod,
        };
        Self {
            kind,
            // `HORIZON_LAUNCHER_SOURCE=horizon` opens on Horizon's registry, for screenshots;
            // shader packs are only there.
            source: match std::env::var("HORIZON_LAUNCHER_SOURCE").as_deref() {
                _ if kind == PackKind::Shader => SourceId::Horizon,
                Ok("horizon") => SourceId::Horizon,
                _ => SourceId::Modrinth,
            },
            text: LineEdit::new(String::new(), 100, 40),
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            everything: false,
            search: 0,
            searching: false,
            searched: false,
            results: Vec::new(),
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            more: false,
            error: None,
            installed: Vec::new(),
            loaded: false,
            home: None,
            java: None,
            worlds: Vec::new(),
            world: None,
            busy: None,
            message: None,
            native_notice: None,
            updates: HashMap::new(),
            confirm_remove: None,
            checks: HashMap::new(),
            about: HashMap::new(),
            browse: Default::default(),
            tx,
            rx,
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.is_focused(window);
    }

    pub(super) fn entry(&self, p: &Project) -> Option<&installed::Entry> {
        self.installed
            .iter()
            .find(|e| e.source == p.source && e.project == p.id)
    }

    /// The Java instance `kind` goes into as files, when one is selected.
    pub(super) fn java_for(&self, kind: PackKind) -> Option<&Target> {
        self.java.as_ref().filter(|t| t.takes(kind))
    }

    /// Whether a Java mod's check is still to come before it can be installed.
    pub(super) fn checking(&self, p: &Project) -> bool {
        self.java_for(p.kind).is_none()
            && p.kind == PackKind::Mod
            && p.source != SourceId::Horizon
            && matches!(
                self.checks.get(&(p.source, p.id.clone())),
                None | Some(Checking::Running)
            )
            && p.support().installable()
    }

    /// How well a result runs: a Java mod's from its check once it has one. In a Java
    /// instance, a result was searched for its version and loader: installing checks the file.
    pub(super) fn support(&self, p: &Project) -> Support {
        if self.java_for(p.kind).is_some() {
            return Support::Works;
        }
        let metadata = p.support();
        if p.kind != PackKind::Mod || p.source == SourceId::Horizon || !metadata.installable() {
            return metadata;
        }
        match self.checks.get(&(p.source, p.id.clone())) {
            Some(Checking::Done(c)) => java::support_of(c),
            Some(Checking::Failed(e)) => Support::Partial(format!("Not checked: {e}")),
            _ => Support::Partial("Checking its jar…".into()),
        }
    }
}

/// `51404735` → `51.4M`.
fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A registry project's rating: five pixel stars, the mean's nearest whole number of them lit, the
/// mean and how many; None while the ratings aren't known.
pub(super) fn stars(id: &str, t: &theme::Theme) -> Option<Div> {
    let row = div().flex().items_center().gap(px(2.));
    Some(match horizon::rating(id)? {
        None => row.text_color(t.muted).child("No ratings yet"),
        Some(r) => {
            let lit = r.average.round() as usize;
            row.children((0..5).map(|i| {
                blocks::icon(
                    "star",
                    12.,
                    if i < lit {
                        t.warn
                    } else {
                        t.muted.opacity(0.6)
                    },
                )
            }))
            .child(div().ml(px(4.)).child(format!(
                "{:.1} ({})",
                r.average,
                count(r.count)
            )))
        }
    })
}

/// A checksum as the page shows it: the algorithm and the first and last hex digits.
fn short_sum(checksum: &str) -> String {
    let (algo, hex) = checksum.split_once(':').unwrap_or(("", checksum));
    if hex.len() <= 20 {
        return checksum.to_string();
    }
    format!("{algo} {}…{}", &hex[..12], &hex[hex.len() - 8..])
}

impl Launcher {
    /// Opens the Mods page on its Modpacks tab, with the pack `slug` open when given (the
    /// sidebar's packs and its + button).
    pub(super) fn show_modpack(&mut self, slug: Option<String>) {
        self.go(Page::Mods);
        if slug.is_some() {
            self.pack_open = slug;
            self.mods.report_list.reset();
        }
        if self.mods.kind != PackKind::Modpack {
            self.mods.kind = PackKind::Modpack;
            self.mods.browse.filters = Default::default();
            self.mods.browse.open = None;
            if self.mods.loaded {
                self.mods_search(0);
            }
        }
    }

    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(&mut self) -> bool {
        let mut changed = false;
        let mut to_check = Vec::new();
        let mut loaded = Vec::new();
        let mut done = false;
        let mut native_on = false;
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Found(search, offset, result) if search == m.search => {
                    m.searching = false;
                    m.searched = true;
                    match result {
                        Ok(found) => {
                            m.more = found.len() == PAGE;
                            m.results.truncate(offset);
                            m.results.extend(found.iter().cloned());
                            m.error = None;
                            to_check.extend(found);
                        }
                        Err(e) => m.error = Some(e),
                    }
                }
                Event::Found(..) => {}
                Event::Progress(line, f) => {
                    if let Some(busy) = &mut m.busy {
                        busy.1 = line;
                        busy.2 = f;
                    }
                }
                Event::Done(result, modpack) => {
                    done = true;
                    m.busy = None;
                    m.message = Some(result);
                    m.installed = installed::Installed::load(&m.home()).entries;
                    native_on = native_mods::turns_on(native_on, &m.installed);
                    m.installed_list.reset();
                    if modpack {
                        // The Modpacks page reads its list again.
                        self.packs_loaded = None;
                    }
                }
                Event::Updates(found) => m.updates = found,
                Event::Checked(key, result) => {
                    let state = match result {
                        Ok(c) => Checking::Done(c),
                        Err(e) => Checking::Failed(e),
                    };
                    m.checks.insert(key, state);
                }
                Event::Loaded(l) => loaded.push(l),
                Event::About(about) => {
                    // Rows grow by a line: measure them again (`cull`).
                    m.about.extend(about);
                    m.results_list.reset();
                }
                Event::TakenDown(lines) => {
                    m.message = Some(Ok(lines.join("\n")));
                    m.installed = installed::Installed::load(&m.home()).entries;
                    m.installed_list.reset();
                }
            }
        }
        // A native mod installed: Native mods turn on, so Settings offers its Build.
        if native_on && !self.settings.native_mods {
            self.settings.native_mods = true;
            self.settings.native_mods_noticed = true;
            self.settings.save();
        }
        // `HORIZON_LAUNCHER_PROJECT=<slug>` (with `HORIZON_LAUNCHER_PAGE=mods`): opens that
        // project's page once a search finds it, for screenshots.
        if self.mods.browse.open.is_none() {
            if let Ok(want) = std::env::var("HORIZON_LAUNCHER_PROJECT") {
                if let Some(p) = self
                    .mods
                    .results
                    .iter()
                    .find(|p| p.id == want || p.slug == want)
                    .cloned()
                {
                    std::env::remove_var("HORIZON_LAUNCHER_PROJECT");
                    self.mods_open(p);
                }
            }
        }
        self.mods_check(to_check);
        if done {
            // A data pack put into a world shows in its list on Home.
            self.world_packs.reread();
        }
        for l in loaded {
            self.mods_loaded(l);
        }
        changed
    }

    pub(super) fn mods_search(&mut self, offset: usize) {
        let m = &mut self.mods;
        m.search += 1;
        m.searching = true;
        if offset == 0 {
            m.results.clear();
            m.more = false;
            m.results_list.reset();
            m.installed_list.reset();
        }
        let query = match m.java_for(m.kind) {
            Some(target) => Query {
                offset,
                limit: PAGE,
                ..target.search(m.text.text(), m.kind, &m.browse.filters)
            },
            None => Query {
                everything: m.everything,
                offset,
                limit: PAGE,
                filters: m.browse.filters.clone(),
                ..Query::new(m.text.text(), m.kind)
            },
        };
        // A Java instance's files are Modrinth's; Horizon's registry is for Horizon.
        let source = match m.java_for(m.kind) {
            Some(_) => SourceId::Modrinth,
            None => m.source,
        };
        let (tx, search) = (m.tx.clone(), m.search);
        std::thread::spawn(move || {
            let result = browser::source(source)
                .ok_or_else(|| format!("{} isn't set up.", source.label()))
                .and_then(|s| s.search(&query));
            let about: HashMap<String, Vec<String>> = match (&result, source) {
                (Ok(found), SourceId::Horizon) => found
                    .iter()
                    .map(|p| (p.id.clone(), horizon::trust_and_capabilities(&p.id)))
                    .collect(),
                _ => HashMap::new(),
            };
            let _ = tx.send(Event::Found(search, offset, result));
            if !about.is_empty() {
                let _ = tx.send(Event::About(about));
            }
        });
    }

    /// Reads the registry's index again when something installed came from it, and turns off
    /// what it took down since (`horizon::turn_off_revoked`).
    fn mods_taken_down(&mut self) {
        if !self
            .mods
            .installed
            .iter()
            .any(|e| e.source == SourceId::Horizon)
        {
            return;
        }
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let Ok(index) = horizon::index() else {
                return;
            };
            let lines = horizon::turn_off_revoked(&index, &home);
            if !lines.is_empty() {
                let _ = tx.send(Event::TakenDown(lines));
            }
        });
    }

    /// Turns a Horizon mod the browser installed on or off, as the game's Options > Mods does;
    /// in a Java instance, renames its file.
    fn mods_switch(&mut self, e: &installed::Entry) {
        let home = self.mods.home();
        if self.mods.java.is_some() {
            let on = !java_instance::is_on(&home, e);
            self.mods.message = Some(java_instance::set_on(&home, e, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }));
            return;
        }
        let Some(id) = horizon::mod_id(e) else {
            return;
        };
        let on = !java::is_enabled(&home, id);
        let revoked = horizon::known_index().and_then(|index| {
            let hash = e.checksum.trim_start_matches("SHA-256:");
            index.revoked(&e.project, hash).map(|r| r.reason.clone())
        });
        self.mods.message = Some(match revoked.filter(|_| on) {
            Some(reason) => Err(format!("{} was taken down: {reason}", e.title)),
            None => java::set_enabled(&home, id, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }),
        });
    }

    /// Checks the Java mods among `found` that aren't checked yet, a few at a time, on threads:
    /// a cached check answers at once, else the jar is downloaded and read.
    fn mods_check(&mut self, found: Vec<Project>) {
        let m = &mut self.mods;
        if m.java.is_some() {
            return;
        }
        let todo: Vec<Project> = found
            .into_iter()
            .filter(|p| p.kind == PackKind::Mod && p.source != SourceId::Horizon)
            .filter(|p| p.support().installable())
            .filter(|p| !m.checks.contains_key(&(p.source, p.id.clone())))
            .collect();
        if todo.is_empty() {
            return;
        }
        for p in &todo {
            m.checks.insert((p.source, p.id.clone()), Checking::Running);
        }
        // A version other than 26.3 asked for: check the newest file for 1.21 and later.
        let everything = m.everything || m.browse.filters.other_version();
        let tx = m.tx.clone();
        let queue = std::sync::Mutex::new(todo.into_iter());
        std::thread::spawn(move || {
            // Checks are kept in the default game folder, shared by every instance.
            let home = backend::game_home();
            std::thread::scope(|s| {
                for _ in 0..CHECKING {
                    s.spawn(|| {
                        while let Some(p) = queue.lock().ok().and_then(|mut q| q.next()) {
                            let result = browser::source(p.source)
                                .ok_or_else(|| format!("{} isn't set up.", p.source.label()))
                                .and_then(|source| {
                                    java::check_project(&home, &*source, &p, everything, &|_| {})
                                });
                            // The project's page, if open, shows the verdict beside its version.
                            if let Ok((version, check)) = &result {
                                let key = (p.source, p.id.clone());
                                let _ =
                                    tx.send(Event::Loaded(super::mods_project::Loaded::Checked(
                                        key,
                                        version.id.clone(),
                                        check.verdict,
                                    )));
                            }
                            let result = result.map(|(_, check)| Box::new(check));
                            let _ = tx.send(Event::Checked((p.source, p.id), result));
                        }
                    });
                }
            });
        });
    }

    /// Looks for newer versions of what the browser installed.
    fn mods_check_updates(&mut self) {
        let entries = self.mods.installed.clone();
        if entries.is_empty() {
            return;
        }
        let tx = self.mods.tx.clone();
        let java = self.mods.java.clone();
        std::thread::spawn(move || {
            let mut found = HashMap::new();
            for e in entries {
                let Some(source) = browser::source(e.source) else {
                    continue;
                };
                let newer = match &java {
                    Some(target) => java_instance::update_for(&*source, target, &e),
                    None => installed::update_for(&*source, &e),
                };
                if let Ok(Some(v)) = newer {
                    found.insert((e.source, e.project.clone()), v.number);
                }
            }
            let _ = tx.send(Event::Updates(found));
        });
    }

    /// Updates every installed pack a newer version was found for, one after the other on a
    /// thread, each checked as an install is. One failing doesn't stop the others.
    fn mods_update_all(&mut self) {
        if self.mods.busy.is_some() || self.mods.updates.is_empty() {
            return;
        }
        let mut todo: Vec<(SourceId, String, String)> = self
            .mods
            .installed
            .iter()
            .filter(|e| {
                self.mods
                    .updates
                    .contains_key(&(e.source, e.project.clone()))
            })
            .map(|e| (e.source, e.project.clone(), e.title.clone()))
            .collect();
        todo.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        self.mods.message = None;
        self.mods.busy = Some(("all".into(), "Updating…".into(), 0.));
        self.mods.updates.clear();
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let java = self.mods.java.clone();
        std::thread::spawn(move || {
            let count = todo.len();
            let (mut done, mut failed) = (Vec::new(), Vec::new());
            for (i, (source_id, id, title)) in todo.into_iter().enumerate() {
                let step = |line: &str, f: f32| {
                    let overall = (i as f32 + f) / count as f32;
                    let _ = tx.send(Event::Progress(
                        format!("{} of {count}: {line}", i + 1),
                        overall,
                    ));
                };
                let updated = browser::source(source_id)
                    .ok_or_else(|| format!("{} isn't set up", source_id.label()))
                    .and_then(|source| {
                        let p = source.project(&id)?;
                        match &java {
                            Some(target) => {
                                java_instance::install(&*source, target, &p, &home, &step)
                            }
                            None => installed::install(&*source, &p, true, &home, &step),
                        }
                    });
                match updated {
                    Ok(entries) => done.extend(
                        entries
                            .into_iter()
                            .filter(|e| e.project == id)
                            .map(|e| format!("{} {}", e.title, e.number)),
                    ),
                    Err(e) => failed.push(format!("{title}: {e}")),
                }
            }
            let updated = match done.is_empty() {
                true => String::new(),
                false => format!("Updated {}, checksums checked.", done.join(", ")),
            };
            let result = match failed.is_empty() {
                true => Ok(updated),
                false => Err(format!("{updated} Not updated: {}", failed.join("; "))
                    .trim()
                    .to_string()),
            };
            let _ = tx.send(Event::Done(result, false));
        });
    }

    /// Installs (or updates) a project on a thread: a pack into its folder, a modpack into
    /// Modpacks. `known` is the project when the page has it; else it is asked for first.
    pub(super) fn mods_install(&mut self, source: SourceId, id: String, known: Option<Project>) {
        if self.mods.busy.is_some() {
            return;
        }
        // The first native mod: what it starts, before anything is installed.
        let first_native = source == SourceId::Horizon
            && native_mods::needs_notice(
                self.settings.native_mods,
                self.settings.native_mods_noticed,
            )
            && horizon::is_native_mod(&id);
        if first_native {
            self.mods.native_notice = Some((source, id, known));
            return;
        }
        self.mods.native_notice = None;
        self.mods.message = None;
        self.mods.busy = Some((id.clone(), format!("Asking {}…", source.label()), 0.));
        self.mods.updates.remove(&(source, id.clone()));
        let everything = self.mods.everything || self.mods.browse.filters.other_version();
        // A modpack is ported for the selected version as soon as it is imported.
        let resources = self
            .selected()
            .filter(|tag| backend::is_prepared(tag))
            .map(|tag| backend::resources(tag));
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let java = self.mods.java.clone();
        let world = self.mods.world.clone().and_then(|path| {
            let (_, name) = self.mods.worlds.iter().find(|(p, _)| *p == path)?;
            Some((path, name.clone()))
        });
        std::thread::spawn(move || {
            let last = std::cell::Cell::new(-1);
            let progress = |line: &str, f: f32| {
                let pct = (f * 100.) as i32;
                if last.replace(pct) != pct {
                    let _ = tx.send(Event::Progress(line.to_string(), f));
                }
            };
            let mut modpack = false;
            let result = browser::source(source)
                .ok_or_else(|| format!("{} isn't set up.", source.label()))
                .and_then(|source| {
                    let p = match known {
                        Some(p) => p,
                        None => source.project(&id)?,
                    };
                    if let Some(target) = java.as_ref().filter(|t| t.takes(p.kind)) {
                        let done = java_instance::install(&*source, target, &p, &home, &progress)?;
                        let names: Vec<String> = done
                            .iter()
                            .map(|e| format!("{} {}", e.title, e.number))
                            .collect();
                        return Ok(format!(
                            "Installed {}, checksums checked.",
                            names.join(", ")
                        ));
                    }
                    if p.kind == PackKind::DataPack {
                        let (world, name) =
                            world.ok_or("Pick the world the data pack goes into first.")?;
                        let done =
                            installed::install_datapack(&*source, &p, everything, &world, &|f| {
                                progress(&p.title, f)
                            })?;
                        return Ok(format!(
                            "Put {} {} in {name}'s data packs, checksum checked: the game turns \
                             it on when the world opens. {}",
                            p.title,
                            done.number,
                            done.coverage.summary()
                        ));
                    }
                    modpack = p.kind == PackKind::Modpack;
                    if modpack && p.source == SourceId::Horizon {
                        let pack = horizon::pack::install(&p.id, &progress)?;
                        return Ok(format!(
                            "Installed {} {} as its own profile with {}, each checked. Play it \
                             from the list above.",
                            pack.name,
                            pack.version,
                            pack.mods.join(", ")
                        ));
                    }
                    if modpack {
                        let folder = modpacks::ports_dir().join("downloads");
                        let file = installed::fetch_modpack(&*source, &p, &folder, &|f| {
                            progress(&p.title, f)
                        })?;
                        let log = |line: String| progress(&line, 1.);
                        let pack = modpacks::import_and_port(&file, resources.as_deref(), &log)?;
                        return Ok(format!(
                            "{} Play it from the list above.",
                            modpacks::outcome(&pack)
                        ));
                    }
                    let done = installed::install(&*source, &p, everything, &home, &progress)?;
                    let names: Vec<String> = done
                        .iter()
                        .map(|e| format!("{} {}", e.title, e.number))
                        .collect();
                    if p.source == SourceId::Horizon && p.kind == PackKind::Mod {
                        return Ok(format!(
                            "Installed {} into horizon-mods and turned it on, checksum checked \
                             (Options > Mods in the game).",
                            names.join(", ")
                        ));
                    }
                    if p.kind == PackKind::Mod {
                        let skipped = done.last().map_or(0, |e| e.skipped.len());
                        return Ok(format!(
                            "Ported {} into horizon-mods and turned on (Options > Mods in the \
                             game).{}",
                            names.join(", "),
                            if skipped > 0 {
                                " What didn't port is listed under Installed."
                            } else {
                                ""
                            }
                        ));
                    }
                    Ok(format!(
                        "Installed {}, checksums checked. Turn it on in the game: Options > \
                         Resource Packs.",
                        names.join(", ")
                    ))
                });
            let _ = tx.send(Event::Done(result, modpack));
        });
    }

    fn mods_field(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let m = &self.mods;
        let focused = m.focused;
        let field = div()
            .id("mods-search")
            .track_focus(&m.focus)
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(320.))
            .h(px(theme::FIELD + 4.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(blocks::icon("search", 18., t.muted))
            .child(line_edit::content(
                &m.text,
                &t,
                focused,
                false,
                false,
                "Search, then Enter",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.mods.focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.mods.text, ev, cx) {
                    Key::Enter => this.mods_search(0),
                    Key::Escape => window.blur(),
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, |this| &mut this.mods.text)
    }

    pub(super) fn mods_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // Packs and mods go into the selected instance: picking another reads its list.
        let home = self.selected_instance().map(|i| i.home.clone());
        let java = self
            .selected_instance()
            .and_then(|i| i.java.as_ref())
            .map(|j| j.target());
        // Another version or loader: other results.
        if self.mods.loaded && java != self.mods.java {
            self.mods.java = java.clone();
            self.mods_search(0);
        }
        self.mods.java = java;
        if !self.mods.loaded || (home.is_some() && self.mods.home != home) {
            self.mods.home = home;
            self.mods.installed = installed::Installed::load(&self.mods.home()).entries;
            self.mods.installed_list.reset();
            self.mods.updates.clear();
            self.mods.confirm_remove = None;
            self.mods.read_worlds();
            self.mods_check_updates();
            self.mods_taken_down();
            if !self.mods.loaded {
                self.mods_search(0);
            }
            self.mods.loaded = true;
        }
        let header = page_header(
            "Mods",
            "Mods and shader packs made for Horizon from its own reviewed registry, and Java \
             mods, modpacks and resource packs from Modrinth: Horizon ports what a Java mod \
             carries (blocks, items, recipes, worldgen) and says before you install what runs. Each file is checked \
             against its source's checksum before it reaches your game folder.",
            &t,
        );
        let this = cx.entity().downgrade();
        let kinds = blocks::tabs("mods-kind", &KINDS, self.mods.kind, &t, {
            let this = this.clone();
            move |kind, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.mods.kind != kind {
                        this.mods.kind = kind;
                        // Horizon's own shader packs are on its registry only.
                        if kind == PackKind::Shader {
                            this.mods.source = SourceId::Horizon;
                        }
                        // Data packs are on Modrinth only; a world may have been made since.
                        if kind == PackKind::DataPack {
                            this.mods.source = SourceId::Modrinth;
                            this.mods.read_worlds();
                        }
                        // Categories and loaders differ by kind.
                        this.mods.browse.filters = Default::default();
                        this.mods.browse.open = None;
                        this.mods_search(0);
                    }
                    cx.notify();
                });
            }
        });
        let everything = self.mods.everything;
        let toggle = div()
            .id("mods-everything")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("mods-everything-switch", everything, &t))
            .child(
                text(theme::LABEL)
                    .text_color(t.ink_soft)
                    .child("Show everything"),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.everything = !this.mods.everything;
                this.mods_search(0);
                cx.notify();
            }));
        // CurseForge support is coming soon: shown, inert, never searched (like the Microsoft
        // account's "Coming soon").
        // A Java instance's mods and resource packs come from Modrinth, for its version.
        let java = self.mods.java_for(self.mods.kind).is_some();
        let source_note = (self.mods.kind != PackKind::DataPack && !java)
            .then(|| div().flex().child(self.mods_source_switch(cx)));
        // The registry lists only what was made for Horizon: no "everything", no Modrinth filters.
        let registry = self.mods.source == SourceId::Horizon;
        let filters = (!registry).then(|| self.mods_filters(cx));
        let controls = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(self.mods_field(cx))
                    .when(!registry && !java, |d| d.child(toggle)),
            )
            .children(source_note)
            .children(filters)
            .children(self.mods_target_line(cx));
        // The kinds are inventory tabs on the panel that holds the open one.
        let body = blocks::panel(&t).flex().flex_col().gap(px(24.)).p(px(20.));
        let page = |body: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(32.))
                .child(header)
                .child(div().flex().flex_col().child(kinds).child(body))
        };
        let body = body.children(self.native_notice(cx));
        if let Some(project) = self.mods_project_page(cx) {
            return page(body.children(self.mods_busy_plate()).child(project));
        }
        let mut body = body.children(self.mods_busy_plate());
        if self.mods.kind == PackKind::Modpack {
            body = body
                .child(self.imported_packs(cx))
                .child(div().h(px(2.)).bg(t.rule))
                .child(section_title("Browse modpacks", "search", &t));
        }
        body = body.child(controls);
        if let Some(list) = self.mods_installed_list(cx) {
            body = body.child(list);
        }
        page(body.child(self.mods_results(cx)))
    }

    /// Which instance installs go into, when there is more than one to pick from. A modpack
    /// becomes an instance of its own.
    fn mods_target_line(&self, cx: &mut Context<Self>) -> Option<Div> {
        let instance = self.selected_instance()?;
        if self.mods.kind == PackKind::DataPack {
            return Some(self.mods_world_picker(&instance.name.clone(), cx));
        }
        if self.instances.list.len() < 2 || self.mods.kind == PackKind::Modpack {
            return None;
        }
        Some(
            text(theme::CAPTION)
                .text_color(self.theme.muted)
                .child(format!(
                    "Installs into {}: pick another instance on the left.",
                    instance.name
                )),
        )
    }

    /// The world data packs go into: one of the instance's, picked here.
    fn mods_world_picker(&self, instance: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let caption = |s: String| text(theme::CAPTION).text_color(t.muted).child(s);
        if self.mods.worlds.is_empty() {
            return div().child(caption(format!(
                "A data pack goes into a world, and {instance} has none yet: create one in the \
                 game, then come back."
            )));
        }
        let mut chips = div().flex().flex_wrap().items_center().gap(px(6.));
        for (i, (path, name)) in self.mods.worlds.iter().enumerate() {
            let on = self.mods.world.as_ref() == Some(path);
            let path = path.clone();
            chips = chips.child(
                button(
                    SharedString::from(format!("mods-world-{i}")),
                    name.clone(),
                    if on { Kind::Secondary } else { Kind::Ghost },
                    &t,
                )
                .when(on, |b| b.border_color(t.accent))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods.world = Some(path.clone());
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(format!(
                "Installs into a world of {instance}; the game turns the pack on when the world \
                 opens."
            )))
            .child(chips)
    }

    /// The install under way and the last install's answer.
    /// The one-time notice before the first native mod is installed: what the build takes. Its
    /// button installs the mod; Native mods then turn on.
    fn native_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.mods.native_notice.as_ref()?;
        let t = self.theme;
        Some(
            plate(&t)
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(16.))
                .text_size(px(theme::UI.px))
                .text_color(t.ink)
                .child(tr(native_mods::NOTICE).to_string())
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button(
                                "native-notice-install",
                                "Install and turn on native mods",
                                Kind::Primary,
                                &t,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.settings.native_mods_noticed = true;
                                    this.settings.save();
                                    if let Some((source, id, known)) =
                                        this.mods.native_notice.take()
                                    {
                                        this.mods_install(source, id, known);
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            button("native-notice-cancel", "Cancel", Kind::Secondary, &t).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.mods.native_notice = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                ),
        )
    }

    pub(super) fn mods_busy_plate(&self) -> Vec<Div> {
        let t = self.theme;
        let mut out = Vec::new();
        if let Some((what, line, f)) = &self.mods.busy {
            let caption = if what == "all" {
                "Updating"
            } else {
                "Installing"
            };
            out.push(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .child(caps(caption).text_color(t.accent))
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.ink_soft)
                            .truncate()
                            .child(format!("{line}  {:.0}%", f * 100.)),
                    ),
            );
        }
        match &self.mods.message {
            Some(Ok(done)) => out.push(
                plate(&t)
                    .p(px(16.))
                    .text_size(px(theme::UI.px))
                    .text_color(t.ink)
                    .child(done.clone()),
            ),
            Some(Err(e)) => out.push(notice(e.clone(), &t)),
            None => {}
        }
        out
    }

    fn mods_installed_list(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let kind = self.mods.kind;
        let entries: Vec<&installed::Entry> = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind)
            .collect();
        if entries.is_empty() {
            return None;
        }
        let busy = self.mods.busy.is_some();
        let row = |i: usize| {
            let e = entries[i];
            let key = (e.source, e.project.clone());
            let mut right = div().flex().items_center().gap(px(8.));
            if let Some(number) = self.mods.updates.get(&key) {
                let mut update = button(
                    SharedString::from(format!("mods-update-{}", e.project)),
                    format!("Update to {number}"),
                    Kind::Secondary,
                    &t,
                );
                if busy {
                    update = disabled(update);
                } else {
                    let (source, id) = key.clone();
                    update = update.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(source, id.clone(), None);
                        cx.notify();
                    }));
                }
                right = right.child(update);
            }
            let confirming = self.mods.confirm_remove.as_deref() == Some(e.project.as_str());
            let id = e.project.clone();
            let remove = button(
                SharedString::from(format!("mods-remove-{}", e.project)),
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
                if this.mods.confirm_remove.as_ref() == Some(&id) {
                    let home = this.mods.home();
                    let removed = match this.mods.java {
                        Some(_) => java_instance::remove(&home, &id),
                        None => installed::remove(&home, &id),
                    };
                    this.mods.message = Some(removed.map(|e| format!("Removed {}.", e.title)));
                    this.mods.installed = installed::Installed::load(&home).entries;
                    this.mods.installed_list.reset();
                    this.mods.confirm_remove = None;
                } else {
                    this.mods.confirm_remove = Some(id.clone());
                }
                cx.notify();
            }));
            let on = match &self.mods.java {
                _ if e.kind != PackKind::Mod => None,
                Some(_) => Some(java_instance::is_on(&self.mods.home(), e)),
                None if e.source == SourceId::Horizon => Some(
                    horizon::mod_id(e).is_some_and(|id| java::is_enabled(&self.mods.home(), id)),
                ),
                None => None,
            };
            if let Some(on) = on {
                let entry = e.clone();
                right = right.child(
                    div()
                        .id(SharedString::from(format!("mods-switch-{}", e.project)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        // Its id sits under the row's own, so one name serves every row.
                        .child(switch("mods-switch-knob", on, &t))
                        .child(text(theme::LABEL).text_color(t.ink_soft).child(if on {
                            "On"
                        } else {
                            "Off"
                        }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_switch(&entry);
                            cx.notify();
                        })),
                );
            }
            right = right.child(remove);
            let detail = div()
                .flex()
                .flex_col()
                .child(format!(
                    "{}{} · from {}{} · {}",
                    e.verdict
                        .as_ref()
                        .map(|v| format!("{v} · "))
                        .unwrap_or_default(),
                    e.number,
                    e.source.label(),
                    if e.dependency {
                        ", needed by another pack"
                    } else {
                        ""
                    },
                    e.file
                ))
                .children(e.skipped.iter().map(|s| {
                    div()
                        .text_color(t.ink_soft)
                        .child(format!("Not ported: {s}"))
                }))
                .child(
                    div()
                        .font_family(theme::MONO)
                        .child(format!("checked: {}", short_sum(&e.checksum))),
                );
            index_row(
                i + 1,
                e.title.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        // Every kind's updates at once, whichever tab is open.
        let update_all = (!self.mods.updates.is_empty()).then(|| {
            let b = button(
                "mods-update-all",
                format!("Update all ({})", self.mods.updates.len()),
                Kind::Secondary,
                &t,
            );
            if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.mods_update_all();
                    cx.notify();
                }))
            }
        });
        let list = self.mods.installed_list.column(
            &self.content_scroll.handle,
            entries.len(),
            8.,
            |index| self.mods.confirm_remove.as_ref() == Some(&entries[index].project),
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(caps("Installed").text_color(t.muted))
                        .children(update_all),
                )
                .child(list),
        )
    }

    fn mods_results(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let m = &self.mods;
        if let Some(e) = &m.error {
            return div().child(notice(e.clone(), &t));
        }
        if m.results.is_empty() {
            let line = if m.searching || !m.searched {
                "Searching…"
            } else if let Some(target) = m.java_for(m.kind) {
                return centered(
                    &format!("Nothing found for {}.", target.describe(m.kind)),
                    &t,
                );
            } else if m.source == SourceId::Horizon {
                "Nothing in Horizon's registry matches yet."
            } else if m.everything {
                "Nothing found."
            } else {
                "Nothing found that runs in Horizon. Show everything lists the rest."
            };
            return centered(line, &t);
        }
        let result_count = m.results.len();
        let rows = m.results_list.clone();
        let scroll = self.content_scroll.handle.clone();
        let row = |i: usize| {
            let project = self.mods.results[i].clone();
            let icon = self.mods_icon(&project, 56.);
            let p = &project;
            let m = &self.mods;
            let busy = m.busy.as_ref().map(|b| b.0.as_str());
            let support = m.support(p);
            let checking = m.checking(p);
            let color = match support {
                Support::Works => t.success,
                Support::Partial(_) => t.warn,
                Support::No(_) => t.muted,
            };
            let mut right = div().flex().items_center().gap(px(8.)).child(blocks::badge(
                support.label(),
                color,
                &t,
            ));
            let page_url = p.page.clone();
            if !page_url.is_empty() {
                right = right.child(
                    button(
                        SharedString::from(format!("mods-page-{}", p.id)),
                        "Page",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| cx.open_url(&page_url)),
                );
            }
            if busy == Some(p.id.as_str()) {
                right = right.child(caps("Installing").text_color(t.accent));
            } else if m.entry(p).is_some() {
                right = right.child(caps("Installed").text_color(t.muted));
            } else if checking {
                right = right.child(caps("Checking").text_color(t.muted));
            } else if support.installable() {
                let label = if p.kind == PackKind::Modpack {
                    "Import"
                } else {
                    "Install"
                };
                let mut install = button(
                    SharedString::from(format!("mods-install-{}", p.id)),
                    label,
                    Kind::Secondary,
                    &t,
                );
                if busy.is_some() {
                    install = disabled(install);
                } else {
                    let p = p.clone();
                    install = install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(p.source, p.id.clone(), Some(p.clone()));
                        cx.notify();
                    }));
                }
                right = right.child(install);
            }
            let mut by = Vec::new();
            if !p.author.is_empty() {
                by.push(format!("by {}", p.author));
            }
            by.push(format!("{} downloads", count(p.downloads)));
            by.push(format!("from {}", p.source.label()));
            if !p.license.is_empty() {
                by.push(p.license.clone());
            }
            let detail = div()
                .flex()
                .flex_col()
                .child(div().truncate().child(p.summary.clone()))
                .child(by.join(" · "))
                .children(
                    (p.source == SourceId::Horizon)
                        .then(|| stars(&p.id, &t))
                        .flatten(),
                )
                .children(
                    support
                        .reason()
                        .map(|r| div().text_color(t.ink_soft).child(r.to_string())),
                )
                .children(
                    m.about
                        .get(&p.id)
                        .filter(|_| p.source == SourceId::Horizon)
                        .map(|lines| div().text_color(t.ink_soft).child(lines.join(" · "))),
                );
            let open = p.clone();
            let name = div()
                .id(SharedString::from(format!("mods-open-{}", p.id)))
                .flex()
                .items_center()
                .gap(px(12.))
                .cursor_pointer()
                .child(icon)
                .child(div().hover(|s| s.underline()).child(p.title.clone()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_open(open.clone());
                    cx.notify();
                }));
            index_row(
                i + 1,
                name,
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        let list = rows.column(&scroll, result_count, 8., |_| false, row);
        let mut out = div().flex().flex_col().child(list);
        let m = &self.mods;
        if m.more {
            let mut more = button("mods-more", "More", Kind::Ghost, &t);
            if m.searching {
                more = disabled(more);
            } else {
                let offset = m.results.len();
                more = more.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_search(offset);
                    cx.notify();
                }));
            }
            out = out.child(div().flex().pt(px(16.)).child(more));
        }
        out
    }
}

impl Launcher {
    /// The source switch: Modrinth or Horizon's registry, CurseForge shown as coming soon and not
    /// clickable, like the Microsoft account's "Coming soon".
    fn mods_source_switch(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let this = cx.entity().downgrade();
        source_switch(&self.theme, self.mods.source, move |source, _, cx| {
            let _ = this.update(cx, |this, cx| {
                if this.mods.source != source {
                    this.mods.source = source;
                    this.mods.browse.filters = Default::default();
                    this.mods.browse.open = None;
                    this.mods_search(0);
                }
                cx.notify();
            });
        })
    }
}

fn source_switch(
    t: &Theme,
    current: SourceId,
    on_pick: impl Fn(SourceId, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let soon = div()
        .flex()
        .items_center()
        .h(px(theme::SEGMENT))
        .px(px(14.))
        .pb(px(2.))
        .opacity(theme::DISABLED)
        .child(
            blocks::pixel(14., FontWeight::MEDIUM)
                .text_color(t.muted)
                .child("CurseForge (coming soon)"),
        );
    segments("mods-source", &SOURCES, current, t, on_pick).child(soon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_checksums_read_short() {
        assert_eq!(count(999), "999");
        assert_eq!(count(1_500), "1.5k");
        assert_eq!(count(51_404_735), "51.4M");
        let sum = format!("SHA-512:{}", "ab".repeat(64));
        assert_eq!(short_sum(&sum), "SHA-512 abababababab…abababab");
        assert_eq!(short_sum("SHA-1:abcd"), "SHA-1:abcd");
    }
}
