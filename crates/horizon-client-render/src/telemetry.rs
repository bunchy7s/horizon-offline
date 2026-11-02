//! The game's part in opt-in data collection (`horizon_telemetry`, docs/telemetry.md). When the
//! player allows it, the game keeps a scrubbed crash report, or a play session's performance
//! summary, in the outbox. It never sends those itself: the launcher does, at its next start (the
//! game's only sending is the anonymous "I'm playing" ping, `heartbeat.rs`).
//! Without the player's yes for that kind nothing is kept, and crash-reports/ stays local.
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use horizon_telemetry::payload::{Crash, Display, Perf, Settings, System, Timings};
use horizon_telemetry::scrub::Scrubber;
use horizon_telemetry::{Item, Payload, Source, outbox};

/// A session shorter than this says too little to keep.
const MIN_SECONDS: f32 = 60.0;

/// The details of a crash report that say what the game was doing, and only those: not the
/// world, the server or the resource packs.
const CRASH_DETAILS: [&str; 3] = ["Horizon Version", "Graphics Adapter", "Render Settings"];

static STARTED: OnceLock<Instant> = OnceLock::new();
static CORE_READY_MS: AtomicU32 = AtomicU32::new(u32::MAX);
static REST_READY_MS: AtomicU32 = AtomicU32::new(u32::MAX);
/// The GPU, once the renderer has its adapter: name, maker, kind, driver, backend, its own memory.
static GPU: Mutex<Option<Gpu>> = Mutex::new(None);
/// The screen the window opened on.
static DISPLAY: Mutex<Option<Display>> = Mutex::new(None);

/// The adapter as the renderer found it.
#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    /// `integrated`, `discrete`, `virtual`, `cpu` or `other`.
    pub kind: String,
    pub driver: String,
    pub backend: String,
}

/// When the game started, for the time to the title screen. Call first thing in `main`.
pub fn mark_start() {
    STARTED.get_or_init(Instant::now);
}

pub fn mark_core_ready() {
    mark_ready(&CORE_READY_MS);
}

pub fn mark_rest_ready() {
    mark_ready(&REST_READY_MS);
}

fn mark_ready(mark: &AtomicU32) {
    if let Some(started) = STARTED.get() {
        let ms = started.elapsed().as_millis().min(u128::from(u32::MAX - 1)) as u32;
        let _ = mark.compare_exchange(u32::MAX, ms, Ordering::Relaxed, Ordering::Relaxed);
    }
}

fn readiness_ms(mark: &AtomicU32) -> Option<u32> {
    let ms = mark.load(Ordering::Relaxed);
    (ms != u32::MAX).then_some(ms)
}

pub fn set_gpu(gpu: Gpu) {
    *GPU.lock().unwrap_or_else(|e| e.into_inner()) = Some(gpu);
}

/// The screen the window is on: size in physical pixels, refresh rate, scale.
pub fn set_display(width: u32, height: u32, refresh_hz: u32, scale: f64) {
    *DISPLAY.lock().unwrap_or_else(|e| e.into_inner()) = Some(Display {
        width,
        height,
        refresh_hz,
        scale: scale as f32,
    });
}

/// The screen `window` is on, else the primary one, else the only one: right after the window
/// is made, Wayland doesn't say yet which screen it is on.
pub fn monitor(window: &winit::window::Window) -> Option<winit::monitor::MonitorHandle> {
    let only = || {
        let mut all = window.available_monitors();
        all.next().filter(|_| all.next().is_none())
    };
    window
        .current_monitor()
        .or_else(|| window.primary_monitor())
        .or_else(only)
}

/// Keeps [`monitor`]'s size, refresh rate and scale for the items' `system.display`.
pub fn note_display(window: &winit::window::Window) {
    if let Some(m) = monitor(window) {
        set_display(
            m.size().width,
            m.size().height,
            m.refresh_rate_millihertz().map_or(0, |r| r.div_ceil(1000)),
            m.scale_factor(),
        );
    }
}

/// wgpu's device type in the words items use.
pub fn gpu_kind(device_type: &str) -> &'static str {
    match device_type {
        "IntegratedGpu" => "integrated",
        "DiscreteGpu" => "discrete",
        "VirtualGpu" => "virtual",
        "Cpu" => "cpu",
        _ => "other",
    }
}

