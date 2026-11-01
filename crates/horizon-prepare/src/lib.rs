//! Prepares Horizon's resources from the player's own Minecraft Java Edition: imports the pinned
//! client jar, libraries and version JSON from their official launcher install and the server
//! jar they downloaded (each checked against research-lock.json), copies the game's assets and
//! data, and runs the vanilla exporters (mods/horizon/src/test/java/dev/horizonlab/parity), which
//! call Mojang's own code and so stay Java. The result, `.local/horizon-client/<version>` with its
//! `horizon-resources.json` manifest, is what the game's `--resources` and the launcher read.
//!
//! This is the Rust port of the Python preparer (tools/prepare_native_client.py and the helpers
//! it imported); its files are byte for byte the ones the script wrote (tools/compare_prepare.py
//! checks it), so the manifest's `schema` is unchanged. Unchanged steps are skipped on the next
//! run, keyed by the sha256 of their inputs and outputs recorded in the manifest, as before.

pub mod artifacts;
mod check;
pub mod exporters;
mod files;
mod inventory;
pub mod java;
mod pool;
pub mod pyjson;
mod sounds;
mod survival;
mod worker;

pub use check::{check, missing_extras, STAMP};
pub use files::clone_or_copy;
pub use sounds::{add_assets, fonts, languages, packs, panorama, sounds, BUILT_IN_PACKS};

use pyjson::Value;
use sha2::Digest;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Where progress lines go: the terminal for the binary, the log pane for the launcher.
pub type Log<'a> = &'a dyn Fn(&str);

/// Where the fraction done goes (`--progress json`), from 0 to 1.
pub type Progress<'a> = &'a dyn Fn(f32);

/// The stages of `steps`, each opened by `Run::stage`: what the fraction counts.
const STAGES: u32 = 18;

/// The manifest's `schema`: the prepared files' layout, which the launcher compares.
pub const SCHEMA: i64 = 7;

/// What this preparer makes. The launcher copies a prepared version's resources to a newly installed
/// version whose preparer has the same number (and the same pinned jars), so a release that changes
/// this crate without changing its output asks nobody to prepare again. Bump it, and append the new
/// digest to output-versions.txt, when the prepared files change: CI prepares with every change to
/// the preparer and fails when the output is not the one output-versions.txt gives this number
/// (`python3 tools/prepare_output.py check`). The launcher reads this line from the version
/// folder's sources: keep it on one line, as it is.
pub const OUTPUT_VERSION: u32 = 3;

/// Digest of this crate's sources (build.rs): the launcher prepares a version in-process when
/// that version ships these same sources, and runs the version's own binary otherwise.
pub const SOURCES_SHA256: &str = env!("HORIZON_PREPARE_SOURCES_SHA256");

/// `SOURCES_SHA256` of the sources in `crate_dir` (a version folder's crates/horizon-prepare),
/// computed as build.rs computes it; None when the folder has none.
pub fn sources_sha256(crate_dir: &Path) -> Option<String> {
    let mut files = Vec::new();
    for name in ["Cargo.toml", "build.rs"] {
        files.push((name.to_string(), fs::read(crate_dir.join(name)).ok()?));
    }
    let mut sources: Vec<String> = fs::read_dir(crate_dir.join("src"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    sources.sort();
    for name in sources {
        let data = fs::read(crate_dir.join("src").join(&name)).ok()?;
        files.push((format!("src/{name}"), data));
    }
    let mut hash = sha2::Sha256::new();
    for (name, data) in files {
        // Line endings as checked out: a Windows checkout's CRLF is the same source.
        let data: Vec<u8> = data.into_iter().filter(|b| *b != b'\r').collect();
        hash.update(format!("{name}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hex(&hash.finalize()))
}

pub struct Options {
    /// The version folder (a release folder, or a checkout): research-lock.json, the exporters'
    /// Java sources, optional precompiled `exporters/`, and `.local/` where everything is written.
    pub root: PathBuf,
    /// Import the pinned game files from the player's own copy first (`--prepare`).
    pub import: bool,
    /// The official launcher's game directory (`--minecraft-dir`, else HORIZON_MINECRAFT_DIR,
    /// else the launcher's own).
    pub minecraft_dir: Option<PathBuf>,
    /// The pinned server jar the player downloaded (`--server-jar`, else HORIZON_SERVER_JAR).
    pub server_jar: Option<PathBuf>,
    /// The Java runtime to try after JAVA_HOME and the PATH (the launcher's pick, e.g. Minecraft's
    /// own Java it downloaded from Mojang).
    pub java_home: Option<PathBuf>,
    /// Files named one by one instead of found in `minecraft_dir` (`--version-json`,
    /// `--client-jar`, `--libraries-dir`, `--assets-dir`).
    pub inputs: artifacts::Inputs,
    /// Where the resources go (`--output`); None for `root`'s `.local/horizon-client/<version>`.
    /// The exporters' scratch files stay in `root`'s `.local/` either way.
    pub output: Option<PathBuf>,
}

/// The prepared resources' folder: `output`, else `root`'s `.local/horizon-client/<version>`.
pub fn resources_dir(root: &Path, version: &str, output: Option<&Path>) -> PathBuf {
    match output {
        Some(output) => output.to_path_buf(),
        None => root.join(".local/horizon-client").join(version),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex(&sha2::Sha256::digest(data))
}

pub(crate) fn digest(path: &Path, algorithm: &str) -> Result<String, String> {
    fn stream<D: Digest>(mut file: fs::File) -> std::io::Result<String> {
        let mut hash = D::new();
        let mut buffer = vec![0; 1 << 20];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                return Ok(hex(&hash.finalize()));
            }
            hash.update(&buffer[..n]);
        }
    }
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match algorithm {
        "sha1" => stream::<sha1::Sha1>(file),
        _ => stream::<sha2::Sha256>(file),
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

fn mkdir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The lock file each preparation holds in the folders it writes (`LOCK`).
pub const LOCK: &str = ".horizon-prepare.lock";

/// One preparation at a time per folder: two launchers or instances preparing into one `--output`,
/// or sharing `root`'s `.local`, would interleave their writes. A second one waits, saying so;
/// the lock goes with the returned file, so with the process too.
pub(crate) fn hold_lock(dir: &Path, log: Log) -> Result<fs::File, String> {
    mkdir(dir)?;
    let path = dir.join(LOCK);
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(error)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            log(&format!(
                "Waiting for another horizon-prepare to finish with {}",
                dir.display()
            ));
            file.lock().map_err(error)?;
        }
        Err(fs::TryLockError::Error(e)) => return Err(error(e)),
    }
    Ok(file)
}

/// Whether two folders are one, through `..`, links and case; a missing folder is created first.
fn same_dir(a: &Path, b: &Path) -> Result<bool, String> {
    let real = |dir: &Path| {
        mkdir(dir)?;
        fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))
    };
    Ok(real(a)? == real(b)?)
}

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let text = String::from_utf8(read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    pyjson::loads(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Python's `Path.write_text`: text mode, so `\n` is the platform's line ending.
fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if cfg!(windows) {
        write(path, text.replace('\n', "\r\n").as_bytes())
    } else {
        write(path, text.as_bytes())
    }
}

/// tools/lab.py's `write_json`: indented by two, with a final newline.
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    mkdir(path.parent().unwrap())?;
    write_text(path, &(pyjson::dumps(value, &pyjson::INDENT2) + "\n"))
}

/// Writes `data` unless the file already holds it, so unchanged files keep their times.
fn write_if_changed(target: &Path, data: &[u8]) -> Result<(), String> {
    if !(target.exists() && read(target)? == data) {
        mkdir(target.parent().unwrap())?;
        write(target, data)?;
    }
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    write(to, &read(from)?)
}

/// `" in 12.3 s"`: how long a step took, for the progress lines.
pub(crate) fn took(started: Instant) -> String {
    format!(" in {:.1} s", started.elapsed().as_secs_f64())
}

/// `path` relative to `root`, with the platform's separators, as Python's
/// `str(path.relative_to(ROOT))` prints it.
pub(crate) fn relative_display(path: &Path, root: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))?;
    Ok(relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(std::path::MAIN_SEPARATOR_STR))
}

