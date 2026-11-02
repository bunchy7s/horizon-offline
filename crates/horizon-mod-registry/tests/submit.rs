//! The whole submission against a mock GitHub: sign in, submit the sample
//! signed mod (`crates/horizon-mods/tests/index`), and what lands on GitHub;
//! then what is refused. Then ratings: who may rate what, and what is shown.
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path as UrlPath, Query, State},
    http::{Request, StatusCode, header},
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use horizon_mod_registry::{App, Config, github, router};
use horizon_mods::index::registry::Entry;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const PUBLIC: &str = "https://playhorizon.test";
const HMOD_SHA: &str = "e1135345692f6dd8fd4c51920151944b82333e17049c9bb7d756f27b8deefc53";

#[derive(Default)]
struct Mock {
    /// Path -> text on main.
    main: HashMap<String, String>,
    assets: Vec<(String, usize)>,
    branches: Vec<String>,
    /// (branch, path) -> (text, message, replaced blob).
    commits: HashMap<(String, String), (String, String, Option<String>)>,
    pulls: Vec<Value>,
    /// (pull request number, labels added).
    labels: Vec<(u64, Value)>,
    /// The account the next sign-in gets.
    login: String,
    /// GitHub takes this long to find the staging release.
    slow: Option<std::time::Duration>,
    /// GitHub takes this long to open a pull request, and then doesn't.
    slow_pull: Option<std::time::Duration>,
    /// Branches moved back to main.
    resets: Vec<String>,
}

type Shared = Arc<Mutex<Mock>>;

