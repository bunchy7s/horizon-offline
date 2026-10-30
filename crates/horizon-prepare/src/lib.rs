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
pub mod exporters;
mod files;
mod inventory;
pub mod java;
pub mod pyjson;
mod sounds;
mod survival;
mod worker;

pub use files::clone_or_copy;
pub use sounds::{fonts, languages, packs, panorama, sounds, BUILT_IN_PACKS};

use pyjson::Value;
use sha2::Digest;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Where progress lines go: the terminal for the binary, the log pane for the launcher.
pub type Log<'a> = &'a dyn Fn(&str);

/// The manifest's `schema`: the prepared files' layout, which the launcher compares.
pub const SCHEMA: i64 = 7;

/// What this preparer makes. The launcher copies a prepared version's resources to a newly installed
/// version whose preparer has the same number (and the same pinned jars), so a release that changes
/// this crate without changing its output asks nobody to prepare again. Bump it, and append the new
/// digest to output-versions.txt, when the prepared files change: CI prepares with every change to
/// the preparer and fails when the output is not the one output-versions.txt gives this number
/// (`python3 tools/prepare_output.py check`). The launcher reads this line from the version
/// folder's sources: keep it on one line, as it is.
pub const OUTPUT_VERSION: u32 = 2;

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

pub(crate) fn open_zip(path: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn zip_bytes(
    archive: &mut zip::ZipArchive<fs::File>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut data)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(data)
}

pub(crate) fn zip_text(
    archive: &mut zip::ZipArchive<fs::File>,
    name: &str,
) -> Result<String, String> {
    String::from_utf8(zip_bytes(archive, name)?).map_err(|e| format!("{name}: {e}"))
}

/// The archive's member names in its own order (Python's `namelist`).
fn zip_names(archive: &zip::ZipArchive<fs::File>) -> Vec<String> {
    (0..archive.len())
        .filter_map(|i| archive.name_for_index(i).map(str::to_string))
        .collect()
}

/// `{"minecraft:<path>": text}` for every `<prefix><path>.json`, sorted by member name
/// (the comprehension the Python preparer used for tags, worldgen and damage resources).
fn json_resources(archive: &mut zip::ZipArchive<fs::File>, prefix: &str) -> Result<Value, String> {
    let mut names: Vec<String> = zip_names(archive)
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
    bundles: std::cell::OnceCell<PathBuf>,
    workers: std::cell::RefCell<[Option<worker::Worker>; 2]>,
    previous: Value,
    origin: Value,
    log: Log<'a>,
}

const PARITY: &str = "mods/horizon/src/test/java/dev/horizonlab/parity";

impl Run<'_> {
    fn bundle(&self) -> Result<&Path, String> {
        if self.bundles.get().is_none() {
            self.bundles.set(exporters::classes(self)?).unwrap();
        }
        Ok(self.bundles.get().unwrap())
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

    /// `java -Xmx<heap> -cp <classes>:<classpath> dev.horizonlab.parity.<main> <args>`.
    fn run(
        &self,
        heap: &str,
        classpath: &str,
        main: &str,
        args: &[String],
        log: &fs::File,
        log_path: &Path,
    ) -> Result<(), String> {
        let side = usize::from(classpath == self.client_classpath);
        let classes = self
            .bundle()?
            .join(if side == 1 { "client" } else { "server" });
        if !matches!(
            main,
            "VanillaCommandParseOracle" | "VanillaCreativeTabsExporter"
        ) && std::env::var_os("HORIZON_PREPARE_ISOLATED").is_none()
        {
            let mut workers = self.workers.borrow_mut();
            if workers[side].is_none() {
                workers[side] = Some(worker::Worker::new(self, &classes, classpath)?);
            }
            return workers[side].as_mut().unwrap().run(main, args, log_path);
        }
        let separator = if cfg!(windows) { ";" } else { ":" };
        let mut all = vec![
            format!("-Xmx{heap}"),
            "-cp".into(),
            format!("{}{separator}{classpath}", classes.display()),
            format!("dev.horizonlab.parity.{main}"),
        ];
        all.extend_from_slice(args);
        self.jdk.java(&all, None, log)
    }

    /// Runs each exporter from the verified shared bundle, with output in
    /// the log at `log`: the shape of every exporter step.
    #[allow(clippy::too_many_arguments)]
    fn export(
        &self,
        what: &str,
        classpath: &str,
        classes: &Path,
        _sources: &[&Path],
        mains: &[&str],
        heap: &str,
        args: &[String],
        log: &Path,
    ) -> Result<(), String> {
        (self.log)(&format!("Exporting {what} ({})", mains.join(", ")));
        mkdir(classes)?;
        let file = self.log_file(log)?;
        let fail = |e: String| format!("{e} while exporting {what}; see {}", log.display());
        for main in mains {
            self.run(heap, classpath, main, args, &file, log)
                .map_err(fail)?;
        }
        Ok(())
    }
}