/// This machine, with the GPU and the screen once the renderer knows them. The drive is the one
/// under the game folder (the current folder).
pub fn system() -> System {
    let home = std::env::current_dir().ok();
    let mut system = System::detect_in(home.as_deref());
    if let Some(gpu) = GPU.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        system.vram_mb = match gpu.kind.as_str() {
            "discrete" => horizon_telemetry::system::vram_mb(&gpu.vendor).unwrap_or(0),
            _ => 0,
        };
        system.gpu = gpu.name;
        system.gpu_vendor = gpu.vendor;
        system.gpu_kind = gpu.kind;
        system.driver = gpu.driver;
        system.backend = gpu.backend;
    }
    system.display = DISPLAY.lock().unwrap_or_else(|e| e.into_inner()).clone();
    system
}

/// The release (`v0.2.3`), or the client's crate version for a build from source.
pub fn version() -> String {
    crate::crash::release_tag().unwrap_or_else(|| format!("dev {}", env!("CARGO_PKG_VERSION")))
}

/// What a crash report becomes when it may leave: the error and stack, the thread, and the
/// details in [`CRASH_DETAILS`], with the world's name and folder and the server's address
/// scrubbed wherever they appear. No log lines.
pub fn crash_item(report: &crate::crash::Report) -> Item {
    let mut scrubber = Scrubber::new();
    for (key, value) in &report.details {
        match key.as_str() {
            // `Name (saves/folder)`, as game.rs sets it.
            "World" => {
                let (name, folder) = value.rsplit_once(" (saves/").unwrap_or((value, ""));
                scrubber = scrubber.term(name).term(folder.trim_end_matches(')'));
            }
            "Server" => {
                let host = value.rsplit_once(':').map_or(value.as_str(), |(h, _)| h);
                scrubber = scrubber.term(value.as_str()).term(host);
            }
            _ => {}
        }
    }
    let crash = Crash {
        system: system(),
        error: scrubber.scrub(&report.error),
        stack: scrubber.scrub(&report.stack),
        thread: report.thread.clone(),
        details: report
            .details
            .iter()
            .filter(|(k, _)| CRASH_DETAILS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), scrubber.scrub(v)))
            .collect(),
    };
    let mut item = Item::new(Source::Game, version(), Payload::Crash(crash));
    // A huge stack keeps its top, where the error is.
    while item.validate().is_err() {
        let Payload::Crash(crash) = &mut item.payload else {
            break;
        };
        let keep = crash.stack.lines().count() * 3 / 4;
        if keep == 0 {
            crash.stack.clear();
            break;
        }
        crash.stack = crash
            .stack
            .lines()
            .take(keep)
            .map(|l| format!("{l}\n"))
            .collect();
    }
    item
}

/// Keeps the crash for the launcher to send, if the player allows crash reports.
pub fn keep_crash(home: &Path, report: &crate::crash::Report) {
    if let Err(e) = outbox::push(home, &crash_item(report)) {
        log::warn!("Horizon: could not keep the crash report to send: {e}");
    }
}

/// Keeps a benchmark report (`--benchmark`) for the launcher to send, if the player allows
/// benchmark uploads: the report as docs/benchmark-report.md defines it, dated to the day.
pub fn keep_benchmark(home: &Path, report: &horizon_bench::Report) {
    let Ok(data) = serde_json::to_value(report.for_upload()) else {
        return;
    };
    if let Err(e) = outbox::push(
        home,
        &Item::new(Source::Game, version(), Payload::Benchmark(data)),
    ) {
        log::warn!("Horizon: could not keep the benchmark report to send: {e}");
    }
}

/// One play session's performance, from entering a world to leaving it: frame and tick
/// times as [`Timings`] (fixed size whatever the length), never the frames themselves.
#[derive(Default)]
pub struct Session {
    frames: Timings,
    ticks: Timings,
    played_ms: f32,
    to_title_ms: Option<u32>,
    /// The start was measured: only the first session carries it.
    title_measured: bool,
    to_world_ms: Option<u32>,
    opening: Option<Instant>,
}

impl Session {
    /// The title screen drew a frame: the first time, how long the start took.
    pub fn title_shown(&mut self) {
        if !self.title_measured {
            self.title_measured = true;
            crate::spans::startup::mark("title.shown");
            // The frame itself: the menu bundle can be ready seconds before
            // the renderer that draws it (pipelines compiling on a slow driver).
            self.to_title_ms = STARTED.get().map(|s| s.elapsed().as_millis() as u32);
        }
    }

