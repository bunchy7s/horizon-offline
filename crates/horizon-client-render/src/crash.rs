//! Crash reports, laid out like vanilla's CrashReport.getFriendlyReport: a comment, the time,
//! a description, the error with its stack, then the details of the game and the machine.
//! Written to `crash-reports/crash-<yyyy-MM-dd_HH.mm.ss>-client.txt` in the game folder when
//! the main thread panics or `main` gets an error back. It leaves the machine only scrubbed, and
//! only if the player allows crash reports (`telemetry`).
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Local};

/// What the game is doing, set as it goes: graphics adapter, settings, packs, world.
static DETAILS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());
static REPORTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// How many `contain` calls this thread is inside.
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
    /// The message of the last panic `contain` caught on this thread, from the hook.
    static CAUGHT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Runs `f`, and returns a panic in it as an error with the panic's message instead of letting
/// it end the game. For work that is not the game's own and whose failure the caller recovers
/// from, such as handing a shader pack's programs to the GPU driver's compiler: the panic goes
/// to the log with its stack, and no crash report is made for it.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    CONTAINED.with(|c| c.set(c.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CONTAINED.with(|c| c.set(c.get() - 1));
    result.map_err(|payload| {
        CAUGHT
            .with(|c| c.borrow_mut().take())
            .unwrap_or_else(|| payload_text(&*payload))
    })
}

/// For the hook: a panic inside `contain` is logged with its stack and kept for `contain` to
/// return. False for any other panic.
fn contained_panic(info: &std::panic::PanicHookInfo) -> bool {
    if CONTAINED.with(Cell::get) == 0 {
        return false;
    }
    let message = panic_message(info);
    let stack = stack_lines(
        &std::backtrace::Backtrace::force_capture().to_string(),
        true,
    );
    log::error!(
        "Recovered from an error: {message}\n{}",
        stack
            .lines()
            .map(|l| format!("\tat {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    CAUGHT.with(|c| *c.borrow_mut() = Some(message));
    true
}

/// Sets one line of the report's details (replacing the last value under `key`).
pub fn set_detail(key: &'static str, value: impl Into<String>) {
    let mut details = DETAILS.lock().unwrap_or_else(|e| e.into_inner());
    let value = value.into();
    match details.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => details.push((key, value)),
    }
}

pub fn clear_detail(key: &'static str) {
    DETAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| *k != key);
}

fn details() -> Vec<(&'static str, String)> {
    DETAILS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Vanilla picks a line from a list of jokes; these only say what happened.
const COMMENTS: &[&str] = &[
    "Something went wrong.",
    "The game stopped here.",
    "This should not have happened.",
    "Details below.",
];

/// One crash, ready to be written.
pub struct Report {
    pub time: DateTime<Local>,
    pub description: String,
    /// The error's message, first line of the stack section.
    pub error: String,
    /// Where it happened and how it got there: one frame a line (`function (file:line:col)`).
    pub stack: String,
    pub thread: String,
    /// `client` or `server`, the end of the file name.
    pub side: &'static str,
    pub details: Vec<(String, String)>,
    pub log: Vec<String>,
}

/// `crash-2026-10-14_09.05.07-client.txt`
pub fn file_name(time: &DateTime<Local>, side: &str) -> String {
    format!("crash-{}-{side}.txt", time.format("%Y-%m-%d_%H.%M.%S"))
}

impl Report {
    /// The report's text.
    pub fn text(&self) -> String {
        let comment = COMMENTS[self.time.timestamp().rem_euclid(COMMENTS.len() as i64) as usize];
        let mut out = String::new();
        let _ = writeln!(out, "---- Horizon Crash Report ----");
        let _ = writeln!(out, "// {comment}\n");
        let _ = writeln!(out, "Time: {}", self.time.format("%Y-%m-%d %H:%M:%S"));
        let _ = writeln!(out, "Description: {}\n", self.description);
        let _ = writeln!(out, "{}", self.error);
        for line in self.stack.lines() {
            let _ = writeln!(out, "\tat {line}");
        }
        let _ = writeln!(
            out,
            "\n\nA detailed walkthrough of the error, its code path and all known details is as follows:"
        );
        let _ = writeln!(out, "{}\n", "-".repeat(87));
        let _ = writeln!(out, "-- Head --");
        let _ = writeln!(out, "Thread: {}\n", self.thread);
        let _ = writeln!(out, "-- System Details --");
        let _ = writeln!(out, "Details:");
        for (key, value) in &self.details {
            let _ = writeln!(out, "\t{key}: {value}");
        }
        let _ = writeln!(out, "\n-- Last {} log lines --", self.log.len());
        for line in &self.log {
            let _ = writeln!(out, "{line}");
        }
        out
    }

    /// Writes the report in `home/crash-reports` and returns its path.
    pub fn save(&self, home: &Path) -> std::io::Result<PathBuf> {
        let dir = crate::logging::crash_reports_dir(home);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(&self.time, self.side));
        std::fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// The release this binary is (`v0.2.3`); a nightly's label (`Nightly 2026-10-28 · 0c02067`); for a
/// release from before 0.2, its tag and the client's crate version; for a build from source, that
/// version alone.
pub fn version() -> String {
    if let Some(label) = horizon_core::release::nightly_label() {
        return label;
    }
    let version = env!("CARGO_PKG_VERSION");
    match (horizon_core::release::RELEASE, release_tag()) {
        (Some(_), Some(tag)) => tag,
        (None, Some(tag)) => format!("{tag} (client {version})"),
        (_, None) => version.to_owned(),
    }
}

/// The release this binary came with: the version release.yml built in (docs/versioning.md), else
/// the folder the launcher installed it in, named after the tag (releases from before 0.2). A
/// nightly is its version as the launcher installs it (`nightly-2026-10-28-0c02067`).
pub fn release_tag() -> Option<String> {
    if let Some(nightly) = horizon_core::release::nightly_version() {
        return Some(nightly);
    }
    if let Some(version) = horizon_core::release::RELEASE {
        return Some(format!("v{version}"));
    }
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.file_name()?.to_str()?;
    (folder.starts_with('v') && folder[1..].starts_with(|c: char| c.is_ascii_digit()))
        .then(|| folder.to_owned())
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// `PRETTY_NAME` of /etc/os-release and the kernel, or the system's own version string.
fn operating_system() -> String {
    let arch = std::env::consts::ARCH;
    let version = if cfg!(target_os = "linux") {
        let name = read("/etc/os-release").and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        });
        let kernel = read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_owned());
        match (name, kernel) {
            (Some(n), Some(k)) => Some(format!("{n}, kernel {k}")),
            (n, k) => n.or(k),
        }
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        command_output("cmd", &["/C", "ver"])
    } else {
        None
    };
    format!(
        "{} ({arch}) version {}",
        std::env::consts::OS,
        version.unwrap_or_else(|| "unknown".into())
    )
}

/// Logical processors and the CPU's name (`12x AMD Ryzen 5 5600X ...`).
pub fn cpu() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let name = if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_owned())
        })
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    format!(
        "{threads}x {}",
        name.unwrap_or_else(|| std::env::consts::ARCH.into())
    )
}