/// Python's `str.splitlines`.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            lines.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|(_, n)| *n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// An open jar. Its clones share the parsed member list (the client jar's is ~10 MB) and read
/// at their own offsets, so the threads reading one jar each take a clone, not a new parse.
pub(crate) type Jar = zip::ZipArchive<JarFile>;

pub(crate) fn open_zip(path: &Path) -> Result<Jar, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    let file = JarFile {
        file: std::sync::Arc::new(file),
        position: 0,
        len,
    };
    zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))
}

/// A file read with positioned reads: clones share the handle but not the position.
#[derive(Clone)]
pub(crate) struct JarFile {
    file: std::sync::Arc<fs::File>,
    position: u64,
    len: u64,
}

impl Read for JarFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(&*self.file, buf, self.position)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(&*self.file, buf, self.position)?;
        self.position += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for JarFile {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let (base, offset) = match to {
            std::io::SeekFrom::Start(n) => (n, 0),
            std::io::SeekFrom::End(n) => (self.len, n),
            std::io::SeekFrom::Current(n) => (self.position, n),
        };
        self.position = base.checked_add_signed(offset).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start")
        })?;
        Ok(self.position)
    }
}

pub(crate) fn zip_bytes(archive: &mut Jar, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut data)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(data)
}

pub(crate) fn zip_text(archive: &mut Jar, name: &str) -> Result<String, String> {
    String::from_utf8(zip_bytes(archive, name)?).map_err(|e| format!("{name}: {e}"))
}

/// The archive's member names in its own order (Python's `namelist`).
fn zip_names(archive: &Jar) -> Result<Vec<String>, String> {
    archive
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

/// `{"minecraft:<path>": text}` for every `<prefix><path>.json`, sorted by member name
/// (the comprehension the Python preparer used for tags, worldgen and damage resources).
fn json_resources(archive: &mut Jar, prefix: &str) -> Result<Value, String> {
    let mut names: Vec<String> = zip_names(archive)?
        .into_iter()
        .filter(|n| n.starts_with(prefix) && n.ends_with(".json"))
        .collect();
    names.sort();
    let mut resources = Value::Object(Vec::new());
    for name in names {
        let text = zip_text(archive, &name)?;
        let key = format!("minecraft:{}", &name[prefix.len()..name.len() - 5]);
        resources.insert(&key, Value::Str(text));
    }
    Ok(resources)
}

/// A relative member path with no `..` and not absolute (Python's `Path(name)` checks).
fn member_path(name: &str, what: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if path.is_absolute()
        || name.starts_with('/')
        || path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(what.to_string());
    }
    Ok(path.to_path_buf())
}

/// `{relative path: sha256}` of exporter sources, in the order given
/// (tools/test_rust_parity.py's `source_hashes`).
fn source_hashes(root: &Path, sources: &[&Path]) -> Result<Value, String> {
    let mut hashes = Value::Object(Vec::new());
    for source in sources {
        hashes.insert(
            &relative_display(source, root)?,
            Value::Str(digest(source, "sha256")?),
        );
    }
    Ok(hashes)
}

/// A classpath entry list with the platform's separator.
fn classpath_join(head: &Path, rest: &[PathBuf]) -> String {
    java::classpath(std::iter::once(head).chain(rest.iter().map(PathBuf::as_path)))
}

struct Run<'a> {
    root: PathBuf,
    local: PathBuf,
    resources: PathBuf,
    jdk: java::Jdk,
    java_major: i64,
    server: PathBuf,
    client: PathBuf,
    /// The server jar and its libraries: the classpath of the server-side exporters.
    server_classpath: String,
    /// The client jar and its libraries: the classpath of the client-side exporters.
    client_classpath: String,
    /// The compiled exporter classes, or why they didn't compile: one attempt for all lanes.
    bundles: Mutex<Option<Result<PathBuf, String>>>,
    /// The exporter JVMs, which run at the same time: the server's, the client's, and on a
    /// machine with cores to spare a second server one for the generator's block behaviour.
    workers: [Mutex<Option<worker::Worker>>; 3],
    previous: Value,
    origin: Value,
    /// Progress lines from every lane; the caller's thread prints them.
    log: &'a (dyn Fn(&str) + Sync),
    /// Tells the caller's thread that a lane opened a stage of `steps`: it counts them.
    stage: &'a (dyn Fn() + Sync),
    inputs: artifacts::Inputs,
}

const PARITY: &str = "mods/horizon/src/test/java/dev/horizonlab/parity";

