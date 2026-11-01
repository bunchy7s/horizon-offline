//! `horizon-report [SECONDS]` on the dedicated server's console (and `horizon-server report`): records every
//! tick's time and spans for a window (60 s unless told), then writes a performance report to share,
//! `reports/perf-<time>.json` and a short `.txt` beside it in the server's folder. It says where the ticks'
//! time went (the spans of `HORIZON_SPIKES`), shows the worst ticks with their spans, counts the entities by
//! type and the chunks of each level, and gives the memory, the CPU and the view settings. It holds no player
//! names, UUIDs or addresses, no seed and no world name.
use super::super::{Dim, Game};
use horizon_client::spans::{self, Span};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The longest window a report records.
pub(super) const MAX_SECONDS: u32 = 600;
/// The worst ticks a report lists with their spans.
const WORST: usize = 10;

struct Tick {
    wall_ms: f32,
    cpu_ms: Option<f32>,
    spans: Vec<Span>,
}

/// A report being recorded.
pub(super) struct Recording {
    seconds: u32,
    started: Instant,
    cpu_s: Option<f64>,
    /// Whether spans were on before (`HORIZON_SPIKES`): they stay on after.
    spans_were_on: bool,
    ticks: Vec<Tick>,
}

impl Recording {
    pub(super) fn start(seconds: u32) -> Self {
        let spans_were_on = spans::enabled();
        spans::enable(true);
        Self {
            seconds,
            started: Instant::now(),
            cpu_s: horizon_bench::cpu_seconds(std::process::id()),
            spans_were_on,
            ticks: Vec::with_capacity(seconds as usize * 20),
        }
    }
    pub(super) fn add(&mut self, wall_ms: f32, cpu_ms: Option<f32>, spans: Vec<Span>) {
        self.ticks.push(Tick {
            wall_ms,
            cpu_ms,
            spans,
        });
    }
    pub(super) fn done(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(u64::from(self.seconds))
    }
}

fn pct(sorted: &[f32], q: f64) -> f64 {
    let i = ((sorted.len() as f64 * q) as usize).min(sorted.len().saturating_sub(1));
    sorted.get(i).map_or(0.0, |&v| f64::from(v))
}
fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The tick times and spans of a recording, as the report's `ticks` and `spans`.
fn tick_summary(r: &Recording, window_s: f64) -> (Value, Value, Value) {
    let mut walls: Vec<f32> = r.ticks.iter().map(|t| t.wall_ms).collect();
    walls.sort_by(f32::total_cmp);
    let n = r.ticks.len().max(1) as f64;
    let mean = |f: &dyn Fn(&Tick) -> f64| r.ticks.iter().map(f).sum::<f64>() / n;
    let ticks = json!({
        "count": r.ticks.len(),
        "tps": round(r.ticks.len() as f64 / window_s.max(1e-9)),
        "mspt": {
            "average": round(mean(&|t| f64::from(t.wall_ms))),
            "p50": round(pct(&walls, 0.5)),
            "p95": round(pct(&walls, 0.95)),
            "p99": round(pct(&walls, 0.99)),
            "max": round(walls.last().copied().map_or(0.0, f64::from)),
        },
        "cpu_ms_average": round(mean(&|t| f64::from(t.cpu_ms.unwrap_or(t.wall_ms)))),
        "over_50_ms": walls.iter().filter(|&&w| w > 50.0).count(),
        "over_100_ms": walls.iter().filter(|&&w| w > 100.0).count(),
    });
    let mut totals: BTreeMap<&str, (f64, f64, usize)> = BTreeMap::new();
    for t in &r.ticks {
        for s in &t.spans {
            let e = totals.entry(s.name).or_default();
            e.0 += f64::from(s.ms);
            e.1 += f64::from(s.cpu_ms);
            e.2 += 1;
        }
    }
    let mut by_time: Vec<_> = totals.into_iter().collect();
    by_time.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let tick_ms = mean(&|t| f64::from(t.wall_ms)).max(1e-9);
    let spans = Value::Array(
        by_time
            .iter()
            .map(|(name, (ms, cpu, seen))| {
                json!({
                    "name": name,
                    "ms_per_tick": round(ms / n),
                    "cpu_ms_per_tick": round(cpu / n),
                    "share_of_tick": round(ms / n / tick_ms),
                    "ticks_seen": seen,
                })
            })
            .collect(),
    );
    let mut worst: Vec<&Tick> = r.ticks.iter().collect();
    worst.sort_by(|a, b| b.wall_ms.total_cmp(&a.wall_ms));
    let worst = Value::Array(
        worst
            .into_iter()
            .take(WORST)
            .map(|t| {
                json!({
                    "ms": round(f64::from(t.wall_ms)),
                    "cpu_ms": t.cpu_ms.map(|c| round(f64::from(c))),
                    "spans": t.spans.iter().take(8).map(|s| json!([s.name, round(f64::from(s.ms))])).collect::<Vec<_>>(),
                })
            })
            .collect(),
    );
    (ticks, spans, worst)
}

