use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The service: who wears what, every request logged.
#[derive(Clone, Default)]
struct Fake {
    log: Arc<Mutex<Vec<String>>>,
    offline: Arc<AtomicBool>,
    wearing: Arc<Mutex<HashMap<u128, Vec<&'static str>>>>,
}

impl Fake {
    fn log(&self) -> Vec<String> {
        std::mem::take(&mut self.log.lock().unwrap())
    }
}

/// An item with one box (24 vertices), or `huge`: past the budget.
fn item_file(id: &str) -> String {
    let cube = r#"{"origin": [-1, -1, -1], "size": [2, 2, 2], "uv": [0, 0]}"#;
    let cubes = vec![cube; if id == "huge" { 30 } else { 1 }].join(",");
    format!(
        r#"{{"format": 1, "id": "{id}", "slot": "head", "bone": "head",
          "texture": {{"png": "{}"}}, "parts": [{{"name": "a", "cubes": [{cubes}]}}]}}"#,
        crate::pose::tests::png(16, 16)
    )
}

impl Transport for Fake {
    fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let body: serde_json::Value = serde_json::from_slice(body).unwrap();
        let asked = body["players"].as_array().unwrap();
        self.log
            .lock()
            .unwrap()
            .push(format!("POST {path} {}", asked.len()));
        if self.offline.load(Ordering::SeqCst) {
            return Err("offline".into());
        }
        let wearing = self.wearing.lock().unwrap();
        let mut players = serde_json::Map::new();
        for uuid in asked {
            let uuid = uuid.as_str().unwrap();
            if let Some(items) = wearing.get(&u128::from_str_radix(uuid, 16).unwrap()) {
                let refs = items
                    .iter()
                    .map(|id| serde_json::json!({"id": id, "hash": "h1"}))
                    .collect();
                players.insert(uuid.to_owned(), serde_json::Value::Array(refs));
            }
        }
        Ok(serde_json::json!({"ttl": 300, "players": players})
            .to_string()
            .into_bytes())
    }

    fn get(&self, path: &str) -> Result<Vec<u8>, String> {
        self.log.lock().unwrap().push(format!("GET {path}"));
        if self.offline.load(Ordering::SeqCst) {
            return Err("offline".into());
        }
        let id = path
            .trim_start_matches("/v1/items/")
            .split('?')
            .next()
            .unwrap();
        Ok(item_file(id).into_bytes())
    }
}

/// A clock the test moves by whole seconds.
fn clock() -> (Clock, Arc<AtomicU64>) {
    let start = Instant::now();
    let seconds = Arc::new(AtomicU64::new(0));
    let s = seconds.clone();
    let clock: Clock = Arc::new(move || start + Duration::from_secs(s.load(Ordering::SeqCst)));
    (clock, seconds)
}

fn run(c: &mut Cosmetics) {
    (c.worker.as_mut().unwrap())();
}

fn ids(worn: Option<Worn>) -> Vec<String> {
    worn.into_iter()
        .flat_map(|w| w.iter().map(|i| i.id.clone()).collect::<Vec<_>>())
        .collect()
}

fn setup(cache: Option<PathBuf>) -> (Fake, Cosmetics, Arc<AtomicU64>) {
    let fake = Fake::default();
    {
        let mut wearing = fake.wearing.lock().unwrap();
        for uuid in 0..10 {
            wearing.insert(uuid, vec!["hat"]);
        }
        wearing.insert(5, vec!["hat", "pet"]);
    }
    let (clock, time) = clock();
    let c = Cosmetics::manual(fake.clone(), cache, clock);
    (fake, c, time)
}

#[test]
fn asks_once_for_the_players_in_view_in_batches() {
    let (fake, mut c, _) = setup(None);
    {
        let mut frame = c.frame();
        assert!((0..100).all(|uuid| frame.worn(uuid).is_none()));
    }
    run(&mut c);
    run(&mut c);
    let log = fake.log();
    let posts: Vec<_> = log.iter().filter(|l| l.starts_with("POST")).collect();
    assert_eq!(posts, ["POST /v1/worn 64", "POST /v1/worn 36"]);
    // Each item once, however many wear it.
    let mut gets: Vec<_> = log.iter().filter(|l| l.starts_with("GET")).collect();
    gets.sort();
    assert_eq!(
        gets,
        ["GET /v1/items/hat?hash=h1", "GET /v1/items/pet?hash=h1"]
    );
    let mut frame = c.frame();
    assert_eq!(ids(frame.worn(0)), ["hat"]);
    assert_eq!(ids(frame.worn(5)), ["hat", "pet"]);
    assert!(frame.worn(50).is_none());
    drop(frame);
    // Everyone is known: nothing more to ask.
    run(&mut c);
    assert!(fake.log().is_empty());
}

