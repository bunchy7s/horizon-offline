//! The launcher's own measurements, in the benchmark report format (docs/benchmark-report.md):
//! start to the first frame, the release list fetch, installs and preparing, Play to the game's
//! first frame, its memory, UI frame times and size. Each session rewrites one report in
//! `benchmarks/` of the launcher's data folder, a local log the player can attach to an issue;
//! nothing leaves the machine (an upload needs the telemetry consent).
//!
//! `horizon-launcher benchmark` measures the same things on purpose: several starts (the first is
//! the cold one), page switching, repeated release list fetches, idle CPU.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use horizon_bench::{App, Frames, Hardware, Kind, Report};

/// The first thing `main` does; every start time counts from here.
pub fn process_started() -> Instant {
    static STARTED: OnceLock<Instant> = OnceLock::new();
    *STARTED.get_or_init(Instant::now)
}

/// Frames further apart than this are the UI resting, not UI frame times.
const IDLE_GAP: Duration = Duration::from_millis(250);
pub use horizon_launcher::backend::GAME_FIRST_FRAME;

pub struct Perf {
    report: Report,
    path: PathBuf,
    values: BTreeMap<String, Vec<f64>>,
    first_frame: Option<Instant>,
    last_frame: Option<Instant>,
    ui_frames: Vec<f32>,
    saved: Instant,
    dirty: bool,
    /// Off for the windows `horizon-launcher benchmark` starts: they report to it instead.
    log: bool,
    /// Machine probes run after the first frame, never while opening the window.
    hardware: Option<Receiver<Machine>>,
}

#[derive(Debug)]
struct Machine {
    hardware: Hardware,
    power: Option<String>,
    load_average: Option<f64>,
}

impl Machine {
    fn detect() -> Self {
        Self {
            hardware: Hardware::detect(),
            power: horizon_bench::power_source().map(str::to_owned),
            load_average: horizon_bench::load_average(),
        }
    }
}

impl Perf {
    pub fn new(scenario: &str) -> Perf {
        let hardware = Hardware {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            ..Hardware::default()
        };
        let mut report = Report::with_hardware(Kind::Launcher, app(), hardware);
        report.conditions.scenario = Some(scenario.to_owned());
        let path = crate::backend::data_dir().join("benchmarks").join(format!(
            "launcher-{}-{scenario}.json",
            report
                .created
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
        ));
        let mut perf = Perf {
            report,
            path,
            values: BTreeMap::new(),
            first_frame: None,
            last_frame: None,
            ui_frames: Vec::new(),
            saved: Instant::now(),
            dirty: false,
            log: true,
            hardware: None,
        };
        if let Some(bytes) = binary_bytes() {
            perf.record("launcher.binary_bytes", bytes);
        }
        perf
    }

    pub fn keep_local_log(&mut self, log: bool) {
        self.log = log;
    }

    /// One more value of a measurement; the report keeps their summary.
    pub fn record(&mut self, name: &str, value: f64) {
        self.values.entry(name.to_owned()).or_default().push(value);
        self.dirty = true;
    }

    pub fn record_since(&mut self, name: &str, since: Instant) {
        self.record(name, since.elapsed().as_secs_f64() * 1000.0);
    }

    /// Called on every frame the window draws. Returns true for the first.
    pub fn frame(&mut self) -> bool {
        let now = Instant::now();
        let first = self.first_frame.is_none();
        if first {
            self.first_frame = Some(now);
            self.record_since("launcher.start_ms", process_started());
            let (sender, receiver) = mpsc::channel();
            self.hardware = Some(receiver);
            std::thread::spawn(move || {
                // The window may close before the optional report details finish.
                let _ = sender.send(Machine::detect());
            });
        } else if let Some(gap) = self.last_frame.map(|t| now - t).filter(|g| *g < IDLE_GAP) {
            self.ui_frames.push(gap.as_secs_f32() * 1000.0);
        }
        self.last_frame = Some(now);
        first
    }

    pub fn ui_frame_times(&self) -> &[f32] {
        &self.ui_frames
    }