impl Run<'_> {
    /// Opens the next stage of `steps`.
    fn stage(&self) {
        (self.stage)();
    }

    /// The exporter classes, compiled once: a lane that needs them while another compiles waits,
    /// and a failed compile fails every lane without compiling again.
    fn bundle(&self) -> Result<PathBuf, String> {
        self.bundles
            .lock()
            .unwrap()
            .get_or_insert_with(|| exporters::classes(self))
            .clone()
    }

    /// Stops an exporter JVM once its lane needs it no more, to free its memory.
    fn close_worker(&self, worker: usize) {
        drop(self.workers[worker].lock().unwrap().take());
    }

    /// The worker of a classpath's side: 0 for the server's, 1 for the client's.
    fn side(&self, classpath: &str) -> usize {
        usize::from(classpath == self.client_classpath)
    }

    fn oracle(&self, name: &str) -> PathBuf {
        self.root.join(PARITY).join(name)
    }

    fn previous(&self, key: &str) -> Option<&Value> {
        self.previous.get(key)
    }

    fn same(&self, key: &str) -> bool {
        self.previous(key) == self.origin.get(key)
    }

    fn file_hash(&self, name: &str) -> Result<Value, String> {
        Ok(Value::Str(digest(&self.resources.join(name), "sha256")?))
    }

    /// Every file exists and has the hash the previous manifest recorded under `key`.
    fn files_unchanged(&self, key: &str, names: &[&str]) -> Result<bool, String> {
        for name in names {
            if !self.resources.join(name).exists()
                || self.previous(key).and_then(|h| h.get(name)) != Some(&self.file_hash(name)?)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn log_file(&self, path: &Path) -> Result<fs::File, String> {
        fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `java -Xmx<heap> -cp <classes>:<classpath> dev.horizonlab.parity.<main> <args>`, or the
    /// same task on exporter JVM `worker`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        worker: usize,
        heap: &str,
        classpath: &str,
        main: &str,
        args: &[String],
        log: &fs::File,
        log_path: &Path,
    ) -> Result<(), String> {
        let side = self.side(classpath);
        let classes = self
            .bundle()?
            .join(if side == 1 { "client" } else { "server" });
        if !matches!(
            main,
            "VanillaCommandParseOracle" | "VanillaCreativeTabsExporter"
        ) && std::env::var_os("HORIZON_PREPARE_ISOLATED").is_none()
        {
            let name = ["server", "client", "server-blocks"][worker];
            let mut worker = self.workers[worker].lock().unwrap();
            if worker.is_none() {
                *worker = Some(worker::Worker::new(self, name, &classes, classpath)?);
            }
            return worker.as_mut().unwrap().run(main, args, log_path);
        }
        let separator = if cfg!(windows) { ";" } else { ":" };
        let mut all = vec![
            format!("-Xmx{heap}"),
            "-cp".into(),
            format!("{}{separator}{classpath}", classes.display()),
            format!("dev.horizonlab.parity.{main}"),
        ];
        all.extend_from_slice(args);
        // Vanilla's code in the exporters logs relative to the working directory (logs/): keep
        // that in the scratch, not where the preparer was started.
        self.jdk.java(&all, Some(&self.local), log)
    }

    /// Runs each exporter from the verified shared bundle, with output in
    /// the log at `log`: the shape of every exporter step.
    #[allow(clippy::too_many_arguments)]
    fn export(
        &self,
        what: &str,
        classpath: &str,
        classes: &Path,
        mains: &[&str],
        heap: &str,
        args: &[String],
        log: &Path,
    ) -> Result<(), String> {
        let worker = self.side(classpath);
        self.export_on(worker, what, classpath, classes, mains, heap, args, log)
    }

    /// `export`, on exporter JVM `worker`.
    #[allow(clippy::too_many_arguments)]
    fn export_on(
        &self,
        worker: usize,
        what: &str,
        classpath: &str,
        classes: &Path,
        mains: &[&str],
        heap: &str,
        args: &[String],
        log: &Path,
    ) -> Result<(), String> {
        (self.log)(&format!("Exporting {what} ({})", mains.join(", ")));
        let fail = |e: String| format!("{e} while exporting {what}; see {}", log.display());
        // Compiled first, so the time is this step's (the JVM's start included).
        self.bundle().map_err(fail)?;
        let started = Instant::now();
        mkdir(classes)?;
        let file = self.log_file(log)?;
        for main in mains {
            self.run(worker, heap, classpath, main, args, &file, log)
                .map_err(fail)?;
        }
        (self.log)(&format!("Exported {what}{}", took(started)));
        Ok(())
    }
}

/// Prepares the resources; returns their folder (`resources_dir`).
pub fn prepare(options: &Options, log: Log) -> Result<PathBuf, String> {
    prepare_impl(options, log, &|_| {}, false)
}

/// `prepare`, telling `progress` the fraction done as each stage begins, and 1 at the end.
pub fn prepare_with_progress(
    options: &Options,
    log: Log,
    progress: Progress,
) -> Result<PathBuf, String> {
    prepare_impl(options, log, progress, false)
}

/// Release build tool: compile and link-check only Horizon's original exporter classes.
pub fn compile_exporters(options: &Options, log: Log) -> Result<PathBuf, String> {
    prepare_impl(options, log, &|_| {}, true)
}

fn prepare_impl(
    options: &Options,
    log: Log,
    progress: Progress,
    compile_only: bool,
) -> Result<PathBuf, String> {
    let started = Instant::now();
    let root = &std::path::absolute(&options.root)
        .map_err(|e| format!("{}: {e}", options.root.display()))?;
    let lock = artifacts::Lock::read(root)?;
    // Java first: nothing below can finish without it, and the player should hear it at once.
    let mut homes: Vec<PathBuf> = options.java_home.iter().cloned().collect();
    homes.extend(java::minecraft_runtimes(
        &artifacts::game_files(options.minecraft_dir.as_deref(), &lock.version).runtimes,
    ));
    let jdk = java::find_runtime(lock.java_major, &homes)?;
    // The import and the exporters' scratch files.
    let _local_lock = hold_lock(&root.join(".local"), log)?;
    let reused = if options.import {
        artifacts::reuse_verified(root, log)?
    } else {
        None
    };
    let (found, client_libraries) = match reused {
        Some(checked) => checked,
        None => {
            if options.import {
                artifacts::import(
                    root,
                    &lock,
                    options.minecraft_dir.as_deref(),
                    &options.inputs,
                    options.server_jar.as_deref(),
                    log,
                )?;
            }
            let found = artifacts::verify(&lock)?;
            let libraries = artifacts::client_libraries(&lock, &found.manifest)?;
            (found, libraries)
        }
    };
    let local = root.join(".local");
    let resources = match &options.output {
        Some(output) => {
            std::path::absolute(output).map_err(|e| format!("{}: {e}", output.display()))?
        }
        None => resources_dir(root, &lock.version, None),
    };
    // `--output` may name the `.local` folder itself, whose lock is held already: a second
    // handle on the same lock file would wait for this process forever.
    let _resources_lock = if same_dir(&resources, &local)? {
        None
    } else {
        Some(hold_lock(&resources, log)?)
    };
    let manifest_path = resources.join("horizon-resources.json");
    let oracle = root.join(PARITY).join("VanillaHeightmapOracle.java");
    let jar_hashes = pool::map(&[&found.jars["client"], &found.jars["server"]], |jar| {
        digest(jar, "sha256")
    })?;
    let origin = Value::Object(vec![
        ("minecraft".into(), Value::Str(lock.version.clone())),
        ("client_sha256".into(), Value::Str(jar_hashes[0].clone())),
        ("server_sha256".into(), Value::Str(jar_hashes[1].clone())),
        (
            "exporter_sha256".into(),
            Value::Str(digest(&oracle, "sha256")?),
        ),
    ]);
    let previous = if manifest_path.exists() {
        read_json(&manifest_path)?
    } else {
        Value::Object(Vec::new())
    };
    log(&format!("Checked the game files{}", took(started)));
    // The steps run on lanes of their own; their progress lines and stages come back here,
    // to `log` and `progress`.
    let (send, lines) = std::sync::mpsc::channel::<Option<String>>();
    let minecraft = options.minecraft_dir.as_deref();
    let result = std::thread::scope(|scope| {
        let work = scope.spawn(move || {
            let staged = send.clone();
            let say = move |line: &str| {
                let _ = send.send(Some(line.to_string()));
            };
            let stage = move || {
                let _ = staged.send(None);
            };
            let run = Run {
                root: root.clone(),
                local,
                resources: resources.clone(),
                jdk,
                java_major: lock.java_major,
                server: found.jars["server"].clone(),
                client: found.jars["client"].clone(),
                server_classpath: classpath_join(&found.jars["server"], &found.libraries),
                client_classpath: classpath_join(&found.jars["client"], &client_libraries),
                bundles: Mutex::new(None),
                workers: [Mutex::new(None), Mutex::new(None), Mutex::new(None)],
                previous,
                origin,
                log: &say,
                stage: &stage,
                inputs: options.inputs.clone(),
            };
            if compile_only {
                return run.bundle();
            }
            // Not current until this preparation finishes: --check says so meanwhile, and
            // after a failure.
            check::remove_stamp(&resources)?;
            steps(&run, &lock, &oracle, minecraft)?;
            check::write_stamp(&resources)?;
            Ok(resources)
        });
        relay(lines, log, progress);
        work.join()
            .unwrap_or_else(|_| Err("The preparation stopped unexpectedly".into()))
    })?;
    if !compile_only {
        progress(1.0);
        log(&format!(
            "Prepared Horizon native-client resources: {}",
            result.display()
        ));
    }
    log(&format!("Done{}", took(started)));
    Ok(result)
}

/// Hands the lanes' messages to the caller's `log` and `progress`: a line, or None for a stage
/// opened. The fraction done is the stages opened before the one opening.
fn relay(messages: std::sync::mpsc::Receiver<Option<String>>, log: Log, progress: Progress) {
    let mut stages = 0;
    for message in messages {
        match message {
            Some(line) => log(&line),
            None => {
                progress(stages.min(STAGES) as f32 / STAGES as f32);
                stages += 1;
            }
        }
    }
}

fn arg(path: &Path) -> String {
    path.display().to_string()
}

/// The manifest entries a lane works out, by key.
type Entries = Vec<(&'static str, Value)>;

/// The exporters, the data generator and the asset copy, on four lanes (five with the split
/// below) that run at once unless `pool::jobs` is 1:
/// the server exporter JVM's steps in a fixed order (they share its state), the client
/// exporter JVM's then the command registries' JVM, the data generator's then the creative
/// inventory's, and the copying, which needs no Java. At most three JVMs run at a time, four
/// with the split below.
/// From this many processors (`pool::jobs`) the generator's block behaviour gets its own
/// server JVM.
/// It pays a second Bootstrap and ~400 MB, which only pays off when the other lanes leave cores
/// idle: a 6-core laptop finished its other JVMs 20 s before the server lane.
const SPLIT_BLOCKS_JOBS: usize = 6;

fn steps(
    run: &Run,
    lock: &artifacts::Lock,
    oracle: &Path,
    minecraft: Option<&Path>,
) -> Result<(), String> {
    let root = &run.resources;
    let repo = &run.root;
    let server_cp = run.server_classpath.as_str();
    let client_cp = run.client_classpath.as_str();

    run.stage();
    let block_tags = "data/minecraft/tags/block/";
    let tags = json_resources(&mut open_zip(&run.server)?, block_tags)?;
    if tags != json_resources(&mut open_zip(&run.client)?, block_tags)? {
        return Err("Reference block tags disagree".into());
    }
    write_json(&root.join("block-tags.json"), &tags)?;
    let tags = &tags;

    let binding = run.oracle("VanillaBlockStateBindingOracle.java");
    let helper = run.oracle("VanillaCuboidModelOracle.java");
    let binding_sources = source_hashes(repo, &[&binding, &helper])?;
    let visibility = run.oracle("VanillaFaceVisibilityOracle.java");
    let collision = run.oracle("VanillaCollisionOracle.java");
    let collision_sources = source_hashes(repo, &[&collision])?;
    let movement = run.oracle("VanillaMovementRegistryOracle.java");
    let attributes = run.oracle("VanillaEntityAttributeOracle.java");
    let landing = run.oracle("VanillaBlockLandingOracle.java");
    let motion_sources = source_hashes(
        repo,
        &[&movement, &attributes, oracle, &landing, &collision],
    )?;
    let motion_files = [
        "movement-properties.nbt",
        "attributes.json",
        "landing-definitions.json",
    ];
    let damage = run.oracle("VanillaDamageSourceOracle.java");
    let health = run.oracle("VanillaPlayerHealthOracle.java");
    let health_sources = source_hashes(repo, &[&damage, &health, &attributes])?;
    let health_files = [
        "damage-types.json",
        "damage-tags.json",
        "damage-catalog.json",
        "player-float-ids.json",
    ];
    let render_sources = source_hashes(repo, &[&visibility, &collision, oracle])?;
    let render_files = ["render-definitions.json", "occlusion.nbt"];
    let command_oracle = run.oracle("VanillaCommandParseOracle.java");
    let command_sources = source_hashes(repo, &[&command_oracle])?;

    let split_blocks = pool::jobs() >= SPLIT_BLOCKS_JOBS;

    // One file each, exported again when the exporter or the jar it reads changed.
    let single = |what: &str,
                  file: &str,
                  oracles: &[&str],
                  key: &str,
                  jar: &str,
                  classes: &str,
                  heap: &str,
                  extra: &[String],
                  log: &str|
     -> Result<Value, String> {
        let target = root.join(file);
        let sources: Vec<PathBuf> = oracles.iter().map(|o| run.oracle(o)).collect();
        let sources: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
        let hashes = source_hashes(repo, &sources)?;
        if !(run.previous(key) == Some(&hashes) && target.exists() && run.same(jar)) {
            let main = oracles[0].strip_suffix(".java").unwrap();
            let mut args = vec![arg(&target)];
            args.extend_from_slice(extra);
            run.export(
                what,
                if jar == "client_sha256" {
                    client_cp
                } else {
                    server_cp
                },
                &run.local.join(classes),
                &[main],
                heap,
                &args,
                &root.join(log),
            )?;
        }
        Ok(hashes)
    };
    let single = &single;

    // The server exporter JVM, in the order these steps always ran.
    let server: pool::Lane<Entries> = Box::new(|| {
        // Every block state and its facts.
        let facts = root.join("facts.json");
        let origin_same = run
            .origin
            .as_object()
            .unwrap()
            .iter()
            .all(|(k, v)| run.previous(k) == Some(v));
        run.stage();
        if !(origin_same
            && facts.exists()
            && run.previous("facts_sha256") == Some(&run.file_hash("facts.json")?))
        {
            run.export(
                "block states",
                server_cp,
                &root.join("classes"),
                &["VanillaHeightmapOracle"],
                "512M",
                &["export".into(), arg(&facts)],
                &root.join("export.log"),
            )?;
        }

        run.stage();
        if !(run.previous("motion_exporters_sha256") == Some(&motion_sources)
            && run.same("server_sha256")
            && run.previous("block_tags_sha256") == Some(&run.file_hash("block-tags.json")?)
            && run.files_unchanged("motion_files_sha256", &motion_files)?)
        {
            let export = run.local.join("horizon-client-motion");
            mkdir(&export)?;
            write_json(&export.join("tags.json"), tags)?;
            run.export(
                "movement and attributes",
                server_cp,
                &export.join("classes"),
                &["VanillaBlockLandingOracle", "VanillaEntityAttributeOracle"],
                "768M",
                &["export".into(), arg(&export)],
                &export.join("export.log"),
            )?;
            copy(
                &export.join("movement-properties.nbt"),
                &root.join("movement-properties.nbt"),
            )?;
            copy(&export.join("catalog.json"), &root.join("attributes.json"))?;
            copy(
                &export.join("landing-definitions.json"),
                &root.join("landing-definitions.json"),
            )?;
        }

        run.stage();
        if !(run.previous("health_exporters_sha256") == Some(&health_sources)
            && run.same("server_sha256")
            && run.files_unchanged("health_files_sha256", &health_files)?)
        {
            let export = run.local.join("horizon-client-health");
            mkdir(&export)?;
            let mut server = open_zip(&run.server)?;
            let mut client = open_zip(&run.client)?;
            for (file, prefix) in [
                ("types.json", "data/minecraft/damage_type/"),
                ("tags.json", "data/minecraft/tags/damage_type/"),
            ] {
                let resources = json_resources(&mut server, prefix)?;
                if resources != json_resources(&mut client, prefix)? {
                    return Err("Reference damage resources disagree".into());
                }
                write_json(&export.join(file), &resources)?;
            }
            run.export(
                "damage and health",
                server_cp,
                &export.join("classes"),
                &["VanillaPlayerHealthOracle", "VanillaDamageSourceOracle"],
                "768M",
                &["export".into(), arg(&export)],
                &export.join("export.log"),
            )?;
            for (source, target) in [
                ("types.json", "damage-types.json"),
                ("tags.json", "damage-tags.json"),
                ("catalog.json", "damage-catalog.json"),
                ("ids.json", "player-float-ids.json"),
            ] {
                copy(&export.join(source), &root.join(target))?;
            }
        }

        let collision_file = root.join("cached-collision.nbt");
        run.stage();
        if !(run.previous("collision_exporters_sha256") == Some(&collision_sources)
            && run.same("server_sha256")
            && collision_file.exists()
            && run.previous("collision_sha256") == Some(&run.file_hash("cached-collision.nbt")?))
        {
            let export = run.local.join("horizon-client-collision");
            mkdir(&export)?;
            run.export(
                "collision shapes",
                server_cp,
                &export.join("classes"),
                &["VanillaCollisionOracle"],
                "768M",
                &["generate".into(), arg(&export)],
                &export.join("export.log"),
            )?;
            copy(&export.join("cached-collision.nbt"), &collision_file)?;
        }

        run.stage();
        if !(run.previous("render_exporters_sha256") == Some(&render_sources)
            && run.same("server_sha256")
            && run.previous("block_tags_sha256") == Some(&run.file_hash("block-tags.json")?)
            && run.files_unchanged("render_files_sha256", &render_files)?)
        {
            let export = run.local.join("horizon-client-render");
            mkdir(&export)?;
            write_json(&export.join("tags.json"), tags)?;
            run.export(
                "face visibility",
                server_cp,
                &export.join("classes"),
                &["VanillaFaceVisibilityOracle"],
                "768M",
                &["export".into(), arg(&export)],
                &export.join("export.log"),
            )?;
            copy(
                &export.join("definitions.json"),
                &root.join("render-definitions.json"),
            )?;
            copy(&export.join("occlusion.nbt"), &root.join("occlusion.nbt"))?;
        }

        run.stage();
        let gameplay = single(
            "gameplay facts",
            "gameplay-facts.json",
            &["VanillaGameplayFactsExporter.java"],
            "gameplay_exporters_sha256",
            "server_sha256",
            "gameplay-facts/classes",
            "1G",
            &[],
            "gameplay-export.log",
        )?;
        // Entity dimensions, categories and default attributes for the client's mobs.
        run.stage();
        let entity_types = single(
            "entity types",
            "entity-types.json",
            &["VanillaEntityTypesExporter.java"],
            "entity_types_exporters_sha256",
            "server_sha256",
            "entity-types/classes",
            "1G",
            &[],
            "entity-types-export.log",
        )?;
        // Block classes, instance configuration and per-state support/signal/fluid
        // facts for horizon-world's level behaviour (level::Behaviours::load).
        run.stage();
        let behaviour = single(
            "block behaviours",
            "block-behaviours.json",
            &["VanillaBlockBehaviourExporter.java"],
            "behaviour_exporters_sha256",
            "server_sha256",
            "block-behaviours/classes",
            "1G",
            &[],
            "behaviour-export.log",
        )?;
        if !split_blocks {
            worldgen_blocks(run, 0)?;
        }
        run.close_worker(0);
        Ok(vec![
            ("gameplay_exporters_sha256", gameplay),
            ("entity_types_exporters_sha256", entity_types),
            ("behaviour_exporters_sha256", behaviour),
        ])
    });

    // The client exporter JVM, then the command registries' own.
    let client: pool::Lane<Entries> = Box::new(|| {
        // Block state to model bindings, from the client.
        let model_states = root.join("model-states.json");
        run.stage();
        if !(run.previous("model_state_exporters_sha256") == Some(&binding_sources)
            && run.same("client_sha256")
            && model_states.exists()
            && run.previous("model_states_sha256") == Some(&run.file_hash("model-states.json")?))
        {
            run.export(
                "model bindings",
                client_cp,
                &root.join("client-classes"),
                &["VanillaBlockStateBindingOracle"],
                "512M",
                &["export".into(), arg(&model_states)],
                &root.join("model-state-export.log"),
            )?;
        }
        run.stage();
        let entity_models = single(
            "entity models",
            "entity-models.json",
            &["VanillaEntityModelExporter.java"],
            "entity_model_exporters_sha256",
            "client_sha256",
            "entity-models/classes",
            "1G",
            &[],
            "entity-model-export.log",
        )?;
        // Mob AI and animation facts: per-state path types, spawn placements and the
        // client's keyframe animation definitions (needs the client jar).
        run.stage();
        let mob_data = single(
            "mob data",
            "mob-data.json",
            &["VanillaMobDataExporter.java"],
            "mob_data_exporters_sha256",
            "client_sha256",
            "mob-data/classes",
            "1G",
            &[arg(&run.client)],
            "mob-data-export.log",
        )?;
        run.close_worker(1);

        // Vanilla's built-in registries for in-game command parsing (mob effects,
        // data component types, dialogs, block entity and tag members).
        let command_registries = root.join("command-registries.json");
        run.stage();
        if !(run.previous("command_registries_exporters_sha256") == Some(&command_sources)
            && command_registries.exists()
            && run.same("server_sha256"))
        {
            let export = run.local.join("command-registries");
            run.export(
                "command registries",
                server_cp,
                &export.join("classes"),
                &["VanillaCommandParseOracle"],
                "1G",
                &["registries".into(), arg(&export)],
                &root.join("command-registries-export.log"),
            )?;
            copy(&export.join("registries.json"), &command_registries)?;
        }
        Ok(vec![
            ("entity_model_exporters_sha256", entity_models),
            ("mob_data_exporters_sha256", mob_data),
        ])
    });

    // The data generator's JVM, then the creative inventory's.
    let reports: pool::Lane<Entries> = Box::new(|| {
        run.stage();
        if datagen_reports_missing(root) || !run.same("server_sha256") {
            // Vanilla's own data generator reports every item's default components.
            (run.log)("Running vanilla's data generator reports");
            let started = Instant::now();
            let datagen = run.local.join("datagen");
            mkdir(&datagen)?;
            let log_path = datagen.join("datagen.log");
            let log = run.log_file(&log_path)?;
            run.jdk
                .java(
                    &[
                        "-DbundlerMainClass=net.minecraft.data.Main".into(),
                        "-jar".into(),
                        arg(&lock.directory.join("jars").join("server.jar")),
                        "--reports".into(),
                        "--output".into(),
                        "out".into(),
                    ],
                    Some(&datagen),
                    &log,
                )
                .map_err(|e| format!("{e} in the data generator; see {}", log_path.display()))?;
            let reports = datagen.join("out").join("reports");
            install_datagen_reports(root, &reports)?;
            (run.log)(&format!("Ran the data generator{}", took(started)));
        }

        // The registry and command reports, which the game reads next to these
        // files when it serves players (a dedicated server, a LAN world): kept
        // here too, so this folder works when it is copied or mounted alone.
        let reports = run.local.join("datagen").join("out").join("reports");
        for name in ["registries.json", "commands.json"] {
            let (from, to) = (reports.join(name), root.join(name));
            if from.is_file() && (!to.exists() || !run.same("server_sha256")) {
                copy(&from, &to)?;
            }
        }

        // The creative inventory (creative-inventory.json): every tab built by vanilla's
        // CreativeModeTabs over the vanilla data pack's registries, with the search
        // tab's tooltip lines and tags.
        run.stage();
        let creative = single(
            "the creative inventory",
            "creative-inventory.json",
            &["VanillaCreativeTabsExporter.java", "ParityData.java"],
            "creative_exporters_sha256",
            "server_sha256",
            "creative-inventory/classes",
            "2G",
            &[],
            "creative-export.log",
        )?;
        Ok(vec![("creative_exporters_sha256", creative)])
    });

    // The playable client needs every client asset (textures, models, fonts, GUI)
    // and every data-pack resource (worldgen, recipes, loot tables, tags), and
    // the vanilla pack's icon (`pack.png`) for the pack screen.
    let files: pool::Lane<Entries> = Box::new(|| {
        run.stage();
        (run.log)("Copying the client's assets and data");
        let started = Instant::now();
        let mut indexed = sounds::Assets::load(repo, Some(root), minecraft, &run.inputs, run.log);
        let (version, count) = extract_client_resources(&run.client, root, indexed.as_mut().ok())?;
        sounds::optional_imports(indexed.as_mut().map_err(|e| e.as_str()), run.log);
        (run.log)(&format!(
            "Copied the client's assets and data{}",
            took(started)
        ));
        worldgen_bundles(run)?;
        Ok(vec![
            (
                "data_version",
                version
                    .get("world_version")
                    .cloned()
                    .ok_or("version.json has no world_version")?,
            ),
            ("resource_files", Value::Int(count.to_string())),
        ])
    });

    let mut lanes = vec![server, client, reports, files];
    if split_blocks {
        // With cores to spare, the generator's block behaviour (a quarter of the server lane's
        // time) runs on a second server JVM beside the first; it pays its own Bootstrap.
        lanes.push(Box::new(|| {
            worldgen_blocks(run, 2)?;
            run.close_worker(2);
            Ok(Vec::new())
        }));
    }
    let entries: Entries = pool::lanes(lanes)?.into_iter().flatten().collect();
    // Biome parameter tables come from the data generator reports above.
    run.stage();
    worldgen_tables(run)?;

    let entry = |key: &str| entries.iter().find(|(k, _)| *k == key).unwrap().1.clone();
    let file_hashes = |names: &[&str]| -> Result<Value, String> {
        Ok(Value::Object(
            names
                .iter()
                .map(|n| Ok((n.to_string(), run.file_hash(n)?)))
                .collect::<Result<_, String>>()?,
        ))
    };
    run.stage();
    let mut manifest = vec![("schema".to_string(), Value::Int(SCHEMA.to_string()))];
    manifest.extend(run.origin.as_object().unwrap().iter().cloned());
    manifest.extend([
        ("health_exporters_sha256".into(), health_sources),
        ("health_files_sha256".into(), file_hashes(&health_files)?),
        ("motion_exporters_sha256".into(), motion_sources),
        ("motion_files_sha256".into(), file_hashes(&motion_files)?),
        ("collision_exporters_sha256".into(), collision_sources),
        (
            "collision_sha256".into(),
            run.file_hash("cached-collision.nbt")?,
        ),
        ("render_exporters_sha256".into(), render_sources),
        ("render_files_sha256".into(), file_hashes(&render_files)?),
        ("model_state_exporters_sha256".into(), binding_sources),
        (
            "gameplay_exporters_sha256".into(),
            entry("gameplay_exporters_sha256"),
        ),
        (
            "entity_model_exporters_sha256".into(),
            entry("entity_model_exporters_sha256"),
        ),
        (
            "creative_exporters_sha256".into(),
            entry("creative_exporters_sha256"),
        ),
        (
            "mob_data_exporters_sha256".into(),
            entry("mob_data_exporters_sha256"),
        ),
        (
            "entity_types_exporters_sha256".into(),
            entry("entity_types_exporters_sha256"),
        ),
        (
            "behaviour_exporters_sha256".into(),
            entry("behaviour_exporters_sha256"),
        ),
        (
            "command_registries_exporters_sha256".into(),
            command_sources,
        ),
        (
            "model_states_sha256".into(),
            run.file_hash("model-states.json")?,
        ),
        ("data_version".into(), entry("data_version")),
        ("facts_sha256".into(), run.file_hash("facts.json")?),
        (
            "block_tags_sha256".into(),
            run.file_hash("block-tags.json")?,
        ),
        ("resource_files".into(), entry("resource_files")),
    ]);
    write_json(
        &root.join("horizon-resources.json"),
        &Value::Object(manifest),
    )
}

/// Check existing JAR assets by their ZIP size/CRC without inflating every member again.
/// Indexed panorama/font/language replacements have stronger SHA-1 checks and take precedence.
/// The members are checked and written on several threads, each with the jar open.
fn extract_client_resources(
    jar: &Path,
    root: &Path,
    mut indexed: Option<&mut sounds::Assets>,
) -> Result<(Value, usize), String> {
    let mut client = open_zip(jar)?;
    let version = pyjson::loads(&zip_text(&mut client, "version.json")?)?;
    let mut count = 0;
    // Members whose files could be one on a case-insensitive file system, in the jar's order:
    // one thread writes them all, so the last one wins as it did.
    let mut groups: Vec<Vec<(String, PathBuf)>> = Vec::new();
    let mut group_of = std::collections::HashMap::new();
    for name in zip_names(&client)? {
        if !(name.starts_with("assets/") || name.starts_with("data/") || name == "pack.png")
            || name.ends_with('/')
        {
            continue;
        }
        let relative = member_path(&name, "Invalid resource path")?;
        count += 1;
        if let Some(assets) = &mut indexed {
            if assets.imported(&name)? {
                continue;
            }
        }
        let target = root.join(relative);
        let group = *group_of.entry(name.to_lowercase()).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group].push((name, target));
    }
    pool::map(&groups, |group| {
        // A clone shares the jar's parsed member list and reads at its own offset.
        let mut client = client.clone();
        for (name, target) in group {
            extract_member(&mut client, name, target)?;
        }
        Ok(())
    })?;
    Ok((version, count))
}

/// Writes one member unless its file already has the member's size and CRC.
fn extract_member(client: &mut Jar, name: &str, target: &Path) -> Result<(), String> {
    let member = client.by_name(name).map_err(|e| e.to_string())?;
    let (size, crc) = (member.size(), member.crc32());
    drop(member);
    if target.is_file() && fs::metadata(target).map_err(|e| e.to_string())?.len() == size {
        let mut file = fs::File::open(target).map_err(|e| e.to_string())?;
        let mut hash = crc32fast::Hasher::new();
        let mut buffer = [0; 16 * 1024];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        if hash.finalize() == crc {
            return Ok(());
        }
    }
    mkdir(target.parent().unwrap())?;
    write(target, &zip_bytes(client, name)?)
}

fn datagen_reports_missing(root: &Path) -> bool {
    ["items.json", "registries.json", "commands.json"]
        .iter()
        .any(|name| !root.join(name).is_file())
}

fn install_datagen_reports(root: &Path, reports: &Path) -> Result<(), String> {
    for name in ["registries.json", "commands.json"] {
        copy(&reports.join(name), &root.join(name))?;
    }
    let mut components = Value::Object(Vec::new());
    for file in json_files(&reports.join("minecraft/components/item"))? {
        let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
        let report = read_json(&file)?;
        let component = report
            .get("components")
            .ok_or_else(|| format!("{} has no components", file.display()))?;
        components.insert(&format!("minecraft:{stem}"), component.clone());
    }
    write_text(
        &root.join("items.json"),
        &pyjson::dumps(&components, &pyjson::DEFAULT),
    )?;
    let biome_parameters = root.join("biome-parameters");
    mkdir(&biome_parameters)?;
    for file in json_files(&reports.join("biome_parameters/minecraft"))? {
        copy(&file, &biome_parameters.join(file.file_name().unwrap()))?;
    }
    Ok(())
}

/// `sorted(directory.glob('*.json'))`: glob skips names starting with a dot.
fn json_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .map_err(|e| format!("{}: {e}", directory.display()))?
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.ends_with(".json") && !name.starts_with('.')
        })
        .map(|e| e.path())
        .collect();
    files.sort();
    Ok(files)
}

