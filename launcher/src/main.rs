//! Horizon launcher: installs Horizon builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod agents_page;
mod art;
mod blocks;
mod cli;
mod cli_tools;
mod collection;
mod console;
mod console_page;
mod cull;
mod datapacks_page;
mod dev_options;
mod export_page;
mod game_tools;
mod import_page;
mod instances_page;
mod internet;
mod language_page;
mod launch_page;
mod line_edit;
mod markdown;
mod mcp;
mod modpacks_page;
mod mods_page;
mod mods_project;
mod multiplayer_page;
mod owner_page;
mod packs_page;
mod perf;
mod plate;
mod screenshots_page;
mod scrollbar;
mod servers_page;
mod shortcut_page;
mod skin_page;
mod sky;
mod specimen;
mod switch_page;
mod theme;
mod themes_page;
mod titlebar;
mod while_playing;
mod worlds_page;

use gpui::{
    actions, div, ease_out_quint, prelude::*, px, relative, size, Animation, AnimationExt,
    AnyElement, App, Application, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle,
    FontWeight, Hsla, KeyBinding, KeyDownEvent, Menu, MenuItem, PathPromptOptions, SharedString,
    Stateful, Subscription, Window, WindowBounds, WindowOptions,
};
use horizon_launcher::account::{self, Accounts, SignIn, SignInState};
use horizon_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use horizon_launcher::i18n::{self, tr, trf};
use horizon_launcher::instances;
use horizon_launcher::native_mods;
use horizon_launcher::telemetry;
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    /// Mods, resource packs and modpacks (imported ones and the browser's).
    Mods,
    Servers,
    Notes,
    Settings,
}

impl Page {
    /// `HORIZON_LAUNCHER_PAGE=mods` (or `modpacks`, `shaders`, `servers`, `versions`, `notes`,
    /// `settings`) opens on that page, for screenshots; Home otherwise.
    fn from_env() -> Page {
        match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
            Ok("mods" | "modpacks" | "shaders" | "datapacks") => Page::Mods,
            Ok("servers") => Page::Servers,
            Ok("versions") => Page::Versions,
            Ok("notes") => Page::Notes,
            Ok("settings") => Page::Settings,
            _ => Page::Play,
        }
    }

    /// Every page, with its name and icon: the top bar shows all but Settings, which has its
    /// own button.
    const ALL: [(Page, &'static str, &'static str); 6] = [
        (Page::Play, "Home", "home"),
        (Page::Mods, "Mods", "package"),
        (Page::Servers, "Servers", "server"),
        (Page::Versions, "Versions", "archive"),
        (Page::Notes, "Patch notes", "notes"),
        (Page::Settings, "Settings", "sliders"),
    ];
}

/// The launcher's own update, as the footer and Settings show it.
#[derive(Clone, PartialEq)]
enum SelfUpdate {
    Idle,
    Downloading(String, f32),
    Ready(String),
    Failed(String),
}

/// The last game the launcher started ended badly: what it left behind.
#[derive(Clone)]
struct Crash {
    /// The crash report the run wrote, if it wrote one.
    report: Option<std::path::PathBuf>,
    /// One line on what went wrong: the report's error, else how the process ended.
    summary: String,
    copied: bool,
}

/// The opening: the wordmark holds at least `OPENING_HOLD` (until the release
/// list is in, at most `OPENING_MAX`), then the launcher appears over `OPENING_REVEAL`.
const OPENING_HOLD: Duration = horizon_theme::OPENING_HOLD;
const OPENING_MAX: Duration = horizon_theme::OPENING_MAX;
const OPENING_REVEAL: Duration = horizon_theme::OPENING_REVEAL;

/// The phase of the opening at one moment: how far the reveal has gone, 0 to 1.
struct Opening {
    /// Set on the first frame, so a slow start does not eat into the hold.
    started: Option<Instant>,
    reveal: Option<Instant>,
    focus: FocusHandle,
}

fn ease_out_cubic(x: f32) -> f32 {
    horizon_theme::ease_out_cubic(x)
}

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

const LOG_LINES: usize = 2000;

