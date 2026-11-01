//! The database: one SQLite file, kept small.
//!
//! - `systems`: each distinct machine description once. Rows point at it, so a CPU and GPU
//!   name isn't stored again with every session. `extra` holds the later hardware fields
//!   (cores, clocks, GPU maker and kind, VRAM, drive, screen, power, form) as JSON.
//! - `perf`: one row per play session, in columns. A frame or tick summary is an 80-byte blob
//!   ([`pack`]), not JSON.
//! - `reports`: crash and bug reports and benchmark uploads, their `data` zstd-compressed.
//! - `perf_daily`: the anonymous totals per day, version, OS, GPU vendor and render distance
//!   that `/v1/aggregate` reads. Rebuilt from `perf` when a day changes (new sessions, a
//!   deletion), and kept after the raw rows expire.
//!
//! - `live_minute`, `live_hour`, `live_day`: the live count over time (`history`).
//!
//! Dates are the day the server received an item (days since 1970, UTC). The sender's IP
//! address is never stored.
use std::collections::{HashMap, HashSet};

use horizon_telemetry::Kind;
use horizon_telemetry::batch::Batch;
use horizon_telemetry::payload::{BUCKET_EDGES_MS, Payload, Summary, System};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const BUCKETS: usize = BUCKET_EDGES_MS.len() + 1;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS systems (
    id INTEGER PRIMARY KEY,
    os TEXT NOT NULL, os_version TEXT NOT NULL, arch TEXT NOT NULL, cpu TEXT NOT NULL,
    threads INTEGER NOT NULL, ram_gb INTEGER NOT NULL,
    gpu TEXT NOT NULL, driver TEXT NOT NULL, backend TEXT NOT NULL, extra TEXT NOT NULL DEFAULT '{}',
    UNIQUE (os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend, extra)
);
CREATE TABLE IF NOT EXISTS perf (
    install BLOB NOT NULL, day INTEGER NOT NULL, version TEXT NOT NULL, system INTEGER NOT NULL,
    render_distance INTEGER NOT NULL, simulation_distance INTEGER NOT NULL, max_fps INTEGER NOT NULL,
    vsync INTEGER NOT NULL, fullscreen INTEGER NOT NULL, graphics TEXT NOT NULL,
    seconds INTEGER NOT NULL, frames BLOB NOT NULL, ticks BLOB,
    peak_ram_mb INTEGER NOT NULL, to_title_ms INTEGER, to_world_ms INTEGER
);
CREATE INDEX IF NOT EXISTS perf_install ON perf (install);
CREATE INDEX IF NOT EXISTS perf_day ON perf (day);
CREATE TABLE IF NOT EXISTS reports (
    install BLOB NOT NULL, day INTEGER NOT NULL, kind INTEGER NOT NULL, version TEXT NOT NULL,
    system INTEGER, body BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS reports_install ON reports (install);
CREATE INDEX IF NOT EXISTS reports_kind_day ON reports (kind, day);
CREATE TABLE IF NOT EXISTS perf_daily (
    day INTEGER NOT NULL, version TEXT NOT NULL, os TEXT NOT NULL, gpu_vendor TEXT NOT NULL,
    render_distance INTEGER NOT NULL,
    sessions INTEGER NOT NULL, installs INTEGER NOT NULL, seconds INTEGER NOT NULL,
    buckets TEXT NOT NULL, p50_sum REAL NOT NULL, p99_sum REAL NOT NULL,
    PRIMARY KEY (day, version, os, gpu_vendor, render_distance)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS dirty (day INTEGER PRIMARY KEY) WITHOUT ROWID;
";

fn kind_code(kind: Kind) -> i64 {
    match kind {
        Kind::Perf => 0,
        Kind::Crash => 1,
        Kind::Bug => 2,
        Kind::Benchmark => 3,
    }
}

/// An install ID as 16 bytes.
pub fn install_key(id: &str) -> Option<Vec<u8>> {
    horizon_telemetry::consent::is_install_id(id).then(|| {
        (0..16)
            .map(|i| u8::from_str_radix(&id[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect()
    })
}

/// A summary as 80 bytes: the count, four percentiles, then the 14 buckets, little-endian.
pub fn pack(s: &Summary) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 16 + 4 * BUCKETS);
    out.extend(s.count.to_le_bytes());
    for ms in [s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms] {
        out.extend(ms.to_le_bytes());
    }
    for i in 0..BUCKETS {
        out.extend(s.buckets.get(i).copied().unwrap_or(0).to_le_bytes());
    }
    out
}

pub fn unpack(bytes: &[u8]) -> Option<Summary> {
    if bytes.len() != 8 + 16 + 4 * BUCKETS {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap_or_default());
    let f32_at = |i: usize| f32::from_bits(u32_at(i));
    Some(Summary {
        count: u64::from_le_bytes(bytes[..8].try_into().ok()?),
        p50_ms: f32_at(8),
        p95_ms: f32_at(12),
        p99_ms: f32_at(16),
        max_ms: f32_at(20),
        buckets: (0..BUCKETS).map(|i| u32_at(24 + 4 * i)).collect(),
    })
}

/// The GPU's maker, the one GPU detail the public totals group by.
pub fn gpu_vendor(gpu: &str) -> &'static str {
    let gpu = gpu.to_ascii_lowercase();
    [
        ("apple", "apple"),
        ("nvidia", "nvidia"),
        ("geforce", "nvidia"),
        ("radeon", "amd"),
        ("amd", "amd"),
        ("intel", "intel"),
        ("adreno", "qualcomm"),
        ("llvmpipe", "software"),
    ]
    .into_iter()
    .find(|(needle, _)| gpu.contains(needle))
    .map_or(if gpu.is_empty() { "unknown" } else { "other" }, |(_, v)| v)
}

pub struct Store {
    pub(crate) db: Connection,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let mut db = Connection::open(path)?;
        db.execute_batch(SCHEMA)?;
        db.execute_batch(crate::hub::SCHEMA)?;
        db.execute_batch(crate::intake::SCHEMA)?;
        db.execute_batch(crate::history::SCHEMA)?;
        migrate(&mut db)?;
        Ok(Self { db })
    }

    /// How many items an install has sent on `day`.
    pub fn count_on(&self, install: &[u8], day: u32) -> rusqlite::Result<u32> {
        self.db.query_row(
            "SELECT (SELECT count(*) FROM perf WHERE install = ?1 AND day = ?2)
                  + (SELECT count(*) FROM reports WHERE install = ?1 AND day = ?2)",
            params![install, day],
            |r| r.get(0),
        )
    }

    /// Keeps a decoded batch, received on `day`.
    pub fn insert(&mut self, batch: &Batch, day: u32) -> rusqlite::Result<()> {
        let install = install_key(&batch.install_id).unwrap_or_default();
        let tx = self.db.transaction()?;
        for item in &batch.items {
            match &item.payload {
                Payload::Perf(p) => {
                    let system = system_id(&tx, &p.system)?;
                    tx.execute(
                        "INSERT INTO perf (install, day, version, system, render_distance, simulation_distance,
                         max_fps, vsync, fullscreen, graphics, seconds, frames, ticks, peak_ram_mb,
                         to_title_ms, to_world_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                        params![
                            install, day, item.version, system,
                            p.settings.render_distance, p.settings.simulation_distance, p.settings.max_fps,
                            p.settings.vsync, p.settings.fullscreen, p.settings.graphics,
                            p.seconds, pack(&p.frames), p.ticks.as_ref().map(pack), p.peak_ram_mb,
                            p.to_title_ms, p.to_world_ms,
                        ],
                    )?;
                }
                payload => {
                    let system = match payload {
                        Payload::Crash(c) => Some(system_id(&tx, &c.system)?),
                        Payload::Bug(b) => Some(system_id(&tx, &b.system)?),
                        _ => None,
                    };
                    let data = match payload {
                        Payload::Crash(c) => serde_json::to_vec(c),
                        Payload::Bug(b) => serde_json::to_vec(b),
                        Payload::Benchmark(r) => serde_json::to_vec(r),
                        Payload::Perf(_) => unreachable!("handled above"),
                    }
                    .unwrap_or_default();
                    let body = ruzstd::encoding::compress_to_vec(
                        data.as_slice(),
                        ruzstd::encoding::CompressionLevel::Fastest,
                    );
                    tx.execute(
                        "INSERT INTO reports (install, day, kind, version, system, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![install, day, kind_code(item.kind()), item.version, system, body],
                    )?;
                }
            }
        }
        tx.execute("INSERT OR IGNORE INTO dirty (day) VALUES (?1)", [day])?;
        tx.commit()
    }

    /// Deletes everything an install sent; the days it touched get their totals rebuilt.
    pub fn delete_install(&mut self, install: &[u8]) -> rusqlite::Result<usize> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO dirty (day) SELECT DISTINCT day FROM perf WHERE install = ?1",
            [install],
        )?;
        let mut deleted = tx.execute("DELETE FROM perf WHERE install = ?1", [install])?;
        deleted += tx.execute("DELETE FROM reports WHERE install = ?1", [install])?;
        tx.commit()?;
        self.rebuild_dirty()?;
        Ok(deleted)
    }

    /// Removes raw rows received before `oldest_day`; the daily totals stay.
    pub fn expire(&mut self, oldest_day: u32) -> rusqlite::Result<usize> {
        // A day's totals are final before its rows go.
        self.rebuild_dirty()?;
        let mut n = self
            .db
            .execute("DELETE FROM perf WHERE day < ?1", [oldest_day])?;
        n += self
            .db
            .execute("DELETE FROM reports WHERE day < ?1", [oldest_day])?;
        self.db.execute(
            "DELETE FROM systems WHERE id NOT IN (SELECT system FROM perf)
             AND id NOT IN (SELECT system FROM reports WHERE system IS NOT NULL)",
            [],
        )?;
        Ok(n)
    }

    /// Rebuilds the daily totals of every day marked changed.
    pub fn rebuild_dirty(&mut self) -> rusqlite::Result<()> {
        let days: Vec<u32> = {
            let mut q = self.db.prepare("SELECT day FROM dirty")?;
            q.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for day in days {
            self.rebuild_day(day)?;
        }
        Ok(())
    }

    fn rebuild_day(&mut self, day: u32) -> rusqlite::Result<()> {
        #[derive(Default)]
        struct Group {
            sessions: u64,
            installs: HashSet<Vec<u8>>,
            seconds: u64,
            buckets: [u64; BUCKETS],
            p50_sum: f64,
            p99_sum: f64,
        }
        let tx = self.db.transaction()?;
        let mut groups: HashMap<(String, String, &'static str, u8), Group> = HashMap::new();
        {
            let mut q = tx.prepare(
                "SELECT perf.install, perf.version, systems.os, systems.gpu, perf.render_distance,
                 perf.seconds, perf.frames FROM perf JOIN systems ON systems.id = perf.system
                 WHERE perf.day = ?1",
            )?;
            let mut rows = q.query([day])?;
            while let Some(r) = rows.next()? {
                let gpu: String = r.get(3)?;
                let key = (r.get(1)?, r.get(2)?, gpu_vendor(&gpu), r.get(4)?);
                let Some(frames) = unpack(&r.get::<_, Vec<u8>>(6)?) else {
                    continue;
                };
                let g = groups.entry(key).or_default();
                g.sessions += 1;
                g.installs.insert(r.get(0)?);
                g.seconds += r.get::<_, u64>(5)?;
                for (sum, n) in g.buckets.iter_mut().zip(&frames.buckets) {
                    *sum += u64::from(*n);
                }
                g.p50_sum += f64::from(frames.p50_ms);
                g.p99_sum += f64::from(frames.p99_ms);
            }
        }
        tx.execute("DELETE FROM perf_daily WHERE day = ?1", [day])?;
        for ((version, os, vendor, distance), g) in groups {
            tx.execute(
                "INSERT INTO perf_daily VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    day,
                    version,
                    os,
                    vendor,
                    distance,
                    g.sessions,
                    g.installs.len(),
                    g.seconds,
                    serde_json::to_string(&g.buckets.to_vec()).unwrap_or_default(),
                    g.p50_sum,
                    g.p99_sum,
                ],
            )?;
        }
        tx.execute("DELETE FROM dirty WHERE day = ?1", [day])?;
        tx.commit()
    }

    /// The anonymous totals for the days `from..=to`, groups with fewer than `k` install-days
    /// left out, and the newest benchmark uploads.
    pub fn aggregate(&mut self, from: u32, to: u32, k: u64) -> rusqlite::Result<serde_json::Value> {
        self.rebuild_dirty()?;
        /// One group's sums over the window.
        #[derive(Default)]
        struct Totals {
            sessions: u64,
            install_days: u64,
            seconds: u64,
            buckets: [u64; BUCKETS],
            p50_sum: f64,
            p99_sum: f64,
        }
        let mut groups: HashMap<(String, String, String, u8), Totals> = HashMap::new();
        {
            let mut q = self.db.prepare(
                "SELECT version, os, gpu_vendor, render_distance, sessions, installs, seconds, buckets,
                 p50_sum, p99_sum FROM perf_daily WHERE day BETWEEN ?1 AND ?2",
            )?;
            let mut rows = q.query([from, to])?;
            while let Some(r) = rows.next()? {
                let g = groups
                    .entry((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    .or_default();
                g.sessions += r.get::<_, u64>(4)?;
                g.install_days += r.get::<_, u64>(5)?;
                g.seconds += r.get::<_, u64>(6)?;
                let buckets: Vec<u64> =
                    serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default();
                for (sum, n) in g.buckets.iter_mut().zip(buckets) {
                    *sum += n;
                }
                g.p50_sum += r.get::<_, f64>(8)?;
                g.p99_sum += r.get::<_, f64>(9)?;
            }
        }
        let (mut sessions, mut hours) = (0, 0.0);
        let mut out: Vec<serde_json::Value> = groups
            .into_iter()
            .filter(|(_, g)| g.install_days >= k)
            .map(|((version, os, vendor, distance), g)| {
                sessions += g.sessions;
                hours += g.seconds as f64 / 3600.0;
                let avg = |sum: f64| (sum / g.sessions as f64 * 10.0).round() / 10.0;
                serde_json::json!({
                    "version": version, "os": os, "gpu_vendor": vendor, "render_distance": distance,
                    "sessions": g.sessions, "install_days": g.install_days,
                    "hours": (g.seconds as f64 / 360.0).round() / 10.0,
                    "frame_buckets": g.buckets.to_vec(), "p50_ms": avg(g.p50_sum), "p99_ms": avg(g.p99_sum),
                })
            })
            .collect();
        out.sort_by(|a, b| b["sessions"].as_u64().cmp(&a["sessions"].as_u64()));
        let crashes: Vec<serde_json::Value> = {
            let mut q = self.db.prepare(
                "SELECT version, count(*) FROM reports WHERE kind = ?1 AND day BETWEEN ?2 AND ?3
                 GROUP BY version ORDER BY 2 DESC LIMIT 20",
            )?;
            q.query_map(params![kind_code(Kind::Crash), from, to], |r| {
                Ok(serde_json::json!({ "version": r.get::<_, String>(0)?, "crashes": r.get::<_, u64>(1)? }))
            })?
            .collect::<Result<_, _>>()?
        };
        let breakdowns = self.breakdowns(from, to)?;
        Ok(serde_json::json!({
            "from_day": from, "to_day": to, "min_install_days": k,
            "bucket_edges_ms": BUCKET_EDGES_MS.map(|e| (f64::from(e) * 10.0).round() / 10.0),
            "totals": { "sessions": sessions, "hours": (hours * 10.0).round() / 10.0 },
            "groups": out, "crashes_by_version": crashes, "breakdowns": breakdowns,
        }))
    }

    /// Play sessions in `from..=to` per hardware class (`classes`): for each RAM, VRAM, drive,
    /// OS and GPU kind class with at least `MIN_INSTALLS` different installs, its installs,
    /// sessions, hours and mean p50 and p99 frame times. From the raw rows, so only within the
    /// retention period.
    fn breakdowns(&self, from: u32, to: u32) -> rusqlite::Result<serde_json::Value> {
        #[derive(Default)]
        struct Cell {
            installs: HashSet<Vec<u8>>,
            sessions: u64,
            seconds: u64,
            p50_sum: f64,
            p99_sum: f64,
        }
        let mut cells: [HashMap<&'static str, Cell>; 5] = Default::default();
        let mut q = self.db.prepare(
            "SELECT perf.install, perf.seconds, perf.frames, systems.os, systems.ram_gb, systems.extra
             FROM perf JOIN systems ON systems.id = perf.system WHERE perf.day BETWEEN ?1 AND ?2",
        )?;
        let mut rows = q.query([from, to])?;
        while let Some(r) = rows.next()? {
            let Some(frames) = unpack(&r.get::<_, Vec<u8>>(2)?) else {
                continue;
            };
            let extra: Extra = serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default();
            let system = System {
                os: r.get(3)?,
                ram_gb: r.get(4)?,
                gpu_kind: extra.gpu_kind,
                vram_mb: extra.vram_mb,
                storage: extra.storage,
                ..System::default()
            };
            let install: Vec<u8> = r.get(0)?;
            let seconds: u64 = r.get(1)?;
            for (dim, class) in crate::classes::of_system(&system).into_iter().enumerate() {
                let Some(class) = class else { continue };
                let c = cells[dim].entry(class).or_default();
                c.installs.insert(install.clone());
                c.sessions += 1;
                c.seconds += seconds;
                c.p50_sum += f64::from(frames.p50_ms);
                c.p99_sum += f64::from(frames.p99_ms);
            }
        }
        let mut out = serde_json::Map::new();
        for (dim, by_class) in crate::classes::DIMENSIONS.iter().zip(cells) {
            let mut list: Vec<(&str, Cell)> = by_class
                .into_iter()
                .filter(|(_, c)| c.installs.len() >= crate::summary::MIN_INSTALLS)
                .collect();
            list.sort_by(|a, b| {
                b.1.installs
                    .len()
                    .cmp(&a.1.installs.len())
                    .then(a.0.cmp(b.0))
            });
            if list.is_empty() {
                continue;
            }
            let avg = |sum: f64, n: u64| (sum / n as f64 * 10.0).round() / 10.0;
            out.insert(
                dim.to_string(),
                list.into_iter()
                    .map(|(class, c)| {
                        serde_json::json!({
                            "class": class, "installs": c.installs.len(), "sessions": c.sessions,
                            "hours": (c.seconds as f64 / 360.0).round() / 10.0,
                            "p50_ms": avg(c.p50_sum, c.sessions), "p99_ms": avg(c.p99_sum, c.sessions),
                        })
                    })
                    .collect(),
            );
        }
        Ok(serde_json::Value::Object(out))
    }

    /// Every benchmark upload received since `from_day`, with its install (to count installs, never shown)
    /// and release, for `summary`.
    pub fn benchmark_uploads(
        &self,
        from_day: u32,
    ) -> rusqlite::Result<Vec<crate::summary::Upload>> {
        let mut q = self
            .db
            .prepare("SELECT install, version, body FROM reports WHERE kind = ?1 AND day >= ?2")?;
        q.query_map(params![kind_code(Kind::Benchmark), from_day], |r| {
            Ok(crate::summary::Upload {
                install: r.get(0)?,
                version: r.get(1)?,
                report: inflate(&r.get::<_, Vec<u8>>(2)?),
            })
        })?
        .collect()
    }

    /// The day an install's newest row came in, if any (tests, and checking a deletion).
    pub fn newest_day(&self, install: &[u8]) -> rusqlite::Result<Option<u32>> {
        self.db
            .query_row(
                "SELECT max(day) FROM (SELECT day FROM perf WHERE install = ?1
                 UNION ALL SELECT day FROM reports WHERE install = ?1)",
                [install],
                |r| r.get(0),
            )
            .optional()
            .map(Option::flatten)
    }
}

/// The hardware fields after the first nine, as `systems.extra` keeps them.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Extra {
    pub cores: u16,
    pub cpu_base_mhz: u32,
    pub cpu_boost_mhz: u32,
    pub gpu_vendor: String,
    pub gpu_kind: String,
    pub vram_mb: u32,
    pub storage: String,
    pub display: Option<horizon_telemetry::payload::Display>,
    pub power: String,
    pub form: String,
}

impl Extra {
    fn of(s: &System) -> Self {
        Self {
            cores: s.cores,
            cpu_base_mhz: s.cpu_base_mhz,
            cpu_boost_mhz: s.cpu_boost_mhz,
            gpu_vendor: s.gpu_vendor.clone(),
            gpu_kind: s.gpu_kind.clone(),
            vram_mb: s.vram_mb,
            storage: s.storage.clone(),
            display: s.display.clone(),
            power: s.power.clone(),
            form: s.form.clone(),
        }
    }
}

/// Databases from before `systems.extra`: the table rebuilt with it (ids kept), once.
fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let has_extra = db
        .prepare("SELECT 1 FROM pragma_table_info('systems') WHERE name = 'extra'")?
        .exists([])?;
    if has_extra {
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute_batch(
        "CREATE TABLE systems_new (
            id INTEGER PRIMARY KEY,
            os TEXT NOT NULL, os_version TEXT NOT NULL, arch TEXT NOT NULL, cpu TEXT NOT NULL,
            threads INTEGER NOT NULL, ram_gb INTEGER NOT NULL,
            gpu TEXT NOT NULL, driver TEXT NOT NULL, backend TEXT NOT NULL, extra TEXT NOT NULL DEFAULT '{}',
            UNIQUE (os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend, extra)
        );
        INSERT INTO systems_new (id, os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend)
            SELECT id, os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend FROM systems;
        DROP TABLE systems;
        ALTER TABLE systems_new RENAME TO systems;",
    )?;
    tx.commit()
}