const WORLDGEN_BUNDLES: [(&str, &str); 19] = [
    ("noises", "worldgen/noise"),
    ("densities", "worldgen/density_function"),
    ("rules", "worldgen/material_rule"),
    ("conditions", "worldgen/material_condition"),
    ("settings", "worldgen/noise_settings"),
    ("carvers", "worldgen/carver"),
    ("biomes", "worldgen/biome"),
    ("world_presets", "worldgen/world_preset"),
    ("features", "worldgen/feature"),
    ("placed_features", "worldgen/placed_feature"),
    ("state_providers", "worldgen/block_state_provider"),
    ("biome_tags", "tags/worldgen/biome"),
    ("feature_tags", "tags/worldgen/feature"),
    ("block_tags", "tags/block"),
    ("fluid_tags", "tags/fluid"),
    ("processor_lists", "worldgen/processor_list"),
    ("template_pools", "worldgen/template_pool"),
    ("structures", "worldgen/structure"),
    ("structure_sets", "worldgen/structure_set"),
];

const BLOCK_BEHAVIOR_FILES: [&str; 6] = [
    "block-properties.nbt",
    "cached-collision.nbt",
    "cached-support.nbt",
    "survival-rules.json",
    "survival-extra-definitions.json",
    "placement-definitions.json",
];

