//! The live count over time, for playhorizon.dev/stats: `GET /v1/live/history`.
//!
//! Once a minute the server writes the live count (`live`) down: how many play, how many have a
//! world open to LAN or friends, and how many per platform. Only those totals, never a session,
//! a version or an address. Three tables, coarser as they age:
//!
//! - `live_minute`: one row a minute, kept [`crate::Config::live_minute_days`] (7 days).
//! - `live_hour`: per hour, the minutes' sum and count (for the average) and the highest one with
//!   its minute, kept [`crate::Config::live_hour_days`] (90 days).
//! - `live_day`: the same per day (UTC), kept forever.
//!
//! The `official_` tables hold the official server's player count the same way
//! ([`Series::Official`]), from its server-list status ping (`horizon_telemetry::server_status`),
//! playing only: a minute the server didn't answer has no row, so its downtime shows as a gap.
//!
//! The hour and day rows are updated with each minute, so they hold the current hour and day too.
//! Times are Unix seconds (UTC), each row's the start of its minute, hour or day.
use std::collections::BTreeMap;

use horizon_telemetry::live::{Count, PLATFORMS};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::store::Store;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS live_minute (
    t INTEGER PRIMARY KEY, playing INTEGER NOT NULL, lan INTEGER NOT NULL,
    linux INTEGER NOT NULL, macos INTEGER NOT NULL, windows INTEGER NOT NULL, other INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS live_hour (
    t INTEGER PRIMARY KEY, samples INTEGER NOT NULL, peak INTEGER NOT NULL, peak_at INTEGER NOT NULL,
    sum INTEGER NOT NULL, lan_peak INTEGER NOT NULL, lan_sum INTEGER NOT NULL,
    linux_sum INTEGER NOT NULL, macos_sum INTEGER NOT NULL, windows_sum INTEGER NOT NULL,
    other_sum INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS live_day (
    t INTEGER PRIMARY KEY, samples INTEGER NOT NULL, peak INTEGER NOT NULL, peak_at INTEGER NOT NULL,
    sum INTEGER NOT NULL, lan_peak INTEGER NOT NULL, lan_sum INTEGER NOT NULL,
    linux_sum INTEGER NOT NULL, macos_sum INTEGER NOT NULL, windows_sum INTEGER NOT NULL,
    other_sum INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS official_minute (
    t INTEGER PRIMARY KEY, playing INTEGER NOT NULL, lan INTEGER NOT NULL,
    linux INTEGER NOT NULL, macos INTEGER NOT NULL, windows INTEGER NOT NULL, other INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS official_hour (
    t INTEGER PRIMARY KEY, samples INTEGER NOT NULL, peak INTEGER NOT NULL, peak_at INTEGER NOT NULL,
    sum INTEGER NOT NULL, lan_peak INTEGER NOT NULL, lan_sum INTEGER NOT NULL,
    linux_sum INTEGER NOT NULL, macos_sum INTEGER NOT NULL, windows_sum INTEGER NOT NULL,
    other_sum INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS official_day (
    t INTEGER PRIMARY KEY, samples INTEGER NOT NULL, peak INTEGER NOT NULL, peak_at INTEGER NOT NULL,
    sum INTEGER NOT NULL, lan_peak INTEGER NOT NULL, lan_sum INTEGER NOT NULL,
    linux_sum INTEGER NOT NULL, macos_sum INTEGER NOT NULL, windows_sum INTEGER NOT NULL,
    other_sum INTEGER NOT NULL
) WITHOUT ROWID;
";

/// The per-platform columns, in [`PLATFORMS`]' order.
const PLATFORM_COLUMNS: [&str; 4] = ["linux", "macos", "windows", "other"];

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

/// Which count: the games' pings, or the official server's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Series {
    Players,
    Official,
}

impl Series {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "players" => Self::Players,
            "official" => Self::Official,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Players => "players",
            Self::Official => "official",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Players => "live",
            Self::Official => "official",
        }
    }
}

/// One minute's count, as it is stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    pub playing: u64,
    pub lan: u64,
    /// Per platform, in [`PLATFORMS`]' order.
    pub platforms: [u64; 4],
}

impl Sample {
    pub fn of(count: &Count) -> Self {
        let mut platforms = [0; 4];
        for (slot, name) in platforms.iter_mut().zip(PLATFORMS) {
            *slot = count.platforms.get(name).copied().unwrap_or(0);
        }
        Self {
            playing: count.playing,
            lan: count.lan,
            platforms,
        }
    }
}

/// What `GET /v1/live/history?range=` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Range {
    /// The last 24 hours, in 5-minute points.
    Day,
    /// The last 7 days, hourly.
    Week,
    /// The last 30 days, every 3 hours.
    Month,
    /// Every day since the first.
    All,
}

