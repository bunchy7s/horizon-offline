//! The Servers page: create a server in one dialog, then start, stop and watch it, give friends its
//! address, manage who may join, change its settings, back it up and update it, all with buttons. Every
//! action runs `horizon-server` or calls the running server's control API (`horizon_launcher::servers`).
use super::*;
use horizon_launcher::heads;
use horizon_launcher::servers::{
    self, Applied, Backup, Entry, Fix, NewServer, Problem, Status, World,
};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The servers this launcher started in this session, stopped when it closes unless they run as a
/// service (`stop_on_quit`).
static STARTED_HERE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// How often the open server's status and log are read, and the list's.
const POLL_OPEN: Duration = Duration::from_millis(1000);
const POLL_LIST: Duration = Duration::from_secs(3);

/// Lines of the server's log kept on the page.
const CONSOLE_KEEP: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Field {
    Name,
    Seed,
    Players,
    Command,
    AddPlayer,
    Motd,
    Port,
    Token,
    Email,
    /// The instance dialog's (`instances_page.rs`).
    InstanceName,
    InstanceGroup,
    /// Home's Multiplayer section's (`multiplayer_page.rs`).
    ServerName,
    ServerAddress,
    /// Home's Console's search (`console_page.rs`).
    ConsoleSearch,
    /// The Launch panel's (`launch_page.rs`).
    PreLaunch,
    Wrapper,
    LaunchEnv,
    /// Settings' new offline name (`switch_page.rs`).
    OfflineName,
    DeleteServer,
}

impl Field {
    const ALL: [Field; 19] = [
        Field::Name,
        Field::Seed,
        Field::Players,
        Field::Command,
        Field::AddPlayer,
        Field::Motd,
        Field::Port,
        Field::Token,
        Field::Email,
        Field::InstanceName,
        Field::InstanceGroup,
        Field::ServerName,
        Field::ServerAddress,
        Field::ConsoleSearch,
        Field::PreLaunch,
        Field::Wrapper,
        Field::LaunchEnv,
        Field::OfflineName,
        Field::DeleteServer,
    ];
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Tab {
    Overview,
    Players,
    Settings,
    Backups,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Overview, "Overview"),
    (Tab::Players, "Players"),
    (Tab::Settings, "Settings"),
    (Tab::Backups, "Backups"),
];

const MODES: [(&str, &str); 3] = [
    ("survival", "Survival"),
    ("creative", "Creative"),
    ("adventure", "Adventure"),
];

const DIFFICULTIES: [(&str, &str); 4] = [
    ("peaceful", "Peaceful"),
    ("easy", "Easy"),
    ("normal", "Normal"),
    ("hard", "Hard"),
];

/// What the background work reports.
pub(super) enum Event {
    Polled {
        dir: PathBuf,
        status: Box<Status>,
        logs: Option<(u64, Vec<String>)>,
    },
    Statuses(Vec<(PathBuf, Status)>),
    Line(String),
    Done(Result<String, Problem>),
    Created(Result<Entry, Problem>),
    Deleted {
        dir: PathBuf,
        result: Result<(), String>,
    },
    Details {
        dir: PathBuf,
        config: Option<Value>,
        backups: Vec<Backup>,
        service: bool,
    },
    Applied(Result<Applied, Problem>),
    Pvp(Option<bool>),
    UpdateCheck(Result<Value, Problem>),
    Failed(String),
}

/// The Create dialog's answers.
pub(super) struct Create {
    new_world: bool,
    copy: Option<PathBuf>,
    worlds: Vec<(String, PathBuf)>,
    mode: &'static str,
    difficulty: &'static str,
    max_players: u32,
    friends_only: bool,
    check_accounts: bool,
    eula: bool,
    /// Create was pressed without the EULA ticked.
    eula_missing: bool,
}

/// What the sidebar shows of the Servers page: its list once the page has read it, and whether
/// a server answers.
impl Servers {
    pub(super) fn sidebar_list(&self) -> Option<&[Entry]> {
        self.loaded.then_some(&self.list[..])
    }

    pub(super) fn is_up(&self, dir: &std::path::Path) -> bool {
        self.statuses.get(dir).is_some_and(|s| s.phase == "running")
    }
}

pub(super) struct Servers {
    list: Vec<Entry>,
    loaded: bool,
    pub(super) open: Option<PathBuf>,
    tab: Tab,
    statuses: HashMap<PathBuf, Status>,
    polling: bool,
    polled: Option<Instant>,
    log: VecDeque<String>,
    log_next: u64,
    log_dir: Option<PathBuf>,
    console_scroll: Scrollbar,
    create_scroll: Scrollbar,
    /// The action under way, and what it printed.
    busy: Option<String>,
    job_log: VecDeque<String>,
    problem: Option<Problem>,
    note: Option<String>,
    pub(super) create: Option<Create>,
    creating: bool,
    config: Option<Value>,
    /// Settings waiting for `config set`, and whether one runs.
    queued: Vec<(String, String)>,
    applying: bool,
    /// Settings saved while the server ran that wait for its next start.
    restart_needed: BTreeSet<String>,
    backups: Vec<Backup>,
    confirm_restore: Option<String>,
    confirm_delete: bool,
    service: Option<bool>,
    update: Option<Value>,
    /// `update --check` is under way.
    checking: bool,
    pvp: Option<bool>,
    texts: HashMap<Field, String>,
    focus: HashMap<Field, FocusHandle>,
    focused: Option<Field>,
    /// The field to put the keyboard in on the next frame.
    pub(super) focus_soon: Option<Field>,
    /// A suggested value shown selected: the first key typed replaces it.
    selected: Option<Field>,
    /// A field whose value was refused, and why: shown under it until it's edited.
    invalid: Option<(Field, &'static str)>,
    copied: Option<Instant>,
    faces: HashMap<String, Option<Vec<[u8; 4]>>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Servers {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            list: Vec::new(),
            loaded: false,
            open: None,
            tab: Tab::Overview,
            statuses: HashMap::new(),
            polling: false,
            polled: None,
            log: VecDeque::new(),
            log_next: 0,
            log_dir: None,
            console_scroll: Scrollbar::default(),
            create_scroll: Scrollbar::default(),
            busy: None,
            job_log: VecDeque::new(),
            problem: None,
            note: None,
            create: None,
            creating: false,
            config: None,
            queued: Vec::new(),
            applying: false,
            restart_needed: BTreeSet::new(),
            backups: Vec::new(),
            confirm_restore: None,
            confirm_delete: false,
            service: None,
            update: None,
            checking: false,
            pvp: None,
            texts: HashMap::new(),
            focus: HashMap::new(),
            focused: None,
            focus_soon: None,
            selected: None,
            invalid: None,
            copied: None,
            faces: HashMap::new(),
            tx,
            rx,
        }
    }

    pub(super) fn text(&mut self, f: Field) -> &mut String {
        self.texts.entry(f).or_default()
    }

    pub(super) fn value(&self, f: Field) -> &str {
        self.texts.get(&f).map_or("", String::as_str)
    }

    /// Which field has the keyboard, read every frame.
    pub(super) fn note_focus(&mut self, window: &mut Window) {
        if let Some(h) = self.focus_soon.take().and_then(|f| self.focus.get(&f)) {
            window.focus(h);
        }
        self.focused = Field::ALL
            .into_iter()
            .find(|f| self.focus.get(f).is_some_and(|h| h.is_focused(window)));
        if self.focused.is_some() && self.focused != self.selected {
            self.selected = None;
        }
    }

    fn status(&self) -> Status {
        self.open
            .as_ref()
            .and_then(|d| self.statuses.get(d).cloned())
            .unwrap_or_else(Status::stopped)
    }

    /// A setting from server.toml (as `config --json` gave it).
    fn setting(&self, section: &str, key: &str) -> Option<&Value> {
        self.config
            .as_ref()
            .map(|c| &c[section][key])
            .filter(|v| !v.is_null())
    }

    /// A setting as shown: what was just asked for, else what server.toml holds.
    fn shown(&self, section: &str, key: &str) -> String {
        let full = format!("{section}.{key}");
        if let Some((_, v)) = self.queued.iter().rev().find(|(k, _)| *k == full) {
            return v.clone();
        }
        match self.setting(section, key) {
            Some(Value::String(s)) => s.clone(),
            None => String::new(),
            Some(v) => v.to_string(),
        }
    }
}

/// Stops the servers this launcher started, unless they run in the background as a service.
pub(super) fn stop_on_quit() {
    for dir in STARTED_HERE.lock().unwrap().drain(..) {
        if servers::service_installed(&dir) {
            continue;
        }
        if let Some(api) = servers::Api::open(&dir) {
            let _ = api.call("POST", "/api/v1/stop", None);
        }
    }
}

/// The player's own name, to let them in on a friends-only server: the Microsoft account that plays,
/// else the name the game made up for them (`horizon-player.txt`).
fn own_name(accounts: &Accounts) -> Option<String> {
    accounts
        .selected()
        .map(|a| a.name.clone())
        .or_else(crate::offline_name)
}

fn bytes(n: u64) -> String {
    match n {
        0..1_000_000 => format!("{} kB", n.div_ceil(1000)),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.2} GB", n as f64 / 1e9),
    }
}

/// A backup's date and kind from its name (`2026-10-19_11-28-57-manual.zip`, UTC), for people.
/// What a key did in a one-line field.
#[derive(Debug, PartialEq)]
pub(super) enum Key {
    Enter,
    Escape,
    /// Command/Ctrl-V: the caller pastes the clipboard's first line.
    Paste,
    Edited,
    /// Not the field's: Tab (focus), arrows, shortcuts.
    Pass,
}

/// Types `key` into `text`. A `selected` suggestion is replaced by what's typed or pasted, and
/// cleared by Backspace.
pub(super) fn type_key(
    text: &mut String,
    selected: bool,
    key: &str,
    key_char: Option<&str>,
    command: bool,
) -> Key {
    let replace = |text: &mut String| {
        if selected {
            text.clear();
        }
    };
    match key {
        "enter" => Key::Enter,
        "escape" => Key::Escape,
        "v" if command => {
            replace(text);
            Key::Paste
        }
        "backspace" => {
            if selected {
                text.clear();
            } else {
                text.pop();
            }
            Key::Edited
        }
        _ if command => Key::Pass,
        // Not every platform gives Space a character.
        "space" => {
            replace(text);
            text.push(' ');
            Key::Edited
        }
        _ => match key_char {
            Some(ch) if !ch.is_empty() && !ch.chars().any(char::is_control) => {
                replace(text);
                text.push_str(ch);
                Key::Edited
            }
            _ => Key::Pass,
        },
    }
}