struct Launcher {
    theme: Theme,
    page: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    /// The rows of the page's long list (Versions, Patch notes) last drawn.
    long_list: cull::Cull,
    opening: Option<Opening>,
    updates: Updates,
    checked: Option<Instant>,
    update_error: Option<String>,
    self_update: SelfUpdate,
    /// The Horizon version being installed as an update, and whether it becomes the selected one.
    update_install: Option<(String, bool)>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    releases: Load<Vec<Release>>,
    installed: Vec<String>,
    installing: Option<(String, f32)>,
    /// The version being deleted on a background thread. It is already out of `installed`, so
    /// nothing can launch, select or prepare it, and its row can't install or remove it again.
    removing: Option<String>,
    /// What each installed version takes on disk, measured on a thread for the Versions page, and
    /// the installed list it was measured for.
    disk: Vec<(String, u64)>,
    disk_for: Vec<String>,
    install_error: Option<(String, String)>,
    /// Native mods (`native_mods`): the build's step while it runs, why it failed, and the
    /// warning before a build while it is open, with the mods it names.
    native_step: Option<String>,
    native_error: Option<String>,
    native_confirm: Option<Result<Vec<native_mods::Bundle>, String>>,
    confirm_remove: Option<String>,
    picker_open: bool,
    /// The top bar's quick switch of who plays (`switch_page.rs`).
    account_menu: bool,
    /// Settings > Account's skin panel for an offline name (`skin_page.rs`).
    skin: Option<skin_page::SkinPanel>,
    /// Why the last picture for Home's background was refused.
    background_error: Option<String>,
    settings: Settings,
    /// The Discord, sponsor and website links: built in until links.json on main has been read.
    links: backend::Links,
    preparing: bool,
    prepare_log: VecDeque<String>,
    prepare_error: Option<String>,
    client: Option<Running>,
    client_error: Option<String>,
    /// When the running client started, to tell its crash report from older ones.
    client_started: Option<std::time::SystemTime>,
    /// The folder the running game keeps its settings in: the game home, or a modpack's.
    client_home: Option<std::path::PathBuf>,
    crash: Option<Crash>,
    /// The Servers page (`servers_page.rs`).
    servers: servers_page::Servers,
    /// Its Friends over the internet panel (`internet.rs`).
    internet: internet::Internet,
    /// The window's own focus, so Tab reaches the controls when nothing else has the keyboard.
    root_focus: FocusHandle,
    /// Microsoft accounts (accounts.json), the sign-in under way and what it last showed.
    accounts: Accounts,
    sign_in: Option<SignIn>,
    sign_in_state: Option<SignInState>,
    /// What to say when the keychain doesn't keep sign-ins ([`account::no_keychain`]).
    keychain: Option<String>,
    /// Whether an account here owns the game, by the keychain's last answers (the owner gate);
    /// none until the keychain has been read.
    owner: Option<bool>,
    /// Play was refused (or Home's Sign in pressed): the owner gate's screen is open.
    owner_screen_open: bool,
    /// Clicks on the sidebar's launcher version, and whether they revealed the developer options
    /// (`dev_options`) for this run.
    dev_clicks: u8,
    dev_options: bool,
    /// The game is waiting for its session (a token renewal), or a Java instance for its files
    /// and Java (`start_java`), before it starts.
    starting_client: Option<String>,
    /// Play was pressed on a version not prepared yet: it starts once this preparation ends well.
    play_when_prepared: Option<String>,
    /// Join a friend: the code typed on the Play page, and the one the game
    /// starting now joins.
    friend_code: line_edit::LineEdit,
    friend_focused: bool,
    friend_focus: FocusHandle,
    joining: Option<String>,
    /// Where the next start goes at once: `--world <folder>` or `--server <address>`.
    quick_play: Option<(&'static str, String)>,
    /// The instance the running game plays and when it started: its play time grows on exit.
    client_run: Option<(String, Instant)>,
    /// The running game's instance variables, for its post-exit command.
    client_vars: horizon_launcher::launch::Vars,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: line_edit::LineEdit,
    args_focused: bool,
    args_focus: FocusHandle,
    /// Settings' Proxy field, the same way (`proxy`).
    proxy_input: line_edit::LineEdit,
    proxy_focused: bool,
    proxy_focus: FocusHandle,
    /// Opt-in data collection: the player's answer and Settings' section (`collection`).
    collection: collection::Collection,
    /// Settings' "Connect an AI agent" (`agents_page`).
    agents: agents_page::AgentsUi,
    /// How many play Horizon now, while the server answers ("N playing now" on the Play page).
    playing: Option<u64>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
    /// Redraws when the window gains or loses focus: the sky moves only in front.
    _activation: Subscription,
    /// The session's measurements (`perf.rs`), and when the steps they time began.
    perf: perf::Perf,
    install_started: Option<Instant>,
    prepare_started: Option<Instant>,
    play_pressed: Option<Instant>,
    /// `horizon-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
    /// The Modpacks page: the imported packs (read when the page first opens), the one shown,
    /// the report's filter, and the pipeline step running now with its log.
    packs: Vec<horizon_launcher::modpacks::Imported>,
    /// Modpacks installed from Horizon's registry, each its own profile.
    registry_packs: Vec<horizon_launcher::browser::horizon::pack::Installed>,
    packs_loaded: Option<Instant>,
    pack_open: Option<String>,
    pack_filter: Option<horizon_port_pack::report::Status>,
    pack_busy: bool,
    pack_log: VecDeque<String>,
    pack_error: Option<String>,
    /// The game starting now plays this pack's profile (its work folder).
    starting_pack: Option<std::path::PathBuf>,
    /// The Mods page: the browser's search, installs and updates (`mods_page.rs`).
    mods: mods_page::Mods,
    /// The instances, for the sidebar and Play, and their dialog (`instances_page.rs`).
    instances: instances_page::Instances,
    /// Home's Worlds section, of the selected instance (`worlds_page.rs`).
    worlds: worlds_page::Worlds,
    multiplayer: multiplayer_page::Multiplayer,
    screenshots: screenshots_page::Screenshots,
    game_packs: packs_page::Packs,
    world_packs: datapacks_page::WorldPacks,
    export: Option<export_page::ExportPanel>,
    launch: Option<launch_page::LaunchPanel>,
    importing: import_page::Importing,
    console: console_page::Console,
    /// What the window did when the game started (`while_playing.rs`).
    aside: while_playing::Aside,
    /// The selected instance's desktop shortcut (`shortcut_page.rs`).
    shortcuts: shortcut_page::Shortcuts,
    /// The name the game made up for an offline player (`horizon-player.txt`), for the top bar.
    offline_name: Option<String>,
    /// The player's servers, for the sidebar until the Servers page has read them.
    side_servers: Vec<horizon_launcher::servers::Entry>,
    /// The pixel art made for this window (`art.rs`), and pictures downloaded by URL.
    sky: sky::Sky,
    /// When the sky next asks for a frame, and the timer that will draw it.
    sky_wake: Option<(Instant, gpui::Task<()>)>,
    /// The window's scale, as of its last frame, for art drawn outside `render`'s reach.
    scale: f32,
}

impl Launcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                break;
            }
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `HORIZON_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("HORIZON_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = horizon_launcher::desktop::repair(backend::exe_path());
            });
        }
        let installed = backend::installed();
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = installed.first().cloned();
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let mut this = Self {
            theme: Theme::for_appearance(settings.appearance, settings.accent, window.appearance()),
            page: Page::from_env(),
            switches: 0,
            content_scroll: Scrollbar::default(),
            long_list: cull::Cull::default(),
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            updates: Updates::default(),
            checked: None,
            update_error: None,
            self_update: SelfUpdate::Idle,
            update_install: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            installing: None,
            removing: None,
            disk: Vec::new(),
            disk_for: Vec::new(),
            install_error: None,
            native_step: None,
            native_error: None,
            native_confirm: None,
            confirm_remove: None,
            picker_open: false,
            account_menu: false,
            skin: None,
            background_error: None,
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle(),
            proxy_input: line_edit::LineEdit::new(
                settings.proxy.clone().unwrap_or_default(),
                256,
                44,
            ),
            proxy_focused: false,
            proxy_focus: cx.focus_handle(),
            settings,
            links: backend::Links::default(),
            preparing: false,
            prepare_log: VecDeque::new(),
            prepare_error: None,
            client: None,
            client_error: None,
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            accounts: Accounts::load(),
            sign_in: None,
            sign_in_state: None,
            keychain: None,
            owner: None,
            owner_screen_open: false,
            dev_clicks: 0,
            dev_options: false,
            starting_client: None,
            play_when_prepared: None,
            friend_code: line_edit::LineEdit::new(String::new(), 256, 32),
            friend_focused: false,
            friend_focus: cx.focus_handle(),
            joining: None,
            quick_play: None,
            client_run: None,
            client_vars: Vec::new(),
            collection: collection::Collection::load(),
            agents: agents_page::AgentsUi::default(),
            playing: None,
            tx,
            rx,
            _appearance: appearance,
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            play_pressed: None,
            bench: perf::Bench::from_env(),
            packs: Vec::new(),
            registry_packs: Vec::new(),
            packs_loaded: None,
            pack_open: None,
            pack_filter: None,
            pack_busy: false,
            pack_log: VecDeque::new(),
            pack_error: None,
            starting_pack: None,
            mods: mods_page::Mods::new(cx),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            screenshots: screenshots_page::Screenshots::default(),
            game_packs: packs_page::Packs::default(),
            world_packs: datapacks_page::WorldPacks::default(),
            export: None,
            launch: None,
            importing: Default::default(),
            console: Default::default(),
            aside: while_playing::Aside::new(window.window_handle()),
            shortcuts: Default::default(),
            offline_name: offline_name(),
            side_servers: horizon_launcher::servers::list(),
            sky: sky::Sky::default(),
            sky_wake: None,
            scale: window.scale_factor(),
        };
        this.reload_instances();
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        let keychain_tx = this.tx.clone();
        // Asking the keychain can block (D-Bus), so not on this thread.
        let accounts = this.accounts.clone();
        std::thread::spawn(move || {
            let store = account::store();
            let _ = keychain_tx.send(Msg::Keychain(account::no_keychain()));
            let _ = keychain_tx.send(Msg::Owner(accounts.has_owner(store)));
        });
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.fetch_startup_releases();
        this.fetch_links();
        // What the game and the launcher kept, if the player allows it and there is a server.
        telemetry::send_in_background();
        telemetry::watch_playing(this.tx.clone());
        this
    }

    fn go(&mut self, page: Page) {
        // The game may have made up a name since, and the CLI or another window may have
        // changed the accounts: read them again (small files, once a switch).
        self.offline_name = offline_name();
        self.accounts = Accounts::load();
        if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
            self.long_list = cull::Cull::default();
            if page == Page::Mods {
                self.mods.reset_lists();
            }
        }
        self.picker_open = false;
        self.account_menu = false;
    }

    fn check_updates(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updates(backend::check_updates().map(Box::new)));
        });
    }

    fn update_launcher(&mut self, release: Release) {
        if !backend::self_update_allowed()
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Ready(_)
            )
        {
            return;
        }
        let version = backend::launcher_version(&release.tag)
            .unwrap_or_default()
            .to_string();
        self.self_update = SelfUpdate::Downloading(version, 0.);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::LauncherProgress(p));
            };
            let _ = tx.send(Msg::LauncherUpdated(backend::self_update(
                &release, progress,
            )));
        });
    }

    /// Installs the Horizon version the last check found, keeping the player's choice of
    /// version unless they were on the newest one.
    fn install_update(&mut self) {
        let Some(release) = self.updates.horizon.clone() else {
            return;
        };
        if self.installing.is_some() || self.removing.as_ref() == Some(&release.tag) {
            return;
        }
        let select =
            backend::select_after_update(self.settings.selected.as_deref(), &self.installed);
        self.update_install = Some((release.tag.clone(), select));
        self.install(&release);
    }

    /// The native mods turned on in the instance Play starts, unpacked and checked, with their
    /// bundles: what the warning names and the build compiles.
    fn native_mods_to_build(
        &self,
    ) -> Result<(std::path::PathBuf, Vec<native_mods::Bundle>), String> {
        let folder = native_mods::installed_mods(&instances::selected().home)?;
        let found = native_mods::bundles(&folder)?;
        Ok((folder, found))
    }

    /// Builds the selected instance's native mods, once the player accepted the warning. The
    /// build reuses the game files of the version the instance plays, else the newest installed.
    fn build_native(&mut self) {
        if self.native_step.is_some() {
            return;
        }
        self.native_confirm = None;
        self.native_error = None;
        let instance = instances::selected();
        let base = instance
            .version
            .clone()
            .filter(|v| !native_mods::is_native(v))
            .or_else(|| {
                self.installed
                    .iter()
                    .find(|t| backend::is_game_tag(t) || t.starts_with("nightly-"))
                    .cloned()
            });
        let Some(base) = base else {
            self.native_error = Some(tr("Install Horizon first.").into());
            return;
        };
        let folder = match self.native_mods_to_build() {
            Ok((folder, _)) => folder,
            Err(e) => {
                self.native_error = Some(e);
                return;
            }
        };
        self.native_step = Some(tr("Starting the build").into());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = native_mods::build(&base, &folder, Some(&instance), |step| {
                let _ = tx.send(Msg::NativeProgress(step.label()));
            });
            let _ = tx.send(Msg::Native(result));
        });
    }

    fn fetch_releases(&mut self) {
        self.releases = Load::Loading;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Releases(backend::releases()));
        });
    }

    fn fetch_startup_releases(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let updates = backend::releases_and_updates(|releases| {
                let _ = tx.send(Msg::Releases(releases));
            });
            let _ = tx.send(Msg::Updates(updates.map(Box::new)));
        });
    }

    fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        self.perf.tick();
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            Some(perf::Step::Go(page)) => {
                self.go(Page::ALL[page % Page::ALL.len()].0);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.go(Page::Versions);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => {
                    if let (Ok(_), Some(checked)) = (&r, self.checked) {
                        self.perf.record_since("launcher.release_list_ms", checked);
                    }
                    self.releases = r.map_or_else(Load::Failed, Load::Ready)
                }
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Keychain(said) => self.keychain = said,
                Msg::Owner(owns) => self.owner = Some(owns),
                Msg::Playing(playing) => self.playing = playing,
                Msg::JavaGame(id, home, cmd) => {
                    self.starting_client = None;
                    let spawned = cmd.and_then(|(cmd, vars)| {
                        self.client_vars = vars;
                        backend::spawn(Job::Client, cmd, self.tx.clone())
                    });
                    match spawned {
                        Ok(r) => {
                            self.client = Some(r);
                            self.client_home = Some(home);
                            self.client_run = Some((id, Instant::now()));
                            self.client_error = account::take_unsaved();
                            self.step_aside(cx);
                        }
                        Err(e) => {
                            if e == account::NoOwner.to_string() {
                                self.owner = Some(false);
                                self.owner_screen_open = true;
                            }
                            self.client_error = Some(e);
                        }
                    }
                }
                Msg::Session(session) => {
                    if let Some(tag) = self.starting_client.take() {
                        let mut cmd = backend::client_command(&tag);
                        let (file, offline) = match session {
                            Ok(pass) => pass,
                            Err(account::NoOwner) => {
                                self.owner = Some(false);
                                self.owner_screen_open = true;
                                self.client_error = Some(format!("{}.", tr(account::NEEDS_OWNER)));
                                self.starting_pack = None;
                                self.joining = None;
                                self.quick_play = None;
                                continue;
                            }
                        };
                        // Past the gate by the developer option alone is no owner.
                        if !self.settings.dev_offline {
                            self.owner = Some(true);
                        }
                        if let Some(file) = file {
                            cmd.arg("--session-file").arg(file);
                        }
                        // Still play, offline, and say why.
                        if let Some(e) = offline {
                            self.client_error = Some(format!("Playing offline: {e}"));
                        } else if let Some(e) = account::take_unsaved() {
                            self.client_error = Some(e);
                        }
                        self.accounts = Accounts::load();
                        // An offline name the player picked (the quick switch).
                        if let Some(name) = self.accounts.name_arg() {
                            cmd.arg("--name").arg(name);
                        }
                        cmd.envs(self.accounts.look_env());
                        // A pack's Play button plays that pack; Play plays the selected instance.
                        let (profile, run) = match self.starting_pack.take() {
                            Some(work) => {
                                let id = instances::list()
                                    .into_iter()
                                    .find(|i| i.work == work)
                                    .map(|i| i.id);
                                let applied =
                                    horizon_launcher::modpacks::apply_profile(&mut cmd, &work);
                                (applied, id)
                            }
                            None => {
                                let instance = instances::selected();
                                (instances::apply(&mut cmd, &instance), Some(instance.id))
                            }
                        };
                        if let Err(e) = profile {
                            self.client_error = Some(e);
                            continue;
                        }
                        if let Some(code) = self.joining.take() {
                            if let Err(e) = backend::join_friend(&mut cmd, &code) {
                                self.client_error = Some(e);
                            }
                        }
                        if let Some((flag, value)) = self.quick_play.take() {
                            cmd.arg(flag).arg(value);
                        }
                        backend::append_game_args(&mut cmd);
                        // Last, around every argument: the instance's pre-launch command,
                        // wrapper and variables.
                        let instance = run
                            .as_ref()
                            .and_then(|id| instances::list().into_iter().find(|i| &i.id == id));
                        self.client_vars = instance
                            .as_ref()
                            .map(|i| i.vars(cmd.get_program()))
                            .unwrap_or_default();
                        cmd.envs(self.client_vars.iter().map(|(k, v)| (k, v)));
                        let launch = instance.map(|i| i.launch).unwrap_or_default();
                        let cmd = horizon_launcher::launch::wrap(cmd, &launch);
                        self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                        match backend::spawn(Job::Client, cmd, self.tx.clone()) {
                            Ok(r) => {
                                self.client = Some(r);
                                self.client_run = run.map(|id| (id, Instant::now()));
                                self.step_aside(cx);
                            }
                            Err(e) => self.client_error = Some(e),
                        }
                    }
                }
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|(t, _)| *t == tag);
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        let name = if as_update.is_some() {
                            "launcher.update_ms"
                        } else {
                            "launcher.install_ms"
                        };
                        self.perf.record_since(name, started);
                    }
                    match result {
                        Ok(()) => {
                            if self.updates.horizon.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.horizon = None;
                            }
                            if as_update.is_none_or(|(_, select)| select) {
                                self.settings.selected = Some(tag);
                            }
                            self.settings.save();
                            // After the save: the selected version is never pruned.
                            let tx = self.tx.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(Msg::Pruned(backend::prune_old_versions()));
                            });
                        }
                        Err(e) => self.install_error = Some((tag, e)),
                    }
                }
                Msg::Removed(tag, result) => {
                    if self.removing.as_ref() == Some(&tag) {
                        self.removing = None;
                    }
                    self.installed = backend::installed();
                    self.settings.selected =
                        selection_after_removal(&self.installed, Settings::load().selected);
                    if let Err(e) = result {
                        self.install_error = Some((tag, e));
                    }
                }
                Msg::DiskUse(disk) => self.disk = disk,
                Msg::Pruned(gone) => {
                    if !gone.is_empty() {
                        self.installed = backend::installed();
                    }
                }
                Msg::Updates(Ok(updates)) => {
                    self.update_error = None;
                    self.updates = *updates;
                    if self.settings.auto_install() {
                        self.install_update();
                    }
                    if let Some(release) = self.updates.launcher.clone() {
                        self.update_launcher(release);
                    }
                }
                Msg::Updates(Err(e)) => self.update_error = Some(e),
                Msg::NativeProgress(step) => self.native_step = Some(step),
                Msg::Native(result) => {
                    self.native_step = None;
                    self.installed = backend::installed();
                    self.reload_instances();
                    if let Err(e) = result {
                        self.native_error = Some(e);
                    }
                }
                Msg::LauncherProgress(p) => {
                    if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                        *f = p;
                    }
                }
                Msg::LauncherUpdated(result) => {
                    let version = match &self.self_update {
                        SelfUpdate::Downloading(v, _) => v.clone(),
                        _ => String::new(),
                    };
                    self.self_update = match result {
                        Ok(()) => SelfUpdate::Ready(version),
                        Err(e) => SelfUpdate::Failed(e),
                    };
                }
                Msg::PackLine(line) => {
                    self.pack_log.push_back(line);
                    if self.pack_log.len() > LOG_LINES {
                        self.pack_log.pop_front();
                    }
                }
                Msg::PackDone(result) => self.pack_done(result),
                // Old versions log their session; it never reaches the window.
                Msg::Line(_, line) if line.contains("Session ID is") => {}
                Msg::Line(job, line) => {
                    if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                        if let Some(pressed) = self.play_pressed.take() {
                            self.perf
                                .record_since("launcher.play_to_game_frame_ms", pressed);
                        }
                    }
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        self.prepare_log.push_back(line);
                        if self.prepare_log.len() > LOG_LINES {
                            self.prepare_log.pop_front();
                        }
                    }
                }
                Msg::Exit(job, result) => {
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            // Play pressed before the version was prepared plays now.
                            if let Some(tag) = self.play_when_prepared.take() {
                                if error.is_none() && backend::is_prepared(&tag) {
                                    self.starting_pack = None;
                                    self.start_client(tag);
                                }
                            }
                            if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                                self.perf.record_since("launcher.prepare_ms", started);
                            }
                            // A preparer process's exit status says little, so its last line
                            // follows; the launcher's own errors are full sentences already.
                            self.prepare_error = error.map(|e| {
                                let tail = self
                                    .prepare_log
                                    .iter()
                                    .rev()
                                    .find(|l| !l.trim().is_empty())
                                    .filter(|_| e.starts_with("Exited with"));
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                        }
                        Job::Client => {
                            self.client = None;
                            self.come_back(cx);
                            // The game saved: last played, sizes and pictures changed.
                            self.worlds.stale();
                            self.multiplayer.stale();
                            self.screenshots.stale();
                            self.game_packs.stale();
                            let vars = std::mem::take(&mut self.client_vars);
                            if let Some((id, started)) = self.client_run.take() {
                                if let Err(e) = instances::record_play(&id, started.elapsed()) {
                                    eprintln!("Play time not saved: {e}");
                                }
                                if let Some(i) = instances::list().into_iter().find(|i| i.id == id)
                                {
                                    horizon_launcher::launch::run_after_exit(
                                        &i.launch, &i.home, &vars,
                                    );
                                }
                                self.reload_instances();
                            }
                            // The run's own game folder: an instance's or a pack's keeps its reports.
                            let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                            let report = self
                                .client_started
                                .take()
                                .and_then(|since| backend::crash_report_since(&home, since));
                            if error.is_some() && report.is_none() && self.switched_to_dx12() {
                                self.client_error = Some(
                                    "Horizon's Vulkan renderer crashed while starting on this \
                                     graphics card, so the game now uses DirectX 12. Press Play \
                                     to start it again. Video Settings > Graphics API switches \
                                     back."
                                        .into(),
                                );
                            } else if error.is_some() || report.is_some() {
                                let summary = report
                                    .as_ref()
                                    .and_then(|p| std::fs::read_to_string(p).ok())
                                    .and_then(|text| backend::crash_summary(&text))
                                    .or_else(|| {
                                        let tail = self
                                            .prepare_log
                                            .iter()
                                            .rev()
                                            .find(|l| !l.trim().is_empty());
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                self.crash = Some(Crash {
                                    report,
                                    summary,
                                    copied: false,
                                });
                            }
                        }
                        Job::Server => {}
                    }
                }
            }
        }
        changed |= self.servers_tick();
        changed |= self.mods_tick();
        if let Some(sign_in) = &self.sign_in {
            let state = sign_in.state();
            let changed_state = format!("{state:?}") != format!("{:?}", self.sign_in_state);
            if changed_state {
                if let SignInState::Done(Ok(_)) = state {
                    self.accounts = Accounts::load();
                    // Sign-in refuses accounts that don't own the game.
                    self.owner = Some(true);
                    self.owner_screen_open = false;
                    self.client_error = None;
                    if !self.settings.owner_notice_seen {
                        self.settings.owner_notice_seen = true;
                        self.settings.save();
                    }
                }
                self.sign_in_state = Some(state);
                changed = true;
            }
        }
        if self
            .checked
            .is_some_and(|at| at.elapsed() > backend::CHECK_EVERY)
        {
            self.check_updates();
        }
        if changed {
            cx.notify();
        }
    }

    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (#746), switches the game to DirectX 12
    /// (Windows only; only where the player left the graphics API on Auto). True when it did.
    fn switched_to_dx12(&self) -> bool {
        if !cfg!(windows) || !backend::vulkan_start_crashed(&self.prepare_log) {
            return false;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_to_dx12(&home) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!("Horizon launcher: could not switch to DirectX 12: {e}"),
            }
        }
        switched
    }

    /// The version Play starts: the selected instance's own, else the launcher's.
    fn selected(&self) -> Option<&String> {
        self.selected_instance()
            .and_then(|i| self.version_of(i))
            .or_else(|| self.launcher_version())
    }

    /// The launcher's version: what instances without one of their own play.
    fn launcher_version(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    fn install(&mut self, release: &Release) {
        if release.asset.is_none() || self.removing.as_ref() == Some(&release.tag) {
            return;
        }
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = backend::install(&release, progress);
            let _ = tx.send(Msg::Installed(tag, result));
        });
    }

    /// Deletes an installed version on a background thread: a version is thousands of files and
    /// the window must keep drawing meanwhile. `Msg::Removed` ends it.
    fn remove(&mut self, tag: &str) {
        self.confirm_remove = None;
        if self.removing.is_some()
            || self.installing.as_ref().is_some_and(|(t, _)| t == tag)
            || !self.installed.iter().any(|t| t == tag)
        {
            return;
        }
        self.install_error = None;
        self.removing = Some(tag.to_string());
        self.installed.retain(|t| t != tag);
        self.settings.selected =
            selection_after_removal(&self.installed, self.settings.selected.take());
        let (tag, tx) = (tag.to_string(), self.tx.clone());
        std::thread::spawn(move || {
            let result = backend::remove_version(&tag);
            let _ = tx.send(Msg::Removed(tag, result));
        });
    }

    /// Prepares the selected version, its Minecraft files from the player's install or, for an
    /// owner who asks, from Mojang.
    fn prepare(&mut self, from: backend::MinecraftFrom) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        horizon_launcher::mojang::CANCEL.store(false, Ordering::Relaxed);
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = backend::preparer(&tag, &minecraft, from, &log)
                .inspect(|(_, minecraft)| backend::sounds(&tag, minecraft, &log))
                .and_then(|(p, _)| match p {
                    backend::Preparer::Process(cmd) => {
                        backend::spawn(Job::Prepare, cmd, tx.clone()).map(|_running| false)
                    }
                    backend::Preparer::InProcess(options) => {
                        backend::prepare_in_process(&options, &log).map(|_| true)
                    }
                });
            match result {
                // A process sends its own Exit when it ends.
                Ok(false) => {}
                Ok(true) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
    }

    fn play(&mut self) {
        let java = self
            .selected_instance()
            .filter(|i| i.java.is_some())
            .cloned();
        if let Some(instance) = java {
            // Mojang's game: no Horizon version or preparation, only no game already running.
            // It joins a server; a friend's code and opening a world are Horizon's.
            self.joining = None;
            let server = self
                .quick_play
                .take()
                .filter(|(flag, _)| *flag == "--server")
                .map(|(_, ip)| ip);
            if self.client.is_some() || self.starting_client.is_some() {
                return;
            }
            self.starting_pack = None;
            self.start_java(instance, server);
            return;
        }
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        if !self.can_start(&tag) {
            return;
        }
        self.starting_pack = None;
        self.start_client(tag);
    }

    /// Starts a Java instance's Minecraft, joining `server` if given: its files and Java first
    /// (downloaded the first time, their lines in the log), then `drain` starts the game when
    /// the command arrives.
    fn start_java(&mut self, instance: instances::Instance, server: Option<String>) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        self.starting_client = Some(instance.id.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let log = |line: String| {
                let _ = tx.send(Msg::Line(Job::Client, line));
            };
            let cmd = backend::java_game(&instance, server.as_deref(), &log).map(|mut cmd| {
                let vars = instance.vars(cmd.get_program());
                cmd.envs(vars.iter().map(|(k, v)| (k, v)));
                (horizon_launcher::launch::wrap(cmd, &instance.launch), vars)
            });
            let _ = tx.send(Msg::JavaGame(instance.id, instance.home, cmd));
        });
    }

    /// Whether `tag` can start now ([`starts_now`]): prepared, no preparation under way, no game
    /// running or starting.
    pub(crate) fn can_start(&self, tag: &str) -> bool {
        let running = self.client.is_some() || self.starting_client.is_some();
        starts_now(backend::is_prepared(tag), self.preparing, running)
    }

    /// Whether the selected instance's version can start now, from a shortcut (a server's Join,
    /// a world's Play, a friend's code): Horizon's only, never a Java instance.
    pub(crate) fn can_start_selected(&self) -> bool {
        self.selected_instance().is_none_or(|i| i.java.is_none())
            && self.selected().is_some_and(|tag| self.can_start(tag))
    }

    /// Whether a server's Join starts the selected instance now: a Java instance too, as Play
    /// would (no game running or starting).
    pub(crate) fn can_join_selected(&self) -> bool {
        match self.selected_instance() {
            Some(i) if i.java.is_some() => self.client.is_none() && self.starting_client.is_none(),
            _ => self.can_start_selected(),
        }
    }

    /// Starts the game: the session first (a network round trip when the token needs
    /// renewing), then `drain` starts the process when it arrives.
    fn start_client(&mut self, tag: String) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // What the resources lack (built-in packs, panorama, sounds) and the session (a network
        // round trip when the token needs renewing) come first; `drain` starts the game when the
        // session arrives.
        self.starting_client = Some(tag.clone());
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        let offline_without_owner = self.settings.dev_offline;
        std::thread::spawn(move || {
            backend::complete_assets(&tag);
            let session = account::session_for_game(&backend::game_home(), offline_without_owner);
            let _ = tx.send(Msg::Session(session));
        });
    }

    /// Starts the Microsoft sign-in; Settings and the owner gate's screen show its code.
    fn start_sign_in(&mut self) {
        if let Some(old) = self.sign_in.replace(SignIn::start()) {
            old.cancel();
        }
        self.sign_in_state = Some(SignInState::Starting);
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.settings.save();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

// ----- Building blocks ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// The element under the pointer and the one it just left, with when that happened. Hover
/// backgrounds ease in and out from these rather than snapping.
struct HoverState {
    on: Option<(SharedString, Instant)>,
    off: Option<(SharedString, Instant)>,
}

static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    on: None,
    off: None,
});

fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    el.on_hover(move |hovered, window, _| {
        let mut state = HOVER.lock().unwrap();
        let is_on = state.on.as_ref().is_some_and(|(h, _)| *h == id);
        if *hovered && !is_on {
            state.off = state.on.take();
            state.on = Some((id.clone(), Instant::now()));
        } else if !*hovered && is_on {
            state.off = Some((id.clone(), Instant::now()));
            state.on = None;
        }
        window.refresh();
    })
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-bleed))
        .right(px(-bleed))
        .rounded(px(radius));
    let state = HOVER.lock().unwrap();
    // Each transition gets its own animation id, from when it started, so it plays once.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(format!("{id}-{dir}"), (at - start).as_micros() as usize)
    };
    let ease = || Animation::new(theme::QUICK).with_easing(ease_out_quint());
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return layer
            .with_animation(key("in", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * t))
            })
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < theme::QUICK)
    {
        return layer
            .with_animation(key("out", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * (1. - t)))
            })
            .into_any_element();
    }
    layer.into_any_element()
}

/// Text set in one step of the type scale: headings (heading, title, display) in the pixel face.
fn text(step: theme::Step) -> Div {
    let d = div().text_size(px(step.px)).line_height(px(step.line_px()));
    let d = if step.mono {
        d.font_family(theme::MONO)
    } else if step.px >= theme::HEADING.px {
        d.font_family(theme::PIXEL)
    } else {
        d
    };
    if step.weight >= 500 {
        d.font_weight(FontWeight::MEDIUM)
    } else {
        d
    }
}

/// A release's tag as a title: mono, so its digits read at a glance.
fn version_tag(tag: &str) -> Div {
    div()
        .font_family(theme::MONO)
        .text_size(px(22.))
        .line_height(px(28.))
        .font_weight(FontWeight::MEDIUM)
        .child(
            horizon_launcher::nightly::label(tag)
                .unwrap_or_else(|| tag.trim_start_matches('v').to_string()),
        )
}

/// Horizon's mark: two stems, their reflection below, and the horizon across in the accent.
fn mark(size: f32, t: &Theme) -> Div {
    let u = size / 16.;
    let stem = |x: f32, y: f32, color: Hsla| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(2. * u))
            .h(px(4. * u))
            .bg(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(stem(4., 3., t.ink))
        .child(stem(10., 3., t.ink))
        .child(stem(4., 8., t.ink.opacity(0.36)))
        .child(stem(10., 8., t.ink.opacity(0.36)))
        .child(
            div()
                .absolute()
                .left(px(u))
                .top(px(7. * u))
                .w(px(14. * u))
                .h(px(u))
                .bg(t.accent),
        )
}

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
static KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

/// A button: a block with a light bevel on top and a darker lip below, its label in the pixel
/// face. Primary (the one main action) is the violet face; Secondary a raised block; Ghost only
/// its label until the pointer is over it.
fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    let label = SharedString::from(tr(&label).to_string());
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_face.opacity(0.10),
        Kind::Secondary => t.bevel.opacity(if t.dark { 0.10 } else { 0.35 }),
        Kind::Ghost => t.raised,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(14.))
        .font_family(theme::PIXEL)
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, 0., 0.));
    let base = match kind {
        Kind::Primary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.face_bevel, t.face_lip, theme::LIP))
            .bg(t.face)
            .text_color(t.on_face),
        Kind::Secondary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
            .bg(t.button)
            .text_color(t.ink),
        Kind::Ghost => base.text_color(t.muted).hover(move |s| s.text_color(t.ink)),
    };
    // The label sits on the block's face, above its lip; pressing pushes it down.
    let base = base.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(if kind == Kind::Ghost { 0. } else { 2. }))
            .child(label),
    );
    keyboard(track_hover(base, id), &t).active(|s| s.opacity(theme::PRESSED).top(px(1.)))
}

/// Copies to the clipboard, or into `HORIZON_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`horizon_launcher::desktop_seams`).
fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    if let Some(file) = horizon_launcher::desktop_seams::clipboard_file() {
        let bytes = match item.entries().first() {
            Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
            _ => item.text().unwrap_or_default().into_bytes(),
        };
        if let Err(e) = std::fs::write(&file, bytes) {
            eprintln!("{}: {e}", file.display());
        }
        return;
    }
    cx.write_to_clipboard(item.clone());
    #[cfg(target_os = "linux")]
    if let Some(text) = item.text() {
        sync_system_clipboard(text);
    }
}

