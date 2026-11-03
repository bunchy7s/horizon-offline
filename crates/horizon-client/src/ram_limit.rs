//! The player's RAM limit: from the launcher's slider at the start (`HORIZON_MEMORY_LIMIT_MB`), and
//! `/ramlimit` while the game runs. The window answers that command itself, so it works on any
//! server. On Windows the system holds the game under the limit (`horizon_threads::memory_limit`).
//! Under pressure, [`Governor`] turns far terrain off and lowers the render distance in use, never
//! the option the player saved, and gives them back when there is room. Only a game that can't run
//! under the limit at all ends with a crash report.
use horizon_threads::capped;
use horizon_threads::memory_limit::{self, Pressure};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The command's name, which the chat box lists with the server's.
pub const COMMAND: &str = "ramlimit";
const ENV: &str = "HORIZON_MEMORY_LIMIT_MB";
/// A ceiling at the allocator: past it the game ends instead of paging.
const HARD_ENV: &str = "HORIZON_MEMORY_HARD_MB";
const MIB: u64 = 1024 * 1024;
/// The lowest render distance the game runs at (`Options.renderDistance`'s lowest).
pub const LOWEST_DISTANCE: i32 = 2;
/// A step down waits this long for the last one to free its memory.
const STEP_DOWN_EVERY: Duration = Duration::from_secs(3);
/// A step back up waits this long after any change, so the game doesn't go up and down.
const STEP_UP_EVERY: Duration = Duration::from_secs(30);

/// What a [`Governor`] step changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    FarTerrainOff,
    Lowered(i32),
    Raised(i32),
    FarTerrainBack,
}

/// How far the game has stepped down to stay under the limit.
#[derive(Debug, Default)]
pub struct Governor {
    /// The render distance in use, when lower than the player's.
    cap: Option<i32>,
    far_off: bool,
    changed: Option<Instant>,
}

impl Governor {
    /// The render distance to use when the player chose `chosen`.
    pub fn distance(&self, chosen: i32) -> i32 {
        self.cap.map_or(chosen, |cap| cap.min(chosen))
    }

    /// Whether far terrain is off for memory.
    pub fn far_off(&self) -> bool {
        self.far_off
    }

    /// Whether there is nothing left to give up.
    pub fn at_lowest(&self, chosen: i32, has_far: bool) -> bool {
        (self.far_off || !has_far) && self.distance(chosen) <= LOWEST_DISTANCE
    }

    /// Gives everything back at once: the limit was lifted.
    pub fn reset(&mut self) -> bool {
        let stepped = self.cap.is_some() || self.far_off;
        *self = Governor::default();
        stepped
    }

    /// One look at how tight memory is.
    pub fn step(
        &mut self,
        now: Instant,
        pressure: Pressure,
        chosen: i32,
        has_far: bool,
    ) -> Option<Step> {
        let waited = |every| {
            self.changed
                .is_none_or(|at| now.duration_since(at) >= every)
        };
        let step = match pressure {
            Pressure::Over { keep } => {
                if !waited(STEP_DOWN_EVERY) {
                    return None;
                }
                if has_far && !self.far_off {
                    self.far_off = true;
                    Some(Step::FarTerrainOff)
                } else {
                    let now_at = self.distance(chosen);
                    // Chunks' memory grows with the distance's square.
                    let next = ((f64::from(now_at) * keep.sqrt()) as i32)
                        .min(now_at - 1)
                        .max(LOWEST_DISTANCE);
                    (next < now_at).then(|| {
                        self.cap = Some(next);
                        Step::Lowered(next)
                    })
                }
            }
            Pressure::Room if waited(STEP_UP_EVERY) => match self.cap {
                Some(cap) => {
                    let next = cap + 1;
                    self.cap = (next < chosen).then_some(next);
                    Some(Step::Raised(next.min(chosen)))
                }
                None if self.far_off => {
                    self.far_off = false;
                    Some(Step::FarTerrainBack)
                }
                None => None,
            },
            _ => None,
        };
        if step.is_some() {
            self.changed = Some(now);
        }
        step
    }
}