fn backup_label(b: &Backup) -> (String, String) {
    let stem = b.name.trim_end_matches(".zip");
    let (when, kind) = match stem.get(..19) {
        Some(when) if stem.len() > 20 => (when, &stem[20..]),
        _ => return (stem.to_string(), String::new()),
    };
    let (date, time) = when.split_once('_').unwrap_or((when, ""));
    let label = format!("{date} {} UTC", time.replace('-', ":"));
    let kind = match kind {
        "auto" => "Automatic".to_string(),
        "manual" => "Made by hand".to_string(),
        "before-update" => "Before an update".to_string(),
        // horizon-server writes the tag with dashes (before-v2026-10-19-10).
        k if k.starts_with("before-v")
            && k[8..].chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            format!("Before updating to {}", k[7..].replace('-', "."))
        }
        k if k.starts_with("before-") => format!("Before updating to {}", &k[7..]),
        k => k.to_string(),
    };
    (label, kind)
}

fn uptime(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h {:02} min", secs / 3600, secs / 60 % 60),
        _ => format!("{} d {} h", secs / 86_400, secs / 3600 % 24),
    }
}

/// Keeps a field's text to 256 characters.
fn clip(text: &mut String) {
    if let Some((at, _)) = text.char_indices().nth(256) {
        text.truncate(at);
    }
}

impl Launcher {
    // ----- Background work ------------------------------------------------------------------

