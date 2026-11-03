//! The player's cap on the memory the game holds, and the last resort behind it.
//!
//! On Windows the system enforces it: a hard working set limit never lets the game hold more in
//! RAM, and moves what it used least out instead of failing, so a low cap costs speed. Elsewhere
//! no such limit exists, and the game keeps itself under the cap by holding less
//! (`horizon_client::ram_limit`, which reads [`pressure`]).
//!
//! A watcher thread reads the process's resident memory and, over the cap, gives free pages back.
//! Only when the game has nothing left to give up ([`can_shrink`] has stopped being called) and
//! stays over for [`GRACE`] does it call the game's `exceeded`. A cap the system enforces can't be
//! gone over, so there the sign is a game frozen by it: no [`frame`] for [`STALL`] while it faults
//! hard. No thread runs until a cap is set, and there is no lowest cap: one the game can't live
//! under ends the game.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

const NONE: u64 = u64::MAX;
const EVERY: Duration = Duration::from_secs(1);
/// How long the game may stay over the cap with nothing left to give up: a world's first chunks,
/// or a burst the last step down has not freed yet, pass within it.
pub const GRACE: Duration = Duration::from_secs(20);
/// How long a game may draw nothing while faulting hard under a cap the system enforces.
pub const STALL: Duration = Duration::from_secs(60);
/// Page faults a second that say a game drawing nothing is busy fetching memory back, not just
/// minimized or waiting.
#[cfg_attr(not(windows), allow(dead_code))]
const STALL_FAULTS: f64 = 20_000.;
/// How long one [`can_shrink`] counts for.
const SHRINK_WORD: Duration = Duration::from_secs(5);

static LIMIT: AtomicU64 = AtomicU64::new(NONE);
static WATCHER: Once = Once::new();
static STARTED: OnceLock<Instant> = OnceLock::new();
/// Until when, in milliseconds since [`STARTED`], the game says it can still shrink.
static SHRINKS_UNTIL: AtomicU64 = AtomicU64::new(0);
/// When the last [`frame`] was, in milliseconds since [`STARTED`].
static LAST_FRAME: AtomicU64 = AtomicU64::new(0);

fn since_start() -> u64 {
    STARTED.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The game drew a frame.
pub fn frame() {
    LAST_FRAME.store(since_start(), Ordering::Relaxed);
}

/// The game can still hold less. Said again every second while that is true, so it ends by itself
/// when the world closes.
pub fn can_shrink() {
    let until = since_start() + SHRINK_WORD.as_millis() as u64;
    SHRINKS_UNTIL.store(until, Ordering::Relaxed);
}

/// Sets the cap in bytes, or removes it. `exceeded(used, limit)` runs on the watcher thread when
/// the game can't get under the cap; only the first call's function is kept.
pub fn set(limit: Option<u64>, exceeded: fn(u64, u64)) {
    LIMIT.store(limit.unwrap_or(NONE), Ordering::Relaxed);
    // A new cap gets the whole of STALL.
    frame();
    system_cap(limit);
    if limit.is_some() {
        WATCHER.call_once(|| {
            let watcher = std::thread::Builder::new()
                .name("Memory limit".into())
                .spawn(move || {
                    let mut over = Over::default();
                    let mut faults = Faults::default();
                    loop {
                        std::thread::sleep(EVERY);
                        let per_second = faults.per_second();
                        let limit = get();
                        over.follow(limit);
                        if let Some(limit) = limit {
                            check(&mut over, per_second, limit, exceeded);
                        }
                    }
                });
            if let Err(e) = watcher {
                eprintln!("The memory limit can't be watched: {e}");
            }
        });
    }
}

/// The cap in bytes, if one is set.
pub fn get() -> Option<u64> {
    Some(LIMIT.load(Ordering::Relaxed)).filter(|limit| *limit != NONE)
}

/// How tight memory is for the game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pressure {
    /// It should hold less; `keep` is the share of what it holds to aim for.
    Over {
        keep: f64,
    },
    /// It could hold more.
    Room,
    Steady,
}

/// Against a cap nothing enforces: what the game holds. It should act before it gets there.
fn pressure_of_usage(used: u64, limit: u64) -> Pressure {
    if used > limit / 10 * 9 {
        Pressure::Over {
            keep: limit as f64 * 0.8 / used as f64,
        }
    } else if used < limit / 10 * 7 {
        Pressure::Room
    } else {
        Pressure::Steady
    }
}