async fn mock_github() -> (String, Shared) {
    let state: Shared = Arc::default();
    state.lock().unwrap().login = "alice".into();
    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(|Json(body): Json<Value>| async move {
                Json(if body["code"] == "good" {
                    json!({"access_token": "gho_user"})
                } else {
                    json!({"error": "bad_verification_code"})
                })
            }),
        )
        .route(
            "/user",
            get(|State(s): State<Shared>| async move {
                let login = s.lock().unwrap().login.clone();
                let id = if login == "alice" { 7 } else { 8 };
                Json(json!({"login": login, "id": id}))
            }),
        )
        .route(
            "/repos/o/r/contents/mods/{file}",
            get(
                |State(s): State<Shared>, UrlPath(file): UrlPath<String>| async move {
                    let s = s.lock().unwrap();
                    match s.main.get(&format!("mods/{file}")) {
                        Some(text) => Ok(Json(json!({"content": STANDARD.encode(text), "sha": "blob1"}))),
                        None => Err(StatusCode::NOT_FOUND),
                    }
                },
            )
            .put(
                |State(s): State<Shared>, UrlPath(file): UrlPath<String>, Json(b): Json<Value>| async move {
                    let text = String::from_utf8(STANDARD.decode(b["content"].as_str().unwrap()).unwrap()).unwrap();
                    let key = (b["branch"].as_str().unwrap().to_owned(), format!("mods/{file}"));
                    let value = (text, b["message"].as_str().unwrap().to_owned(), b["sha"].as_str().map(str::to_owned));
                    s.lock().unwrap().commits.insert(key, value);
                    Json(json!({}))
                },
            ),
        )
        .route(
            "/repos/alice/coords",
            get(|| async { Json(json!({"private": false, "owner": {"login": "alice"}})) }),
        )
        .route(
            "/repos/alice/secret",
            get(|| async { Json(json!({"private": true, "owner": {"login": "alice"}})) }),
        )
        .route(
            "/repos/o/r/releases/tags/staging",
            get(|State(s): State<Shared>| async move {
                let slow = s.lock().unwrap().slow;
                if let Some(slow) = slow {
                    tokio::time::sleep(slow).await;
                }
                let names: Vec<Value> = s.lock().unwrap().assets.iter().map(|(n, _)| json!({"name": n})).collect();
                Json(json!({"id": 42, "assets": names}))
            }),
        )
        .route(
            "/repos/o/r/releases/42/assets",
            post(
                |State(s): State<Shared>, Query(q): Query<HashMap<String, String>>, body: Bytes| async move {
                    s.lock().unwrap().assets.push((q["name"].clone(), body.len()));
                    Json(json!({}))
                },
            ),
        )
        .route(
            "/repos/o/r/git/ref/heads/main",
            get(|| async { Json(json!({"object": {"sha": "abc123"}})) }),
        )
        .route(
            "/repos/o/r/git/refs",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                assert_eq!(b["sha"], "abc123");
                let name = b["ref"].as_str().unwrap().trim_start_matches("refs/heads/").to_owned();
                let mut s = s.lock().unwrap();
                if s.branches.contains(&name) {
                    return Err(StatusCode::UNPROCESSABLE_ENTITY);
                }
                s.branches.push(name);
                Ok(Json(json!({})))
            }),
        )
        .route(
            "/repos/o/r/git/refs/heads/{*name}",
            patch(|State(s): State<Shared>, UrlPath(name): UrlPath<String>, Json(b): Json<Value>| async move {
                assert_eq!((&b["sha"], &b["force"]), (&json!("abc123"), &json!(true)));
                let mut s = s.lock().unwrap();
                s.commits.retain(|(branch, _), _| *branch != name);
                s.resets.push(name);
                Json(json!({}))
            }),
        )
        .route(
            "/repos/o/r/compare/{*spec}",
            get(|State(s): State<Shared>, UrlPath(spec): UrlPath<String>| async move {
                let branch = spec.strip_prefix("main...").unwrap();
                let s = s.lock().unwrap();
                let commits: Vec<Value> = s
                    .commits
                    .iter()
                    .filter(|((b, _), _)| b == branch)
                    .map(|(_, (_, message, _))| json!({"commit": {"message": message}}))
                    .collect();
                Json(json!({"ahead_by": commits.len(), "commits": commits}))
            }),
        )
        .route(
            "/repos/o/r/pulls",
            get(|State(s): State<Shared>, Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q["state"], "open");
                let s = s.lock().unwrap();
                let open: Vec<Value> = s
                    .pulls
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| format!("o:{}", p["head"].as_str().unwrap()) == q["head"] && p["state"] != "closed")
                    .map(|(i, _)| json!({"html_url": format!("https://github.com/o/r/pull/{}", i + 1)}))
                    .collect();
                Json(json!(open))
            })
            .post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                let slow = s.lock().unwrap().slow_pull;
                if let Some(slow) = slow {
                    tokio::time::sleep(slow).await;
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                let mut s = s.lock().unwrap();
                s.pulls.push(b);
                let n = s.pulls.len();
                Ok(Json(json!({"html_url": format!("https://github.com/o/r/pull/{n}"), "number": n})))
            }),
        )
        .route(
            "/repos/o/r/issues/{n}/labels",
            post(|State(s): State<Shared>, UrlPath(n): UrlPath<u64>, Json(b): Json<Value>| async move {
                s.lock().unwrap().labels.push((n, b["labels"].clone()));
                Json(json!([]))
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, state)
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../horizon-mods/tests/index")
}

struct Client {
    app: Router,
    cookies: HashMap<String, String>,
}

impl Client {
    async fn send(&mut self, request: Request<Body>) -> (StatusCode, Value, Option<String>) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        for c in response.headers().get_all(header::SET_COOKIE) {
            let (pair, _) = c.to_str().unwrap().split_once(';').unwrap();
            let (k, v) = pair.split_once('=').unwrap();
            self.cookies.insert(k.into(), v.into());
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            location,
        )
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn get(&self, uri: &str) -> Request<Body> {
        Request::get(uri)
            .header(header::COOKIE, self.cookie_header())
            .body(Body::empty())
            .unwrap()
    }

    async fn sign_in(&mut self) -> String {
        self.sign_in_from("").await
    }

    /// Signs in from the page `back` (`?back=` of the sign-in; "" for none).
    async fn sign_in_from(&mut self, back: &str) -> String {
        let login = match back {
            "" => "/api/mods/auth/login".to_owned(),
            back => format!("/api/mods/auth/login?back={back}"),
        };
        let (status, _, location) = self.send(self.get(&login)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = location.unwrap();
        assert!(location.contains(
            "redirect_uri=https%3A%2F%2Fplayhorizon.test%2Fapi%2Fmods%2Fauth%2Fcallback"
        ));
        let state = location
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_owned();
        // Another state than the one this browser started with is refused.
        let (status, ..) = self
            .send(self.get("/api/mods/auth/callback?code=good&state=other"))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, location) = self
            .send(self.get(&format!("/api/mods/auth/callback?code=good&state={state}")))
            .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let page = match back {
            "/mods" | "/mods/zoom" => back,
            _ => "/mods/submit",
        };
        assert_eq!(location, Some(format!("https://playhorizon.test{page}")));
        let (status, me, _) = self.send(self.get("/api/mods/me")).await;
        assert_eq!(status, StatusCode::OK);
        me["csrf"].as_str().unwrap().to_owned()
    }

    async fn submit(&mut self, csrf: &str, form: &Value, file: &[u8]) -> (StatusCode, Value) {
        self.submit_with(csrf, form, Some(file)).await
    }

    async fn submit_with(
        &mut self,
        csrf: &str,
        form: &Value,
        file: Option<&[u8]>,
    ) -> (StatusCode, Value) {
        let boundary = "XhorizonX";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"entry\"\r\n\r\n{form}\r\n"
        )
        .into_bytes();
        if let Some(file) = file {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
            body.extend_from_slice(file);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let request = Request::post("/api/mods/submit")
            .header(header::COOKIE, self.cookie_header())
            .header(header::ORIGIN, PUBLIC)
            .header("x-csrf-token", csrf)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let (status, json, _) = self.send(request).await;
        (status, json)
    }
}

fn form() -> Value {
    let key = std::fs::read_to_string(fixtures().join("test.pub")).unwrap();
    json!({
        "id": "coords_hud",
        "kind": "mod",
        "name": "Coordinates HUD",
        "summary": "Your coordinates in a corner.",
        "authors": ["Alice"],
        "license": "MIT",
        "source": "https://github.com/alice/coords",
        "key": key.lines().last().unwrap(),
        "signature": std::fs::read_to_string(fixtures().join("coords_hud-0.1.0.hmod.minisig")).unwrap(),
        "version": "0.1.0",
        "game": "26.3",
    })
}

async fn setup() -> (Client, Shared) {
    setup_with(|_| {}).await
}

async fn setup_with(change: impl FnOnce(&mut github::GitHub)) -> (Client, Shared) {
    setup_keyed(change, Some(UPDATE_KEY)).await
}

const UPDATE_KEY: &[u8] = b"a test update key";

async fn setup_keyed(
    change: impl FnOnce(&mut github::GitHub),
    update_key: Option<&[u8]>,
) -> (Client, Shared) {
    let (base, mock) = mock_github().await;
    let scratch =
        std::env::temp_dir().join(format!("horizon-mod-registry-test-{}", std::process::id()));
    let mut github = github::GitHub::new(
        github::Endpoints {
            web: base.clone(),
            api: base.clone(),
            uploads: base,
        },
        "o/r".into(),
        "client".into(),
        "secret".into(),
        "bot".into(),
    );
    change(&mut github);
    let config = Config {
        public_url: PUBLIC.into(),
        session_key: b"a test key of at least thirty-two bytes".to_vec(),
        github,
        update_key: update_key.map(<[u8]>::to_vec),
        scratch,
        database: ":memory:".into(),
    };
    let client = Client {
        app: router(App::new(config).unwrap()),
        cookies: HashMap::new(),
    };
    (client, mock)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_submission_becomes_a_pull_request() {
    let (mut client, mock) = setup().await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    // Not signed in.
    let (status, _) = client.submit("x", &form(), &file).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let csrf = client.sign_in().await;
    let (status, _) = client.submit("wrong", &form(), &file).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["pull_request"], "https://github.com/o/r/pull/1");
    {
        let m = mock.lock().unwrap();
        assert_eq!(m.assets, [(format!("{HMOD_SHA}.hmod"), file.len())]);
        assert_eq!(m.branches, ["submit/coords_hud-0.1.0"]);
        let (text, message, replaced) = &m.commits[&(
            "submit/coords_hud-0.1.0".into(),
            "mods/coords_hud.toml".into(),
        )];
        let entry: Entry = toml::from_str(text).unwrap();
        assert_eq!(entry.owners, ["alice"]);
        assert_eq!(entry.versions[0].sha256, HMOD_SHA);
        assert_eq!(entry.versions[0].file, "coords_hud-0.1.0.hmod");
        assert!(entry.versions[0].signature.is_some() && entry.key.is_some());
        assert!(message.contains("Co-authored-by: alice <7+alice@users.noreply.github.com>"));
        assert_eq!(replaced, &None);
        let body = m.pulls[0]["body"].as_str().unwrap();
        assert!(
            body.starts_with("<!-- submitted-by: alice -->\n🤖 Submitted through "),
            "{body}"
        );
        assert!(body.contains("/mods/submit) by @alice."), "{body}");
        assert!(body.contains("may draw text on your screen"), "{body}");
    }
    // The same version again: its pull request is open.
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::CONFLICT, "{json}");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .ends_with("its pull request is open (https://github.com/o/r/pull/1)"),
        "{json}"
    );
    assert_eq!(mock.lock().unwrap().assets.len(), 1, "uploaded once");
}

