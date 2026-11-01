//! The hub's intake through its HTTP interface: votes and bug reports (opened as issues by a fake GitHub).
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use horizon_telemetry_server::github::{Filer, Issue};
use horizon_telemetry_server::store::Store;
use horizon_telemetry_server::{App, Config, router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const DAY: u32 = 20_741;
const VOTER: &str = "0b7c3c9e-3f0a-4d8e-9a39-4c1f5a6b7d21";

/// The issues the fake GitHub opened, and whether it is up.
#[derive(Default)]
struct GitHub {
    opened: Mutex<Vec<Issue>>,
    down: Mutex<bool>,
}

struct Hub {
    app: Arc<App>,
    github: Arc<GitHub>,
    day: Arc<AtomicU32>,
}

fn hub(config: Config) -> Hub {
    let github = Arc::new(GitHub::default());
    let fake = github.clone();
    let filer: Arc<Filer> = Arc::new(move |issue: &Issue| {
        if *fake.down.lock().unwrap() {
            return Err("GitHub answered 502".into());
        }
        let mut opened = fake.opened.lock().unwrap();
        opened.push(Issue {
            title: issue.title.clone(),
            body: issue.body.clone(),
            labels: issue.labels.clone(),
        });
        Ok(format!(
            "https://github.com/horizon-rust/horizon/issues/{}",
            1000 + opened.len()
        ))
    });
    let day = Arc::new(AtomicU32::new(DAY));
    let clock = day.clone();
    let app = App::with_filer(
        Store::open(":memory:").unwrap(),
        config,
        Box::new(move || clock.load(Ordering::SeqCst)),
        Some(filer),
    );
    Hub { app, github, day }
}

fn config() -> Config {
    Config {
        hub_posts_per_hour: 1000.0,
        ..Config::default()
    }
}

impl Hub {
    async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = router(self.app.clone()).oneshot(req).await.unwrap();
        let code = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }
}

fn report(title: &str) -> Value {
    json!({
        "title": title,
        "did": "Opened a new world\nWalked to the sea",
        "happened": "The water is black",
        "expected": "Blue water, as in vanilla",
        "version": "v0.2.57",
        "system": "Windows 11, Ryzen 5 5600, RX 6600",
        "website": "",
    })
}