/// GPUI's Wayland clipboard sends the last key press's serial, which a compositor ignores
/// after a mouse click: hand the text to the desktop's own tool too, the first one present
/// for this session (`wl-copy` on Wayland, `xclip` or `xsel` on X11).
#[cfg(target_os = "linux")]
fn sync_system_clipboard(text: String) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    const TOOLS: [(&str, &str, &[&str]); 3] = [
        ("WAYLAND_DISPLAY", "wl-copy", &[]),
        ("DISPLAY", "xclip", &["-selection", "clipboard"]),
        ("DISPLAY", "xsel", &["--clipboard", "--input"]),
    ];
    if text.is_empty() {
        return;
    }
    // Off the UI thread: a tool can take its time.
    std::thread::spawn(move || {
        for (session, tool, args) in TOOLS {
            if std::env::var_os(session).is_none() {
                continue;
            }
            let Ok(mut child) = Command::new(tool)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|status| status.success()) {
                return;
            }
        }
    });
}

/// An on/off switch: a square knob sliding in a sunk track that turns violet when on.
fn switch(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let id: SharedString = id.into();
    let knob = if on { t.on_face } else { t.muted };
    keyboard(div().id(id.clone()), &t)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .border_2()
        .border_color(t.outline)
        .cursor_pointer()
        .bg(if on { t.face } else { t.well })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .bg(knob)
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(if on { t.face_lip } else { t.lip }),
                )
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 20. * p } else { 22. - 20. * p })),
                ),
        )
}

