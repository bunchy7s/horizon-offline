//! What an item holds. Summaries only: a session's frame times become a few percentiles and
//! [`BUCKET_EDGES_MS`] counts; benchmark reports are the documented report
//! (`docs/benchmark-report.md`) as it is.
use serde::{Deserialize, Serialize};

/// The four kinds of data a player can allow, one switch each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Perf,
    Crash,
    Bug,
    Benchmark,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Perf, Kind::Crash, Kind::Bug, Kind::Benchmark];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Perf => "perf",
            Kind::Crash => "crash",
            Kind::Bug => "bug",
            Kind::Benchmark => "benchmark",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The largest an item of this kind may be, as JSON.
    pub fn max_bytes(self) -> usize {
        match self {
            Kind::Perf => 4 * 1024,
            Kind::Crash => 32 * 1024,
            Kind::Bug => 80 * 1024,
            Kind::Benchmark => 128 * 1024,
        }
    }

    /// How many items of this kind one install keeps a day; the rest are dropped.
    pub fn daily_cap(self) -> u32 {
        match self {
            Kind::Perf => 12,
            Kind::Crash => 5,
            Kind::Bug => 5,
            Kind::Benchmark => 10,
        }
    }
}

/// What made the item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Launcher,
    Game,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Days since 1970 (UTC): no finer time is kept.
    pub day: u32,
    pub source: Source,
    /// The Horizon release (`v0.4.2`), or the launcher's version.
    pub version: String,
    #[serde(flatten)]
    pub payload: Payload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Payload {
    Perf(Perf),
    Crash(Crash),
    Bug(Bug),
    /// A benchmark report as `docs/benchmark-report.md` defines it, with its own schema field.
    Benchmark(serde_json::Value),
}

impl Item {
    pub fn new(source: Source, version: impl Into<String>, payload: Payload) -> Self {
        Self {
            day: crate::today(),
            source,
            version: version.into(),
            payload,
        }
    }

    pub fn kind(&self) -> Kind {
        match self.payload {
            Payload::Perf(_) => Kind::Perf,
            Payload::Crash(_) => Kind::Crash,
            Payload::Bug(_) => Kind::Bug,
            Payload::Benchmark(_) => Kind::Benchmark,
        }
    }

    /// Checks what the client and the server both refuse: an item over its kind's size, an
    /// overlong text, a benchmark report that is not an object.
    pub fn validate(&self) -> Result<(), String> {
        let size = serde_json::to_vec(self).map_or(usize::MAX, |j| j.len());
        let kind = self.kind();
        if size > kind.max_bytes() {
            return Err(format!(
                "a {} item of {size} bytes is over its {} bytes",
                kind.name(),
                kind.max_bytes()
            ));
        }
        if self.version.len() > 64 {
            return Err("the version is too long".into());
        }
        match &self.payload {
            Payload::Bug(bug) if bug.text.trim().is_empty() => {
                Err("a bug report needs text".into())
            }
            Payload::Bug(bug) if bug.text.chars().count() > Bug::MAX_TEXT => Err(format!(
                "a bug report is at most {} characters",
                Bug::MAX_TEXT
            )),
            Payload::Benchmark(report) if !report.is_object() => {
                Err("a benchmark report is a JSON object".into())
            }
            Payload::Perf(perf)
                if perf.slow_frames.len() > SlowFrames::MAX_SPANS
                    || perf
                        .slow_frames
                        .iter()
                        .any(|s| s.span.len() > SlowFrames::MAX_NAME) =>
            {
                Err(format!(
                    "at most {} slow-frame spans of {} bytes",
                    SlowFrames::MAX_SPANS,
                    SlowFrames::MAX_NAME
                ))
            }
            _ => Ok(()),
        }
    }
}

