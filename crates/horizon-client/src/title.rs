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
use horizon_world::datafix::world_upgrader::{self, WorldUpgrader};
use std::path::{Path, PathBuf};
use std::{
    cell::{Cell, RefCell},
    sync::mpsc,
};

mod confirm_experiments;
mod create_world;
mod experiments;
mod flat;
mod onboarding;
#[cfg(test)]
mod tests;
pub mod world_management;
pub use create_world::{DIFFICULTIES, WorldOptions};
pub use flat::{FlatLayer, FlatSettings, State as FlatEditor};
pub use onboarding::Onboarding;

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
/// RecoverWorldDataScreen's column (title, message, issues, the restore row
/// and Back, 8 apart, centred on the screen) with `lines` lines of message
/// and issues: where the restore row starts.
fn recover_buttons_top(h: f32, lines: usize) -> f32 {
    let text = lines as f32 * 9.0;
    let top = ((h.ceil() - (9.0 + 8.0 + text + 8.0 + 8.0 + 20.0 + 8.0 + 20.0)) / 2.0).trunc();
    top + 9.0 + 8.0 + text + 8.0 + 8.0
}
/// `RecoverWorldDataScreen.buildInfo`: "State from <date>: " and the file's
/// issue; nothing for a level.dat_old that is just missing.
fn recover_info(dir: &Path, old: bool, issue: Option<&crate::save::DatIssue>) -> Option<Text> {
    use crate::save::DatIssue;
    if old && issue == Some(&DatIssue::Missing) {
        return None;
    }
    let file = dir.join(if old { "level.dat_old" } else { "level.dat" });
    let date = match std::fs::metadata(file).and_then(|m| m.modified()) {
        Ok(time) => {
            let date: chrono::DateTime<chrono::Local> = time.into();
            date.format("%d/%m/%Y, %H:%M").to_string()
        }
        Err(_) => lang::get_or("recover_world.state_entry.unknown", "unknown"),
    };
    let state = format_translation(
        &lang::get_or("recover_world.state_entry", "State from %s: "),
        &[Text::colored(date, GRAY)],
    );
    let issue = match issue {
        None => Text::colored(lang::get_or("recover_world.issue.none", "No issues"), GREEN),
        Some(DatIssue::Missing) => Text::colored(
            lang::get_or("recover_world.issue.missing_file", "Missing file"),
            RED,
        ),
        Some(DatIssue::Unreadable(why)) => Text::colored(why.clone(), RED),
    };
    Some(Text::empty().append(state).append(issue))
}
/// ChatFormatting GRAY, GREEN and RED.
const GRAY: u32 = 0xAAAAAA;
const GREEN: u32 = 0x55FF55;
const RED: u32 = 0xFF5555;
/// `RecoverWorldDataScreen.collectIssue` for level.dat and level.dat_old.
fn recover_issues(dir: &Path) -> [Option<crate::save::DatIssue>; 2] {
    [
        crate::save::level_dat_issue(dir, false),
        crate::save::level_dat_issue(dir, true),
    ]
}
/// The SafetyScreen's column (WarningScreen: title, 12 px around the message
/// box of its lines and 4 px padding, the checkbox, then the buttons, 8 px
/// apart): the title's top.
/// `LogoRenderer.extractRenderState`: the logo's top 44 of 64 rows, then the
/// edition banner's top 14 of 16 rows overlapping its bottom by 7.
fn draw_logo(atlas: &HudAtlas, batch: &mut HudBatch, w: f32) {
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
}

/// `OptimizeWorldScreen.DIMENSION_COLORS`.
fn dimension_color(level: &str) -> u32 {
    match level {
        world_upgrader::OVERWORLD => rgba(0x33, 0x66, 0x22, 255),
        world_upgrader::NETHER => rgba(0x66, 0x44, 0x33, 255),
        world_upgrader::END => rgba(0x77, 0x88, 0x55, 255),
        _ => rgba(0xdd, 0xdd, 0xdd, 255),
    }
}

/// English text of the status keys `UpgradeStatusTranslator` returns.
fn status_fallback(key: &str) -> &'static str {
    match key {
        "optimizeWorld.stage.failed" => "Failed! :(",
        "optimizeWorld.stage.upgrading.chunks" => "Upgrading all chunks...",
        "optimizeWorld.stage.upgrading.entities" => "Upgrading all entities...",
        "optimizeWorld.stage.upgrading.poi" => "Upgrading all points of interest...",
        "optimizeWorld.stage.finished.chunks" => "Finishing up upgrading chunks...",
        "optimizeWorld.stage.finished.entities" => "Finishing up upgrading entities...",
        "optimizeWorld.stage.finished.poi" => "Finishing up upgrading points of interest...",
        _ => "Counting chunks...",
    }
}

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