    /// A world starts loading.
    pub fn world_opening(&mut self) {
        if self.opening.is_none() {
            crate::spans::startup::arm("world.opening");
        }
        self.opening.get_or_insert_with(Instant::now);
    }

    /// An entry queued during startup was cancelled before opening a world.
    pub fn cancel_world_opening(&mut self) {
        self.opening = None;
    }

    /// A frame drawn while playing (not in menus or loading).
    pub fn frame(&mut self, ms: f32) {
        if let Some(opening) = self.opening.take() {
            crate::spans::startup::mark("world.first_frame");
            if self.to_world_ms.is_none() {
                self.to_world_ms = Some(opening.elapsed().as_millis() as u32);
            }
        }
        self.frames.record(ms);
        self.played_ms += ms;
    }

    /// One tick of the integrated server, in single player.
    pub fn tick(&mut self, ms: f32) {
        self.ticks.record(ms);
    }

    /// The session's summary, if it was long enough to say something.
    pub fn summary(&self, settings: Settings) -> Option<Perf> {
        let seconds = self.played_ms / 1000.0;
        (seconds >= MIN_SECONDS).then(|| Perf {
            system: system(),
            settings,
            seconds: seconds as u32,
            frames: self.frames.summary(),
            ticks: (self.ticks.count() > 0).then(|| self.ticks.summary()),
            peak_ram_mb: peak_ram_mb(),
            to_title_ms: self.to_title_ms,
            to_world_ms: self.to_world_ms,
            to_core_ms: self.to_title_ms.and_then(|_| readiness_ms(&CORE_READY_MS)),
            to_rest_ms: self.to_title_ms.and_then(|_| readiness_ms(&REST_READY_MS)),
        })
    }

    /// Keeps the session's summary for the launcher to send, if the player allows performance
    /// statistics, and starts the next session (the time to the title goes with the first).
    pub fn finish(&mut self, home: &Path, settings: Settings) {
        if horizon_telemetry::Consent::load(home).allows(horizon_telemetry::Kind::Perf)
            && let Some(perf) = self.summary(settings)
        {
            let item = Item::new(Source::Game, version(), Payload::Perf(perf));
            if let Err(e) = outbox::push(home, &item) {
                log::warn!("Horizon: could not keep the session's performance summary: {e}");
            }
        }
        *self = Self {
            title_measured: self.title_measured,
            ..Self::default()
        };
    }
}