/// `mods/coords_hud.toml` on main as `alice` owns it: what the form's page
/// says, a dummy first version.
fn published(owner: &str, key: bool) -> String {
    let f = form();
    let text = format!(
        "id = \"coords_hud\"\nkind = \"mod\"\nname = \"Coordinates HUD\"\nsummary = \"Your coordinates in a corner.\"\nauthors = [\"Alice\"]\nowners = [\"{owner}\"]\nlicense = \"MIT\"\nsource = \"https://github.com/alice/coords\"\n{}\n[[versions]]\nversion = \"0.0.1\"\npublished = \"2026-01-01\"\ngame = \"26.3\"\nfile = \"coords_hud-0.0.1.hmod\"\nsha256 = \"{}\"\n",
        if key {
            format!("key = {}\n", f["key"])
        } else {
            String::new()
        },
        "0".repeat(64)
    );
    toml::from_str::<Entry>(&text).unwrap();
    text
}

/// What an update by `alice` leaves on GitHub: (the pull request's body, its labels).
async fn update(
    main: String,
    change: impl FnOnce(&mut Value),
    key: Option<&[u8]>,
) -> (String, Vec<(u64, Value)>) {
    let (mut client, mock) = setup_keyed(|_| {}, key).await;
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), main);
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let csrf = client.sign_in().await;
    let mut f = form();
    change(&mut f);
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let m = mock.lock().unwrap();
    (
        m.pulls[0]["body"].as_str().unwrap().to_owned(),
        m.labels.clone(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owners_update_is_signed_for_the_repository() {
    use hmac::{Hmac, Mac};
    let (body, labels) = update(published("alice", true), |_| {}, Some(UPDATE_KEY)).await;
    let trailer = body.split("<!-- owner-update\n").nth(1).expect(&body);
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(UPDATE_KEY).unwrap();
    mac.update(
        format!("owner-update-v1\ncoords_hud\n0.1.0\n{HMOD_SHA}\ncoords_hud-0.1.0.hmod\nalice")
            .as_bytes(),
    );
    let hmac: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        trailer,
        format!(
            "id: coords_hud\nversion: 0.1.0\nsha256: {HMOD_SHA}\nfile: coords_hud-0.1.0.hmod\nsubmitter: alice\nhmac: {hmac}\n-->"
        ),
    );
    assert_eq!(labels, [(1, json!(["owner-update"]))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn everything_else_waits_for_a_reviewer() {
    let unsigned = |body: &str, labels: &[(u64, Value)]| {
        assert!(!body.contains("owner-update"), "{body}");
        assert!(labels.is_empty());
    };
    // A new project, not an update (alice isn't in a missing entry's owners).
    let (mut client, mock) = setup().await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let csrf = client.sign_in().await;
    client.submit(&csrf, &form(), &file).await;
    {
        let m = mock.lock().unwrap();
        unsigned(m.pulls[0]["body"].as_str().unwrap(), &m.labels);
    }
    // The feature is off without the secret.
    let (body, labels) = update(published("alice", true), |_| {}, None).await;
    unsigned(&body, &labels);
    // The page's text changes with the version.
    let (body, labels) = update(
        published("alice", true),
        |f| f["summary"] = json!("Another."),
        Some(UPDATE_KEY),
    )
    .await;
    unsigned(&body, &labels);
    // A new icon.
    let (body, labels) = update(
        published("alice", true),
        |f| f["icon"] = json!("https://x.test/i.png"),
        Some(UPDATE_KEY),
    )
    .await;
    unsigned(&body, &labels);
    // A project with no key (and so no signature) is signed as well.
    let unkeyed = |f: &mut Value| {
        f.as_object_mut().unwrap().remove("key");
        f.as_object_mut().unwrap().remove("signature");
    };
    let (body, _) = update(published("alice", false), unkeyed, Some(UPDATE_KEY)).await;
    assert!(body.contains("<!-- owner-update"));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_form_refuses() {
    let (mut client, mock) = setup().await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let csrf = client.sign_in().await;
    let refused = |status: StatusCode, json: &Value, part: &str| {
        assert!(
            json["error"].as_str().unwrap_or("").contains(part),
            "{status} {json}"
        );
    };
    // A file that isn't what the page says.
    let mut f = form();
    f["id"] = json!("other_mod");
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    refused(status, &json, "the file's id is \"coords_hud\"");
    // A broken signature.
    let mut f = form();
    f["signature"] = json!(
        std::fs::read_to_string(fixtures().join("coords_hud-0.1.0.hmod.minisig"))
            .unwrap()
            .replace("RUTV", "RUTW")
    );
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    // A shader pack runs the game's own check: a mod's file isn't one.
    let mut f = form();
    f["kind"] = json!("shader");
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    refused(status, &json, "horizon-shaders.toml: missing");
    // An update by someone who isn't an owner.
    let owned = "id = \"coords_hud\"\nkind = \"mod\"\nname = \"C\"\nsummary = \"s\"\nowners = [\"bob\"]\nlicense = \"MIT\"\nsource = \"https://x.test\"\n\n[[versions]]\nversion = \"0.0.1\"\npublished = \"2026-01-01\"\ngame = \"26.3\"\nfile = \"coords_hud-0.0.1.hmod\"\nsha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n";
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), owned.into());
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    refused(status, &json, "bob's");
    // From another site.
    let mut request = Request::post("/api/mods/submit")
        .header(header::COOKIE, client.cookie_header())
        .header(header::ORIGIN, "https://evil.test")
        .header("x-csrf-token", &csrf)
        .body(Body::empty())
        .unwrap();
    request.headers_mut().insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=x".parse().unwrap(),
    );
    let (status, ..) = client.send(request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Too big: past the file's own limit, and past the whole body's.
    let max = horizon_mods::index::MAX_FILE as usize;
    for size in [max + 1, max + (200 << 10)] {
        let (status, json) = client.submit(&csrf, &form(), &vec![0; size]).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{size}: {json}");
        refused(status, &json, "the file is over 64 MiB");
    }
    let m = mock.lock().unwrap();
    assert!(
        m.assets.is_empty() && m.pulls.is_empty(),
        "nothing reached GitHub"
    );
}

/// The origin gate in front of a submission, before the sign-in and the
/// form: how the answer to a request from `origin`.
async fn origin_gate(app: &Router, origin: Option<&str>) -> StatusCode {
    let mut request = Request::post("/api/mods/submit");
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    let request = request.body(Body::empty()).unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

/// #6159: a submission must come from the origin of `REGISTRY_PUBLIC_URL`,
/// which is what the site's pages are. An `Origin` never carries a path, so
/// a public URL that does (the form is a page below the site's root) still
/// takes its own pages; another site's page is refused before the sign-in is
/// looked at, and so is a request that names no origin.
#[tokio::test]
async fn a_submission_comes_from_the_sites_own_pages() {
    let site = |public_url: &str| {
        router(
            App::new(Config {
                public_url: public_url.into(),
                session_key: b"a test key of at least thirty-two bytes".to_vec(),
                github: github::GitHub::new(
                    github::Endpoints::default(),
                    "o/r".into(),
                    "client".into(),
                    "secret".into(),
                    "bot".into(),
                ),
                update_key: None,
                scratch: std::env::temp_dir().join("horizon-mod-registry-test"),
                database: ":memory:".into(),
            })
            .unwrap(),
        )
    };
    // A public URL with the form's page in it, as the site's pages are.
    let app = site("https://playhorizon.test/mods");
    assert_eq!(
        origin_gate(&app, Some("https://playhorizon.test")).await,
        StatusCode::UNAUTHORIZED
    );
    // Another site's page, and one that names no origin at all.
    assert_eq!(
        origin_gate(&app, Some("https://evil.test")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(origin_gate(&app, None).await, StatusCode::FORBIDDEN);
    // A public URL that is the origin itself keeps working.
    let app = site("https://playhorizon.test");
    assert_eq!(
        origin_gate(&app, Some("https://playhorizon.test")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        origin_gate(&app, Some("https://evil.test")).await,
        StatusCode::FORBIDDEN
    );
}

const ENTRY: &str = "kind = \"mod\"\nname = \"N\"\nsummary = \"s\"\nlicense = \"MIT\"\nsource = \"https://x.test\"\n\n[[versions]]\nversion = \"1.0.0\"\npublished = \"2026-01-01\"\ngame = \"26.3\"\nfile = \"f.hmod\"\nsha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n";

impl Client {
    async fn rate(&mut self, csrf: &str, id: &str, body: &str) -> (StatusCode, Value) {
        let request = Request::post(format!("/api/mods/{id}/rating"))
            .header(header::COOKIE, self.cookie_header())
            .header(header::ORIGIN, PUBLIC)
            .header("x-csrf-token", csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let (status, json, _) = self.send(request).await;
        (status, json)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ratings() {
    let (mut client, mock) = setup().await;
    {
        let mut m = mock.lock().unwrap();
        m.main.insert(
            "mods/zoom.toml".into(),
            format!("id = \"zoom\"\nowners = [\"bob\"]\n{ENTRY}"),
        );
        m.main.insert(
            "mods/mine.toml".into(),
            format!("id = \"mine\"\nowners = [\"Alice\"]\n{ENTRY}"),
        );
    }
    // Nothing rated yet; signed out, no own rating and no rating at all.
    let (status, json, _) = client.send(client.get("/api/mods/ratings")).await;
    assert_eq!((status, json), (StatusCode::OK, json!({"projects": {}})));
    let (status, ..) = client.send(client.get("/api/mods/zoom/rating")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = client.rate("x", "zoom", r#"{"stars": 4}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A sign-in from a project's page comes back to it; anywhere else is the form.
    client.sign_in_from("https://evil.test/").await;
    let csrf = client.sign_in_from("/mods/zoom").await;
    let (status, json) = client.rate("wrong", "zoom", r#"{"stars": 4}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{json}");
    for stars in ["0", "6"] {
        let (status, json) = client
            .rate(&csrf, "zoom", &format!(r#"{{"stars": {stars}}}"#))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    }
    let (status, json) = client.rate(&csrf, "zoom", r#"{"stars": 4}"#).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json, json!({"stars": 4, "average": 4.0, "count": 1}));
    // Changing it replaces it.
    let (status, json) = client.rate(&csrf, "zoom", r#"{"stars": 2}"#).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["count"], 1);
    let (_, json, _) = client.send(client.get("/api/mods/zoom/rating")).await;
    assert_eq!(json, json!({"stars": 2}));
    let (_, json, _) = client.send(client.get("/api/mods/minimap/rating")).await;
    assert_eq!(json, json!({"stars": null}));

    // Not one's own project, nor one that isn't published.
    let (status, json) = client.rate(&csrf, "mine", r#"{"stars": 5}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{json}");
    let (status, json) = client.rate(&csrf, "nothing", r#"{"stars": 5}"#).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");

    // Another account's rating counts once more; the totals never say whose.
    let mut other = Client {
        app: client.app.clone(),
        cookies: HashMap::new(),
    };
    mock.lock().unwrap().login = "carol".into();
    let carol = other.sign_in().await;
    let (status, json) = other.rate(&carol, "zoom", r#"{"stars": 5}"#).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json, json!({"stars": 5, "average": 3.5, "count": 2}));
    let (status, json, _) = client.send(client.get("/api/mods/ratings")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json,
        json!({"projects": {"zoom": {"average": 3.5, "count": 2}}})
    );

    // The limit per account and hour.
    let mut last = StatusCode::OK;
    for _ in 0..40 {
        (last, _) = client.rate(&csrf, "zoom", r#"{"stars": 3}"#).await;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_github_is_named() {
    let (mut client, mock) =
        setup_with(|gh| gh.call_timeout = std::time::Duration::from_millis(300)).await;
    mock.lock().unwrap().slow = Some(std::time::Duration::from_secs(3));
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let csrf = client.sign_in().await;
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{json}");
    assert_eq!(
        json["error"],
        "GitHub is slow right now: try again in a few minutes"
    );
}

/// Over a real connection: a submission refused before its body is read
/// still gets its answer once the browser has sent the whole file, instead
/// of a connection closed mid-upload (the form's "the service didn't answer").
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_waits_for_the_upload() {
    use std::io::{Read, Write};
    let (client, _mock) = setup().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, client.app).await.unwrap() });
    let answer = tokio::task::spawn_blocking(move || {
        let size = 16 << 20;
        let mut stream = std::net::TcpStream::connect(at).unwrap();
        write!(
            stream,
            "POST /api/mods/submit HTTP/1.1\r\nHost: x\r\nConnection: close\r\nOrigin: {PUBLIC}\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {size}\r\n\r\n"
        )
        .unwrap();
        let chunk = vec![0u8; 64 << 10];
        for _ in 0..size / chunk.len() {
            stream.write_all(&chunk).expect("the whole file is read");
        }
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    })
    .await
    .unwrap();
    assert!(answer.starts_with("HTTP/1.1 401"), "{answer}");
    assert!(answer.contains("{\"error\":"), "{answer}");
}

/// A submission that GitHub broke off after making its branch: the same
/// submitter's retry takes the branch up again; anyone else's is refused,
/// and so is any retry once the pull request is open.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_takes_up_its_own_branch() {
    let (mut client, mock) =
        setup_with(|gh| gh.call_timeout = std::time::Duration::from_millis(300)).await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let branch = "submit/coords_hud-0.1.0".to_owned();
    let key = (branch.clone(), "mods/coords_hud.toml".to_owned());

    // Someone else's branch, with no pull request: not taken over, even when
    // its commit copies the form's words for alice (only the service signs them).
    let csrf = client.sign_in().await;
    {
        let mut m = mock.lock().unwrap();
        m.branches.push(branch.clone());
        m.commits.insert(
            key.clone(),
            (
                "x".into(),
                "Add coords_hud 0.1.0\n\nSubmitted by @alice through the form.\n\nSubmission: forged".into(),
                None,
            ),
        );
    }
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::CONFLICT, "{json}");
    assert!(
        json["error"].as_str().unwrap().contains("by someone else"),
        "{json}"
    );
    {
        let mut m = mock.lock().unwrap();
        assert!(m.resets.is_empty() && m.pulls.is_empty());
        m.branches.clear();
        m.commits.clear();
    }

    // GitHub times out opening the pull request: the branch and its commit are there.
    mock.lock().unwrap().slow_pull = Some(std::time::Duration::from_secs(2));
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{json}");
    {
        let mut m = mock.lock().unwrap();
        assert_eq!(m.branches, [branch.as_str()]);
        assert!(m.commits.contains_key(&key) && m.pulls.is_empty());
        m.slow_pull = None;
    }
    // The retry starts the branch again from main and opens the pull request.
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["pull_request"], "https://github.com/o/r/pull/1");
    {
        let m = mock.lock().unwrap();
        assert_eq!(m.resets, [branch.as_str()]);
        let message = &m.commits[&key].1;
        assert!(
            message.contains("Submitted by @alice through the form.\n\nSubmission: "),
            "{message}"
        );
        assert_eq!(m.pulls[0]["head"], branch.as_str());
    }
    // Now its pull request is open: refused, the branch left alone.
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::CONFLICT, "{json}");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("pull request is open"),
        "{json}"
    );
    assert_eq!(mock.lock().unwrap().resets.len(), 1);
    // A closed one doesn't hold the version: a new pull request.
    mock.lock().unwrap().pulls[0]["state"] = json!("closed");
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["pull_request"], "https://github.com/o/r/pull/2");

    // A branch with no commit yet (cut off before the entry was written) is taken up too.
    {
        let mut m = mock.lock().unwrap();
        m.pulls.clear();
        m.commits.clear();
        m.resets.clear();
    }
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(mock.lock().unwrap().resets, [branch.as_str()]);
}

/// A native rewrite (`.hnmod`) of `coords_hud` at `version`.
fn hnmod(version: &str) -> Vec<u8> {
    use std::io::Write;
    let revision = "a".repeat(40);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("horizon-native.json", options).unwrap();
    let manifest = json!({"schema": 1, "id": "coords_hud", "version": version, "game": "26.3", "horizon": revision, "mixins": "b".repeat(40)});
    zip.write_all(manifest.to_string().as_bytes()).unwrap();
    zip.start_file("mixins/coords_hud/horizon-mixin.toml", options)
        .unwrap();
    write!(
        zip,
        "[mod]\nid = \"coords_hud\"\nversion = \"{version}\"\n[compatibility]\nhorizon_revision = \"{revision}\"\n"
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

/// A WebAssembly mod's entry turns `native-mod` once, by its owner; its
/// `.hmod` versions stay. No other kind change.
#[tokio::test(flavor = "multi_thread")]
async fn a_mod_turns_native_once() {
    let (mut client, mock) = setup().await;
    let published = format!(
        "id = \"coords_hud\"\nkind = \"mod\"\nname = \"C\"\nsummary = \"s\"\nowners = [\"Alice\"]\nlicense = \"MIT\"\nsource = \"https://x.test\"\n\n[[versions]]\nversion = \"0.1.0\"\npublished = \"2026-01-01\"\ngame = \"26.3\"\nfile = \"coords_hud-0.1.0.hmod\"\nsha256 = \"{HMOD_SHA}\"\n"
    );
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), published.clone());
    let csrf = client.sign_in().await;
    let mut native = form();
    native["kind"] = json!("native-mod");
    native["version"] = json!("0.2.0");
    native["key"] = Value::Null;
    native["signature"] = Value::Null;
    let file = hnmod("0.2.0");

    // Not by someone who isn't an owner.
    mock.lock().unwrap().main.insert(
        "mods/coords_hud.toml".into(),
        published.replace("\"Alice\"", "\"bob\""),
    );
    let (status, json) = client.submit(&csrf, &native, &file).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{json}");

    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), published);
    let (status, json) = client.submit(&csrf, &native, &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let text = {
        let m = mock.lock().unwrap();
        let (text, _, replaced) = &m.commits[&(
            "submit/coords_hud-0.2.0".into(),
            "mods/coords_hud.toml".into(),
        )];
        assert_eq!(replaced.as_deref(), Some("blob1"));
        assert!(m.assets[0].0.ends_with(".hnmod"), "{:?}", m.assets);
        let body = m.pulls[0]["body"].as_str().unwrap();
        assert!(body.contains("(native-mod, update: turns native"), "{body}");
        // Reviewed by hand: never merged as an owner update.
        assert!(
            !body.contains("owner-update") && m.labels.is_empty(),
            "{body}"
        );
        text.clone()
    };
    let entry: Entry = toml::from_str(&text).unwrap();
    assert_eq!(entry.kind.name(), "native-mod");
    let files: Vec<&str> = entry.versions.iter().map(|v| v.file.as_str()).collect();
    assert_eq!(files, ["coords_hud-0.2.0.hnmod", "coords_hud-0.1.0.hmod"]);

    // Once native, never back.
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), text);
    let wasm = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let (status, json) = client.submit(&csrf, &form(), &wasm).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    assert_eq!(
        json["error"],
        "coords_hud is a native-mod, not a mod: a project's kind never changes (only a mod may turn native-mod, once)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_is_listed_by_its_repository_and_never_updated_by_the_form() {
    let (mut client, mock) = setup().await;
    let csrf = client.sign_in().await;
    let listing = json!({
        "id": "coords_hud", "kind": "native-mod", "repo": "alice/coords", "name": "Coordinates HUD",
        "summary": "Your coordinates in a corner.", "license": "MIT",
    });
    let (status, json) = client.submit_with(&csrf, &listing, None).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    {
        let m = mock.lock().unwrap();
        assert!(m.assets.is_empty(), "no file is uploaded");
        let (text, _, blob) = &m.commits[&(
            "list/coords_hud".to_owned(),
            "mods/coords_hud.toml".to_owned(),
        )];
        assert!(blob.is_none());
        let entry: Entry = toml::from_str(text).unwrap();
        assert_eq!(entry.repo.as_deref(), Some("alice/coords"));
        assert_eq!(entry.owners, ["alice"]);
        assert!(entry.versions.is_empty());
        let body = m.pulls[0]["body"].as_str().unwrap();
        assert!(
            body.contains("the submitter") && !body.contains("owner-update"),
            "{body}"
        );
        assert!(m.labels.is_empty());
    }
    // A repository that is not found or private, a missing licence, a file beside a repository.
    let mut other = listing.clone();
    other["id"] = json!("other");
    for (field, value) in [
        ("repo", json!("alice/secret")),
        ("repo", json!("alice/nothing")),
        ("license", json!("")),
    ] {
        let mut bad = other.clone();
        bad[field] = value;
        let (status, json) = client.submit_with(&csrf, &bad, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    }
    let (status, _) = client.submit_with(&csrf, &other, Some(b"x")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // An update of a project listed by its repository: publish a release instead.
    let main = published("alice", false).replacen(
        "kind = \"mod\"\n",
        "kind = \"mod\"\nrepo = \"alice/coords\"\n",
        1,
    );
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), main);
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::CONFLICT, "{json}");
    assert!(
        json["error"].as_str().unwrap().contains("GitHub release"),
        "{json}"
    );
}
