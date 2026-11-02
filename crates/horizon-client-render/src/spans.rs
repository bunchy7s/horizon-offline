//! Named timings within a frame, for finding what makes a frame slow:
//! `HORIZON_SPIKES=MS` prints the spans of every frame whose CPU time
//! exceeds MS milliseconds, and of every server tick (dedicated or
//! integrated) longer than MS. Each span keeps its wall time and the
//! thread's CPU time inside it, so a span that waited (for a lock, the GPU
//! or a core on a busy machine) shows apart from one that worked. Costs two
//! clock reads per span when enabled and a relaxed load otherwise.
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static SPANS: RefCell<Vec<Span>> = const { RefCell::new(Vec::new()) };
}

/// A named timing: wall milliseconds and the thread's CPU milliseconds in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub name: &'static str,
    pub ms: f32,
    pub cpu_ms: f32,
}

pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
/// Whether spans are being recorded.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
/// Run `f`, recording its duration under `name` when spans are enabled.
#[inline]
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !ENABLED.load(Ordering::Relaxed) {
        return f();
    }
    let (started, cpu) = (Instant::now(), thread_cpu());
    let out = f();
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    let cpu_ms = thread_cpu()
        .zip(cpu)
        .map_or(ms, |(end, start)| (end - start).as_secs_f32() * 1000.0);
    SPANS.with(|s| s.borrow_mut().push(Span { name, ms, cpu_ms }));
    out
}
/// Consecutive steps of one function timed as spans: each [`Laps::lap`] records the time since
/// the previous one (or since [`laps`]) under its name. Nothing is read while spans are off.
pub struct Laps(Option<(Instant, Option<Duration>)>);
/// Start timing the steps of a function (see [`Laps`]).
#[inline]
pub fn laps() -> Laps {
    Laps(enabled().then(|| (Instant::now(), thread_cpu())))
}
impl Laps {
    /// Record the step that just ended under `name`, and start the next.
    #[inline]
    pub fn lap(&mut self, name: &'static str) {
        let Some((started, cpu)) = &mut self.0 else {
            return;
        };
        let (now, now_cpu) = (Instant::now(), thread_cpu());
        let ms = (now - *started).as_secs_f32() * 1000.0;
        let cpu_ms = now_cpu
            .zip(*cpu)
            .map_or(ms, |(end, start)| (end - start).as_secs_f32() * 1000.0);
        SPANS.with(|s| s.borrow_mut().push(Span { name, ms, cpu_ms }));
        (*started, *cpu) = (now, now_cpu);
    }
}
/// The CPU time this thread has used, where the platform tells it: a tick
/// with more wall time than CPU time waited (for a lock, or for a core on a
/// busy machine).
pub fn thread_cpu() -> Option<Duration> {
    #[cfg(unix)]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        Some(Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    }
    #[cfg(not(unix))]
    horizon_threads::thread_cpu()
}

/// The frame's spans, summed by name, slowest first; clears them.
pub fn take() -> Vec<Span> {
    let spans = SPANS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let mut sums: Vec<Span> = Vec::new();
    for span in spans {
        match sums.iter_mut().find(|s| s.name == span.name) {
            Some(sum) => {
                sum.ms += span.ms;
                sum.cpu_ms += span.cpu_ms;
            }
            None => sums.push(span),
        }
    }
    sums.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    sums
}
/// `name wall` for spans of at least 0.5 ms, with `/cpu` when the thread
/// worked for less than nine tenths of it: the slow-frame and slow-tick lines.
pub fn describe(spans: &[Span]) -> String {
    let list: Vec<String> = spans
        .iter()
        .filter(|s| s.ms >= 0.5)
        .map(|s| {
            if s.cpu_ms < s.ms * 0.9 {
                format!("{} {:.1}/{:.1}", s.name, s.ms, s.cpu_ms)
            } else {
                format!("{} {:.1}", s.name, s.ms)
            }
        })
        .collect();
    list.join(", ")
}