    /// Samples memory and writes the report when something changed, at most every few seconds.
    pub fn tick(&mut self) {
        self.poll_hardware();
        if self.saved.elapsed() < Duration::from_secs(5) {
            return;
        }
        if let Some(rss) = horizon_bench::rss_bytes(std::process::id()) {
            self.values
                .entry("launcher.rss_mb".into())
                .or_default()
                .push(rss as f64 / f64::from(1 << 20));
        }
        if self.dirty {
            self.save();
        }
        self.saved = Instant::now();
    }

    pub fn report(&mut self) -> &Report {
        self.poll_hardware();
        for (name, values) in &self.values {
            self.report.metric(name, values);
        }
        self.report.frames = Frames::of("ui", 0, &self.ui_frames, &[], &[])
            .into_iter()
            .collect();
        self.report.created = horizon_bench::utc_now();
        &self.report
    }

    /// Takes the machine's details into the report once their thread has them.
    fn poll_hardware(&mut self) {
        let Some(receiver) = &self.hardware else {
            return;
        };
        match receiver.try_recv() {
            Ok(machine) => {
                self.report.hardware = machine.hardware;
                self.report.conditions.power = machine.power;
                self.report.conditions.load_avg_1m = machine.load_average;
                self.hardware = None;
                self.dirty = true;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.hardware = None,
        }
    }

    pub fn save(&mut self) {
        self.dirty = false;
        if !self.log {
            return;
        }
        let json = self.report().to_json();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, json + "\n");
    }
}

impl Drop for Perf {
    fn drop(&mut self) {
        if self.dirty {
            self.save();
        }
    }
}

/// What a window started by `horizon-launcher benchmark` does next (`Launcher::drain`, every 80 ms).
pub enum Step {
    Wait,
    /// Show this page (an index into `Page::ALL`, wrapping).
    Go(usize),
    /// Show the longest list (Versions) and scroll it on every frame while [`Bench::scrolling`].
    Scroll,
    Quit,
}

const SWITCHING: Duration = Duration::from_secs(4);
const SCROLLING: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_secs(3);
/// The environment variable that makes a launcher window measure itself into the file it names.
const BENCH_ENV: &str = "HORIZON_LAUNCHER_BENCH";

/// A launcher window measuring itself: after its first frame it switches pages every tick for
/// a few seconds (each switch animates, so the UI draws continuously), scrolls the longest list
/// on every frame for a few more, then rests, then writes what it measured for `benchmark` and
/// quits.
pub struct Bench {
    out: PathBuf,
    began: Option<Instant>,
    page: usize,
    /// When scrolling began, and the UI frames drawn by then and by its end.
    scroll: Option<(Instant, usize, Option<usize>)>,
    idle_cpu: Option<(Instant, f64)>,
}

impl Bench {
    /// Whether this launcher measures itself (`HORIZON_LAUNCHER_BENCH` set).
    pub fn wanted() -> bool {
        std::env::var_os(BENCH_ENV).is_some()
    }

    pub fn from_env() -> Option<Bench> {
        let out = std::env::var_os(BENCH_ENV)?;
        Some(Bench {
            out: out.into(),
            began: None,
            page: 0,
            scroll: None,
            idle_cpu: None,
        })
    }

    /// The window scrolls its content on every frame now.
    pub fn scrolling(&self) -> bool {
        matches!(self.scroll, Some((_, _, None)))
    }