/// The chunk generator's resource bundle in `<resources>/worldgen` (see GeneratorData::load), from
/// the jars: structure templates and the worldgen registries.
fn worldgen_bundles(run: &Run) -> Result<(), String> {
    (run.log)("Writing the world generator's bundle");
    let started = Instant::now();
    let out = run.resources.join("worldgen");
    mkdir(&out)?;
    // Structure templates: gzip NBT files under structure/<namespace>/<path>.nbt.
    let prefix = "data/minecraft/structure/";
    let (server, client) = (open_zip(&run.server)?, open_zip(&run.client)?);
    // zip_names gives each name once.
    let templates: Vec<String> = zip_names(&server)?
        .into_iter()
        .filter(|name| name.starts_with(prefix) && name.ends_with(".nbt"))
        .collect();
    pool::map(&templates, |name| {
        let relative = member_path(&name[prefix.len()..], "Invalid structure path")?;
        let data = zip_bytes(&mut server.clone(), name)?;
        write_if_changed(&out.join("structure/minecraft").join(relative), &data)
    })?;
    pool::map(&WORLDGEN_BUNDLES, |(name, path)| {
        let prefix = format!("data/minecraft/{path}/");
        let resources = json_resources(&mut server.clone(), &prefix)?;
        if resources.as_object().is_none_or(<[_]>::is_empty)
            || resources != json_resources(&mut client.clone(), &prefix)?
        {
            return Err(format!("Reference worldgen resources disagree: {path}"));
        }
        write_text(
            &out.join(format!("{name}.json")),
            &pyjson::dumps(&resources, &pyjson::COMPACT),
        )
    })?;
    (run.log)(&format!(
        "Wrote the world generator's bundle{}",
        took(started)
    ));
    Ok(())
}