#[test]
fn nothing_in_view_asks_nothing() {
    let (fake, mut c, _) = setup(None);
    drop(c.frame());
    run(&mut c);
    assert!(fake.log().is_empty());
}

#[test]
fn asks_again_after_the_ttl_keeping_the_answer_meanwhile() {
    let (fake, mut c, time) = setup(None);
    let _ = c.frame().worn(5);
    run(&mut c);
    fake.log();
    fake.wearing.lock().unwrap().insert(5, vec!["pet"]);
    time.store(299, Ordering::SeqCst);
    assert_eq!(ids(c.frame().worn(5)), ["hat", "pet"]);
    run(&mut c);
    assert!(fake.log().is_empty());
    time.store(300, Ordering::SeqCst);
    // Expired: still drawn as known while asked again.
    assert_eq!(ids(c.frame().worn(5)), ["hat", "pet"]);
    run(&mut c);
    assert_eq!(fake.log(), ["POST /v1/worn 1"]);
    assert_eq!(ids(c.frame().worn(5)), ["pet"]);
}

#[test]
fn offline_keeps_the_last_answers_and_backs_off() {
    let (fake, mut c, time) = setup(None);
    let _ = c.frame().worn(1);
    run(&mut c);
    fake.log();
    fake.offline.store(true, Ordering::SeqCst);
    time.store(400, Ordering::SeqCst);
    let _ = c.frame().worn(1);
    let _ = c.frame().worn(42);
    run(&mut c);
    assert_eq!(fake.log(), ["POST /v1/worn 2"]);
    assert_eq!(ids(c.frame().worn(1)), ["hat"]);
    // 15 s, then 30 s, before asking again.
    for (at, asks) in [(414, false), (415, true), (444, false), (445, true)] {
        time.store(at, Ordering::SeqCst);
        run(&mut c);
        assert_eq!(!fake.log().is_empty(), asks, "at {at}");
    }
    fake.offline.store(false, Ordering::SeqCst);
    time.store(505, Ordering::SeqCst);
    run(&mut c);
    assert_eq!(fake.log(), ["POST /v1/worn 2"]);
    assert_eq!(ids(c.frame().worn(1)), ["hat"]);
    assert!(c.frame().worn(42).is_none());
}

#[test]
fn item_files_are_kept_on_disk() {
    let dir = std::env::temp_dir().join(format!("horizon-cosmetics-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (fake, mut c, _) = setup(Some(dir.clone()));
    let _ = c.frame().worn(5);
    run(&mut c);
    assert_eq!(fake.log().len(), 3);
    assert!(dir.join("hat-h1.json").is_file());
    // A new game: the items read from disk, not asked for.
    let (fake, mut c, _) = setup(Some(dir.clone()));
    let _ = c.frame().worn(5);
    run(&mut c);
    assert_eq!(fake.log(), ["POST /v1/worn 1"]);
    assert_eq!(ids(c.frame().worn(5)), ["hat", "pet"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_item_past_the_limits_is_left_out_and_not_asked_again() {
    let (fake, mut c, time) = setup(None);
    fake.wearing.lock().unwrap().insert(7, vec!["huge", "hat"]);
    let _ = c.frame().worn(7);
    run(&mut c);
    assert_eq!(fake.log().len(), 3);
    assert_eq!(ids(c.frame().worn(7)), ["hat"]);
    time.store(300, Ordering::SeqCst);
    let _ = c.frame().worn(7);
    run(&mut c);
    assert_eq!(fake.log(), ["POST /v1/worn 1"]);
}

#[test]
fn an_item_not_sent_is_asked_for_again_soon() {
    let (fake, mut c, time) = setup(None);
    let _ = c.frame().worn(3);
    // The answer arrives, the item doesn't.
    struct ItemsDown(Fake);
    impl Transport for ItemsDown {
        fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, String> {
            self.0.post(path, body)
        }
        fn get(&self, _: &str) -> Result<Vec<u8>, String> {
            Err("down".into())
        }
    }
    let shared = c.shared.clone();
    work(&shared, &ItemsDown(fake.clone()), &None);
    assert!(c.frame().worn(3).is_none());
    time.store(BACKOFF.0.as_secs(), Ordering::SeqCst);
    let _ = c.frame().worn(3);
    run(&mut c);
    assert_eq!(ids(c.frame().worn(3)), ["hat"]);
}

#[test]
fn the_thread_answers_a_frame() {
    let fake = Fake::default();
    fake.wearing.lock().unwrap().insert(9, vec!["hat"]);
    let c = Cosmetics::new(fake, None);
    let _ = c.frame().worn(9);
    let deadline = Instant::now() + Duration::from_secs(10);
    while c.frame().worn(9).is_none() {
        assert!(Instant::now() < deadline, "no answer");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(ids(c.frame().worn(9)), ["hat"]);
}