/// A section's title inside a page: a pixel icon in the accent and the name in the pixel face.
fn section_title(label: &'static str, icon: &'static str, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(blocks::icon(icon, 20., t.accent))
        .child(
            blocks::pixel(18., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(label)),
        )
}

/// A segmented control: a sunk track, the chosen segment a raised block.
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let t = *t;
    div()
        .id(id)
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .children(options.iter().map(|&(value, label)| {
            let active = current == value;
            let pick = on_pick.clone();
            keyboard(div().id(SharedString::from(format!("{id}-{label}"))), &t)
                .relative()
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .pb(px(2.))
                .font_family(theme::PIXEL)
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.button).text_color(t.ink).children(blocks::edges(
                        t.button_bevel,
                        t.lip,
                        3.,
                    ))
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.ink))
                })
                .child(div().relative().child(tr(label)))
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
        }))
}

/// What the Play button does for the selected version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayPress {
    Play,
    /// Not prepared, for an account that owns the game: prepare it (from the player's install,
    /// else downloaded from Mojang for that account), then play, as the official launcher does.
    PrepareThenPlay,
    /// Not prepared and no owner (signing in comes first), or busy preparing or playing.
    Nothing,
}

/// `owner`: whether an account here owns the game, by the keychain (`Launcher::owner`).
fn play_press(prepared: bool, owner: Option<bool>, busy: bool) -> PlayPress {
    match (prepared, owner) {
        _ if busy => PlayPress::Nothing,
        (true, _) => PlayPress::Play,
        (false, Some(true)) => PlayPress::PrepareThenPlay,
        (false, _) => PlayPress::Nothing,
    }
}

/// Whether a shortcut that starts the game at once (a server's Join, a world's Play, a friend's
/// code, a pack's Play) starts it now: only when Play would play. Never while a preparation runs:
/// the game would open on half-written files.
fn starts_now(prepared: bool, preparing: bool, running: bool) -> bool {
    play_press(prepared, None, preparing || running) == PlayPress::Play
}

/// The selected version once one has been removed: the saved choice if it is still installed,
/// else the newest left.
fn selection_after_removal(installed: &[String], saved: Option<String>) -> Option<String> {
    saved
        .filter(|s| installed.contains(s))
        .or_else(|| installed.first().cloned())
}

fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

/// A page's title in the pixel face and one line on what it is for.
fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            blocks::pixel(36., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(title).to_string()),
        )
        .child(
            text(theme::UI)
                .max_w(px(520.))
                .text_color(t.muted)
                .child(tr(description).to_string()),
        )
}

/// One row of a list, as its own panel: a name in the pixel face with an optional detail under
/// it, then a value or a control on the right. Rows were numbered in the quiet look; `_n` keeps
/// the call sites as they are.
/// How an available update is named: a Nightly by its title ("Nightly 2026-10-28 · 0c02067"), a
/// release by its tag.
fn update_name(release: &Release) -> &str {
    match release.nightly {
        Some(_) => &release.name,
        None => &release.tag,
    }
}

fn index_row(
    _n: usize,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    slot_row(None, name, detail, right, t)
}

/// A row with an icon slot (`blocks::slot`) on its left: a project in the Mods page's lists.
fn slot_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    blocks::panel(t)
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(theme::ROW))
        .mb(px(8.))
        .px(px(16.))
        .py(px(12.))
        .children(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .text_color(t.ink)
                        .child(name),
                )
                .children(detail.map(|d| {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(d)
                        .into_any_element()
                })),
        )
        .children(right)
}

/// A plate that stands apart from the page: notices, the console, the agents note.
fn plate(t: &Theme) -> Div {
    blocks::panel(t)
}

/// Something went wrong: a dot in the danger colour, then what happened.
fn notice(text_: String, t: &Theme) -> Div {
    plate(t)
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .text_size(px(theme::UI.px))
        .line_height(px(21.))
        .text_color(t.ink)
        .border_l(px(6.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .bottom(px(-2.))
                .left(px(-6.))
                .w(px(6.))
                .bg(t.danger),
        )
        .child(blocks::icon("info-box", 20., t.danger))
        .child(div().flex_1().min_w_0().child(text_))
}

/// An empty state: a short sentence, and the one thing to do about it.
fn centered(text_: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(16.))
        .py(px(48.))
        .child(text(theme::UI).text_color(t.muted).child(text_.to_string()))
}

/// The name the game made up for a player without an account, once it has run.
fn offline_name() -> Option<String> {
    player_name(&std::fs::read_to_string(backend::game_home().join("horizon-player.txt")).ok()?)
}

/// The name in `horizon-player.txt`: the game writes `name:Player12345` (horizon-client's
/// offline_name.rs).
fn player_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("name:"))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
}

/// `1234` as `1,234`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A mono line in upper case: the status above a title, the footer's links.
fn caps(s: impl AsRef<str>) -> Div {
    text(theme::MICRO).child(s.as_ref().to_uppercase())
}

fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

// ----- Pages -------------------------------------------------------------------------------------