    pub fn tick(&mut self, perf: &Perf) -> Step {
        let Some(first) = perf.first_frame else {
            return Step::Wait;
        };
        let began = *self.began.get_or_insert_with(Instant::now);
        if began.elapsed() < SWITCHING {
            self.page += 1;
            return Step::Go(self.page);
        }
        let frames = perf.ui_frame_times().len();
        match &mut self.scroll {
            None => {
                self.scroll = Some((Instant::now(), frames, None));
                return Step::Scroll;
            }
            Some((at, _, end @ None)) => {
                if at.elapsed() < SCROLLING {
                    return Step::Wait;
                }
                *end = Some(frames);
            }
            Some(_) => {}
        }
        let pid = std::process::id();
        let Some((rested, cpu)) = self.idle_cpu else {
            self.idle_cpu = Some((
                Instant::now(),
                horizon_bench::cpu_seconds(pid).unwrap_or(0.0),
            ));
            return Step::Wait;
        };
        if rested.elapsed() < IDLE {
            return Step::Wait;
        }
        let cpu_idle_pct = horizon_bench::cpu_seconds(pid)
            .map(|now| 100.0 * (now - cpu) / rested.elapsed().as_secs_f64());
        let epoch_ms = |at: Instant| {
            let ago = at.elapsed();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            (now.saturating_sub(ago)).as_secs_f64() * 1000.0
        };
        let ui = perf.ui_frame_times();
        let scroll = match self.scroll {
            Some((_, from, Some(to))) => &ui[from.min(to)..to.min(ui.len())],
            _ => &[],
        };
        let result = serde_json::json!({
            "first_frame_epoch_ms": epoch_ms(first),
            "ui_ms": ui,
            "scroll_ms": scroll,
            "rss_mb": horizon_bench::rss_bytes(pid).map(|b| b as f64 / f64::from(1 << 20)),
            "cpu_idle_pct": cpu_idle_pct,
        });
        let _ = std::fs::write(&self.out, result.to_string());
        Step::Quit
    }
}

/// `horizon-launcher benchmark`: starts the launcher `repeats` times (the first start is the cold
/// one), each switching pages then resting, times the release list fetch, and writes one report.
pub fn benchmark(repeats: usize) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut report = Report::new(Kind::Launcher, app());
    report.conditions.scenario = Some("benchmark".into());
    report.conditions.repeats = Some(repeats as u32);
    report.conditions.power = horizon_bench::power_source().map(str::to_owned);
    report.conditions.load_avg_1m = horizon_bench::load_average();
    let (mut starts, mut rss, mut cpu) = (Vec::new(), Vec::new(), Vec::new());
    let scratch =
        std::env::temp_dir().join(format!("horizon-launcher-bench-{}", std::process::id()));
    for repeat in 0..repeats {
        let _ = std::fs::remove_file(&scratch);
        let spawned = std::time::SystemTime::now();
        let mut child = std::process::Command::new(&exe)
            .env(BENCH_ENV, &scratch)
            .spawn()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err(
                    "the launcher window drew nothing for 90 s: is the screen locked or asleep?"
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = std::fs::read_to_string(&scratch)
            .map_err(|_| "the launcher window quit without measuring".to_owned())?;
        let _ = std::fs::remove_file(&scratch);
        let result: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let spawned_ms = spawned
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;
        if let Some(first) = result["first_frame_epoch_ms"].as_f64() {
            starts.push(first - spawned_ms);
        }
        rss.extend(result["rss_mb"].as_f64());
        cpu.extend(result["cpu_idle_pct"].as_f64());
        let times = |key: &str| -> Vec<f32> {
            result[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_f64())
                .map(|v| v as f32)
                .collect()
        };
        let (ui, scroll) = (times("ui_ms"), times("scroll_ms"));
        report
            .frames
            .extend(Frames::of("pages", repeat as u32, &ui, &[], &[]));
        report
            .frames
            .extend(Frames::of("scroll", repeat as u32, &scroll, &[], &[]));
        let mut sorted = scroll.clone();
        sorted.sort_by(f32::total_cmp);
        let at = |q: f32| {
            sorted
                .get(((sorted.len() as f32 - 1.) * q) as usize)
                .copied()
        };
        eprintln!(
            "start {}: {:.0} ms to the first frame; scrolling {} frames, median {:.1} ms, p95 {:.1} ms",
            repeat + 1,
            starts.last().unwrap_or(&0.0),
            sorted.len(),
            at(0.5).unwrap_or(0.),
            at(0.95).unwrap_or(0.)
        );
    }
    report.metric("launcher.cold_start_ms", &starts[..starts.len().min(1)]);
    report.metric(
        "launcher.warm_start_ms",
        starts.get(1..).unwrap_or_default(),
    );
    report.metric("launcher.rss_mb", &rss);
    report.metric("launcher.cpu_idle_pct", &cpu);
    let mut fetches = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        crate::backend::releases()?;
        fetches.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report.metric("launcher.release_list_ms", &fetches);
    if let Some(bytes) = binary_bytes() {
        report.metric("launcher.binary_bytes", &[bytes]);
    }
    // Kept for upload only if the player allows benchmark uploads (the outbox checks).
    if let Ok(data) = serde_json::to_value(report.for_upload()) {
        let item = horizon_telemetry::Item::new(
            horizon_telemetry::Source::Launcher,
            crate::backend::VERSION,
            horizon_telemetry::Payload::Benchmark(data),
        );
        let _ = horizon_telemetry::outbox::push(&crate::backend::game_home(), &item);
    }
    report
        .write_to(&crate::backend::data_dir().join("benchmarks"))
        .map_err(|e| e.to_string())
}

