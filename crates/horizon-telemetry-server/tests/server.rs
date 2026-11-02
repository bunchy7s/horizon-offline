//! The server through its HTTP interface, and end to end with the launcher's uploader.
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use horizon_telemetry::batch::{self, Batch};
use horizon_telemetry::payload::{
    Crash, Payload, Perf, Settings, SlowFrames, Source, System, Timings,
};
use horizon_telemetry::{Item, outbox};
use horizon_telemetry_server::store::{self, Store};
use horizon_telemetry_server::{App, Config, router};
use http_body_util::BodyExt;
use tower::ServiceExt;

const DAY: u32 = 20_741;

fn install(n: u32) -> String {
    format!("{n:032x}")
}

fn perf(gpu: &str, distance: u8) -> Item {
    let mut t = Timings::new();
    (0..30_000).for_each(|i| t.record(8.0 + (i % 50) as f32 / 10.0));
    let mut item = Item::new(
        Source::Game,
        "v2026.10.15",
        Payload::Perf(Perf {
            system: System {
                os: "linux".into(),
                os_version: "Fedora Linux 43".into(),
                cpu: "AMD Ryzen 9 7950X".into(),
                threads: 32,
                ram_gb: 32,
                gpu: gpu.into(),
                driver: "570.86".into(),
                backend: "Vulkan".into(),
                ..System::default()
            },
            settings: Settings {
                render_distance: distance,
                simulation_distance: 12,
                graphics: "fancy".into(),
                ..Settings::default()
            },
            seconds: 1800,
            frames: t.summary(),
            peak_ram_mb: 2300,
            ..Perf::default()
        }),
    );
    item.day = DAY;
    item
}

struct Server {
    app: Arc<App>,
    day: Arc<AtomicU32>,
}

fn server(config: Config) -> Server {
    let day = Arc::new(AtomicU32::new(DAY));
    let clock = day.clone();
    let app = App::with_clock(
        Store::open(":memory:").unwrap(),
        config,
        Box::new(move || clock.load(Ordering::SeqCst)),
    );
    Server { app, day }
}

impl Server {
    async fn send(&self, req: Request<Body>) -> (StatusCode, String) {
        let res = router(self.app.clone()).oneshot(req).await.unwrap();
        let code = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (code, String::from_utf8_lossy(&body).into_owned())
    }

    async fn post(&self, id: &str, items: Vec<Item>) -> StatusCode {
        let body = batch::encode(&Batch::new(id, items));
        self.post_raw(body, Some("zstd")).await
    }

    async fn post_raw(&self, body: Vec<u8>, encoding: Option<&str>) -> StatusCode {
        let mut req = Request::post("/v1/batch").header("x-forwarded-for", "203.0.113.9");
        if let Some(e) = encoding {
            req = req.header("content-encoding", e);
        }
        self.send(req.body(Body::from(body)).unwrap()).await.0
    }