/// The game's resident memory: the peak on Linux, what it uses now elsewhere.
fn peak_ram_mb() -> u32 {
    let kib = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("VmHWM:"))?
                    .split_whitespace()
                    .nth(1)?
                    .parse::<u64>()
                    .ok()
            })
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
    } else {
        None
    };
    kib.map_or(0, |k| (k / 1024) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_telemetry::{Choices, Consent, Kind};

    fn home(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-client-telemetry-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn allow(home: &Path, choices: Choices) {
        let mut consent = Consent::load(home);
        consent.answer(choices);
        consent.save(home).unwrap();
    }

    fn report() -> crate::crash::Report {
        crate::crash::Report {
            time: chrono::Local::now(),
            description: "Unexpected error".into(),
            error:
                "panicked at src/save.rs:10:5:\nno level.dat in /Users/ana/horizon/saves/Ana's Base"
                    .into(),
            stack: "horizon_client::save::load (crates/horizon-client/src/save.rs:10:5)\n".into(),
            thread: "Render thread".into(),
            side: "client",
            details: vec![
                (
                    "Horizon Version".into(),
                    "v2026.10.15 (client 0.1.0)".into(),
                ),
                (
                    "Operating System".into(),
                    "macos (aarch64) version macOS 26.0".into(),
                ),
                ("World".into(), "Castle Ana (saves/Ana's Base)".into()),
                ("Server".into(), "play.example.net:25565".into()),
                ("Resource Packs".into(), "Ana's pack".into()),
            ],
            log: vec!["[CHAT] <Ana> hi".into()],
        }
    }

    #[test]
    fn a_crash_keeps_only_what_says_what_happened() {
        let item = crash_item(&report());
        let Payload::Crash(crash) = &item.payload else {
            panic!("a crash item");
        };
        assert_eq!(
            crash.error,
            "panicked at src/save.rs:10:5:\nno level.dat in /Users/<user>/horizon/saves/<private>"
        );
        assert_eq!(
            crash.details,
            [(
                "Horizon Version".to_owned(),
                "v2026.10.15 (client 0.1.0)".to_owned()
            )]
        );
        let json = serde_json::to_string(&item).unwrap();
        for private in [
            "Castle Ana",
            "Ana's Base",
            "example.net",
            "Ana's pack",
            "CHAT",
        ] {
            assert!(!json.contains(private), "{private} in {json}");
        }
        assert!(item.validate().is_ok());
    }

    #[test]
    fn a_huge_stack_is_cut_to_fit() {
        let mut report = report();
        report.stack =
            "horizon_client::game::deep (crates/horizon-client/src/game.rs:1:1)\n".repeat(2000);
        let item = crash_item(&report);
        assert!(item.validate().is_ok());
        let Payload::Crash(crash) = item.payload else {
            panic!("a crash item");
        };
        assert!(crash.stack.lines().count() > 100);
    }

    #[test]
    fn benchmark_reports_need_their_own_consent_and_go_dated_to_the_day() {
        let home = home("benchmark");
        let mut report = horizon_bench::Report::new(horizon_bench::Kind::Game, Default::default());
        report.created = "2026-10-15T14:03:27Z".into();
        report.label = Some("my machine".into());
        keep_benchmark(&home, &report);
        allow(
            &home,
            Choices {
                performance: true,
                ..Choices::NONE
            },
        );
        keep_benchmark(&home, &report);
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                benchmarks: true,
                ..Choices::NONE
            },
        );
        keep_benchmark(&home, &report);
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        let text = serde_json::to_string(&pending[0].1).unwrap();
        assert!(
            text.contains("2026-10-15T00:00:00Z") && !text.contains("my machine"),
            "{text}"
        );
    }

    #[test]
    fn crashes_stay_local_without_consent() {
        let home = home("crash");
        keep_crash(&home, &report());
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                performance: true,
                ..Choices::NONE
            },
        );
        keep_crash(&home, &report());
        assert!(outbox::pending(&home).is_empty());
        allow(
            &home,
            Choices {
                crashes: true,
                ..Choices::NONE
            },
        );
        keep_crash(&home, &report());
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.kind(), Kind::Crash);
    }

    #[test]
    fn cancelled_startup_entry_does_not_time_menu_browsing() {
        let mut session = Session::default();
        session.world_opening();
        assert!(session.opening.is_some());
        session.cancel_world_opening();
        assert!(session.opening.is_none());
        session.frame(16.0);
        assert!(session.to_world_ms.is_none());
        session.world_opening();
        session.frame(16.0);
        assert!(session.to_world_ms.is_some());
    }

    #[test]
    fn a_session_is_kept_only_with_consent_and_once_long_enough() {
        let home = home("session");
        let mut session = Session::default();
        session.world_opening();
        (0..2000).for_each(|_| session.frame(16.6));
        session.finish(&home, Settings::default());
        assert!(outbox::pending(&home).is_empty(), "33 s, and no consent");
        allow(
            &home,
            Choices {
                performance: true,
                ..Choices::NONE
            },
        );
        (0..2000).for_each(|_| session.frame(16.6));
        session.finish(&home, Settings::default());
        assert!(outbox::pending(&home).is_empty(), "33 s is too short");
        session.world_opening();
        (0..4000).for_each(|_| session.frame(16.6));
        (0..1200).for_each(|_| session.tick(4.0));
        session.finish(
            &home,
            Settings {
                render_distance: 12,
                ..Settings::default()
            },
        );
        let pending = outbox::pending(&home);
        assert_eq!(pending.len(), 1);
        let Payload::Perf(perf) = &pending[0].1.payload else {
            panic!("a perf item");
        };
        assert_eq!(perf.seconds, 66);
        assert_eq!(perf.frames.count, 4000);
        assert_eq!(perf.ticks.as_ref().unwrap().p50_ms, 4.0);
        assert_eq!(perf.settings.render_distance, 12);
        assert!(perf.to_world_ms.is_some());
        assert!(serde_json::to_vec(&pending[0].1).unwrap().len() < 1024);
    }
}
