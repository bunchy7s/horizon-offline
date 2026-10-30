//! Title, world selection and world creation screens (TitleScreen,
//! SelectWorldScreen, CreateWorldScreen layouts), drawn as HUD quads.
use crate::{
    dialog_screen::BOX,
    game_rules_screen::{Exit, RulesScreen},
    hud::{HudAtlas, HudBatch, rgba},
    lang,
    save::Level,
    text::{Text, format_translation},
};
use std::path::{Path, PathBuf};
use std::{
    cell::{Cell, RefCell},
    sync::mpsc,
};

mod create_world;
mod flat;
#[cfg(test)]
mod tests;
pub mod world_management;
pub use create_world::{DIFFICULTIES, WorldOptions};
pub use flat::{FlatLayer, FlatSettings, State as FlatEditor};

/// SelectWorldScreen's `HeaderAndFooterLayout(8 + 9 + 8 + 20 + 4, 60)`.
const LIST_TOP: f32 = 49.0;
const LIST_BOTTOM: f32 = 60.0;
const ROW_HEIGHT: f32 = 36.0;
const ROW_WIDTH: f32 = 270.0;

fn edit_top(h: f32) -> f32 {
    ((h.ceil() - 234.0) / 2.0).floor()
}
pub(crate) fn confirm_top(h: f32, lines: usize) -> f32 {
    ((h.ceil() - 61.0 - lines as f32 * 9.0) / 2.0).floor()
}
/// The SafetyScreen's column (WarningScreen: title, 12 px around the message
/// box of its lines and 4 px padding, the checkbox, then the buttons, 8 px
/// apart): the title's top.
fn warning_top(h: f32, lines: usize) -> f32 {
    ((h.ceil() - 102.0 - lines as f32 * 9.0) / 2.0).floor()
}

fn translate(key: &str, fallback: &str, args: &[&dyn std::fmt::Display]) -> String {
    let args: Vec<Text> = args
        .iter()
        .map(|arg| Text::literal(arg.to_string()))
        .collect();
    format_translation(&lang::get_or(key, fallback), &args).plain()
}