fn system_id(tx: &rusqlite::Transaction, s: &System) -> rusqlite::Result<i64> {
    let extra = serde_json::to_string(&Extra::of(s)).unwrap_or_else(|_| "{}".into());
    tx.execute(
        "INSERT OR IGNORE INTO systems (os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![s.os, s.os_version, s.arch, s.cpu, s.threads, s.ram_gb, s.gpu, s.driver, s.backend, extra],
    )?;
    tx.query_row(
        "SELECT id FROM systems WHERE os = ?1 AND os_version = ?2 AND arch = ?3 AND cpu = ?4
         AND threads = ?5 AND ram_gb = ?6 AND gpu = ?7 AND driver = ?8 AND backend = ?9 AND extra = ?10",
        params![
            s.os,
            s.os_version,
            s.arch,
            s.cpu,
            s.threads,
            s.ram_gb,
            s.gpu,
            s.driver,
            s.backend,
            extra
        ],
        |r| r.get(0),
    )
}

/// A stored body back as JSON.
pub fn inflate(body: &[u8]) -> serde_json::Value {
    use std::io::Read;
    let mut json = Vec::new();
    ruzstd::decoding::StreamingDecoder::new(body)
        .ok()
        .and_then(|d| d.take(1 << 20).read_to_end(&mut json).ok())
        .and_then(|_| serde_json::from_slice(&json).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database from before `systems.extra` opens, keeps its machines' ids and takes new ones.
    #[test]
    fn systems_from_before_extra_are_kept() {
        let path = std::env::temp_dir().join(format!(
            "horizon-telemetry-migrate-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE systems (
                    id INTEGER PRIMARY KEY,
                    os TEXT NOT NULL, os_version TEXT NOT NULL, arch TEXT NOT NULL, cpu TEXT NOT NULL,
                    threads INTEGER NOT NULL, ram_gb INTEGER NOT NULL,
                    gpu TEXT NOT NULL, driver TEXT NOT NULL, backend TEXT NOT NULL,
                    UNIQUE (os, os_version, arch, cpu, threads, ram_gb, gpu, driver, backend)
                );
                INSERT INTO systems VALUES (7, 'linux', 'Fedora 43', 'x86_64', 'Ryzen', 32, 32, 'RTX 4070', '570', 'Vulkan');",
            )
            .unwrap();
        }
        let path_str = path.to_str().unwrap();
        let mut store = Store::open(path_str).unwrap();
        let old: (String, String) = store
            .db
            .query_row("SELECT gpu, extra FROM systems WHERE id = 7", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(old, ("RTX 4070".to_owned(), "{}".to_owned()));
        let s = System {
            os: "linux".into(),
            gpu: "RTX 4070".into(),
            vram_mb: 12288,
            gpu_kind: "discrete".into(),
            storage: "nvme".into(),
            ..System::default()
        };
        let tx = store.db.transaction().unwrap();
        let id = system_id(&tx, &s).unwrap();
        assert_eq!(system_id(&tx, &s).unwrap(), id, "the same machine once");
        let extra: String = tx
            .query_row("SELECT extra FROM systems WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        tx.commit().unwrap();
        let back: Extra = serde_json::from_str(&extra).unwrap();
        assert_eq!((back.vram_mb, back.storage.as_str()), (12288, "nvme"));
        drop(store);
        Store::open(path_str).unwrap(); // a second open doesn't rebuild again
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path_str}{suffix}"));
        }
    }
}
