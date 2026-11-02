//! The Wardrobe client against a local mock of the cosmetics service (its answers' shapes).
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

#[derive(Default)]
struct Mock {
    /// The claim window's state, as the service would say it.
    claim: &'static str,
    /// How long a session lasts, in seconds.
    expires_in: u64,
    /// The server ids Mojang's session server was told this account joined.
    joined: Vec<String>,
    /// Every request: method, path, Authorization header, body.
    log: Vec<(String, String, String, String)>,
}

fn me(claim: &str, owned: Value, equipped: Value) -> Value {
    json!({
        "uuid": UUID, "name": "Notch", "owned": owned, "equipped": equipped,
        "claim": {"state": claim, "items": ["hat", "owl"], "starts_at": 100, "ends_at": 200, "claimed": owned != json!([])},
    })
}

fn route(m: &Mock, method: &str, path: &str, auth: &str, body: &str) -> (u16, Value) {
    let signed_in = auth == "Bearer svc-session";
    match (method, path) {
        ("GET", "/v1/catalogue") => (
            200,
            json!({
                "items": [{"id": "hat", "hash": "ab12", "slot": "head", "names": {"en_us": "Hat", "fr_fr": "Chapeau"}}],
                "claim": {"state": m.claim, "items": ["hat"], "starts_at": null, "ends_at": null},
            }),
        ),
        ("POST", "/v1/auth/start") => (200, json!({"server_id": "issued-1", "expires_in": 120})),
        ("POST", "/v1/auth/finish") => {
            let v: Value = serde_json::from_str(body).unwrap();
            let id = v["server_id"].as_str().unwrap_or("");
            if id != "issued-1" || !m.joined.iter().any(|j| j == id) {
                return (401, json!({"error": "not_joined"}));
            }
            (
                200,
                json!({"session": "svc-session", "uuid": UUID, "name": v["name"], "expires_in": m.expires_in}),
            )
        }
        (_, "/v1/me" | "/v1/claim" | "/v1/equipped") if !signed_in => {
            (401, json!({"error": "no_session"}))
        }
        ("GET", "/v1/me") => (200, me(m.claim, json!([]), json!({}))),
        ("POST", "/v1/claim") if m.claim != "open" => (403, json!({"error": "claim_closed"})),
        ("POST", "/v1/claim") => (200, me("open", json!(["hat", "owl"]), json!({}))),
        ("PUT", "/v1/equipped") => {
            let v: Value = serde_json::from_str(body).unwrap();
            match v["item"].as_str() {
                Some("hat") => (200, me(m.claim, json!(["hat"]), json!({"head": "hat"}))),
                _ => (403, json!({"error": "not_owned"})),
            }
        }
        _ => (404, json!({"error": "not_found"})),
    }
}

/// A one-thread HTTP/1.1 server, one request per connection.
fn serve(f: impl FnOnce(&mut Mock)) -> (Arc<Mutex<Mock>>, Service) {
    let mut mock = Mock {
        claim: "open",
        expires_in: 3600,
        ..Default::default()
    };
    f(&mut mock);
    let mock = Arc::new(Mutex::new(mock));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let shared = mock.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_owned(),
                parts.next().unwrap_or("").to_owned(),
            );
            let (mut length, mut auth) = (0, String::new());
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (k, v) = header.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "authorization" => auth = v.trim().to_owned(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            if (method.as_str(), path.as_str()) == ("GET", "/") {
                continue;
            }
            let (status, reply) = {
                let mut m = shared.lock().unwrap();
                m.log
                    .push((method.clone(), path.clone(), auth.clone(), body.clone()));
                route(&m, &method, &path, &auth, &body)
            };
            let text = reply.to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (mock, Service::at(&format!("{base}/")))
}

/// Sign in with a `join` that Mojang accepts: it records the id as joined.
fn sign_in(m: &Arc<Mutex<Mock>>, service: &Service) -> Session {
    service
        .sign_in("Notch", |id| {
            m.lock().unwrap().joined.push(id.to_owned());
            Ok(())
        })
        .unwrap()
}

#[test]
fn signing_in_goes_through_mojang_and_sends_the_service_no_token() {
    let (m, service) = serve(|_| {});
    // `join` fails (Mojang refused): the service is never asked to finish.
    let refused = service.sign_in("Notch", |_| Err("refused".into()));
    assert_eq!(refused.unwrap_err(), Error::Account("refused".into()));
    // `join` claims success but Mojang never saw it: the service refuses the id.
    let unconfirmed = service.sign_in("Notch", |_| Ok(())).unwrap_err();
    assert!(matches!(unconfirmed, Error::Account(ref e) if e.contains("Sign in again")));

    let session = sign_in(&m, &service);
    assert_eq!(
        (session.uuid.as_str(), session.name.as_str()),
        (UUID, "Notch")
    );
    assert!(!format!("{session:?}").contains("svc-session"));
    let me = service.me(&session).unwrap();
    assert_eq!(me.claim.state, ClaimState::Open);

    let log = &m.lock().unwrap().log;
    let paths: Vec<&str> = log.iter().map(|(_, p, ..)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/v1/auth/start",
            "/v1/auth/start",
            "/v1/auth/finish",
            "/v1/auth/start",
            "/v1/auth/finish",
            "/v1/me"
        ]
    );
    // Only the service's own session is ever sent as a credential.
    for (_, path, auth, body) in log {
        assert!(!body.contains("accessToken"));
        assert_eq!(auth == "Bearer svc-session", path == "/v1/me");
    }
}

#[test]
fn claiming_and_equipping_follow_the_window_and_ownership() {
    let (m, service) = serve(|m| m.claim = "not_yet");
    let catalogue = service.catalogue().unwrap();
    assert_eq!(catalogue.claim.state, ClaimState::NotYet);
    assert_eq!(catalogue.items[0].name("fr_fr"), "Chapeau");
    assert_eq!(catalogue.items[0].name("ja_jp"), "Hat");
    let session = sign_in(&m, &service);
    assert_eq!(service.claim(&session).unwrap_err(), Error::ClaimClosed);
    m.lock().unwrap().claim = "open";
    let me = service.claim(&session).unwrap();
    assert_eq!(me.owned, ["hat", "owl"]);
    assert_eq!(me.claim.claimed, Some(true));
    let me = service.equip(&session, "head", Some("hat")).unwrap();
    assert_eq!(me.equipped.get("head").map(String::as_str), Some("hat"));
    assert_eq!(
        service.equip(&session, "head", Some("crown")).unwrap_err(),
        Error::NotYours
    );
}

#[test]
fn an_ended_session_is_not_sent_and_offline_says_so() {
    let (m, service) = serve(|m| m.expires_in = 30);
    // Less than the minute's margin: the session counts as ended at once.
    let session = sign_in(&m, &service);
    let before = m.lock().unwrap().log.len();
    assert_eq!(service.me(&session).unwrap_err(), Error::SignedOut);
    assert_eq!(m.lock().unwrap().log.len(), before);
    // Nothing listens on port 9.
    let offline = Service::at("http://127.0.0.1:9");
    assert_eq!(offline.catalogue().unwrap_err(), Error::Offline);
}
