//! The dedicated server's chat filter (docs/chat-filter.md), without the game: `server.toml`'s
//! `[chat_filter]`, the matcher for the slur list (`data/slurs.txt`, or the owner's `words_file`), and
//! the escalation (a refused message, then a mute, then a temporary ban), with the mutes kept in a file
//! so that they outlive a restart. The server's hub (`horizon_client::lan`) asks it before it relays a
//! player's text and carries out what it decides.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod normalise;

pub use normalise::{Run, Text};

/// The list built into the game, used when `words_file` is empty.
pub const DEFAULT_WORDS: &str = include_str!("../data/slurs.txt");

/// The mutes, beside server.properties.
pub const MUTES_FILE: &str = "chat-filter-mutes.json";

/// `server.toml`'s `[chat_filter]`: on by default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    /// The hit, counted within `window_minutes`, that mutes the player; 0: never.
    pub mute_after: u32,
    /// The hit that bans the player for `tempban_hours`; 0: never.
    pub tempban_after: u32,
    /// How long a hit counts towards the next ones (the time spent muted doesn't count).
    pub window_minutes: u32,
    pub mute_minutes: u32,
    pub tempban_hours: u32,
    /// The slur list, relative to the server's folder; empty: the list built into the game.
    pub words_file: String,
    /// Every refused text with the player and what followed, relative to the server's folder.
    pub log: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            mute_after: 2,
            tempban_after: 3,
            window_minutes: 10,
            mute_minutes: 10,
            tempban_hours: 24,
            words_file: String::new(),
            log: "logs/chat-filter.log".into(),
        }
    }
}

impl Config {
    /// The `[chat_filter]` section of a whole `server.toml`; the default without it.
    pub fn from_server_toml(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            chat_filter: Config,
        }
        // Other sections are the supervisor's to check: only this one is read here.
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut only = toml::Table::new();
        if let Some(section) = table.remove("chat_filter") {
            only.insert("chat_filter".into(), section);
        }
        let file: File = only
            .try_into()
            .map_err(|e: toml::de::Error| format!("[chat_filter]: {e}"))?;
        file.chat_filter.check()?;
        Ok(file.chat_filter)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.window_minutes == 0 {
            return Err("chat_filter.window_minutes must be 1 or more".into());
        }
        Ok(())
    }
}

/// How an entry of the list matches.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Inside a longer word too.
    Embedded,
    /// Only as a whole word.
    Whole,
    /// Inside a word, or spread over several words from the start of one.
    Spread,
}

#[derive(Debug)]
struct Entry {
    runs: Vec<Run>,
    kind: Kind,
}

/// The slur list, compiled: each entry's letters as runs, indexed by first letter.
#[derive(Debug)]
pub struct Filter {
    entries: Vec<Entry>,
    by_first: [Vec<u16>; 27],
    /// Words never matched: exact (`false`) or as a prefix (`true`).
    allowed: Vec<(Vec<Run>, bool)>,
}

impl Filter {
    /// Compiles a list in `data/slurs.txt`'s format.
    pub fn new(list: &str) -> Self {
        let mut f = Filter {
            entries: Vec::new(),
            by_first: Default::default(),
            allowed: Vec::new(),
        };
        for line in list.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(word) = line.strip_prefix('!') {
                let (word, prefix) = match word.strip_suffix('*') {
                    Some(w) => (w, true),
                    None => (word, false),
                };
                f.allowed.push((normalise::word(word), prefix));
                continue;
            }
            let (kind, word) = match line.as_bytes()[0] {
                b'=' => (Kind::Whole, &line[1..]),
                b'~' => (Kind::Spread, &line[1..]),
                _ => (Kind::Embedded, line),
            };
            let runs = normalise::word(word);
            let Some(first) = runs.first() else {
                continue;
            };
            f.by_first[usize::from(first.letter)].push(f.entries.len() as u16);
            f.entries.push(Entry { runs, kind });
        }
        f
    }

    /// The built-in list.
    pub fn default_list() -> Self {
        Self::new(DEFAULT_WORDS)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `text` holds a slur of the list.
    pub fn matches(&self, text: &str) -> bool {
        let t = Text::new(text);
        let runs = &t.runs;
        for &(start, end) in t.words.iter().chain(&t.spaced) {
            let word = &runs[start..end];
            if self.is_allowed(word) {
                continue;
            }
            for at in 0..word.len() {
                for &e in &self.by_first[usize::from(word[at].letter)] {
                    let e = &self.entries[usize::from(e)];
                    let whole = e.kind == Kind::Whole;
                    if (!whole || at == 0) && fits(&e.runs, &word[at..], whole) {
                        return true;
                    }
                }
            }
        }
        // Spread entries over the whole text, from the start of a word, across at least two.
        for (w, &at) in t.joined_at.iter().enumerate() {
            for &e in &self.by_first[usize::from(t.joined[at].letter)] {
                let e = &self.entries[usize::from(e)];
                if e.kind == Kind::Spread
                    && fits(&e.runs, &t.joined[at..], false)
                    && t.spans_words(w, e.runs.len())
                {
                    return true;
                }
            }
        }
        false
    }

    fn is_allowed(&self, word: &[Run]) -> bool {
        self.allowed.iter().any(|(a, prefix)| {
            if *prefix {
                // A prefix: its last run may be part of a longer one.
                word.len() >= a.len() && fits(a, word, false)
            } else {
                fits(a, word, true)
            }
        })
    }
}