/// Prepares the resources; returns their folder (`.local/horizon-client/<version>`).
pub fn prepare(options: &Options, log: Log) -> Result<PathBuf, String> {
    prepare_impl(options, log, false)
}

/// Release build tool: compile and link-check only Horizon's original exporter classes.
pub fn compile_exporters(options: &Options, log: Log) -> Result<PathBuf, String> {
    prepare_impl(options, log, true)
}

fn prepare_impl(options: &Options, log: Log, compile_only: bool) -> Result<PathBuf, String> {
    let root = &std::path::absolute(&options.root)
        .map_err(|e| format!("{}: {e}", options.root.display()))?;
    let lock = artifacts::Lock::read(root)?;
    // Java first: nothing below can finish without it, and the player should hear it at once.
    let mut homes: Vec<PathBuf> = options.java_home.iter().cloned().collect();
    homes.extend(java::minecraft_runtimes(
        &artifacts::game_files(options.minecraft_dir.as_deref(), &lock.version).runtimes,
    ));
    let jdk = java::find_runtime(lock.java_major, &homes)?;
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
    let resources = local.join("horizon-client").join(&lock.version);
    mkdir(&resources)?;
    let manifest_path = resources.join("horizon-resources.json");
    let oracle = root.join(PARITY).join("VanillaHeightmapOracle.java");
    let origin = Value::Object(vec![
        ("minecraft".into(), Value::Str(lock.version.clone())),
        (
            "client_sha256".into(),
            Value::Str(digest(&found.jars["client"], "sha256")?),
        ),
        (
            "server_sha256".into(),
            Value::Str(digest(&found.jars["server"], "sha256")?),
        ),
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
        bundles: std::cell::OnceCell::new(),
        workers: std::cell::RefCell::new([None, None]),
        previous,
        origin,
        log,
    };
    if compile_only {
        return Ok(run.bundle()?.to_path_buf());
    }
    steps(&run, &lock, &oracle, options.minecraft_dir.as_deref())?;
    log(&format!(
        "Prepared Horizon native-client resources: {}",
        resources.display()
    ));
    Ok(resources)
}

fn arg(path: &Path) -> String {
    path.display().to_string()
}

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

    // Every block state and its facts.
    let facts = root.join("facts.json");
    let origin_same = run
        .origin
        .as_object()
        .unwrap()
        .iter()
        .all(|(k, v)| run.previous(k) == Some(v));
    if !(origin_same
        && facts.exists()
        && run.previous("facts_sha256") == Some(&run.file_hash("facts.json")?))
    {
        run.export(
            "block states",
            server_cp,
            &root.join("classes"),
            &[oracle],
            &["VanillaHeightmapOracle"],
            "512M",
            &["export".into(), arg(&facts)],
            &root.join("export.log"),
        )?;
    }

    // Block state to model bindings, from the client.
    let binding = run.oracle("VanillaBlockStateBindingOracle.java");
    let helper = run.oracle("VanillaCuboidModelOracle.java");
    let binding_sources = source_hashes(repo, &[&binding, &helper])?;
    let model_states = root.join("model-states.json");
    if !(run.previous("model_state_exporters_sha256") == Some(&binding_sources)
        && run.same("client_sha256")
        && model_states.exists()
        && run.previous("model_states_sha256") == Some(&run.file_hash("model-states.json")?))
    {
        run.export(
            "model bindings",
            client_cp,
            &root.join("client-classes"),
            &[&binding, &helper],
            &["VanillaBlockStateBindingOracle"],
            "512M",
            &["export".into(), arg(&model_states)],
            &root.join("model-state-export.log"),
        )?;
    }

    let block_tags = "data/minecraft/tags/block/";
    let tags = json_resources(&mut open_zip(&run.server)?, block_tags)?;
    if tags != json_resources(&mut open_zip(&run.client)?, block_tags)? {
        return Err("Reference block tags disagree".into());
    }
    write_json(&root.join("block-tags.json"), &tags)?;

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
    if !(run.previous("motion_exporters_sha256") == Some(&motion_sources)
        && run.same("server_sha256")
        && run.previous("block_tags_sha256") == Some(&run.file_hash("block-tags.json")?)
        && run.files_unchanged("motion_files_sha256", &motion_files)?)
    {
        let export = run.local.join("horizon-client-motion");
        mkdir(&export)?;
        write_json(&export.join("tags.json"), &tags)?;
        run.export(
            "movement and attributes",
            server_cp,
            &export.join("classes"),
            &[&movement, &attributes, oracle, &landing, &collision],
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

    let damage = run.oracle("VanillaDamageSourceOracle.java");
    let health = run.oracle("VanillaPlayerHealthOracle.java");
    let health_sources = source_hashes(repo, &[&damage, &health, &attributes])?;
    let health_files = [
        "damage-types.json",
        "damage-tags.json",
        "damage-catalog.json",
        "player-float-ids.json",
    ];
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
            &[&damage, &health, &attributes],
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
            &[&collision],
            &["VanillaCollisionOracle"],
            "768M",
            &["generate".into(), arg(&export)],
            &export.join("export.log"),
        )?;
        copy(&export.join("cached-collision.nbt"), &collision_file)?;
    }

    let render_sources = source_hashes(repo, &[&visibility, &collision, oracle])?;
    let render_files = ["render-definitions.json", "occlusion.nbt"];
    if !(run.previous("render_exporters_sha256") == Some(&render_sources)
        && run.same("server_sha256")
        && run.previous("block_tags_sha256") == Some(&run.file_hash("block-tags.json")?)
        && run.files_unchanged("render_files_sha256", &render_files)?)
    {
        let export = run.local.join("horizon-client-render");
        mkdir(&export)?;
        write_json(&export.join("tags.json"), &tags)?;
        run.export(
            "face visibility",
            server_cp,
            &export.join("classes"),
            &[&visibility, &collision, oracle],
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

    // The playable client needs every client asset (textures, models, fonts, GUI)
    // and every data-pack resource (worldgen, recipes, loot tables, tags), and
    // the vanilla pack's icon (`pack.png`) for the pack screen.
    (run.log)("Copying the client's assets and data");
    let mut indexed = sounds::Assets::load(repo, minecraft, run.log);
    let (version, count) = extract_client_resources(&run.client, root, indexed.as_mut().ok())?;
    sounds::optional_imports(indexed.as_mut().map_err(|e| e.as_str()), run.log);

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
                &sources,
                &[main],
                heap,
                &args,
                &root.join(log),
            )?;
        }
        Ok(hashes)
    };
    let gameplay_sources = single(
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
    let entity_types_sources = single(
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
    let behaviour_sources = single(
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

    if datagen_reports_missing(root) || !run.same("server_sha256") {
        // Vanilla's own data generator reports every item's default components.
        (run.log)("Running vanilla's data generator reports");
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

    // Biome parameter tables come from the data generator reports above.
    prepare_worldgen(run)?;

    let entity_sources = single(
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
    let mob_sources = single(
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

    // Vanilla's built-in registries for in-game command parsing (mob effects,
    // data component types, dialogs, block entity and tag members).
    let command_registries = root.join("command-registries.json");
    let command_oracle = run.oracle("VanillaCommandParseOracle.java");
    let command_sources = source_hashes(repo, &[&command_oracle])?;
    if !(run.previous("command_registries_exporters_sha256") == Some(&command_sources)
        && command_registries.exists()
        && run.same("server_sha256"))
    {
        let export = run.local.join("command-registries");
        run.export(
            "command registries",
            server_cp,
            &export.join("classes"),
            &[&command_oracle],
            &["VanillaCommandParseOracle"],
            "1G",
            &["registries".into(), arg(&export)],
            &root.join("command-registries-export.log"),
        )?;
        copy(&export.join("registries.json"), &command_registries)?;
    }

    // The creative inventory (creative-inventory.json): every tab built by vanilla's
    // CreativeModeTabs over the vanilla data pack's registries, with the search
    // tab's tooltip lines and tags.
    let creative_sources = single(
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

    let file_hashes = |names: &[&str]| -> Result<Value, String> {
        Ok(Value::Object(
            names
                .iter()
                .map(|n| Ok((n.to_string(), run.file_hash(n)?)))
                .collect::<Result<_, String>>()?,
        ))
    };
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
        ("gameplay_exporters_sha256".into(), gameplay_sources),
        ("entity_model_exporters_sha256".into(), entity_sources),
        ("creative_exporters_sha256".into(), creative_sources),
        ("mob_data_exporters_sha256".into(), mob_sources),
        ("entity_types_exporters_sha256".into(), entity_types_sources),
        ("behaviour_exporters_sha256".into(), behaviour_sources),
        (
            "command_registries_exporters_sha256".into(),
            command_sources,
        ),
        (
            "model_states_sha256".into(),
            run.file_hash("model-states.json")?,
        ),
        (
            "data_version".into(),
            version
                .get("world_version")
                .cloned()
                .ok_or("version.json has no world_version")?,
        ),
        ("facts_sha256".into(), run.file_hash("facts.json")?),
        (
            "block_tags_sha256".into(),
            run.file_hash("block-tags.json")?,
        ),
        ("resource_files".into(), Value::Int(count.to_string())),
    ]);
    write_json(
        &root.join("horizon-resources.json"),
        &Value::Object(manifest),
    )
}

/// Check existing JAR assets by their ZIP size/CRC without inflating every member again.
/// Indexed panorama/font/language replacements have stronger SHA-1 checks and take precedence.
fn extract_client_resources(
    jar: &Path,
    root: &Path,
    mut indexed: Option<&mut sounds::Assets>,
) -> Result<(Value, usize), String> {
    let mut client = open_zip(jar)?;
    let version = pyjson::loads(&zip_text(&mut client, "version.json")?)?;
    let mut count = 0;
    for name in zip_names(&client) {
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
        let member = client.by_name(&name).map_err(|e| e.to_string())?;
        let (size, crc) = (member.size(), member.crc32());
        drop(member);
        if target.is_file() && fs::metadata(&target).map_err(|e| e.to_string())?.len() == size {
            let mut file = fs::File::open(&target).map_err(|e| e.to_string())?;
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
                continue;
            }
        }
        mkdir(target.parent().unwrap())?;
        write(&target, &zip_bytes(&mut client, &name)?)?;
    }
    Ok((version, count))
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

/// The chunk generator's resource bundle in `<resources>/worldgen` (see GeneratorData::load).
fn prepare_worldgen(run: &Run) -> Result<(), String> {
    (run.log)("Writing the world generator's bundle");
    let root = &run.resources;
    let local = &run.local;
    let out = root.join("worldgen");
    mkdir(&out)?;
    // Structure templates: gzip NBT files under structure/<namespace>/<path>.nbt.
    let mut server = open_zip(&run.server)?;
    let prefix = "data/minecraft/structure/";
    for name in zip_names(&server) {
        if name.starts_with(prefix) && name.ends_with(".nbt") {
            let relative = member_path(&name[prefix.len()..], "Invalid structure path")?;
            let data = zip_bytes(&mut server, &name)?;
            write_if_changed(&out.join("structure/minecraft").join(relative), &data)?;
        }
    }
    let mut client = open_zip(&run.client)?;
    for (name, path) in WORLDGEN_BUNDLES {
        let prefix = format!("data/minecraft/{path}/");
        let resources = json_resources(&mut server, &prefix)?;
        if resources.as_object().is_none_or(<[_]>::is_empty)
            || resources != json_resources(&mut client, &prefix)?
        {
            return Err(format!("Reference worldgen resources disagree: {path}"));
        }
        write_text(
            &out.join(format!("{name}.json")),
            &pyjson::dumps(&resources, &pyjson::COMPACT),
        )?;
    }
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
    // Block behavior used by feature placement checks (survival, support, collision).
    // These come from the Java exporters of the simple-block parity run, or a direct export.
    let all_present = |dir: &Path| BLOCK_BEHAVIOR_FILES.iter().all(|n| dir.join(n).exists());
    let mut behavior = local.join("rust-simple-block-parity");
    if !all_present(&behavior) {
        behavior = local.join("horizon-client-block-behavior");
        export_block_behavior(run, &behavior)?;
    }
    let blocks = out.join("blocks");
    if all_present(&behavior) {
        mkdir(&blocks)?;
        for name in BLOCK_BEHAVIOR_FILES {
            copy(&behavior.join(name), &blocks.join(name))?;
        }
    }
    mkdir(&blocks)?;
    let structures = local.join("horizon-client-structures");
    run.export(
        "structure blocks",
        &run.server_classpath,
        &structures.join("classes"),
        &[&run.oracle("VanillaStructureBlockExporter.java")],
        &["VanillaStructureBlockExporter"],
        "1G",
        &[arg(&blocks.join("structure-blocks.json"))],
        &structures.join("export.log"),
    )?;
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
fn export_block_behavior(run: &Run, export: &Path) -> Result<(), String> {
    (run.log)("Exporting block behavior (VanillaSimpleBlockOracle)");
    let classes = export.join("classes");
    mkdir(&classes)?;
    let log_path = export.join("export.log");
    let log = run.log_file(&log_path)?;
    let fail = |e: String| {
        format!(
            "{e} while exporting block behavior; see {}",
            log_path.display()
        )
    };
    run.run(
        "1G",
        &run.server_classpath,
        "VanillaSimpleBlockOracle",
        &["export".into(), arg(export)],
        &log,
        &log_path,
    )
    .map_err(fail)?;
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
    fn member_paths_stay_inside() {
        assert!(member_path("assets/a.png", "bad").is_ok());
        assert!(member_path("../a", "bad").is_err());
        assert!(member_path("/a", "bad").is_err());
    }
}