/// The generator's block behaviour and structure block files, from the server exporter JVM.
fn worldgen_blocks(run: &Run, worker: usize) -> Result<(), String> {
    let local = &run.local;
    let blocks = run.resources.join("worldgen").join("blocks");
    // Block behavior used by feature placement checks (survival, support, collision).
    // These come from the Java exporters of the simple-block parity run, or a direct export.
    let all_present = |dir: &Path| BLOCK_BEHAVIOR_FILES.iter().all(|n| dir.join(n).exists());
    let inputs = stamp_inputs(run)?;
    let mut behavior = local.join("rust-simple-block-parity");
    if !all_present(&behavior) {
        behavior = local.join("horizon-client-block-behavior");
        let outputs: Vec<PathBuf> = BLOCK_BEHAVIOR_FILES
            .iter()
            .map(|n| behavior.join(n))
            .collect();
        let stamp = behavior.join("stamp.json");
        if !stamp_matches(&stamp, &inputs, &outputs) {
            export_block_behavior(run, worker, &behavior)?;
            write_stamp(&stamp, &inputs, &outputs)?;
        }
    }
    mkdir(&blocks)?;
    for name in BLOCK_BEHAVIOR_FILES {
        copy(&behavior.join(name), &blocks.join(name))?;
    }
    let structures = local.join("horizon-client-structures");
    let output = blocks.join("structure-blocks.json");
    let stamp = structures.join("stamp.json");
    if !stamp_matches(&stamp, &inputs, std::slice::from_ref(&output)) {
        run.export_on(
            worker,
            "structure blocks",
            &run.server_classpath,
            &structures.join("classes"),
            &["VanillaStructureBlockExporter"],
            "1G",
            &[arg(&output)],
            &structures.join("export.log"),
        )?;
        write_stamp(&stamp, &inputs, std::slice::from_ref(&output))?;
    }
    Ok(())
}