    /// Runs `job` on a thread with the page busy; its lines and result arrive as events.
    fn server_job(
        &mut self,
        label: impl Into<String>,
        job: impl FnOnce(&dyn Fn(String)) -> Result<String, Problem> + Send + 'static,
    ) {
        if self.servers.busy.is_some() {
            return;
        }
        self.servers.busy = Some(label.into());
        self.servers.job_log.clear();
        self.servers.problem = None;
        self.servers.note = None;
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |l: String| {
                let _ = lines.send(Event::Line(l));
            };
            let _ = tx.send(Event::Done(job(&log)));
        });
    }

    /// `horizon-server <args>` on the open server, as a job.
    fn server_command(&mut self, label: &str, args: Vec<String>) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        self.server_job(label.to_string(), move |log| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            servers::run_streamed(&dir, &args, log).map(|_| String::new())
        });
    }

    /// Reads the open server's settings, backups and service state again.
    fn load_details(&self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let config = servers::config(&dir);
            let backups = servers::backups(&dir).unwrap_or_default();
            let service = servers::service_installed(&dir);
            let _ = tx.send(Event::Details {
                dir,
                config,
                backups,
                service,
            });
        });
    }

    pub(super) fn open_server(&mut self, dir: PathBuf) {
        if self.servers.open.as_ref() == Some(&dir) {
            return;
        }
        let s = &mut self.servers;
        s.open = Some(dir);
        s.tab = Tab::Overview;
        s.config = None;
        s.backups.clear();
        s.service = None;
        s.update = None;
        s.pvp = None;
        s.problem = None;
        s.note = None;
        s.confirm_restore = None;
        s.confirm_delete = false;
        s.texts.remove(&Field::DeleteServer);
        s.restart_needed.clear();
        s.texts.remove(&Field::Motd);
        s.texts.remove(&Field::Port);
        s.invalid = None;
        s.polled = None;
        self.load_details();
    }

    /// Called with the launcher's other work: what the background sent, and the next status poll.
    pub(super) fn servers_tick(&mut self) -> bool {
        let overview = self.page == Page::Servers
            && self.servers.open.is_some()
            && self.servers.tab == Tab::Overview;
        let mut changed = self.internet_tick(overview);
        while let Ok(event) = self.servers.rx.try_recv() {
            changed = true;
            self.servers_event(event);
        }
        if self.page != Page::Servers || self.servers.polling || !self.servers.loaded {
            return changed;
        }
        let every = if self.servers.open.is_some() {
            POLL_OPEN
        } else {
            POLL_LIST
        };
        if self.servers.polled.is_some_and(|at| at.elapsed() < every) {
            return changed;
        }
        self.servers.polled = Some(Instant::now());
        self.servers.polling = true;
        let tx = self.servers.tx.clone();
        match self.servers.open.clone() {
            Some(dir) => {
                let since = if self.servers.log_dir.as_ref() == Some(&dir) {
                    self.servers.log_next
                } else {
                    0
                };
                std::thread::spawn(move || {
                    let api = servers::Api::open(&dir);
                    let status = api
                        .as_ref()
                        .and_then(|a| a.status())
                        .unwrap_or_else(Status::stopped);
                    let logs = api.and_then(|a| a.logs(since));
                    let _ = tx.send(Event::Polled {
                        dir,
                        status: Box::new(status),
                        logs,
                    });
                });
            }
            None => {
                let dirs: Vec<PathBuf> = self.servers.list.iter().map(|e| e.dir.clone()).collect();
                std::thread::spawn(move || {
                    let all = dirs
                        .into_iter()
                        .map(|d| {
                            let s = servers::status(&d);
                            (d, s)
                        })
                        .collect();
                    let _ = tx.send(Event::Statuses(all));
                });
            }
        }
        changed
    }

    fn servers_event(&mut self, event: Event) {
        let s = &mut self.servers;
        match event {
            Event::Polled { dir, status, logs } => {
                s.polling = false;
                let was = s.statuses.get(&dir).map(|st| st.phase.clone());
                if was.as_deref() != Some("running") && status.phase == "running" {
                    // A fresh start took every saved setting.
                    s.restart_needed.clear();
                    s.pvp = None;
                }
                if let Some((next, lines)) = logs {
                    if s.log_dir.as_ref() != Some(&dir) {
                        s.log.clear();
                        s.log_dir = Some(dir.clone());
                    }
                    if !lines.is_empty() {
                        s.log.extend(lines);
                        while s.log.len() > CONSOLE_KEEP {
                            s.log.pop_front();
                        }
                        s.console_scroll.handle.scroll_to_bottom();
                    }
                    s.log_next = next;
                }
                let read_pvp = status.phase == "running" && s.pvp.is_none();
                s.statuses.insert(dir, *status);
                if read_pvp {
                    self.read_pvp();
                }
            }
            Event::Statuses(all) => {
                s.polling = false;
                s.statuses.extend(all);
            }
            Event::Line(line) => {
                s.job_log.push_back(line);
                while s.job_log.len() > 6 {
                    s.job_log.pop_front();
                }
            }
            Event::Done(result) => {
                s.busy = None;
                s.polled = None;
                match result {
                    Ok(note) if !note.is_empty() => s.note = Some(note),
                    Ok(_) => {}
                    Err(p) => s.problem = Some(p),
                }
                self.load_details();
            }
            Event::Created(result) => {
                s.creating = false;
                match result {
                    Ok(entry) => {
                        s.create = None;
                        s.list = servers::list();
                        self.open_server(entry.dir.clone());
                        self.servers.note = Some(format!(
                            "{} is ready. Press Start, then give your friends the address below.",
                            entry.name
                        ));
                    }
                    Err(p) => s.problem = Some(p),
                }
            }
            Event::Details {
                dir,
                config,
                backups,
                service,
            } => {
                if s.open.as_ref() == Some(&dir) {
                    s.config = config;
                    s.backups = backups;
                    s.service = Some(service);
                }
            }
            Event::Deleted { dir, result } => {
                s.busy = None;
                s.confirm_delete = false;
                s.list = servers::list();
                s.statuses.remove(&dir);
                STARTED_HERE
                    .lock()
                    .unwrap()
                    .retain(|started| started != &dir);
                if !s.list.iter().any(|entry| entry.dir == dir) {
                    s.open = None;
                    self.side_servers = s.list.clone();
                }
                match result {
                    Ok(()) => s.note = Some("Server moved to the trash.".into()),
                    Err(e) => s.problem = Some(Problem::plain(e)),
                }
            }
            Event::Applied(result) => {
                s.applying = false;
                match result {
                    Ok(applied) if applied.running => s.restart_needed.extend(applied.restart),
                    Ok(_) => {}
                    Err(p) => s.problem = Some(p),
                }
                if s.queued.is_empty() {
                    self.load_details();
                } else {
                    self.apply_settings();
                }
            }
            Event::Pvp(on) => s.pvp = on,
            Event::UpdateCheck(result) => {
                s.checking = false;
                match result {
                    Ok(v) => s.update = Some(v),
                    Err(p) => s.problem = Some(p),
                }
            }
            Event::Failed(e) => s.problem = Some(Problem::plain(e)),
        }
    }

    /// The `pvp` game rule, asked of the running game (it's a rule of the world, not a setting).
    fn read_pvp(&self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let on = servers::Api::open(&dir)
                .and_then(|a| a.command("gamerule pvp").ok())
                .and_then(|out| {
                    let text = out.join(" ");
                    let text = text.trim_end();
                    if text.ends_with("true") {
                        Some(true)
                    } else if text.ends_with("false") {
                        Some(false)
                    } else {
                        None
                    }
                });
            let _ = tx.send(Event::Pvp(on));
        });
    }

    /// Queues a setting for `horizon-server config set`; they go one batch at a time.
    fn set_setting(&mut self, key: &str, value: String) {
        self.servers.queued.retain(|(k, _)| k != key);
        self.servers.queued.push((key.to_string(), value));
        self.servers.note = None;
        if !self.servers.applying {
            self.apply_settings();
        }
    }

    fn apply_settings(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let batch = std::mem::take(&mut self.servers.queued);
        if batch.is_empty() {
            return;
        }
        self.servers.applying = true;
        // Shown as asked until the file is read back.
        if let Some(config) = &mut self.servers.config {
            for (key, value) in &batch {
                if let Some((section, field)) = key.split_once('.') {
                    let new = match &config[section][field] {
                        Value::Bool(_) => Value::Bool(value == "true"),
                        Value::Number(_) => value.parse::<i64>().map_or(Value::Null, Value::from),
                        _ => Value::String(value.clone()),
                    };
                    config[section][field] = new;
                }
            }
        }
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let pairs: Vec<(&str, String)> =
                batch.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let _ = tx.send(Event::Applied(servers::set_config(&dir, &pairs)));
        });
    }

    /// A command for the running game, through the control API; its answer shows in the console.
    fn api_command(&mut self, line: String) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        self.servers.log.push_back(format!("> {line}"));
        self.servers.console_scroll.handle.scroll_to_bottom();
        std::thread::spawn(move || {
            let result = servers::Api::open(&dir)
                .ok_or_else(|| "The server isn't running.".to_string())
                .and_then(|a| a.command(&line));
            if let Err(e) = result {
                let _ = tx.send(Event::Failed(e));
            }
        });
    }

    fn start_server(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        STARTED_HERE.lock().unwrap().push(dir.clone());
        self.servers.restart_needed.clear();
        self.server_job("Starting", move |log| {
            log("A new world takes a minute the first time".into());
            servers::run_streamed(&dir, &["start"], log).map(|_| String::new())
        });
    }

    fn fix(&mut self, fix: Fix, cx: &mut Context<Self>) {
        self.servers.problem = None;
        match fix {
            Fix::UsePort(port) => {
                self.set_setting("server.port", port.to_string());
                self.servers.texts.remove(&Field::Port);
                self.servers.note = Some(format!(
                    "The server now uses port {port}. Press Start again."
                ));
            }
            Fix::Prepare => match self.selected().cloned() {
                Some(tag) if backend::is_prepared(&tag) => {
                    self.server_job("Copying the game files", move |log| {
                        log(format!("Copying the game files prepared for Horizon {tag}"));
                        servers::set_up(&tag)
                            .map(|_| "The game files are ready. Press Start again.".to_string())
                            .map_err(Problem::plain)
                    });
                }
                _ => {
                    self.servers.create = None;
                    self.go(Page::Play);
                    if !self.preparing {
                        self.prepare(backend::MinecraftFrom::Install);
                    }
                }
            },
            Fix::AgreeEula => {
                if let Some(c) = &mut self.servers.create {
                    c.eula = true;
                    c.eula_missing = false;
                } else if let Some(dir) = &self.servers.open {
                    if let Err(e) = backend::agree_eula(dir) {
                        self.servers.problem = Some(Problem::plain(e));
                    }
                }
            }
            Fix::ShowLog => {
                self.servers.tab = Tab::Overview;
                if let Some(dir) = &self.servers.open {
                    cx.open_with_system(&dir.join("logs"));
                }
            }
        }
    }

    // ----- Creating -------------------------------------------------------------------------

    fn open_create(&mut self, cx: &mut Context<Self>) {
        self.focus_handle(Field::Name, cx);
        self.servers.focus_soon = Some(Field::Name);
        let worlds = servers::player_worlds();
        let name = if self.servers.list.is_empty() {
            "My server".to_string()
        } else {
            format!("Server {}", self.servers.list.len() + 1)
        };
        *self.servers.text(Field::Name) = name;
        self.servers.selected = Some(Field::Name);
        self.servers.text(Field::Seed).clear();
        *self.servers.text(Field::Players) = own_name(&self.accounts).unwrap_or_default();
        self.servers.problem = None;
        self.servers.create_scroll = Scrollbar::default();
        self.servers.create = Some(Create {
            new_world: true,
            copy: worlds.first().map(|w| w.1.clone()),
            worlds,
            mode: "survival",
            difficulty: "normal",
            max_players: 10,
            friends_only: true,
            check_accounts: false,
            eula: false,
            eula_missing: false,
        });
    }

    fn create_server(&mut self) {
        if self.servers.creating {
            return;
        }
        let Some(c) = &mut self.servers.create else {
            return;
        };
        if !c.eula {
            c.eula_missing = true;
            return;
        }
        let Some(c) = self.servers.create.as_ref() else {
            return;
        };
        let world = match (&c.copy, c.new_world) {
            (Some(dir), false) => World::Copy(dir.clone()),
            _ => World::New {
                seed: self.servers.value(Field::Seed).to_string(),
            },
        };
        let players: Vec<String> = self
            .servers
            .value(Field::Players)
            .split([',', ' '])
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        let name = self.servers.value(Field::Name).trim().to_string();
        let spec = NewServer {
            name: if name.is_empty() {
                "My server".into()
            } else {
                name
            },
            world,
            game_mode: c.mode.into(),
            difficulty: c.difficulty.into(),
            max_players: c.max_players,
            friends_only: c.friends_only,
            check_accounts: c.check_accounts,
            players,
            eula: c.eula,
        };
        let tag = self.selected().cloned();
        let tx = self.servers.tx.clone();
        self.servers.creating = true;
        self.servers.problem = None;
        self.servers.job_log.clear();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |l: String| {
                let _ = lines.send(Event::Line(l));
            };
            let _ = tx.send(Event::Created(servers::create(&spec, tag.as_deref(), &log)));
        });
    }

    // ----- Building blocks ------------------------------------------------------------------

    fn focus_handle(&mut self, f: Field, cx: &mut Context<Self>) -> FocusHandle {
        self.servers
            .focus
            .entry(f)
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }

    /// A one-line text field: typing, paste, Backspace, Enter to confirm, Escape, Tab to the next.
    pub(super) fn field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let handle = self.focus_handle(f, cx);
        let focused = self.servers.focused == Some(f);
        let selected = focused && self.servers.selected == Some(f);
        let invalid = self.servers.invalid.is_some_and(|(g, _)| g == f);
        let value = self.servers.value(f).to_string();
        let shown: String = {
            let chars: Vec<char> = value.chars().collect();
            if f == Field::Token {
                "•".repeat(chars.len().min(40))
            } else {
                chars[chars.len().saturating_sub(60)..].iter().collect()
            }
        };
        div()
            .id(SharedString::from(format!("field-{f:?}")))
            .track_focus(&handle)
            .flex()
            .items_center()
            .w(px(width))
            .h(px(theme::FIELD))
            .px(px(12.))
            .overflow_hidden()
            .rounded(px(theme::RADIUS))
            .border_1()
            .border_color(if invalid {
                t.danger
            } else if focused {
                t.accent
            } else {
                t.rule
            })
            .bg(t.surface)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(if value.is_empty() && !focused {
                div().text_color(t.muted).child(placeholder)
            } else {
                div()
                    .text_color(t.ink)
                    .when(selected, |d| {
                        d.rounded(px(2.)).bg(t.accent.opacity(theme::SELECTION))
                    })
                    .child(shown)
            })
            .when(focused, |d| {
                d.child(div().flex_none().w(px(1.)).h(px(16.)).bg(t.ink))
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                // A click puts the caret in: the suggestion is no longer selected.
                if matches!(ev, ClickEvent::Mouse(_)) {
                    this.servers.selected = None;
                }
                if let Some(h) = this.servers.focus.get(&f) {
                    window.focus(h);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let command = k.modifiers.control || k.modifiers.platform;
                if k.key != "enter" && this.servers.invalid.is_some_and(|(g, _)| g == f) {
                    this.servers.invalid = None;
                }
                let selected = this.servers.selected.take() == Some(f);
                let typed = type_key(
                    this.servers.text(f),
                    selected,
                    &k.key,
                    k.key_char.as_deref(),
                    command,
                );
                match typed {
                    Key::Enter if matches!(f, Field::InstanceName | Field::InstanceGroup) => {
                        this.save_instance(cx)
                    }
                    Key::Enter => this.field_enter(f),
                    Key::Escape => {
                        this.servers.confirm_delete = false;
                        if this.servers.create.is_some() && !this.servers.creating {
                            this.servers.create = None;
                        }
                        this.instance_escape();
                        window.blur();
                    }
                    Key::Paste => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            let line = text.lines().next().unwrap_or_default().trim().to_string();
                            this.servers.text(f).push_str(&line);
                        }
                    }
                    Key::Edited => {}
                    // Tab and the rest go on to the window: Tab moves the focus.
                    Key::Pass => return,
                }
                clip(this.servers.text(f));
                cx.stop_propagation();
                cx.notify();
            }))
    }

    /// Escape closes the Create dialog. Whether it did.
    pub(super) fn servers_escape(&mut self) -> bool {
        if self.page == Page::Servers && self.servers.confirm_delete {
            self.servers.confirm_delete = false;
            return true;
        }
        if self.page != Page::Servers || self.servers.create.is_none() || self.servers.creating {
            return false;
        }
        self.servers.create = None;
        self.servers.problem = None;
        true
    }

    fn field_enter(&mut self, f: Field) {
        match f {
            Field::Name | Field::Seed | Field::Players => self.create_server(),
            Field::Command => {
                let line = std::mem::take(self.servers.text(Field::Command));
                let line = line.trim().trim_start_matches('/').to_string();
                if !line.is_empty() {
                    self.api_command(line);
                }
            }
            Field::AddPlayer => self.add_player(),
            Field::Motd => {
                let motd = self.servers.value(Field::Motd).trim().to_string();
                if motd.is_empty() {
                    self.servers.problem =
                        Some(Problem::plain("Type the line players see under the name."));
                } else {
                    self.set_setting("server.motd", motd);
                }
            }
            Field::Port => self.save_port(),
            Field::Token => self.save_token(),
            Field::Email => self.invite_friend(),
            Field::InstanceName
            | Field::InstanceGroup
            | Field::ConsoleSearch
            | Field::DeleteServer => {}
            Field::PreLaunch | Field::Wrapper | Field::LaunchEnv => self.save_launch(),
            Field::OfflineName => self.offline_name_enter(),
            Field::ServerName | Field::ServerAddress => self.add_server(),
        }
    }

    fn save_port(&mut self) {
        match self.servers.value(Field::Port).trim().parse::<u16>() {
            Ok(port) if port >= 1024 => self.set_setting("server.port", port.to_string()),
            _ => {
                self.servers.invalid = Some((
                    Field::Port,
                    "Use a number from 1024 to 65535 (25565 is Minecraft's own).",
                ))
            }
        }
    }

    fn add_player(&mut self) {
        let name = self.servers.value(Field::AddPlayer).trim().to_string();
        let valid = (3..=16).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            self.servers.invalid = Some((
                Field::AddPlayer,
                "A Minecraft name has 3 to 16 letters, digits or underscores.",
            ));
            return;
        }
        self.servers.text(Field::AddPlayer).clear();
        self.server_command(
            &format!("Letting {name} in"),
            vec!["whitelist".into(), "add".into(), name],
        );
    }

    /// A player's face, from the default skin their UUID picks (the server knows no other).
    fn head(&mut self, uuid: &str, size: f32) -> Div {
        let resources = servers::game_files(&servers::runtime_root(), "26.3");
        let face = self
            .servers
            .faces
            .entry(uuid.to_string())
            .or_insert_with(|| heads::default_face(&resources, uuid))
            .clone();
        let cell = size / 8.;
        let mut d = div()
            .flex_none()
            .flex()
            .flex_col()
            .size(px(size))
            .rounded(px(3.))
            .overflow_hidden()
            .bg(self.theme.plate);
        if let Some(pixels) = face {
            for row in pixels.chunks(8) {
                d = d.child(div().flex().children(row.iter().map(|p| {
                    div().size(px(cell)).bg(Hsla::from(gpui::Rgba {
                        r: p[0] as f32 / 255.,
                        g: p[1] as f32 / 255.,
                        b: p[2] as f32 / 255.,
                        a: 1.,
                    }))
                })));
            }
        }
        d
    }

    /// A row's small button.
    fn small(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self) + 'static,
    ) -> Stateful<Div> {
        let b = button(id, label, Kind::Ghost, &self.theme)
            .h(px(28.))
            .px(px(10.))
            .text_size(px(theme::CAPTION.px));
        if enabled {
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                action(this);
                cx.notify();
            }))
        } else {
            disabled(b)
        }
    }

    // ----- The page -------------------------------------------------------------------------

    pub(super) fn servers_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        if !self.servers.loaded {
            self.servers.list = servers::list();
            self.servers.loaded = true;
            if self.servers.open.is_none() && self.servers.list.len() == 1 {
                let dir = self.servers.list[0].dir.clone();
                self.open_server(dir);
            }
        }
        if let Some(dir) = self.servers.open.clone() {
            if self.servers.list.iter().any(|e| e.dir == dir) {
                return self.server_dashboard(&dir, cx);
            }
            self.servers.open = None;
        }
        let header = page_header(
            "Servers",
            "Host a world for your friends on this computer. Create one, press Start, and give them \
             the address.",
            &t,
        );
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                button("server-create", "Create a server", Kind::Primary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_create(cx);
                        cx.notify();
                    }),
                ),
            )
            .child({
                let b = button("server-add", "Add a server folder…", Kind::Ghost, &t);
                if self.servers.creating {
                    disabled(b)
                } else {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.add_server_folder(cx);
                        cx.notify();
                    }))
                }
            });
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(header)
            .child(actions);
        if let Some(p) = self
            .servers
            .problem
            .clone()
            .filter(|_| self.servers.create.is_none())
        {
            page = page.child(self.problem_notice(p, cx));
        }
        let partner = super::partner::banner("partner-servers", 600., &t);
        if self.servers.list.is_empty() {
            return page
                .child(centered(
                    "No server yet. Create one: it takes a few seconds, and with the defaults \
                     (tick the EULA, press Create) you get a friends-only survival world.",
                    &t,
                ))
                .child(partner);
        }
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, e) in self.servers.list.clone().into_iter().enumerate() {
            let status = self
                .servers
                .statuses
                .get(&e.dir)
                .cloned()
                .unwrap_or_else(Status::stopped);
            let detail = if status.running() {
                format!(
                    "{} · {} of {} players",
                    phase_label(&status),
                    status.players.len(),
                    status.max_players.unwrap_or(0)
                )
            } else {
                format!("{} · {}", phase_label(&status), short(&e.dir))
            };
            let dir = e.dir.clone();
            list = list.child(index_row(
                i + 1,
                e.name.clone(),
                Some(detail.into_any_element()),
                Some(
                    button(
                        SharedString::from(format!("server-open-{i}")),
                        "Open",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
                ),
                &t,
            ));
        }
        page.child(list).child(partner)
    }

    fn add_server_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(dir) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.add_existing(dir);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// A folder with a server.toml joins the list; a vanilla server folder is set up first (its
    /// server.properties becomes server.toml; its world, lists and EULA stay).
    fn add_existing(&mut self, dir: PathBuf) {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Server".into());
        let tag = self.selected().cloned();
        let tx = self.servers.tx.clone();
        self.servers.creating = true;
        self.servers.problem = None;
        std::thread::spawn(move || {
            let result = (|| {
                if servers::program().is_none() {
                    let tag =
                        tag.ok_or_else(|| Problem::plain("Install a Horizon version first."))?;
                    servers::set_up(&tag).map_err(Problem::plain)?;
                }
                if !dir.join("server.toml").is_file() {
                    servers::run(&dir, &["init", "--yes"])?;
                }
                let entry = Entry { name, dir };
                servers::add(entry.clone()).map_err(Problem::plain)?;
                Ok(entry)
            })();
            let _ = tx.send(Event::Created(result));
        });
    }

    fn problem_notice(&self, p: Problem, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut n = notice(p.message.clone(), &t).items_center();
        if let Some(fix) = p.fix {
            let label = match &fix {
                Fix::UsePort(port) => format!("Use port {port}"),
                Fix::Prepare => "Prepare the game files".into(),
                Fix::AgreeEula => "I agree to the EULA".into(),
                Fix::ShowLog => "Open the logs".into(),
            };
            n = n.child(
                button("problem-fix", label, Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.fix(fix.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        n.child(
            button("problem-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.servers.problem = None;
                    cx.notify();
                },
            )),
        )
    }

    fn server_dashboard(&mut self, dir: &Path, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let status = self.servers.status();
        let name = self
            .servers
            .list
            .iter()
            .find(|e| e.dir == dir)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let running = status.running();
        let busy = self.servers.busy.clone();
        let mut top = div().flex().flex_col().gap(px(10.));
        if self.servers.list.len() > 1 || self.servers.create.is_none() {
            top = top.child(
                div().flex().child(
                    caps("← All servers")
                        .id("servers-back")
                        .text_color(t.muted)
                        .cursor_pointer()
                        .hover(move |s| s.text_color(t.ink))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.servers.open = None;
                            this.servers.polled = None;
                            this.servers.problem = None;
                            this.servers.note = None;
                            cx.notify();
                        })),
                ),
            );
        }
        let dot = match status.phase.as_str() {
            "running" if status.crashes > 0 => t.accent,
            "running" => t.success,
            "stopped" => t.muted,
            _ => t.accent,
        };
        top = top
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().size(px(8.)).rounded_full().bg(dot))
                    .child(caps(phase_label(&status)).text_color(t.muted))
                    .when_some(status.uptime.filter(|_| running), |d, up| {
                        d.child(caps(format!("· up {}", uptime(up))).text_color(t.muted))
                    }),
            )
            .child(text(theme::DISPLAY).text_color(t.ink).child(name));
        if status.crashes > 0 {
            top = top.child(text(theme::CAPTION).text_color(t.muted).child(format!(
                    "The game crashed {} time{} and was restarted by itself{}.",
                    status.crashes,
                    if status.crashes == 1 { "" } else { "s" },
                    status
                        .last_crash
                        .as_ref()
                        .map(|c| format!(" (last: {c})"))
                        .unwrap_or_default()
                )));
        }

        // The big buttons.
        let big = |b: Stateful<Div>| {
            b.min_w(px(132.))
                .h(px(theme::CONTROL_LARGE))
                .px(px(28.))
                .text_size(px(theme::UI.px))
        };
        let idle = busy.is_none();
        let start = big(button(
            "server-start",
            if running { "Stop" } else { "Start" },
            if running {
                Kind::Secondary
            } else {
                Kind::Primary
            },
            &t,
        ));
        let start = if !idle {
            disabled(start)
        } else if running {
            start.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.server_command("Stopping", vec!["stop".into()]);
                cx.notify();
            }))
        } else {
            start.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.start_server();
                cx.notify();
            }))
        };
        let restart = big(button("server-restart", "Restart", Kind::Secondary, &t));
        let restart = if idle && running {
            restart.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.servers.restart_needed.clear();
                this.server_command("Restarting", vec!["restart".into()]);
                cx.notify();
            }))
        } else {
            disabled(restart)
        };
        let panel = button("server-panel", "Open the web panel", Kind::Ghost, &t);
        let panel = if running {
            let d = dir.to_path_buf();
            panel.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                match servers::Api::open(&d).map(|a| a.panel_link()) {
                    Some(Ok(url)) => cx.open_url(&url),
                    Some(Err(e)) => this.servers.problem = Some(Problem::plain(e)),
                    None => {}
                }
                cx.notify();
            }))
        } else {
            disabled(panel)
        };
        let folder = dir.to_path_buf();
        let buttons = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(start)
            .child(restart)
            .child(panel)
            .child(
                button("server-folder", "Open the folder", Kind::Ghost, &t)
                    .on_click(move |_, _, cx| cx.open_with_system(&folder)),
            );

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(top)
            .child(buttons);
        if let Some(label) = &busy {
            let last = self.servers.job_log.back().cloned().unwrap_or_default();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(progress_bar(None, &t))
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .truncate()
                            .child(format!("{label}… {last}")),
                    ),
            );
        }
        if let Some(p) = self.servers.problem.clone() {
            page = page.child(self.problem_notice(p, cx));
        }
        if let Some(n) = self.servers.note.clone() {
            page = page.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(16.))
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(t.success))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::UI.px))
                            .text_color(t.ink)
                            .child(n),
                    ),
            );
        }
        if running && !self.servers.restart_needed.is_empty() {
            let names: Vec<&str> = self
                .servers
                .restart_needed
                .iter()
                .map(|k| setting_name(k))
                .collect();
            page = page.child(
                plate(&t)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(16.))
                    .child(div().flex_none().size(px(6.)).rounded_full().bg(t.accent))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::UI.px))
                            .text_color(t.ink)
                            .child(format!("Saved. Restart to apply: {}.", names.join(", "))),
                    )
                    .child({
                        let b =
                            button("restart-apply", "Restart now", Kind::Secondary, &t).bg(t.plate);
                        if idle {
                            b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.servers.restart_needed.clear();
                                this.server_command("Restarting", vec!["restart".into()]);
                                cx.notify();
                            }))
                        } else {
                            disabled(b)
                        }
                    }),
            );
        }
        page = page.child(self.stats(&status));
        let this = cx.entity().downgrade();
        page = page.child(div().flex().child(segments(
            "server-tabs",
            &TABS,
            self.servers.tab,
            &t,
            move |tab, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.servers.tab = tab;
                    this.servers.confirm_restore = None;
                    cx.notify();
                });
            },
        )));
        let body = match self.servers.tab {
            Tab::Overview => self.overview_tab(&status, cx),
            Tab::Players => self.players_tab(dir, &status, cx),
            Tab::Settings => self.settings_tab(&status, cx),
            Tab::Backups => self.backups_tab(&status, cx),
        };
        page.child(body)
    }

    fn stats(&self, status: &Status) -> Div {
        let t = self.theme;
        let tile = |value: String, label: &'static str| {
            plate(&t)
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(4.))
                .p(px(16.))
                .child(text(theme::TITLE).text_color(t.ink).child(value))
                .child(caps(label).text_color(t.muted))
        };
        let dash = || "–".to_string();
        let on = status.running();
        div()
            .flex()
            .gap(px(12.))
            .child(tile(
                if on {
                    format!(
                        "{} / {}",
                        status.players.len(),
                        status.max_players.unwrap_or(0)
                    )
                } else {
                    dash()
                },
                "Players online",
            ))
            .child(tile(
                status
                    .tps
                    .filter(|_| on)
                    .map_or_else(dash, |v| format!("{v:.1}")),
                "TPS · 20 is full speed",
            ))
            .child(tile(
                status
                    .mspt
                    .filter(|_| on)
                    .map_or_else(dash, |v| format!("{v:.1} ms")),
                "MSPT · tick time",
            ))
            .child(tile(
                status
                    .ram_bytes
                    .filter(|_| on)
                    .map_or_else(dash, |b| format!("{:.2} GB", b as f64 / 1e9)),
                "RAM",
            ))
    }

    fn overview_tab(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let port = self
            .servers
            .setting("server", "port")
            .and_then(Value::as_u64)
            .unwrap_or(25565);
        let lan = servers::lan_address();
        let address = match lan {
            Some(ip) if port == 25565 => ip.to_string(),
            Some(ip) => format!("{ip}:{port}"),
            None => format!("localhost:{port}"),
        };
        let copied = self
            .servers
            .copied
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2));
        let copy_text = address.clone();
        let address_plate = plate(&t)
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(20.))
            .child(caps("Address on this network").text_color(t.muted))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        text(theme::TITLE)
                            .font_family(theme::MONO)
                            .text_color(t.ink)
                            .child(address),
                    )
                    .child(
                        button(
                            "address-copy",
                            if copied { "Copied" } else { "Copy" },
                            Kind::Secondary,
                            &t,
                        )
                        .bg(t.plate)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            copy_to_clipboard(cx, gpui::ClipboardItem::new_string(
                                copy_text.clone(),
                            ));
                            this.servers.copied = Some(Instant::now());
                            cx.notify();
                        })),
                    ),
            )
            .child(text(theme::CAPTION).text_color(t.ink_soft).child(
                "Friends on the same Wi-Fi: Multiplayer, Add Server, then paste this address.",
            ))
            .child(text(theme::CAPTION).text_color(t.muted).child(format!(
                "Friends elsewhere can't reach it as it is: use Tailscale (free, nothing to change \
                 on your router, only people you invite), or forward TCP port {port} on your router \
                 to this computer."
            )));
        let mut tab = div().flex().flex_col().gap(px(20.)).child(address_plate);
        if !status.players.is_empty() {
            let mut row = div().flex().flex_wrap().gap(px(16.));
            for p in status.players.clone() {
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(self.head(&p.uuid, 20.))
                        .child(text(theme::UI).text_color(t.ink).child(p.name.clone())),
                );
            }
            tab = tab.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(caps("Online now").text_color(t.muted))
                    .child(row),
            );
        }
        tab.child(self.internet_panel(port as u16, cx))
            .child(self.server_console(status.running(), cx))
    }

    fn server_console(&mut self, running: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let lines: Vec<Div> = self
            .servers
            .log
            .iter()
            .map(|l| {
                div()
                    .when(l.starts_with("> "), |d| d.text_color(t.accent))
                    .child(l.clone())
            })
            .collect();
        let empty = self.servers.log.is_empty();
        let log = div()
            .id("server-console")
            .h(px(240.))
            .px(px(20.))
            .py(px(12.))
            .overflow_y_scroll()
            .font_family(theme::MONO)
            .text_size(px(theme::CAPTION.px))
            .line_height(px(19.))
            .text_color(t.ink_soft)
            .when(empty, |d| {
                d.child(div().text_color(t.muted).child(if running {
                    "Waiting for the server's log…"
                } else {
                    "The server's log shows here while it runs."
                }))
            })
            .children(lines);
        let log = self.servers.console_scroll.wrap(log, &t);
        let input = if running {
            self.field(
                Field::Command,
                "Type a command: say Hello, time set day, gamemode creative Alex…",
                520.,
                cx,
            )
            .flex_1()
            .border_0()
            .bg(t.plate_soft)
            .font_family(theme::MONO)
            .text_size(px(theme::CAPTION.px))
            .into_any_element()
        } else {
            div()
                .px(px(12.))
                .h(px(theme::FIELD))
                .flex()
                .items_center()
                .text_size(px(theme::CAPTION.px))
                .text_color(t.muted)
                .child("Start the server to type commands")
                .into_any_element()
        };
        plate(&t)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(40.))
                    .px(px(20.))
                    .border_b_1()
                    .border_color(t.rule)
                    .child(caps("Console").text_color(t.muted)),
            )
            .child(log)
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .py(px(4.))
                    .border_t_1()
                    .border_color(t.rule)
                    .child(div().text_color(t.accent).ml(px(12.)).child("›"))
                    .child(input),
            )
    }

    fn players_tab(&mut self, dir: &Path, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let running = status.running();
        let idle = self.servers.busy.is_none();
        let ops = servers::names(dir, "ops.json");
        let whitelist = servers::entries(dir, "whitelist.json");
        let banned = servers::names(dir, "banned-players.json");
        let is_op = |n: &str| ops.iter().any(|o| o.eq_ignore_ascii_case(n));
        let mut tab = div().flex().flex_col().gap(px(28.));

        // Online now.
        let mut online = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, p) in status.players.clone().into_iter().enumerate() {
            let op = is_op(&p.name);
            let (n1, n2, n3) = (p.name.clone(), p.name.clone(), p.name.clone());
            let actions = div()
                .flex()
                .gap(px(2.))
                .child(self.small(
                    SharedString::from(format!("online-op-{i}")),
                    if op {
                        "Remove operator"
                    } else {
                        "Make operator"
                    },
                    idle,
                    cx,
                    move |this| {
                        let verb = if op { "deop" } else { "op" };
                        this.server_command("Changing operators", vec![verb.into(), n1.clone()]);
                    },
                ))
                .child(self.small(
                    SharedString::from(format!("online-kick-{i}")),
                    "Kick",
                    running,
                    cx,
                    move |this| this.api_command(format!("kick {n2}")),
                ))
                .child(self.small(
                    SharedString::from(format!("online-ban-{i}")),
                    "Ban",
                    running,
                    cx,
                    move |this| this.api_command(format!("ban {n3}")),
                ));
            let ping = p.ping.map(|ms| format!("{ms} ms ping"));
            let about = match (op, ping) {
                (true, Some(ping)) => Some(format!("Operator · {ping}")),
                (true, None) => Some("Operator".to_string()),
                (false, ping) => ping,
            };
            online = online.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .min_h(px(theme::ROW))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.rule)
                    .child(self.head(&p.uuid, 24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(text(theme::UI).text_color(t.ink).child(p.name.clone()))
                            .children(
                                about.map(|a| text(theme::CAPTION).text_color(t.muted).child(a)),
                            ),
                    )
                    .child(actions),
            );
        }
        tab = tab.child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    text(theme::HEADING)
                        .text_color(t.ink)
                        .child(format!("Online now ({})", status.players.len())),
                )
                .child(if status.players.is_empty() {
                    text(theme::UI)
                        .text_color(t.muted)
                        .child(if running {
                            "Nobody is playing right now."
                        } else {
                            "The server is stopped."
                        })
                        .into_any_element()
                } else {
                    online.into_any_element()
                }),
        );

        // Who may join.
        let friends_only = self
            .servers
            .setting("server", "whitelist")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let toggle = switch("friends-only", friends_only, &t).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.set_setting("server.whitelist", (!friends_only).to_string());
                cx.notify();
            },
        ));
        let add = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.field(Field::AddPlayer, "A player's name", 240., cx))
            .child({
                let b = button("player-add", "Let in", Kind::Secondary, &t);
                if idle {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.add_player();
                        cx.notify();
                    }))
                } else {
                    disabled(b)
                }
            });
        let mut people: Vec<(String, String)> = whitelist.clone();
        for op in &ops {
            if !people.iter().any(|(n, _)| n.eq_ignore_ascii_case(op)) {
                people.push((op.clone(), String::new()));
            }
        }
        let mut allowed = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, (name, uuid)) in people.into_iter().enumerate() {
            let op = is_op(&name);
            let listed = whitelist.iter().any(|(n, _)| n == &name);
            let (n1, n2) = (name.clone(), name.clone());
            let mut actions = div().flex().gap(px(2.)).child(self.small(
                SharedString::from(format!("allowed-op-{i}")),
                if op {
                    "Remove operator"
                } else {
                    "Make operator"
                },
                idle,
                cx,
                move |this| {
                    let verb = if op { "deop" } else { "op" };
                    this.server_command("Changing operators", vec![verb.into(), n1.clone()]);
                },
            ));
            if listed {
                actions = actions.child(self.small(
                    SharedString::from(format!("allowed-remove-{i}")),
                    "Remove",
                    idle,
                    cx,
                    move |this| {
                        this.server_command(
                            "Updating the list",
                            vec!["whitelist".into(), "remove".into(), n2.clone()],
                        );
                    },
                ));
            }
            let head = if uuid.is_empty() {
                div().size(px(24.)).flex_none()
            } else {
                self.head(&uuid, 24.)
            };
            let about = match (op, listed) {
                (true, true) => Some("Operator: can run every command"),
                (true, false) => Some("Operator (not on the list: can join anyway)"),
                _ => None,
            };
            allowed = allowed.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .min_h(px(theme::ROW))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.rule)
                    .child(head)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(text(theme::UI).text_color(t.ink).child(name.clone()))
                            .when_some(about, |d, a| {
                                d.child(text(theme::CAPTION).text_color(t.muted).child(a))
                            }),
                    )
                    .child(actions),
            );
        }
        tab = tab.child(
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(text(theme::HEADING).text_color(t.ink).child("Who may join"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(toggle)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(text(theme::UI).text_color(t.ink).child("Friends only"))
                                .child(text(theme::CAPTION).text_color(t.muted).child(
                                    if friends_only {
                                        "On: only the players below can join."
                                    } else {
                                        "Off: anyone who has the address can join."
                                    },
                                )),
                        ),
                )
                .child(add)
                .children(self.servers.invalid.and_then(|(f, e)| {
                    (f == Field::AddPlayer)
                        .then(|| text(theme::CAPTION).text_color(t.danger).child(e))
                }))
                .child(allowed),
        );

        // Banned.
        if !banned.is_empty() {
            let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
            for (i, name) in banned.into_iter().enumerate() {
                let n = name.clone();
                list = list.child(index_row(
                    i + 1,
                    name,
                    None,
                    Some(
                        self.small(
                            SharedString::from(format!("unban-{i}")),
                            "Unban",
                            running,
                            cx,
                            move |this| this.api_command(format!("pardon {n}")),
                        )
                        .into_any_element(),
                    ),
                    &t,
                ));
            }
            tab = tab.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(text(theme::HEADING).text_color(t.ink).child("Banned"))
                    .when(!running, |d| {
                        d.child(
                            text(theme::CAPTION)
                                .text_color(t.muted)
                                .child("Start the server to unban someone."),
                        )
                    })
                    .child(list),
            );
        }
        tab
    }

    /// A row of the settings form: a name, one line on it, and its control.
    fn setting_row(&self, name: &str, about: String, control: AnyElement) -> Div {
        self.setting_row_checked(name, about, None, control)
    }

    /// A setting row whose text gives way to `error` when the value was refused.
    fn setting_row_checked(
        &self,
        name: &str,
        about: String,
        error: Option<&'static str>,
        control: AnyElement,
    ) -> Div {
        let t = self.theme;
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .min_h(px(theme::ROW))
            .py(px(12.))
            .border_b_1()
            .border_color(t.rule)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(text(theme::UI).text_color(t.ink).child(name.to_string()))
                    .child(match error {
                        Some(e) => text(theme::CAPTION).text_color(t.danger).child(e),
                        None => text(theme::CAPTION).text_color(t.muted).child(about),
                    }),
            )
            .child(control)
    }

    /// − value + for a number setting within `range`.
    fn stepper(
        &self,
        key: &'static str,
        range: std::ops::RangeInclusive<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let (section, field) = key.split_once('.').unwrap_or_default();
        let value: i64 = self.servers.shown(section, field).parse().unwrap_or(0);
        let make = |id: &'static str, label: &'static str, to: i64, cx: &mut Context<Self>| {
            let b = button(
                SharedString::from(format!("{key}-{id}")),
                label,
                Kind::Secondary,
                &t,
            )
            .w(px(36.))
            .px(px(0.));
            if range.contains(&to) && to != value {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_setting(key, to.to_string());
                    cx.notify();
                }))
            } else {
                disabled(b)
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(make("less", "−", value - 1, cx))
            .child(
                text(theme::UI)
                    .w(px(44.))
                    .flex()
                    .justify_center()
                    .text_color(t.ink)
                    .child(value.to_string()),
            )
            .child(make("more", "+", value + 1, cx))
            .into_any_element()
    }

    fn choice(
        &self,
        id: &'static str,
        key: &'static str,
        options: &[(&'static str, &'static str)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (section, field) = key.split_once('.').unwrap_or_default();
        let current = self.servers.shown(section, field);
        let current: &'static str = options
            .iter()
            .find(|(v, _)| *v == current)
            .map_or("", |(v, _)| v);
        let this = cx.entity().downgrade();
        segments(id, options, current, &self.theme, move |value, _, cx| {
            let _ = this.update(cx, |this, cx| {
                this.set_setting(key, value.to_string());
                cx.notify();
            });
        })
        .into_any_element()
    }

    fn settings_tab(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let running = status.running();
        if self.servers.config.is_none() {
            return div().child(centered(
                if self.older_horizon() {
                    "This server's Horizon is too old to show its settings here. Update it on \
                     the Backups tab, then they appear."
                } else {
                    "Reading the settings…"
                },
                &t,
            ));
        }
        if self.servers.focused != Some(Field::Motd)
            && !self.servers.texts.contains_key(&Field::Motd)
        {
            *self.servers.text(Field::Motd) = self.servers.shown("server", "motd");
        }
        if self.servers.focused != Some(Field::Port)
            && !self.servers.texts.contains_key(&Field::Port)
        {
            *self.servers.text(Field::Port) = self.servers.shown("server", "port");
        }
        let save = |id: &'static str, f: Field, cx: &mut Context<Self>| {
            button(id, "Save", Kind::Secondary, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.field_enter(f);
                    cx.notify();
                },
            ))
        };
        let motd = div()
            .flex()
            .gap(px(8.))
            .child(self.field(Field::Motd, "A Minecraft Server", 260., cx))
            .child(save("motd-save", Field::Motd, cx))
            .into_any_element();
        let port = div()
            .flex()
            .gap(px(8.))
            .child(self.field(Field::Port, "25565", 100., cx))
            .child(save("port-save", Field::Port, cx))
            .into_any_element();
        let pvp = match (running, self.servers.pvp) {
            (true, Some(on)) => switch("pvp", on, &t)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.servers.pvp = Some(!on);
                    this.api_command(format!("gamerule pvp {}", !on));
                    cx.notify();
                }))
                .into_any_element(),
            _ => disabled(switch("pvp", true, &t)).into_any_element(),
        };
        let accounts = self
            .servers
            .setting("server", "online_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let accounts_switch = switch("online-mode", accounts, &t)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_setting("server.online_mode", (!accounts).to_string());
                cx.notify();
            }))
            .into_any_element();
        let live = if running {
            "Applies at once."
        } else {
            "Used from the next start."
        };
        let later = if running {
            "Saved now, used after a restart."
        } else {
            "Used from the next start."
        };
        let form = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.rule)
            .child(self.setting_row(
                "Message in the server list",
                format!("What players see under the server's name (the MOTD). {later}"),
                motd,
            ))
            .child(self.setting_row(
                "Game mode",
                format!("For players who join for the first time. {live}"),
                self.choice("set-mode", "world.game_mode", &MODES, cx),
            ))
            .child(self.setting_row(
                "Difficulty",
                live.into(),
                self.choice("set-difficulty", "world.difficulty", &DIFFICULTIES, cx),
            ))
            .child(self.setting_row(
                "Players can hurt each other (PvP)",
                if running {
                    "A rule of the world. Applies at once.".into()
                } else {
                    "A rule of the world: start the server to change it.".into()
                },
                pvp,
            ))
            .child(self.setting_row(
                "Most players at once",
                format!("Operators can always join. {later}"),
                self.stepper("server.max_players", 1..=100, cx),
            ))
            .child(self.setting_row(
                "View distance",
                format!("How far players see, in chunks (2 to 32). Lower is lighter. {later}"),
                self.stepper("server.view_distance", 2..=32, cx),
            ))
            .child(self.setting_row(
                "Simulation distance",
                format!("How far from players crops grow and mobs move, in chunks. {later}"),
                self.stepper("server.simulation_distance", 2..=32, cx),
            ))
            .child(
                self.setting_row_checked(
                    "Port",
                    format!(
                    "1024 to 65535. Friends type it after the address unless it's 25565. {later}"
                ),
                    self.servers
                        .invalid
                        .and_then(|(f, e)| (f == Field::Port).then_some(e)),
                    port,
                ),
            )
            .child(self.setting_row(
                "Check Minecraft accounts",
                if accounts {
                    format!("On: players must own Minecraft and sign in with Microsoft. {later}")
                } else {
                    format!("Off: anyone can play under any name, so keep Friends only on. {later}")
                },
                accounts_switch,
            ))
            .child(self.background_row(cx))
            .child(self.delete_server_section(status, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(text(theme::CAPTION).text_color(t.muted).child(
                "Changes save as you make them. Those the game can't take while it runs wait for \
                 a restart, and the page says so.",
            ))
            .child(form)
    }

    fn delete_server_section(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let name = self
            .servers
            .list
            .iter()
            .find(|entry| Some(&entry.dir) == self.servers.open.as_ref())
            .map(|entry| entry.name.clone())
            .unwrap_or_default();
        let ready = !status.running()
            && self.servers.busy.is_none()
            && !self.servers.applying
            && self.servers.queued.is_empty()
            && self.servers.service == Some(false);
        let hint = if status.running() {
            Some("Stop the server before deleting it.")
        } else if self.servers.service == Some(true) {
            Some("Turn off running in the background before deleting it.")
        } else if !ready {
            Some("Wait for the current operation to finish.")
        } else {
            None
        };
        let mut controls = div()
            .flex()
            .flex_col()
            .items_end()
            .flex_none()
            .w(px(260.))
            .gap(px(12.));
        if self.servers.confirm_delete {
            let matches = self.servers.value(Field::DeleteServer) == name && !name.is_empty();
            let field = self.field(Field::DeleteServer, "Server name", 225., cx);
            let confirm = button(
                "server-delete-confirm",
                "Move to trash",
                Kind::Secondary,
                &t,
            )
            .text_color(t.danger);
            let confirm = if ready && matches {
                confirm.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(dir) = this.servers.open.clone() {
                        this.servers.busy = Some("Moving server to trash".into());
                        let tx = this.servers.tx.clone();
                        std::thread::spawn(move || {
                            let result = servers::delete(&dir);
                            let _ = tx.send(Event::Deleted { dir, result });
                        });
                    }
                    cx.notify();
                }))
            } else {
                disabled(confirm)
            };
            controls = controls
                .child(field)
                .child(text(theme::CAPTION).text_color(t.muted).child(
                    gpui::StyledText::new(format!("Type {name} to confirm.")).with_highlights([(
                        5..5 + name.len(),
                        gpui::HighlightStyle {
                            font_weight: Some(FontWeight::BOLD),
                            ..Default::default()
                        },
                    )]),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("server-delete-cancel", "Cancel", Kind::Ghost, &t).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.servers.confirm_delete = false;
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(confirm),
                );
        } else {
            let delete =
                button("server-delete", "Delete server…", Kind::Secondary, &t).text_color(t.danger);
            let delete = if ready {
                delete.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.servers.confirm_delete = true;
                    this.servers.text(Field::DeleteServer).clear();
                    this.focus_handle(Field::DeleteServer, cx);
                    this.servers.focus_soon = Some(Field::DeleteServer);
                    cx.notify();
                }))
            } else {
                disabled(delete)
            };
            controls = controls.child(delete);
        }
        if let Some(hint) = hint {
            controls = controls.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .w(px(260.))
                    .text_align(gpui::TextAlign::Right)
                    .child(hint),
            );
        }
        self.setting_row(
            "Delete server",
            "Remove this server from Horizon and move its worlds, settings and backups to the trash.".into(),
            controls.into_any_element(),
        )
        .items_start()
    }

    fn background_row(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = self.servers.service.unwrap_or(false);
        let idle = self.servers.busy.is_none() && self.servers.service.is_some();
        let toggle = switch("background", on, &t);
        let toggle = if idle {
            toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.server_command(
                    if on {
                        "Turning off running in the background"
                    } else {
                        "Setting it to run in the background"
                    },
                    vec![
                        "service".into(),
                        if on { "uninstall" } else { "install" }.into(),
                    ],
                );
                cx.notify();
            }))
        } else {
            disabled(toggle)
        };
        self.setting_row(
            "Run in the background and at login",
            if on {
                "On: starts when you log in to this computer, keeps running when the launcher \
                 closes, and comes back by itself if the computer restarts."
                    .into()
            } else {
                "Off: the server stops when you close the launcher.".into()
            },
            toggle.into_any_element(),
        )
    }

    fn updates_section(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let idle = self.servers.busy.is_none();
        let check = self.servers.update.clone();
        let release = status
            .release
            .clone()
            .or_else(|| {
                check
                    .as_ref()
                    .and_then(|u| u["installed"].as_str().map(str::to_string))
            })
            .or_else(servers::runtime_release)
            .unwrap_or_else(|| "?".into());
        let available = status.update_available.clone().or_else(|| {
            check
                .as_ref()
                .filter(|u| u["available"] == true)
                .and_then(|u| u["newest"].as_str().map(str::to_string))
        });
        let about = match (&available, &check, &status.update_last) {
            (Some(tag), _, _) => format!(
                "Update available: Horizon {tag} (this server runs {release}). Updating backs up \
                 the world first, and goes back by itself if the new version doesn't start."
            ),
            (None, Some(_), _) => format!("Up to date: Horizon {release}."),
            (None, None, Some((last, _))) => format!("Horizon {release}. {last}"),
            (None, None, None) => format!("Horizon {release}."),
        };
        let control = match available {
            Some(tag) => {
                let b = button("update-now", "Update now", Kind::Primary, &t);
                if idle {
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.servers.update = None;
                        if let Some(dir) = this.servers.open.clone() {
                            let done = format!(
                                "Updated to Horizon {tag}. The world was backed up first (Backups)."
                            );
                            this.server_job(format!("Updating to {tag}"), move |log| {
                                servers::run_streamed(&dir, &["update"], log).map(|_| done)
                            });
                        }
                        cx.notify();
                    }))
                } else {
                    disabled(b)
                }
            }
            None if self.servers.checking => {
                disabled(button("update-check", "Checking…", Kind::Secondary, &t))
            }
            None if !idle => disabled(button(
                "update-check",
                "Check for updates",
                Kind::Secondary,
                &t,
            )),
            None => button("update-check", "Check for updates", Kind::Secondary, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(dir) = this.servers.open.clone() {
                        this.servers.checking = true;
                        let tx = this.servers.tx.clone();
                        std::thread::spawn(move || {
                            let result = servers::run(&dir, &["update", "--check", "--json"])
                                .and_then(|out| {
                                    serde_json::from_str(out.trim())
                                        .map_err(|e| Problem::plain(e.to_string()))
                                });
                            let _ = tx.send(Event::UpdateCheck(result));
                        });
                    }
                    cx.notify();
                }),
            ),
        };
        let mut rows = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.rule)
            .child(self.setting_row("This server's Horizon", about, control.into_any_element()));
        if self.servers.setting("updates", "keep").is_some() {
            rows = rows.child(self.setting_row(
                "Older versions kept",
                "Downloaded versions kept besides the one in use and the one an update can go back \
                 to."
                    .into(),
                self.stepper("updates.keep", 0..=5, cx),
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(text(theme::HEADING).text_color(t.ink).child("Updates"))
            .child(rows)
    }

    /// The server's details came back without its settings: its horizon-server predates
    /// `config` (and `backups --json`).
    fn older_horizon(&self) -> bool {
        self.servers.service.is_some() && self.servers.config.is_none()
    }

    fn backups_tab(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let idle = self.servers.busy.is_none();
        let every = self.servers.shown("backups", "every");
        let auto = !every.is_empty() && every != "0";
        let keep = self.servers.shown("backups", "keep");
        let now = {
            let b = button("backup-now", "Back up now", Kind::Primary, &t);
            if idle {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.server_command("Backing up", vec!["backup".into()]);
                    cx.notify();
                }))
            } else {
                disabled(b)
            }
        };
        let toggle = switch("auto-backup", auto, &t).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.set_setting("backups.every", if auto { "0".into() } else { "6h".into() });
                cx.notify();
            },
        ));
        let mut list = div().flex().flex_col().border_t_1().border_color(t.rule);
        for (i, b) in self.servers.backups.clone().into_iter().enumerate() {
            let (when, kind) = backup_label(&b);
            let confirming = self.servers.confirm_restore.as_deref() == Some(b.name.as_str());
            let name = b.name.clone();
            let restored = format!(
                "The world is back as it was on {when}. The one it replaced is in the backups \
                 folder."
            );
            let restore = button(
                SharedString::from(format!("restore-{i}")),
                if confirming {
                    "Replace the world"
                } else {
                    "Restore"
                },
                if confirming {
                    Kind::Primary
                } else {
                    Kind::Ghost
                },
                &t,
            );
            let restore = if idle {
                restore.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.servers.confirm_restore.as_ref() == Some(&name) {
                        this.servers.confirm_restore = None;
                        if let Some(dir) = this.servers.open.clone() {
                            let (name, restored) = (name.clone(), restored.clone());
                            this.server_job("Restoring", move |log| {
                                servers::run_streamed(&dir, &["restore", &name, "--yes"], log)
                                    .map(|_| restored)
                            });
                        }
                    } else {
                        this.servers.confirm_restore = Some(name.clone());
                    }
                    cx.notify();
                }))
            } else {
                disabled(restore)
            };
            let mut right = div().flex().items_center().gap(px(8.));
            if confirming {
                right = right.child(
                    button("restore-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.servers.confirm_restore = None;
                            cx.notify();
                        },
                    )),
                );
            }
            right = right.child(restore);
            let detail = if kind.is_empty() {
                bytes(b.bytes)
            } else {
                format!("{kind} · {}", bytes(b.bytes))
            };
            list = list.child(index_row(
                i + 1,
                when,
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            ));
        }
        let mut tab = div().flex().flex_col().gap(px(20.)).child(
            div().flex().items_center().gap(px(16.)).child(now).child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(if status.running() {
                        "Players stay online: the world is saved, then zipped."
                    } else {
                        "Zips the world and the server's settings into its backups folder."
                    }),
            ),
        );
        if self.older_horizon() {
            return tab
                .child(text(theme::UI).text_color(t.muted).child(
                    "This server's Horizon is too old to list its backups here: they're in its \
                     backups folder. Update it below.",
                ))
                .child(self.updates_section(status, cx));
        }
        tab = tab.child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .child(toggle)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            text(theme::UI)
                                .text_color(t.ink)
                                .child("Back up automatically"),
                        )
                        .child(text(theme::CAPTION).text_color(t.muted).child(if auto {
                            format!(
                                "On: every {every} while the server runs, keeping the last \
                                     {keep}. Backups you make yourself are never deleted."
                            )
                        } else {
                            "Off: only the backups you make.".to_string()
                        })),
                ),
        );
        if self.servers.confirm_restore.is_some() {
            tab = tab.child(notice(
                "Restoring replaces the world with this backup. The world as it is now isn't \
                 deleted: it's moved into the backups folder. A running server stops and starts \
                 again with the restored world."
                    .into(),
                &t,
            ));
        }
        tab = if self.servers.backups.is_empty() {
            tab.child(text(theme::UI).text_color(t.muted).child("No backup yet."))
        } else {
            tab.child(list)
        };
        tab.child(self.updates_section(status, cx))
    }

    // ----- The Create dialog ----------------------------------------------------------------

    pub(super) fn create_dialog(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let c = self.servers.create.as_ref()?;
        let (new_world, copy, worlds) = (c.new_world, c.copy.clone(), c.worlds.clone());
        let (mode, difficulty, max_players) = (c.mode, c.difficulty, c.max_players);
        let (friends_only, check_accounts, eula, eula_missing) =
            (c.friends_only, c.check_accounts, c.eula, c.eula_missing);
        let creating = self.servers.creating;
        let row = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(caps(label).text_color(t.muted))
                .child(control)
        };
        let weak = cx.entity().downgrade();
        let pick = move |f: fn(&mut Create, &'static str)| {
            let weak = weak.clone();
            move |value: &'static str, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    if let Some(c) = &mut this.servers.create {
                        f(c, value);
                    }
                    cx.notify();
                });
            }
        };
        let world_kind = segments(
            "create-world",
            &[("new", "New world"), ("copy", "Copy one of my worlds")],
            if new_world { "new" } else { "copy" },
            &t,
            pick(|c, v| c.new_world = v == "new"),
        );
        let world_detail = if new_world {
            self.field(
                Field::Seed,
                "Seed (optional: leave empty for a random world)",
                400.,
                cx,
            )
            .into_any_element()
        } else if worlds.is_empty() {
            text(theme::CAPTION)
                .text_color(t.muted)
                .child("You have no single-player world yet: play one first, or choose New world.")
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().gap(px(2.));
            for (i, (name, path)) in worlds.into_iter().take(8).enumerate() {
                let chosen = copy.as_ref() == Some(&path);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("create-copy-{i}")))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .cursor_pointer()
                        .when(chosen, |d| d.bg(t.plate_soft))
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(
                            div()
                                .size(px(12.))
                                .rounded_full()
                                .border_1()
                                .border_color(if chosen { t.accent } else { t.muted })
                                .when(chosen, |d| d.bg(t.accent)),
                        )
                        .child(text(theme::UI).text_color(t.ink).child(name))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(c) = &mut this.servers.create {
                                c.copy = Some(path.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(list)
                .child(
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child("The server gets a copy: your single-player world stays as it is."),
                )
                .into_any_element()
        };
        let modes = segments("create-mode", &MODES, mode, &t, pick(|c, v| c.mode = v));
        let difficulties = segments(
            "create-difficulty",
            &DIFFICULTIES,
            difficulty,
            &t,
            pick(|c, v| c.difficulty = v),
        );
        let who = segments(
            "create-who",
            &[
                ("friends", "Friends only"),
                ("anyone", "Anyone with the address"),
            ],
            if friends_only { "friends" } else { "anyone" },
            &t,
            pick(|c, v| c.friends_only = v == "friends"),
        );
        let step = |id: &'static str, label: &'static str, delta: i32, cx: &mut Context<Self>| {
            button(id, label, Kind::Secondary, &t)
                .w(px(36.))
                .px(px(0.))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.servers.create {
                        c.max_players = (c.max_players as i32 + delta).clamp(1, 100) as u32;
                    }
                    cx.notify();
                }))
        };
        let players = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(step("create-fewer", "−", -1, cx))
            .child(
                text(theme::UI)
                    .w(px(44.))
                    .flex()
                    .justify_center()
                    .text_color(t.ink)
                    .child(max_players.to_string()),
            )
            .child(step("create-more", "+", 1, cx));
        let checkbox = |id: &'static str, on: bool| {
            div()
                .id(id)
                .flex_none()
                .size(px(18.))
                .rounded(px(4.))
                .border_1()
                .border_color(if on { t.accent } else { t.muted })
                .bg(if on { t.accent } else { t.surface })
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.on_accent)
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| d.child("✓"))
        };
        let accounts = div()
            .flex()
            .items_start()
            .gap(px(10.))
            .child(
                checkbox("create-accounts", check_accounts).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        if let Some(c) = &mut this.servers.create {
                            c.check_accounts = !c.check_accounts;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        text(theme::UI)
                            .text_color(t.ink)
                            .child("Check Minecraft accounts"),
                    )
                    .child(text(theme::CAPTION).text_color(t.muted).child(
                        "Only if every player signs in with Microsoft. Leave it off for Horizon \
                         players without an account: Friends only keeps strangers out.",
                    )),
            );
        let eula_line = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(checkbox("create-eula", eula).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.servers.create {
                        c.eula = !c.eula;
                        c.eula_missing = false;
                    }
                    cx.notify();
                },
            )))
            .child(
                text(theme::UI)
                    .text_color(if eula_missing { t.danger } else { t.ink })
                    .child("I agree to the Minecraft EULA"),
            )
            .child(
                div()
                    .id("create-eula-read")
                    .text_size(px(theme::UI.px))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .child("Read it")
                    .on_click(|_, _, cx| cx.open_url(servers::EULA_URL)),
            );
        let body = div()
            .id("create-body")
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(20.))
            .px(px(28.))
            .pt(px(28.))
            .pb(px(20.))
            .overflow_y_scroll()
            .child(caps("New server").text_color(t.muted))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child("Create a server"),
            )
            .child(text(theme::CAPTION).text_color(t.muted).child(
                "Everything has a default: tick the EULA and Create makes a friends-only survival \
                 world. You can change all of it later.",
            ))
            .child(row(
                "Name",
                self.field(Field::Name, "My server", 400., cx)
                    .into_any_element(),
            ))
            .child(row(
                "World",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().child(world_kind))
                    .child(world_detail)
                    .into_any_element(),
            ))
            .child(row(
                "Game mode",
                div().flex().child(modes).into_any_element(),
            ))
            .child(row(
                "Difficulty",
                div().flex().child(difficulties).into_any_element(),
            ))
            .child(row("Most players at once", players.into_any_element()))
            .child(row(
                "Who can join",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().child(who))
                    .when(friends_only, |d| {
                        d.child(self.field(
                            Field::Players,
                            "Players allowed in, separated by commas (you first)",
                            480.,
                            cx,
                        ))
                        .child(text(theme::CAPTION).text_color(t.muted).child(
                            "The first one becomes an operator. Add more later on the Players tab.",
                        ))
                    })
                    .child(accounts)
                    .into_any_element(),
            ))
            .child(eula_line);
        let mut foot = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_end()
            .gap(px(12.))
            .px(px(28.))
            .pt(px(8.))
            .pb(px(28.));
        if creating {
            let last = self.servers.job_log.back().cloned().unwrap_or_default();
            foot = foot.child(
                text(theme::CAPTION)
                    .flex_1()
                    .min_w_0()
                    .text_color(t.muted)
                    .truncate()
                    .child(format!("Creating… {last}")),
            );
        } else if let Some(p) = &self.servers.problem {
            foot = foot.child(
                text(theme::CAPTION)
                    .flex_1()
                    .min_w_0()
                    .text_color(t.danger)
                    .child(p.message.clone()),
            );
            if let Some(fix @ Fix::Prepare) = p.fix.clone() {
                foot = foot.child(
                    button("create-fix", "Prepare the game files", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.fix(fix.clone(), cx);
                            cx.notify();
                        }),
                    ),
                );
            }
        } else if eula_missing {
            foot = foot.child(
                text(theme::CAPTION)
                    .flex_1()
                    .text_color(t.danger)
                    .child("Tick the EULA box to create the server."),
            );
        }
        let cancel = button("create-cancel", "Cancel", Kind::Secondary, &t);
        let create = button("create-go", "Create", Kind::Primary, &t);
        foot = if creating {
            foot.child(disabled(cancel)).child(disabled(create))
        } else {
            foot.child(cancel.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.servers.create = None;
                this.servers.problem = None;
                cx.notify();
            })))
            .child(create.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.create_server();
                cx.notify();
            })))
        };
        let card = plate(&t)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(640.))
            .max_h_full()
            .bg(t.page)
            .border_1()
            .border_color(t.rule)
            .child(self.servers.create_scroll.wrap(body, &t))
            .child(foot);
        Some(
            div()
                .id("create-dialog")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .px(px(32.))
                .py(px(24.))
                .bg(t.page.opacity(0.92))
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .occlude()
                .child(card),
        )
    }
}