/// Why `WorldOpenFlows.askForBackup` asks before a world opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemIssue {
    /// `WorldOptions.isOldCustomizedWorld`.
    Customized,
    /// A `worldGenSettingsLifecycle` that is not stable.
    Experimental,
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
    /// `LevelSummary.isLocked`: another running copy has the world open.
    pub locked: bool,
    /// `LevelSummary.CorruptedLevelSummary`: level.dat did not read, so the row
    /// only knows its folder and offers recovery instead of play.
    pub corrupted: bool,
    /// `LevelSummary.isCompatible` false: the saved game version's name, for a world
    /// from another series.
    pub incompatible: Option<String>,
    /// `LevelSummary.requiresFileFixing`: opening the world upgrades its files first.
    pub file_fixing: bool,
    /// `BackupStatus.DOWNGRADE`: a newer version saved the world.
    pub downgrade: bool,
}
impl WorldEntry {
    /// `LevelSummary.isCompatible`: the saved series is this build's.
    fn compatible(&self) -> bool {
        self.incompatible.is_none()
    }
    /// `LevelSummary.isDisabled`.
    fn disabled(&self) -> bool {
        self.locked || !self.compatible()
    }
    /// `LevelSummary.primaryActionActive`: a corrupted world offers recovery.
    fn can_play(&self) -> bool {
        self.corrupted || !self.disabled()
    }
    /// `LevelSummary.canEdit` and `canRecreate`.
    fn can_edit(&self) -> bool {
        !self.corrupted && !self.disabled() && !self.file_fixing
    }
    /// `BackupStatus.shouldBackup`: the world needs its files fixed or comes from
    /// a newer version.
    fn should_backup(&self) -> bool {
        self.file_fixing || self.downgrade
    }
    /// `CorruptedLevelSummary`: named after its folder, nothing else known.
    fn corrupted(folder: String, modified: std::time::SystemTime) -> Self {
        Self {
            name: folder.clone(),
            folder,
            game_mode: 0,
            hardcore: false,
            modified,
            commands: false,
            version: String::new(),
            locked: false,
            corrupted: true,
            incompatible: None,
            file_fixing: false,
            downgrade: false,
        }
    }
}
/// The AlertScreen an `Error` shows in place of the access failure.
#[derive(Clone, Copy, Debug)]
pub enum Alert {
    /// The red `selectWorld.incompatible.title`.
    Incompatible,
    /// `selectWorld.recreate.error.title`.
    Recreate,
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
        /// With neither of the above: `WorldOpenFlows.askForBackup` over a customized or
        /// experimental world (`openWorldCheckWorldStemCompatibility`).
        stem: Option<StemIssue>,
    },
    /// EditWorldScreen's Optimize World: BackupConfirmScreen with the Erase Cached Data box.
    OptimizeConfirm {
        world: WorldEntry,
        /// The Edit World screen's name field, restored by Cancel.
        name: String,
        erase_cache: bool,
    },
    /// `OptimizeWorldScreen`: the progress of `Title.optimize`.
    Optimize {
        world: WorldEntry,
    },
    /// `openWorldCheckDiskSpace`'s ConfirmScreen: little space is left on the device.
    LowDiskSpace {
        world: WorldEntry,
    },
    Delete {
        world: WorldEntry,
    },
    /// RecoverWorldDataScreen: opening a world whose level.dat does not read
    /// (WorldOpenFlows.openWorldLoadLevelData) lists what is wrong with
    /// level.dat and level.dat_old and offers to go back to the old one.
    Recover {
        world: WorldEntry,
        /// `collectIssue` for level.dat, then level.dat_old.
        issues: [Option<crate::save::DatIssue>; 2],
    },
    /// Where `attemptRestore` ends: the "Recovery done" ConfirmScreen
    /// (Continue opens the world), or its AlertScreen when it failed.
    Recovered {
        world: WorldEntry,
        success: bool,
    },
    Error {
        message: String,
        return_to: Box<Mode>,
        /// An AlertScreen, not the access failure.
        alert: Option<Alert>,
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
    /// AccessibilityOnboardingScreen: the first screen of a first launch.
    Onboarding(Onboarding),
    /// `BanNoticeScreens`: a ConfirmLinkScreen `Gui.addInitialScreens` queued.
    BanNotice(crate::ban_notice::Notice),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Options,
    Accessibility,
    Language,
    /// AccessibilityOnboardingScreen's Music & Sounds button.
    Sounds,
    /// The title's bottom text (`PlainTextButton`): `CreditsAndAttributionScreen`.
    Credits,
    /// `Blaze3D.openUri`: a ConfirmLinkScreen's Open in Browser.
    OpenUri(String),
    /// `ConfirmLinkScreen.copyToClipboard`.
    CopyText(String),
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
        lan: bool,
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
    /// The bottom notice's `PlainTextButton` as last drawn: x, y, width, height.
    credits_button: Cell<[f32; 4]>,
    /// `Minecraft.allowsMultiplayer()` once the account's attributes are known.
    pub allows_multiplayer: bool,
    /// The ban screens `Gui.addInitialScreens` queued, shown once the title is up.
    ban_queue: std::collections::VecDeque<crate::ban_notice::Notice>,
    /// A ban notice's three buttons as last drawn (`ConfirmLinkScreen`'s row).
    ban_buttons: Cell<[(f32, f32); 3]>,
    backup: Option<mpsc::Receiver<Result<u64, String>>>,
    backup_shown: Cell<bool>,
    /// The backup a `BackupBeforeUpgrade` asked for opens its world when done; whether it
    /// was the question about customized or experimental settings, the last one asked.
    backup_then_open: Option<bool>,
    /// The backup `OptimizeConfirm` asked for starts the optimization when done (with the
    /// Erase Cached Data choice).
    optimize_after_backup: Option<bool>,
    /// The backup `RecoverWorldDataScreen.attemptRestore` makes first; the restore runs when it
    /// is done, whether the backup worked or not.
    restore_after_backup: bool,
    /// The running `WorldUpgrader` of `Mode::Optimize`.
    optimize: Option<WorldUpgrader>,
    /// The Erase Cached Data box as last drawn: x, y, width, height.
    erase_check: Cell<[f32; 4]>,
    /// The folder to open next, once that backup has finished.
    open_request: Option<String>,
    /// EditWorldScreen's and WorldOpenFlows' `SystemToast` and its slide.
    toast: Option<(crate::toasts::Slide, WorldToast)>,
    icons: RefCell<std::collections::HashMap<PathBuf, (std::time::SystemTime, Option<Vec<u8>>)>>,
    splash: Option<String>,
    /// `SplashManager.getSplash` rolled the player's own name instead of a list entry.
    splash_you: bool,
    /// The saves listed for Home, re-read every couple of seconds.
    recent: std::cell::RefCell<Option<(std::time::Instant, Vec<WorldEntry>)>>,
    /// WorldCreationGameRulesScreen, over Create New World while it is open.
    rules: RefCell<Option<RulesScreen>>,
    /// The onboarding screen has run out: the owner saves `onboardAccessibility` as false.
    pub onboarding_finished: bool,
    /// Focused button id, for keyboard navigation.
    pub focused_button: Option<u8>,
    /// CreateWorldScreen's Experiments screen, over Create New World.
    experiments: RefCell<Option<experiments::ExperimentsScreen>>,
    /// ConfirmExperimentalFeaturesScreen, over Create New World.
    confirm_experiments: RefCell<Option<confirm_experiments::ConfirmExperimentsScreen>>,
    /// The world creation data is being read: the screen says
    /// `createWorld.preparing` until it is done.
    preparing: bool,
    /// The confirm screen's Back: the Data Packs screen reopens.
    reopen_data_packs: bool,
}
pub(crate) struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    enabled: bool,
    id: u8,
    icon: Option<(&'static str, f32)>,
    /// `SpriteIconButton.TextAndIcon`: the icon at the right end, the text centred.
    trailing: Option<(&'static str, f32)>,
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
            // findLevelCandidates: a folder with a level.dat or a level.dat_old.
            let file = |name: &str| std::fs::metadata(path.join(name)).ok().filter(|m| m.is_file());
            // getFileModificationTime: level.dat's, else level.dat_old's.
            let file = file("level.dat").or_else(|| file("level.dat_old"))?;
            let modified = file.modified().unwrap_or(std::time::UNIX_EPOCH);
            let folder = e.file_name().to_string_lossy().into_owned();
            // readLevelSummary reads level.dat alone; when it fails the world is
            // still listed, as a CorruptedLevelSummary named after its folder.
            let level = Level::read_summary(&path);
            let root = crate::save::read_dat(&path.join("level.dat"));
            let (Some(level), Some(root)) = (level, root) else {
                return Some(WorldEntry::corrupted(folder, modified));
            };
            let data = root.get("Data")?;
            let modified = match data.get("LastPlayed") {
                Some(horizon_world::nbt::Tag::Long(millis)) if *millis >= 0 => std::time::UNIX_EPOCH + std::time::Duration::from_millis(*millis as u64),
                _ => modified,
            };
            Some(WorldEntry {
                folder,
                name: level.name,
                game_mode: level.game_mode,
                hardcore: level.hardcore,
                modified,
                commands: matches!(data.get("allowCommands"), Some(horizon_world::nbt::Tag::Byte(v)) if *v != 0),
                version: data.get("Version").and_then(|v| v.get("Name"))
                    .and_then(|v| match v { horizon_world::nbt::Tag::String(s) => Some(s.to_string_lossy()), _ => None })
                    .unwrap_or_else(|| lang::get_or("selectWorld.versionUnknown", "Unknown")),
                locked: crate::save::SessionLock::is_locked(&path),
                corrupted: false,
                incompatible: crate::save::incompatible_version(&path),
                file_fixing: crate::save::needs_file_fixes(&path),
                downgrade: crate::save::downgraded_from(&path).is_some(),
            })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}
/// LevelSummary.createInfo: a hardcore world's mode in red (CommonColors.RED),
/// else `gameMode.` + its GameType name, then commands and the version.
fn world_info(world: &WorldEntry) -> crate::text::Text {
    // LevelSummary.createInfo: a locked world says so in red (ChatFormatting.RED).
    if world.locked {
        return crate::text::Text::colored(
            lang::get_or(
                "selectWorld.locked",
                "Locked by another running instance of Minecraft.",
            ),
            0xFF5555,
        );
    }
    // LevelSummary.createInfo: a world from another series says so in red.
    if let Some(version) = &world.incompatible {
        let version = if version.is_empty() {
            lang::get_or("selectWorld.versionUnknown", "unknown")
        } else {
            version.clone()
        };
        return crate::text::Text::colored(
            translate(
                "selectWorld.incompatible.info",
                "Incompatible version: %s",
                &[&version],
            ),
            crate::text::RED,
        );
    }
    // CorruptedLevelSummary.INFO: recover_world.warning in red (-65536).
    if world.corrupted {
        return crate::text::Text::colored(
            lang::get_or("recover_world.warning", "Failed to load world summary"),
            0xFF0000,
        );
    }
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
    // A world that asks for a backup shows its version in italics, red when downgrading.
    let version = crate::text::Text::literal(world.version.clone());
    let version = if !world.should_backup() {
        version
    } else if world.downgrade {
        version.with_color(crate::text::RED)
    } else {
        version.italic()
    };
    mode.append(crate::text::Text::literal(format!(
        "{commands}, {} ",
        lang::get_or("selectWorld.version", "Version:"),
    )))
    .append(version)
}
/// A translation for the narrator (`Component.translatable(key, args)`).
pub(crate) fn narration_text(key: &str, args: &[&str]) -> String {
    crate::screen_narrator::translate(key, args)
}
/// `WorldListEntry.getNarration`: "Selected: Selected <name>, last played:
/// <date>, <info>" (the date as `Date.toString` writes it).
fn world_narration(world: &WorldEntry) -> String {
    let date: chrono::DateTime<chrono::Local> = world.modified.into();
    let date = date.format("%a %b %d %H:%M:%S %Z %Y").to_string();
    let info = world_info(world).plain();
    let entry = narration_text("narrator.select.world_info", &[&world.name, &date, &info]);
    // A locked world's info already says so (LevelSummary.createInfo), and
    // getNarration adds it again.
    let entry = if world.locked {
        crate::widgets::join_for_narration(&[&entry, &narration_text("selectWorld.locked", &[])])
    } else {
        entry
    };
    narration_text("narrator.select", &[&entry])
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

/// `SplashManager.getSplash`: outside the special dates, a one in `size` roll
/// (`nextInt(size) == 42`) shows the logged-in player's name instead of a splash.
fn rolls_user_splash(text: &str, seed: u64, date: (u32, u32)) -> bool {
    use horizon_core::random::{LegacyRandom, RandomSource};
    if matches!(date, (12, 24) | (1, 1) | (10, 31)) {
        return false;
    }
    let size = text
        .lines()
        .map(str::trim)
        .filter(|s| horizon_core::random::java_string_hash(s.encode_utf16()) != 125780783)
        .count();
    i32::try_from(size)
        .is_ok_and(|size| size > 0 && LegacyRandom::new(seed as i64).next_int_bound(size) == Ok(42))
}

impl Title {
    /// Names the player for the `<NAME> IS YOU` splash; `name` is only asked for when that splash was rolled.
    pub fn with_user(mut self, name: impl FnOnce() -> String) -> Self {
        if self.splash_you {
            self.splash = Some(format!("{} IS YOU", name().to_uppercase()));
        }
        self
    }
    pub fn new(saves: PathBuf, resources: &Path) -> Self {
        use chrono::Datelike;
        let today = chrono::Local::now();
        let splashes =
            std::fs::read_to_string(resources.join("assets/minecraft/texts/splashes.txt"))
                .unwrap_or_default();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize;
        let splash_you = rolls_user_splash(
            &splashes,
            (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((seed as u64) >> 7),
            (today.month(), today.day()),
        );
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
            credits_button: Cell::new([0.0; 4]),
            allows_multiplayer: true,
            ban_queue: Default::default(),
            ban_buttons: Cell::new([(0.0, 0.0); 3]),
            backup: None,
            backup_shown: Cell::new(false),
            backup_then_open: None,
            optimize_after_backup: None,
            restore_after_backup: false,
            optimize: None,
            erase_check: Cell::new([0.0; 4]),
            open_request: None,
            toast: None,
            icons: RefCell::new(std::collections::HashMap::new()),
            splash: choose_splash(&splashes, seed, (today.month(), today.day())),
            splash_you,
            recent: std::cell::RefCell::new(None),
            rules: RefCell::new(None),
            onboarding_finished: false,
            focused_button: None,
            experiments: RefCell::new(None),
            confirm_experiments: RefCell::new(None),
            preparing: false,
            reopen_data_packs: false,
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
    /// `createWorld.preparing` is up: the world creation data is being read.
    pub fn preparing(&self) -> bool {
        self.preparing
    }
    /// The Create screen's chosen packs: the temp folder and the selected
    /// ids, for the worldgen data the preparing read loads.
    pub fn creation_packs(&self) -> Option<(Option<std::path::PathBuf>, Vec<String>)> {
        match &self.mode {
            Mode::Create { options, .. } => {
                Some((options.pack_dir.clone(), options.data_packs.clone()))
            }
            _ => None,
        }
    }
    /// The read finished: the flag goes, and the create screen opens — a
    /// fresh one entering from the world list, the one the player already
    /// filled in after the data packs changed (`openCreateWorldScreen`).
    pub fn finish_preparing(&mut self) {
        self.preparing = false;
        if !matches!(self.mode, Mode::Create { .. }) {
            self.mode = Self::new_world();
        }
    }
    /// `PackSelectionScreen.onFilesDrop`: a pack dropped on the window joins
    /// the Create New World screen's data packs (the screen's watcher lists
    /// it within a second).
    pub fn drop_file(&mut self, path: std::path::PathBuf) {
        let Mode::Create { options, .. } = &mut self.mode else {
            return;
        };
        if options.pack_dir.is_none() {
            options.pack_dir = crate::datapack::temp_folder().ok();
        }
        let Some(dir) = options.pack_dir.clone() else {
            return;
        };
        if !crate::datapack::copy_pack(&path, &dir) {
            log::warn!("Horizon: {} is not a data pack", path.display());
        }
    }
    /// The world's data pack selection, for the game to reoffer.
    pub fn data_packs(&self) -> Vec<String> {
        match &self.mode {
            Mode::Create { options, .. } => options.data_packs.clone(),
            _ => Vec::new(),
        }
    }
    /// The new world's pack folder, for the Data Packs screen to reopen.
    pub fn pack_dir(&self) -> Option<PathBuf> {
        match &self.mode {
            Mode::Create { options, .. } => options.pack_dir.clone(),
            _ => None,
        }
    }
    /// The Data Packs screen's Done (`tryApplyNewDataPacks`): a selection
    /// with feature packs asks for experimental features, so the confirm
    /// screen comes up first; anything else applies at once and the data
    /// is read again.
    pub fn offer_data_packs(&mut self, resources: &std::path::Path, ids: Vec<String>) {
        let experimental = crate::datapack::feature_packs(resources)
            .into_iter()
            .any(|p| ids.contains(&p.id));
        if experimental {
            *self.confirm_experiments.get_mut() = Some(
                confirm_experiments::ConfirmExperimentsScreen::new(resources, &ids),
            );
        } else {
            self.set_data_packs(ids);
            self.preparing = true;
        }
    }
    /// Whether the confirm screen declined: the Data Packs screen reopens.
    pub fn take_reopen_data_packs(&mut self) -> bool {
        std::mem::take(&mut self.reopen_data_packs)
    }
    /// `onAbort`: the Data Packs screen reopens with the selection and the
    /// folder it had (`take_reopen_data_packs` said it wants to).
    pub fn reopen_data_packs_action(&mut self) -> Option<Action> {
        if !self.take_reopen_data_packs() {
            return None;
        }
        let (dir, selected) = match &self.mode {
            Mode::Create { options, .. } => (
                options.pack_dir.clone().unwrap_or_default(),
                options.data_packs.clone(),
            ),
            _ => (Default::default(), Vec::new()),
        };
        Some(Action::DataPacks { dir, selected })
    }
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
    /// Mouse wheel over the classic world and server lists, or cycle buttons. Returns true if a button was cycled.
    pub fn scroll_worlds(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if let Some(rules) = self.rules.get_mut() {
            rules.wheel(delta, size.1.ceil());
            return false;
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.wheel(delta, cursor, size);
            return false;
        }
        if let Mode::CreateFlat { flat, .. } = &mut self.mode {
            flat::wheel(flat, delta, cursor, size);
            return false;
        }
        if let Mode::Create {
            creative, options, ..
        } = &mut self.mode
        {
            let (w, h) = size;
            let buttons = create_world::buttons(w, h, *creative, options);
            if let Some(b) = buttons.iter().find(|b| {
                b.enabled
                    && cursor.0 >= b.x
                    && cursor.0 < b.x + b.w
                    && cursor.1 >= b.y
                    && cursor.1 < b.y + 20.0
            }) {
                let step = if delta > 0.0 { -1 } else { 1 };
                options.cycle(b.id, creative, step);
                return true;
            }
        }
        let Mode::Select { worlds, .. } = &self.mode else {
            return false;
        };
        let (top, bottom, _, content) = self.world_list(worlds, size);
        if cursor.1 < top || cursor.1 >= bottom {
            return false;
        }
        let mut scroll = self.scroll.get();
        scroll.wheel(delta, ROW_HEIGHT / 2.0, top, bottom, content);
        self.scroll.set(scroll);
        false
    }
    pub fn failed(&mut self, error: impl std::fmt::Display) {
        let return_to = Box::new(self.mode.clone());
        self.mode = Mode::Error {
            message: error.to_string(),
            return_to,
            alert: None,
        };
    }
    /// Polling is done once per rendered frame so filesystem work never blocks drawing.
    pub fn poll_backup(&mut self) {
        self.poll_onboarding(std::time::Instant::now());
        self.poll_account();
        self.poll_optimize();
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
            let world = world.clone();
            let folder = world.folder.clone();
            if std::mem::take(&mut self.restore_after_backup) {
                // `makeBackupAndShowToast(..).thenAcceptAsync(..)` in attemptRestore: the restore
                // runs, backup failed or not.
                self.attempt_restore(world.clone());
            } else if let Some(erase_cache) = self.optimize_after_backup.take() {
                // `conditionallyMakeBackupAndShowToast(..).thenAcceptAsync(..)`: the toast shows
                // and OptimizeWorldScreen opens, backup failed or not.
                self.begin_optimize(world.clone(), erase_cache);
            } else {
                self.return_to_list(Some(&folder));
            }
            // EditWorldScreen opens the world after the backup, failed or not: through the
            // checks WorldOpenFlows still has to make.
            if let Some(stem_asked) = self.backup_then_open.take()
                && let Action::Play { folder } = self.after_prompt(world.clone(), stem_asked)
            {
                self.open_request = Some(folder);
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
    /// The server list's narration of LAN worlds found since the last call
    /// (`JoinMultiplayerScreen.tick`'s `updateNetworkServers`).
    pub fn lan_found_narrations(&self, size: (f32, f32)) -> Vec<Text> {
        match &self.mode {
            Mode::Multiplayer(screen) => screen.lan_found_narrations(size),
            _ => Vec::new(),
        }
    }
    /// Whether the system toast holds the `ToastManager`'s first slot.
    pub fn has_toast(&self) -> bool {
        self.toast.is_some()
    }
    /// The world a finished backup made before its upgrade opens next.
    pub fn take_open_request(&mut self) -> Option<String> {
        self.open_request.take()
    }
    /// `WorldOpenFlows.openWorldLoadLevelData` for a world opened without the list (the control
    /// socket): level data that does not read shows RecoverWorldDataScreen. False when the world
    /// reads and opens as usual.
    pub fn open_recover(&mut self, folder: &str) -> bool {
        let dir = self.saves.join(folder);
        let Some(issue) = crate::save::level_data_needs_recovery(&dir) else {
            return false;
        };
        let world = list_worlds(&self.saves)
            .into_iter()
            .find(|w| w.folder == folder)
            .unwrap_or_else(|| {
                WorldEntry::corrupted(folder.to_owned(), std::time::SystemTime::now())
            });
        let issues = [Some(issue), crate::save::level_dat_issue(&dir, true)];
        self.mode = Mode::Recover { world, issues };
        true
    }
    /// `WorldOpenFlows.openWorld`: a world that can't be locked, as one another
    /// instance has open, stays closed behind `onWorldAccessFailure`
    /// (`createWorldAccess`); then `openWorldCheckVersionCompatibility`: a world from
    /// another series is an alert, one whose file fixes are due or that a newer version
    /// saved asks for a backup first; any other opens at once.
    fn play(&mut self, world: WorldEntry) -> Action {
        // LevelSummary.primaryActionActive: a world another copy has open cannot be joined.
        if world.locked {
            return Action::None;
        }
        if crate::save::SessionLock::acquire(&self.saves.join(&world.folder)).is_err() {
            self.toast = Some((
                crate::toasts::Slide::default(),
                WorldToast::AccessFailure(world.folder),
            ));
            return Action::None;
        }
        // openWorldLoadLevelData: a level.dat that does not read opens
        // RecoverWorldDataScreen instead of the world.
        let issues = recover_issues(&self.saves.join(&world.folder));
        if issues[0].is_some() {
            self.mode = Mode::Recover { world, issues };
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
                alert: Some(Alert::Incompatible),
            };
            return Action::None;
        }
        if crate::save::needs_file_fixes(&self.saves.join(&world.folder)) {
            self.mode = Mode::BackupBeforeUpgrade {
                world,
                downgrade: None,
                stem: None,
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
                stem: None,
            };
            return Action::None;
        }
        self.after_prompt(world, false)
    }
    /// What `WorldOpenFlows` does once a world's version questions are answered:
    /// `openWorldCheckWorldStemCompatibility` (unless `stem_asked`: its backup question was
    /// the one just answered), then `openWorldCheckDiskSpace`, then the load.
    fn after_prompt(&mut self, world: WorldEntry, stem_asked: bool) -> Action {
        let dir = self.saves.join(&world.folder);
        // The generator settings of a world that still needs its file fixes are read as the
        // fixes will leave them (`world_gen_settings`), so its question comes with this open.
        if !stem_asked {
            let issue = if crate::save::is_old_customized_world(&dir) {
                Some(StemIssue::Customized)
            } else if crate::save::world_gen_unstable(&dir) {
                Some(StemIssue::Experimental)
            } else {
                None
            };
            if let Some(stem) = issue {
                self.mode = Mode::BackupBeforeUpgrade {
                    world,
                    downgrade: None,
                    stem: Some(stem),
                };
                return Action::None;
            }
        }
        if crate::save::low_disk_space(&dir) {
            self.mode = Mode::LowDiskSpace { world };
            return Action::None;
        }
        Action::Play {
            folder: world.folder,
        }
    }
    /// `RecoverWorldDataScreen.attemptRestore`: with level.dat still broken and
    /// level.dat_old readable, level.dat_old takes level.dat's place
    /// (`restoreLevelDataFromOld`); anything else is a failure.
    fn attempt_restore(&mut self, world: WorldEntry) {
        let dir = self.saves.join(&world.folder);
        let issues = recover_issues(&dir);
        let success = issues[0].is_some()
            && issues[1].is_none()
            && crate::save::restore_level_data_from_old(&dir);
        if !success {
            log::error!(
                "Failed to recover world, files not as expected. level.dat: {:?}, level.dat_old: {:?}",
                issues[0],
                issues[1]
            );
        }
        *self.recent.borrow_mut() = None;
        self.mode = Mode::Recovered { world, success };
    }
    /// `OptimizeWorldScreen.create`: a world that still needs its file fixes or whose level data
    /// does not read can't be optimized (vanilla logs and `setScreen(null)`, the main menu here).
    /// Horizon stamps level.dat when the run finishes, not before: opening a world with an
    /// older DataVersion is what converts it whole.
    fn begin_optimize(&mut self, world: WorldEntry, erase_cache: bool) {
        let dir = self.saves.join(&world.folder);
        // `getUnfixedDataTagWithFallback`: level.dat_old stands in for a level.dat that does not read.
        crate::save::fall_back_to_level_dat_old(&dir);
        if crate::save::needs_file_fixes(&dir)
            || crate::save::level_data_needs_recovery(&dir).is_some()
        {
            eprintln!("Failed to load datapacks, can't optimize world");
            self.mode = Mode::Main;
            return;
        }
        let levels = world_upgrader::world_dimensions(&dir);
        self.optimize = Some(WorldUpgrader::new(&dir, levels, erase_cache, false));
        self.mode = Mode::Optimize { world };
    }
    /// `OptimizeWorldScreen.tick`: a finished upgrader ends the screen with `true`.
    fn poll_optimize(&mut self) {
        if matches!(self.mode, Mode::Optimize { .. })
            && self
                .optimize
                .as_ref()
                .is_some_and(WorldUpgrader::is_finished)
        {
            self.end_optimize(true);
        }
    }
    /// The callback of `OptimizeWorldScreen`, then `removed()` (cancel and close the upgrader):
    /// back to the refreshed world list.
    fn end_optimize(&mut self, success: bool) {
        let Mode::Optimize { world } = &self.mode else {
            return;
        };
        let folder = world.folder.clone();
        if let Some(mut upgrader) = self.optimize.take() {
            upgrader.cancel();
            if success && upgrader.status_key() != "optimizeWorld.stage.failed" {
                // The level data at the current version, which vanilla saved in `create`.
                if let Err(e) =
                    horizon_world::datafix::files::fix_world_files(&self.saves.join(&folder))
                {
                    eprintln!("level.dat: {e}");
                }
            }
        }
        self.return_to_list(Some(&folder));
    }
    /// `OptimizeWorldScreen.extractRenderState`.
    fn draw_optimize(&self, atlas: &HudAtlas, batch: &mut HudBatch, w: f32, h: f32, name: &str) {
        let Some(upgrader) = &self.optimize else {
            return;
        };
        let grey = rgba(160, 160, 160, 255);
        centered(
            atlas,
            batch,
            &translate("optimizeWorld.title", "Optimizing World '%s'", &[&name]),
            20.0,
            u32::MAX,
        );
        let (x0, x1) = ((w / 2.0).floor() - 150.0, (w / 2.0).floor() + 150.0);
        let y0 = (h.ceil() / 4.0).floor() + 100.0;
        let y1 = y0 + 10.0;
        let key = upgrader.status_key();
        centered(
            atlas,
            batch,
            &lang::get_or(&key, status_fallback(&key)),
            y0 - 9.0 - 2.0,
            grey,
        );
        if upgrader.total_chunks() <= 0 {
            return;
        }
        batch.fill(
            atlas,
            x0 - 1.0,
            y0 - 1.0,
            x1 - x0 + 2.0,
            y1 - y0 + 2.0,
            rgba(0, 0, 0, 255),
        );
        for (i, (key, fallback, value)) in [
            (
                "optimizeWorld.info.converted",
                "Upgraded chunks: %s",
                upgrader.converted(),
            ),
            (
                "optimizeWorld.info.skipped",
                "Skipped chunks: %s",
                upgrader.skipped(),
            ),
            (
                "optimizeWorld.info.total",
                "Total chunks: %s",
                upgrader.total_chunks(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            batch.text(
                atlas,
                &translate(key, fallback, &[&value]),
                x0,
                40.0 + 12.0 * i as f32,
                grey,
                true,
            );
        }
        let mut progress = 0.0;
        for level in upgrader.levels() {
            let length = (upgrader.dimension_progress(level) * (x1 - x0)).floor();
            batch.fill(
                atlas,
                x0 + progress,
                y0,
                length,
                y1 - y0,
                dimension_color(level),
            );
            progress += length;
        }
        let done = upgrader.converted() + upgrader.skipped();
        let counter = translate(
            "optimizeWorld.progress.counter",
            "%s / %s",
            &[&done, &upgrader.total_chunks()],
        );
        let percent = translate(
            "optimizeWorld.progress.percentage",
            "%s%%",
            &[&((upgrader.total_progress() * 100.0).floor() as i32)],
        );
        centered(atlas, batch, &counter, y0 + 2.0 * 9.0 + 2.0, grey);
        centered(atlas, batch, &percent, y0 + 1.0, grey);
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
            trailing: None,
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
                        self.allows_multiplayer,
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
            Mode::Select { worlds, selected } => {
                let world = selected.and_then(|i| worlds.get(i));
                // SelectWorldScreen.updateButtonStatus: primaryActionMessage, canEdit, canRecreate.
                let corrupted = world.is_some_and(|w| w.corrupted);
                let editable = world.is_some_and(WorldEntry::can_edit);
                let play = if corrupted {
                    lang::get_or("recover_world.button", "Attempt to Recover")
                } else if world.is_some_and(|w| w.file_fixing) {
                    lang::get_or("selectWorld.upgrade_and_play", "Upgrade and Play")
                } else {
                    lang::get_or("selectWorld.select", "Play Selected World")
                };
                vec![
                    b(
                        (w / 2.0).floor() - 154.0,
                        h - 52.0,
                        150.0,
                        &play,
                        world.is_some_and(WorldEntry::can_play),
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
                        editable,
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
                        editable,
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
                ]
            }
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
                        true,
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
            Mode::LowDiskSpace { .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        94,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        95,
                    ),
                ]
            }
            Mode::Recover { issues, .. } => {
                let y = recover_buttons_top(h, self.message_lines.get());
                // The restore button is active only when level.dat is the broken one
                // and level.dat_old reads.
                vec![
                    b(
                        (w / 2.0).floor() - 60.0,
                        y,
                        120.0,
                        &lang::get_or("recover_world.restore", "Attempt to Restore"),
                        issues[0].is_some() && issues[1].is_none(),
                        90,
                    ),
                    b(
                        (w / 2.0).floor() - 60.0,
                        y + 28.0,
                        120.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        91,
                    ),
                ]
            }
            Mode::Recovered { success: true, .. } => {
                let y = confirm_top(h, self.message_lines.get())
                    + 41.0
                    + self.message_lines.get() as f32 * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 152.0,
                        y,
                        150.0,
                        &lang::get_or("gui.continue", "Continue"),
                        true,
                        92,
                    ),
                    b(
                        (w / 2.0).floor() + 2.0,
                        y,
                        150.0,
                        &lang::get_or("gui.back", "Back"),
                        true,
                        93,
                    ),
                ]
            }
            Mode::Recovered { success: false, .. } => {
                // AlertScreen.init: the button follows the text, kept inside the screen.
                let y = (90.0 + self.message_lines.get() as f32 * 9.0 + 12.0)
                    .max(h / 6.0 + 96.0)
                    .min(h - 24.0);
                vec![b(
                    (w / 2.0).floor() - 75.0,
                    y,
                    150.0,
                    &lang::get_or("gui.back", "Back"),
                    true,
                    93,
                )]
            }
            Mode::Error { alert, .. } => {
                let lines = self.message_lines.get();
                let y = if alert.is_some() {
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
            Mode::OptimizeConfirm { .. } => {
                // BackupConfirmScreen.init with the Erase Cached Data box: the buttons sit
                // under the text at fixed offsets.
                let y = 100.0 + (self.message_lines.get() as f32 + 1.0) * 9.0;
                vec![
                    b(
                        (w / 2.0).floor() - 155.0,
                        y,
                        150.0,
                        &lang::get_or(
                            "optimizeWorld.confirm.proceed",
                            "Create Backup and Optimize",
                        ),
                        true,
                        43,
                    ),
                    b(
                        (w / 2.0).floor() + 5.0,
                        y,
                        150.0,
                        &lang::get_or("selectWorld.backupJoinSkipButton", "I Know What I'm Doing!"),
                        true,
                        44,
                    ),
                    b(
                        (w / 2.0).floor() - 75.0,
                        y + 24.0,
                        150.0,
                        &lang::get_or("gui.cancel", "Cancel"),
                        true,
                        45,
                    ),
                ]
            }
            Mode::Optimize { .. } => vec![b(
                (w / 2.0).floor() - 100.0,
                (h / 4.0).floor() + 150.0,
                200.0,
                &lang::get_or("gui.cancel", "Cancel"),
                true,
                46,
            )],
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
            Mode::Onboarding(_) => self.onboarding_buttons(w, h),
            Mode::BanNotice(_) => {
                // ConfirmLinkScreen.addButtons: Open in Browser, Copy to Clipboard, then the
                // notice's Acknowledge, 100 wide each.
                let labels = [
                    lang::get_or("chat.link.open", "Open in Browser"),
                    lang::get_or("chat.copy", "Copy to Clipboard"),
                    lang::get_or("gui.acknowledge", "Acknowledge"),
                ];
                let positions = self.ban_buttons.get();
                (0..3)
                    .map(|i| {
                        b(
                            positions[i].0,
                            positions[i].1,
                            100.0,
                            &labels[i],
                            true,
                            120 + i as u8,
                        )
                    })
                    .collect()
            }
            Mode::Backup { .. } => Vec::new(),
            Mode::Multiplayer(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| b(x, y, bw, &label, enabled, id))
                .collect(),
        }
    }
    /// The screen this mode is, as the narrator reads it: its id and
    /// `getNarrationMessage` (the title, or `joinForNarration(title, message)`
    /// for the confirm and alert screens). `None` for the screens that
    /// narrate themselves.
    fn narration(&self) -> Option<crate::widgets::ScreenInfo> {
        use crate::widgets::{ScreenInfo, join_for_narration as join};
        let get = |key: &str, fallback: &str| lang::get_or(key, fallback);
        let info = match &self.mode {
            // TitleScreen: `narrator.screen.title`, not its (empty) title.
            Mode::Main => ScreenInfo::new("title", get("narrator.screen.title", "Title Screen")),
            Mode::Select { .. } => {
                ScreenInfo::new("select_world", get("selectWorld.title", "Select World"))
            }
            Mode::Create { .. } => ScreenInfo::new(
                "create_world",
                get("selectWorld.create", "Create New World"),
            ),
            // PresetFlatWorldScreen over CreateFlatWorldScreen.
            Mode::CreateFlat { flat, .. } if flat.page == flat::Page::Presets => ScreenInfo::new(
                "create_world/flat/presets",
                get("createWorld.customize.presets.title", "Select a Preset"),
            ),
            Mode::CreateFlat { .. } => ScreenInfo::new(
                "create_world/flat",
                get(
                    "createWorld.customize.flat.title",
                    "Superflat Customization",
                ),
            ),
            Mode::Edit { .. } => {
                ScreenInfo::new("edit_world", get("selectWorld.edit.title", "Edit World"))
            }
            // EditWorldScreen.makeBackupAndShowToast: a GenericWaitingScreen.
            Mode::Backup { .. } => ScreenInfo::new(
                "backup",
                join(&[
                    &get("selectWorld.waitingForBackup.title", "Creating Backup"),
                    &get(
                        "selectWorld.waitingForBackup.message",
                        "Depending on the size of your world, this may take a while. Please do not close the game or shut off your device.",
                    ),
                ]),
            ),
            // BackupConfirmScreen narrates its title, the question.
            Mode::BackupBeforeUpgrade { downgrade, .. } => {
                let question = match downgrade {
                    Some(_) => get(
                        "selectWorld.backupQuestion.downgrade",
                        "Downgrading a world is not supported",
                    ),
                    None => get(
                        "selectWorld.backupQuestion.file_fixing_required",
                        "Create a backup before upgrading this world?",
                    ),
                };
                ScreenInfo::new("backup_confirm", question)
            }
            // EditWorldScreen's Optimize World: a BackupConfirmScreen, which
            // narrates its title.
            Mode::OptimizeConfirm { .. } => ScreenInfo::new(
                "optimize_confirm",
                get("optimizeWorld.confirm.title", "Optimize World"),
            ),
            Mode::Optimize { world } => ScreenInfo::new(
                "optimize_world",
                translate(
                    "optimizeWorld.title",
                    "Optimizing World '%s'",
                    &[&world.name],
                ),
            ),
            // openWorldCheckDiskSpace's ConfirmScreen.
            Mode::LowDiskSpace { .. } => ScreenInfo::new(
                "low_disk_space",
                join(&[
                    &get(
                        "selectWorld.warning.lowDiskSpace.title",
                        "Warning! Low disk space!",
                    ),
                    &get(
                        "selectWorld.warning.lowDiskSpace.description",
                        "There is not much space left on your device.
Running out of disk space while in game can lead to your world being damaged.",
                    ),
                ]),
            ),
            // AccessibilityOnboardingScreen narrates its title.
            Mode::Onboarding(_) => ScreenInfo::new(
                "accessibility_onboarding",
                get(
                    "accessibility.onboarding.screen.title",
                    "Welcome to Minecraft!

Would you like to enable the Narrator or visit the Accessibility Settings?",
                ),
            ),
            // BanNoticeScreens: a ConfirmLinkScreen, its title and message.
            Mode::BanNotice(notice) => {
                let now = chrono::Utc::now().timestamp_millis();
                ScreenInfo::new(
                    "ban_notice",
                    join(&[&notice.title().plain(), &notice.message(now).plain()]),
                )
            }
            Mode::Delete { world } => ScreenInfo::new(
                "delete_world",
                join(&[
                    &get(
                        "selectWorld.deleteQuestion",
                        "Are you sure you want to delete this world?",
                    ),
                    &translate(
                        "selectWorld.deleteWarning",
                        "'%s' will be lost forever! (A long time!)",
                        &[&world.name],
                    ),
                ]),
            ),
            Mode::Recover { world, .. } => {
                let message = format_translation(
                    &get(
                        "recover_world.message",
                        "The following issues occurred while trying to read world folder \"%s\".\nIt might be possible to restore the world from an older state or you can report this issue on the bug tracker.",
                    ),
                    &[Text::literal(world.folder.clone())],
                );
                ScreenInfo::new(
                    "recover_world",
                    join(&[
                        &get("recover_world.title", "Failed to load world"),
                        &message.plain(),
                    ]),
                )
            }
            Mode::Recovered { success, .. } => {
                let message = if *success {
                    get("recover_world.done.success", "Recovery was successful!")
                } else {
                    get(
                        "recover_world.done.failed",
                        "Failed to recover from previous state.",
                    )
                };
                ScreenInfo::new(
                    "recover_world/done",
                    join(&[&get("recover_world.done.title", "Recovery done"), &message]),
                )
            }
            Mode::Error { message, alert, .. } => {
                let title = match alert {
                    Some(Alert::Incompatible) => {
                        get("selectWorld.incompatible.title", "Incompatible version")
                    }
                    Some(Alert::Recreate) => {
                        get("selectWorld.recreate.error.title", "An error occurred!")
                    }
                    None => get("selectWorld.access_failure", "Failed to access world"),
                };
                ScreenInfo::new("world_error", join(&[&title, message]))
            }
            // SafetyScreen: WarningScreen's narration, the header and the
            // message on the next line.
            Mode::MultiplayerWarning { .. } => ScreenInfo::new(
                "multiplayer_warning",
                format!(
                    "{}\n{}",
                    get(
                        "multiplayerWarning.header",
                        "Caution: Third-Party Online Play"
                    ),
                    get(
                        "multiplayerWarning.message",
                        "Caution: Online play is offered by third-party servers that are not owned, operated, or supervised by Mojang Studios or Microsoft. During online play, you may be exposed to unmoderated chat messages or other types of user-generated content that may not be suitable for everyone.",
                    ),
                ),
            ),
            Mode::Multiplayer(_) => return None,
        };
        Some(info)
    }

    /// A button's narration: `gui.narrate.button` of its label, or the icon
    /// buttons' own message and tooltip, in the order the screen adds them.
    fn button_narration(&self, b: &Button) -> crate::widgets::Narratable {
        use crate::widgets::Narratable;
        let rect = [b.x, b.y, b.w, 20.0];
        match &self.mode {
            Mode::Main => {}
            Mode::Create {
                creative, options, ..
            } => return create_world::button_narration(b, *creative, options),
            Mode::Multiplayer(screen) => {
                return screen.button_narration(b.id, rect, &b.label, b.enabled);
            }
            _ => return Narratable::button(b.label.clone(), rect, b.enabled),
        }
        // CommonButtons and FriendsButton narrate their `.narration`, and
        // read their tooltip as a hint.
        let icon = |narration: &str, tooltip: &str| {
            Narratable::named(narration_text(narration, &[]), rect, b.enabled)
                .hint(narration_text(tooltip, &[]))
        };
        let narratable = match b.id {
            5 => icon("gui.friends.open.narration", "gui.friends.open.tooltip"),
            6 => icon("options.language.narration", "options.language.tooltip"),
            7 => icon(
                "accessibility.onboarding.accessibility.button.narration",
                "options.accessibility.tooltip",
            ),
            _ => Narratable::button(b.label.clone(), rect, b.enabled),
        };
        // TitleScreen.init adds the icon row after Minecraft Realms and before
        // Options... and Quit Game, which Horizon draws first.
        narratable.order(match b.id {
            5..=7 => 1,
            2 | 3 => 2,
            _ => 0,
        })
    }

    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
    ) {
        self.draw_with_items(atlas, None, batch, cursor, hide_splash);
    }
    /// [`Title::draw`] with the item models, which draw the item icons of the
    /// Superflat screens the way `GuiGraphics.renderItem` does.
    pub fn draw_with_items(
        &self,
        atlas: &HudAtlas,
        items: Option<&crate::item_render::ItemRenderer>,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        if let Some(rules) = self.rules.borrow_mut().as_mut() {
            return rules.draw(batch, atlas, cursor);
        }
        if self.preparing {
            // `CreateWorldScreen.openCreateWorldScreen`'s queued
            // `GenericMessageScreen`: the menu background and the message
            // alone, centred, no progress.
            crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
            let text =
                crate::lang::get_or("createWorld.preparing", "Preparing for world creation...");
            let tw = atlas.font.width_text(&Text::literal(text.clone()));
            let (tx, ty) = (w / 2.0 - tw / 2.0, h / 2.0 - 4.5);
            // `FocusableTextWidget.builder(title, font, 12)`: 12 pixels of
            // padding around the text, on a black box with a 1-pixel border.
            let (bx, by) = (tx - 12.0, ty - 12.0);
            let (bw, bh) = (tw + 24.0, 9.0 + 24.0);
            batch.fill(atlas, bx - 1.0, by - 1.0, bw + 2.0, bh + 2.0, 0xFFD0D0D0);
            batch.fill(atlas, bx, by, bw, bh, 0xFF000000);
            batch.text(atlas, &text, tx, ty, u32::MAX, false);
            return;
        }
        if let Some(screen) = self.confirm_experiments.borrow().as_ref() {
            return screen.draw(batch, atlas, cursor);
        }
        if let Some(screen) = self.experiments.borrow().as_ref() {
            return screen.draw(batch, atlas, cursor);
        }
        let mut fade_from = None;
        if crate::widgets::narrating()
            && let Some(info) = self.narration()
        {
            crate::widgets::begin_screen(|| info);
        }
        match &self.mode {
            Mode::Main => {
                draw_logo(atlas, batch, w);
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
                // TitleScreen.init's `PlainTextButton`: the text's width wide, 10
                // high at `height - 10`; `extractContents` underlines it while
                // hovered or focused, and its press opens the credits.
                let tw = HudBatch::text_width(atlas, notice).ceil();
                let rect = [w - tw - 2.0, h - 10.0, tw, 10.0];
                self.credits_button.set(rect);
                crate::widgets::record(notice, rect, true);
                let inside = cursor.0 >= rect[0]
                    && cursor.0 < rect[0] + rect[2]
                    && cursor.1 >= rect[1]
                    && cursor.1 < rect[1] + rect[3];
                crate::widgets::widget_cursor(inside, true);
                let mut text = Text::literal(notice);
                if inside {
                    text = text.underlined();
                }
                batch.text_styled(atlas, &text, rect[0], rect[1], u32::MAX, true);
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
                crate::widgets::name_last_edit_box(|| narration_text("selectWorld.search", &[]));
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
                // WorldSelectionList: the row under the mouse, else the
                // selected one.
                crate::widgets::narratable(|| {
                    use crate::widgets::{ListKind, Narratable, Row};
                    let rows: Vec<&WorldEntry> =
                        self.visible_worlds(worlds).map(|(_, w)| w).collect();
                    let row = |r: usize| Row::new(r, world_narration(rows[r]));
                    let selected = self
                        .visible_worlds(worlds)
                        .position(|(i, _)| Some(i) == *selected);
                    Narratable::list(
                        ListKind::Object,
                        rows.len(),
                        hovered.map(|(r, _)| row(r)),
                        selected.map(row),
                        [0.0, top, w, bottom - top],
                    )
                    .key(crate::widgets::key_of(&"select_world/worlds"))
                });
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
                        // The icon says what a click does.
                        let names: &[&str] = if world.corrupted {
                            &["error", "marked_join"]
                        } else if world.disabled() {
                            &["error"]
                        } else if world.downgrade {
                            &["marked_join", "error"]
                        } else if world.should_backup() {
                            &["marked_join"]
                        } else {
                            &["join"]
                        };
                        let lit = if on_icon { "_highlighted" } else { "" };
                        for name in names {
                            let sprite = format!("gui/sprites/world_list/{name}{lit}");
                            batch.sprite_scaled(atlas, &sprite, x, y, 32.0, 32.0, u32::MAX);
                        }
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
            Mode::CreateFlat { flat, .. } => flat::draw(self, atlas, items, batch, cursor, flat),
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
                crate::widgets::name_last_edit_box(|| narration_text("selectWorld.enterName", &[]));
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
            Mode::Recover { world, issues } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let dir = self.saves.join(&world.folder);
                let title =
                    Text::literal(lang::get_or("recover_world.title", "Failed to load world"))
                        .bold();
                let message = format_translation(
                    &lang::get_or(
                        "recover_world.message",
                        "The following issues occurred while trying to read world folder \"%s\".\nIt might be possible to restore the world from an older state or you can report this issue on the bug tracker.",
                    ),
                    &[Text::colored(world.folder.clone(), GRAY)],
                );
                let message: Vec<Text> = atlas
                    .font
                    .split_lines(&message, w - 50.0)
                    .into_iter()
                    .map(|(line, _)| line)
                    .collect();
                let issues: Vec<Text> = [false, true]
                    .into_iter()
                    .zip(issues)
                    .filter_map(|(old, issue)| recover_info(&dir, old, issue.as_ref()))
                    .flat_map(|info| atlas.font.split_lines(&info, w - 50.0))
                    .map(|(line, _)| line)
                    .collect();
                self.message_lines.set(message.len() + issues.len());
                let row = recover_buttons_top(h, message.len() + issues.len());
                let top = row - 8.0 - 8.0 - (message.len() + issues.len()) as f32 * 9.0 - 8.0 - 9.0;
                let center =
                    |text: &Text| (w / 2.0).floor() - (atlas.font.width_text(text) / 2.0).floor();
                batch.text_styled(atlas, &title, center(&title), top, u32::MAX, true);
                let mut y = top + 17.0;
                for line in &message {
                    batch.text_styled(atlas, line, center(line), y, u32::MAX, true);
                    y += 9.0;
                }
                // The issues widget is left aligned, the widget centred.
                let width = issues
                    .iter()
                    .map(|l| atlas.font.width_text(l))
                    .fold(0.0, f32::max);
                let x = (w / 2.0).floor() - (width / 2.0).floor();
                for line in &issues {
                    batch.text_styled(atlas, line, x, y + 8.0, u32::MAX, true);
                    y += 9.0;
                }
            }
            Mode::Recovered { success, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let title = lang::get_or("recover_world.done.title", "Recovery done");
                let message = if *success {
                    lang::get_or("recover_world.done.success", "Recovery was successful!")
                } else {
                    lang::get_or(
                        "recover_world.done.failed",
                        "Failed to recover from previous state.",
                    )
                };
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                // ConfirmScreen centres its column; AlertScreen puts the title at 70, the text at 90.
                let (top, text_top) = if *success {
                    let top = confirm_top(h, lines.len());
                    (top, top + 17.0)
                } else {
                    (70.0, 90.0)
                };
                centered(atlas, batch, &title, top, u32::MAX);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, text_top + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::BackupBeforeUpgrade {
                downgrade, stem, ..
            } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                // BackupStatus.DOWNGRADE is severe: its question is red (-2142128).
                let (question, warning, color) = match (downgrade, stem) {
                    (None, Some(StemIssue::Customized)) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.customized",
                            "Customized worlds are no longer supported",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.customized",
                            "Unfortunately, we do not support customized worlds in this version of Minecraft. We can still load this world and keep everything the way it was, but any newly generated terrain will no longer be customized. We're sorry for the inconvenience!",
                        ),
                        u32::MAX,
                    ),
                    (None, Some(StemIssue::Experimental)) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.experimental",
                            "Worlds using Experimental Settings are not supported",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.experimental",
                            "This world uses experimental settings that could stop working at any time. We cannot guarantee it will load or work. Here be dragons!",
                        ),
                        u32::MAX,
                    ),
                    (Some(version), _) => (
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
                    (None, None) => (
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
            Mode::OptimizeConfirm { erase_cache, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let description = lang::get_or(
                    "optimizeWorld.confirm.description",
                    "This will attempt to optimize your world by making sure all data is stored in the most recent game format. This can take a very long time, depending on your world. Once done, your world may play faster but will no longer be compatible with older versions of the game. Are you sure you wish to proceed?",
                );
                let lines = wrapped(atlas, &description, w - 50.0);
                self.message_lines.set(lines.len());
                centered(
                    atlas,
                    batch,
                    &lang::get_or("optimizeWorld.confirm.title", "Optimize World"),
                    50.0,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, 70.0 + i as f32 * 9.0, u32::MAX);
                }
                // The Checkbox: at `width / 2 - 155 + 80`, `76 + textSize`.
                let label = lang::get_or("selectWorld.backupEraseCache", "Erase Cached Data");
                let width = BOX + 4.0 + HudBatch::text_width(atlas, &label).ceil();
                let x = (w / 2.0).floor() - 155.0 + 80.0;
                let y = 76.0 + (lines.len() as f32 + 1.0) * 9.0;
                self.erase_check.set([x, y, width, BOX]);
                let hover = (x..x + width).contains(&cursor.0) && (y..y + BOX).contains(&cursor.1);
                let sprite = match (*erase_cache, hover) {
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
            Mode::Optimize { world } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                self.draw_optimize(atlas, batch, w, h, &world.name);
            }
            Mode::LowDiskSpace { .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let message = lang::get_or(
                    "selectWorld.warning.lowDiskSpace.description",
                    "There is not much space left on your device.\nRunning out of disk space while in game can lead to your world being damaged.",
                );
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                let top = confirm_top(h, lines.len());
                // ChatFormatting.RED.
                centered(
                    atlas,
                    batch,
                    &lang::get_or(
                        "selectWorld.warning.lowDiskSpace.title",
                        "Warning! Low disk space!",
                    ),
                    top,
                    rgba(255, 85, 85, 255),
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
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
            Mode::Error { message, alert, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let lines: Vec<String> = message
                    .split('\n')
                    .flat_map(|line| wrapped(atlas, line, w - 50.0))
                    .collect();
                self.message_lines.set(lines.len());
                // AlertScreen: the title at 70, the text at 90.
                let (top, text_top, key, fallback, color) = if let Some(alert) = alert {
                    let (key, fallback, color) = match alert {
                        Alert::Incompatible => (
                            "selectWorld.incompatible.title",
                            "Incompatible version",
                            rgba(255, 0, 0, 255),
                        ),
                        Alert::Recreate => (
                            "selectWorld.recreate.error.title",
                            "An error occurred!",
                            u32::MAX,
                        ),
                    };
                    (70.0, 90.0, key, fallback, color)
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
                // WarningScreen's FittingMultiLineTextWidget narrates the
                // message; the title's StringWidget isn't active.
                crate::widgets::narratable(|| {
                    crate::widgets::Narratable::text(message.clone(), [box_x, box_y, box_w, box_h])
                });
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
                crate::widgets::narratable(|| {
                    crate::widgets::Narratable::checkbox(
                        label.clone(),
                        *check,
                        [x, y, width, BOX],
                        true,
                    )
                    .hovered(hover)
                });
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
            Mode::Onboarding(_) => fade_from = Some(self.draw_onboarding(atlas, batch, w, h)),
            Mode::BanNotice(notice) => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                let now = chrono::Utc::now().timestamp_millis();
                let layout = crate::confirm_screen::Choice::new(
                    &atlas.font,
                    (w, h),
                    &notice.title(),
                    &notice.message(now),
                    3,
                    100.0,
                );
                for (line, x, y) in &layout.lines {
                    batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
                }
                self.ban_buttons
                    .set([layout.buttons[0], layout.buttons[1], layout.buttons[2]]);
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
            crate::widgets::narratable(|| self.button_narration(&b).hovered(inside));
            crate::widgets::widget_cursor(inside, b.enabled);
            let is_focused = self.focused_button == Some(b.id);
            let hover = b.enabled && (inside || is_focused);
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
            if let Some((icon, size)) = b.trailing {
                // TextAndIcon: `x + width - spriteWidth - 2`, centred in the height.
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/{icon}"),
                    b.x + b.w - size - 2.0,
                    b.y + 10.0 - (size / 2.0).floor(),
                    size,
                    size,
                    u32::MAX,
                );
            }
        }
        if let Some(from) = fade_from {
            self.draw_onboarding_logo(atlas, batch, w, from);
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
        if let Mode::Select { worlds, selected } = &self.mode {
            self.select_tooltip(atlas, batch, cursor, (w, h), worlds, *selected);
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
    /// The tooltip of a world's icon (drawn with the row, as vanilla does) or, for a
    /// world that needs its files upgraded first, of the Play, Edit and Re-Create
    /// buttons (`SelectWorldScreen.updateButtonStatus`).
    fn select_tooltip(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        size: (f32, f32),
        worlds: &[WorldEntry],
        selected: Option<usize>,
    ) {
        let red = |key: &str| Text::translate(key, &[]).with_color(crate::text::RED);
        if let Some((row, (rx, ry))) = self.world_row_at(worlds, cursor, size)
            && (0.0..32.0).contains(&rx)
            && (0.0..32.0).contains(&ry)
            && let Some((_, world)) = self.visible_worlds(worlds).nth(row)
        {
            let lines = if world.corrupted {
                Vec::new()
            } else if world.locked {
                atlas.font.split(&red("selectWorld.locked"), 175.0)
            } else if world.incompatible.is_some() {
                atlas
                    .font
                    .split(&red("selectWorld.incompatible.tooltip"), 175.0)
            } else if world.downgrade {
                vec![
                    red("selectWorld.tooltip.fromNewerVersion1"),
                    red("selectWorld.tooltip.fromNewerVersion2"),
                ]
            } else {
                Vec::new()
            };
            crate::ui::tooltip(batch, atlas, &lines, cursor);
            return;
        }
        if !selected
            .and_then(|i| worlds.get(i))
            .is_some_and(|w| w.file_fixing)
        {
            return;
        }
        let (x, y) = cursor;
        let hovered = self
            .buttons(size.0, size.1)
            .into_iter()
            .find(|b| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + 20.0);
        let key = match hovered.map(|b| b.id) {
            Some(10) => "selectWorld.requiresFileFixingTooltip.play",
            Some(13) => "selectWorld.requiresFileFixingTooltip.edit",
            Some(15) => "selectWorld.requiresFileFixingTooltip.recreate",
            _ => return,
        };
        let lines = atlas.font.split(&Text::translate(key, &[]), 170.0);
        crate::ui::tooltip(batch, atlas, &lines, cursor);
    }
    /// The action and whether to play `AbstractWidget.playButtonClickSound`.
    /// Navigation buttons can return `Action::None` while still sounding.
    pub fn click(&mut self, x: f32, y: f32, size: (f32, f32)) -> (Action, bool) {
        self.remember_direct_address();
        // The game rules take the press in `press_rules`, which has the atlas.
        if self.rules.get_mut().is_some()
            || self.experiments.get_mut().is_some()
            || self.confirm_experiments.get_mut().is_some()
            || self.preparing
        {
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
        if let Mode::OptimizeConfirm { erase_cache, .. } = &mut self.mode {
            let [bx, by, bw, bh] = self.erase_check.get();
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                *erase_cache = !*erase_cache;
                return (Action::None, true);
            }
        }
        if matches!(self.mode, Mode::Main) {
            let [bx, by, bw, bh] = self.credits_button.get();
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                return (Action::Credits, true);
            }
        }
        let hit = self
            .buttons(w, h)
            .into_iter()
            .find(|b| b.enabled && x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + 20.0)
            .map(|b| b.id);
        if hit.is_some() {
            self.focused_button = hit;
        }
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
                // The title button is no longer part of the world-select screen.
                // Clear it so Enter can activate the selected world after a
                // mouse click opened Select World.
                self.focused_button = None;
                self.enter_singleplayer();
                Action::None
            }
            (Mode::Main, Some(2)) => Action::Quit,
            (Mode::BanNotice(_), Some(id @ 120..=122)) => {
                // BanNoticeScreens' callback: Open in Browser opens the link, Copy copies it
                // and answers no; all of them run `next`.
                self.next_ban_notice();
                match id {
                    120 => Action::OpenUri(crate::ban_notice::SUSPENSION_HELP.to_owned()),
                    121 => Action::CopyText(crate::ban_notice::SUSPENSION_HELP.to_owned()),
                    _ => Action::None,
                }
            }
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
                crate::multiplayer::Outcome::Connect { address, lan } => {
                    Action::Connect { address, lan }
                }
                crate::multiplayer::Outcome::None => Action::None,
            },
            (Mode::Main, Some(5)) => {
                self.mode = Mode::Multiplayer(crate::multiplayer::Screen::friend());
                Action::None
            }
            (Mode::Onboarding(_), Some(onboarding::SOUNDS)) => Action::Sounds,
            (Mode::Onboarding(_), Some(onboarding::ACCESSIBILITY)) => Action::Accessibility,
            (Mode::Onboarding(_), Some(onboarding::LANGUAGE)) => Action::Language,
            (Mode::Onboarding(_), Some(onboarding::CONTINUE)) => {
                self.onboarding_close();
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
            (Mode::BackupBeforeUpgrade { world, stem, .. }, Some(40)) => {
                let world = world.clone();
                self.backup_then_open = Some(stem.is_some());
                self.start_backup(world);
                Action::None
            }
            (Mode::BackupBeforeUpgrade { world, stem, .. }, Some(41)) => {
                let (world, stem_asked) = (world.clone(), stem.is_some());
                self.after_prompt(world, stem_asked)
            }
            // ConfirmScreen: Continue loads the world, Back (or Escape) leaves it closed.
            (Mode::LowDiskSpace { world }, Some(94)) => Action::Play {
                folder: world.folder.clone(),
            },
            (Mode::LowDiskSpace { .. }, Some(95)) => {
                self.escape();
                Action::None
            }
            (
                Mode::OptimizeConfirm {
                    world, erase_cache, ..
                },
                Some(43),
            ) => {
                let (world, erase_cache) = (world.clone(), *erase_cache);
                self.optimize_after_backup = Some(erase_cache);
                self.start_backup(world);
                Action::None
            }
            (
                Mode::OptimizeConfirm {
                    world, erase_cache, ..
                },
                Some(44),
            ) => {
                let (world, erase_cache) = (world.clone(), *erase_cache);
                self.begin_optimize(world, erase_cache);
                Action::None
            }
            (Mode::OptimizeConfirm { .. }, Some(45)) => {
                self.escape();
                Action::None
            }
            (Mode::Optimize { .. }, Some(46)) => {
                // Cancel: `upgrader.cancel()`, then the callback with false.
                if let Some(upgrader) = self.optimize.as_mut() {
                    upgrader.cancel();
                }
                self.end_optimize(false);
                Action::None
            }
            (Mode::BackupBeforeUpgrade { world, .. }, Some(42)) => {
                let folder = world.folder.clone();
                self.return_to_list(Some(&folder));
                Action::None
            }
            (Mode::Select { .. }, Some(11)) => {
                self.create_return_to_main = false;
                // `CreateWorldScreen.openCreateWorldScreen`: the world
                // generation data is read first, behind a loading screen;
                // the game opens the create screen after (`preparing`).
                self.preparing = true;
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
                        let level = Level::read(&self.saves.join(&world.folder));
                        // WorldListEntry.recreateWorld: a world that will not read is an
                        // AlertScreen over the list.
                        let Some(level) = level else {
                            self.mode = Mode::Error {
                                message: lang::get_or(
                                    "selectWorld.recreate.error.text",
                                    "Something went wrong while trying to recreate a world.",
                                ),
                                return_to: Box::new(self.mode.clone()),
                                alert: Some(Alert::Recreate),
                            };
                            return Action::None;
                        };
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
                    // WorldListEntry.canInteract: only a world that can be played joins.
                    if (on_icon || *selected == Some(index)) && world.can_play() {
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
                    34 => {
                        self.mode = Mode::OptimizeConfirm {
                            world,
                            name,
                            erase_cache: false,
                        };
                        return Action::None;
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
            (Mode::Recover { world, .. }, Some(90)) => {
                // attemptRestore: the world is backed up first (`makeBackupAndShowToast`), and the
                // restore runs once that is done.
                let world = world.clone();
                self.restore_after_backup = true;
                self.start_backup(world);
                Action::None
            }
            (Mode::Recovered { world, .. }, Some(92)) => {
                let world = world.clone();
                self.play(world)
            }
            (Mode::Recover { .. } | Mode::Recovered { .. }, Some(91 | 93)) => {
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
            ) if !matches!(id, 21 | 22 | 30 | 31) => {
                options.press(id, creative);
                Action::None
            }
            // CreateWorldScreen.openExperimentsScreen: the feature packs the
            // jar ships, switched on or off.
            (Mode::Create { options, .. }, Some(30)) => {
                let flags = experiments::repository_flags(&self.resources, &options.data_packs);
                *self.experiments.get_mut() = Some(experiments::ExperimentsScreen::new(flags));
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
        // `GenericMessageScreen`: no field takes input during the read.
        if self.preparing {
            return;
        }
        self.edit_field(|edit| edit.insert_text(text));
    }
    pub fn backspace(&mut self) {
        self.field_key(winit::keyboard::KeyCode::Backspace, false, false);
    }
    /// EditBox.keyPressed: moving and deleting in the focused field.
    pub fn field_key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        if self.preparing {
            return false;
        }
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
            // WorldSelectionList.notifyListUpdated: the filtered list is
            // narrated at once.
            crate::widgets::narrate_now();
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
        self.confirm_key_size((854.0, 480.0)).0
    }
    /// `confirm_key` with actual viewport size for button geometry lookup.
    /// Returns the triggered action and whether a UI button click sound should play.
    pub fn confirm_key_size(&mut self, size: (f32, f32)) -> (Action, bool) {
        self.remember_direct_address();
        if self.rules.get_mut().is_some() {
            return (Action::None, false);
        }
        if matches!(self.mode, Mode::Edit { .. }) && self.focused_button.is_none() {
            self.confirm_edit();
            return (Action::None, true);
        }
        if self.list_focused() && self.focused_button.is_none() {
            let action = self.join_selected();
            let sound = matches!(action, Action::Play { .. });
            return (action, sound);
        }
        if let Some(id) = self.focused_button {
            let (w, h) = size;
            let buttons = self.buttons(w, h);
            if let Some(b) = buttons.into_iter().find(|b| b.id == id && b.enabled) {
                let action = self.click_action(b.x, b.y, (w, h), Some(id));
                return (action, true);
            }
            return (Action::None, false);
        }
        match &self.mode {
            Mode::Create { .. } => {
                let action = self.create_action();
                let sound = matches!(action, Action::Create { .. });
                (action, sound)
            }
            Mode::Multiplayer(screen) => match screen.confirm() {
                crate::multiplayer::Outcome::Connect { address, lan } => {
                    (Action::Connect { address, lan }, true)
                }
                _ => (Action::None, false),
            },
            _ => (Action::None, false),
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
        if !world.can_play() {
            return Action::None;
        }
        self.play(world)
    }
    pub fn tab(&mut self) {
        self.tab_dir(false, (854.0, 480.0));
    }
    /// Tab navigation through widgets and fields in either forward or reverse order.
    pub fn tab_dir(&mut self, reverse: bool, size: (f32, f32)) {
        if self.rules.get_mut().is_some() {
            return;
        }
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.tab();
            return;
        }
        let (w, h) = size;
        let buttons = self.buttons(w, h);
        let enabled: Vec<u8> = buttons.iter().filter(|b| b.enabled).map(|b| b.id).collect();

        match &mut self.mode {
            Mode::Select { .. } => {
                if self.search_focused {
                    self.search_focused = false;
                    self.focused_button = enabled.first().copied();
                } else if let Some(cur) = self.focused_button {
                    if let Some(pos) = enabled.iter().position(|&id| id == cur) {
                        if !reverse && pos + 1 < enabled.len() {
                            self.focused_button = Some(enabled[pos + 1]);
                        } else if reverse && pos > 0 {
                            self.focused_button = Some(enabled[pos - 1]);
                        } else {
                            self.focused_button = None;
                            self.search_focused = true;
                        }
                    } else {
                        self.focused_button = None;
                        self.search_focused = true;
                    }
                } else {
                    self.search_focused = true;
                }
            }
            Mode::Create { focus, options, .. } => {
                let field_id = if options.tab == 1 { 1 } else { 0 };
                if *focus != 2 {
                    *focus = 2;
                    self.focused_button = if reverse {
                        enabled.last().copied()
                    } else {
                        enabled.first().copied()
                    };
                } else if let Some(cur) = self.focused_button {
                    if let Some(pos) = enabled.iter().position(|&id| id == cur) {
                        if !reverse && pos + 1 < enabled.len() {
                            self.focused_button = Some(enabled[pos + 1]);
                        } else if reverse && pos > 0 {
                            self.focused_button = Some(enabled[pos - 1]);
                        } else {
                            self.focused_button = None;
                            *focus = field_id;
                        }
                    } else {
                        self.focused_button = None;
                        *focus = field_id;
                    }
                } else {
                    *focus = field_id;
                }
            }
            Mode::Edit { focused, .. } => {
                if *focused {
                    *focused = false;
                    self.focused_button = if reverse {
                        enabled.last().copied()
                    } else {
                        enabled.first().copied()
                    };
                } else if let Some(cur) = self.focused_button {
                    if let Some(pos) = enabled.iter().position(|&id| id == cur) {
                        if !reverse && pos + 1 < enabled.len() {
                            self.focused_button = Some(enabled[pos + 1]);
                        } else if reverse && pos > 0 {
                            self.focused_button = Some(enabled[pos - 1]);
                        } else {
                            self.focused_button = None;
                            *focused = true;
                        }
                    } else {
                        self.focused_button = None;
                        *focused = true;
                    }
                } else {
                    *focused = true;
                }
            }
            _ => {
                if enabled.is_empty() {
                    return;
                }
                let next = match self.focused_button {
                    Some(cur) => {
                        if let Some(pos) = enabled.iter().position(|&id| id == cur) {
                            if reverse {
                                if pos == 0 {
                                    enabled.last().copied()
                                } else {
                                    Some(enabled[pos - 1])
                                }
                            } else {
                                if pos + 1 >= enabled.len() {
                                    enabled.first().copied()
                                } else {
                                    Some(enabled[pos + 1])
                                }
                            }
                        } else {
                            if reverse {
                                enabled.last().copied()
                            } else {
                                enabled.first().copied()
                            }
                        }
                    }
                    None => {
                        if reverse {
                            enabled.last().copied()
                        } else {
                            enabled.first().copied()
                        }
                    }
                };
                self.focused_button = next;
            }
        }
    }
    /// 2D spatial arrow key navigation across enabled buttons.
    pub fn arrow_key(&mut self, key: winit::keyboard::KeyCode, size: (f32, f32)) {
        use winit::keyboard::KeyCode;
        if self.rules.get_mut().is_some() {
            return;
        }
        if self.has_caret_field() {
            return;
        }
        if self.list_focused()
            && self.focused_button.is_none()
            && let Mode::Select { worlds, selected } = &mut self.mode
        {
            let len = worlds.len();
            if len > 0 {
                match key {
                    KeyCode::ArrowUp => {
                        *selected = Some(selected.map_or(0, |i| i.saturating_sub(1)));
                        return;
                    }
                    KeyCode::ArrowDown => {
                        *selected = Some(selected.map_or(0, |i| (i + 1).min(len - 1)));
                        return;
                    }
                    _ => {}
                }
            }
        }
        let (w, h) = size;
        let buttons = self.buttons(w, h);
        let enabled: Vec<&Button> = buttons.iter().filter(|b| b.enabled).collect();
        if enabled.is_empty() {
            return;
        }
        let Some(cur_id) = self.focused_button else {
            self.focused_button = Some(enabled[0].id);
            return;
        };
        let Some(cur_b) = enabled.iter().find(|b| b.id == cur_id) else {
            self.focused_button = Some(enabled[0].id);
            return;
        };
        let (cx, cy) = (cur_b.x + cur_b.w / 2.0, cur_b.y + 10.0);
        let mut best: Option<(u8, f32)> = None;
        for b in &enabled {
            if b.id == cur_id {
                continue;
            }
            let (bx, by) = (b.x + b.w / 2.0, b.y + 10.0);
            let (dx, dy) = (bx - cx, by - cy);
            let valid = match key {
                KeyCode::ArrowUp => dy < -4.0,
                KeyCode::ArrowDown => dy > 4.0,
                KeyCode::ArrowLeft => dx < -4.0,
                KeyCode::ArrowRight => dx > 4.0,
                _ => false,
            };
            if !valid {
                continue;
            }
            let dist = match key {
                KeyCode::ArrowUp | KeyCode::ArrowDown => dy.abs() + dx.abs() * 2.0,
                KeyCode::ArrowLeft | KeyCode::ArrowRight => dx.abs() + dy.abs() * 2.0,
                _ => f32::MAX,
            };
            if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                best = Some((b.id, dist));
            }
        }
        if let Some((next_id, _)) = best {
            self.focused_button = Some(next_id);
        }
    }
    /// The keyboard's focus landed on the world row at `point`
    /// (`AbstractSelectionList.setFocused`): it is selected, never joined,
    /// and the search box no longer has the keys. Whether a row was there.
    pub fn focus_world_row(&mut self, point: (f32, f32), size: (f32, f32)) -> bool {
        if self.rules.get_mut().is_some() {
            return false;
        }
        let Mode::Select { worlds, .. } = &self.mode else {
            return false;
        };
        let size = (size.0.ceil(), size.1.ceil());
        let Some(index) = self
            .world_row_at(worlds, point, size)
            .and_then(|(row, _)| self.visible_worlds(worlds).nth(row).map(|(i, _)| i))
        else {
            return false;
        };
        self.search_focused = false;
        if let Mode::Select { selected, .. } = &mut self.mode {
            *selected = Some(index);
        }
        true
    }
    /// Which screen the title shows (a new one starts the keyboard's focus
    /// again, `Screen.setInitialFocus`).
    pub fn screen_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.mode).hash(&mut hasher);
        if let Mode::Multiplayer(screen) = &self.mode {
            std::mem::discriminant(screen).hash(&mut hasher);
        }
        self.rules.borrow().is_some().hash(&mut hasher);
        hasher.finish()
    }
    /// The keyboard's focus left a text field for another widget: the field
    /// loses the keys (`EditBox.setFocused(false)`).
    pub fn blur_field(&mut self) {
        match &mut self.mode {
            Mode::Select { .. } => self.search_focused = false,
            Mode::Edit { focused, .. } => *focused = false,
            Mode::Create { focus, .. } => *focus = 2,
            _ => {}
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
        if self.confirm_experiments.borrow().is_some() {
            let click = self
                .confirm_experiments
                .borrow()
                .as_ref()
                .and_then(|s| s.click(atlas, cursor, size));
            match click {
                Some(confirm_experiments::Click::Details) => {
                    self.confirm_experiments
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .details = true;
                    return true;
                }
                Some(confirm_experiments::Click::Proceed) => {
                    let pending = self
                        .confirm_experiments
                        .borrow_mut()
                        .take()
                        .map(|s| s.pending);
                    // `tryApplyNewDataPacks` accepted: the packs apply, and
                    // their data is read again (`applyNewPackConfig`'s load
                    // screen).
                    if let Some(pending) = pending {
                        self.set_data_packs(pending);
                        self.preparing = true;
                    }
                    return true;
                }
                Some(confirm_experiments::Click::Back) => {
                    let screen = self.confirm_experiments.borrow();
                    let in_details = screen.as_ref().is_some_and(|s| s.details);
                    drop(screen);
                    if in_details {
                        self.confirm_experiments
                            .borrow_mut()
                            .as_mut()
                            .unwrap()
                            .details = false;
                    } else {
                        // `onAbort`: the confirm declines, the Data Packs
                        // screen reopens (the game side holds the folder).
                        self.confirm_experiments.borrow_mut().take();
                        self.reopen_data_packs = true;
                    }
                    return true;
                }
                None => return false,
            }
        }
        if self.experiments.borrow().is_some() {
            let click = self
                .experiments
                .borrow()
                .as_ref()
                .and_then(|s| s.click(atlas, cursor, size));
            match click {
                // ExperimentsScreen: a switch only flips the screen's own
                // list; the selection is written back when Done is pressed.
                Some(experiments::Click::Switch(i)) => {
                    if let Some(f) = self
                        .experiments
                        .borrow_mut()
                        .as_mut()
                        .and_then(|s| s.flags.get_mut(i))
                    {
                        f.enabled = !f.enabled;
                    }
                    return true;
                }
                Some(experiments::Click::Outcome(outcome)) => {
                    // ExperimentsScreen.onDone: the other packs keep their
                    // order, the selected features are appended in reverse
                    // screen order. Cancel drops the switches.
                    if outcome == experiments::Outcome::Done
                        && let Mode::Create { options, .. } = &mut self.mode
                        && let Some(flags) = self.experiments.borrow().as_ref().map(|s| {
                            s.flags
                                .iter()
                                .map(|f| (f.id.clone(), f.enabled))
                                .collect::<Vec<_>>()
                        })
                    {
                        for (id, _) in &flags {
                            options.data_packs.retain(|p| p != id);
                        }
                        for (id, _) in flags.iter().rev().filter(|(_, e)| *e) {
                            options.data_packs.push(id.clone());
                        }
                    }
                    *self.experiments.get_mut() = None;
                    // `applyNewPackConfig`'s load screen: the data is read
                    // again for the packs the screen picked (Done only).
                    if outcome == experiments::Outcome::Done {
                        self.preparing = true;
                    }
                    return true;
                }
                None => return false,
            }
        }
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
    /// `Gui.buildInitialScreens`' chain: the next queued ban notice, else the title.
    fn next_ban_notice(&mut self) {
        self.mode = match self.ban_queue.pop_front() {
            Some(notice) => Mode::BanNotice(notice),
            None => Mode::Main,
        };
    }
    /// The account fetches vanilla starts in `Minecraft`'s constructor
    /// (`userPropertiesFuture`, `profileFuture`), run once per launch on a thread, only
    /// for a real online session (offline and demo have none). Their result decides
    /// `allowsMultiplayer` and queues `Gui.addInitialScreens`' ban notices, which open
    /// as soon as the title is showing.
    fn poll_account(&mut self) {
        use crate::ban_notice::{AccountStatus, initial_notices};
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        static STATUS: OnceLock<AccountStatus> = OnceLock::new();
        static STARTED: AtomicBool = AtomicBool::new(false);
        static QUEUED: AtomicBool = AtomicBool::new(false);
        let Some(session) = crate::auth::current() else {
            return;
        };
        if !STARTED.swap(true, Ordering::Relaxed) {
            let session = session.clone();
            let spawned = std::thread::Builder::new()
                .name("Account status".into())
                .spawn(move || {
                    let _ = STATUS.set(crate::auth::fetch_account_status(&session));
                });
            if spawned.is_err() {
                log::warn!("Horizon: could not start the account status fetch");
            }
        }
        let Some(status) = STATUS.get() else {
            return;
        };
        self.allows_multiplayer = status.allows_multiplayer();
        if !QUEUED.swap(true, Ordering::Relaxed) {
            self.ban_queue
                .extend(initial_notices(status, &session.name));
        }
        if matches!(self.mode, Mode::Main) {
            self.next_ban_notice();
        }
    }
    pub fn escape(&mut self) {
        // GenericMessageScreen.shouldCloseOnEsc is false: the preparing
        // read takes no input, the player's options survive it.
        if self.preparing {
            return;
        }
        if self.confirm_experiments.get_mut().take().is_some() {
            self.reopen_data_packs = true;
            return;
        }
        if self.experiments.get_mut().take().is_some() {
            return;
        }
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
            // OptimizeWorldScreen: `shouldCloseOnEsc` is false; the Backup wait takes no input.
            Mode::Backup { .. } | Mode::Optimize { .. } => {}
            // BackupConfirmScreen.keyPressed: Escape runs `onCancel`, the Edit World screen again.
            Mode::OptimizeConfirm { world, name, .. } => {
                self.mode = Mode::Edit {
                    world: world.clone(),
                    name: name.clone(),
                    focused: true,
                };
            }
            Mode::Edit { world, .. }
            | Mode::Delete { world }
            | Mode::Recover { world, .. }
            | Mode::Recovered { world, .. }
            | Mode::LowDiskSpace { world }
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
            // Screen.keyPressed: Escape closes the screen, which fades out.
            Mode::Onboarding(_) => self.onboarding_close(),
            // ConfirmScreen.keyPressed: Escape answers no, which runs `next`.
            Mode::BanNotice(_) => self.next_ban_notice(),
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
    fn ban_notices_chain_into_the_title_and_gate_multiplayer() {
        use crate::ban_notice::{BanDetails, Notice};
        let mut title = Title::new(
            PathBuf::new(),
            Path::new("/nonexistent/horizon-test-resources"),
        );
        let multiplayer = |title: &Title| {
            title
                .buttons(427.0, 240.0)
                .into_iter()
                .find(|b| b.id == 4)
                .unwrap()
                .enabled
        };
        assert!(multiplayer(&title));
        title.allows_multiplayer = false;
        assert!(!multiplayer(&title));
        // Gui.buildInitialScreens: each notice runs `next`, the last one opens the title.
        let ban = BanDetails {
            expires_ms: None,
            reason: None,
            reason_message: None,
        };
        title.ban_queue = [Notice::Name("Steve".into()), Notice::Skin].into();
        title.mode = Mode::BanNotice(Notice::Multiplayer(ban));
        assert_eq!(title.buttons(427.0, 240.0).len(), 3);
        title.escape();
        assert!(matches!(&title.mode, Mode::BanNotice(Notice::Name(n)) if n == "Steve"));
        title.escape();
        assert!(matches!(title.mode, Mode::BanNotice(Notice::Skin)));
        title.next_ban_notice();
        assert!(matches!(title.mode, Mode::Main));
    }

    #[test]
    fn the_bottom_text_button_opens_the_credits() {
        // TitleScreen.init's PlainTextButton: a press inside its drawn rect.
        let mut title = Title::new(
            PathBuf::new(),
            Path::new("/nonexistent/horizon-test-resources"),
        );
        title.credits_button.set([300.0, 230.0, 120.0, 10.0]);
        assert_eq!(
            title.click(310.0, 235.0, (427.0, 240.0)),
            (Action::Credits, true)
        );
        assert_eq!(
            title.click(310.0, 225.0, (427.0, 240.0)),
            (Action::None, false)
        );
    }

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