impl Game<'_> {
    /// The level in place and the parked ones: their chunks and their entities by type.
    fn report_levels(&self) -> Value {
        let mut levels = Vec::new();
        let here = |dim: Dim,
                    world: &horizon_client::world::World,
                    mobs: &[horizon_client::mobs::Mob]| {
            let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
            for m in mobs.iter().filter(|m| !m.removed) {
                *by_type.entry(&m.kind.id).or_default() += 1;
            }
            let mut by_type: Vec<_> = by_type.into_iter().collect();
            by_type.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            json!({
                "dimension": dim.id(),
                "chunks": world.chunks.len(),
                "block_entities": world.block_entities.len(),
                "mobs": mobs.iter().filter(|m| !m.removed).count(),
                "mobs_by_type": by_type.into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
            })
        };
        let mut current = here(self.dim, &self.world, &self.mobs);
        current["item_entities"] = json!(self.dropped.len());
        current["experience_orbs"] = json!(self.orbs.len());
        current["projectiles"] = json!(self.creatures.projectiles.len());
        current["minecarts"] = json!(self.carts.len());
        current["boats"] = json!(self.boats.len());
        levels.push(current);
        for dim in self.levels.parked_dims() {
            if let (Some(world), Some(mobs)) =
                (self.levels.parked_world(dim), self.levels.parked_mobs(dim))
            {
                levels.push(here(dim, world, mobs));
            }
        }
        Value::Array(levels)
    }

    /// The report of a finished recording: written to `reports/` in the server's folder, its summary logged.
    pub(super) fn finish_report(&mut self, r: Recording, view: (u32, u32), max_players: usize) {
        if !r.spans_were_on {
            spans::enable(false);
        }
        let window_s = r.started.elapsed().as_secs_f64();
        let (ticks, spans, worst) = tick_summary(&r, window_s);
        let cpu = horizon_bench::cpu_seconds(std::process::id())
            .zip(r.cpu_s)
            .map(|(end, start)| round((end - start) / window_s.max(1e-9)));
        let players = self.lan.as_ref().map_or(0, |h| h.hub.guests.len());
        let report = json!({
            "format": 1,
            "horizon": horizon_client::crash::version(),
            "minecraft": horizon_core::VANILLA_VERSION,
            "window_s": round(window_s),
            "ticks": ticks,
            "spans": spans,
            "worst_ticks": worst,
            "players": players,
            "levels": self.report_levels(),
            "memory": { "rss_mb": horizon_bench::rss_bytes(std::process::id()).map(|b| b >> 20) },
            "process": process_limits(),
            "cpu_cores_used": cpu,
            "config": {
                "view_distance": view.0,
                "simulation_distance": view.1,
                "max_players": max_players,
            },
        });
        let text = summary(&report);
        // The machine's description runs commands: off the tick.
        std::thread::spawn(move || {
            let mut report = report;
            let hw = horizon_bench::Hardware::detect();
            report["machine"] = json!({
                "os": hw.os, "arch": hw.arch, "cpu": hw.cpu, "cpu_threads": hw.cpu_threads, "ram_mb": hw.ram_mb,
            });
            match write_report(Path::new("reports"), &report, &text) {
                Ok(path) => log::info!(
                    "Horizon: performance report written to {}\n{text}",
                    path.display()
                ),
                Err(e) => log::error!("Horizon: could not write the performance report: {e}"),
            }
        });
    }
}