/// The machine, as coarse as is useful: no serial numbers, names or exact memory. Every field
/// explains performance; `docs/telemetry.md` lists them. Empty or zero: not known.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct System {
    /// `linux`, `macos` or `windows`.
    pub os: String,
    /// The distribution or system version (`macOS 26.0`, `Fedora Linux 43`).
    pub os_version: String,
    pub arch: String,
    pub cpu: String,
    pub threads: u16,
    /// Physical cores.
    pub cores: u16,
    /// The CPU's base and top boost clock, where the OS says.
    pub cpu_base_mhz: u32,
    pub cpu_boost_mhz: u32,
    /// Installed memory, rounded to the GiB.
    pub ram_gb: u16,
    pub gpu: String,
    /// `nvidia`, `amd`, `intel`, `apple`…
    pub gpu_vendor: String,
    /// `integrated`, `discrete`, `virtual` or `cpu`.
    pub gpu_kind: String,
    /// The GPU's own memory, rounded to 256 MiB (0 for a GPU that shares the system's).
    pub vram_mb: u32,
    pub driver: String,
    /// `Metal`, `Vulkan`, `Dx12`…
    pub backend: String,
    /// The drive under the game folder: `nvme`, `ssd` or `hdd`.
    pub storage: String,
    /// The screen the game's window is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    /// `ac` or `battery`, when the game started.
    pub power: String,
    /// `laptop` or `desktop`.
    pub form: String,
}

/// A screen: its size in physical pixels, refresh rate and scale.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub scale: f32,
}

/// The settings that change performance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub render_distance: u8,
    pub simulation_distance: u8,
    /// 0: unlimited.
    pub max_fps: u16,
    pub vsync: bool,
    pub fullscreen: bool,
    /// `fast`, `fancy` or `fabulous`.
    pub graphics: String,
}

/// One play session's performance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Perf {
    pub system: System,
    pub settings: Settings,
    /// Seconds of play measured (menus and loading screens excluded).
    pub seconds: u32,
    pub frames: Summary,
    /// The integrated server's milliseconds per tick, in single player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticks: Option<Summary>,
    /// The game's peak resident memory.
    pub peak_ram_mb: u32,
    /// From start to the title screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_title_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_core_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_rest_ms: Option<u32>,
    /// From choosing a world to playing in it, the first time this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_world_ms: Option<u32>,
    /// The frames of [`SLOW_FRAME_MS`] or more while playing, by what took longest in each:
    /// what froze. At most [`SlowFrames::MAX_SPANS`] entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slow_frames: Vec<SlowFrames>,
}

/// A frame this long or longer is a freeze worth explaining ([`Perf::slow_frames`]): the
/// edge of [`Summary::buckets`]' last bucket.
pub const SLOW_FRAME_MS: f32 = 250.0;

/// The slow frames whose longest step was one span: a named step of the render thread's frame
/// (`spans.rs` in the client: `render.upload`, `tick`, `world.stream`…), the same names in
/// every game of a release. `span` is empty when no step was timed in the frame (it waited
/// outside them).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlowFrames {
    pub span: String,
    pub frames: u32,
    /// The longest of these frames and the longest this span took in one of them.
    pub frame_max_ms: f32,
    pub span_max_ms: f32,
}

impl SlowFrames {
    /// Distinct spans a session keeps; the later ones count under [`SlowFrames::OTHER`].
    pub const MAX_SPANS: usize = 16;
    pub const MAX_NAME: usize = 64;
    pub const OTHER: &'static str = "other";

    /// Counts a slow frame of `frame_ms` whose longest span was `span` (`""`: none) for
    /// `span_ms`, in a session's list.
    pub fn record(list: &mut Vec<Self>, frame_ms: f32, span: &str, span_ms: f32) {
        let span = if span.len() > Self::MAX_NAME {
            Self::OTHER
        } else {
            span
        };
        let at = match list.iter().position(|s| s.span == span) {
            Some(at) => at,
            None if list.len() + 1 < Self::MAX_SPANS => {
                list.push(Self {
                    span: span.to_owned(),
                    ..Self::default()
                });
                list.len() - 1
            }
            None => match list.iter().position(|s| s.span == Self::OTHER) {
                Some(at) => at,
                None => {
                    list.push(Self {
                        span: Self::OTHER.to_owned(),
                        ..Self::default()
                    });
                    list.len() - 1
                }
            },
        };
        let entry = &mut list[at];
        entry.frames = entry.frames.saturating_add(1);
        entry.frame_max_ms = entry.frame_max_ms.max(frame_ms);
        entry.span_max_ms = entry.span_max_ms.max(span_ms);
    }
}

/// A crash, scrubbed: the error and its stack, the thread, and the details that say what the
/// game was doing (graphics adapter, settings), never its log, world or server.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crash {
    pub system: System,
    pub error: String,
    /// One frame a line, `function (file:line:col)`.
    pub stack: String,
    pub thread: String,
    pub details: Vec<(String, String)>,
}