/// What an exporter's output kept beside its stamp depends on: every exporter source and pin
/// (the exporter bundle's key), the server jar, and this preparer.
fn stamp_inputs(run: &Run) -> Result<Value, String> {
    Ok(Value::Object(vec![
        ("exporters".into(), Value::Str(exporters::key(&run.root)?)),
        (
            "server_sha256".into(),
            run.origin
                .get("server_sha256")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("preparer".into(), Value::str(SOURCES_SHA256)),
    ]))
}

/// The stamp a run that exported `outputs` from `inputs` left: the outputs are still those files.
/// It lives in `.local` beside the exporter's working files, not in the prepared resources.
fn stamp_matches(stamp: &Path, inputs: &Value, outputs: &[PathBuf]) -> bool {
    let Ok(recorded) = read_json(stamp) else {
        return false;
    };
    recorded.get("inputs") == Some(inputs)
        && stamp_outputs(outputs).is_ok_and(|outputs| recorded.get("outputs") == Some(&outputs))
}

fn stamp_outputs(outputs: &[PathBuf]) -> Result<Value, String> {
    let hashes = pool::map(outputs, |path| digest(path, "sha256"))?;
    Ok(Value::Object(
        outputs
            .iter()
            .zip(hashes)
            .map(|(path, hash)| (arg(path), Value::Str(hash)))
            .collect(),
    ))
}

fn write_stamp(stamp: &Path, inputs: &Value, outputs: &[PathBuf]) -> Result<(), String> {
    write_json(
        stamp,
        &Value::Object(vec![
            ("inputs".into(), inputs.clone()),
            ("outputs".into(), stamp_outputs(outputs)?),
        ]),
    )
}

/// The generator's tables from the block facts and the data generator's reports.
fn worldgen_tables(run: &Run) -> Result<(), String> {
    let root = &run.resources;
    let local = &run.local;
    let out = root.join("worldgen");
    let facts = read_json(&root.join("facts.json"))?;
    let short = || "A block state in facts.json is short".to_string();
    let flags = facts
        .get("states")
        .and_then(Value::as_array)
        .ok_or("facts.json has no states")?
        .iter()
        .map(|s| {
            Ok(Value::Array(vec![
                s.index(1).cloned().ok_or_else(short)?,
                s.index(2).cloned().ok_or_else(short)?,
            ]))
        })
        .collect::<Result<Vec<_>, String>>()?;
    write_text(
        &out.join("block_flags.json"),
        &pyjson::dumps(&Value::Array(flags), &pyjson::COMPACT),
    )?;
    let reports = out.join("biome_parameters");
    mkdir(&reports)?;
    let mut presets = false;
    for source in [
        root.join("biome-parameters"),
        local.join("datagen/out/reports/biome_parameters/minecraft"),
    ] {
        if source.join("overworld.json").exists() {
            for report in json_files(&source)? {
                copy(&report, &reports.join(report.file_name().unwrap()))?;
            }
            presets = true;
            break;
        }
    }
    if !presets {
        // Fall back to the Java-exported quantized tables from the material-stage parity run.
        let tables = local.join("rust-material-stage-parity/presets.json");
        if !tables.exists() {
            return Err(
                "No multi-noise biome parameter tables; run the data generator reports".into(),
            );
        }
        copy(&tables, &out.join("presets.json"))?;
    }
    Ok(())
}

/// The vanilla exporters of block properties, support, collision and survival definitions.
/// Feature placement needs them: without these files the generator skips every placed feature
/// whose blocks are checked for survival or support (trees, flowers, grass, kelp, ...).
fn export_block_behavior(run: &Run, worker: usize, export: &Path) -> Result<(), String> {
    run.export_on(
        worker,
        "block behavior",
        &run.server_classpath,
        &export.join("classes"),
        &["VanillaSimpleBlockOracle"],
        "1G",
        &["export".into(), arg(export)],
        &export.join("export.log"),
    )?;
    let rules = survival::bind_definitions(
        &read_json(&export.join("survival-classes.json"))?,
        &read_json(&export.join("survival-placement-classes.json"))?,
        &read_json(&export.join("survival-support-tags.json"))?,
    )?;
    write_text(
        &export.join("survival-rules.json"),
        &pyjson::dumps(&rules, &pyjson::COMPACT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_in_the_scratch_folder_is_one_folder_however_it_is_named() {
        let dir = std::env::temp_dir().join(format!("horizon-prepare-same-{}", std::process::id()));
        let local = dir.join(".local");
        assert!(same_dir(&local, &dir.join("x").join("..").join(".local")).unwrap());
        assert!(!same_dir(&local, &local.join("horizon-client")).unwrap());
        // The lock it would have taken twice: once, then the output skips it.
        let _held = hold_lock(&local, &|_| {}).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn datagen_reports_are_installed_beside_resources_and_missing_reports_need_repair() {
        let dir =
            std::env::temp_dir().join(format!("horizon-prepare-reports-{}", std::process::id()));
        let root = dir.join("resources");
        let reports = dir.join("reports");
        mkdir(&root).unwrap();
        mkdir(&reports.join("minecraft/components/item")).unwrap();
        mkdir(&reports.join("biome_parameters/minecraft")).unwrap();
        write_text(&root.join("items.json"), "{}").unwrap();
        assert!(datagen_reports_missing(&root));
        write_text(
            &reports.join("registries.json"),
            "{\"minecraft:item\":{\"entries\":{}}}",
        )
        .unwrap();
        write_text(&reports.join("commands.json"), "{\"type\":\"root\"}").unwrap();
        write_text(
            &reports.join("minecraft/components/item/stone.json"),
            "{\"components\":{\"minecraft:max_stack_size\":64}}",
        )
        .unwrap();
        write_text(
            &reports.join("biome_parameters/minecraft/overworld.json"),
            "{}",
        )
        .unwrap();
        install_datagen_reports(&root, &reports).unwrap();
        assert!(!datagen_reports_missing(&root));
        for name in ["registries.json", "commands.json"] {
            assert_eq!(
                read(&root.join(name)).unwrap(),
                read(&reports.join(name)).unwrap()
            );
        }
        assert!(read_json(&root.join("items.json"))
            .unwrap()
            .get("minecraft:stone")
            .is_some());
        assert!(root.join("biome-parameters/overworld.json").is_file());
        for name in ["registries.json", "commands.json", "items.json"] {
            fs::remove_file(root.join(name)).unwrap();
            assert!(datagen_reports_missing(&root), "{name}");
            install_datagen_reports(&root, &reports).unwrap();
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lanes_messages_reach_the_log_and_the_fraction_only_grows() {
        let (send, messages) = std::sync::mpsc::channel();
        for stage in 0..STAGES {
            send.send(Some(format!("line {stage}"))).unwrap();
            send.send(None).unwrap();
        }
        drop(send);
        let lines = std::cell::RefCell::new(Vec::new());
        let fractions = std::cell::RefCell::new(Vec::new());
        relay(
            messages,
            &|line| lines.borrow_mut().push(line.to_string()),
            &|done| fractions.borrow_mut().push(done),
        );
        let lines = lines.into_inner();
        assert_eq!(lines.len() as u32, STAGES);
        assert_eq!(lines[3], "line 3");
        // Each stage reports the ones before it: the last stops short of 1, which only the end
        // of `prepare` reports.
        let fractions = fractions.into_inner();
        let expected: Vec<f32> = (0..STAGES).map(|n| n as f32 / STAGES as f32).collect();
        assert_eq!(fractions, expected);
    }

    /// `--progress json` reaches 1 only at the end: `STAGES` counts the stages `steps` opens.
    #[test]
    fn steps_open_every_stage() {
        // A Windows checkout may have CRLF line ends.
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let body = &source[source.find("\nfn steps(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert_eq!(body.matches("run.stage();").count() as u32, STAGES);
    }

    #[test]
    fn output_versions_end_at_the_output_version() {
        let text = include_str!("../output-versions.txt");
        let versions: Vec<(u32, &str)> = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (n, digest) = l.split_once(' ').unwrap();
                (n.parse().unwrap(), digest.trim())
            })
            .collect();
        let numbers: Vec<u32> = versions.iter().map(|v| v.0).collect();
        assert_eq!(numbers, (1..=OUTPUT_VERSION).collect::<Vec<_>>());
        for (i, (_, digest)) in versions.iter().enumerate() {
            assert_eq!(digest.len(), 64);
            assert!(!versions[..i].iter().any(|(_, d)| d == digest));
        }
        // The launcher finds the number with this line in a version folder's sources.
        let lib = include_str!("lib.rs");
        let line = format!("pub const OUTPUT_VERSION: u32 = {OUTPUT_VERSION};");
        assert!(lib.lines().any(|l| l == line));
    }

    #[test]
    fn the_source_digest_is_the_one_build_rs_computed() {
        assert_eq!(
            sources_sha256(Path::new(env!("CARGO_MANIFEST_DIR"))).as_deref(),
            Some(SOURCES_SHA256)
        );
        assert_eq!(sources_sha256(Path::new("/nonexistent")), None);
    }

    #[test]
    fn lines_split_as_python_splits_them() {
        assert_eq!(split_lines("a\r\nb\rc\n\nd"), ["a", "b", "c", "", "d"]);
        assert_eq!(split_lines("a\n"), ["a"]);
    }

    #[test]
    fn stamps_skip_an_export_only_while_its_inputs_and_outputs_are_unchanged() {
        let dir =
            std::env::temp_dir().join(format!("horizon-prepare-stamp-{}", std::process::id()));
        let outputs = [dir.join("out/a.json"), dir.join("out/b.nbt")];
        mkdir(&dir.join("out")).unwrap();
        for output in &outputs {
            write(output, b"exported").unwrap();
        }
        let stamp = dir.join("work/stamp.json");
        let inputs = Value::Object(vec![("exporters".into(), Value::str("key"))]);
        assert!(!stamp_matches(&stamp, &inputs, &outputs), "never exported");
        write_stamp(&stamp, &inputs, &outputs).unwrap();
        assert!(stamp_matches(&stamp, &inputs, &outputs));
        let changed = Value::Object(vec![("exporters".into(), Value::str("new key"))]);
        assert!(
            !stamp_matches(&stamp, &changed, &outputs),
            "new exporters or jar"
        );
        write(&outputs[1], b"damaged").unwrap();
        assert!(!stamp_matches(&stamp, &inputs, &outputs), "output changed");
        write(&outputs[1], b"exported").unwrap();
        assert!(stamp_matches(&stamp, &inputs, &outputs));
        fs::remove_file(&outputs[0]).unwrap();
        assert!(!stamp_matches(&stamp, &inputs, &outputs), "output missing");
        write(&stamp, b"{not json").unwrap();
        assert!(!stamp_matches(&stamp, &inputs, &outputs));
        // A stamp that records no outputs never vouches for a missing one.
        write_json(
            &stamp,
            &Value::Object(vec![("inputs".into(), inputs.clone())]),
        )
        .unwrap();
        assert!(!stamp_matches(&stamp, &inputs, &outputs), "no outputs");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn client_resources_are_extracted_once_and_repaired_when_damaged() {
        use std::io::Write;
        let dir =
            std::env::temp_dir().join(format!("horizon-prepare-extract-{}", std::process::id()));
        mkdir(&dir).unwrap();
        let jar = dir.join("client.jar");
        let mut archive = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let mut members = vec![(
            "version.json".to_string(),
            b"{\"world_version\": 1}".to_vec(),
        )];
        for i in 0..400 {
            members.push((
                format!("assets/minecraft/textures/{i}.png"),
                vec![i as u8; i],
            ));
        }
        members.push(("data/minecraft/Tag.json".into(), b"upper".to_vec()));
        members.push(("data/minecraft/tag.json".into(), b"lower".to_vec()));
        members.push(("pack.png".into(), b"icon".to_vec()));
        members.push(("META-INF/MANIFEST.MF".into(), b"not a resource".to_vec()));
        for (name, data) in &members {
            archive.start_file(name.as_str(), options).unwrap();
            archive.write_all(data).unwrap();
        }
        archive.finish().unwrap();
        let root = dir.join("resources");
        let (version, count) = extract_client_resources(&jar, &root, None).unwrap();
        assert_eq!(version.get("world_version"), Some(&Value::Int("1".into())));
        assert_eq!(count, 403);
        let resources = &members[1..members.len() - 1];
        // On a case-insensitive file system (macOS, Windows) the two names are one file, and
        // the later member of the pair wins. (`Path`s compare case-sensitively, so ask the disk.)
        let folded = root.join("DATA/minecraft/tag.json").exists();
        let check = || {
            for (name, data) in resources {
                let file = root.join(name);
                if !(folded && name.ends_with("Tag.json")) {
                    assert_eq!(&read(&file).unwrap(), data, "{name}");
                }
            }
        };
        check();
        assert!(!root.join("META-INF").exists());
        let damaged = root.join("assets/minecraft/textures/9.png");
        write(&damaged, &[0; 9]).unwrap();
        fs::remove_file(root.join("assets/minecraft/textures/399.png")).unwrap();
        let kept = root.join("assets/minecraft/textures/5.png");
        let before = fs::metadata(&kept).unwrap().modified().unwrap();
        assert_eq!(
            extract_client_resources(&jar, &root, None).unwrap().1,
            count
        );
        check();
        assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn clones_of_a_jar_read_on_their_own() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("horizon-prepare-jar-{}", std::process::id()));
        mkdir(&dir).unwrap();
        let path = dir.join("a.jar");
        let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let members: Vec<(String, Vec<u8>)> = (0..3u8)
            .map(|i| {
                (
                    format!("{i}.bin"),
                    (0..200_000u32).map(|n| (n % 251) as u8 ^ i).collect(),
                )
            })
            .collect();
        for (name, data) in &members {
            archive
                .start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(data).unwrap();
        }
        archive.finish().unwrap();
        let mut first = open_zip(&path).unwrap();
        let mut second = first.clone();
        // Halfway through one member, the other clone reads a whole one.
        let mut entry = first.by_name("0.bin").unwrap();
        let mut start = vec![0; 1000];
        entry.read_exact(&mut start).unwrap();
        assert_eq!(zip_bytes(&mut second, "2.bin").unwrap(), members[2].1);
        let mut rest = Vec::new();
        entry.read_to_end(&mut rest).unwrap();
        start.extend(rest);
        assert_eq!(start, members[0].1);
        drop(entry);
        assert_eq!(zip_bytes(&mut first, "1.bin").unwrap(), members[1].1);
        assert_eq!(zip_names(&second).unwrap(), ["0.bin", "1.bin", "2.bin"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn member_paths_stay_inside() {
        assert!(member_path("assets/a.png", "bad").is_ok());
        assert!(member_path("../a", "bad").is_err());
        assert!(member_path("/a", "bad").is_err());
    }
}
