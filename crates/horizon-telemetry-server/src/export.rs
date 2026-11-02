//! The protected, read-only export for the people who tune performance: the raw benchmark
//! reports and the per-hardware totals, with no minimum number of installs (the public
//! `/v1/aggregate` and `/v1/benchmarks/summary` hide small groups).
//!
//! - `GET /v1/export/benchmarks?after=<id>&limit=100`: `{"reports": [{"id", "day", "version",
//!   "report"}], "next": <id>|null}`, oldest first. Pass `next` as `after` for the next page.
//! - `GET /v1/export/hardware?days=30&offset=0&limit=200`: `{"machines": [...], "next_offset"}`:
//!   one entry per distinct machine description (CPU threads and cores, RAM, GPU, drive, OS…),
//!   with its installs, sessions, hours and frame-time numbers, busiest first.
//!
//! Both need `Authorization: Bearer <TELEMETRY_EXPORT_TOKEN>`, are off (404) while the server has
//! no such token, answer gzip to a client that accepts it, and are never cached. Install IDs
//! never leave the database: a report has no owner here, and a machine only a count of them.
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use horizon_telemetry::payload::BUCKET_EDGES_MS;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::store::{Extra, Store, unpack};
use crate::{App, peer, status};

/// Equal, in a time that depends on the token's length only, not on where they differ.
pub(crate) fn same(sent: &[u8], token: &[u8]) -> bool {
    let mut diff = sent.len() ^ token.len();
    for (i, t) in token.iter().enumerate() {
        diff |= usize::from(sent.get(i).copied().unwrap_or(0) ^ t);
    }
    diff == 0
}

/// The answer to send instead when the request's `Bearer` token doesn't check out: 404 while the
/// export is off, 401 for a missing or wrong token, 429 once an address has failed too often.
/// None: go ahead.
fn refuse(
    app: &App,
    headers: &HeaderMap,
    req_peer: Option<std::net::SocketAddr>,
) -> Option<Response> {
    let Some(token) = app.config.export_token.as_deref() else {
        return Some(StatusCode::NOT_FOUND.into_response());
    };
    let sent = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if same(sent.as_bytes(), token.as_bytes()) {
        return None;
    }
    // Only failures take from the address's bucket, so guessing is slow and using the token isn't.
    if !app.allow(app.client(headers, req_peer)) {
        return Some(status(StatusCode::TOO_MANY_REQUESTS, "too many requests"));
    }
    Some(StatusCode::UNAUTHORIZED.into_response())
}

/// `value` as JSON, gzipped when the client accepts it.
fn answer(headers: &HeaderMap, value: Value) -> Response {
    let json = value.to_string().into_bytes();
    let gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',').any(|c| {
                c.trim()
                    .split(';')
                    .next()
                    .is_some_and(|c| c.eq_ignore_ascii_case("gzip"))
            })
        });
    let mut res = if gzip {
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let packed = z.write_all(&json).and_then(|()| z.finish());
        match packed {
            Ok(packed) => {
                let mut res = packed.into_response();
                res.headers_mut()
                    .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
                res
            }
            Err(_) => json.into_response(),
        }
    } else {
        json.into_response()
    };
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    res
}

#[derive(Deserialize)]
pub struct Page {
    after: Option<i64>,
    limit: Option<u32>,
}