/// A bug report the player wrote, with the log only if they chose to add it after reading it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bug {
    pub system: System,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}

impl Bug {
    pub const MAX_TEXT: usize = 4000;
}

/// Durations at a glance: percentiles and how many fell in each of [`BUCKET_EDGES_MS`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub count: u64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub max_ms: f32,
    /// One count a bucket: under 4 ms, 4–6 ms, … 100–250 ms, 250 ms and more.
    pub buckets: Vec<u32>,
}

/// The upper edges of [`Summary::buckets`] in milliseconds, around the usual refresh rates
/// (240, 165, 144, 120, 60, 30 Hz…); the last bucket has no edge.
pub const BUCKET_EDGES_MS: [f32; 13] = [
    4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.7, 20.0, 25.0, 33.3, 50.0, 100.0, 250.0,
];

/// Records durations in tenths of a millisecond up to [`Timings::FINE_MS`] (beyond, they count
/// at the top) for percentiles finer than the buckets; 20 KB whatever the session's length.
#[derive(Clone, Debug)]
pub struct Timings {
    tenths: Vec<u32>,
    count: u64,
    max_ms: f32,
}

impl Default for Timings {
    fn default() -> Self {
        Self::new()
    }
}

impl Timings {
    pub const FINE_MS: usize = 500;

    pub fn new() -> Self {
        Self {
            tenths: vec![0; Self::FINE_MS * 10 + 1],
            count: 0,
            max_ms: 0.0,
        }
    }

