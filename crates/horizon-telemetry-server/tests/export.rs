//! The protected export through its HTTP interface: off without a token, 401 without the right
//! one, raw benchmark reports paginated, per-machine totals with the CPU core counts, gzip.
use std::io::Read;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use horizon_telemetry::Item;
use horizon_telemetry::batch::{self, Batch};
use horizon_telemetry::payload::{Payload, Perf, Settings, Source, System, Timings};
use horizon_telemetry_server::store::Store;
use horizon_telemetry_server::{App, Config, router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const DAY: u32 = 20_741;
const TOKEN: &str = "a-long-random-export-token";

fn install(n: u32) -> String {
    format!("{n:032x}")
}

fn app(token: Option<&str>) -> Arc<App> {
    App::with_clock(
        Store::open(":memory:").unwrap(),
        Config {
            requests_per_hour: 100_000.0,
            export_token: token.map(str::to_owned),
            ..Config::default()
        },
        Box::new(|| DAY),
    )
}

fn perf(cpu: &str, threads: u16, cores: u16, storage: &str) -> Item {
    let mut t = Timings::new();
    (0..3000).for_each(|i| t.record(6.0 + (i % 50) as f32 / 10.0));
    let mut item = Item::new(
        Source::Game,
        "v0.2.97",
        Payload::Perf(Perf {
            system: System {
                os: "windows".into(),
                cpu: cpu.into(),
                threads,
                cores,
                ram_gb: 32,
                gpu: "NVIDIA GeForce RTX 4070".into(),
                gpu_vendor: "nvidia".into(),
                gpu_kind: "discrete".into(),
                backend: "Dx12".into(),
                storage: storage.into(),
                ..System::default()
            },
            settings: Settings {
                render_distance: 12,
                ..Settings::default()
            },
            seconds: 3600,
            frames: t.summary(),
            peak_ram_mb: 2500,
            ..Perf::default()
        }),
    );
    item.day = DAY;
    item
}

fn benchmark(n: u32) -> Item {
    let mut item = Item::new(
        Source::Game,
        "v0.2.97",
        Payload::Benchmark(json!({
            "format": "horizon-benchmark-report", "version": 1, "n": n,
            "hardware": { "os": "windows", "cpu_threads": 32, "cpu_cores": 16, "storage": "nvme" },
        })),
    );
    item.day = DAY;
    item
}

async fn send(app: &Arc<App>, req: Request<Body>) -> (StatusCode, Vec<u8>, Option<String>) {
    let res = router(app.clone()).oneshot(req).await.unwrap();
    let code = res.status();
    let encoding = res
        .headers()
        .get("content-encoding")
        .map(|v| v.to_str().unwrap().to_owned());
    (
        code,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
        encoding,
    )
}

async fn upload(app: &Arc<App>, id: u32, items: Vec<Item>) {
    let body = batch::encode(&Batch::new(&install(id), items));
    let req = Request::post("/v1/batch")
        .header("content-encoding", "zstd")
        .body(Body::from(body))
        .unwrap();
    assert_eq!(send(app, req).await.0, StatusCode::NO_CONTENT);
}

async fn get(app: &Arc<App>, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::get(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let (code, body, encoding) = send(app, req.body(Body::empty()).unwrap()).await;
    assert_eq!(encoding, None, "plain unless gzip is asked for");
    (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn it_is_off_without_a_token_and_refuses_a_wrong_one() {
    for uri in ["/v1/export/benchmarks", "/v1/export/hardware"] {
        let off = app(None);
        assert_eq!(get(&off, uri, None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&off, uri, Some(TOKEN)).await.0, StatusCode::NOT_FOUND);

        let on = app(Some(TOKEN));
        assert_eq!(get(&on, uri, None).await.0, StatusCode::UNAUTHORIZED);
        for wrong in [
            "",
            "a-long-random-export-toke",
            "a-long-random-export-tokenX",
            "nope",
        ] {
            assert_eq!(
                get(&on, uri, Some(wrong)).await.0,
                StatusCode::UNAUTHORIZED,
                "{wrong:?}"
            );
        }
        // The wrong scheme, even with the right secret.
        let req = Request::get(uri)
            .header("authorization", format!("Basic {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&on, req).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(get(&on, uri, Some(TOKEN)).await.0, StatusCode::OK);
    }
}

#[tokio::test]
async fn guessing_is_rate_limited_but_using_the_token_is_not() {
    let app = App::with_clock(
        Store::open(":memory:").unwrap(),
        Config {
            requests_per_hour: 10.0,
            export_token: Some(TOKEN.into()),
            ..Config::default()
        },
        Box::new(|| DAY),
    );
    let mut codes = Vec::new();
    for _ in 0..5 {
        codes.push(get(&app, "/v1/export/hardware", Some("guess")).await.0);
    }
    assert_eq!(codes[0], StatusCode::UNAUTHORIZED);
    assert_eq!(codes[4], StatusCode::TOO_MANY_REQUESTS);
    for _ in 0..20 {
        assert_eq!(
            get(&app, "/v1/export/hardware", Some(TOKEN)).await.0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn benchmark_reports_come_out_raw_and_in_pages() {
    let app = app(Some(TOKEN));
    for n in 0..5 {
        upload(&app, n + 1, vec![benchmark(n)]).await;
    }
    // A play session is not a benchmark.
    upload(&app, 9, vec![perf("AMD Ryzen 9 7950X", 32, 16, "nvme")]).await;

    let (code, first) = get(&app, "/v1/export/benchmarks?limit=2", Some(TOKEN)).await;
    assert_eq!(code, StatusCode::OK);
    let reports = first["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0]["report"]["hardware"]["cpu_threads"], 32);
    assert_eq!(reports[0]["report"]["hardware"]["cpu_cores"], 16);
    assert_eq!(
        (reports[0]["day"].as_u64(), reports[0]["version"].as_str()),
        (Some(DAY.into()), Some("v0.2.97"))
    );

    let mut seen = vec![
        reports[0]["report"]["n"].clone(),
        reports[1]["report"]["n"].clone(),
    ];
    let mut next = first["next"].clone();
    let mut pages = 1;
    while let Some(after) = next.as_i64() {
        let (_, page) = get(
            &app,
            &format!("/v1/export/benchmarks?limit=2&after={after}"),
            Some(TOKEN),
        )
        .await;
        seen.extend(
            page["reports"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["report"]["n"].clone()),
        );
        next = page["next"].clone();
        pages += 1;
    }
    assert_eq!(seen, [0, 1, 2, 3, 4].map(Value::from));
    assert_eq!(pages, 3);

    // Whose report it was never leaves the database.
    let (_, all) = get(&app, "/v1/export/benchmarks", Some(TOKEN)).await;
    assert_eq!(all["next"], Value::Null);
    assert!(!all.to_string().contains(&install(1)));
    assert!(!all.to_string().contains("install"));
}

#[tokio::test]
async fn machines_come_with_their_core_counts_and_no_minimum() {
    let app = app(Some(TOKEN));
    // One install on a 16-core / 32-thread machine, two on a 4-core / 8-thread one.
    upload(&app, 1, vec![perf("AMD Ryzen 9 7950X", 32, 16, "nvme")]).await;
    upload(&app, 2, vec![perf("Intel Core i5-8250U", 8, 4, "ssd")]).await;
    upload(&app, 3, vec![perf("Intel Core i5-8250U", 8, 4, "ssd")]).await;

    let (code, out) = get(&app, "/v1/export/hardware?days=30", Some(TOKEN)).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(out["total_machines"], 2);
    let machines = out["machines"].as_array().unwrap();
    // Busiest first.
    assert_eq!(machines[0]["cpu"], "Intel Core i5-8250U");
    assert_eq!(
        (
            machines[0]["threads"].as_u64(),
            machines[0]["cores"].as_u64()
        ),
        (Some(8), Some(4))
    );
    assert_eq!(
        (
            machines[0]["installs"].as_u64(),
            machines[0]["sessions"].as_u64()
        ),
        (Some(2), Some(2))
    );
    assert_eq!(machines[0]["storage"], "ssd");
    assert_eq!(machines[0]["hours"], 2.0);
    assert!(machines[0]["p99_ms"].as_f64().unwrap() > 0.0);
    assert_eq!(
        machines[0]["frame_buckets"].as_array().unwrap().len(),
        out["bucket_edges_ms"].as_array().unwrap().len() + 1
    );
    assert_eq!(machines[1]["cpu"], "AMD Ryzen 9 7950X");
    assert_eq!(
        (
            machines[1]["threads"].as_u64(),
            machines[1]["cores"].as_u64()
        ),
        (Some(32), Some(16))
    );
    assert_eq!(machines[1]["installs"], 1);
    assert!(!out.to_string().contains(&install(1)));

    // Pages by offset.
    let (_, page) = get(&app, "/v1/export/hardware?limit=1", Some(TOKEN)).await;
    assert_eq!(page["machines"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    let (_, page) = get(&app, "/v1/export/hardware?limit=1&offset=1", Some(TOKEN)).await;
    assert_eq!(page["machines"][0]["cpu"], "AMD Ryzen 9 7950X");
    assert_eq!(page["next_offset"], Value::Null);

    // Outside the window: nothing.
    let old = App::with_clock(
        Store::open(":memory:").unwrap(),
        Config {
            export_token: Some(TOKEN.into()),
            ..Config::default()
        },
        Box::new(|| DAY + 90),
    );
    let (_, none) = get(&old, "/v1/export/hardware?days=30", Some(TOKEN)).await;
    assert_eq!(none["total_machines"], 0);
}

#[tokio::test]
async fn gzip_when_asked() {
    let app = app(Some(TOKEN));
    upload(&app, 1, vec![benchmark(7)]).await;
    let req = Request::get("/v1/export/benchmarks")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("accept-encoding", "br, gzip;q=0.8")
        .body(Body::empty())
        .unwrap();
    let res = router(app.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-encoding"], "gzip");
    assert_eq!(res.headers()["content-type"], "application/json");
    assert_eq!(res.headers()["cache-control"], "no-store");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let mut json = String::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_string(&mut json)
        .unwrap();
    let out: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["reports"][0]["report"]["n"], 7);
}
