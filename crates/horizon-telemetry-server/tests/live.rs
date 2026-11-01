//! The live count through its HTTP interface, and end to end with the game's ping and the
//! launcher's read (`horizon_telemetry::live`).
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use horizon_telemetry::live::{self, Count, Ping};
use horizon_telemetry_server::history::{Sample, Series};
use horizon_telemetry_server::store::Store;
use horizon_telemetry_server::{App, Config, router};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app(config: Config) -> Arc<App> {
    App::new(Store::open(":memory:").unwrap(), config)
}

fn quick() -> Config {
    Config {
        official_server: None,
        live_per_hour: 100_000.0,
        live_window: Duration::from_secs(1),
        trust_proxy: true,
        ..Config::default()
    }
}

fn ping(n: u32, version: &str, lan: bool) -> String {
    serde_json::to_string(&Ping {
        session: format!("{n:032x}"),
        version: version.into(),
        platform: "linux".into(),
        lan,
    })
    .unwrap()
}

async fn send(app: &Arc<App>, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = router(app.clone()).oneshot(req).await.unwrap();
    let (code, headers) = (res.status(), res.headers().clone());
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (code, headers, String::from_utf8_lossy(&body).into_owned())
}

async fn post(app: &Arc<App>, body: impl Into<Body>, from: &str) -> StatusCode {
    let req = Request::post("/v1/live")
        .header("x-forwarded-for", from)
        .body(body.into())
        .unwrap();
    send(app, req).await.0
}

async fn count(app: &Arc<App>) -> Count {
    let req = Request::get("/v1/live")
        .header("x-forwarded-for", "198.51.100.1")
        .body(Body::empty())
        .unwrap();
    let (code, headers, body) = send(app, req).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://playhorizon.dev"
    );
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn sessions_count_while_they_ping_and_expire_after() {
    let app = app(quick());
    assert_eq!(
        post(&app, ping(1, "0.2.0", false), "203.0.113.1").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(&app, ping(2, "0.2.0", true), "203.0.113.2").await,
        StatusCode::NO_CONTENT
    );
    // A second ping from the same session is still one player.
    assert_eq!(
        post(&app, ping(1, "0.2.0", false), "203.0.113.1").await,
        StatusCode::NO_CONTENT
    );
    let now = count(&app).await;
    assert_eq!((now.playing, now.lan), (2, 1));
    assert_eq!(now.versions["0.2.0"], 2);
    assert_eq!(now.platforms["linux"], 2);

    // Session 1 keeps pinging; session 2 stops.
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        post(&app, ping(1, "0.2.0", false), "203.0.113.1").await;
    }
    let later = count(&app).await;
    assert_eq!((later.playing, later.lan), (1, 0));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(count(&app).await, Count::default());
}