/// Under a cap the system enforces the game sits at the cap whether it needs the memory or not, so
/// usage says nothing: how often it has to fetch back what was moved out does. Measured on a real
/// game loading chunks at 30 frames a second: 30,000-50,000 page faults a second with no cap or any
/// cap down to 500 MB, 140,000 at 300 MB, 290,000 at 150 MB. Loading chunks fast faults a lot too,
/// so `horizon_client::ram_limit` also asks whether frames are suffering.
#[cfg_attr(not(windows), allow(dead_code))]
fn pressure_of_faults(per_second: f64) -> Pressure {
    if per_second > 200_000. {
        Pressure::Over { keep: 0.8 }
    } else if per_second < 75_000. {
        Pressure::Room
    } else {
        Pressure::Steady
    }
}

/// Against a hard ceiling, what the process commits is the budget: the working set sits at its
/// cap whatever the game needs, and page faults only say how hard it pages, not how much it
/// holds. A ceiling counts every page the process committed, the renderer's own included.
pub fn pressure_of_commit(commit: u64, limit: u64) -> Pressure {
    if commit > limit / 10 * 9 {
        Pressure::Over {
            keep: limit as f64 * 0.8 / commit as f64,
        }
    } else if commit < limit / 10 * 7 {
        Pressure::Room
    } else {
        Pressure::Steady
    }
}

/// How tight memory is under `limit`. Reads a difference since the last call: call it once a
/// second, from one place.
pub fn pressure(limit: u64) -> Pressure {
    #[cfg(windows)]
    if SYSTEM_CAP.load(Ordering::Relaxed) {
        // The first reading has nothing to compare with.
        static FAULTS: std::sync::Mutex<Faults> = std::sync::Mutex::new(Faults::new());
        let per_second = FAULTS.lock().ok().and_then(|mut f| f.per_second());
        return per_second.map_or(Pressure::Steady, pressure_of_faults);
    }
    resident_bytes().map_or(Pressure::Steady, |used| pressure_of_usage(used, limit))
}

/// Whether the system itself holds the game under the cap.
pub fn system_enforced() -> bool {
    #[cfg(windows)]
    return SYSTEM_CAP.load(Ordering::Relaxed);
    #[cfg(not(windows))]
    false
}

/// How long the game has been over the cap with nothing left to give up.
#[derive(Default)]
struct Over {
    since: Option<Instant>,
    /// The cap the grace is counted against.
    limit: Option<u64>,
}

impl Over {
    /// A cap turned off, or set to another value, starts its grace again.
    fn follow(&mut self, limit: Option<u64>) {
        if limit != self.limit {
            self.since = None;
            self.limit = limit;
        }
    }
    /// Whether this reading ends the game.
    fn gave_up(&mut self, now: Instant, over: bool, can_shrink: bool) -> bool {
        if !over || can_shrink {
            self.since = None;
            return false;
        }
        now.duration_since(*self.since.get_or_insert(now)) >= GRACE
    }
}

/// Whether a game has frozen under a cap the system enforces.
fn frozen(no_frame_for: Duration, faults_per_second: f64) -> bool {
    no_frame_for >= STALL && faults_per_second >= STALL_FAULTS
}

fn check(state: &mut Over, faults: Option<f64>, limit: u64, exceeded: fn(u64, u64)) {
    // A hard ceiling counts what the process committed, the renderer's own
    // pages included, since that is the number the player set.
    let hard = crate::capped::limit().is_some();
    let measured = if hard {
        Some(crate::allocator_commit().0 as u64)
    } else {
        resident_bytes()
    };
    let Some(mut used) = measured else {
        return;
    };
    // A cap the system enforces can't be gone over, so there the sign is a game
    // frozen by it. A hard ceiling can be gone over, and says so.
    if system_enforced() && !hard {
        let no_frame_for = since_start().saturating_sub(LAST_FRAME.load(Ordering::Relaxed));
        if frozen(Duration::from_millis(no_frame_for), faults.unwrap_or(0.)) {
            // The report has to be written, which a frozen game can't do under the cap.
            system_cap(None);
            exceeded(used, limit);
        }
        return;
    }
    if used > limit {
        crate::release_free_memory();
        used = if hard {
            crate::allocator_commit().0 as u64
        } else {
            resident_bytes().unwrap_or(used)
        };
    }
    let can_shrink = since_start() < SHRINKS_UNTIL.load(Ordering::Relaxed);
    if state.gave_up(Instant::now(), used > limit, can_shrink) {
        exceeded(used, limit);
    }
}