impl Launcher {
    /// The top bar: the mark and name over the sidebar (in the title bar on Windows and Linux
    /// client-side decorations), the pages with their icons, then the account and Settings.
    fn topbar(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut nav = div().flex().flex_none().h_full().gap(px(2.));
        for (page, label, icon) in Page::ALL {
            if page == Page::Settings {
                continue;
            }
            let active = self.page == page;
            let id = SharedString::from(format!("nav-{label}"));
            let item = div()
                .id(id.clone())
                .relative()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(14.))
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.raised).child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(4.))
                            .bg(t.accent),
                    )
                })
                .when(!active, |d| d.child(hover_layer(&id, t.raised, 0., 0.)))
                .child(div().relative().child(blocks::icon(
                    icon,
                    18.,
                    if active { t.accent } else { t.muted },
                )))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .relative()
                        .text_color(if active { t.ink } else { t.muted })
                        .child(tr(label)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(page);
                    cx.notify();
                }));
            nav = nav.child(keyboard(track_hover(item, id), &t));
        }
        // Only a signed-in account says "Microsoft account"; otherwise an offline name, or the
        // one the game made up.
        let (name, how) = self.who_plays();
        let account = keyboard(div().id("account"), &t)
            .relative()
            .flex()
            .min_w_0()
            .max_w(px(180.))
            .items_center()
            .gap(px(10.))
            .h(px(40.))
            .pl(px(4.))
            .pr(px(12.))
            .bg(t.panel)
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(30.))
                    .bg(t.well)
                    .child(blocks::icon("user", 18., t.muted)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        blocks::pixel(14., FontWeight::MEDIUM)
                            .truncate()
                            .text_color(t.ink)
                            .child(name),
                    )
                    .child(text(theme::CAPTION).text_color(t.muted).child(how)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.account_menu = !this.account_menu;
                if this.account_menu {
                    // Who can play may have changed elsewhere since the window opened.
                    this.accounts = Accounts::load();
                }
                cx.notify();
            }));
        let account = div()
            .relative()
            .child(account)
            .children(self.account_menu(cx));
        let settings_open = self.page == Page::Settings;
        let settings = keyboard(div().id("nav-Settings"), &t)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.))
            .bg(if settings_open { t.raised } else { t.panel })
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .when(settings_open, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(4.))
                        .bg(t.accent),
                )
            })
            .child(blocks::icon(
                "sliders",
                20.,
                if settings_open { t.accent } else { t.ink_soft },
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Settings);
                cx.notify();
            }));
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(theme::TOPBAR))
            .pl(px(18.))
            .pr(px(12.))
            .bg(t.side)
            .border_b_2()
            .border_color(t.outline)
            // Custom title bars carry the mark and name above the sidebar.
            .when(!titlebar::draws_brand(window), |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .w(px(theme::SIDEBAR - 18.))
                        .child(mark(24., &t))
                        .child(
                            blocks::pixel(20., FontWeight::SEMIBOLD)
                                .text_color(t.ink)
                                .child("HORIZON"),
                        ),
                )
            })
            .child(nav)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .child(account)
                    .child(settings),
            )
    }

    /// The sidebar: Horizon and the imported modpacks, the player's servers, then the
    /// launcher's own update, the community links and its version.
    fn sidebar(&mut self, _window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        self.load_packs();
        let heading = |label: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(6.))
                .pt(px(10.))
                .pb(px(6.))
                .child(
                    blocks::pixel(13., FontWeight::MEDIUM)
                        .text_color(t.muted)
                        .child(label),
                )
        };
        let sub = |s: String| text(theme::CAPTION).text_color(t.muted).truncate().child(s);
        let title = |s: String| {
            blocks::pixel(15., FontWeight::MEDIUM)
                .text_color(t.ink)
                .truncate()
                .child(s)
        };
        let dot = |c: Hsla| {
            div()
                .absolute()
                .left(px(42.))
                .top(px(42.))
                .size(px(10.))
                .bg(c)
                .border_2()
                .border_color(t.side)
        };
        let add = keyboard(div().id("add-instance"), &t)
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .bg(t.panel)
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .child(blocks::icon("plus", 14., t.ink_soft))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_instance_dialog(None);
                cx.notify();
            }));
        let items = self.instance_items(heading, cx);
        let list = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(tr("INSTANCES").into()).child(add))
            .children(items);
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        let mut list = list;
        if !servers.is_empty() {
            list = list.child(heading(tr("SERVERS").into()));
        }
        for e in servers {
            let up = self.servers.is_up(&e.dir);
            let open = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            list = list.child(
                blocks::side_item(format!("side-server-{}", e.dir.display()).into(), open, &t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(44.))
                            .bg(t.well)
                            .border_2()
                            .border_color(t.outline)
                            .child(blocks::icon("server", 22., t.ink_soft)),
                    )
                    .child(dot(if up { t.success } else { t.muted }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(title(e.name.clone()))
                            .child(sub(tr(if up { "Running" } else { "Stopped" }).into())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let link = |id: &'static str, label: &'static str, icon: &'static str, url: String| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(8.))
                .py(px(6.))
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(t.ink).bg(t.raised.opacity(0.6)))
                .child(blocks::icon(icon, 18., t.muted))
                .child(blocks::pixel(14., FontWeight::MEDIUM).child(label))
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        let mut foot = div().mt_auto().flex().flex_col().gap(px(2.)).pt(px(12.));
        match &self.self_update {
            SelfUpdate::Downloading(version, f) => {
                foot = foot.child(
                    blocks::panel(&t)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(10.))
                        .mb(px(6.))
                        .child(
                            blocks::pixel(13., FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .child(format!("Launcher {version}  {:.0}%", f * 100.)),
                        )
                        .child(div().flex().child(progress_bar(Some((*f, *f)), &t))),
                )
            }
            SelfUpdate::Ready(version) => {
                foot = foot.child(
                    blocks::panel(&t)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(10.))
                        .mb(px(6.))
                        .child(
                            blocks::pixel(13., FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .child(format!("Launcher updated to {version}")),
                        )
                        .child(button("restart", "Restart", Kind::Primary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.self_update = SelfUpdate::Failed(backend::restart());
                                cx.notify();
                            }),
                        )),
                )
            }
            _ => {}
        }
        if !self.links.sponsor.is_empty() {
            foot = foot.child(link(
                "sponsor",
                tr("Sponsor"),
                "heart",
                self.links.sponsor.clone(),
            ));
        }
        if !self.links.discord.is_empty() {
            foot = foot.child(link(
                "discord",
                "Discord",
                "message",
                self.links.discord.clone(),
            ));
        }
        if !self.links.website.is_empty() {
            foot = foot.child(link(
                "website",
                tr("Website"),
                "globe",
                self.links.website.clone(),
            ));
        }
        foot = foot.child(
            caps(format!("Launcher {}", backend::VERSION))
                .id("launcher-version")
                .px(px(8.))
                .pt(px(6.))
                .text_color(t.muted)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.version_clicked();
                    cx.notify();
                })),
        );
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR))
            .h_full()
            .px(px(10.))
            .pb(px(12.))
            .bg(t.side)
            .border_r_2()
            .border_color(t.outline)
            .overflow_y_scroll()
            .child(list)
            .child(foot)
    }

    fn picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let label = self
            .selected()
            .cloned()
            .unwrap_or_else(|| "No version".into());
        // A block as tall as Play: "VERSION" over the tag, and a chevron.
        let mut wrap = div().relative().child(
            keyboard(div().id("picker"), &t)
                .relative()
                .flex()
                .items_center()
                .gap(px(16.))
                .h(px(60.))
                .pl(px(14.))
                .pr(px(12.))
                .bg(t.panel)
                .border_2()
                .border_color(t.outline)
                .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                .cursor_pointer()
                .hover(move |s| s.bg(t.raised))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            blocks::pixel(11., FontWeight::MEDIUM)
                                .text_color(t.muted)
                                .child("VERSION"),
                        )
                        .child(
                            div()
                                .font_family(theme::MONO)
                                .text_size(px(15.))
                                .text_color(t.ink)
                                .child(label),
                        ),
                )
                .child(blocks::icon("chevron-down", 20., t.muted))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                })),
        );
        if self.picker_open {
            let mut menu = div()
                .absolute()
                .top(px(66.))
                .right_0()
                .w(px(300.))
                .p(px(6.))
                .border_2()
                .border_color(t.outline)
                .bg(t.surface)
                .flex()
                .flex_col();
            // Picking a version keeps the selected instance on it; the launcher's own version
            // (newest unless chosen otherwise on Versions) follows updates.
            let follows = self.launcher_version().cloned();
            for tag in &self.installed {
                let selected = self.selected() == Some(tag);
                let tag = tag.clone();
                let latest = follows.as_ref() == Some(&tag);
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("pick-{tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::MONO)
                        .text_size(px(theme::CAPTION.px))
                        .text_color(t.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(tag.clone())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .when(latest, |d| {
                                    d.child(caps("follows updates").text_color(t.muted))
                                })
                                .when(selected, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                                }),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pin_version((!latest).then(|| tag.clone()));
                            this.picker_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .text_size(px(theme::LABEL.px))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                    .child("Manage versions")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        this.picker_open = false;
                        cx.notify();
                    })),
            );
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        wrap
    }

    fn play_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // A Java instance plays Mojang's game: no Horizon version, nothing of Horizon's to prepare.
        let java = self.selected_instance().and_then(|i| i.java.clone());
        let Some(tag) = self
            .selected()
            .cloned()
            .or_else(|| java.is_some().then(String::new))
        else {
            // No hero without a version: the page starts below the top bar like the others.
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .pt(px(28.))
                .child(page_header(
                    "Play",
                    "Start the game, or run a server for others to join.",
                    &t,
                ))
                .child(centered(tr("No version installed yet."), &t).child(
                    button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Versions);
                            cx.notify();
                        }),
                    ),
                ));
        };
        let prepared = backend::is_prepared(&tag);
        // Prepared without sounds (#1035): Prepare again is the fix, so it stands out.
        let silent = prepared && !backend::has_sounds(&tag);
        let version = java
            .as_ref()
            .map_or_else(|| backend::game_version(&tag), |j| j.minecraft.clone());
        let hero = match &java {
            // Its own game: the Minecraft version and the loader, never Horizon's.
            Some(j) => {
                let game = match &j.loader {
                    Some(l) => format!("{version} · {} {}", l.kind.name(), l.version),
                    None => version.clone(),
                };
                self.hero("", &game, true, cx)
            }
            None => self.hero(&tag, &version, prepared, cx),
        };
        let running = self.client.is_some() || self.starting_client.is_some();
        let play_label = if running { "Running" } else { "Play" };
        // The label clears the icon in every language ("Играть" is wider than "Play").
        let mut play = button("play", play_label, Kind::Primary, &t)
            .min_w(px(200.))
            .h(px(60.))
            .pl(px(60.))
            .pr(px(28.))
            .text_size(px(28.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                div()
                    .absolute()
                    .left(px(26.))
                    .top(px(17.))
                    .child(blocks::icon("play", 22., t.on_face)),
            );
        match play_press(
            prepared || java.is_some(),
            self.owner,
            self.preparing || running,
        ) {
            PlayPress::Nothing => play = disabled(play),
            PlayPress::Play => {
                play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = false;
                    this.play();
                    cx.notify();
                }))
            }
            PlayPress::PrepareThenPlay => {
                play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = false;
                    this.play_when_prepared = this.selected().cloned();
                    this.prepare(backend::MinecraftFrom::Install);
                    cx.notify();
                }))
            }
        }
        let actions = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                button("host", "Host a server", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.go(Page::Servers);
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    // Horizon's versions don't apply to a Java instance.
                    .children(java.is_none().then(|| self.picker(cx)))
                    .child(play),
            );

        // Not prepared: Horizon can download Minecraft from Mojang (no account needed in this
        // fork), or use the player's own install.
        let first = (!prepared && !self.preparing)
            .then_some(self.owner_shown())
            .flatten();
        // No launcher folder with Minecraft on this computer: an owner needs none.
        let no_install = self.settings.minecraft_dir.is_none();
        let resources_value = match first {
            _ if self.preparing => tr("Preparing…").to_string(),
            Some(_) if no_install => trf(
                "Not prepared. Play downloads Minecraft {version} from Mojang and prepares it.",
                &[("version", &version)],
            ),
            Some(_) => trf(
                "Not prepared. Horizon can download Minecraft {version} from Mojang, or use your own install.",
                &[("version", &version)],
            ),
            None => backend::resources_summary(prepared, !silent, &version),
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared && !silent {
                Kind::Ghost
            } else {
                Kind::Secondary
            },
            &t,
        );
        if self.preparing {
            prepare = disabled(prepare);
        } else {
            prepare = prepare.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.prepare(backend::MinecraftFrom::Install);
                cx.notify();
            }));
        }
        let prepare = match first {
            Some(_) if no_install => div().child(
                button(
                    "prepare-download",
                    "Download and prepare",
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.prepare(backend::MinecraftFrom::Mojang);
                    cx.notify();
                })),
            ),
            Some(_) => div()
                .flex()
                .gap(px(8.))
                .child(
                    button("prepare-install", "From my install", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.prepare(backend::MinecraftFrom::Install);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(
                        "prepare-download",
                        "Download and prepare",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.prepare(backend::MinecraftFrom::Mojang);
                        cx.notify();
                    })),
                ),
            None => div().child(prepare),
        };
        let owner = self.owner == Some(true);
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                div()
                    .child(if owner {
                        tr("None needed: Horizon downloads Minecraft from Mojang for your account.")
                    } else {
                        tr("Not found. Choose the folder where your launcher keeps Minecraft.")
                    })
                    .into_any_element()
            },
            |p| {
                div()
                    .font_family(theme::MONO)
                    .child(short(p))
                    .into_any_element()
            },
        );
        let mut rows = div()
            .flex()
            .flex_col()
            .child(section_title("Ready to play", "check", &t).mb(px(12.)))
            .children(self.instance_row(1, cx))
            .children(self.export_panel(cx))
            .children(self.launch_panel(cx));
        rows = match &java {
            Some(java) => rows.child(index_row(
                2,
                tr("Game"),
                Some(
                    div()
                        .child(trf(
                            "Minecraft {version} (Java Edition). Play downloads it from Mojang for your account the first time, with the Java it needs.",
                            &[("version", &java.minecraft)],
                        ))
                        .into_any_element(),
                ),
                None,
                &t,
            )),
            None => rows
                .child(index_row(
                    2,
                    tr("Resources"),
                    Some(div().child(resources_value).into_any_element()),
                    Some(prepare.into_any_element()),
                    &t,
                ))
                .child(index_row(
                3,
                tr("Minecraft folder"),
                Some(minecraft),
                Some(
                    button("choose-mc", "Choose…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                        .into_any_element(),
                ),
                &t,
            )),
        };
        {
            if java.is_none() {
                rows = rows.child(self.join_friend_row(self.can_start(&tag), cx));
            }
            // The top bar shows who plays; a row only when there is something to do about it:
            // no account that owns the game (Play needs one), or offline.
            if self.owner_shown() == Some(false) {
                rows = rows.child(index_row(
                    5,
                    tr("Microsoft account"),
                    Some(
                        div()
                            .child(format!("{}.", tr(account::NEEDS_OWNER)))
                            .into_any_element(),
                    ),
                    Some(
                        button("gate-sign-in", "Sign in", Kind::Primary, &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.sign_in_to_play();
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            } else if self.accounts.selected().is_none() && account::client_id().is_some() {
                rows = rows.child(index_row(
                    5,
                    tr("Playing offline"),
                    Some(
                        div()
                            .child(tr("Online servers need a Microsoft account."))
                            .into_any_element(),
                    ),
                    Some(
                        button("accounts", "Accounts…", Kind::Ghost, &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.go(Page::Settings);
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            }
        }

        // The banner: the hero and the Play row over the sky (`sky.rs`), or over the player's
        // own picture under a veil of the page colour. It reaches the content's edges and the top
        // bar.
        let veil = |angle: f32, from: f32, to: f32, at: f32| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(gpui::linear_gradient(
                    angle,
                    gpui::linear_color_stop(t.page.opacity(from), 0.),
                    gpui::linear_color_stop(t.page.opacity(to), at),
                ))
        };
        let picture = self.home_background().map(|path| {
            [
                gpui::img(path)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover)
                    .into_any_element(),
                veil(90., 0.82, 0., 0.62).into_any_element(),
                veil(180., 0., 0.75, 1.).into_any_element(),
            ]
        });
        let banner = div()
            .relative()
            .flex()
            .flex_col()
            .justify_end()
            .min_h(px(340.))
            .mx(px(-theme::PAGE_GUTTER))
            .px(px(theme::PAGE_GUTTER))
            .pb(px(28.))
            .gap(px(24.))
            .overflow_hidden()
            .children(picture.into_iter().flatten())
            .child(div().relative().child(hero))
            .child(div().relative().child(actions));
        let mut page = div().flex().flex_col().gap(px(28.)).child(banner);
        // The preparation's progress and error right under the buttons, where the player
        // looks after pressing Prepare, not below the fold (#666).
        if self.preparing {
            let last = self.prepare_log.back().cloned().unwrap_or_default();
            // A download from Mojang shows how far it is, and can stop (it goes on from there
            // next time).
            let download = horizon_launcher::mojang::progress();
            let cancel = download.map(|_| {
                button("download-cancel", "Cancel", Kind::Ghost, &t).on_click(|_, _, _| {
                    horizon_launcher::mojang::CANCEL.store(true, Ordering::Relaxed)
                })
            });
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(progress_bar(
                                download.map(|(done, total)| {
                                    let f = done as f32 / total as f32;
                                    (f, f)
                                }),
                                &t,
                            ))
                            .children(cancel),
                    )
                    .child(
                        text(theme::MICRO)
                            .text_color(t.muted)
                            .truncate()
                            .child(last),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(tr("The first preparation takes a few minutes.")),
                    ),
            );
        }
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        page = page.child(rows);
        if let Some(console) = self.console_section(cx) {
            page = page.child(console);
        }
        if let Some(worlds) = self.worlds_section(cx) {
            page = page.child(worlds);
        }
        if let Some(servers) = self.multiplayer_section(cx) {
            page = page.child(servers);
        }
        if let Some(packs) = self.packs_section(cx) {
            page = page.child(packs);
        }
        if let Some(shots) = self.screenshots_section(cx) {
            page = page.child(shots);
        }
        if let Some(e) = self.client_error.clone() {
            page = page.child(notice(e, &t));
        }
        if let Some(crash) = self.crash.clone() {
            page = page.child(self.crash_notice(crash, cx));
        }
        page
    }

    /// "Horizon crashed": the report and the logs, and where to report it.
    fn crash_notice(&self, crash: Crash, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut actions = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
        if let Some(report) = crash.report.clone() {
            let open = report.clone();
            actions = actions
                .child(
                    button("crash-open", "Open report", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click(move |_, _, cx| cx.open_with_system(&open)),
                )
                .child(
                    button(
                        "crash-copy",
                        if crash.copied {
                            "Copied"
                        } else {
                            "Copy report"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .bg(t.plate)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Ok(text) = std::fs::read_to_string(&report) {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text));
                                if let Some(c) = &mut this.crash {
                                    c.copied = true;
                                }
                            }
                            cx.notify();
                        },
                    )),
                );
        }
        actions = actions.child(
            button("crash-logs", "Open logs folder", Kind::Secondary, &t)
                .bg(t.plate)
                .on_click(|_, _, cx| {
                    let dir = backend::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                }),
        );
        let discord = self.links.discord.clone();
        if !discord.is_empty() {
            actions = actions.child(
                button("crash-discord", "Report on Discord", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(move |_, _, cx| cx.open_url(&discord)),
            );
        }
        actions = actions.child(
            button("crash-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.crash = None;
                    cx.notify();
                },
            )),
        );
        let help = tr(if crash.report.is_some() {
            "When you report it on Discord, attach the crash report file (or paste it with Copy report) and say what you were doing."
        } else {
            "The game ended without a crash report. When you report it on Discord, attach latest.log from the logs folder and say what you were doing."
        });
        let mut body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Horizon crashed")),
            );
        if !crash.summary.is_empty() {
            body = body.child(div().text_color(t.ink_soft).child(crash.summary));
        }
        if let Some(report) = &crash.report {
            body = body.child(
                text(theme::MICRO)
                    .text_color(t.muted)
                    .truncate()
                    .child(short(report)),
            );
        }
        body = body.child(actions).child(
            text(theme::CAPTION)
                .mt(px(4.))
                .text_color(t.muted)
                .child(help),
        );
        plate(&t)
            .flex()
            .gap(px(14.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .text_color(t.ink)
            .child(
                div()
                    .flex_none()
                    .mt(px(11.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(t.danger),
            )
            .child(body)
    }

    /// The Play page's focal point: what will start, and a line from its notes. No `tag`: a Java
    /// instance's Minecraft.
    fn hero(&self, tag: &str, version: &str, prepared: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let summary = match &self.releases {
            Load::Ready(list) => list
                .iter()
                .find(|r| r.tag == tag)
                .and_then(|r| backend::notes_summary(&r.body)),
            _ => None,
        };
        let title = if tag.is_empty() {
            "Minecraft"
        } else {
            "Horizon"
        };
        // Never squeezed by the rows below: the ping notice can make it tall.
        let mut hero = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .py(px(5.))
                            .bg(t.side.opacity(0.85))
                            .border_2()
                            .border_color(t.outline)
                            .child(
                                div()
                                    .size(px(8.))
                                    .bg(if prepared { t.success } else { t.warn }),
                            )
                            .child(caps(format!("Minecraft {version}")).text_color(t.ink_soft))
                            .child(div().size(px(4.)).bg(t.muted))
                            .child(
                                caps(tr(if prepared { "Ready" } else { "Needs preparing" }))
                                    .text_color(if prepared { t.success } else { t.warn }),
                            ),
                    )
                    .children(self.playing.map(|n| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(div().size(px(4.)).rounded_full().bg(t.rule))
                            .child(
                                caps(format!("{} playing now", thousands(n))).text_color(t.muted),
                            )
                    })),
            )
            .child(
                blocks::pixel(60., FontWeight::BOLD)
                    .line_height(px(62.))
                    .text_color(t.ink)
                    .child(title),
            )
            // The version is in the picker beside Play: here only what it brings.
            .children(summary.map(|line| {
                text(theme::BODY)
                    .max_w(px(560.))
                    .text_color(t.ink_soft)
                    .child(line)
            }));
        // A Java instance (no Horizon tag) isn't offered Horizon's updates.
        if let Some(release) = self.updates.horizon.as_ref().filter(|_| !tag.is_empty()) {
            let installing = self.installing.as_ref().filter(|(t, _)| *t == release.tag);
            let line = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(theme::CAPTION.px))
                .text_color(t.muted)
                .child(div().size(px(6.)).rounded_full().bg(t.accent))
                .child(format!("{} available", update_name(release)));
            hero = hero.child(match installing {
                Some((_, p)) => line
                    .child(
                        div()
                            .w(px(120.))
                            .flex()
                            .child(progress_bar(Some(self.bar), &t)),
                    )
                    .child(format!("{:.0}%", p * 100.)),
                None => line.child(
                    div()
                        .id("install-update")
                        .text_color(t.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .child("Install")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.install_update();
                            cx.notify();
                        })),
                ),
            });
        }
        // Players who answered the first-run question before the ping existed read about it
        // once here.
        if !self.settings.ping_notice_seen && !self.collection.consent.needs_asking() {
            hero = hero.child(
                plate(&t)
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .w_full()
                    .max_w(px(640.))
                    .child(
                        text(theme::UI)
                            .text_color(t.ink)
                            .child(telemetry::PING_NAME),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(telemetry::PING),
                    )
                    .child(
                        div()
                            .id("ping-notice-ok")
                            .text_size(px(theme::UI.px))
                            .text_color(t.accent)
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .child("Got it")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.settings.ping_notice_seen = true;
                                this.settings.save();
                                cx.notify();
                            })),
                    ),
            );
        }
        hero
    }

    fn settings_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let toggle = switch("auto-install", self.settings.auto_install(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.install_new_versions = Some(!this.settings.auto_install());
                this.settings.save();
                if this.settings.auto_install() {
                    this.install_update();
                }
                cx.notify();
            }),
        );
        let prune = switch("auto-prune", self.settings.auto_prune(), &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.prune_old_versions = Some(!this.settings.auto_prune());
                this.settings.save();
                cx.notify();
            },
        ));
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.settings.save();
                cx.notify();
            },
        ));
        // Linux: the launcher's entry in the applications menu (`horizon_launcher::desktop`).
        let menu_entry = switch("desktop-entry", self.settings.desktop_entry(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                let on = !this.settings.desktop_entry();
                this.settings.desktop_entry = Some(on);
                this.settings.save();
                let _ = if on {
                    horizon_launcher::desktop::install(backend::exe_path()).map(|_| ())
                } else {
                    horizon_launcher::desktop::remove()
                };
                cx.notify();
            }),
        );
        let horizon = match (&self.updates.horizon, &self.update_error) {
            (Some(r), _) => trf("{version} available", &[("version", update_name(r))]),
            (None, Some(e)) => e.clone(),
            (None, None) if self.checked.is_some() => tr("Up to date").to_string(),
            _ => tr("Checking…").to_string(),
        };
        let horizon_action = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| self.installing.is_none())
            .map(|_| {
                button("settings-install", "Install", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    }))
                    .into_any_element()
            });
        let launcher = match &self.self_update {
            SelfUpdate::Downloading(v, p) => {
                let done = format!("{:.0}", p * 100.);
                let said = trf(
                    "downloading {version}, {percent}%",
                    &[("version", v), ("percent", &done)],
                );
                format!("{}  ·  {said}", backend::VERSION)
            }
            SelfUpdate::Ready(v) => trf(
                "Updated to {version}. Restart the launcher to use it.",
                &[("version", v)],
            ),
            SelfUpdate::Failed(e) => format!(
                "{}  ·  {}",
                backend::VERSION,
                trf("the update failed: {error}", &[("error", e)])
            ),
            SelfUpdate::Idle => match &self.updates.launcher {
                Some(r) => format!(
                    "{}  ·  {}",
                    backend::VERSION,
                    trf(
                        "{version} available",
                        &[(
                            "version",
                            backend::launcher_version(&r.tag).unwrap_or_default()
                        )]
                    )
                ),
                None if self.checked.is_some() && self.update_error.is_none() => {
                    format!("{}  ·  {}", backend::VERSION, tr("up to date"))
                }
                None => backend::VERSION.to_string(),
            },
        };
        let launcher_action = match &self.self_update {
            SelfUpdate::Ready(_) => Some(
                button("restart-settings", "Restart", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.self_update = SelfUpdate::Failed(backend::restart());
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            SelfUpdate::Downloading(..) => None,
            _ => Some(
                button("check", "Check now", Kind::Ghost, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.check_updates();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        };
        let mono = |s: String| div().font_family(theme::MONO).child(s).into_any_element();
        let plain = |s: &str| div().child(tr(s).to_string()).into_any_element();
        let (accounts, n) = self.account_rows(cx);
        let (offline, n) = self.offline_rows(n, cx);
        let appearance = segments(
            "appearance",
            &[
                (backend::Appearance::Dark, "Dark"),
                (backend::Appearance::Light, "Light"),
                (backend::Appearance::System, "System"),
            ],
            self.settings.appearance,
            &t,
            {
                let this = cx.entity().downgrade();
                move |choice, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.appearance = choice;
                        this.settings.save();
                        titlebar::apply_appearance(window, choice);
                        this.theme = this.settings_theme(window);
                        cx.notify();
                    });
                }
            },
        );
        let group = |label: &'static str, icon: &'static str| {
            section_title(label, icon, &t).mt(px(20.)).mb(px(12.))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Settings",
                "Accounts, updates, and the launcher itself.",
                &t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(group("Account", "user").mt_0())
                    .child(accounts)
                    .child(offline)
                    .child(group("Updates", "reload"))
                    .child(index_row(
                        n + 1,
                        tr("Install new versions automatically"),
                        Some(plain(
                            "Your selected version only changes if you were playing the newest one.",
                        )),
                        Some(toggle.into_any_element()),
                        &t,
                    ))
                    .child(index_row(
                        n + 2,
                        tr("Delete old versions automatically"),
                        Some(
                            div()
                                .child(trf(
                                    "Keeps the {count} newest, the one in use and the ones your instances use. Deleted versions install again from the Versions page.",
                                    &[("count", &self.settings.keep_newest().to_string())],
                                ))
                                .into_any_element(),
                        ),
                        Some(prune.into_any_element()),
                        &t,
                    ))
                    .child(group("Launcher", "monitor"))
                    .child(index_row(
                        n + 3,
                        tr("Appearance"),
                        Some(plain("Dark by default. System follows your computer's light or dark.")),
                        Some(appearance.into_any_element()),
                        &t,
                    ))
                    .child(self.accent_row(cx))
                    .child(self.home_background_row(cx))
                    .child(self.language_row(cx))
                    .child(self.while_playing_row(n + 3, cx))
                    .child(index_row(
                        n + 3,
                        tr("Show the opening"),
                        Some(plain("The wordmark, while the release list loads.")),
                        Some(opening.into_any_element()),
                        &t,
                    ))
                    .child(group("Network", "globe"))
                    .child(self.proxy_row(n + 4, cx))
                    .child(group("Game", "gamepad"))
                    .child(self.game_args_row(n + 5, cx))
                    .child(self.native_mods_row(n + 5, cx))
                    .child(group("AI", "terminal"))
                    .child(self.no_ai_row(n + 6, cx))
                    .child(group("About", "info-box"))
                    .child(index_row(n + 7, "Horizon", Some(plain(&horizon)), horizon_action, &t))
                    .child(index_row(
                        n + 8,
                        tr("Launcher"),
                        Some(plain(&launcher)),
                        launcher_action,
                        &t,
                    ))
                    .child(index_row(
                        n + 9,
                        tr("Data folder"),
                        Some(mono(short(&backend::data_dir()))),
                        None,
                        &t,
                    ))
                    .child(index_row(
                        n + 10,
                        tr("Logs"),
                        Some(mono(format!(
                            "{}  ·  {}",
                            short(&backend::logs_dir()),
                            tr("crash reports beside it")
                        ))),
                        Some(
                            button("open-logs", "Open", Kind::Secondary, &t)
                                .on_click(|_, _, cx| {
                                    let dir = backend::logs_dir();
                                    let _ = std::fs::create_dir_all(&dir);
                                    cx.open_with_system(&dir);
                                })
                                .into_any_element(),
                        ),
                        &t,
                    ))
                    .when(cfg!(target_os = "linux"), |d| {
                        d.child(index_row(
                            n + 11,
                            tr("Applications menu"),
                            Some(plain(if self.settings.desktop_entry() {
                                "Horizon and its icon are in your desktop's menu."
                            } else {
                                "Not in your desktop's menu."
                            })),
                            Some(menu_entry.into_any_element()),
                            &t,
                        ))
                    }),
            )
            .child(self.collection_section(cx))
            .children(self.agents_section(cx))
            .children(self.dev_options_section(cx))
    }

    /// Settings' Game arguments row: the extra arguments the game starts with, kept in
    /// `settings.json` and appended to the launcher's own (quotes group a word with spaces).
    /// Settings' Native mods row (docs/native-mods.md): off by default. On, Build compiles the
    /// native mods installed in the instance Play starts, after a warning that names them, and
    /// that instance then plays the build.
    fn native_mods_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = self.settings.native_mods;
        let toggle =
            switch("native-mods", on, &t).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.native_mods = !this.settings.native_mods;
                this.native_confirm = None;
                this.settings.save();
                cx.notify();
            }));
        let mut detail = div().flex().flex_col().gap(px(4.)).child(
            tr("Native mods from the Mods page compile into the game with the launcher's own Rust. Not sandboxed: they can do anything you can on this computer.").to_string(),
        );
        if on {
            let status = match (&self.native_step, &self.native_error) {
                (Some(step), _) => Some(div().child(step.clone())),
                (None, Some(e)) => Some(div().text_color(t.danger).child(e.clone())),
                (None, None) => instances::selected()
                    .version
                    .filter(|v| native_mods::is_native(v))
                    .map(|v| {
                        div().child(trf("{version} plays in this instance.", &[("version", &v)]))
                    }),
            };
            detail = detail.children(status);
            if self.native_step.is_none() && self.native_confirm.is_none() {
                detail = detail.child(div().flex().gap(px(8.)).mt(px(4.)).child(
                    button("native-build", "Build", Kind::Primary, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.native_confirm = Some(this.native_mods_to_build().map(|(_, b)| b));
                            cx.notify();
                        },
                    )),
                ));
            }
            if let Some(mods) = &self.native_confirm {
                detail = detail.child(self.native_warning(mods, cx));
            }
        }
        index_row(
            n,
            tr("Native mods (experimental)"),
            Some(detail.into_any_element()),
            Some(toggle.into_any_element()),
            &t,
        )
    }

    /// The warning before a native build: what the mods can do and which mods. The build starts
    /// only from its button.
    fn native_warning(
        &self,
        mods: &Result<Vec<native_mods::Bundle>, String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let list = match mods {
            Ok(found) => found
                .iter()
                .map(|b| format!("{} {}", b.id, b.version))
                .collect::<Vec<_>>()
                .join(", "),
            Err(e) => e.clone(),
        };
        let panel = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mt(px(8.))
            .p(px(12.))
            .bg(t.well)
            .child(div().text_color(t.danger).child(tr("These mods will run with full access to your computer").to_string()))
            .child(tr("They can read, change or delete your files, and use your accounts and network, like any program you run. Build them only if you trust their authors: the Mods page names them.").to_string())
            .child(trf("Mods: {mods}", &[("mods", &list)]))
            .child(tr("The build compiles the game: 20 to 30 minutes and a few GB of disk the first time (it downloads Rust), a few minutes after that.").to_string());
        let mut buttons = div().flex().gap(px(8.)).mt(px(4.));
        if mods.is_ok() {
            buttons = buttons.child(
                button(
                    "native-trust",
                    "I trust these mods: build",
                    Kind::Primary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.build_native();
                    cx.notify();
                })),
            );
        }
        panel.child(buttons.child(
            button("native-cancel", "Cancel", Kind::Secondary, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.native_confirm = None;
                    cx.notify();
                },
            )),
        ))
    }

    fn game_args_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.args_focused;
        let field = div()
            .id("game-args")
            .track_focus(&self.args_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.args_input,
                &t,
                focused,
                false,
                false,
                "For example --server-thread",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.args_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.args_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(),
                    line_edit::Key::Edited => {
                        this.settings.game_args = this.args_input.text().to_string();
                        this.settings.save();
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.args_input);
        index_row(
            n,
            "Game arguments",
            Some(
                div()
                    .child("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts.")
                    .into_any_element(),
            ),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Settings' Proxy row: the HTTP proxy every download and sign-in goes through, kept in
    /// `settings.json` and read back by `proxy.rs` on each request.
    fn proxy_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.proxy_focused;
        let field = div()
            .id("proxy")
            .track_focus(&self.proxy_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.proxy_input,
                &t,
                focused,
                false,
                false,
                "For example http://127.0.0.1:7890",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.proxy_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.proxy_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(),
                    line_edit::Key::Edited => this.save_proxy(),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.proxy_input);
        // A half-typed address is no reason to complain; one left behind is.
        let detail = match (
            focused,
            horizon_launcher::proxy::refuse(self.proxy_input.text()),
        ) {
            (false, Some(why)) => div().text_color(t.danger).child(why).into_any_element(),
            _ => div()
                .child(
                    "Downloads, sign-ins and updates go through this HTTP proxy \
                     (http://host:port). Empty means none: HTTPS_PROXY is used instead when the \
                     launcher was started with one.",
                )
                .into_any_element(),
        };
        index_row(n, "Proxy", Some(detail), Some(field.into_any_element()), &t)
    }

    /// Keeps the Proxy field in `settings.json`, and tells the parts that build their own agents
    /// (`horizon-port-pack`) which proxy to use.
    fn save_proxy(&mut self) {
        let value = self.proxy_input.text().trim().to_owned();
        self.settings.proxy = (!value.is_empty()).then_some(value);
        self.settings.save();
        horizon_launcher::proxy::announce(self.settings.proxy.clone());
    }

    /// Settings' Microsoft accounts: who is signed in and plays, the sign-in with its code. The
    /// rows are numbered from 01; returns them and how many there are.
    fn account_rows(&mut self, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        let selected = self.accounts.selected.clone();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let uuid = a.uuid.clone();
            let mut actions = div().flex().gap(px(4.));
            if plays {
                actions = actions.child(
                    button(format!("offline-{i}"), "Play offline", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let mut accounts = Accounts::load();
                            accounts.selected = None;
                            if accounts.save().is_ok() {
                                this.accounts = accounts;
                            }
                            cx.notify();
                        }),
                    ),
                );
            } else {
                let uuid = uuid.clone();
                actions = actions.child(
                    button(format!("select-{i}"), "Play as", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(account::Player::Microsoft(uuid.clone()));
                            cx.notify();
                        }),
                    ),
                );
            }
            actions = actions.child(
                button(format!("sign-out-{i}"), "Sign out", Kind::Ghost, &t).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        let mut accounts = Accounts::load();
                        match accounts
                            .remove(account::store(), &uuid)
                            .and_then(|()| accounts.save())
                        {
                            Ok(()) => this.accounts = accounts,
                            Err(e) => this.sign_in_state = Some(SignInState::Done(Err(e))),
                        }
                        this.owner = Some(this.accounts.has_owner(account::store()));
                        cx.notify();
                    },
                )),
            );
            n += 1;
            rows = rows.child(index_row(
                n,
                a.name.clone(),
                Some(
                    div()
                        .child(tr(if plays {
                            "Microsoft account · plays"
                        } else {
                            "Microsoft account · signed in"
                        }))
                        .into_any_element(),
                ),
                Some(actions.into_any_element()),
                &t,
            ));
        }
        let configured = account::client_id().is_some();
        let busy = matches!(
            self.sign_in_state,
            Some(SignInState::Starting | SignInState::Code(_))
        );
        let detail = match &self.sign_in_state {
            _ if !configured => div().child(tr(account::NOT_CONFIGURED)),
            Some(SignInState::Starting) => div().child(tr("Asking Microsoft…")),
            Some(SignInState::Code(code)) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(trf(
                    "Open {link} in your browser and enter this code:",
                    &[("link", &code.verification_uri)],
                ))
                .child(
                    text(theme::HEADING)
                        .font_family(theme::MONO)
                        .text_color(t.ink)
                        .child(code.user_code.clone()),
                ),
            Some(SignInState::Done(Ok(a))) => div().child(format!("Signed in as {}.", a.name)),
            Some(SignInState::Done(Err(e))) => div().text_color(t.danger).child(e.clone()),
            None if self.accounts.accounts.is_empty() => div().child(tr(
                "Horizon needs a Microsoft account that owns Minecraft: Java Edition, like other launchers. It also plays online servers with your name and skin.",
            )),
            None => div().child(tr("Add another account.")),
        };
        let detail = match self.keychain.clone() {
            Some(said) if !self.accounts.accounts.is_empty() => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(detail)
                .child(div().child(said)),
            _ => detail,
        };
        let action = if !configured {
            None
        } else if let Some(SignInState::Code(code)) = &self.sign_in_state {
            let (uri, user_code) = (code.verification_uri.clone(), code.user_code.clone());
            Some(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(
                        button("sign-in-copy", "Copy code", Kind::Secondary, &t).on_click(
                            move |_, _, cx| {
                                copy_to_clipboard(
                                    cx,
                                    gpui::ClipboardItem::new_string(user_code.clone()),
                                )
                            },
                        ),
                    )
                    .child(
                        button("sign-in-open", "Open link", Kind::Secondary, &t)
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                    .child(
                        button("sign-in-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(s) = this.sign_in.take() {
                                    s.cancel();
                                }
                                this.sign_in_state = None;
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            )
        } else if busy {
            None
        } else {
            Some(
                button("sign-in", "Sign in with Microsoft", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_sign_in();
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        };
        n += 1;
        let title = if self.accounts.accounts.is_empty() {
            "Microsoft account"
        } else {
            "Another account"
        };
        rows = rows.child(index_row(
            n,
            tr(title),
            Some(detail.into_any_element()),
            action,
            &t,
        ));
        (rows, n)
    }

    /// Join a friend: a code field and Join, which starts the game straight
    /// into the friend's world (errors show in the game).
    fn join_friend_row(&self, can_play: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.friend_focused;
        let field = div()
            .id("friend-code")
            .track_focus(&self.friend_focus)
            .flex()
            .items_center()
            .w(px(260.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.friend_code,
                &t,
                focused,
                false,
                false,
                "Paste an invite code",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friend_code, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.join_friend(),
                    line_edit::Key::Escape => window.blur(),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.friend_code);
        let valid = horizon_friends::invite::parse(self.friend_code.text()).is_some();
        let mut join = button("join-friend", "Join", Kind::Secondary, &t);
        if can_play && valid {
            join = join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_friend();
                cx.notify();
            }));
        } else {
            join = disabled(join);
        }
        index_row(
            4,
            tr("Join a friend"),
            Some(
                div()
                    .child(tr("Paste the code a friend shows when they invite you."))
                    .into_any_element(),
            ),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(field)
                    .child(join)
                    .into_any_element(),
            ),
            &t,
        )
    }

    /// Plays the selected instance straight into a world (`--world`) or a server (`--server`):
    /// flags every Horizon release takes.
    pub(crate) fn quick_play(&mut self, flag: &'static str, value: String) {
        self.quick_play = Some((flag, value));
        self.play();
        if self.starting_client.is_none() {
            self.quick_play = None;
        }
    }

    fn join_friend(&mut self) {
        if horizon_friends::invite::parse(self.friend_code.text()).is_none() {
            return;
        }
        self.joining = Some(self.friend_code.text().to_string());
        self.play();
        if self.starting_client.is_none() {
            self.joining = None;
        }
    }

    fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = page_header(
            "Versions",
            "Builds from GitHub. Each one installs into its own folder.",
            &t,
        );
        // Measured again when the installed versions change.
        if self.disk_for != self.installed {
            self.disk_for = self.installed.clone();
            let (tags, tx) = (self.installed.clone(), self.tx.clone());
            std::thread::spawn(move || {
                let disk = tags.into_iter().map(|t| {
                    let n = backend::disk_use(&t);
                    (t, n)
                });
                let _ = tx.send(Msg::DiskUse(disk.collect()));
            });
        }
        let on_disk = |tag: &String| {
            let found = self.disk.iter().find(|(t, _)| t == tag);
            found
                .filter(|_| self.installed.contains(tag))
                .map(|(_, n)| *n)
        };
        let used: u64 = self.installed.iter().filter_map(on_disk).sum();
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(header)
            .children((used > 0).then(|| {
                text(theme::UI).text_color(t.muted).child(trf(
                    "Installed versions use {size}.",
                    &[("size", &megabytes(used))],
                ))
            }));
        let releases = match &self.releases {
            Load::Loading => return page.child(centered("Loading releases…", &t)),
            Load::Failed(e) => {
                return page.child(centered(&format!("Couldn't load releases. {e}"), &t).child(
                    button("retry", "Try again", Kind::Secondary, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.fetch_releases();
                            cx.notify();
                        },
                    )),
                ))
            }
            Load::Ready(r) if r.is_empty() => return page.child(centered("No releases yet.", &t)),
            Load::Ready(r) => r,
        };
        // A hundred releases: only the rows near the screen are drawn.
        let keep = |i: usize| {
            let tag = &releases[i].tag;
            self.installing.as_ref().is_some_and(|(t, _)| t == tag)
                || self.install_error.as_ref().is_some_and(|(t, _)| t == tag)
                || self.confirm_remove.as_ref() == Some(tag)
                || self.removing.as_ref() == Some(tag)
        };
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            8.,
            keep,
            |i| {
                let release = releases[i].clone();
                let tag = release.tag.clone();
                let installed = self.installed.contains(&tag);
                let selected = self.selected() == Some(&tag);
                let mut meta = vec![release.date.clone()];
                if let Some((_, size)) = release.asset {
                    meta.push(megabytes(size));
                }
                if let Some(n) = on_disk(&tag) {
                    meta.push(trf("{size} on disk", &[("size", &megabytes(n))]));
                }
                let summary = release
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_else(|| release.name.clone());
                let right: gpui::AnyElement = if let Some((_, p)) =
                    self.installing.as_ref().filter(|(t, _)| *t == tag)
                {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(Some(self.bar), &t))
                        .child(
                            text(theme::MICRO)
                                .w(px(36.))
                                .text_color(t.muted)
                                .child(format!("{:.0}%", p * 100.)),
                        )
                        .into_any_element()
                } else if self.removing.as_ref() == Some(&tag) {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(None, &t))
                        .child(text(theme::MICRO).text_color(t.muted).child("Removing…"))
                        .into_any_element()
                } else if installed {
                    let confirming = self.confirm_remove.as_ref() == Some(&tag);
                    let (t1, t2) = (tag.clone(), tag.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .when(!selected && !confirming, |d| {
                            d.child(
                                button(format!("use-{tag}"), "Use", Kind::Ghost, &t).on_click(
                                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.settings.selected = Some(t1.clone());
                                        this.settings.save();
                                        cx.notify();
                                    }),
                                ),
                            )
                        })
                        .child(
                            button(
                                format!("remove-{tag}"),
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
                                    if this.confirm_remove.as_ref() == Some(&t2) {
                                        this.remove(&t2);
                                    } else {
                                        this.confirm_remove = Some(t2.clone());
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .into_any_element()
                } else if release.asset.is_none() {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child("No build for this platform")
                        .into_any_element()
                } else {
                    let mut b = button(format!("install-{tag}"), "Install", Kind::Secondary, &t);
                    if self.installing.is_some() {
                        b = disabled(b);
                    } else {
                        b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.install(&release);
                            cx.notify();
                        }));
                    }
                    b.into_any_element()
                };
                let status = if selected {
                    Some(caps("In use").text_color(t.accent))
                } else if installed {
                    Some(caps("Installed").text_color(t.ink_soft))
                } else {
                    None
                };
                let error = self
                    .install_error
                    .as_ref()
                    .filter(|(t, _)| *t == tag)
                    .map(|(_, e)| e.clone());
                let id = SharedString::from(format!("release-{tag}"));
                let row = div()
                    .id(id.clone())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .py(px(14.))
                    .px(px(16.))
                    .mb(px(8.))
                    .bg(t.panel)
                    .border_2()
                    .border_color(t.outline)
                    .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                    .child(hover_layer(&id, t.raised, 0., 0.))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .child(version_tag(&tag).text_color(t.ink))
                                    .child(
                                        text(theme::UI)
                                            .text_color(t.muted)
                                            .truncate()
                                            .child(summary),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_end()
                                    .gap(px(2.))
                                    .flex_none()
                                    .children(status)
                                    .child(caps(meta.join("  ·  ")).text_color(t.muted)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .w(px(190.))
                                    .flex_none()
                                    .child(right),
                            ),
                    )
                    .children(error.map(|e| notice(e, &t)));
                track_hover(row, id)
            },
        );
        page.child(list)
    }

    fn notes_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let sponsor = (!self.links.sponsor.is_empty()).then(|| {
            plate(&t)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            text(theme::HEADING)
                                .text_color(t.ink)
                                .child("Support the work below"),
                        )
                        .child(text(theme::UI).text_color(t.muted).child(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        )),
                )
                .child(
                    button("sponsor", "Sponsor on GitHub", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click({
                            let url = self.links.sponsor.clone();
                            move |_: &ClickEvent, _, cx| cx.open_url(&url)
                        }),
                )
        });
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Patch notes",
                "What changed in each release, newest first.",
                &t,
            ))
            .children(sponsor);
        let releases = match &self.releases {
            Load::Loading => return page.child(centered("Loading patch notes…", &t)),
            Load::Failed(e) => {
                return page.child(
                    centered(&format!("Couldn't load patch notes. {e}"), &t).child(
                        button("retry-notes", "Try again", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.fetch_releases();
                                cx.notify();
                            }),
                        ),
                    ),
                )
            }
            Load::Ready(r) => r,
        };
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            10.,
            |_| false,
            |i| {
                let release = &releases[i];
                let summary = release
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .relative()
                    .gap(px(16.))
                    .p(px(20.))
                    .mb(px(10.))
                    .bg(t.panel)
                    .border_2()
                    .border_color(t.outline)
                    .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .child(version_tag(&release.tag).text_color(t.ink))
                                    .when(!summary.is_empty(), |d| {
                                        d.child(text(theme::UI).text_color(t.muted).child(summary))
                                    }),
                            )
                            .child(caps(release.date.clone()).flex_none().text_color(t.muted)),
                    )
                    .child(
                        div()
                            .max_w(px(640.))
                            .child(if release.body.trim().is_empty() {
                                text(theme::UI)
                                    .text_color(t.muted)
                                    .child("No notes for this release.")
                                    .into_any_element()
                            } else {
                                markdown::render(&release.body, &t)
                            }),
                    )
            },
        );
        page.child(list)
    }
}

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
/// Progress: a sunk track filling with the accent; work of unknown length shuttles a block.
fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .bg(t.accent)
        .border_b_2()
        .border_color(t.face_lip);
    let track = div()
        .relative()
        .flex_1()
        .h(px(12.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .overflow_hidden();
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if specimen::wanted() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(t.side)
                .children(titlebar::render(&t, window))
                .child(specimen::render(&t));
        }
        self.scale = window.scale_factor();
        self.perf.frame();
        if self.bench.as_ref().is_some_and(perf::Bench::scrolling) {
            self.bench_scroll(window);
        }
        self.servers.note_focus(window);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        self.args_focused = self.args_focus.is_focused(window);
        self.proxy_focused = self.proxy_focus.is_focused(window);
        let size = window.viewport_size();
        let content = match self.page {
            Page::Play => self.play_page(cx),
            Page::Versions => self.versions_page(cx),
            Page::Mods => self.mods_page(cx),
            Page::Servers => self.servers_page(cx),
            Page::Notes => self.notes_page(cx),
            Page::Settings => self.settings_page(cx),
        };
        self.sky.free_replaced(window);
        // The new page fades in and rises a few pixels.
        let content = div()
            .relative()
            .w_full()
            // Let the page shrink inside the padded flex row so long notices can wrap.
            .min_w_0()
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                Animation::new(theme::ENTER).with_easing(ease_out_quint()),
                |d, p| d.opacity(p).top(px(theme::RISE * (1. - p))),
            );
        let (reveal, held) = self.opening_phase(window);
        let ui = ease_out_cubic((reveal - 0.3) / 0.7);
        if self.opening.is_none() && window.focused(cx).is_none() {
            window.focus(&self.root_focus);
        }
        let (sky, next) = self.sky.render(
            &t.night,
            (size.width.into(), size.height.into()),
            window.scale_factor(),
            window.is_window_active(),
            Instant::now(),
        );
        if let Some(next) = next {
            self.wake_sky(next, cx);
        }
        let topbar = self.topbar(window, cx);
        let sidebar = self.sidebar(window, cx);
        let mut root = div()
            .id("root")
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "tab" => {
                        KEYBOARD.store(true, Ordering::Relaxed);
                        if k.modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                    }
                    "escape" if this.servers_escape() => {}
                    "escape" if this.instance_escape() => {}
                    "escape" if this.screenshot_escape() => {}
                    "left" if this.screenshot_step(false) => {}
                    "right" if this.screenshot_step(true) => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_any_mouse_down(|_, _, _| KEYBOARD.store(false, Ordering::Relaxed))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .font_family(theme::FONT)
            .text_color(t.ink)
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(topbar)
                    .child(
                        div().flex().flex_1().min_h_0().child(sidebar).child(
                            self.content_scroll
                                .wrap(
                                    div()
                                        .id("content")
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .child(
                                            div()
                                                .flex()
                                                .w_full()
                                                .min_w_0()
                                                .px(px(theme::PAGE_GUTTER))
                                                .pt(px(if self.page == Page::Play {
                                                    0.
                                                } else {
                                                    28.
                                                }))
                                                .pb(px(48.))
                                                .child(content),
                                        ),
                                    &t,
                                )
                                .flex_1()
                                .min_w_0(),
                        ),
                    ),
            );
        if self.page == Page::Servers {
            if let Some(dialog) = self.create_dialog(cx) {
                root = root.child(dialog);
            }
        }
        root = self.import_drop_target(root, cx);
        root = root.children(self.import_note(cx));
        if let Some(dialog) = self.instance_dialog(cx) {
            root = root.child(dialog);
        }
        if self.page == Page::Play {
            if let Some(viewer) = self.screenshot_viewer(cx) {
                root = root.child(viewer);
            }
        }
        if self.asking_consent() {
            root = root.child(self.consent_screen(cx));
        }
        if self.asking_owner() {
            root = root.child(self.owner_screen(cx));
        }
        if let Some(opening) = &self.opening {
            // The wordmark arrives letter by letter, then gives way to the launcher; a click or
            // a key skips ahead.
            let gone = 1. - ease_out_cubic(reveal / 0.6);
            let word = div()
                .flex()
                .font_family(theme::PIXEL)
                .text_size(px(theme::WORDMARK.px * 0.5))
                .line_height(px(theme::WORDMARK.px * 0.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.ink)
                .children("HORIZON".chars().enumerate().map(|(i, c)| {
                    let at = held - i as f32 * theme::STAGGER.as_secs_f32() * 1.5;
                    let p = ease_out_cubic(at / theme::ENTER.as_secs_f32());
                    div()
                        .relative()
                        .opacity(p)
                        .top(px(theme::RISE * 2. * (1. - p)))
                        .child(c.to_string())
                }));
            root = root.child(
                div()
                    .id("opening")
                    .track_focus(&opening.focus)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.page.opacity(gone))
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-24. * (1. - gone)))
                            .opacity(gone)
                            .child(word),
                    )
                    .when(reveal == 0., |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.skip_opening();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, _: &KeyDownEvent, _, cx| {
                                this.skip_opening();
                                cx.notify();
                            },
                        ))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .relative()
            .size_full()
            .bg(t.side)
            .child(sky)
            .children(titlebar::render(&t, window))
            .child(root.flex_1().min_h_0())
            .children(titlebar::resize_handles(window))
    }
}