/// Total memory and what this process uses, in MiB where the system says.
fn memory() -> String {
    const MIB: u64 = 1024 * 1024;
    let kib = |text: &str, key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let (total, used) = if cfg!(target_os = "linux") {
        (
            read("/proc/meminfo")
                .and_then(|t| kib(&t, "MemTotal:"))
                .map(|k| k * 1024),
            read("/proc/self/status")
                .and_then(|t| kib(&t, "VmRSS:"))
                .map(|k| k * 1024),
        )
    } else if cfg!(target_os = "macos") {
        (
            command_output("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok()),
            None,
        )
    } else if cfg!(windows) {
        (
            command_output(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ],
            )
            .and_then(|v| v.parse().ok()),
            None,
        )
    } else {
        (None, None)
    };
    match (total, used) {
        (Some(t), Some(u)) => format!(
            "{} MiB used by the game, {} MiB installed",
            u / MIB,
            t / MIB
        ),
        (Some(t), None) => format!("{} MiB installed", t / MIB),
        _ => "unknown".into(),
    }
}

/// The report's details: Horizon's and the machine's, then what the game set as it ran.
pub fn system_details() -> Vec<(String, String)> {
    let mut out = vec![
        ("Horizon Version".to_owned(), version()),
        (
            "Minecraft Version".to_owned(),
            horizon_core::VANILLA_VERSION.to_owned(),
        ),
        ("Operating System".to_owned(), operating_system()),
        ("CPU".to_owned(), cpu()),
        ("Memory".to_owned(), memory()),
    ];
    out.extend(details().into_iter().map(|(k, v)| (k.to_owned(), v)));
    out
}

fn thread_name() -> String {
    let current = std::thread::current();
    match current.name() {
        Some("main") | None => "Render thread".into(),
        Some(name) => name.into(),
    }
}