enum WorldToast {
    Created {
        folder: String,
        bytes: u64,
    },
    Failed(String),
    /// `SystemToast.onWorldAccessFailure`: the folder of a world that couldn't be locked.
    AccessFailure(String),
}
impl WorldToast {
    fn text(&self) -> (String, String) {
        match self {
            Self::Created { folder, bytes } => (
                translate("selectWorld.edit.backupCreated", "Backed up: %s", &[folder]),
                translate(
                    "selectWorld.edit.backupSize",
                    "Size: %s MB",
                    &[&((*bytes as f64 / 1_048_576.0).ceil() as u64)],
                ),
            ),
            Self::Failed(error) => (
                lang::get_or("selectWorld.edit.backupFailed", "Backup failed"),
                error.clone(),
            ),
            Self::AccessFailure(folder) => (
                lang::get_or("selectWorld.access_failure", "Failed to access world"),
                folder.clone(),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorldEntry {
    pub folder: String,
    pub name: String,
    /// `GameType`: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    pub hardcore: bool,
    pub modified: std::time::SystemTime,
    pub commands: bool,
    pub version: String,
}
#[derive(Clone, Debug)]
pub enum Mode {
    Main,
    Select {
        worlds: Vec<WorldEntry>,
        selected: Option<usize>,
    },
    Create {
        name: String,
        seed: String,
        creative: bool,
        focus: u8,
        options: WorldOptions,
    },
    Edit {
        world: WorldEntry,
        name: String,
        focused: bool,
    },
    Backup {
        world: WorldEntry,
    },
    /// BackupConfirmScreen over a world whose file fixes are due, or, with `downgrade`
    /// (the saved version's name), over a world saved by a newer version.
    BackupBeforeUpgrade {
        world: WorldEntry,
        downgrade: Option<String>,
    },
    Delete {
        world: WorldEntry,
    },
    Error {
        message: String,
        return_to: Box<Mode>,
        /// AlertScreen with the red `selectWorld.incompatible.title`, not the access failure.
        incompatible: bool,
    },
    Multiplayer(crate::multiplayer::Screen),
    /// SafetyScreen: the warning before the first Multiplayer click, with its
    /// "Do not show this screen again" box.
    MultiplayerWarning {
        check: bool,
    },
    /// CreateFlatWorldScreen over Create New World: the fields that screen
    /// keeps and the superflat editor on top of them.
    CreateFlat {
        name: String,
        seed: String,
        creative: bool,
        focus: u8,
        options: WorldOptions,
        flat: flat::State,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Options,
    Accessibility,
    Language,
    Play {
        folder: String,
    },
    Create {
        folder: String,
        name: String,
        seed: Option<i64>,
        creative: bool,
        options: WorldOptions,
    },
    Connect {
        address: String,
    },
    /// Create World's Data Packs screen on the new world's pack folder.
    DataPacks {
        dir: PathBuf,
        selected: Vec<String>,
    },
}
pub struct Title {
    pub mode: Mode,
    /// Direct Connection's address, synchronized with `options.txt`'s `lastServer`.
    pub last_server: String,
    /// `skipMultiplayerWarning`, synchronized with `options.txt` the same way.
    pub skip_multiplayer_warning: bool,
    pub saves: PathBuf,
    /// The prepared resource root: the flat preset list and block registry.
    resources: PathBuf,
    pub started: std::time::Instant,
    search: String,
    search_focused: bool,
    /// The focused text field's cursor, selection and scroll.
    caret: RefCell<crate::ui::Caret>,
    scroll: Cell<crate::ui::Scroll>,
    create_return_to_main: bool,
    message_lines: Cell<usize>,
    /// The SafetyScreen's checkbox as last drawn: x, y, width, height.
    warning_check: Cell<[f32; 4]>,
    backup: Option<mpsc::Receiver<Result<u64, String>>>,
    backup_shown: Cell<bool>,
    /// The backup a `BackupBeforeUpgrade` asked for opens its world when done.
    backup_then_open: bool,
    /// The folder to open next, once that backup has finished.
    open_request: Option<String>,
    /// EditWorldScreen's and WorldOpenFlows' `SystemToast` and its slide.
    toast: Option<(crate::toasts::Slide, WorldToast)>,
    icons: RefCell<std::collections::HashMap<PathBuf, (std::time::SystemTime, Option<Vec<u8>>)>>,
    splash: Option<String>,
    /// The saves listed for Home, re-read every couple of seconds.
    recent: std::cell::RefCell<Option<(std::time::Instant, Vec<WorldEntry>)>>,
    /// WorldCreationGameRulesScreen, over Create New World while it is open.
    rules: RefCell<Option<RulesScreen>>,
}
pub(crate) struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    enabled: bool,
    id: u8,
    icon: Option<(&'static str, f32)>,
}
/// Java String.hashCode for text seeds (WorldOptions.parseSeed).
pub fn parse_seed(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    Some(t.parse::<i64>().unwrap_or_else(|_| {
        i64::from(
            t.encode_utf16()
                .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c))),
        )
    }))
}
pub fn list_worlds(saves: &Path) -> Vec<WorldEntry> {
    let mut worlds: Vec<WorldEntry> = std::fs::read_dir(saves)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let level = Level::read(&path)?;
            let modified = std::fs::metadata(path.join("level.dat"))
                .and_then(|m| m.modified())
                .ok()?;
            let data = crate::save::read_dat(&path.join("level.dat"))?;
            let data = data.get("Data")?;
            let modified = match data.get("LastPlayed") {
                Some(horizon_world::nbt::Tag::Long(millis)) if *millis >= 0 => std::time::UNIX_EPOCH + std::time::Duration::from_millis(*millis as u64),
                _ => modified,
            };
            Some(WorldEntry {
                folder: e.file_name().to_string_lossy().into_owned(),
                name: level.name,
                game_mode: level.game_mode,
                hardcore: level.hardcore,
                modified,
                commands: matches!(data.get("allowCommands"), Some(horizon_world::nbt::Tag::Byte(v)) if *v != 0),
                version: data.get("Version").and_then(|v| v.get("Name"))
                    .and_then(|v| match v { horizon_world::nbt::Tag::String(s) => Some(s.to_string_lossy()), _ => None })
                    .unwrap_or_else(|| lang::get_or("selectWorld.versionUnknown", "Unknown")),
            })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}
/// LevelSummary.createInfo: a hardcore world's mode in red (CommonColors.RED),
/// else `gameMode.` + its GameType name, then commands and the version.
fn world_info(world: &WorldEntry) -> crate::text::Text {
    let mode = if world.hardcore {
        crate::text::Text::colored(lang::get_or("gameMode.hardcore", "Hardcore Mode"), 0xFF0000)
    } else {
        let (key, fallback) = match world.game_mode {
            1 => ("gameMode.creative", "Creative Mode"),
            2 => ("gameMode.adventure", "Adventure Mode"),
            3 => ("gameMode.spectator", "Spectator Mode"),
            _ => ("gameMode.survival", "Survival Mode"),
        };
        crate::text::Text::literal(lang::get_or(key, fallback))
    };
    let commands = if world.commands {
        format!(", {}", lang::get_or("selectWorld.commands", "Commands"))
    } else {
        String::new()
    };
    mode.append(crate::text::Text::literal(format!(
        "{commands}, {} {}",
        lang::get_or("selectWorld.version", "Version:"),
        world.version
    )))
}
fn choose_splash(text: &str, index: usize, date: (u32, u32)) -> Option<String> {
    // SplashManager checks SpecialDates before the resource list (local date).
    let special = match date {
        (12, 24) => Some("Merry X-mas!"),
        (1, 1) => Some("Happy new year!"),
        (10, 31) => Some("OOoooOOOoooo! Spooky!"),
        _ => None,
    };
    if let Some(special) = special {
        return Some(special.to_owned());
    }
    let splashes: Vec<_> = text
        .lines()
        .map(str::trim)
        // SplashManager.prepare keeps blank lines: only the hidden splash is dropped.
        .filter(|s| horizon_core::random::java_string_hash(s.encode_utf16()) != 125780783)
        .collect();
    splashes
        .get(index.checked_rem(splashes.len())?)
        .map(|s| (*s).to_owned())
}

impl Title {
    pub fn new(saves: PathBuf, resources: &Path) -> Self {
        use chrono::Datelike;
        let today = chrono::Local::now();
        let mode = Mode::Main;
        let now = std::time::Instant::now();
        Self {
            mode,
            last_server: String::new(),
            skip_multiplayer_warning: false,
            saves,
            resources: resources.to_owned(),
            started: now,
            search: String::new(),
            search_focused: false,
            caret: RefCell::default(),
            scroll: Cell::default(),
            create_return_to_main: false,
            message_lines: Cell::new(1),
            warning_check: Cell::new([0.0; 4]),
            backup: None,
            backup_shown: Cell::new(false),
            backup_then_open: false,
            open_request: None,
            toast: None,
            icons: RefCell::new(std::collections::HashMap::new()),
            splash: choose_splash(
                &std::fs::read_to_string(resources.join("assets/minecraft/texts/splashes.txt"))
                    .unwrap_or_default(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize,
                (today.month(), today.day()),
            ),
            recent: std::cell::RefCell::new(None),
            rules: RefCell::new(None),
        }
    }
    /// Vanilla bypasses the selector when there are no saves.
    pub fn enter_singleplayer(&mut self) {
        self.search.clear();
        self.search_focused = true;
        self.scroll.set(crate::ui::Scroll::default());
        let worlds = list_worlds(&self.saves);
        self.create_return_to_main = worlds.is_empty();
        self.mode = if worlds.is_empty() {
            Self::new_world()
        } else {
            Mode::Select {
                worlds,
                selected: None,
            }
        };
    }
    pub(crate) fn new_world() -> Mode {
        Mode::Create {
            name: lang::get_or("selectWorld.newWorld", "New World"),
            seed: String::new(),
            creative: false,
            focus: 0,
            options: WorldOptions::default(),
        }
    }
    /// WorldSelectionList.handleNewLevels: a singleplayer list that comes
    /// back empty (the last world deleted, say) opens a fresh Create New
    /// World, whose Cancel goes to the title screen.
    fn return_to_list(&mut self, folder: Option<&str>) {
        let worlds = list_worlds(&self.saves);
        *self.recent.borrow_mut() = None;
        if worlds.is_empty() {
            self.create_return_to_main = true;
            self.mode = Self::new_world();
            return;
        }
        let selected = folder.and_then(|folder| {
            worlds
                .iter()
                .position(|w| w.folder == folder && self.matches_search(w))
        });
        self.mode = Mode::Select { worlds, selected };
    }
    /// Deletes the world `Mode::Delete` asks about and goes back to the
    /// refreshed list; a failure (the world is open elsewhere, say) shows the
    /// error instead and leaves the world. Returns the deleted world's row in
    /// the list before it went, for a layout that moves the selection there.
    pub(crate) fn confirm_delete(&mut self) -> Option<usize> {
        let Mode::Delete { world } = &self.mode else {
            return None;
        };
        let folder = world.folder.clone();
        let row = list_worlds(&self.saves)
            .iter()
            .position(|w| w.folder == folder);
        if let Err(error) = world_management::delete(&self.saves, &folder) {
            self.failed(error);
            return None;
        }
        self.return_to_list(None);
        self.icons.borrow_mut().clear();
        row
    }
    fn cancel_create(&mut self) {
        // removeTempDataPackDir.
        if let Mode::Create { options, .. } = &self.mode
            && let Some(dir) = &options.pack_dir
        {
            let _ = std::fs::remove_dir_all(dir);
        }
        if self.create_return_to_main {
            self.mode = Mode::Main;
        } else {
            self.return_to_list(None);
        }
    }
    /// A press on the superflat screens: their list, share box or buttons.
    fn flat_click(&mut self, x: f32, y: f32, size: (f32, f32), hit: Option<u8>) {
        match hit {
            Some(70) => {
                if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                    flat.remove_selected();
                }
            }
            Some(71) => {
                if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                    flat.open_presets();
                }
            }
            Some(72) => self.close_flat(true),
            Some(73) => self.close_flat(false),
            Some(74) => {
                if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                    flat.apply_share();
                }
            }
            Some(75) => {
                if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                    flat.back_to_layers();
                }
            }
            None => {
                if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                    flat::click(flat, (x, y), size);
                }
            }
            _ => {}
        }
    }
    /// Return to Create New World, keeping (`apply`) or dropping the layers.
    fn close_flat(&mut self, apply: bool) {
        let mode = std::mem::replace(&mut self.mode, Mode::Main);
        if let Mode::CreateFlat {
            name,
            seed,
            creative,
            focus,
            mut options,
            flat,
        } = mode
        {
            if apply {
                options.world_type = crate::save::WorldPreset::Flat;
                options.flat_settings = Some(flat.settings);
            }
            self.mode = Mode::Create {
                name,
                seed,
                creative,
                focus,
                options,
            };
        }
    }
    /// The Data Packs screen's Done (`applyNewPackConfig`): its selection for
    /// the new world.
    pub fn set_data_packs(&mut self, ids: Vec<String>) {
        if let Mode::Create { options, .. } = &mut self.mode {
            options.data_packs = ids;
        }
    }
    fn matches_search(&self, world: &WorldEntry) -> bool {
        let filter = self.search.to_lowercase();
        world.name.to_lowercase().contains(&filter) || world.folder.to_lowercase().contains(&filter)
    }
    fn visible_worlds<'a>(
        &self,
        worlds: &'a [WorldEntry],
    ) -> impl Iterator<Item = (usize, &'a WorldEntry)> {
        worlds
            .iter()
            .enumerate()
            .filter(|(_, w)| self.matches_search(w))
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// The world list's top, bottom, rows' left edge and content height.
    fn world_list(&self, worlds: &[WorldEntry], (w, h): (f32, f32)) -> (f32, f32, f32, f32) {
        let (w, h) = (w.ceil(), h.ceil());
        let content = self.visible_worlds(worlds).count() as f32 * ROW_HEIGHT + 4.0;
        let left = (w / 2.0).floor() - ROW_WIDTH / 2.0;
        (LIST_TOP, h - LIST_BOTTOM, left, content)
    }
    /// The visible row under the pointer and the pointer relative to its
    /// content (`AbstractSelectionList.getEntryAtPosition`).
    fn world_row_at(
        &self,
        worlds: &[WorldEntry],
        (x, y): (f32, f32),
        size: (f32, f32),
    ) -> Option<(usize, (f32, f32))> {
        let (top, bottom, left, _) = self.world_list(worlds, size);
        if y < top || y >= bottom || x < left || x >= left + ROW_WIDTH {
            return None;
        }
        let offset = self.scroll.get().amount.trunc();
        let row = ((y - top - 2.0 + offset) / ROW_HEIGHT).floor();
        if row < 0.0 || row as usize >= self.visible_worlds(worlds).count() {
            return None;
        }
        let content_y = top + 4.0 + row * ROW_HEIGHT - offset;
        Some((row as usize, (x - left - 2.0, y - content_y)))
    }
    /// The pointer moved with the button held (`mouseDragged`).
    pub fn drag(&mut self, cursor: (f32, f32), size: (f32, f32)) {
        if let Some(rules) = self.rules.get_mut() {
            return rules.drag(cursor.1, size.1.ceil());
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            return screen.drag(cursor.1, size);
        }
        if let Mode::CreateFlat { flat, .. } = &mut self.mode {
            flat::drag(flat, cursor, size);
            return;
        }
        if let Mode::Select { worlds, .. } = &self.mode {
            let (top, bottom, _, content) = self.world_list(worlds, size);
            let mut scroll = self.scroll.get();
            scroll.drag(cursor.1, top, bottom, content);
            self.scroll.set(scroll);
        }
    }
    /// The button was released (`onRelease`).
    pub fn release(&mut self) {
        if let Some(rules) = self.rules.get_mut() {
            rules.release();
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.release();
        }
        if let Mode::CreateFlat { flat, .. } = &mut self.mode {
            flat::release(flat);
        }
        self.scroll.get_mut().release();
    }
    /// Mouse wheel over the classic world and server lists.
    pub fn scroll_worlds(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        if let Some(rules) = self.rules.get_mut() {
            return rules.wheel(delta, size.1.ceil());
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.wheel(delta, cursor, size);
            return;
        }
        if let Mode::CreateFlat { flat, .. } = &mut self.mode {
            flat::wheel(flat, delta, cursor, size);
            return;
        }
        let Mode::Select { worlds, .. } = &self.mode else {
            return;
        };
        let (top, bottom, _, content) = self.world_list(worlds, size);
        if cursor.1 < top || cursor.1 >= bottom {
            return;
        }
        let mut scroll = self.scroll.get();
        scroll.wheel(delta, ROW_HEIGHT / 2.0, top, bottom, content);
        self.scroll.set(scroll);
    }
    pub fn failed(&mut self, error: impl std::fmt::Display) {
        let return_to = Box::new(self.mode.clone());
        self.mode = Mode::Error {
            message: error.to_string(),
            return_to,
            incompatible: false,
        };
    }
    /// Polling is done once per rendered frame so filesystem work never blocks drawing.
    pub fn poll_backup(&mut self) {
        if !self.backup_shown.get() {
            return;
        }
        let Some(receiver) = &self.backup else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Backup worker stopped".into()),
        };
        self.backup = None;
        if let Mode::Backup { world } = &self.mode {
            let folder = world.folder.clone();
            self.return_to_list(Some(&folder));
            // EditWorldScreen opens the world after the backup, failed or not.
            if std::mem::take(&mut self.backup_then_open) {
                self.open_request = Some(folder.clone());
            }
            let toast = match result {
                Ok(bytes) => WorldToast::Created { folder, bytes },
                Err(error) => WorldToast::Failed(error),
            };
            self.toast = Some((crate::toasts::Slide::default(), toast));
        }
    }
    /// `ToastManager.update` for the system toast: slides it in, keeps it
    /// for 5 s times the notification display time, slides it out. The
    /// sound of a new visibility (`Toast.Visibility`), if any.
    pub fn update_toast(&mut self, multiplier: f32) -> Option<&'static str> {
        let (slide, _) = self.toast.as_mut()?;
        // SystemToast.update: shown until displayTime (5000 ms) has passed.
        let time = 5000.0 * f64::from(multiplier);
        let (sound, finished) = slide.update_now(|fully| (fully as f64) < time);
        if finished {
            self.toast = None;
        }
        sound.map(|show| {
            if show {
                "minecraft:ui.toast.in"
            } else {
                "minecraft:ui.toast.out"
            }
        })
    }
    /// The world a finished backup made before its upgrade opens next.
    pub fn take_open_request(&mut self) -> Option<String> {
        self.open_request.take()
    }
    /// `WorldOpenFlows.openWorld`: a world that can't be locked, as one another
    /// instance has open, stays closed behind `onWorldAccessFailure`
    /// (`createWorldAccess`); then `openWorldCheckVersionCompatibility`: a world from
    /// another series is an alert, one whose file fixes are due or that a newer version
    /// saved asks for a backup first; any other opens at once.
    fn play(&mut self, world: WorldEntry) -> Action {
        if crate::save::SessionLock::acquire(&self.saves.join(&world.folder)).is_err() {
            self.toast = Some((
                crate::toasts::Slide::default(),
                WorldToast::AccessFailure(world.folder),
            ));
            return Action::None;
        }
        // openWorldCheckVersionCompatibility: a world from another series is an AlertScreen.
        if let Some(version) = crate::save::incompatible_version(&self.saves.join(&world.folder)) {
            let version = if version.is_empty() {
                lang::get_or("selectWorld.versionUnknown", "unknown")
            } else {
                version
            };
            self.mode = Mode::Error {
                message: translate(
                    "selectWorld.incompatible.description",
                    "This world cannot be opened in this version.\nIt was last played in version %s.",
                    &[&version],
                ),
                return_to: Box::new(self.mode.clone()),
                incompatible: true,
            };
            return Action::None;
        }
        if crate::save::needs_file_fixes(&self.saves.join(&world.folder)) {
            self.mode = Mode::BackupBeforeUpgrade {
                world,
                downgrade: None,
            };
            return Action::None;
        }
        // BackupStatus.DOWNGRADE: a world from a newer version asks for a backup too.
        if let Some(version) = crate::save::downgraded_from(&self.saves.join(&world.folder)) {
            let version = if version.is_empty() {
                lang::get_or("selectWorld.versionUnknown", "unknown")
            } else {
                version
            };
            self.mode = Mode::BackupBeforeUpgrade {
                world,
                downgrade: Some(version),
            };
            return Action::None;
        }
        Action::Play {
            folder: world.folder,
        }
    }
    fn start_backup(&mut self, world: WorldEntry) {
        let (send, receive) = mpsc::channel();
        let saves = self.saves.clone();
        let folder = world.folder.clone();
        std::thread::spawn(move || {
            let _ = send.send(world_management::backup(&saves, &folder).map_err(|e| e.to_string()));
        });
        self.backup_shown.set(false);
        self.backup = Some(receive);
        self.mode = Mode::Backup { world };
    }
    fn icon(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        world: &WorldEntry,
        slot: usize,
        x: f32,
        y: f32,
    ) {
        let path = self.saves.join(&world.folder).join("icon.png");
        let mut icons = self.icons.borrow_mut();
        let pixels = if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            let cached = icons
                .entry(path.clone())
                .or_insert_with(|| (std::time::UNIX_EPOCH, None));
            if cached.0 != modified {
                *cached = (
                    modified,
                    crate::hud::decode(&path)
                        .ok()
                        .filter(|(w, h, _)| *w == 64 && *h == 64)
                        .map(|(_, _, p)| p),
                );
            }
            cached.1.as_ref()
        } else {
            None
        };
        let sprite = format!("horizon/world-icon{slot}");
        if let Some(pixels) = pixels
            && let Some(rect) = atlas.sprites.get(&sprite)
        {
            batch.patches.push(crate::font::Patch {
                x: (rect.u0 * atlas.width as f32).round() as u32,
                y: (rect.v0 * atlas.height as f32).round() as u32,
                width: 64,
                height: 64,
                rgba: pixels.clone(),
            });
            batch.sprite_scaled(atlas, &sprite, x, y, 32.0, 32.0, u32::MAX);
        } else {
            batch.sprite_scaled(atlas, "misc/unknown_server", x, y, 32.0, 32.0, u32::MAX);
        }
    }
    /// The worlds, newest first, cached for two seconds (drawn every frame).
    /// Saves changed behind the screens' back (agent control renamed or deleted a world): the
    /// list shows them again.
    pub fn worlds_changed(&mut self) {
        if matches!(self.mode, Mode::Select { .. }) {
            self.return_to_list(None);
        }
        *self.recent.borrow_mut() = None;
    }
    pub fn recent_worlds(&self) -> Vec<WorldEntry> {
        let mut recent = self.recent.borrow_mut();
        match &*recent {
            Some((at, worlds)) if at.elapsed().as_secs() < 2 => worlds.clone(),
            _ => {
                let worlds = list_worlds(&self.saves);
                *recent = Some((std::time::Instant::now(), worlds.clone()));
                worlds
            }
        }
    }
    /// Create New World: the folder name and options (FileUtil.findAvailableName).
    pub fn create_action(&self) -> Action {
        let Mode::Create {
            name,
            seed,
            creative,
            options,
            ..
        } = &self.mode
        else {
            return Action::None;
        };
        Action::Create {
            folder: self.target_folder(name),
            name: name.clone(),
            seed: parse_seed(seed),
            creative: *creative,
            options: options.clone(),
        }
    }
    /// The folder a new world named `name` gets (`findResultFolder`).
    fn target_folder(&self, name: &str) -> String {
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if base.trim().is_empty() {
            "World".to_owned()
        } else {
            base.trim().to_owned()
        };
        // FileUtil.findAvailableName: append (n) until unused.
        let mut folder = base.clone();
        let mut n = 1;
        while self.saves.join(&folder).exists() {
            folder = format!("{base} ({n})");
            n += 1;
        }
        folder
    }
    fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        // The game rules draw and press their own.
        if self.rules.borrow().is_some() {
            return Vec::new();
        }
        let (w, h) = (w.ceil(), h.ceil());
        let b = |x: f32, y: f32, w: f32, label: &str, enabled: bool, id: u8| Button {
            x,
            y,
            w,
            label: label.into(),
            enabled,
            id,
            icon: None,
        };
        match &self.mode {
            Mode::Main => {
                let cx = (w / 2.0).floor();
                let y = (h / 4.0).floor() + 48.0;
                let mut buttons = vec![
                    b(
                        cx - 100.0,
                        y,
                        200.0,
                        &lang::get_or("menu.singleplayer", "Singleplayer"),
                        true,
                        1,
                    ),
                    b(
                        cx - 100.0,
                        y + 24.0,
                        200.0,
                        &lang::get_or("menu.multiplayer", "Multiplayer"),
                        true,
                        4,
                    ),
                    b(
                        cx - 100.0,
                        y + 48.0,
                        200.0,
                        &lang::get_or("menu.online", "Minecraft Realms"),
                        false,
                        0,
                    ),
                    b(
                        cx - 100.0,
                        y + 96.0,
                        98.0,
                        &lang::get_or("menu.options", "Options..."),
                        true,
                        3,
                    ),
                    b(
                        cx + 2.0,
                        y + 96.0,
                        98.0,
                        &lang::get_or("menu.quit", "Quit Game"),
                        true,
                        2,
                    ),
                ];
                // CommonButtons' icon buttons; Horizon's Join a Friend (an
                // invite code) sits behind vanilla's Friends button.
                for (i, (key, plain, id, icon)) in [
                    ("gui.friends.open", "Friends", 5, "friends/friends"),
                    ("options.language", "Language...", 6, "icon/language"),
                    (
                        "options.accessibility",
                        "Accessibility Settings...",
                        7,
                        "icon/accessibility",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut button = b(
                        cx - 34.0 + i as f32 * 24.0,
                        y + 72.0,
                        20.0,
                        &lang::get_or(key, plain),
                        true,
                        id,
                    );
                    button.icon = Some((icon, 15.0));
                    buttons.push(button);
                }
                buttons
            }
            Mode::Select { selected, .. } => vec![
                b(
                    (w / 2.0).floor() - 154.0,
                    h - 52.0,
                    150.0,
                    &lang::get_or("selectWorld.select", "Play Selected World"),
                    selected.is_some(),
                    10,
                ),
                b(
                    (w / 2.0).floor() + 4.0,
                    h - 52.0,
                    150.0,
                    &lang::get_or("selectWorld.create", "Create New World"),
                    true,
                    11,
                ),
                b(
                    (w / 2.0).floor() - 154.0,
                    h - 28.0,
                    71.0,
                    &lang::get_or("selectWorld.edit", "Edit"),
                    selected.is_some(),
                    13,
                ),
                b(
                    (w / 2.0).floor() - 75.0,
                    h - 28.0,
                    71.0,
                    &lang::get_or("selectWorld.delete", "Delete"),
                    selected.is_some(),
                    14,
                ),
                b(
                    (w / 2.0).floor() + 4.0,
                    h - 28.0,
                    71.0,
                    &lang::get_or("selectWorld.recreate", "Re-Create"),
                    selected.is_some(),
                    15,
                ),
                b(
                    (w / 2.0).floor() + 83.0,
                    h - 28.0,
                    71.0,
                    &lang::get_or("gui.back", "Back"),
                    true,
                    12,
                ),
            ],
            Mode::Create {
                creative, options, ..
            } => create_world::buttons(w, h, *creative, options),
            Mode::CreateFlat { flat, .. } => flat::buttons(w, h, flat),
            Mode::Edit { world, name, .. } => {
                let x = (w / 2.0).floor() - 100.0;
                let top = edit_top(h);
                vec![
                    b(
                        x,
                        top + 64.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.resetIcon", "Reset Icon"),
                        self.saves.join(&world.folder).join("icon.png").is_file(),
                        30,
                    ),
                    b(
                        x,
                        top + 89.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.openFolder", "Open World Folder"),
                        true,
                        31,
                    ),
                    b(
                        x,
                        top + 114.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backup", "Make Backup"),
                        true,
                        32,
                    ),
                    b(
                        x,
                        top + 139.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.backupFolder", "Open Backups Folder"),
                        true,
                        33,
                    ),
                    b(
                        x,
                        top + 164.0,
                        200.0,
                        &lang::get_or("selectWorld.edit.optimize", "Optimize World"),
                        false,
                        34,
                    ),
                    b(
                        x,
                        top + 214.0,
                        98.0,
                        &lang::get_or("selectWorld.edit.save", "Save"),
                        !name.trim().is_empty(),
                        35,
                    ),
                    b(
                        x + 102.0,
                        top + 214.0,
                        98.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        36,
                    ),
                ]
            }
            Mode::Delete { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.deleteButton", "Delete"),
                        true,
                        37,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        38,
                    ),
                ]
            }
            Mode::Error { incompatible, .. } => {
                let lines = self.message_lines.get();
                let y = if *incompatible {
                    // AlertScreen.init: the button follows the text, kept inside the screen.
                    (90.0 + lines as f32 * 9.0 + 12.0)
                        .max(h / 6.0 + 96.0)
                        .min(h - 24.0)
                } else {
                    confirm_top(h, lines) + 41.0 + lines as f32 * 9.0
                };
                vec![b(
                    (w / 2.0).floor() - 75.0,
                    y,
                    150.0,
                    &lang::get_or("gui.back", "Back"),
                    true,
                    39,
                )]
            }
            Mode::BackupBeforeUpgrade { .. } => {
                // BackupConfirmScreen.init: the buttons sit under the text at fixed offsets.
                let y = 109.0 + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "selectWorld.backupJoinConfirmButton",
                            "Create Backup and Load",
                        ),
                        true,
                        40,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        41,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        42,
                    ),
                ]
            }
            Mode::MultiplayerWarning { .. } => {
                let lines = self.message_lines.get();
                let y = warning_top(h, lines) + 82.0 + lines as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        y,
                        150.0,
                        &lang::get_or("gui.proceed", "Proceed"),
                        true,
                        70,
                    ),
                    b(
                        (w / 2.0).floor() + 4.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        71,
                    ),
                ]
            }
            Mode::Backup { .. } => Vec::new(),
            Mode::Multiplayer(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| b(x, y, bw, &label, enabled, id))
                .collect(),
        }
    }
    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        if let Some(rules) = self.rules.borrow_mut().as_mut() {
            return rules.draw(batch, atlas, cursor);
        }
        match &self.mode {
            Mode::Main => {
                // LogoRenderer: the logo's top 44 of 64 rows, then the edition
                // banner's top 14 of 16 rows overlapping its bottom by 7.
                batch.sprite_region(
                    atlas,
                    "gui/title/minecraft",
                    [(w / 2.0).floor() - 128.0, 30.0, 256.0, 44.0],
                    256.0,
                    64.0,
                );
                batch.sprite_region(
                    atlas,
                    crate::title_edition::SPRITE,
                    [(w / 2.0).floor() - 64.0, 30.0 + 44.0 - 7.0, 128.0, 14.0],
                    128.0,
                    16.0,
                );
                if !hide_splash && let Some(splash) = self.splash.as_deref() {
                    let tw = HudBatch::text_width(atlas, splash);
                    let start = batch.vertices.len();
                    batch.text(
                        atlas,
                        splash,
                        -(tw / 2.0).floor(),
                        -8.0,
                        rgba(255, 255, 0, 255),
                        true,
                    );
                    let scale = (1.8
                        - (self.started.elapsed().as_secs_f32() * std::f32::consts::TAU)
                            .sin()
                            .abs()
                            * 0.1)
                        * 100.0
                        / (tw + 32.0);
                    let (sin, cos) = (-20.0f32).to_radians().sin_cos();
                    for v in &mut batch.vertices[start..] {
                        let [x, y] = v.pos;
                        v.pos = [
                            w / 2.0 + 123.0 + scale * (x * cos - y * sin),
                            69.0 + scale * (x * sin + y * cos),
                        ];
                    }
                }
                let version = format!("Horizon {} (Rust)", horizon_core::VANILLA_VERSION);
                batch.text(atlas, &version, 2.0, h - 10.0, u32::MAX, true);
                // A nightly names its build in the top-left corner, the one spot no button reaches at any GUI
                // scale, so screenshots and reports say which build they come from.
                if let Some(nightly) = horizon_core::release::nightly_label() {
                    batch.text(atlas, &nightly, 2.0, 2.0, u32::MAX, true);
                }
                let notice = "Not affiliated with Mojang or Microsoft";
                let tw = HudBatch::text_width(atlas, notice);
                batch.text(atlas, notice, w - tw - 2.0, h - 10.0, u32::MAX, true);
            }
            Mode::Select { worlds, selected } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                centered(
                    atlas,
                    batch,
                    &lang::get_or("selectWorld.title", "Select World"),
                    8.0,
                    u32::MAX,
                );
                self.caret.borrow_mut().draw(
                    batch,
                    atlas,
                    (SEARCH, &self.search, 50),
                    [(w / 2.0).floor() - 100.0, 21.0, 200.0, 20.0],
                    self.search_focused,
                    self.started,
                );
                if self.search.is_empty() && !self.search_focused {
                    batch.text_styled(
                        atlas,
                        &Text::literal(lang::get_or("gui.selectWorld.search", "Search..."))
                            .italic()
                            .with_color(crate::text::GRAY),
                        (w / 2.0).floor() - 96.0,
                        27.0,
                        u32::MAX,
                        true,
                    );
                }
                let (top, bottom, left, content) = self.world_list(worlds, (w, h));
                let mut scroll = self.scroll.get();
                scroll.clamp(top, bottom, content);
                self.scroll.set(scroll);
                let offset = scroll.amount.trunc();
                // AbstractSelectionList: background, rows clipped to the
                // list, separators, scrollbar.
                crate::ui::list_background(batch, atlas, top, bottom, false);
                let first = batch.vertices.len();
                // Entry content sits 2 pixels inside its row.
                let x = left + 2.0;
                let mut slot = 0;
                let hovered = self.world_row_at(worlds, cursor, (w, h));
                for (row, (i, world)) in self.visible_worlds(worlds).enumerate() {
                    let y = top + 4.0 + row as f32 * ROW_HEIGHT - offset;
                    if y + 32.0 <= top || y >= bottom {
                        continue;
                    }
                    let mid = y - 2.0 + ROW_HEIGHT / 2.0;
                    if mid > top && mid < bottom {
                        let rect = [x - 2.0, y - 2.0, ROW_WIDTH, ROW_HEIGHT];
                        // The control socket's widget tree names the row.
                        crate::widgets::record_as(
                            &world.name,
                            rect,
                            true,
                            crate::widgets::Kind::Row,
                        );
                        crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    }
                    // extractSelection: white once the list has focus.
                    if *selected == Some(i) {
                        batch.fill(atlas, x - 2.0, y - 2.0, ROW_WIDTH, ROW_HEIGHT, u32::MAX);
                        batch.fill(
                            atlas,
                            x - 1.0,
                            y - 1.0,
                            ROW_WIDTH - 2.0,
                            ROW_HEIGHT - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    batch.text(atlas, &world.name, x + 35.0, y + 1.0, u32::MAX, true);
                    let date: chrono::DateTime<chrono::Local> = world.modified.into();
                    let line = format!("{} ({})", world.folder, date.format("%d/%m/%Y, %H:%M"));
                    batch.text(
                        atlas,
                        &line,
                        x + 35.0,
                        y + 12.0,
                        rgba(128, 128, 128, 255),
                        true,
                    );
                    let details = world_info(world);
                    batch.text_styled(
                        atlas,
                        &details,
                        x + 35.0,
                        y + 21.0,
                        rgba(128, 128, 128, 255),
                        true,
                    );
                    self.icon(atlas, batch, world, slot, x, y);
                    slot = (slot + 1) % crate::hud::WORLD_ICON_SLOTS;
                    // WorldListEntry.extractContent: a hovered row dims its
                    // icon under the join arrow, lit under the pointer.
                    if let Some((_, (rx, ry))) = hovered.filter(|(r, _)| *r == row) {
                        batch.fill(atlas, x, y, 32.0, 32.0, 0xA0909090);
                        let on_icon = (0.0..32.0).contains(&rx) && (0.0..32.0).contains(&ry);
                        let sprite = if on_icon {
                            "gui/sprites/world_list/join_highlighted"
                        } else {
                            "gui/sprites/world_list/join"
                        };
                        batch.sprite_scaled(atlas, sprite, x, y, 32.0, 32.0, u32::MAX);
                    }
                }
                // Clip list quads, including their UVs, to the content area.
                for quad in batch.vertices[first..].as_chunks_mut::<6>().0 {
                    let low = quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
                    let high = quad
                        .iter()
                        .map(|v| v.pos[1])
                        .fold(f32::NEG_INFINITY, f32::max);
                    let uv_top = quad.iter().map(|v| v.uv[1]).fold(f32::INFINITY, f32::min);
                    let uv_bot = quad
                        .iter()
                        .map(|v| v.uv[1])
                        .fold(f32::NEG_INFINITY, f32::max);
                    for v in quad {
                        v.pos[1] = v.pos[1].clamp(top, bottom);
                        if high > low {
                            v.uv[1] = uv_top + (uv_bot - uv_top) * (v.pos[1] - low) / (high - low);
                        }
                    }
                }
                crate::ui::separators(batch, atlas, top, bottom, false);
                crate::ui::scrollbar(
                    batch,
                    atlas,
                    left + ROW_WIDTH + 8.0,
                    top,
                    bottom,
                    content,
                    scroll.amount,
                );
            }
            Mode::Create {
                name,
                seed,
                focus,
                options,
                ..
            } => self.draw_create(atlas, batch, cursor, name, seed, *focus, options),
            Mode::CreateFlat { flat, .. } => flat::draw(self, atlas, batch, cursor, flat),
            Mode::Edit { name, focused, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                centered(
                    atlas,
                    batch,
                    &lang::get_or("selectWorld.edit.title", "Edit World"),
                    15.0,
                    u32::MAX,
                );
                let top = edit_top(h);
                batch.text(
                    atlas,
                    &lang::get_or("selectWorld.enterName", "World Name"),
                    (w / 2.0).floor() - 100.0,
                    top + 25.0,
                    rgba(160, 160, 160, 255),
                    true,
                );
                self.caret.borrow_mut().draw(
                    batch,
                    atlas,
                    (EDIT_NAME, name, 32),
                    [(w / 2.0).floor() - 100.0, top + 39.0, 200.0, 20.0],
                    *focused,
                    self.started,
                );
            }
            Mode::Delete { world } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let message = translate(
                    "selectWorld.deleteWarning",
                    "'%s' will be lost forever! (A long time!)",
                    &[&world.name],
                );
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                let top = confirm_top(h, lines.len());
                centered(
                    atlas,
                    batch,
                    &lang::get_or(
                        "selectWorld.deleteQuestion",
                        "Are you sure you want to delete this world?",
                    ),
                    top,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::BackupBeforeUpgrade { downgrade, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                // BackupStatus.DOWNGRADE is severe: its question is red (-2142128).
                let (question, warning, color) = match downgrade {
                    Some(version) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.downgrade",
                            "Downgrading a world is not supported",
                        ),
                        translate(
                            "selectWorld.backupWarning.downgrade",
                            "This world was last played in version %s; you are on version %s. Downgrading a world could cause corruption - we cannot guarantee that it will load or work. If you still want to continue, please make a backup.",
                            &[version, &crate::save::VERSION_NAME],
                        ),
                        rgba(223, 80, 80, 255),
                    ),
                    None => (
                        lang::get_or(
                            "selectWorld.backupQuestion.file_fixing_required",
                            "Create a backup before upgrading this world?",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.file_fixing_required",
                            "This world needs to be upgraded before you can play it. We strongly suggest creating a backup before continuing in case you experience world corruption.",
                        ),
                        u32::MAX,
                    ),
                };
                let lines = wrapped(atlas, &warning, w - 50.0);
                self.message_lines.set(lines.len());
                // BackupConfirmScreen.extractRenderState: the title at 50, the text from 70.
                centered(atlas, batch, &question, 50.0, color);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, 70.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::Backup { .. } => {
                self.backup_shown.set(true);
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                centered(
                    atlas,
                    batch,
                    &lang::get_or("selectWorld.waitingForBackup.title", "Creating Backup"),
                    80.0,
                    u32::MAX,
                );
                let dots = ["O o o", "o O o", "o o O", "o O o"]
                    [(self.started.elapsed().as_millis() / 300 % 4) as usize];
                centered(atlas, batch, dots, 120.0, rgba(128, 128, 128, 255));
                let message = lang::get_or(
                    "selectWorld.waitingForBackup.message",
                    "Depending on the size of your world, this may take a while. Please do not close the game or shut off your device.",
                );
                for (i, line) in wrapped(atlas, &message, 360.0).iter().enumerate() {
                    centered(
                        atlas,
                        batch,
                        line,
                        132.0 + i as f32 * 9.0,
                        rgba(170, 170, 170, 255),
                    );
                }
            }
            Mode::Error {
                message,
                incompatible,
                ..
            } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let lines: Vec<String> = message
                    .split('\n')
                    .flat_map(|line| wrapped(atlas, line, w - 50.0))
                    .collect();
                self.message_lines.set(lines.len());
                // AlertScreen (an incompatible world): the red title at 70, the text at 90.
                let (top, text_top, key, fallback, color) = if *incompatible {
                    (
                        70.0,
                        90.0,
                        "selectWorld.incompatible.title",
                        "Incompatible version",
                        rgba(255, 0, 0, 255),
                    )
                } else {
                    let top = confirm_top(h, lines.len());
                    (
                        top,
                        top + 17.0,
                        "selectWorld.access_failure",
                        "Failed to access world",
                        u32::MAX,
                    )
                };
                centered(atlas, batch, &lang::get_or(key, fallback), top, color);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, text_top + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::MultiplayerWarning { check } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let message = lang::get_or(
                    "multiplayerWarning.message",
                    "Caution: Online play is offered by third-party servers that are not owned, operated, or supervised by Mojang Studios or Microsoft. During online play, you may be exposed to unmoderated chat messages or other types of user-generated content that may not be suitable for everyone.",
                );
                // The message sits in a text box `width - 100` wide with 4 px
                // inside, left aligned (FittingMultiLineTextWidget).
                let box_w = w - 100.0;
                let lines = wrapped(atlas, &message, box_w - 8.0);
                self.message_lines.set(lines.len());
                let top = warning_top(h, lines.len());
                let header = Text::translate("multiplayerWarning.header", &[]).bold();
                batch.text_styled(
                    atlas,
                    &header,
                    (w / 2.0).floor() - (atlas.font.width_text(&header) / 2.0).floor(),
                    top,
                    u32::MAX,
                    true,
                );
                let box_x = ((w - box_w) / 2.0).floor();
                let box_y = top + 29.0;
                let box_h = lines.len() as f32 * 9.0 + 8.0;
                crate::ui::nine_slice(
                    batch,
                    atlas,
                    "gui/sprites/widget/text_field",
                    box_x,
                    box_y,
                    box_w,
                    box_h,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    batch.text(
                        atlas,
                        line,
                        box_x + 4.0,
                        box_y + 4.0 + i as f32 * 9.0,
                        u32::MAX,
                        true,
                    );
                }
                // Checkbox: the box, its label right of it (as the dialogs draw theirs).
                let label =
                    lang::get_or("multiplayerWarning.check", "Do not show this screen again");
                let width = BOX + 4.0 + HudBatch::text_width(atlas, &label).ceil();
                let x = (w / 2.0).floor() - (width / 2.0).floor();
                let y = top + 57.0 + lines.len() as f32 * 9.0;
                self.warning_check.set([x, y, width, BOX]);
                let hover = (x..x + width).contains(&cursor.0) && (y..y + BOX).contains(&cursor.1);
                let sprite = match (*check, hover) {
                    (true, true) => "gui/sprites/widget/checkbox_selected_highlighted",
                    (true, false) => "gui/sprites/widget/checkbox_selected",
                    (false, true) => "gui/sprites/widget/checkbox_highlighted",
                    (false, false) => "gui/sprites/widget/checkbox",
                };
                batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
                batch.text(
                    atlas,
                    &label,
                    x + BOX + 4.0,
                    y + 4.0,
                    rgba(224, 224, 224, 255),
                    true,
                );
            }
            Mode::Multiplayer(screen) => screen.draw(
                atlas,
                batch,
                &mut self.caret.borrow_mut(),
                self.started,
                cursor,
            ),
        }
        for b in self.buttons(w, h) {
            crate::widgets::record(&b.label, [b.x, b.y, b.w, 20.0], b.enabled);
            let inside =
                cursor.0 >= b.x && cursor.0 < b.x + b.w && cursor.1 >= b.y && cursor.1 < b.y + 20.0;
            crate::widgets::widget_cursor(inside, b.enabled);
            let hover = b.enabled && inside;
            let sprite = if !b.enabled {
                "gui/sprites/widget/button_disabled"
            } else if hover {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            // Nine-slice approximation: left and right halves of the 200-wide sprite.
            if let Some(r) = atlas.sprites.get(sprite).copied() {
                let half = b.w / 2.0;
                let mut left = r;
                left.u1 = r.u0 + (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x, b.y, half, 20.0, left, u32::MAX);
                let mut right = r;
                right.u0 = r.u1 - (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x + half, b.y, half, 20.0, right, u32::MAX);
            }
            let color = if b.enabled {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            if let Some((icon, size)) = b.icon {
                // CenteredIcon divides the button and sprite sizes separately:
                // a 15px icon in a 20px button starts at 10 - 7 = 3, not 2.
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/{icon}"),
                    b.x + (b.w / 2.0).floor() - (size / 2.0).floor(),
                    b.y + 10.0 - (size / 2.0).floor(),
                    size,
                    size,
                    u32::MAX,
                );
                continue;
            }
            let tw = HudBatch::text_width(atlas, &b.label);
            batch.text(
                atlas,
                &b.label,
                b.x + (b.w / 2.0).floor() - (tw.ceil() / 2.0).floor(),
                b.y + 6.0,
                color,
                true,
            );
        }
        if let Mode::Create {
            name,
            creative,
            options,
            ..
        } = &self.mode
        {
            self.create_tooltip(atlas, batch, cursor, name, *creative, options);
        }
        if let Mode::Multiplayer(screen) = &self.mode {
            screen.draw_tooltip(atlas, batch, cursor);
        }
        if let Some((slide, toast)) = &self.toast {
            let (title, message) = toast.text();
            let width = HudBatch::text_width(atlas, &title)
                .max(HudBatch::text_width(atlas, &message))
                .ceil()
                .max(160.0)
                + 30.0;
            // Toast.xPos: in from the right edge by the visible portion.
            let x = w - width * slide.portion;
            crate::toasts::background(atlas, batch, x, 0.0, width);
            batch.text(atlas, &title, x + 18.0, 7.0, rgba(255, 255, 0, 255), false);
            batch.text(atlas, &message, x + 18.0, 19.0, u32::MAX, false);
        }
    }
    /// The action and whether to play `AbstractWidget.playButtonClickSound`.
    /// Navigation buttons can return `Action::None` while still sounding.
    pub fn click(&mut self, x: f32, y: f32, size: (f32, f32)) -> (Action, bool) {
        self.remember_direct_address();
        // The game rules take the press in `press_rules`, which has the atlas.
        if self.rules.get_mut().is_some() {
            return (Action::None, false);
        }
        let (w, h) = (size.0.ceil(), size.1.ceil());
        // MenuTabBar: a new tab clears the focus and clicks.
        if let Mode::Create { focus, options, .. } = &mut self.mode
            && let Some(tab) = create_world::tab_at(w, (x, y))
        {
            let changed = options.tab != tab;
            if changed {
                options.tab = tab;
                *focus = 2;
            }
            return (Action::None, changed);
        }
        // The SafetyScreen's box toggles where it was last drawn.
        if let Mode::MultiplayerWarning { check } = &mut self.mode {
            let [bx, by, bw, bh] = self.warning_check.get();
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                *check = !*check;
                return (Action::None, true);
            }
        }
        let hit = self
            .buttons(w, h)
            .into_iter()
            .find(|b| b.enabled && x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + 20.0)
            .map(|b| b.id);
        if matches!(self.mode, Mode::CreateFlat { .. }) {
            self.flat_click(x, y, (w, h), hit);
            return (Action::None, hit.is_some());
        }
        let action = self.click_action(x, y, (w, h), hit);
        // World list selection is silent; launching its selected entry sounds.
        let sound = hit.is_some() || matches!(action, Action::Play { .. });
        (action, sound)
    }
    fn click_action(&mut self, x: f32, y: f32, (w, h): (f32, f32), hit: Option<u8>) -> Action {
        match (&mut self.mode, hit) {
            (Mode::Main, Some(1)) => {
                self.enter_singleplayer();
                Action::None
            }
            (Mode::Main, Some(2)) => Action::Quit,
            (Mode::Main, Some(4)) => {
                // TitleScreen.init: SafetyScreen first, unless it was turned off.
                self.mode = if self.skip_multiplayer_warning {
                    Mode::Multiplayer(crate::multiplayer::Screen::list(&self.saves))
                } else {
                    Mode::MultiplayerWarning { check: false }
                };
                Action::None
            }
            (Mode::MultiplayerWarning { check }, Some(70)) => {
                // SafetyScreen Proceed: the box saves the choice.
                if *check {
                    self.skip_multiplayer_warning = true;
                }
                self.mode = Mode::Multiplayer(crate::multiplayer::Screen::list(&self.saves));
                Action::None
            }
            (Mode::MultiplayerWarning { .. }, Some(71)) => {
                self.mode = Mode::Main;
                Action::None
            }
            (Mode::Multiplayer(_), hit) => match self.multiplayer_click(hit, x, y, (w, h)) {
                crate::multiplayer::Outcome::Back => {
                    self.mode = Mode::Main;
                    Action::None
                }
                crate::multiplayer::Outcome::Connect(address) => Action::Connect { address },
                crate::multiplayer::Outcome::None => Action::None,
            },
            (Mode::Main, Some(5)) => {
                self.mode = Mode::Multiplayer(crate::multiplayer::Screen::friend());
                Action::None
            }
            (Mode::Main, Some(3)) => Action::Options,
            (Mode::Main, Some(7)) => Action::Accessibility,
            (Mode::Main, Some(6)) => Action::Language,
            (Mode::Select { worlds, selected }, Some(10)) => {
                match selected.and_then(|i| worlds.get(i)).cloned() {
                    Some(world) => self.play(world),
                    None => Action::None,
                }
            }
            (Mode::BackupBeforeUpgrade { world, .. }, Some(40)) => {
                let world = world.clone();
                self.backup_then_open = true;
                self.start_backup(world);
                Action::None
            }
            (Mode::BackupBeforeUpgrade { world, .. }, Some(41)) => Action::Play {
                folder: world.folder.clone(),
            },
            (Mode::BackupBeforeUpgrade { world, .. }, Some(42)) => {
                let folder = world.folder.clone();
                self.return_to_list(Some(&folder));
                Action::None
            }
            (Mode::Select { .. }, Some(11)) => {
                self.create_return_to_main = false;
                self.mode = Self::new_world();
                Action::None
            }
            (Mode::Select { worlds, selected }, Some(id @ 13..=15)) => {
                let Some(world) = selected.and_then(|i| worlds.get(i)).cloned() else {
                    return Action::None;
                };
                match id {
                    13 => {
                        self.mode = Mode::Edit {
                            name: world.name.clone(),
                            world,
                            focused: true,
                        }
                    }
                    14 => self.mode = Mode::Delete { world },
                    _ => {
                        if let Some(level) = Level::read(&self.saves.join(&world.folder)) {
                            self.create_return_to_main = false;
                            let options = WorldOptions::of_world(
                                &level,
                                world.commands,
                                &self.saves.join(&world.folder),
                            );
                            self.mode = Mode::Create {
                                name: level.name,
                                seed: level.seed.to_string(),
                                creative: level.creative,
                                focus: 0,
                                options,
                            };
                        }
                    }
                }
                Action::None
            }
            (Mode::Select { .. }, Some(12)) => {
                self.mode = Mode::Main;
                Action::None
            }
            (Mode::Select { .. }, None) => {
                self.search_focused =
                    (21.0..41.0).contains(&y) && (x - (w / 2.0).floor()).abs() < 100.0;
                let Mode::Select { worlds, .. } = &self.mode else {
                    unreachable!()
                };
                let (top, bottom, left, content) = self.world_list(worlds, (w, h));
                if self
                    .scroll
                    .get_mut()
                    .press((x, y), left + ROW_WIDTH + 8.0, top, bottom, content)
                {
                    return Action::None;
                }
                let Mode::Select { worlds, selected } = &self.mode else {
                    unreachable!()
                };
                let chosen = self
                    .world_row_at(worlds, (x, y), (w, h))
                    .and_then(|(row, rel)| {
                        let (index, world) = self.visible_worlds(worlds).nth(row)?;
                        Some((index, world.clone(), rel))
                    });
                if let Some((index, world, (rx, ry))) = chosen {
                    // WorldListEntry.mouseClicked: the icon joins at once.
                    let on_icon = (0.0..32.0).contains(&rx) && (0.0..32.0).contains(&ry);
                    if on_icon || *selected == Some(index) {
                        return self.play(world);
                    }
                    if let Mode::Select { selected, .. } = &mut self.mode {
                        *selected = Some(index);
                    }
                }
                Action::None
            }
            (Mode::Edit { world, name, .. }, Some(id @ (30..=36))) => {
                let world = world.clone();
                let name = name.clone();
                let result = match id {
                    30 => world_management::reset_icon(&self.saves, &world.folder),
                    31 => world_management::open_folder(&self.saves.join(&world.folder)),
                    32 => {
                        self.start_backup(world);
                        return Action::None;
                    }
                    33 => {
                        world_management::open_folder(&world_management::backups_path(&self.saves))
                    }
                    35 => world_management::rename(&self.saves, &world.folder, &name),
                    36 => {
                        self.return_to_list(Some(&world.folder));
                        return Action::None;
                    }
                    _ => return Action::None,
                };
                if let Err(error) = result {
                    self.failed(error);
                } else if id == 35 {
                    self.return_to_list(Some(&world.folder));
                } else if id == 30 {
                    self.icons.borrow_mut().clear();
                }
                Action::None
            }
            (Mode::Edit { focused, .. }, None) => {
                *focused = y >= edit_top(h) + 39.0
                    && y < edit_top(h) + 59.0
                    && (x - (w / 2.0).floor()).abs() < 100.0;
                Action::None
            }
            (Mode::Delete { .. }, Some(37)) => {
                self.confirm_delete();
                Action::None
            }
            (Mode::Delete { .. }, Some(38)) => {
                self.escape();
                Action::None
            }
            (Mode::Error { return_to, .. }, Some(39)) => {
                self.mode = *return_to.clone();
                Action::None
            }
            // CreateWorldScreen.openGameRulesScreen over a copy of the rules.
            (Mode::Create { options, .. }, Some(29)) => {
                *self.rules.get_mut() = Some(RulesScreen::for_creation(&options.rules));
                Action::None
            }
            (
                Mode::Create {
                    name,
                    seed,
                    creative,
                    focus,
                    options,
                },
                Some(26),
            ) => {
                let flat = flat::State::new(&self.resources, options.flat_settings.clone());
                self.mode = Mode::CreateFlat {
                    name: name.clone(),
                    seed: seed.clone(),
                    creative: *creative,
                    focus: *focus,
                    options: options.clone(),
                    flat,
                };
                Action::None
            }
            (
                Mode::Create {
                    creative, options, ..
                },
                Some(id),
            ) if !matches!(id, 21 | 22 | 31) => {
                options.press(id, creative);
                Action::None
            }
            (Mode::Create { .. }, Some(21)) => self.create_action(),
            (Mode::Create { options, .. }, Some(31)) => {
                if options.pack_dir.is_none() {
                    match crate::datapack::temp_folder() {
                        Ok(dir) => options.pack_dir = Some(dir),
                        Err(e) => {
                            log::warn!("Horizon: no folder for data packs: {e}");
                            return Action::None;
                        }
                    }
                }
                Action::DataPacks {
                    dir: options.pack_dir.clone().unwrap_or_default(),
                    selected: options.data_packs.clone(),
                }
            }
            (Mode::Create { .. }, Some(22)) => {
                self.cancel_create();
                Action::None
            }
            (Mode::Create { focus, options, .. }, None) => {
                *focus = create_world::field_at(w, h, options.tab, (x, y)).unwrap_or(2);
                Action::None
            }
            _ => Action::None,
        }
    }
    /// The focused text field, its id and maximum length (copy and paste).
    pub fn focused_field(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        if self.rules.get_mut().is_some() {
            return None;
        }
        match &mut self.mode {
            Mode::Multiplayer(screen) => screen.focused_text(),
            mode => caret_field(mode, self.search_focused, &mut self.search),
        }
    }
    /// Whether the focused field edits at its cursor.
    pub fn has_caret_field(&mut self) -> bool {
        // The game rules' boxes edit themselves (`edit_field`).
        if let Some(rules) = self.rules.get_mut() {
            return rules.typing();
        }
        caret_field(&mut self.mode, self.search_focused, &mut self.search).is_some()
    }
    /// Edit the focused field through its `EditBox`; false when there is none.
    fn edit_field(&mut self, f: impl FnOnce(&mut crate::ui::EditBox)) -> bool {
        if let Some(rules) = self.rules.get_mut() {
            rules.edit(f);
            return rules.typing();
        }
        let Some((id, value, max)) =
            caret_field(&mut self.mode, self.search_focused, &mut self.search)
        else {
            return false;
        };
        let before = value.clone();
        self.caret.get_mut().edit(id, value, max, f);
        if *value != before {
            self.search_changed();
        }
        true
    }
    /// EditBox.charTyped / insertText at the cursor.
    pub fn type_text(&mut self, text: &str) {
        self.edit_field(|edit| edit.insert_text(text));
    }
    pub fn backspace(&mut self) {
        self.field_key(winit::keyboard::KeyCode::Backspace, false, false);
    }
    /// EditBox.keyPressed: moving and deleting in the focused field.
    pub fn field_key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        let mut handled = false;
        self.edit_field(|edit| handled = edit.key(key, ctrl, shift));
        handled
    }
    /// Ctrl+A/C/V/X in the focused field.
    pub fn field_shortcut(
        &mut self,
        shortcut: crate::clipboard::EditShortcut,
        board: &mut dyn crate::clipboard::Clipboard,
    ) {
        self.edit_field(|edit| {
            edit.shortcut(shortcut, board);
        });
    }
    /// The focused field's box on screen.
    fn focused_rect(&self, (w, h): (f32, f32)) -> Option<[f32; 4]> {
        let x = (w / 2.0).floor() - 100.0;
        match &self.mode {
            Mode::Select { .. } if self.search_focused => Some([x, 21.0, 200.0, 20.0]),
            Mode::Edit { focused: true, .. } => Some([x, edit_top(h) + 39.0, 200.0, 20.0]),
            Mode::Create { focus, options, .. } => create_world::field_rect(w, h, options.tab)
                .filter(|(id, _)| id == focus)
                .map(|(_, rect)| rect),
            Mode::Multiplayer(screen) => screen.focused_box((w, h)),
            _ => None,
        }
    }
    /// A left press after `click` focused a field: the cursor goes under the
    /// pointer (`EditBox.onClick`).
    pub fn press_field(
        &mut self,
        atlas: &HudAtlas,
        (x, y): (f32, f32),
        size: (f32, f32),
        shift: bool,
    ) {
        let (w, h) = (size.0.ceil(), size.1.ceil());
        if self.rules.get_mut().is_some() {
            return;
        }
        let Some(rect @ [fx, fy, fw, fh]) = self.focused_rect((w, h)) else {
            return;
        };
        if !crate::ui::hovered((x, y), fx, fy, fw, fh) {
            return;
        }
        if let Some(field) = caret_field(&mut self.mode, self.search_focused, &mut self.search) {
            self.caret.get_mut().press(atlas, field, rect, x, shift);
        }
    }
    /// The pointer moved with the button held (`EditBox.onDrag`).
    pub fn drag_field(&mut self, atlas: &HudAtlas, x: f32) {
        if self.rules.get_mut().is_some() {
            return;
        }
        if let Some(field) = caret_field(&mut self.mode, self.search_focused, &mut self.search) {
            self.caret.get_mut().drag(atlas, field, x);
        }
    }
    /// The button was released: drags stop selecting.
    pub fn release_field(&mut self) {
        self.caret.get_mut().release();
    }
    /// Clipboard replacements also invalidate a selection in the filtered list.
    pub fn search_changed(&mut self) {
        if self.search_focused
            && let Mode::Select { selected, .. } = &mut self.mode
        {
            *selected = None;
            self.scroll.set(crate::ui::Scroll::default());
        }
    }
    pub fn confirm_edit(&mut self) {
        let Mode::Edit {
            world,
            name,
            focused: true,
        } = &self.mode
        else {
            return;
        };
        if name.trim().is_empty() {
            return;
        }
        let folder = world.folder.clone();
        match world_management::rename(&self.saves, &folder, name) {
            Ok(()) => self.return_to_list(Some(&folder)),
            Err(error) => self.failed(error),
        }
    }
    /// `CreateWorldScreen.keyPressed`, `DirectJoinServerScreen.keyPressed` and
    /// `WorldListEntry.keyPressed` on Enter (Space too on the list): Create New
    /// World creates, a rename saves, Direct Connection joins, the selected
    /// world joins.
    pub fn confirm_key(&mut self) -> Action {
        self.remember_direct_address();
        if self.rules.get_mut().is_some() {
            return Action::None;
        }
        if matches!(self.mode, Mode::Edit { .. }) {
            self.confirm_edit();
            return Action::None;
        }
        if self.list_focused() {
            return self.join_selected();
        }
        match &self.mode {
            Mode::Create { .. } => self.create_action(),
            Mode::Multiplayer(screen) => match screen.confirm() {
                crate::multiplayer::Outcome::Connect(address) => Action::Connect { address },
                _ => Action::None,
            },
            _ => Action::None,
        }
    }
    /// The world list has the keyboard: no search box focus.
    pub fn list_focused(&self) -> bool {
        matches!(self.mode, Mode::Select { .. }) && !self.search_focused
    }
    /// `WorldListEntry.keyPressed`: the selected world joins, as its click does.
    fn join_selected(&mut self) -> Action {
        let Mode::Select { worlds, selected } = &self.mode else {
            return Action::None;
        };
        let Some(world) = selected.and_then(|i| worlds.get(i)).cloned() else {
            return Action::None;
        };
        self.play(world)
    }
    pub fn tab(&mut self) {
        if self.rules.get_mut().is_some() {
            return;
        }
        if let Mode::Edit { focused, .. } = &mut self.mode {
            *focused = !*focused;
        }
        if matches!(self.mode, Mode::Select { .. }) {
            self.search_focused = !self.search_focused;
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.tab();
        }
        // The open tab's one text box, then nothing.
        if let Mode::Create { focus, options, .. } = &mut self.mode {
            let field = if options.tab == 1 { 1 } else { 0 };
            *focus = if *focus == field || options.tab == 2 {
                2
            } else {
                field
            };
        }
    }
    /// The server auto-reconnect retries now, once its countdown is over.
    pub fn due_retry(&mut self) -> Option<crate::reconnect::Retry> {
        match &mut self.mode {
            Mode::Multiplayer(screen) => screen.due_retry(),
            _ => None,
        }
    }
    pub(crate) fn remember_direct_address(&mut self) {
        if let Mode::Multiplayer(crate::multiplayer::Screen::Direct { address }) = &self.mode {
            self.last_server.clone_from(address);
        }
    }
    pub(crate) fn multiplayer_click(
        &mut self,
        hit: Option<u8>,
        x: f32,
        y: f32,
        size: (f32, f32),
    ) -> crate::multiplayer::Outcome {
        let Mode::Multiplayer(screen) = &mut self.mode else {
            return crate::multiplayer::Outcome::None;
        };
        let outcome = screen.click(hit, x, y, size, &self.saves);
        if hit == Some(41)
            && let crate::multiplayer::Screen::Direct { address } = screen
        {
            address.clone_from(&self.last_server);
        }
        outcome
    }
    /// A left press on the game rules (they need the atlas for text): whether
    /// it lands on a button, for its sound.
    pub fn press_rules(&mut self, atlas: &HudAtlas, cursor: (f32, f32), size: (f32, f32)) -> bool {
        let size = (size.0.ceil(), size.1.ceil());
        let Some(rules) = self.rules.get_mut() else {
            return false;
        };
        let sound = rules.clicks_button(atlas, cursor, size);
        let exit = rules.click(atlas, cursor, size);
        if let Some(exit) = exit {
            self.close_rules(exit);
        }
        sound
    }
    /// WorldCreationGameRulesScreen's exit callback: Done keeps the edited
    /// rules (`setGameRules`), Cancel drops them.
    fn close_rules(&mut self, exit: Exit) {
        *self.rules.get_mut() = None;
        if let (Exit::Apply(changes), Mode::Create { options, .. }) = (exit, &mut self.mode) {
            for (name, value) in changes {
                options.rules.set(name, value);
            }
        }
    }
    pub fn escape(&mut self) {
        if let Some(rules) = self.rules.get_mut() {
            if let Some(exit) = rules.escape() {
                self.close_rules(exit);
            }
            return;
        }
        self.remember_direct_address();
        if let Mode::Multiplayer(screen) = &mut self.mode
            && !screen.escape(&self.saves)
        {
            return;
        }
        match &self.mode {
            Mode::Backup { .. } => {}
            Mode::Edit { world, .. }
            | Mode::Delete { world }
            | Mode::BackupBeforeUpgrade { world, .. } => {
                let folder = world.folder.clone();
                self.return_to_list(Some(&folder));
            }
            Mode::Error { return_to, .. } => self.mode = *return_to.clone(),
            Mode::CreateFlat { .. } => {
                if matches!(&self.mode, Mode::CreateFlat { flat, .. } if flat.page == flat::Page::Presets)
                {
                    if let Mode::CreateFlat { flat, .. } = &mut self.mode {
                        flat.back_to_layers();
                    }
                } else {
                    self.close_flat(false);
                }
            }
            Mode::Create { .. } => self.cancel_create(),
            _ => self.mode = Mode::Main,
        }
    }
    /// `JoinMultiplayerScreen.keyPressed`: F5 refreshes the server list as
    /// Refresh does (`refreshServerList`).
    pub fn refresh_servers(&mut self) {
        if let Mode::Multiplayer(screen @ crate::multiplayer::Screen::List { .. }) = &mut self.mode
        {
            *screen = crate::multiplayer::Screen::list(&self.saves);
        }
    }
}
fn centered(atlas: &HudAtlas, batch: &mut HudBatch, text: &str, y: f32, color: u32) {
    batch.text(
        atlas,
        text,
        (batch.width.ceil() / 2.0).floor()
            - (HudBatch::text_width(atlas, text).ceil() / 2.0).floor(),
        y,
        color,
        true,
    );
}
fn wrapped(atlas: &HudAtlas, text: &str, width: f32) -> Vec<String> {
    atlas
        .font
        .split(&crate::text::Text::literal(text), width.max(1.0))
        .iter()
        .map(|t| t.plain())
        .collect()
}
const SEARCH: crate::clipboard::FieldId = ("world.search", 0);
const EDIT_NAME: crate::clipboard::FieldId = ("world.edit-name", 0);
const WORLD_NAME: crate::clipboard::FieldId = ("world.name", 0);
const WORLD_SEED: crate::clipboard::FieldId = ("world.seed", 0);
const FLAT_SHARE: crate::clipboard::FieldId = ("world.flat-share", 0);
/// The focused field of the world screens and the server screens: the list's
/// search, a world name or seed, or Add Server, Direct Connection and Join a
/// Friend.
fn caret_field<'a>(
    mode: &'a mut Mode,
    search_focused: bool,
    search: &'a mut String,
) -> Option<(crate::clipboard::FieldId, &'a mut String, usize)> {
    match mode {
        Mode::Select { .. } if search_focused => Some((SEARCH, search, 50)),
        Mode::Edit {
            name,
            focused: true,
            ..
        } => Some((EDIT_NAME, name, 32)),
        Mode::Create {
            name, seed, focus, ..
        } => match focus {
            0 => Some((WORLD_NAME, name, 32)),
            1 => Some((WORLD_SEED, seed, 32)),
            _ => None,
        },
        Mode::Multiplayer(screen) => screen.focused_text(),
        Mode::CreateFlat { flat, .. } if flat.page == flat::Page::Presets && flat.share_focused => {
            Some((FLAT_SHARE, &mut flat.share, 1230))
        }
        _ => None,
    }
}