/// The state, in a few words.
fn phase_label(status: &Status) -> String {
    if let Some(busy) = &status.update_busy {
        return format!("Updating: {busy}");
    }
    match status.phase.as_str() {
        "running" if status.crashes > 0 => "Running · restarted after a crash".into(),
        "running" if status.paused => "Running · paused while nobody plays".into(),
        "running" => "Running".into(),
        "starting" => "Starting".into(),
        "restarting" => "Restarting".into(),
        "stopping" => "Stopping".into(),
        _ => "Stopped".into(),
    }
}

/// A setting's name for people.
fn setting_name(key: &str) -> &str {
    match key {
        "server.motd" => "the message in the server list",
        "server.port" => "the port",
        "server.max_players" => "most players",
        "server.view_distance" => "view distance",
        "server.simulation_distance" => "simulation distance",
        "server.online_mode" => "account checks",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_read_for_people() {
        let b = |name: &str| Backup {
            name: name.into(),
            bytes: 1,
            modified: None,
        };
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-manual.zip")),
            ("2026-10-19 11:28:57 UTC".into(), "Made by hand".into())
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-before-v2026.10.20.zip")).1,
            "Before updating to v2026.10.20"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_13-35-36-before-v2026-10-19-10.zip")).1,
            "Before updating to v2026.10.19.10"
        );
        assert_eq!(
            backup_label(&b("2026-10-14_17-46-40-before-update.zip")).1,
            "Before an update"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-auto.zip")).1,
            "Automatic"
        );
        assert_eq!(backup_label(&b("odd.zip")), ("odd".into(), String::new()));
        assert_eq!(bytes(5_292_542), "5.3 MB");
        assert_eq!(uptime(3725), "1 h 02 min");
        let mut long = "é".repeat(300);
        clip(&mut long);
        assert_eq!(long.chars().count(), 256);
    }

    /// The keyboard in a one-line field, as the Create dialog, Players, Settings and the console use it.
    #[test]
    fn fields_type_as_people_expect() {
        let mut text = String::from("My server");
        // The suggestion is selected: the first letter replaces it.
        assert_eq!(
            type_key(&mut text, true, "f", Some("F"), false),
            Key::Edited
        );
        assert_eq!(text, "F");
        // Space, which some platforms send without a character.
        assert_eq!(
            type_key(&mut text, false, "space", None, false),
            Key::Edited
        );
        assert_eq!(
            type_key(&mut text, false, "n", Some("N"), false),
            Key::Edited
        );
        assert_eq!(text, "F N");
        assert_eq!(
            type_key(&mut text, false, "backspace", None, false),
            Key::Edited
        );
        assert_eq!(text, "F ");
        // Backspace on a suggestion clears it all.
        let mut text = String::from("Server 2");
        type_key(&mut text, true, "backspace", None, false);
        assert_eq!(text, "");
        // Paste replaces a suggestion too; the caller adds the clipboard.
        let mut text = String::from("Server 2");
        assert_eq!(type_key(&mut text, true, "v", Some("v"), true), Key::Paste);
        assert_eq!(text, "");
        // Tab, arrows and shortcuts aren't the field's: Tab moves the focus.
        let mut text = String::from("Ludo");
        for (key, ch, command) in [
            ("tab", Some("\t"), false),
            ("left", None, false),
            ("a", Some("a"), true),
        ] {
            assert_eq!(type_key(&mut text, false, key, ch, command), Key::Pass);
        }
        assert_eq!(
            type_key(&mut text, true, "enter", Some("\n"), false),
            Key::Enter
        );
        assert_eq!(
            type_key(&mut text, false, "escape", None, false),
            Key::Escape
        );
        assert_eq!(text, "Ludo");
    }
}