impl Launcher {
    /// Where the opening is: the reveal's progress (1 when there is no opening) and how long
    /// the wordmark has been showing, in seconds.
    fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        let loaded = !matches!(self.releases, Load::Loading);
        let Some(opening) = &mut self.opening else {
            return (1., 10.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 10.);
        }
        window.request_animation_frame();
        (reveal, held.as_secs_f32())
    }

    /// `horizon-launcher benchmark`: scroll the page a little on every frame, back to the top at
    /// the bottom, so the frame times measure scrolling.
    fn bench_scroll(&mut self, window: &mut Window) {
        let handle = &self.content_scroll.handle;
        let mut offset = handle.offset();
        offset.y -= px(24.);
        if -offset.y > handle.max_offset().height {
            offset.y = px(0.);
        }
        handle.set_offset(offset);
        window.request_animation_frame();
    }

    /// Draws the window again in `after`, when the sky next changes, unless a frame comes sooner.
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

    fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}

actions!(horizon_launcher, [Quit, CloseWindow]);

/// What closing the last window does: stop the servers this launcher started, send the telemetry,
/// exit. A game that is running keeps running. Cmd+Q goes through here too.
fn shut_down(cx: &mut App) {
    servers_page::stop_on_quit();
    telemetry::send_before_exit();
    cx.quit()
}

