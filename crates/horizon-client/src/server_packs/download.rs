//! Server pack downloads into the game directory's `downloads/` cache, as
//! vanilla keeps it (`DownloadQueue`, `HttpUtil.downloadFile`,
//! `DownloadCacheCleaner`): `downloads/<pack id>/<SHA-1 of the file>`, one
//! batch at a time on a thread of its own, every download logged in
//! `downloads/log.json`, and the cache cut to its 20 newest files on start.
use super::{Batch, Request};
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    time::SystemTime,
};

/// `DownloadedPackSource`'s MAX_PACK_SIZE_BYTES (250 MiB).
pub const MAX_SIZE: u64 = 262_144_000;
/// `DownloadQueue.MAX_KEPT_PACKS`.
const MAX_KEPT: usize = 20;
/// Longest wait for a connection, and for any one read from the server. The
/// queue runs batches on one thread, so a server that accepts the connection
/// and then goes silent would otherwise hold the pack screen forever.
const NET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// `createDownloadHeaders`: who is downloading, with which game.
pub fn headers(name: &str, uuid: u128) -> Vec<(&'static str, String)> {
    use horizon_core::version::{RESOURCE_PACK_FORMAT, VANILLA_VERSION, VERSION_ID};
    vec![
        ("X-Minecraft-Username", name.to_owned()),
        ("X-Minecraft-UUID", format!("{uuid:032x}")),
        ("X-Minecraft-Version", VANILLA_VERSION.to_owned()),
        ("X-Minecraft-Version-ID", VERSION_ID.to_owned()),
        (
            "X-Minecraft-Pack-Format",
            format!("{}.{}", RESOURCE_PACK_FORMAT.0, RESOURCE_PACK_FORMAT.1),
        ),
        ("User-Agent", format!("Minecraft Java/{VANILLA_VERSION}")),
    ]
}

/// `HttpUtil.DownloadProgressListener`, on the channel a batch was given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// `requestStart`: the batch's next request begins.
    RequestStart,
    /// `downloadStart`: the response came, with its Content-Length if it
    /// said one.
    DownloadStart(Option<u64>),
    /// `downloadedBytes`: what this request has written so far.
    Bytes(u64),
    /// `requestFinished`: whether it succeeded.
    RequestFinished(bool),
}