/// Writes the report and tells where it is; the log gets the error too.
fn deliver(home: &Path, report: Report) -> Option<PathBuf> {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    // The panic is on stderr already: the stack goes to the log only.
    crate::logging::write_only(
        log::Level::Error,
        &format!(
            "Horizon crashed: {}\n{}",
            report.error,
            report
                .stack
                .lines()
                .map(|l| format!("\tat {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    crate::logging::flush();
    crate::telemetry::keep_crash(home, &report);
    match report.save(home) {
        Ok(path) => {
            eprintln!("Horizon: crash report saved to {}", path.display());
            Some(path)
        }
        Err(e) => {
            eprintln!("Horizon: could not save a crash report: {e}");
            eprintln!("{}", report.text());
            None
        }
    }
}

/// The report for a hung server tick, written by the watchdog thread
/// (`ServerWatchdog`): the stuck thread's stack is not available to Rust, so
/// the report carries none.
pub fn report_watchdog(home: &Path, side: &'static str, message: String) -> Option<PathBuf> {
    deliver(
        home,
        Report {
            time: Local::now(),
            description: "Watching Server".into(),
            error: format!("{message} (Watching Server)"),
            stack: String::new(),
            thread: thread_name(),
            side,
            details: system_details(),
            log: crate::logging::recent_lines(),
        },
    )
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into())
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    let text = payload_text(info.payload());
    match info.location() {
        Some(at) => format!("panicked at {at}:\n{text}"),
        None => format!("panicked:\n{text}"),
    }
}

/// A `std::backtrace` as one line a frame, `function (file:line:col)`. Captured in a panic hook
/// (`from_panic`), it starts at the frame that panicked: the hook's and the panic machinery's
/// frames go. Paths inside the workspace lose the build machine's prefix.
pub fn stack_lines(backtrace: &str, from_panic: bool) -> String {
    // The workspace as it was on the machine that built this binary (it need not exist here).
    // A report names workspace files the same way on every host, whatever
    // separator the build machine used.
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let root = manifest
        .strip_suffix("crates/horizon-client-render")
        .unwrap_or(&manifest);
    let mut frames: Vec<(String, Option<String>)> = Vec::new();
    for line in backtrace.lines() {
        let line = line.trim();
        if let Some(at) = line.strip_prefix("at ") {
            if let Some(frame) = frames.last_mut() {
                frame.1 = Some(short_path(&at.replace('\\', "/"), root).to_owned());
            }
        } else if let Some((n, name)) = line.split_once(": ")
            && n.chars().all(|c| c.is_ascii_digit())
        {
            frames.push((name.to_owned(), None));
        }
    }
    let machinery = |name: &str| {
        [
            "std::panicking::",
            "core::panicking::",
            "std::sys::backtrace::",
            "__rustc::",
        ]
        .iter()
        .any(|p| name.starts_with(p))
            || name.contains("rust_begin_unwind")
    };
    // The hook's frames, then the panic machinery's, then the code that panicked.
    let top = frames.iter().take(30).position(|(name, _)| machinery(name));
    let first = match top.filter(|_| from_panic) {
        Some(start) => {
            start
                + frames[start..]
                    .iter()
                    .take_while(|(name, _)| machinery(name))
                    .count()
        }
        None => 0,
    };
    // Some platforms keep std's own capture frames (`<std::backtrace::Backtrace>::create`).
    let first = first
        + frames[first..]
            .iter()
            .take_while(|(name, _)| name.trim_start_matches('<').starts_with("std::backtrace"))
            .count();
    frames[first..]
        .iter()
        .map(|(name, at)| match at {
            Some(at) => format!("{name} ({at})\n"),
            None => format!("{name}\n"),
        })
        .collect()
}

/// A source path without the build machine's folders: relative to the workspace, or from the
/// crate's folder for dependencies (`winit-0.30.13/src/event_loop.rs:642:56`).
fn short_path<'a>(path: &'a str, root: &str) -> &'a str {
    if let Some(rest) = path.strip_prefix(root) {
        return rest;
    }
    if let Some(i) = path.find("/registry/src/") {
        let rest = &path[i + "/registry/src/".len()..];
        if let Some((_index, krate)) = rest.split_once('/') {
            return krate;
        }
    }
    path
}

/// A report for a panic, with the stack captured now (RUST_BACKTRACE or not).
pub fn panic_report(info: &std::panic::PanicHookInfo, side: &'static str) -> Report {
    Report {
        time: Local::now(),
        description: "Unexpected error".into(),
        error: panic_message(info),
        stack: stack_lines(
            &std::backtrace::Backtrace::force_capture().to_string(),
            true,
        ),
        thread: thread_name(),
        side,
        details: system_details(),
        log: crate::logging::recent_lines(),
    }
}

/// Reports panics: on the main thread a crash report (the panic then ends the game, exit code
/// 101); on other threads the error goes to the log, as vanilla's uncaught exception handler
/// does. The default hook still prints the panic. A panic inside `contain` is only logged.
pub fn install(home: PathBuf, side: &'static str) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if contained_panic(info) {
            return;
        }
        default(info);
        let main = std::thread::current().name() == Some("main");
        if main {
            deliver(&home, panic_report(info, side));
        } else {
            log::error!(
                "Caught previously unhandled exception in thread {}: {}",
                thread_name(),
                panic_message(info)
            );
        }
    }));
}