    async fn aggregate(&self) -> serde_json::Value {
        let (code, body) = self
            .send(
                Request::get("/v1/aggregate?days=30")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(code, StatusCode::OK);
        serde_json::from_str(&body).unwrap()
    }
}

fn generous() -> Config {
    Config {
        requests_per_hour: 100_000.0,
        ..Config::default()
    }
}

#[tokio::test]
async fn totals_show_only_groups_with_enough_installs() {
    let s = server(generous());
    for n in 0..6 {
        assert_eq!(
            s.post(&install(n), vec![perf("NVIDIA GeForce RTX 4070", 12); 2])
                .await,
            StatusCode::NO_CONTENT
        );
    }
    for n in 10..12 {
        assert_eq!(
            s.post(&install(n), vec![perf("Intel(R) Arc A770", 8)])
                .await,
            StatusCode::NO_CONTENT
        );
    }
    let totals = s.aggregate().await;
    let groups = totals["groups"].as_array().unwrap();
    assert_eq!(
        groups.len(),
        1,
        "two installs are too few to show: {totals}"
    );
    let g = &groups[0];
    assert_eq!(g["gpu_vendor"], "nvidia");
    assert_eq!(
        (g["sessions"].as_u64(), g["install_days"].as_u64()),
        (Some(12), Some(6))
    );
    assert_eq!(g["hours"], 6.0);
    assert_eq!(
        g["frame_buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap())
            .sum::<u64>(),
        12 * 30_000
    );
    assert_eq!(totals["bucket_edges_ms"].as_array().unwrap().len(), 13);
    let text = totals.to_string();
    assert!(
        !text.contains(&install(0)) && !text.contains("Ryzen"),
        "no install or machine in the totals"
    );
}

/// Sessions split by hardware class; a class shows only with three installs, and never a raw value.
#[tokio::test]
async fn totals_break_down_by_hardware_class() {
    let s = server(Config {
        min_install_days: 1,
        ..generous()
    });
    for n in 0..4 {
        let mut item = perf("NVIDIA GeForce RTX 4080", 12);
        if let Payload::Perf(p) = &mut item.payload {
            p.system.ram_gb = if n == 3 { 16 } else { 31 };
            p.system.vram_mb = 16376;
            p.system.gpu_kind = "discrete".into();
            p.system.storage = if n == 3 { "hdd" } else { "nvme" }.into();
        }
        assert_eq!(
            s.post(&install(n), vec![item]).await,
            StatusCode::NO_CONTENT
        );
    }
    let totals = s.aggregate().await;
    let b = &totals["breakdowns"];
    assert_eq!(
        b["ram"],
        serde_json::json!([{"class": "32 GB", "installs": 3, "sessions": 3, "hours": 1.5, "p50_ms": b["ram"][0]["p50_ms"], "p99_ms": b["ram"][0]["p99_ms"]}]),
        "16 GB has one install: {totals}"
    );
    assert!(b["ram"][0]["p50_ms"].as_f64().unwrap() > 8.0);
    assert_eq!(b["vram"][0]["class"], "16 GB");
    assert_eq!(b["vram"][0]["installs"], 4);
    assert_eq!(b["storage"].as_array().unwrap().len(), 1);
    assert_eq!(b["os"][0]["class"], "linux");
    assert_eq!(b["gpu_kind"][0]["class"], "discrete");
    let text = totals.to_string();
    for raw in ["16376", "570.86", "Fedora", "hdd", &install(0)] {
        assert!(!text.contains(raw), "{raw} in {text}");
    }
}

/// What froze: slow frames by release and step, summed over sessions; a step shows only once
/// enough installs had it, and never with an install or a machine.
#[tokio::test]
async fn slow_frames_show_what_froze_with_enough_installs() {
    let s = server(Config {
        min_install_days: 2,
        ..generous()
    });
    let slow = |span: &str, frames, frame_max_ms, span_max_ms| {
        let mut item = perf("NVIDIA GeForce RTX 4070", 12);
        if let Payload::Perf(p) = &mut item.payload {
            p.slow_frames = vec![SlowFrames {
                span: span.into(),
                frames,
                frame_max_ms,
                span_max_ms,
            }];
        }
        item
    };
    for (n, frame_max_ms) in [(0, 900.0), (1, 400.0)] {
        let item = slow("renderer/r_uploads", 3, frame_max_ms, 300.0);
        assert_eq!(
            s.post(&install(n), vec![item]).await,
            StatusCode::NO_CONTENT
        );
    }
    let lone = slow("tick/tick_mobs", 1, 260.0, 200.0);
    assert_eq!(
        s.post(&install(2), vec![lone]).await,
        StatusCode::NO_CONTENT
    );
    let quiet = perf("NVIDIA GeForce RTX 4070", 12);
    assert_eq!(
        s.post(&install(3), vec![quiet]).await,
        StatusCode::NO_CONTENT
    );
    let totals = s.aggregate().await;
    assert_eq!(
        totals["slow_frames"],
        serde_json::json!([{
            "version": "v2026.10.15", "span": "renderer/r_uploads", "frames": 6, "sessions": 2,
            "installs": 2, "frame_max_ms": 900.0, "span_max_ms": 300.0,
        }]),
        "one install is too few for tick_mobs: {totals}"
    );
    assert!(!totals.to_string().contains(&install(0)));
}

#[tokio::test]
async fn bad_batches_are_refused() {
    let s = server(generous());
    let good = batch::encode(&Batch::new(&install(1), vec![perf("x", 8)]));
    assert_eq!(
        s.post_raw(good.clone(), None).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        s.post_raw(b"garbage".to_vec(), Some("zstd")).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        s.post_raw(vec![0; batch::MAX_COMPRESSED + 1], Some("zstd"))
            .await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        s.post("not-an-id", vec![perf("x", 8)]).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(s.post_raw(good, Some("zstd")).await, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn an_address_is_rate_limited() {
    let s = server(Config {
        requests_per_hour: 30.0,
        trust_proxy: true,
        ..Config::default()
    });
    for n in 0..3 {
        assert_eq!(
            s.post(&install(n), vec![perf("x", 8)]).await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        s.post(&install(9), vec![perf("x", 8)]).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    let from = |forwarded: &str, real: Option<&str>| {
        let body = batch::encode(&Batch::new(&install(9), vec![perf("x", 8)]));
        let mut req = Request::post("/v1/batch")
            .header("x-forwarded-for", forwarded)
            .header("content-encoding", "zstd");
        if let Some(ip) = real {
            req = req.header("x-real-ip", ip);
        }
        req.body(Body::from(body)).unwrap()
    };
    // A client can't pick its address: the proxy adds the real one last, or sets X-Real-IP.
    assert_eq!(
        s.send(from("198.51.100.7, 203.0.113.9", None)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        s.send(from("198.51.100.7", Some("203.0.113.9"))).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another address has its own allowance.
    assert_eq!(
        s.send(from("203.0.113.9, 198.51.100.7", None)).await.0,
        StatusCode::NO_CONTENT
    );
    // Two IPv6 addresses of one /64 share a bucket; another /64 doesn't.
    for forwarded in ["2001:db8::1", "2001:db8::2", "2001:db8::3"] {
        assert_eq!(
            s.send(from(forwarded, None)).await.0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        s.send(from("2001:db8::ffff:ffff", None)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        s.send(from("2001:db8:1::1", None)).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn an_install_has_a_daily_cap() {
    let s = server(Config {
        items_per_day: 3,
        ..generous()
    });
    assert_eq!(
        s.post(&install(1), vec![perf("x", 8); 2]).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        s.post(&install(1), vec![perf("x", 8); 2]).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        s.post(&install(1), vec![perf("x", 8)]).await,
        StatusCode::NO_CONTENT
    );
    s.day.store(DAY + 1, Ordering::SeqCst);
    assert_eq!(
        s.post(&install(1), vec![perf("x", 8); 3]).await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn deleting_an_install_removes_its_rows_and_its_share_of_the_totals() {
    let s = server(Config {
        min_install_days: 1,
        ..generous()
    });
    let crash = Item::new(Source::Game, "v1", Payload::Crash(Crash::default()));
    assert_eq!(
        s.post(&install(1), vec![perf("Apple M4 Pro", 12), crash])
            .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        s.post(&install(2), vec![perf("Apple M4 Pro", 12)]).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(s.aggregate().await["groups"][0]["sessions"], 2);
    let (code, _) = s
        .send(
            Request::delete(format!("/v1/installs/{}", install(1)))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    let key = store::install_key(&install(1)).unwrap();
    assert_eq!(s.app.newest_day(&key), None);
    let totals = s.aggregate().await;
    assert_eq!(
        totals["groups"][0]["sessions"], 1,
        "the cache was dropped and the day rebuilt"
    );
    assert_eq!(totals["crashes_by_version"].as_array().unwrap().len(), 0);
    let (code, _) = s
        .send(
            Request::delete("/v1/installs/nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn raw_rows_expire_and_the_totals_stay() {
    let s = server(Config {
        min_install_days: 1,
        retention_days: 180,
        ..generous()
    });
    assert_eq!(
        s.post(&install(1), vec![perf("Apple M4 Pro", 12)]).await,
        StatusCode::NO_CONTENT
    );
    s.day.store(DAY + 179, Ordering::SeqCst);
    assert_eq!(s.app.maintain().unwrap(), 0, "still within retention");
    s.day.store(DAY + 181, Ordering::SeqCst);
    assert_eq!(s.app.maintain().unwrap(), 1);
    assert_eq!(
        s.app.newest_day(&store::install_key(&install(1)).unwrap()),
        None
    );
    let (_, body) = s
        .send(
            Request::get("/v1/aggregate?days=366")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let totals: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(totals["groups"][0]["sessions"], 1);
}

#[test]
fn a_session_costs_little_storage() {
    let dir = std::env::temp_dir().join(format!(
        "horizon-telemetry-server-size-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sqlite3");
    let mut store = Store::open(path.to_str().unwrap()).unwrap();
    let gpus = [
        "Apple M4 Pro",
        "NVIDIA GeForce RTX 4070",
        "AMD Radeon RX 7800 XT",
        "Intel(R) Arc A770",
    ];
    let sessions = 10_000;
    for n in 0..sessions / 10 {
        let items = (0..10)
            .map(|i| perf(gpus[(n + i) as usize % 4], 8 + (i % 3) as u8 * 4))
            .collect();
        store
            .insert(&Batch::new(&install(n), items), DAY + n % 30)
            .unwrap();
    }
    store.rebuild_dirty().unwrap();
    drop(store);
    let bytes: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.metadata().unwrap().len())
        .sum();
    let per = bytes / u64::from(sessions);
    eprintln!("{sessions} sessions: {bytes} bytes on disk, {per} bytes a session");
    assert!(per < 250, "{per} bytes a session");
}

#[tokio::test]
async fn the_launchers_uploader_reaches_it_end_to_end() {
    let s = server(Config {
        min_install_days: 1,
        ..generous()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let service =
        router(s.app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let home = std::env::temp_dir().join(format!(
        "horizon-telemetry-server-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    let mut consent = horizon_telemetry::Consent::load(&home);
    consent.answer(horizon_telemetry::Choices::ALL);
    consent.save(&home).unwrap();
    for _ in 0..3 {
        assert!(outbox::push(&home, &perf("Apple M4 Pro", 12)).unwrap());
    }
    let (h, u) = (home.clone(), url.clone());
    let sent = tokio::task::spawn_blocking(move || horizon_telemetry::upload::flush(&h, &u))
        .await
        .unwrap();
    assert_eq!(
        sent,
        horizon_telemetry::upload::Flush::Sent {
            items: 3,
            batches: 1
        }
    );
    assert_eq!(s.aggregate().await["groups"][0]["sessions"], 3);

    let id = consent.install_id.clone();
    let deleted =
        tokio::task::spawn_blocking(move || horizon_telemetry::upload::delete_remote(&url, &id))
            .await
            .unwrap();
    assert_eq!(deleted, Ok(()));
    assert_eq!(
        s.app
            .newest_day(&store::install_key(&consent.install_id).unwrap()),
        None
    );
}