#[cfg(test)]
mod classic_tests {
    use super::*;

    #[test]
    fn classic_main_menu_icons_route_language_and_accessibility() {
        let mut title = Title::new(
            PathBuf::new(),
            Path::new("/nonexistent/horizon-test-resources"),
        );
        for (w, h) in [(960.0, 523.0), (480.0, 320.0), (427.0, 240.0)] {
            let buttons = title.buttons(w, h);
            let cx = (w / 2.0).floor();
            let y = (h / 4.0).floor() + 48.0;
            for (id, x, enabled, icon) in [
                (5, cx - 34.0, true, "friends/friends"),
                (6, cx - 10.0, true, "icon/language"),
                (7, cx + 14.0, true, "icon/accessibility"),
            ] {
                let b = buttons.iter().find(|b| b.id == id).unwrap();
                assert_eq!(
                    (b.x, b.y, b.w, b.enabled, b.icon),
                    (x, y + 72.0, 20.0, enabled, Some((icon, 15.0)))
                );
                assert_eq!(
                    title.click(x + 10.0, y + 82.0, (w, h)),
                    match id {
                        6 => (Action::Language, true),
                        7 => (Action::Accessibility, true),
                        _ => (Action::None, true),
                    }
                );
                if id == 5 {
                    assert!(matches!(
                        title.mode,
                        Mode::Multiplayer(crate::multiplayer::Screen::Friend { .. })
                    ));
                    title.mode = Mode::Main;
                }
            }
            for id in [2, 3] {
                assert_eq!(buttons.iter().find(|b| b.id == id).unwrap().y, y + 96.0);
            }
            // The four-pixel gaps around icons must not activate a button.
            assert_eq!(
                title.click(cx + 12.0, y + 82.0, (w, h)),
                (Action::None, false)
            );
            assert_eq!(
                title.click(cx + 24.0, y + 92.0, (w, h)),
                (Action::None, false)
            );
        }
    }