/// The limit in bytes, if there is one.
pub fn limit() -> Option<u64> {
    memory_limit::get()
}

/// The memory the game holds, in bytes.
pub fn used() -> Option<u64> {
    memory_limit::resident_bytes()
}

/// See [`memory_limit::frame`].
pub fn frame() {
    memory_limit::frame();
}

/// A game this slow, faulting hard under a cap the system enforces, is short of memory.
const SLOW_FPS: f32 = 30.;

/// See [`memory_limit::pressure`]. Under a cap the system enforces the game is already under the
/// limit, so nothing the player sees goes unless the game is slow as well.
pub fn pressure(limit: u64, fps: f32) -> Pressure {
    if capped::limit().is_some() {
        // A hard ceiling is a budget of its own: what the process commits
        // decides, and a smooth frame does not excuse holding more. Shedding
        // down to it is the point, so frames suffering is not the trigger.
        return horizon_threads::allocator_commit()
            .0
            .try_into()
            .map_or(Pressure::Steady, |commit| {
                memory_limit::pressure_of_commit(commit, limit)
            });
    }
    let pressure = memory_limit::pressure(limit);
    needed(pressure, memory_limit::system_enforced(), fps)
}

fn needed(pressure: Pressure, enforced: bool, fps: f32) -> Pressure {
    match pressure {
        Pressure::Over { .. } if enforced && fps >= SLOW_FPS => Pressure::Steady,
        pressure => pressure,
    }
}

/// See [`memory_limit::can_shrink`].
pub fn can_shrink() {
    memory_limit::can_shrink();
}