/// The macOS shortcuts: Cmd+Q quits, Cmd+W closes the window.
#[cfg(target_os = "macos")]
fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]
}

#[cfg(not(target_os = "macos"))]
fn key_bindings() -> Vec<KeyBinding> {
    Vec::new()
}

/// The menu bar on macOS, where "Quit" belongs to the app menu.
fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Horizon Launcher".into(),
            items: vec![MenuItem::action("Quit Horizon Launcher", Quit)],
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
        },
    ]
}

fn main() {
    if let Err(error) = console::attach_for_cli() {
        eprintln!("Could not restore the launcher's terminal streams: {error}");
        std::process::exit(1);
    }
    perf::process_started();
    backend::exe_path();
    // What the launcher downloads goes through the player's proxy, if they set one: the parts
    // that build their own agents (`horizon-port-pack`) are told before anything starts.
    horizon_launcher::proxy::announce(horizon_launcher::backend::Settings::load().proxy);
    if let Some(code) = cli::run() {
        std::process::exit(code);
    }
    Application::new()
        .with_assets(blocks::Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(horizon_theme::fonts::GEIST_MONO),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_REGULAR),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_MEDIUM),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_BOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_DISPLAY_BOLD),
                ])
                .expect("bundled fonts");
            cx.bind_keys(key_bindings());
            cx.on_action(|_: &Quit, cx| shut_down(cx));
            // The key press is being handled inside the window, so the window can only be
            // removed once that update has finished.
            cx.on_action(|_: &CloseWindow, cx| {
                cx.defer(|cx| {
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| window.remove_window());
                    }
                })
            });
            cx.set_menus(menus());
            let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(960.), px(600.))),
                    titlebar: Some(titlebar::options()),
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    app_id: Some("horizon-launcher".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Launcher::new(window, cx)),
            )
            .expect("window");
            cx.on_window_closed(shut_down).detach();
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn cmd_q_quits_and_cmd_w_closes_the_window() {
        use super::{key_bindings, CloseWindow, Quit};
        use gpui::{Keymap, Keystroke};
        let keymap = Keymap::new(key_bindings());
        let key = |s: &str| [Keystroke::parse(s).unwrap()];
        let (hit, _) = keymap.bindings_for_input(&key("cmd-q"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&Quit)));
        let (hit, _) = keymap.bindings_for_input(&key("cmd-w"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&CloseWindow)));
    }

    #[test]
    fn the_selection_survives_a_removal_or_moves_to_the_newest_left() {
        let left = ["v0.2.49".to_string(), "v0.2.48".to_string()];
        let pick =
            |saved: Option<&str>| super::selection_after_removal(&left, saved.map(String::from));
        assert_eq!(pick(Some("v0.2.48")).as_deref(), Some("v0.2.48"));
        assert_eq!(pick(Some("v0.2.47")).as_deref(), Some("v0.2.49"));
        assert_eq!(pick(None).as_deref(), Some("v0.2.49"));
        assert_eq!(
            super::selection_after_removal(&[], Some("v0.2.47".into())),
            None
        );
    }

    /// An owner's Play works on a version not prepared yet: it prepares it (downloading
    /// Minecraft from Mojang when this computer has no install), then plays. Without an owner
    /// it stays off: signing in comes first.
    #[test]
    fn play_prepares_first_for_an_owner() {
        use super::{play_press, PlayPress};
        assert_eq!(play_press(true, Some(true), false), PlayPress::Play);
        assert_eq!(play_press(true, None, false), PlayPress::Play);
        assert_eq!(
            play_press(false, Some(true), false),
            PlayPress::PrepareThenPlay
        );
        assert_eq!(play_press(false, Some(false), false), PlayPress::Nothing);
        assert_eq!(play_press(false, None, false), PlayPress::Nothing);
        assert_eq!(play_press(false, Some(true), true), PlayPress::Nothing);
        assert_eq!(play_press(true, Some(true), true), PlayPress::Nothing);
    }

    /// A server's Join (and every other shortcut) waits like Play: not while the version's
    /// first download and preparation run (#horizon-bugs, 0.2.23: a white window opened).
    #[test]
    fn shortcuts_start_only_when_play_would() {
        use super::starts_now;
        assert!(starts_now(true, false, false));
        assert!(
            !starts_now(false, true, false),
            "first-time setup under way"
        );
        assert!(!starts_now(true, true, false), "prepared again under way");
        assert!(!starts_now(false, false, false), "not prepared");
        assert!(!starts_now(true, false, true), "already running");
    }

    #[test]
    fn the_app_menu_has_quit() {
        let menus = super::menus();
        assert_eq!(menus[0].name, "Horizon Launcher");
        assert!(matches!(
            &menus[0].items[0],
            gpui::MenuItem::Action { name, .. } if name == "Quit Horizon Launcher"
        ));
    }

    #[test]
    fn the_offline_name_is_read_from_its_key() {
        use super::player_name;
        assert_eq!(
            player_name("name:Player63010\n").as_deref(),
            Some("Player63010")
        );
        assert_eq!(player_name("name:Steve\r\n").as_deref(), Some("Steve"));
        assert_eq!(player_name("Player63010\n"), None);
        assert_eq!(player_name(""), None);
    }

    #[test]
    fn counts_get_thousands_separators() {
        let shown: Vec<String> = [0, 7, 999, 1000, 12_300, 1_234_500]
            .into_iter()
            .map(super::thousands)
            .collect();
        assert_eq!(shown, ["0", "7", "999", "1,000", "12,300", "1,234,500"]);
    }
}