    pub fn record(&mut self, ms: f32) {
        if !ms.is_finite() || ms < 0.0 {
            return;
        }
        let bin = ((ms * 10.0) as usize).min(self.tenths.len() - 1);
        self.tenths[bin] = self.tenths[bin].saturating_add(1);
        self.count += 1;
        self.max_ms = self.max_ms.max(ms);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// The duration under which a fraction `p` of them fell, to the tenth of a millisecond.
    fn percentile(&self, p: f64) -> f32 {
        let rank = ((self.count as f64 * p).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (bin, &n) in self.tenths.iter().enumerate() {
            seen += u64::from(n);
            if seen >= rank {
                return if bin + 1 == self.tenths.len() {
                    self.max_ms
                } else {
                    ((bin + 1) as f32 / 10.0).min(self.max_ms)
                };
            }
        }
        self.max_ms
    }

    pub fn summary(&self) -> Summary {
        let mut buckets = vec![0u32; BUCKET_EDGES_MS.len() + 1];
        for (bin, &n) in self.tenths.iter().enumerate() {
            let ms = bin as f32 / 10.0;
            let bucket = BUCKET_EDGES_MS
                .iter()
                .position(|&edge| ms < edge - 0.01)
                .unwrap_or(BUCKET_EDGES_MS.len());
            buckets[bucket] = buckets[bucket].saturating_add(n);
        }
        let round = |ms: f32| (ms * 10.0).round() / 10.0;
        Summary {
            count: self.count,
            p50_ms: round(self.percentile(0.50)),
            p95_ms: round(self.percentile(0.95)),
            p99_ms: round(self.percentile(0.99)),
            max_ms: round(self.max_ms),
            buckets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_tagged_by_kind_and_read_back() {
        let item = Item::new(
            Source::Game,
            "v0.4.2",
            Payload::Bug(Bug {
                text: "Water flickers".into(),
                ..Bug::default()
            }),
        );
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["kind"], "bug");
        assert_eq!(json["source"], "game");
        assert_eq!(json["data"]["text"], "Water flickers");
        assert_eq!(serde_json::from_value::<Item>(json).unwrap(), item);
    }

    #[test]
    fn a_benchmark_report_is_carried_as_it_is() {
        let report = serde_json::json!({"schema": "horizon-benchmark-report/1", "scenes": []});
        let item = Item::new(
            Source::Launcher,
            "0.1.5",
            Payload::Benchmark(report.clone()),
        );
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["data"], report);
        assert!(item.validate().is_ok());
        let not_object = Item::new(Source::Launcher, "0.1.5", Payload::Benchmark(1.into()));
        assert!(not_object.validate().is_err());
    }

    #[test]
    fn oversized_items_are_refused() {
        let bug = |text: String| {
            Item::new(
                Source::Game,
                "v1",
                Payload::Bug(Bug {
                    text,
                    ..Bug::default()
                }),
            )
        };
        assert!(bug("x".repeat(Bug::MAX_TEXT)).validate().is_ok());
        assert!(bug("x".repeat(Bug::MAX_TEXT + 1)).validate().is_err());
        assert!(bug("  ".into()).validate().is_err());
        let huge = Item::new(
            Source::Game,
            "v1",
            Payload::Crash(Crash {
                stack: "frame\n".repeat(10_000),
                ..Crash::default()
            }),
        );
        assert!(huge.validate().unwrap_err().contains("over its"));
    }

    #[test]
    fn timings_give_percentiles_and_buckets() {
        let mut t = Timings::new();
        // 90 frames at 16.6 ms (60 Hz), 9 at 30 ms, one 400 ms hitch.
        (0..90).for_each(|_| t.record(16.6));
        (0..9).for_each(|_| t.record(30.0));
        t.record(400.0);
        t.record(f32::NAN);
        let s = t.summary();
        assert_eq!(s.count, 100);
        assert_eq!(s.p50_ms, 16.7);
        assert_eq!(s.p95_ms, 30.1);
        assert_eq!(s.p99_ms, 30.1);
        assert_eq!(s.max_ms, 400.0);
        assert_eq!(s.buckets.len(), 14);
        assert_eq!(s.buckets[6], 90, "14–16.7 ms");
        assert_eq!(s.buckets[9], 9, "25–33.3 ms");
        assert_eq!(s.buckets[13], 1, "250 ms and more");
        assert_eq!(s.buckets.iter().sum::<u32>(), 100);
    }

    #[test]
    fn hitches_beyond_the_fine_range_keep_their_maximum() {
        let mut t = Timings::new();
        t.record(2_000.0);
        let s = t.summary();
        assert_eq!((s.p50_ms, s.max_ms), (2_000.0, 2_000.0));
        assert_eq!(s.buckets[13], 1);
    }

    #[test]
    fn a_session_summary_is_small() {
        let mut t = Timings::new();
        (0..200_000).for_each(|i| t.record(5.0 + (i % 300) as f32 / 10.0));
        let perf = Item::new(
            Source::Game,
            "v0.4.2",
            Payload::Perf(Perf {
                frames: t.summary(),
                ticks: Some(t.summary()),
                ..Perf::default()
            }),
        );
        let size = serde_json::to_vec(&perf).unwrap().len();
        assert!(size < 1024, "{size} bytes");
        assert!(perf.validate().is_ok());
    }

    /// Slow frames count under their longest span; past the cap, new spans share one entry,
    /// so a session full of different freezes still fits its item.
    #[test]
    fn slow_frames_are_counted_by_span_within_a_cap() {
        let mut list = Vec::new();
        SlowFrames::record(&mut list, 300.0, "render.upload", 200.0);
        SlowFrames::record(&mut list, 900.0, "render.upload", 120.0);
        SlowFrames::record(&mut list, 260.0, "", 0.0);
        assert_eq!(
            list[0],
            SlowFrames {
                span: "render.upload".into(),
                frames: 2,
                frame_max_ms: 900.0,
                span_max_ms: 200.0,
            }
        );
        assert_eq!((list[1].span.as_str(), list[1].frames), ("", 1));
        for i in 0..40 {
            SlowFrames::record(&mut list, 400.0, &format!("step.{i}"), 300.0);
        }
        SlowFrames::record(&mut list, 400.0, &"x".repeat(200), 300.0);
        assert_eq!(list.len(), SlowFrames::MAX_SPANS);
        let other = list.iter().find(|s| s.span == SlowFrames::OTHER).unwrap();
        // 40 spans, 13 of which found room after the first two.
        assert_eq!(other.frames, 40 - 13 + 1);
        let perf = Item::new(
            Source::Game,
            "v0.4.2",
            Payload::Perf(Perf {
                slow_frames: list.clone(),
                ..Perf::default()
            }),
        );
        assert!(perf.validate().is_ok());
        let too_many = Item::new(
            Source::Game,
            "v0.4.2",
            Payload::Perf(Perf {
                slow_frames: [list.clone(), list].concat(),
                ..Perf::default()
            }),
        );
        assert!(too_many.validate().is_err());
    }
}