/// Whether `entry`'s runs start `runs` (are all of it with `whole`): the same letters, each at least
/// as many times as the entry has it.
fn fits(entry: &[Run], runs: &[Run], whole: bool) -> bool {
    if runs.len() < entry.len() || (whole && runs.len() != entry.len()) {
        return false;
    }
    entry
        .iter()
        .zip(runs)
        .all(|(e, r)| e.letter == r.letter && r.count >= e.count)
}

/// What a hit leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The text is refused, nothing more.
    Refused,
    /// Refused, and the player can't chat until then (epoch milliseconds).
    Muted { until: i64 },
    /// Refused, and the player is banned until then.
    Banned { until: i64 },
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Refused => "refused",
            Action::Muted { .. } => "muted",
            Action::Banned { .. } => "banned",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mute {
    pub uuid: String,
    pub name: String,
    /// Epoch milliseconds.
    pub until: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct MutesFile {
    mutes: Vec<Mute>,
}

/// One player's hits: how many, and when the last one stops counting.
#[derive(Clone, Copy, Debug)]
struct Strikes {
    count: u32,
    counts_until: i64,
}

/// The filter, the players' hits and the mutes.
pub struct Guard {
    pub config: Config,
    pub filter: Filter,
    strikes: HashMap<u128, Strikes>,
    mutes: HashMap<u128, Mute>,
    /// Where the mutes are kept; `None` keeps them in memory only (tests).
    file: Option<PathBuf>,
}

const MINUTE: i64 = 60_000;

impl Guard {
    pub fn new(config: Config, filter: Filter, file: Option<PathBuf>) -> Self {
        let mut mutes = HashMap::new();
        if let Some(text) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok())
            && let Ok(saved) = serde_json::from_str::<MutesFile>(&text)
        {
            for m in saved.mutes {
                if let Some(uuid) = parse_uuid(&m.uuid) {
                    mutes.insert(uuid, m);
                }
            }
        }
        Self {
            config,
            filter,
            strikes: HashMap::new(),
            mutes,
            file,
        }
    }

    /// The server's guard: `words_file` or the built-in list, the mutes in `dir`. An unreadable
    /// `words_file` falls back to the built-in list, with why.
    pub fn load(config: Config, dir: &Path) -> (Self, Option<String>) {
        let mut problem = None;
        let filter = if config.words_file.is_empty() {
            Filter::default_list()
        } else {
            match std::fs::read_to_string(dir.join(&config.words_file)) {
                Ok(text) => Filter::new(&text),
                Err(e) => {
                    problem = Some(format!(
                        "chat_filter.words_file {}: {e}; using the built-in list",
                        config.words_file
                    ));
                    Filter::default_list()
                }
            }
        };
        (
            Self::new(config, filter, Some(dir.join(MUTES_FILE))),
            problem,
        )
    }

    /// When the player's mute ends, if they are muted at `now`.
    pub fn muted_until(&self, uuid: u128, now: i64) -> Option<i64> {
        self.mutes
            .get(&uuid)
            .map(|m| m.until)
            .filter(|&until| until > now)
    }

    /// A hit by the player at `now`: the hit's number in the window, and what it leads to.
    pub fn strike(&mut self, uuid: u128, name: &str, now: i64) -> (u32, Action) {
        let c = &self.config;
        let window = i64::from(c.window_minutes) * MINUTE;
        let s = self.strikes.entry(uuid).or_insert(Strikes {
            count: 0,
            counts_until: 0,
        });
        if now > s.counts_until {
            s.count = 0;
        }
        s.count += 1;
        s.counts_until = now + window;
        let count = s.count;
        let action = if c.tempban_after > 0 && count >= c.tempban_after {
            self.strikes.remove(&uuid);
            Action::Banned {
                until: now + i64::from(c.tempban_hours) * 60 * MINUTE,
            }
        } else if c.mute_after > 0 && count >= c.mute_after {
            let until = now + i64::from(c.mute_minutes) * MINUTE;
            // The window starts again when the mute ends.
            s.counts_until = until + window;
            self.mutes.insert(
                uuid,
                Mute {
                    uuid: uuid_string(uuid),
                    name: name.to_owned(),
                    until,
                },
            );
            self.save();
            Action::Muted { until }
        } else {
            Action::Refused
        };
        (count, action)
    }

    /// Lifts a player's mute (and forgets their hits), by name; the name as muted, if they were.
    pub fn unmute(&mut self, name: &str) -> Option<String> {
        let uuid = *self
            .mutes
            .iter()
            .find(|(_, m)| m.name.eq_ignore_ascii_case(name))?
            .0;
        let mute = self.mutes.remove(&uuid)?;
        self.strikes.remove(&uuid);
        self.save();
        Some(mute.name)
    }

    fn save(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let now = now_millis();
        self.mutes.retain(|_, m| m.until > now);
        let mut mutes: Vec<Mute> = self.mutes.values().cloned().collect();
        mutes.sort_by(|a, b| a.name.cmp(&b.name));
        if let Ok(text) = serde_json::to_string_pretty(&MutesFile { mutes }) {
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }
}

/// One line of `logs/chat-filter.log`, without its newline.
pub fn log_line(
    now: i64,
    name: &str,
    uuid: u128,
    place: &str,
    count: u32,
    action: Action,
    text: &str,
) -> String {
    let text: String = text
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    format!(
        "{} {name} {} {place} hit {count} {}: {text}",
        utc(now),
        uuid_string(uuid),
        action.name()
    )
}

/// Appends a line to the log at `path` (its folder made if needed).
pub fn append_log(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for epoch milliseconds.
pub fn utc(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn parse_uuid(s: &str) -> Option<u128> {
    u128::from_str_radix(&s.replace('-', ""), 16).ok()
}

#[cfg(test)]
mod tests;