    #[test]
    fn splash_selection_filters_the_forbidden_hash_and_has_no_fallback() {
        let hidden = "This message will never appear on the splash screen, isn't that weird?";
        assert_eq!(
            horizon_core::random::java_string_hash(hidden.encode_utf16()),
            125780783
        );
        let text = format!("\n First! \r\n{hidden}\nSecond!\n");
        for index in 0..6 {
            assert_eq!(
                choose_splash(&text, index, (10, 15)).as_deref(),
                Some(["", "First!", "Second!"][index % 3])
            );
        }
        assert_eq!(choose_splash(" \r\n", 42, (10, 15)).as_deref(), Some(""));
        for text in ["", hidden] {
            assert_eq!(choose_splash(text, 42, (10, 15)), None);
        }
    }

    #[test]
    fn special_date_splashes_override_resources_even_when_the_list_is_empty() {
        for (date, expected) in [
            ((12, 24), "Merry X-mas!"),
            ((1, 1), "Happy new year!"),
            ((10, 31), "OOoooOOOoooo! Spooky!"),
        ] {
            for text in ["", "Ordinary splash!"] {
                assert_eq!(choose_splash(text, 0, date).as_deref(), Some(expected));
            }
        }
    }

    #[test]
    fn classic_logo_and_icon_geometry_matches_minecraft() {
        let root = Path::new("/nonexistent/horizon-test-resources");
        let mut atlas = HudAtlas::build(root).unwrap();
        let r = crate::hud::Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1024,
            h: 256,
        };
        atlas.sprites.insert("gui/title/minecraft".into(), r);
        atlas.sprites.insert(
            "gui/title/edition".into(),
            crate::hud::Rect { w: 128, h: 16, ..r },
        );
        for (i, icon) in ["friends/friends", "icon/language", "icon/accessibility"]
            .into_iter()
            .enumerate()
        {
            atlas.sprites.insert(
                format!("gui/sprites/{icon}"),
                crate::hud::Rect {
                    u0: 0.25 + i as f32 * 0.125,
                    w: 15,
                    h: 15,
                    ..r
                },
            );
        }
        let mut title = Title::new(PathBuf::new(), root);
        title.splash = Some("Visible splash!".into());
        let mut batch = HudBatch::new([1920, 1080]);
        title.draw(&atlas, &mut batch, (0.0, 0.0), false);
        let cx = (batch.width / 2.0).floor();
        assert_eq!(batch.vertices[0].pos, [cx - 128.0, 30.0]);
        assert_eq!(batch.vertices[2].pos, [cx + 128.0, 74.0]);
        assert_eq!(batch.vertices[2].uv, [1.0, 44.0 / 64.0]);
        assert_eq!(batch.vertices[6].pos, [cx - 64.0, 67.0]);
        assert_eq!(batch.vertices[8].pos, [cx + 64.0, 81.0]);
        assert_eq!(batch.vertices[8].uv, [1.0, 14.0 / 16.0]);
        for i in 0..3 {
            let quad = batch
                .vertices
                .as_chunks::<6>()
                .0
                .iter()
                .find(|quad| quad[0].uv == [0.25 + i as f32 * 0.125, 0.0])
                .unwrap();
            let x = cx - 34.0 + i as f32 * 24.0 + 3.0;
            let y = (batch.height / 4.0).floor() + 48.0 + 72.0 + 3.0;
            assert_eq!(quad[0].pos, [x, y]);
            assert_eq!(quad[2].pos, [x + 15.0, y + 15.0]);
            assert!(quad.iter().all(|v| v.color == u32::MAX));
        }
        assert!(
            batch.vertices[12..]
                .iter()
                .any(|v| v.color == rgba(255, 255, 0, 255))
        );
        let settings = crate::settings::Settings::parse("hideSplashTexts:true\n");
        for (hide_splash, splash) in [
            (
                settings.file.bool("hideSplashTexts"),
                Some("Hidden splash!".into()),
            ),
            (false, None),
        ] {
            title.splash = splash;
            let mut hidden = HudBatch::new([1920, 1080]);
            title.draw(&atlas, &mut hidden, (0.0, 0.0), hide_splash);
            assert!(
                !hidden
                    .vertices
                    .iter()
                    .any(|v| v.color == rgba(255, 255, 0, 255))
            );
        }
    }
}