/// Where a crash report goes, and from which side.
static REPORT_TO: OnceLock<(PathBuf, &'static str)> = OnceLock::new();

/// Applies the launcher's limit, if there is one.
pub fn start(home: &Path, side: &'static str) {
    let _ = REPORT_TO.set((home.to_path_buf(), side));
    if let Ok(value) = std::env::var(ENV) {
        match value.trim().parse::<u64>() {
            Ok(mb) => set(Some(mb)),
            Err(_) => log::warn!("{ENV}={value} is not a number of MiB: no limit set"),
        }
    }
    if let Ok(value) = std::env::var(HARD_ENV) {
        match value.trim().parse::<u64>() {
            Ok(mb) => set_hard(Some(mb)),
            Err(_) => log::warn!("{HARD_ENV}={value} is not a number of MiB: no hard limit set"),
        }
    }
}

fn set(mb: Option<u64>) {
    memory_limit::set(mb.map(|mb| mb.saturating_mul(MIB)), exceeded);
}

/// Sets the ceiling at the allocator, and with no working set limit yet, gives
/// the governor the same budget: alone, a hard ceiling would end the game
/// before anything stepped down.
fn set_hard(mb: Option<u64>) {
    capped::set_limit(mb.map(|mb| mb.saturating_mul(MIB)));
    if let Some(mb) = mb
        && memory_limit::get().is_none()
    {
        set(Some(mb));
    }
}

fn exceeded(used: u64, limit: u64) {
    let message = format!(
        "The game can't run under its RAM limit: it holds {} MiB and the limit is {} MiB. Raise \
         the limit in the launcher's Launch panel.",
        used / MIB,
        limit / MIB
    );
    log::error!("{message}");
    if let Some((home, side)) = REPORT_TO.get() {
        crate::crash::report_memory_limit(home, side, message);
    }
    crate::logging::flush();
    std::process::exit(101);
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Show,
    Set(Option<u64>),
    SetHard(Option<u64>),
    Usage,
}

/// `/ramlimit`, `/ramlimit <MiB>`, `/ramlimit off`. None: another command.
fn parse(command: &str) -> Option<Request> {
    let mut words = command.split_whitespace();
    if words.next() != Some(COMMAND) {
        return None;
    }
    Some(match (words.next(), words.next(), words.next()) {
        (None, _, _) => Request::Show,
        (Some("off"), None, _) => Request::Set(None),
        (Some("hard"), Some("off"), None) => Request::SetHard(None),
        (Some("hard"), Some(mb), None) => mb
            .parse()
            .map_or(Request::Usage, |mb| Request::SetHard(Some(mb))),
        (Some(mb), None, _) => mb
            .parse()
            .map_or(Request::Usage, |mb| Request::Set(Some(mb))),
        _ => Request::Usage,
    })
}

fn describe(limit: Option<u64>, hard: Option<u64>, used: Option<u64>) -> String {
    let limit = limit.map_or("off".to_owned(), |limit| format!("{} MB", limit / MIB));
    let hard_off = hard.is_none();
    let hard = hard.map_or("off".to_owned(), |hard| format!("{} MB", hard / MIB));
    let holds = used.map_or(String::new(), |used| {
        // With a hard limit, the number that counts is the process's commit,
        // since that is what the budget is spent on.
        let what = if hard_off { "game" } else { "process" };
        format!(" The {what} holds {} MB.", used / MIB)
    });
    format!("RAM limit: {limit}. Hard limit: {hard}.{holds}")
}

/// Runs `/ramlimit` and returns what to say in chat, or None when `command` is another command.
pub fn run(command: &str) -> Option<String> {
    Some(match parse(command)? {
        Request::Show => describe(
            memory_limit::get(),
            capped::limit(),
            match capped::limit() {
                Some(_) => Some(horizon_threads::allocator_commit().0 as u64),
                None => memory_limit::resident_bytes(),
            },
        ),
        Request::Set(None) => {
            set(None);
            "RAM limit off until the game closes.".to_owned()
        }
        Request::Set(Some(mb)) => {
            set(Some(mb));
            format!(
                "RAM limit set to {mb} MB until the game closes. The game stays under it, and runs \
                 slower if it's very low."
            )
        }
        Request::SetHard(None) => {
            set_hard(None);
            "Hard memory limit off until the game closes.".to_owned()
        }
        Request::SetHard(Some(mb)) => {
            set_hard(Some(mb));
            format!(
                "Hard memory limit set to {mb} MB until the game closes: past it the game ends \
                 instead of paging."
            )
        }
        Request::Usage => {
            "Usage: /ramlimit, /ramlimit <MB>, /ramlimit off, /ramlimit hard <MB> or /ramlimit \
             hard off"
                .to_owned()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_a_cap_the_system_enforces_only_a_slow_game_gives_anything_up() {
        const OVER: Pressure = Pressure::Over { keep: 0.8 };
        assert_eq!(needed(OVER, true, 200.), Pressure::Steady);
        assert_eq!(needed(OVER, true, 12.), OVER);
        assert_eq!(
            needed(OVER, false, 200.),
            OVER,
            "nothing else holds this game under"
        );
        assert_eq!(needed(Pressure::Room, true, 12.), Pressure::Room);
    }

    const FAR_OVER: Pressure = Pressure::Over { keep: 0.125 };
    const OVER: Pressure = Pressure::Over { keep: 0.8 };

    #[test]
    fn under_pressure_far_terrain_goes_first_then_the_distance_falls_to_the_lowest() {
        let mut g = Governor::default();
        let mut at = Instant::now();
        assert_eq!(g.step(at, FAR_OVER, 32, true), Some(Step::FarTerrainOff));
        assert!(g.far_off());
        assert_eq!(
            g.step(at, FAR_OVER, 32, true),
            None,
            "waits for that to free"
        );
        let mut distances = Vec::new();
        for _ in 0..12 {
            at += STEP_DOWN_EVERY;
            if let Some(Step::Lowered(d)) = g.step(at, FAR_OVER, 32, true) {
                distances.push(d);
            }
        }
        assert!(distances.windows(2).all(|w| w[1] < w[0]), "{distances:?}");
        assert_eq!(distances.last(), Some(&LOWEST_DISTANCE));
        assert!(distances.len() < 6, "gets there fast: {distances:?}");
        assert!(g.at_lowest(32, true));
        assert_eq!(g.distance(32), LOWEST_DISTANCE);
    }

    #[test]
    fn with_room_again_the_distance_and_far_terrain_come_back() {
        let start = Instant::now();
        let mut g = Governor::default();
        g.step(start, OVER, 8, true);
        let mut at = start + STEP_DOWN_EVERY;
        assert_eq!(g.step(at, OVER, 8, true), Some(Step::Lowered(7)));
        // Steady, nothing moves.
        at += STEP_UP_EVERY;
        assert_eq!(g.step(at, Pressure::Steady, 8, true), None);
        assert_eq!(g.step(at, Pressure::Room, 8, true), Some(Step::Raised(8)));
        assert_eq!(g.distance(8), 8);
        assert_eq!(
            g.step(at, Pressure::Room, 8, true),
            None,
            "waits after a change"
        );
        at += STEP_UP_EVERY;
        assert_eq!(
            g.step(at, Pressure::Room, 8, true),
            Some(Step::FarTerrainBack)
        );
        assert!(!g.far_off());
        at += STEP_UP_EVERY;
        assert_eq!(g.step(at, Pressure::Room, 8, true), None);
    }

    #[test]
    fn a_game_under_no_pressure_is_left_alone_and_the_player_can_choose_less() {
        let start = Instant::now();
        let mut g = Governor::default();
        assert_eq!(g.step(start, Pressure::Steady, 32, true), None);
        assert_eq!(g.distance(32), 32);
        assert!(!g.at_lowest(32, true));
        // The player's own distance is already the lowest and there is no far terrain.
        assert!(g.at_lowest(2, false));
        assert_eq!(g.step(start, FAR_OVER, 2, false), None);
    }

    #[test]
    fn lifting_the_limit_gives_everything_back() {
        let mut g = Governor::default();
        assert!(!g.reset());
        g.step(Instant::now(), OVER, 32, true);
        assert!(g.reset());
        assert!(!g.far_off());
    }

    #[test]
    fn the_command_reads_a_limit_with_no_lowest_value() {
        assert_eq!(parse("ramlimit"), Some(Request::Show));
        assert_eq!(parse("ramlimit off"), Some(Request::Set(None)));
        assert_eq!(parse("ramlimit 1"), Some(Request::Set(Some(1))));
        assert_eq!(parse("ramlimit 8192"), Some(Request::Set(Some(8192))));
        assert_eq!(
            parse("ramlimit hard 512"),
            Some(Request::SetHard(Some(512)))
        );
        assert_eq!(parse("ramlimit hard off"), Some(Request::SetHard(None)));
        assert_eq!(parse("ramlimit hard lots"), Some(Request::Usage));
    }

    #[test]
    fn a_bad_limit_asks_for_the_usage_and_other_commands_pass() {
        assert_eq!(parse("ramlimit lots"), Some(Request::Usage));
        assert_eq!(parse("ramlimit -5"), Some(Request::Usage));
        assert_eq!(parse("ramlimit 5 6"), Some(Request::Usage));
        assert_eq!(parse("ramlimits 5"), None);
        assert_eq!(parse("say ramlimit"), None);
    }

    #[test]
    fn the_answer_shows_the_limit_and_what_is_held() {
        assert_eq!(
            describe(Some(2048 * MIB), Some(1024 * MIB), Some(512 * MIB)),
            "RAM limit: 2048 MB. Hard limit: 1024 MB. The process holds 512 MB."
        );
        assert_eq!(
            describe(None, None, None),
            "RAM limit: off. Hard limit: off."
        );
    }
}