type Job = (Vec<Request>, Vec<(&'static str, String)>, Sender<Progress>);

/// `DownloadQueue`: batches download in order, on one thread.
pub struct Queue {
    jobs: Sender<Job>,
    done: Receiver<Batch>,
}
impl Queue {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        vacuum(&dir, MAX_KEPT);
        let (jobs, todo) = channel::<Job>();
        let (finished, done) = channel();
        std::thread::Builder::new()
            .name("download-queue".into())
            .spawn(move || {
                for (requests, headers, progress) in todo {
                    if finished
                        .send(run(&dir, &requests, &headers, &progress))
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { jobs, done })
    }
    pub fn download(
        &self,
        requests: Vec<Request>,
        headers: Vec<(&'static str, String)>,
        progress: Sender<Progress>,
    ) {
        let _ = self.jobs.send((requests, headers, progress));
    }
    /// The batches finished since the last call.
    pub fn finished(&self) -> Vec<Batch> {
        self.done.try_iter().collect()
    }
}

/// `DownloadQueue.runDownload`: each pack into `<dir>/<id>/`, logged.
fn run(
    dir: &Path,
    requests: &[Request],
    headers: &[(&'static str, String)],
    progress: &Sender<Progress>,
) -> Batch {
    let mut batch = Batch::default();
    for request in requests {
        let id = horizon_protocol::crypto::uuid_string(request.id);
        let _ = progress.send(Progress::RequestStart);
        let result = download_file(&dir.join(&id), request, headers, MAX_SIZE, progress);
        let _ = progress.send(Progress::RequestFinished(result.is_ok()));
        // DownloadQueue.LogEntry, its fields in the codec's order.
        let mut entry = vec![
            ("id", serde_json::Value::from(id)),
            ("url", request.url.as_str().into()),
            (
                "time",
                chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
                    .into(),
            ),
        ];
        if let Some(sha1) = &request.sha1 {
            entry.push(("hash", sha1.as_str().into()));
        }
        entry.push(match &result {
            Ok(path) => match fs::metadata(path) {
                Ok(meta) => {
                    let name = path.strip_prefix(dir).unwrap_or(path);
                    let name = name.to_string_lossy().replace('\\', "/");
                    (
                        "file",
                        serde_json::json!({"name": name, "size": meta.len()}),
                    )
                }
                Err(_) => ("error", "no_access".into()),
            },
            Err(e) => {
                log::error!("Horizon: failed to download {}: {e}", request.url);
                ("error", "download_failed".into())
            }
        });
        if let Err(e) = append_log(&dir.join("log.json"), &entry) {
            log::error!("Horizon: failed to log download of {}: {e}", request.url);
        }
        match result {
            Ok(path) => batch.downloaded.push((request.id, path)),
            Err(_) => batch.failed.push(request.id),
        }
    }
    batch
}

/// `JsonEventLog.write`: one JSON object a line, as Gson writes it (with
/// its HTML escaping).
fn append_log(path: &Path, entry: &[(&str, serde_json::Value)]) -> io::Result<()> {
    let fields: Vec<String> = entry
        .iter()
        .map(|(k, v)| format!("{}:{v}", serde_json::Value::from(*k)))
        .collect();
    let mut line = String::new();
    for c in format!("{{{}}}", fields.join(",")).chars() {
        match c {
            '<' | '>' | '&' | '=' | '\'' => line.push_str(&format!("\\u{:04x}", c as u32)),
            c => line.push(c),
        }
    }
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

fn sha1_of(path: &Path) -> io::Result<String> {
    let mut hasher = Sha1::new();
    io::copy(&mut fs::File::open(path)?, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// `checkExistingFile`: the file is there with that hash.
fn matches(path: &Path, sha1: &str) -> bool {
    path.exists() && sha1_of(path).is_ok_and(|actual| actual == sha1)
}
/// `updateModificationTime`: used now, so the cleaner keeps it longest.
fn touch(path: &Path) {
    let _ = fs::File::options()
        .append(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
}

/// `HttpUtil.downloadFile`: into `<dir>/<SHA-1>`. With a hash the cached file
/// is used when it matches, and a download that does not is an error;
/// without one the file is named after the hash of what came.
pub fn download_file(
    dir: &Path,
    request: &Request,
    headers: &[(&'static str, String)],
    max_size: u64,
    progress: &Sender<Progress>,
) -> Result<PathBuf, String> {
    download_file_within(dir, request, headers, max_size, progress, NET_TIMEOUT)
}
/// `download_file` with the connect and read timeout given.
fn download_file_within(
    dir: &Path,
    request: &Request,
    headers: &[(&'static str, String)],
    max_size: u64,
    progress: &Sender<Progress>,
    timeout: std::time::Duration,
) -> Result<PathBuf, String> {
    let target = request.sha1.as_ref().map(|sha1| dir.join(sha1));
    if let (Some(target), Some(sha1)) = (&target, &request.sha1) {
        if matches(target, sha1) {
            log::info!("Horizon: returning cached file since actual hash matches requested");
            touch(target);
            return Ok(target.clone());
        }
        if target.exists() {
            log::warn!(
                "Horizon: existing file {} not found or had mismatched hash",
                target.display()
            );
        }
        fs::remove_file(target)
            .or_else(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .map_err(|e| format!("Failed to remove existing file {}: {e}", target.display()))?;
    }
    let mut call = ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .build()
        .get(&request.url);
    for (name, value) in headers {
        call = call.set(name, value);
    }
    let response = call.call().map_err(|e| e.to_string())?;
    let size = response
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let _ = progress.send(Progress::DownloadStart(size));
    if let Some(size) = size
        && size > max_size
    {
        return Err(format!(
            "Filesize is bigger than maximum allowed (file is {size}, limit is {max_size})"
        ));
    }
    let mut input = response.into_reader();
    let mut report = |written: u64| {
        let _ = progress.send(Progress::Bytes(written));
    };
    match (&target, &request.sha1) {
        (Some(target), Some(sha1)) => {
            let actual = save_hashed(&mut input, target, max_size, &mut report)
                .map_err(|e| e.to_string())?;
            if &actual != sha1 {
                return Err(format!(
                    "Hash of downloaded file ({actual}) did not match requested ({sha1})"
                ));
            }
            Ok(target.clone())
        }
        _ => {
            let tmp = dir.join(format!("download{:016x}.tmp", rand_suffix()));
            let result = save_hashed(&mut input, &tmp, max_size, &mut report).and_then(|actual| {
                let path = dir.join(&actual);
                if matches(&path, &actual) {
                    touch(&path);
                } else {
                    fs::rename(&tmp, &path)?;
                }
                Ok(path)
            });
            let _ = fs::remove_file(&tmp);
            result.map_err(|e| e.to_string())
        }
    }
}
fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::time::Instant::now())
}
/// `downloadAndHash`: write what comes, at most `max_size` bytes.
fn save_hashed(
    input: &mut dyn Read,
    path: &Path,
    max_size: u64,
    progress: &mut dyn FnMut(u64),
) -> io::Result<String> {
    let mut out = fs::File::create(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 8196];
    let mut read_so_far = 0u64;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        read_so_far += n as u64;
        progress(read_so_far);
        if read_so_far > max_size {
            return Err(io::Error::other(format!(
                "Filesize was bigger than maximum allowed (got >= {read_so_far}, limit was {max_size})"
            )));
        }
        out.write_all(&buffer[..n])?;
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// `DownloadCacheCleaner.vacuumCacheDir`: keep the `max_files` files most
/// worth keeping (in the pack folders: the newest, and one per pack before a
/// second of any), then drop the folders left empty.
pub fn vacuum(dir: &Path, max_files: usize) {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() && path.parent() != Some(dir) {
                let time = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((path, time));
            }
        }
    }
    let Some(remove) = files.len().checked_sub(max_files).filter(|n| *n > 0) else {
        return;
    };
    // Newest first, then each file's rank in its folder (0 for a folder's
    // newest), reversed, and stably sorted by rank, highest first.
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut counts = std::collections::HashMap::<PathBuf, usize>::new();
    let mut ranked: Vec<(PathBuf, usize)> = files
        .into_iter()
        .map(|(path, _)| {
            let count = counts
                .entry(path.parent().unwrap_or(dir).to_path_buf())
                .or_default();
            let rank = *count;
            *count += 1;
            (path, rank)
        })
        .collect();
    ranked.reverse();
    ranked.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut emptied = std::collections::HashSet::new();
    for (path, rank) in ranked.into_iter().take(remove) {
        match fs::remove_file(&path) {
            Ok(()) if rank == 0 => {
                emptied.insert(path.parent().unwrap_or(dir).to_path_buf());
            }
            Ok(()) => {}
            Err(e) => log::warn!(
                "Horizon: failed to delete cache file {}: {e}",
                path.display()
            ),
        }
    }
    emptied.remove(dir);
    for d in emptied {
        let _ = fs::remove_dir(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A one-shot HTTP server answering `status` with `body`; the request it got.
    fn serve(status: &str, body: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pack.zip", listener.local_addr().unwrap());
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
            }
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }
    fn sha1(bytes: &[u8]) -> String {
        hex(&Sha1::digest(bytes))
    }
    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-packs-{:016x}", rand_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_pack_lands_under_its_hash_and_is_reused_from_the_cache() {
        let dir = scratch();
        let body = b"PK pack".to_vec();
        let (url, server) = serve("200 OK", body.clone());
        let request = Request {
            id: 7,
            url,
            sha1: Some(sha1(&body)),
        };
        let path = download_file(
            &dir,
            &request,
            &headers("Alex", 0xab),
            MAX_SIZE,
            &channel().0,
        )
        .unwrap();
        assert_eq!(path, dir.join(sha1(&body)));
        assert_eq!(fs::read(&path).unwrap(), body);
        let sent = server.join().unwrap();
        assert!(sent.contains("X-Minecraft-Username: Alex"), "{sent}");
        assert!(sent.contains("X-Minecraft-Pack-Format: 97.1"), "{sent}");
        // Cached: no server this time.
        let cached = Request {
            url: "http://127.0.0.1:9/gone.zip".into(),
            ..request
        };
        assert_eq!(
            download_file(&dir, &cached, &[], MAX_SIZE, &channel().0).unwrap(),
            path
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_wrong_hash_a_dead_url_and_a_large_pack_fail() {
        let dir = scratch();
        let (url, server) = serve("200 OK", b"other bytes".to_vec());
        let wrong = Request {
            id: 1,
            url,
            sha1: Some("0".repeat(40)),
        };
        let err = download_file(&dir, &wrong, &[], MAX_SIZE, &channel().0).unwrap_err();
        assert!(err.contains("did not match requested"), "{err}");
        server.join().unwrap();
        let dead = Request {
            id: 2,
            url: "http://127.0.0.1:9/pack.zip".into(),
            sha1: None,
        };
        assert!(download_file(&dir.join("dead"), &dead, &[], MAX_SIZE, &channel().0).is_err());
        assert!(!dir.join("dead").exists());
        let (url, server) = serve("200 OK", vec![0; 100]);
        let large = Request {
            id: 3,
            url,
            sha1: None,
        };
        let err = download_file(&dir, &large, &[], 10, &channel().0).unwrap_err();
        assert!(err.contains("bigger than maximum"), "{err}");
        let _ = server.join();
        let (url, server) = serve("404 Not Found", Vec::new());
        let missing = Request {
            id: 4,
            url,
            sha1: None,
        };
        assert!(download_file(&dir, &missing, &[], MAX_SIZE, &channel().0).is_err());
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_server_that_goes_silent_fails_the_download() {
        let dir = scratch();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pack.zip", listener.local_addr().unwrap());
        let (release, hold) = channel::<()>();
        let server = std::thread::spawn(move || {
            // Accept, then answer nothing until the test is done.
            let (_stream, _) = listener.accept().unwrap();
            let _ = hold.recv();
        });
        let request = Request {
            id: 9,
            url,
            sha1: None,
        };
        let started = std::time::Instant::now();
        let result = download_file_within(
            &dir,
            &request,
            &[],
            MAX_SIZE,
            &channel().0,
            std::time::Duration::from_millis(300),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        drop(release);
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_queue_logs_each_download() {
        let dir = scratch();
        let (url, server) = serve("200 OK", b"zip".to_vec());
        let queue = Queue::open(dir.clone()).unwrap();
        let (progress, events) = channel();
        let id = 0x5b2c_7a4e_1f3d_4c8b_9a6e_2d4f_8b1c_3e5a;
        queue.download(
            vec![Request {
                id,
                url,
                sha1: None,
            }],
            Vec::new(),
            progress,
        );
        let batch = loop {
            if let Some(b) = queue.finished().pop() {
                break b;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        server.join().unwrap();
        let file = dir
            .join("5b2c7a4e-1f3d-4c8b-9a6e-2d4f8b1c3e5a")
            .join(sha1(b"zip"));
        assert_eq!(batch.downloaded, [(id, file)]);
        // HttpUtil.downloadFile's listener calls, in order.
        assert_eq!(
            events.iter().collect::<Vec<_>>(),
            [
                Progress::RequestStart,
                Progress::DownloadStart(Some(3)),
                Progress::Bytes(3),
                Progress::RequestFinished(true),
            ]
        );
        let log = fs::read_to_string(dir.join("log.json")).unwrap();
        let entry: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(entry["id"], "5b2c7a4e-1f3d-4c8b-9a6e-2d4f8b1c3e5a");
        assert_eq!(entry["file"]["size"], 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_cleaner_keeps_the_newest_file_of_each_pack_longest() {
        let dir = scratch();
        let at = |secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        for (pack, file, time) in [
            ("a", "1", 10),
            ("a", "2", 20),
            ("a", "3", 30),
            ("b", "1", 5),
        ] {
            fs::create_dir_all(dir.join(pack)).unwrap();
            let path = dir.join(pack).join(file);
            fs::write(&path, b"x").unwrap();
            fs::File::options()
                .append(true)
                .open(&path)
                .unwrap()
                .set_modified(at(time))
                .unwrap();
        }
        fs::write(dir.join("log.json"), b"").unwrap();
        vacuum(&dir, 2);
        assert!(dir.join("a/3").exists() && dir.join("b/1").exists());
        assert!(!dir.join("a/1").exists() && !dir.join("a/2").exists());
        assert!(dir.join("log.json").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