#[tokio::test]
async fn votes_count_once_per_browser_and_can_be_taken_back() {
    let hub = hub(config());
    let vote = |up: bool, voter: &str| json!({ "item": "smooth-frames", "voter": voter, "up": up });
    assert_eq!(
        hub.post("/hub/vote", vote(true, VOTER)).await,
        (
            StatusCode::OK,
            json!({ "item": "smooth-frames", "votes": 1 })
        )
    );
    assert_eq!(
        hub.post("/hub/vote", vote(true, VOTER)).await.1["votes"],
        1,
        "a second vote changes nothing"
    );
    let other = "f".repeat(32);
    assert_eq!(
        hub.post("/hub/vote", vote(true, &other)).await.1["votes"],
        2
    );
    assert_eq!(
        hub.get("/hub/votes").await,
        (StatusCode::OK, json!({ "votes": { "smooth-frames": 2 } }))
    );
    assert_eq!(
        hub.post("/hub/vote", vote(false, VOTER)).await.1["votes"],
        1
    );
    assert_eq!(
        hub.get("/hub/votes").await.1["votes"]["smooth-frames"],
        1,
        "a vote drops the cached counts"
    );
    for bad in [
        json!({ "item": "Not An Id", "voter": VOTER }),
        json!({ "item": "a", "voter": "short" }),
        json!([]),
    ] {
        assert_eq!(hub.post("/hub/vote", bad).await.0, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn a_report_becomes_an_issue_in_the_bug_template() {
    let hub = hub(config());
    let (code, body) = hub
        .post("/hub/report", report("Water is black on AMD"))
        .await;
    assert_eq!(code, StatusCode::CREATED);
    assert_eq!(
        body["issue"],
        "https://github.com/horizon-rust/horizon/issues/1001"
    );
    let issue = hub.github.opened.lock().unwrap().remove(0);
    assert_eq!(issue.title, "Water is black on AMD");
    assert_eq!(issue.labels, ["bug", "from-website"]);
    for line in [
        "> **What I did:**\n> Opened a new world\n> Walked to the sea",
        "> **What happened:**\n> The water is black",
        "> **What I expected:**\n> Blue water, as in vanilla",
        "> **Horizon version:** v0.2.57",
        "> **OS, CPU, GPU:** Windows 11, Ryzen 5 5600, RX 6600",
        "Reported on playhorizon.dev",
    ] {
        assert!(issue.body.contains(line), "{line}\n---\n{}", issue.body);
    }
    // The same report again (a double click) answers the same issue and opens nothing.
    let (code, again) = hub
        .post("/hub/report", report("Water is black on AMD"))
        .await;
    assert_eq!(
        (code, &again["issue"]),
        (StatusCode::CREATED, &body["issue"])
    );
    assert!(hub.github.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reports_need_every_template_field_and_skip_bots() {
    let hub = hub(config());
    for empty in ["title", "did", "happened", "expected", "system"] {
        let mut body = report("Water is black on AMD");
        body[empty] = json!("   ");
        let (code, answer) = hub.post("/hub/report", body).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{empty}");
        assert!(answer["error"].as_str().unwrap().starts_with(empty));
    }
    let mut bot = report("Cheap watches");
    bot["website"] = json!("https://spam.example");
    assert_eq!(
        hub.post("/hub/report", bot).await,
        (StatusCode::ACCEPTED, json!({ "id": 0 }))
    );
    assert!(hub.github.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_report_github_refused_is_opened_by_the_upkeep() {
    let hub = hub(config());
    *hub.github.down.lock().unwrap() = true;
    let (code, body) = hub
        .post("/hub/report", report("Crash when opening a chest"))
        .await;
    assert_eq!((code, &body["issue"]), (StatusCode::ACCEPTED, &Value::Null));
    hub.app.file_pending();
    assert_eq!(hub.github.opened.lock().unwrap().len(), 0);
    *hub.github.down.lock().unwrap() = false;
    hub.app.file_pending();
    assert_eq!(hub.github.opened.lock().unwrap().len(), 1);
    hub.app.file_pending();
    assert_eq!(hub.github.opened.lock().unwrap().len(), 1, "opened once");
}

#[tokio::test]
async fn reports_are_closed_without_a_github_token() {
    let app = App::with_clock(
        Store::open(":memory:").unwrap(),
        Config::default(),
        Box::new(|| DAY),
    );
    let res = router(app)
        .oneshot(
            Request::post("/hub/report")
                .header("content-type", "application/json")
                .body(Body::from(report("Water is black").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn reports_are_rate_limited_per_address_and_capped_per_day() {
    let hub = hub(Config {
        hub_posts_per_hour: 20.0,
        hub_reports_per_day: 3,
        ..config()
    });
    // A burst of a tenth of the hourly rate: two, then refused.
    let bug = |n: u32| report(&format!("Bug number {n}"));
    assert_eq!(hub.post("/hub/report", bug(1)).await.0, StatusCode::CREATED);
    assert_eq!(hub.post("/hub/report", bug(2)).await.0, StatusCode::CREATED);
    assert_eq!(
        hub.post("/hub/report", bug(3)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    let busy = self::hub(Config {
        hub_posts_per_hour: 1000.0,
        hub_reports_per_day: 3,
        ..config()
    });
    for n in 0..3 {
        assert_eq!(
            busy.post("/hub/report", bug(n)).await.0,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        busy.post("/hub/report", bug(9)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    busy.day.store(DAY + 1, Ordering::SeqCst);
    assert_eq!(
        busy.post("/hub/report", bug(9)).await.0,
        StatusCode::CREATED,
        "a new day"
    );
}

#[tokio::test]
async fn old_entries_go_and_reports_never_opened_are_given_up() {
    let hub = hub(config());
    *hub.github.down.lock().unwrap() = true;
    let (_, body) = hub
        .post("/hub/report", report("Crash when opening a chest"))
        .await;
    assert!(body["id"].as_i64().unwrap() > 0);
    hub.day.store(DAY + 8, Ordering::SeqCst);
    hub.app.maintain().unwrap();
    *hub.github.down.lock().unwrap() = false;
    hub.app.file_pending();
    assert!(
        hub.github.opened.lock().unwrap().is_empty(),
        "past a week a report is given up"
    );
}

#[test]
fn entries_past_the_keep_days_are_removed() {
    let mut store = Store::open(":memory:").unwrap();
    let fields = report("Crash when opening a chest")
        .as_object()
        .unwrap()
        .clone();
    store.enqueue("report", DAY, DAY - 1, &fields).unwrap();
    // Looked for duplicates from DAY + 1 only: a second entry, not the first one again.
    store.enqueue("report", DAY + 1, DAY + 1, &fields).unwrap();
    assert_eq!(
        store.expire_queue(DAY + 1, DAY).unwrap(),
        1,
        "the older day goes"
    );
    assert_eq!(store.queued_on("report", DAY).unwrap(), 0);
    assert_eq!(store.queued_on("report", DAY + 1).unwrap(), 1);
}

#[tokio::test]
async fn the_website_may_post_from_the_browser() {
    let hub = hub(config());
    let res = router(hub.app.clone())
        .oneshot(
            Request::options("/hub/report")
                .header("origin", "https://playhorizon.dev")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let header = |name: &str| res.headers()[name].to_str().unwrap().to_owned();
    assert_eq!(
        header("access-control-allow-origin"),
        "https://playhorizon.dev"
    );
    assert_eq!(header("access-control-allow-methods"), "POST");
    assert_eq!(header("access-control-allow-headers"), "content-type");
}