#[tokio::test]
async fn bad_pings_are_refused() {
    let app = app(quick());
    let from = "203.0.113.3";
    assert_eq!(post(&app, "not json", from).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        post(&app, ping(1, "<b>0.2</b>", false), from).await,
        StatusCode::BAD_REQUEST
    );
    let extra = r#"{"session":"00000000000000000000000000000001","version":"0.2.0","platform":"linux","lan":false,"name":"Steve"}"#;
    assert_eq!(post(&app, extra, from).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        post(&app, "x".repeat(4096), from).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(count(&app).await.playing, 0);
}

#[tokio::test]
async fn an_address_is_rate_limited_on_its_own_bucket() {
    let app = app(Config {
        live_per_hour: 60.0,
        trust_proxy: true,
        ..Config::default()
    });
    // A burst of a tenth of the hour's 60, then 429.
    let mut codes = Vec::new();
    for n in 0..8 {
        codes.push(post(&app, ping(n, "0.2.0", false), "203.0.113.4").await);
    }
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == StatusCode::NO_CONTENT)
            .count(),
        6
    );
    assert_eq!(codes[7], StatusCode::TOO_MANY_REQUESTS);
    // Another address is not held back.
    assert_eq!(
        post(&app, ping(9, "0.2.0", false), "203.0.113.5").await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn a_full_table_refuses_new_sessions() {
    let app = app(Config {
        live_max_sessions: 2,
        ..quick()
    });
    for n in 0..2 {
        assert_eq!(
            post(&app, ping(n, "0.2.0", false), "203.0.113.6").await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        post(&app, ping(2, "0.2.0", false), "203.0.113.6").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        post(&app, ping(1, "0.2.0", false), "203.0.113.6").await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn the_games_ping_and_the_launchers_read_reach_it_end_to_end() {
    let app = app(Config {
        live_window: Duration::from_secs(120),
        ..quick()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let service = router(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let count = tokio::task::spawn_blocking(move || {
        let a = Ping::new(&live::session_id(), "0.2.0", false);
        let b = Ping::new(&live::session_id(), "dev 0.1.0", true);
        live::send(&url, &a)?;
        live::send(&url, &b)?;
        live::fetch(&url)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!((count.playing, count.lan), (2, 1));
    assert_eq!(count.versions.len(), 2);
    assert_eq!(count.platforms[live::platform()], 2);
}

async fn history(app: &Arc<App>, range: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let req = Request::get(format!("/v1/live/history?{range}"))
        .header("x-forwarded-for", "198.51.100.2")
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn the_history_serves_the_recorded_minutes_and_peaks() {
    let app = app(quick());
    let now = unix_now();
    let minute = now - now % 60;
    let at = |playing: u64, linux: u64| Sample {
        playing,
        lan: 1,
        platforms: [linux, 0, playing - linux, 0],
    };
    // An old peak, three days ago, then today's minutes.
    assert!(
        app.record_sample(Series::Players, minute - 3 * 86_400, &at(1234, 4))
            .unwrap()
    );
    assert!(
        app.record_sample(Series::Players, minute - 120, &at(7, 2))
            .unwrap()
    );
    assert!(
        app.record_sample(Series::Players, minute - 60, &at(9, 3))
            .unwrap()
    );
    assert!(
        !app.record_sample(Series::Players, minute - 30, &at(50, 0))
            .unwrap()
    );

    let (code, headers, body) = history(&app, "range=24h").await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://playhorizon.dev"
    );
    let day: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(day["range"], "24h");
    assert_eq!(day["step"], 300);
    assert_eq!(day["peak"]["playing"], 9);
    assert_eq!(day["peak"]["at"], minute - 60);
    assert_eq!(day["all_time"]["playing"], 1234);
    assert_eq!(day["all_time"]["at"], minute - 3 * 86_400);
    let points = day["series"].as_array().unwrap();
    let total: u64 = points.iter().map(|p| p[2].as_u64().unwrap()).max().unwrap();
    assert_eq!(total, 9);
    assert_eq!(day["platforms"]["linux"], 2.5);
    assert_eq!(day["platforms"]["windows"], 5.5);

    let (_, _, body) = history(&app, "range=7d").await;
    let week: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(week["step"], 3600);
    assert_eq!(week["peak"]["playing"], 1234);
    let (_, _, body) = history(&app, "range=all").await;
    let all: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(all["step"], 86_400);
    assert!(!all["series"].as_array().unwrap().is_empty());

    assert_eq!(history(&app, "range=1y").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_recorder_waits_a_window_then_writes_the_exact_count() {
    let app = app(Config {
        live_window: Duration::from_millis(400),
        ..quick()
    });
    // Just started: games haven't all pinged yet.
    assert!(!app.record_live().unwrap());
    tokio::time::sleep(Duration::from_millis(450)).await;
    for n in 0..3 {
        post(&app, ping(n, "0.2.0", n == 0), "203.0.113.7").await;
    }
    assert!(app.record_live().unwrap());
    let (_, _, body) = history(&app, "range=24h").await;
    let day: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(day["peak"]["playing"], 3);
    assert_eq!(day["platforms"]["linux"], 3.0);
}

#[tokio::test]
async fn the_official_servers_count_is_pinged_and_served_apart() {
    // A stand-in official server answering the status ping with 4 players.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = [0; 256];
        let _ = socket.read(&mut buf).unwrap();
        let json = br#"{"players":{"online":4,"max":50}}"#;
        let mut packet = vec![0x00, json.len() as u8];
        packet.extend_from_slice(json);
        let mut out = vec![packet.len() as u8];
        out.extend(packet);
        socket.write_all(&out).unwrap();
    });
    let app = app(Config {
        official_server: Some(address),
        ..quick()
    });
    let recorded = tokio::task::spawn_blocking({
        let app = app.clone();
        move || app.record_official()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(recorded);
    let (code, _, body) = history(&app, "range=24h&series=official").await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let day: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(day["source"], "official");
    assert_eq!(day["peak"]["playing"], 4);
    // The players' series has nothing: no game pinged.
    let (_, _, body) = history(&app, "range=24h").await;
    let players: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(players["source"], "players");
    assert!(players["peak"].is_null());
    assert_eq!(history(&app, "series=lan").await.0, StatusCode::BAD_REQUEST);
    // Nobody at the address: nothing is written.
    let gone = app_with_official("127.0.0.1:1");
    assert!(
        !tokio::task::spawn_blocking(move || gone.record_official().unwrap())
            .await
            .unwrap()
    );
}

fn app_with_official(address: &str) -> Arc<App> {
    app(Config {
        official_server: Some(address.into()),
        ..quick()
    })
}