/// Whether the system is enforcing the cap.
#[cfg(windows)]
static SYSTEM_CAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Sets the system's hard limit on the working set, or lifts it. Refused (a cap below what the
/// system allows), the watcher treats the game as any other that can't get under its cap.
#[cfg(windows)]
fn system_cap(limit: Option<u64>) {
    use windows_sys::Win32::System::Memory::{
        QUOTA_LIMITS_HARDWS_MAX_DISABLE, QUOTA_LIMITS_HARDWS_MAX_ENABLE,
        QUOTA_LIMITS_HARDWS_MIN_DISABLE, SetProcessWorkingSetSizeEx,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    const MINIMUM: u64 = 16 << 20;
    // Windows' own defaults for a process, with the limit lifted.
    let (low, high, flags) = match limit {
        Some(limit) => (
            MINIMUM.min(limit / 2) as usize,
            usize::try_from(limit).unwrap_or(usize::MAX),
            QUOTA_LIMITS_HARDWS_MAX_ENABLE | QUOTA_LIMITS_HARDWS_MIN_DISABLE,
        ),
        None => (
            200 << 10,
            1380 << 10,
            QUOTA_LIMITS_HARDWS_MAX_DISABLE | QUOTA_LIMITS_HARDWS_MIN_DISABLE,
        ),
    };
    // SAFETY: passes plain values; the current process's pseudo-handle needs no closing.
    let done = unsafe { SetProcessWorkingSetSizeEx(GetCurrentProcess(), low, high, flags) != 0 };
    if !done {
        eprintln!(
            "Windows refused the memory limit: {}",
            std::io::Error::last_os_error()
        );
    }
    SYSTEM_CAP.store(done && limit.is_some(), Ordering::Relaxed);
}

#[cfg(not(windows))]
fn system_cap(_limit: Option<u64>) {}

/// The process's memory counters.
#[cfg(windows)]
fn counters() -> Option<windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: all-zero is a valid value of this plain C struct, and the call writes at most `size`
    // bytes into it; the current process's pseudo-handle needs no closing.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = size;
        (K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) != 0).then_some(counters)
    }
}

/// Page fault rates, between one reading and the next.
#[derive(Default)]
struct Faults {
    #[cfg(windows)]
    last: Option<(Instant, u32)>,
}

impl Faults {
    #[cfg(windows)]
    const fn new() -> Self {
        Faults { last: None }
    }

    /// Page faults a second since the last call; none on the first, or where they aren't known.
    #[cfg(windows)]
    fn per_second(&mut self) -> Option<f64> {
        let faults = counters()?.PageFaultCount;
        let now = Instant::now();
        let before = self.last.replace((now, faults))?;
        let seconds = now.duration_since(before.0).as_secs_f64();
        (seconds > 0.).then(|| f64::from(faults.wrapping_sub(before.1)) / seconds)
    }

    #[cfg(not(windows))]
    fn per_second(&mut self) -> Option<f64> {
        None
    }
}

/// The memory this process holds in RAM, in bytes.
#[cfg(windows)]
pub fn resident_bytes() -> Option<u64> {
    Some(counters()?.WorkingSetSize as u64)
}

/// The memory this process holds in RAM, in bytes.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: reads one system constant.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * u64::try_from(page).ok()?)
}

/// The memory this process holds in RAM, in bytes.
#[cfg(target_vendor = "apple")]
pub fn resident_bytes() -> Option<u64> {
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: all-zero is a valid value of this plain C struct, and the call writes at most `size`
    // bytes into it.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let written = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        );
        (written == size).then_some(info.pti_resident_size)
    }
}

/// The memory this process holds in RAM, in bytes: not known on this system.
#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple"
)))]
pub fn resident_bytes() -> Option<u64> {
    None
}

#[cfg(all(test, any(windows, target_os = "linux", target_vendor = "apple")))]
mod tests {
    use super::*;

    /// The working set is the whole test process's, and one test caps it.
    static WORKING_SET: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn touched_memory_counts_as_resident() {
        let _alone = WORKING_SET.lock().unwrap();
        const SIZE: usize = 256 << 20;
        let before = resident_bytes().unwrap();
        let block = vec![1u8; SIZE];
        let during = resident_bytes().unwrap();
        std::hint::black_box(&block);
        assert!(
            during >= before + SIZE as u64 / 2,
            "{before} then {during} bytes"
        );
    }

    #[test]
    fn usage_says_how_tight_a_cap_nothing_enforces_is() {
        const GB: u64 = 1 << 30;
        assert_eq!(pressure_of_usage(GB / 2, GB), Pressure::Room);
        assert_eq!(pressure_of_usage(GB / 10 * 8, GB), Pressure::Steady);
        let Pressure::Over { keep } = pressure_of_usage(2 * GB, GB) else {
            panic!("twice the cap is over it");
        };
        assert!((keep - 0.4).abs() < 0.001, "{keep}");
    }