/// Vanilla's debug crash (`key.debug.crash` held for ten seconds): the error `main` gets back,
/// with the stack where it was raised.
#[derive(Debug)]
pub struct DebugCrash {
    stack: String,
}

impl DebugCrash {
    pub const MESSAGE: &str = "Manually triggered debug crash";
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            stack: stack_lines(
                &std::backtrace::Backtrace::force_capture().to_string(),
                false,
            ),
        }
    }
}

impl std::fmt::Display for DebugCrash {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(Self::MESSAGE)
    }
}

impl std::error::Error for DebugCrash {}

/// A report for an error `main` got back: the game ends with it.
pub fn report_error(
    home: &Path,
    side: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Option<PathBuf> {
    if let Some(debug) = error.downcast_ref::<DebugCrash>() {
        return deliver(
            home,
            Report {
                time: Local::now(),
                description: DebugCrash::MESSAGE.into(),
                error: DebugCrash::MESSAGE.into(),
                stack: debug.stack.clone(),
                thread: thread_name(),
                side,
                details: system_details(),
                log: crate::logging::recent_lines(),
            },
        );
    }
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let _ = write!(message, "\nCaused by: {e}");
        source = e.source();
    }
    deliver(
        home,
        Report {
            time: Local::now(),
            description: "Fatal error".into(),
            error: message,
            stack: String::new(),
            thread: thread_name(),
            side,
            details: system_details(),
            log: crate::logging::recent_lines(),
        },
    )
}

/// Called every frame: panics once `HORIZON_DEBUG_CRASH` asks (development builds only).
pub fn debug_crash_tick() {
    static AT: std::sync::OnceLock<Option<(std::time::Instant, std::time::Duration)>> =
        std::sync::OnceLock::new();
    if let Some((first, after)) =
        AT.get_or_init(|| debug_crash_after().map(|d| (std::time::Instant::now(), d)))
        && first.elapsed() >= *after
    {
        panic!("Manually triggered debug crash");
    }
}