fn app() -> App {
    App {
        name: "horizon-launcher".into(),
        version: crate::backend::VERSION.into(),
        ..App::default()
    }
}

fn binary_bytes() -> Option<f64> {
    Some(std::fs::metadata(std::env::current_exe().ok()?).ok()?.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_tracking_starts_without_hardware_probes() {
        let mut perf = Perf::new("test");
        perf.keep_local_log(false);
        assert!(perf.hardware.is_none());
        assert_eq!(perf.report.hardware.os, std::env::consts::OS);
        assert!(perf.report.hardware.cpu.is_none());
        assert!(perf.report.conditions.power.is_none());
    }

    #[test]
    fn background_hardware_details_preserve_recorded_metrics() {
        let mut perf = Perf::new("test");
        perf.keep_local_log(false);
        perf.record("launcher.start_ms", 125.);
        let (sender, receiver) = mpsc::channel();
        perf.hardware = Some(receiver);
        assert_eq!(perf.report().metrics["launcher.start_ms"].mean, 125.);
        sender
            .send(Machine {
                hardware: Hardware {
                    cpu: Some("Detected CPU".into()),
                    ..Hardware::default()
                },
                power: Some("ac".into()),
                load_average: Some(0.5),
            })
            .unwrap();
        let report = perf.report();
        assert_eq!(report.hardware.cpu.as_deref(), Some("Detected CPU"));
        assert_eq!(report.conditions.power.as_deref(), Some("ac"));
        assert_eq!(report.metrics["launcher.start_ms"].mean, 125.);
        assert!(perf.hardware.is_none());
    }

    #[test]
    fn the_first_frame_is_the_start_and_close_frames_are_ui_frames() {
        let mut perf = Perf::new("test");
        perf.keep_local_log(false);
        assert!(perf.frame(), "the first frame");
        assert!(!perf.frame());
        std::thread::sleep(IDLE_GAP + Duration::from_millis(20));
        perf.frame();
        assert_eq!(
            perf.ui_frame_times().len(),
            1,
            "a resting UI is not a slow frame"
        );
        perf.record("launcher.release_list_ms", 120.0);
        perf.record("launcher.release_list_ms", 80.0);
        let report = perf.report();
        assert!(report.metrics.contains_key("launcher.start_ms"));
        assert!(report.metrics.contains_key("launcher.binary_bytes"));
        assert_eq!(report.metrics["launcher.release_list_ms"].mean, 100.0);
        assert_eq!(report.frames[0].phase, "ui");
    }

    #[test]
    fn a_bench_window_waits_for_its_first_frame_then_switches_pages() {
        let mut perf = Perf::new("test");
        perf.keep_local_log(false);
        let mut bench = Bench {
            out: std::env::temp_dir().join("unused"),
            began: None,
            page: 0,
            idle_cpu: None,
            scroll: None,
        };
        assert!(matches!(bench.tick(&perf), Step::Wait));
        perf.frame();
        assert!(matches!(bench.tick(&perf), Step::Go(1)));
        assert!(matches!(bench.tick(&perf), Step::Go(2)));
    }
}