impl Range {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "24h" => Self::Day,
            "7d" => Self::Week,
            "30d" => Self::Month,
            "all" => Self::All,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
        }
    }

    /// The table it reads, its point's length, and how far back it goes (None: all of it).
    fn plan(self) -> (Table, i64, Option<i64>) {
        match self {
            Self::Day => (Table::Minute, 300, Some(DAY)),
            Self::Week => (Table::Hour, HOUR, Some(7 * DAY)),
            Self::Month => (Table::Hour, 3 * HOUR, Some(30 * DAY)),
            Self::All => (Table::Day, DAY, None),
        }
    }
}

#[derive(Clone, Copy)]
enum Table {
    Minute,
    Hour,
    Day,
}

impl Table {
    fn name(self, series: Series) -> String {
        let table = match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
        };
        format!("{}_{table}", series.prefix())
    }

    /// The SQL for a row's samples, sum, peak and peak's time, and a platform's sum: a minute row
    /// is one sample.
    fn columns(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match self {
            Self::Minute => ("1", "playing", "playing", "t", ""),
            Self::Hour | Self::Day => ("samples", "sum", "peak", "peak_at", "_sum"),
        }
    }
}

/// A count at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Peak {
    pub playing: u64,
    pub at: i64,
}

/// `GET /v1/live/history`'s answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct History {
    /// `players` or `official`.
    pub source: &'static str,
    pub range: &'static str,
    /// Seconds per point.
    pub step: i64,
    /// The first point's start and now.
    pub from: i64,
    pub to: i64,
    /// `[start, average, highest]` per point that has samples, oldest first. A point with no
    /// sample (the server was down) is missing.
    pub series: Vec<(i64, f64, u64)>,
    /// The highest count in the range and its minute.
    pub peak: Option<Peak>,
    /// The highest count ever recorded and its minute.
    pub all_time: Option<Peak>,
    /// The newest minute's count, at most a few minutes old while the series is recorded.
    pub latest: Option<Peak>,
    /// The average count per platform over the range.
    pub platforms: BTreeMap<String, f64>,
}