/// Development builds crash on purpose when `HORIZON_DEBUG_CRASH` is set (vanilla has F3+C held
/// for ten seconds): the number is seconds after the first frame (default 5).
pub fn debug_crash_after() -> Option<std::time::Duration> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("HORIZON_DEBUG_CRASH").ok()?;
    let seconds = value.parse::<f64>().unwrap_or(5.0).max(0.0);
    Some(std::time::Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_reports_its_built_in_version() {
        // release.yml builds with HORIZON_VERSION (docs/versioning.md): `v0.2.3`, wherever it is installed.
        if let Some(label) = horizon_core::release::nightly_label() {
            assert_eq!(super::version(), label);
            assert_eq!(release_tag(), horizon_core::release::nightly_version());
            return;
        }
        match horizon_core::release::RELEASE {
            Some(version) => {
                assert_eq!(release_tag(), Some(format!("v{version}")));
                assert_eq!(super::version(), format!("v{version}"));
            }
            None => assert!(release_tag().is_none() || super::version().contains("(client ")),
        }
    }

    fn report() -> Report {
        Report {
            time: chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap(),
            description: "Unexpected error".into(),
            error: "panicked at src/game.rs:10:5:\nboom".into(),
            stack: "horizon_client::game::frame (src/game.rs:10:5)\n".into(),
            thread: "Render thread".into(),
            side: "client",
            details: vec![
                ("Horizon Version".into(), "0.1.0".into()),
                ("Minecraft Version".into(), "26.3".into()),
                ("Graphics".into(), "Test GPU (Vulkan), driver 1.0".into()),
            ],
            log: vec!["[09:05:06] [Render thread/INFO]: load assets: 1.20s".into()],
        }
    }

    #[test]
    fn file_names_are_vanillas() {
        assert_eq!(
            file_name(&report().time, "client"),
            "crash-2026-10-14_09.05.07-client.txt"
        );
    }

    #[test]
    fn the_report_reads_like_vanillas() {
        let text = report().text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "---- Horizon Crash Report ----");
        assert!(lines[1].starts_with("// "));
        assert_eq!(lines[3], "Time: 2026-10-14 09:05:07");
        assert_eq!(lines[4], "Description: Unexpected error");
        assert_eq!(lines[6], "panicked at src/game.rs:10:5:");
        assert_eq!(lines[7], "boom");
        assert_eq!(
            lines[8],
            "\tat horizon_client::game::frame (src/game.rs:10:5)"
        );
        assert!(text.contains("-- Head --\nThread: Render thread\n"));
        assert!(text.contains(
            "-- System Details --\nDetails:\n\tHorizon Version: 0.1.0\n\tMinecraft Version: 26.3\n\tGraphics: Test GPU (Vulkan), driver 1.0\n"
        ));
        assert!(text.ends_with(
            "-- Last 1 log lines --\n[09:05:06] [Render thread/INFO]: load assets: 1.20s\n"
        ));
    }

    #[test]
    fn stacks_start_at_the_panicking_frame() {
        let bt = format!(
            "   0: horizon_client::crash::panic_report\n             at {root}/src/crash.rs:296:16\n   1: std::panicking::panic_with_hook\n             at /rustc/abc/library/std/src/panicking.rs:823:13\n   2: core::panicking::panic_fmt\n             at /rustc/abc/library/core/src/panicking.rs:80:14\n   3: horizon_client::crash::debug_crash_tick\n             at {root}/src/crash.rs:359:9\n   4: winit::dispatch\n             at /home/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/event_loop.rs:642:56\n   5: main\n",
            root = env!("CARGO_MANIFEST_DIR")
        );
        assert_eq!(
            stack_lines(&bt, true),
            "horizon_client::crash::debug_crash_tick (crates/horizon-client-render/src/crash.rs:359:9)\nwinit::dispatch (winit-0.30.13/src/event_loop.rs:642:56)\nmain\n"
        );
    }

    #[test]
    fn contained_panics_come_back_as_errors() {
        assert_eq!(contain(|| 7), Ok(7));
        let error = contain(|| -> u8 { panic!("a pack's program") }).unwrap_err();
        assert!(error.contains("a pack's program"), "{error}");
        // Nested, and the thread is no longer inside one after.
        let outer = contain(|| contain(|| -> u8 { panic!("inner") }));
        assert!(outer.unwrap().unwrap_err().contains("inner"));
        assert_eq!(CONTAINED.with(Cell::get), 0);
    }

    #[test]
    fn the_debug_crash_is_reported_as_vanillas() {
        let error: Box<dyn std::error::Error> = DebugCrash::new().into();
        assert_eq!(error.to_string(), "Manually triggered debug crash");
        let debug = error.downcast_ref::<DebugCrash>().unwrap();
        // The stack starts where the crash was raised (DebugCrash::new, in this file).
        let first = debug.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", debug.stack);
    }

    #[test]
    fn a_panic_becomes_a_report() {
        // A real panic through a hook, as the game's would be.
        let caught = std::sync::Arc::new(Mutex::new(None));
        let sink = caught.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_message(info).contains("the test crash") {
                *sink.lock().unwrap() = Some(panic_report(info, "client"));
            }
        }));
        let _ = std::panic::catch_unwind(|| panic!("the test crash"));
        std::panic::set_hook(previous);
        let report = caught.lock().unwrap().take().expect("the hook ran");
        assert!(report.error.contains("the test crash"));
        assert!(report.error.contains("crash.rs"));
        // The first frame is the test's own code, not the panic machinery.
        let first = report.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", report.stack);
        let text = report.text();
        assert!(text.contains("Description: Unexpected error"));
        assert!(text.contains(&format!(
            "\tMinecraft Version: {}\n",
            horizon_core::VANILLA_VERSION
        )));
        assert!(text.contains("\tOperating System: "));
        assert!(text.contains("\tCPU: "));
        let home = std::env::temp_dir().join(format!("horizon-crash-save-{}", std::process::id()));
        let path = report.save(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-client.txt")
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