/// The start's timeline: `HORIZON_STARTUP_TRACE=FILE` writes every named step of
/// the start (resource tables, the renderer, the title, a world's open, a
/// server's join, a dimension change) on whichever thread ran it, one Chrome
/// trace event per line (`tools/startup_trace.py` reads it; wrapped in `[...]`,
/// Perfetto opens it). Times are microseconds since the process started. Off,
/// a step costs one relaxed load.
pub mod startup {
    use std::{
        io::Write as _,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicBool, AtomicU32, Ordering},
        },
        time::Instant,
    };

    static ON: AtomicBool = AtomicBool::new(false);
    static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    static NEXT_THREAD: AtomicU32 = AtomicU32::new(1);
    thread_local! {
        /// This thread's trace id, given (with its name) on its first event.
        static THREAD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// Opens the trace named by `HORIZON_STARTUP_TRACE`, timed from `origin`
    /// (the process's start). Call once, first thing.
    pub fn init(origin: Instant) {
        ORIGIN.get_or_init(|| origin);
        let sink = SINK.get_or_init(|| {
            let path = std::env::var_os("HORIZON_STARTUP_TRACE")?;
            std::fs::File::create(&path).ok().map(Mutex::new)
        });
        ON.store(sink.is_some(), Ordering::Relaxed);
    }

    /// Whether a trace is being written.
    #[inline]
    pub fn on() -> bool {
        ON.load(Ordering::Relaxed)
    }

    fn micros(at: Instant) -> u128 {
        ORIGIN.get().map_or(0, |origin| {
            at.saturating_duration_since(*origin).as_micros()
        })
    }

    fn write(event: std::fmt::Arguments) {
        let Some(Some(sink)) = SINK.get() else {
            return;
        };
        let tid = THREAD.with(|id| {
            if id.get() == 0 {
                id.set(NEXT_THREAD.fetch_add(1, Ordering::Relaxed));
                let name = std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .replace(['"', '\\'], "_");
                let mut file = sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = writeln!(
                    file,
                    r#"{{"ph":"M","name":"thread_name","pid":1,"tid":{},"args":{{"name":"{name}"}}}}"#,
                    id.get()
                );
            }
            id.get()
        });
        let mut file = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = writeln!(file, r#"{{"pid":1,"tid":{tid},{event}}}"#);
    }

    /// A step running until it is dropped.
    #[must_use = "the step ends when this is dropped"]
    pub struct Step {
        name: &'static str,
        started: Instant,
    }
    impl Drop for Step {
        fn drop(&mut self) {
            let end = Instant::now();
            write(format_args!(
                r#""ph":"X","name":"{}","ts":{},"dur":{}"#,
                self.name,
                micros(self.started),
                end.saturating_duration_since(self.started).as_micros()
            ));
        }
    }

    /// Starts a step named `name` (a literal: no quotes or backslashes).
    #[inline]
    pub fn step(name: &'static str) -> Option<Step> {
        on().then(|| Step {
            name,
            started: Instant::now(),
        })
    }

    /// Runs `f` as a step named `name`.
    #[inline]
    pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
        let _step = step(name);
        f()
    }

    /// A moment of the start: the title drawn, the world in view.
    #[inline]
    pub fn mark(name: &'static str) {
        if on() {
            write(format_args!(
                r#""ph":"i","s":"g","name":"{name}","ts":{}"#,
                micros(Instant::now())
            ));
        }
    }

    /// The first time `name` happens, as a mark. `once` is the caller's flag.
    #[inline]
    pub fn mark_once(once: &AtomicBool, name: &'static str) {
        if on() && !once.swap(true, Ordering::Relaxed) {
            mark(name);
        }
    }

    /// Set while a world opens, a server is joined or the dimension changes,
    /// until the view first fills ([`view_filled`]).
    static FILLING: AtomicBool = AtomicBool::new(false);

    /// A world starts loading: the next full view is marked.
    pub fn arm(name: &'static str) {
        if on() {
            mark(name);
            FILLING.store(true, Ordering::Relaxed);
        }
    }

    /// Called each frame in a world: marks `world.view_full` the first frame
    /// after [`arm`] where nothing waits to load, generate, light, mesh or
    /// upload and the resident columns cover perf_bench's share of the view
    /// (0.8 pi r^2: corners are never loaded).
    #[inline]
    pub fn view_filled(resident: usize, pending: usize, radius: u32) {
        if FILLING.load(Ordering::Relaxed)
            && pending == 0
            && resident as f64 >= 0.8 * std::f64::consts::PI * f64::from(radius * radius)
            && FILLING.swap(false, Ordering::Relaxed)
        {
            mark("world.view_full");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    /// Spans of one name add up; a span the thread spent mostly waiting
    /// shows its CPU time after the wall time, short ones are left out.
    #[test]
    fn spans_sum_by_name_and_show_waits() {
        super::SPANS.with(|s| {
            s.borrow_mut().extend([
                Span {
                    name: "tick",
                    ms: 2.0,
                    cpu_ms: 2.0,
                },
                Span {
                    name: "submit",
                    ms: 30.0,
                    cpu_ms: 0.2,
                },
                Span {
                    name: "tick",
                    ms: 3.0,
                    cpu_ms: 2.9,
                },
                Span {
                    name: "hud",
                    ms: 0.3,
                    cpu_ms: 0.3,
                },
            ]);
        });
        let spans = super::take();
        assert_eq!(
            spans[0],
            Span {
                name: "submit",
                ms: 30.0,
                cpu_ms: 0.2
            }
        );
        assert_eq!((spans[1].name, spans[1].ms), ("tick", 5.0));
        assert_eq!(super::describe(&spans), "submit 30.0/0.2, tick 5.0");
        assert!(super::take().is_empty());
    }

    /// Laps record the steps between them while spans are on, and nothing while off.
    #[test]
    fn laps_time_consecutive_steps() {
        let _ = super::take();
        let mut off = super::laps();
        off.lap("unseen");
        super::enable(true);
        let mut laps = super::laps();
        std::thread::sleep(std::time::Duration::from_millis(2));
        laps.lap("first");
        laps.lap("second");
        super::enable(false);
        off.lap("still unseen");
        let spans = super::take();
        let names: Vec<_> = spans.iter().map(|s| s.name).collect();
        assert_eq!(names, ["first", "second"]);
        assert!(spans[0].ms >= 2.0);
    }

    /// The thread's CPU clock moves with work done on it, not with time
    /// spent asleep.
    #[cfg(any(unix, all(windows, target_arch = "x86_64")))]
    #[test]
    fn thread_cpu_counts_work_not_sleep() {
        let start = super::thread_cpu().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let slept = super::thread_cpu().unwrap() - start;
        // Busy until the clock shows 10 ms of work (a loaded machine gives
        // the thread less than its wall time).
        let mut x = 0u64;
        let busy = std::time::Instant::now();
        let worked = loop {
            x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
            let worked = super::thread_cpu().unwrap() - start - slept;
            if worked.as_millis() >= 10 || busy.elapsed().as_secs() >= 10 {
                break worked;
            }
        };
        assert!(slept.as_millis() < 25, "{slept:?} while asleep");
        assert!(worked.as_millis() >= 10, "{worked:?} while busy");
    }
}