/// A few lines for the console: the tick rate and times, the five costliest spans, the worst tick.
fn summary(r: &Value) -> String {
    let t = &r["ticks"];
    let m = &t["mspt"];
    let mut out = format!(
        "{} players, TPS {}, MSPT average {} ms, P50 {}, P95 {}, P99 {}, max {} ({} ticks over 50 ms, {} over 100 ms)\n",
        r["players"],
        t["tps"],
        m["average"],
        m["p50"],
        m["p95"],
        m["p99"],
        m["max"],
        t["over_50_ms"],
        t["over_100_ms"],
    );
    let spans: Vec<String> = r["spans"]
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
        .map(|s| {
            format!(
                "{} {} ms",
                s["name"].as_str().unwrap_or(""),
                s["ms_per_tick"]
            )
        })
        .collect();
    out += &format!("per tick: {}\n", spans.join(", "));
    if let Some(w) = r["worst_ticks"].get(0) {
        out += &format!("worst tick {} ms: {}\n", w["ms"], w["spans"]);
    }
    let mobs: u64 = r["levels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["mobs"].as_u64())
        .sum();
    let chunks: u64 = r["levels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["chunks"].as_u64())
        .sum();
    out += &format!(
        "{mobs} mobs, {chunks} chunks, {} MB resident, {} cores of CPU",
        r["memory"]["rss_mb"], r["cpu_cores_used"]
    );
    let process = &r["process"];
    if let Some(threads) = process["threads"].as_u64() {
        out += &format!("\n{threads} threads");
        for (key, name) in [
            ("cgroup_pids_current", "container tasks"),
            ("cgroup_pids_max", "container task limit"),
            ("max_processes", "max processes (soft / hard)"),
        ] {
            if let Some(value) = process[key].as_str() {
                out += &format!(", {name} {value}");
            }
        }
    }
    out
}

/// What the system lets this process start: its threads now, and the limits
/// a container puts on them and on memory. A server out of threads fails to
/// start the ones it needs (EAGAIN). Read on Linux; null elsewhere.
fn process_limits() -> Value {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let threads = read("/proc/self/status").and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("Threads:"))
            .and_then(|v| v.trim().parse::<u64>().ok())
    });
    // "Max processes   <soft>   <hard>   processes"
    let max_processes = read("/proc/self/limits").and_then(|s| {
        let line = s.lines().find(|l| l.starts_with("Max processes"))?;
        let words: Vec<&str> = line.split_whitespace().skip(2).take(2).collect();
        Some(words.join(" / "))
    });
    let cgroup = |name: &str| read(&format!("/sys/fs/cgroup/{name}")).map(|s| s.trim().to_owned());
    json!({
        "threads": threads,
        "max_processes": max_processes,
        "cgroup_pids_current": cgroup("pids.current"),
        "cgroup_pids_max": cgroup("pids.max"),
        "cgroup_memory_current": cgroup("memory.current"),
        "cgroup_memory_max": cgroup("memory.max"),
    })
}

fn write_report(dir: &Path, report: &Value, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = chrono::Utc::now().format("%Y-%m-%d_%H-%M-%SZ");
    let path = dir.join(format!("perf-{stamp}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(report)? + "\n")?;
    std::fs::write(path.with_extension("txt"), format!("{text}\n"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &'static str, ms: f32) -> Span {
        Span {
            name,
            ms,
            cpu_ms: ms,
        }
    }

    #[test]
    fn ticks_and_spans_are_summed_per_tick() {
        let mut r = Recording {
            seconds: 1,
            started: Instant::now(),
            cpu_s: None,
            spans_were_on: true,
            ticks: Vec::new(),
        };
        for i in 0..20 {
            let wall = if i == 7 { 120.0 } else { 10.0 };
            r.add(
                wall,
                Some(wall - 1.0),
                vec![span("tick_mobs", wall / 2.0), span("tick_lan", 2.0)],
            );
        }
        let (ticks, spans, worst) = tick_summary(&r, 1.0);
        assert_eq!(ticks["count"], 20);
        assert_eq!(ticks["tps"], 20.0);
        assert_eq!(ticks["over_100_ms"], 1);
        assert_eq!(ticks["mspt"]["max"], 120.0);
        assert_eq!(spans[0]["name"], "tick_mobs");
        assert_eq!(spans[0]["ms_per_tick"], 7.75);
        assert_eq!(spans[1]["ms_per_tick"], 2.0);
        assert_eq!(worst[0]["ms"], 120.0);
        assert_eq!(worst[0]["spans"][0], json!(["tick_mobs", 60.0]));
        let text = summary(
            &json!({"ticks": ticks, "spans": spans, "worst_ticks": worst, "players": 3,
            "levels": [{"mobs": 5, "chunks": 9}], "memory": {"rss_mb": 300}, "cpu_cores_used": 1.5}),
        );
        assert!(text.starts_with("3 players, TPS 20.0"), "{text}");
        assert!(
            text.contains("per tick: tick_mobs 7.75 ms, tick_lan 2.0 ms"),
            "{text}"
        );
        assert!(
            text.ends_with("5 mobs, 9 chunks, 300 MB resident, 1.5 cores of CPU"),
            "{text}"
        );
        let text = summary(
            &json!({"ticks": ticks, "spans": spans, "worst_ticks": worst,
            "players": 3, "levels": [], "memory": {"rss_mb": 300}, "cpu_cores_used": 1.5,
            "process": {"threads": 140, "cgroup_pids_current": "160", "cgroup_pids_max": "512",
                "max_processes": "unlimited / unlimited"}}),
        );
        assert!(
            text.ends_with(
                "140 threads, container tasks 160, container task limit 512, \
                 max processes (soft / hard) unlimited / unlimited"
            ),
            "{text}"
        );
    }
}