fn tenth(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

impl Store {
    /// Writes down `sample` as `series`' count of the minute starting at `minute` (Unix seconds),
    /// and adds it to its hour and day. False, and nothing changes, when that minute has one
    /// already.
    pub fn record_live(
        &mut self,
        series: Series,
        minute: i64,
        sample: &Sample,
    ) -> rusqlite::Result<bool> {
        let prefix = series.prefix();
        let minute = minute - minute.rem_euclid(60);
        let tx = self.db.transaction()?;
        let [linux, macos, windows, other] = sample.platforms.map(|n| n as i64);
        let (playing, lan) = (sample.playing as i64, sample.lan as i64);
        let new = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO {prefix}_minute (t, playing, lan, linux, macos, windows, other)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![minute, playing, lan, linux, macos, windows, other],
        )? == 1;
        if new {
            for (table, start) in [
                (format!("{prefix}_hour"), minute - minute.rem_euclid(HOUR)),
                (format!("{prefix}_day"), minute - minute.rem_euclid(DAY)),
            ] {
                // SQLite evaluates every SET against the row as it was, so `peak_at` sees the old
                // peak. An equal count keeps the earlier minute.
                tx.execute(
                    &format!(
                        "INSERT INTO {table} (t, samples, peak, peak_at, sum, lan_peak, lan_sum,
                             linux_sum, macos_sum, windows_sum, other_sum)
                         VALUES (?1, 1, ?2, ?3, ?2, ?4, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT (t) DO UPDATE SET
                             samples = samples + 1,
                             peak = max(peak, excluded.peak),
                             peak_at = CASE WHEN excluded.peak > peak THEN excluded.peak_at ELSE peak_at END,
                             sum = sum + excluded.sum,
                             lan_peak = max(lan_peak, excluded.lan_peak),
                             lan_sum = lan_sum + excluded.lan_sum,
                             linux_sum = linux_sum + excluded.linux_sum,
                             macos_sum = macos_sum + excluded.macos_sum,
                             windows_sum = windows_sum + excluded.windows_sum,
                             other_sum = other_sum + excluded.other_sum"
                    ),
                    params![start, playing, minute, lan, linux, macos, windows, other],
                )?;
            }
        }
        tx.commit()?;
        Ok(new)
    }

    /// Removes minutes older than `minute_days` and hours older than `hour_days` before `now`,
    /// in every series. Days stay.
    pub fn prune_live(
        &self,
        now: i64,
        minute_days: u32,
        hour_days: u32,
    ) -> rusqlite::Result<usize> {
        let mut removed = 0;
        for series in [Series::Players, Series::Official] {
            let prefix = series.prefix();
            removed += self.db.execute(
                &format!("DELETE FROM {prefix}_minute WHERE t < ?1"),
                [now - i64::from(minute_days) * DAY],
            )?;
            removed += self.db.execute(
                &format!("DELETE FROM {prefix}_hour WHERE t < ?1"),
                [now - i64::from(hour_days) * DAY],
            )?;
        }
        Ok(removed)
    }

    /// `which` series over `range`, up to `now` (Unix seconds).
    pub fn live_history(&self, which: Series, range: Range, now: i64) -> rusqlite::Result<History> {
        let (table, step, span) = range.plan();
        let (samples, sum, peak, peak_at, suffix) = table.columns();
        let name = table.name(which);
        let day_table = Table::Day.name(which);
        let from = span.map_or(0, |span| {
            let from = now - span;
            from - from.rem_euclid(step)
        });
        let mut series = Vec::new();
        let mut rows = self.db.prepare(&format!(
            "SELECT t / ?2 * ?2 AS b, CAST(SUM({sum}) AS REAL) / SUM({samples}), MAX({peak})
             FROM {name} WHERE t >= ?1 GROUP BY b ORDER BY b"
        ))?;
        for row in rows.query_map(params![from, step], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })? {
            let (t, avg, max) = row?;
            series.push((t, tenth(avg), max.max(0) as u64));
        }
        let peak_of = |since: i64| {
            self.db
                .query_row(
                    &format!(
                        "SELECT {peak}, {peak_at} FROM {name} WHERE t >= ?1
                         ORDER BY {peak} DESC, {peak_at} ASC LIMIT 1"
                    ),
                    [since],
                    |r| {
                        Ok(Peak {
                            playing: r.get::<_, i64>(0)?.max(0) as u64,
                            at: r.get(1)?,
                        })
                    },
                )
                .optional()
        };
        let all_time = self
            .db
            .query_row(
                &format!(
                    "SELECT peak, peak_at FROM {day_table} ORDER BY peak DESC, peak_at ASC LIMIT 1"
                ),
                [],
                |r| {
                    Ok(Peak {
                        playing: r.get::<_, i64>(0)?.max(0) as u64,
                        at: r.get(1)?,
                    })
                },
            )
            .optional()?;
        let minute_table = Table::Minute.name(which);
        let latest = self
            .db
            .query_row(
                &format!("SELECT playing, t FROM {minute_table} ORDER BY t DESC LIMIT 1"),
                [],
                |r| {
                    Ok(Peak {
                        playing: r.get::<_, i64>(0)?.max(0) as u64,
                        at: r.get(1)?,
                    })
                },
            )
            .optional()?;
        let averages = PLATFORM_COLUMNS
            .iter()
            .map(|c| format!("CAST(SUM({c}{suffix}) AS REAL) / SUM({samples})"))
            .collect::<Vec<_>>()
            .join(", ");
        let platforms = self.db.query_row(
            &format!("SELECT {averages} FROM {name} WHERE t >= ?1"),
            [from],
            |r| {
                (0..PLATFORM_COLUMNS.len())
                    .map(|i| r.get::<_, Option<f64>>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            },
        )?;
        Ok(History {
            source: which.name(),
            range: range.name(),
            step,
            from,
            to: now,
            series,
            peak: peak_of(from)?,
            all_time,
            latest,
            platforms: PLATFORMS
                .iter()
                .zip(platforms)
                .filter_map(|(p, avg)| Some(((*p).to_owned(), tenth(avg?))))
                .filter(|(_, avg)| *avg > 0.0)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(playing: u64, linux: u64) -> Sample {
        Sample {
            playing,
            lan: playing / 2,
            platforms: [linux, playing - linux, 0, 0],
        }
    }

    #[test]
    fn the_columns_follow_the_platforms() {
        assert_eq!(PLATFORM_COLUMNS, PLATFORMS);
    }

    #[test]
    fn a_minute_is_recorded_once_and_rolls_up() {
        let mut store = Store::open(":memory:").unwrap();
        let day = 20_000 * DAY;
        assert!(
            store
                .record_live(Series::Players, day + 10, &sample(4, 1))
                .unwrap()
        );
        // The same minute again (a late tick) changes nothing.
        assert!(
            !store
                .record_live(Series::Players, day + 59, &sample(9, 9))
                .unwrap()
        );
        store
            .record_live(Series::Players, day + 60, &sample(8, 2))
            .unwrap();
        store
            .record_live(Series::Players, day + 120, &sample(8, 8))
            .unwrap();
        store
            .record_live(Series::Players, day + HOUR, &sample(2, 0))
            .unwrap();

        let now = day + HOUR + 30;
        let h = store
            .live_history(Series::Players, Range::Day, now)
            .unwrap();
        assert_eq!(h.series, [(day, 6.7, 8), (day + HOUR, 2.0, 2)]);
        // The first minute that reached 8.
        assert_eq!(
            h.peak,
            Some(Peak {
                playing: 8,
                at: day + 60
            })
        );
        assert_eq!(h.all_time, h.peak);
        assert_eq!(h.platforms["linux"], 2.8);
        assert_eq!(h.platforms["macos"], 2.8);
        assert!(!h.platforms.contains_key("windows"));

        let week = store
            .live_history(Series::Players, Range::Week, now)
            .unwrap();
        assert_eq!(week.series, [(day, 6.7, 8), (day + HOUR, 2.0, 2)]);
        assert_eq!(week.peak, h.peak);
        let all = store
            .live_history(Series::Players, Range::All, now)
            .unwrap();
        assert_eq!(all.series, [(day, 5.5, 8)]);
        assert_eq!(all.step, DAY);
    }

    #[test]
    fn old_minutes_and_hours_go_and_days_stay() {
        let mut store = Store::open(":memory:").unwrap();
        let start = 20_000 * DAY;
        store
            .record_live(Series::Players, start, &sample(30, 0))
            .unwrap();
        let later = start + 100 * DAY;
        store
            .record_live(Series::Players, later, &sample(3, 0))
            .unwrap();
        assert_eq!(store.prune_live(later, 7, 90).unwrap(), 2);
        let all = store
            .live_history(Series::Players, Range::All, later)
            .unwrap();
        assert_eq!(all.series, [(start, 30.0, 30), (later, 3.0, 3)]);
        assert_eq!(all.all_time.unwrap().playing, 30);
        // The 24 hours hold only today; the all-time peak is still the old one.
        let day = store
            .live_history(Series::Players, Range::Day, later + 60)
            .unwrap();
        assert_eq!(day.series, [(later, 3.0, 3)]);
        assert_eq!(day.peak.unwrap().playing, 3);
        assert_eq!(day.all_time.unwrap().at, start);
    }

    #[test]
    fn an_empty_history_has_no_peak() {
        let store = Store::open(":memory:").unwrap();
        let h = store
            .live_history(Series::Players, Range::Month, 20_000 * DAY)
            .unwrap();
        assert!(h.series.is_empty());
        assert_eq!((h.peak, h.all_time, h.latest), (None, None, None));
        assert!(h.platforms.is_empty());
        assert_eq!(Range::parse("30d"), Some(Range::Month));
        assert_eq!(Range::parse("1y"), None);
        assert_eq!(Series::parse("official"), Some(Series::Official));
        assert_eq!(Series::parse("lan"), None);
    }

    #[test]
    fn the_official_series_is_kept_apart() {
        let mut store = Store::open(":memory:").unwrap();
        let day = 20_000 * DAY;
        store
            .record_live(Series::Players, day, &sample(40, 10))
            .unwrap();
        let official = Sample {
            playing: 6,
            ..Sample::default()
        };
        assert!(store.record_live(Series::Official, day, &official).unwrap());
        let h = store
            .live_history(Series::Official, Range::Day, day + 60)
            .unwrap();
        assert_eq!(h.series, [(day, 6.0, 6)]);
        assert_eq!(h.all_time.unwrap().playing, 6);
        assert_eq!(
            h.latest,
            Some(Peak {
                playing: 6,
                at: day
            })
        );
        assert!(h.platforms.is_empty());
        let players = store
            .live_history(Series::Players, Range::Day, day + 60)
            .unwrap();
        assert_eq!(players.all_time.unwrap().playing, 40);
        assert_eq!(store.prune_live(day + 100 * DAY, 7, 90).unwrap(), 4);
    }
}