    #[test]
    fn page_faults_say_how_tight_a_cap_the_system_enforces_is() {
        assert_eq!(pressure_of_faults(50_000.), Pressure::Room);
        assert_eq!(pressure_of_faults(140_000.), Pressure::Steady);
        assert!(matches!(
            pressure_of_faults(290_000.),
            Pressure::Over { .. }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn the_system_holds_the_game_under_a_cap_and_lifting_it_lets_memory_back() {
        let _alone = WORKING_SET.lock().unwrap();
        const SIZE: usize = 256 << 20;
        const CAP: u64 = 128 << 20;
        let mut block = vec![1u8; SIZE];
        system_cap(Some(CAP));
        block.iter_mut().step_by(4096).for_each(|b| *b = 2);
        let capped = resident_bytes().unwrap();
        assert!(capped <= CAP, "{capped} bytes under a cap of {CAP}");
        system_cap(None);
        block.iter_mut().step_by(4096).for_each(|b| *b = 3);
        let lifted = resident_bytes().unwrap();
        std::hint::black_box(&block);
        assert!(lifted > CAP, "{lifted} bytes with the cap lifted");
    }

    #[test]
    fn only_drawing_nothing_while_faulting_hard_counts_as_frozen() {
        assert!(frozen(STALL, STALL_FAULTS * 10.));
        // Slow but drawing, as at 1 frame a second.
        assert!(!frozen(Duration::from_secs(1), STALL_FAULTS * 10.));
        // Minimized or waiting: no frames, and no memory being fetched back.
        assert!(!frozen(STALL * 10, 100.));
    }

    #[test]
    fn only_staying_over_with_nothing_left_to_give_up_ends_the_game() {
        let start = Instant::now();
        let later = |d: Duration| start + d;
        let mut over = Over::default();
        assert!(!over.gave_up(start, true, false));
        assert!(!over.gave_up(later(GRACE / 2), true, false));
        assert!(over.gave_up(later(GRACE), true, false));

        // Getting back under the cap starts the count again.
        let mut over = Over::default();
        assert!(!over.gave_up(start, true, false));
        assert!(!over.gave_up(later(GRACE / 2), false, false));
        assert!(!over.gave_up(later(GRACE), true, false));

        // So does having something left to give up, however long the game is over.
        let mut over = Over::default();
        assert!(!over.gave_up(start, true, false));
        assert!(!over.gave_up(later(GRACE * 5), true, true));
        assert!(!over.gave_up(later(GRACE * 6), true, false));

        // A cap turned off, then on again, or changed, gets its whole grace again.
        let mut over = Over::default();
        over.follow(Some(1 << 30));
        assert!(!over.gave_up(start, true, false));
        over.follow(None);
        over.follow(Some(1 << 30));
        assert!(!over.gave_up(later(GRACE), true, false));
        over.follow(Some(1 << 29));
        assert!(!over.gave_up(later(GRACE * 2), true, false));
        over.follow(Some(1 << 29));
        assert!(over.gave_up(later(GRACE * 3), true, false));
    }

    #[test]
    fn saying_the_game_can_shrink_holds_the_last_resort_off_for_a_while() {
        can_shrink();
        assert!(since_start() < SHRINKS_UNTIL.load(Ordering::Relaxed));
    }

    #[test]
    fn a_commit_budget_says_over_near_it_and_room_far_below() {
        assert_eq!(
            pressure_of_commit(950, 1000),
            Pressure::Over {
                keep: 0.8 * 1000. / 950.
            }
        );
        assert_eq!(pressure_of_commit(800, 1000), Pressure::Steady);
        assert_eq!(pressure_of_commit(600, 1000), Pressure::Room);
    }

    #[test]
    fn a_commit_budget_aims_for_less_the_further_over_it_is() {
        let a = pressure_of_commit(1000, 1000);
        let b = pressure_of_commit(2000, 1000);
        let (Pressure::Over { keep: a }, Pressure::Over { keep: b }) = (a, b) else {
            panic!("both are over");
        };
        assert!(a > b, "twice the budget asks for half as much");
        assert!((b - 0.4).abs() < 1e-9);
    }

    #[test]
    fn a_budget_of_zero_never_says_room() {
        assert!(matches!(pressure_of_commit(1, 0), Pressure::Over { .. }));
    }
}