/// `GET /v1/export/benchmarks`: the raw benchmark reports, `limit` (1-500, default 100) a page.
pub async fn benchmarks(
    State(app): State<Arc<App>>,
    Query(page): Query<Page>,
    req: Request,
) -> Response {
    if let Some(res) = refuse(&app, req.headers(), peer(&req)) {
        return res;
    }
    let limit = page.limit.unwrap_or(100).clamp(1, 500);
    let listed = app
        .store()
        .export_benchmarks(page.after.unwrap_or(0), limit);
    match listed {
        Ok(reports) => {
            let next = (reports.len() as u32 == limit)
                .then(|| reports.last().map(|r| r["id"].clone()))
                .flatten();
            answer(req.headers(), json!({ "reports": reports, "next": next }))
        }
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not export the benchmarks: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

#[derive(Deserialize)]
pub struct Machines {
    days: Option<u32>,
    offset: Option<usize>,
    limit: Option<usize>,
}

/// `GET /v1/export/hardware`: play sessions of the last `days` (1-366, default 30) per machine
/// description, `limit` (1-1000, default 200) machines a page from `offset`.
pub async fn hardware(
    State(app): State<Arc<App>>,
    Query(q): Query<Machines>,
    req: Request,
) -> Response {
    if let Some(res) = refuse(&app, req.headers(), peer(&req)) {
        return res;
    }
    let days = q.days.unwrap_or(30).clamp(1, 366);
    let (offset, limit) = (q.offset.unwrap_or(0), q.limit.unwrap_or(200).clamp(1, 1000));
    let to = (app.today)();
    let made = app.store().export_machines(to + 1 - days.min(to + 1), to);
    match made {
        Ok(all) => {
            let total = all.len();
            let machines: Vec<Value> = all.into_iter().skip(offset).take(limit).collect();
            let next = (offset + machines.len() < total).then_some(offset + machines.len());
            answer(
                req.headers(),
                json!({
                    "from_day": to + 1 - days.min(to + 1), "to_day": to, "total_machines": total,
                    "bucket_edges_ms": BUCKET_EDGES_MS.map(|e| (f64::from(e) * 10.0).round() / 10.0),
                    "machines": machines, "next_offset": next,
                }),
            )
        }
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not export the machines: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

impl Store {
    /// Benchmark reports with an id above `after`, oldest first.
    pub fn export_benchmarks(&self, after: i64, limit: u32) -> rusqlite::Result<Vec<Value>> {
        let mut q = self.db.prepare(
            "SELECT rowid, day, version, body FROM reports WHERE kind = 3 AND rowid > ?1
             ORDER BY rowid LIMIT ?2",
        )?;
        q.query_map(rusqlite::params![after, limit], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "day": r.get::<_, u32>(1)?, "version": r.get::<_, String>(2)?,
                "report": crate::store::inflate(&r.get::<_, Vec<u8>>(3)?),
            }))
        })?
        .collect()
    }

    /// Play sessions in `from..=to` per machine description, the busiest first. A machine is its
    /// CPU, memory, GPU, drive and system: two sessions differing only in the screen or in
    /// battery against mains count as one. No group is left out for being small.
    pub fn export_machines(&self, from: u32, to: u32) -> rusqlite::Result<Vec<Value>> {
        #[derive(Default)]
        struct Machine {
            installs: HashSet<Vec<u8>>,
            sessions: u64,
            seconds: u64,
            p50_sum: f64,
            p99_sum: f64,
            buckets: Vec<u64>,
            peak_ram_mb: u32,
            description: Value,
        }
        let mut machines: HashMap<String, Machine> = HashMap::new();
        let mut q = self.db.prepare(
            "SELECT perf.install, perf.seconds, perf.frames, perf.peak_ram_mb, systems.os, systems.os_version,
             systems.arch, systems.cpu, systems.threads, systems.ram_gb, systems.gpu, systems.driver,
             systems.backend, systems.extra
             FROM perf JOIN systems ON systems.id = perf.system WHERE perf.day BETWEEN ?1 AND ?2",
        )?;
        let mut rows = q.query([from, to])?;
        while let Some(r) = rows.next()? {
            let Some(frames) = unpack(&r.get::<_, Vec<u8>>(2)?) else {
                continue;
            };
            let extra: Extra = serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default();
            let description = json!({
                "os": r.get::<_, String>(4)?, "os_version": r.get::<_, String>(5)?,
                "arch": r.get::<_, String>(6)?, "cpu": r.get::<_, String>(7)?,
                "threads": r.get::<_, u16>(8)?, "cores": extra.cores,
                "cpu_base_mhz": extra.cpu_base_mhz, "cpu_boost_mhz": extra.cpu_boost_mhz,
                "ram_gb": r.get::<_, u16>(9)?,
                "gpu": r.get::<_, String>(10)?, "gpu_vendor": extra.gpu_vendor, "gpu_kind": extra.gpu_kind,
                "vram_mb": extra.vram_mb, "driver": r.get::<_, String>(11)?,
                "backend": r.get::<_, String>(12)?, "storage": extra.storage, "form": extra.form,
            });
            let m = machines.entry(description.to_string()).or_default();
            if m.buckets.is_empty() {
                m.buckets = vec![0; frames.buckets.len()];
                m.description = description;
            }
            m.installs.insert(r.get(0)?);
            m.sessions += 1;
            m.seconds += r.get::<_, u64>(1)?;
            m.peak_ram_mb = m.peak_ram_mb.max(r.get(3)?);
            m.p50_sum += f64::from(frames.p50_ms);
            m.p99_sum += f64::from(frames.p99_ms);
            for (sum, n) in m.buckets.iter_mut().zip(&frames.buckets) {
                *sum += u64::from(*n);
            }
        }
        let mut list: Vec<(String, Machine)> = machines.into_iter().collect();
        // Busiest first; the description breaks ties so pages are stable.
        list.sort_by(|a, b| b.1.sessions.cmp(&a.1.sessions).then(a.0.cmp(&b.0)));
        let avg = |sum: f64, n: u64| (sum / n as f64 * 10.0).round() / 10.0;
        Ok(list
            .into_iter()
            .map(|(_, m)| {
                let mut out = m.description;
                out["installs"] = m.installs.len().into();
                out["sessions"] = m.sessions.into();
                out["hours"] = ((m.seconds as f64 / 360.0).round() / 10.0).into();
                out["p50_ms"] = avg(m.p50_sum, m.sessions).into();
                out["p99_ms"] = avg(m.p99_sum, m.sessions).into();
                out["peak_ram_mb"] = m.peak_ram_mb.into();
                out["frame_buckets"] = m.buckets.into();
                out
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::same;

    #[test]
    fn tokens_are_compared_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abd", b"abc"));
        assert!(!same(b"ab", b"abc"));
        assert!(!same(b"abcd", b"abc"));
        assert!(!same(b"", b"abc"));
        // A token with a trailing NUL byte isn't the token without it.
        assert!(!same(b"abc\0", b"abc"));
    }
}
