//! Only original Horizon classes are packaged. The pinned jars remain player supplied.
use crate::*;

const SETTINGS: &str = "exporters-v1 --release -proc:none -encoding UTF-8";
const SERVER: &[&str] = &[
    "VanillaHeightmapOracle",
    "VanillaBlockLandingOracle",
    "VanillaEntityAttributeOracle",
    "VanillaPlayerHealthOracle",
    "VanillaDamageSourceOracle",
    "VanillaCollisionOracle",
    "VanillaFaceVisibilityOracle",
    "VanillaGameplayFactsExporter",
    "VanillaEntityTypesExporter",
    "VanillaBlockBehaviourExporter",
    "VanillaCommandParseOracle",
    "VanillaCreativeTabsExporter",
    "VanillaSimpleBlockOracle",
    "VanillaStructureBlockExporter",
    "PrepareWorker",
];
const CLIENT: &[&str] = &[
    "VanillaBlockStateBindingOracle",
    "VanillaEntityModelExporter",
    "VanillaMobDataExporter",
    "PrepareWorker",
];

fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            result.extend(files(&path)?);
        } else {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}

// research-lock pins both jars and every accepted client manifest (which pins its libraries).
// artifacts::verify/client_libraries check those dependencies before any bundle is used.
pub(crate) fn key(root: &Path) -> Result<String, String> {
    let mut hash = sha2::Sha256::new();
    hash.update(SETTINGS);
    // The launcher's bundled preparer may differ from the selected game's. Hash that
    // version's compiler settings so a newer release still selects a runtime correctly.
    let compiler = fs::read_to_string(root.join("crates/horizon-prepare/src/exporters.rs"))
        .unwrap_or_else(|_| include_str!("exporters.rs").into());
    hash.update(compiler.replace('\r', ""));
    let mut sources = files(&root.join(PARITY))?;
    sources.retain(|p| p.extension().is_some_and(|e| e == "java"));
    sources.push(root.join("research-lock.json"));
    for path in sources {
        let data: Vec<_> = read(&path)?.into_iter().filter(|b| *b != b'\r').collect();
        hash.update(format!(
            "{}\0{}\0",
            relative_display(&path, root)?.replace('\\', "/"),
            data.len()
        ));
        hash.update(data);
    }
    Ok(hex(&hash.finalize()))
}

fn outputs(dir: &Path) -> Result<Value, String> {
    let mut hashes = Value::Object(Vec::new());
    for path in files(dir)? {
        if path.extension().is_some_and(|e| e == "class") {
            let name = relative_display(&path, dir)?.replace('\\', "/");
            // javac's sourcepath contains our sources only; enforce that at packaging too.
            if !name.starts_with("server/dev/horizonlab/parity/")
                && !name.starts_with("client/dev/horizonlab/parity/")
            {
                return Err(format!("Non-Horizon class in exporter bundle: {name}"));
            }
            hashes.insert(&name, Value::Str(digest(&path, "sha256")?));
        }
    }
    Ok(hashes)
}

fn valid(dir: &Path, key: &str) -> bool {
    let Ok(manifest) = read_json(&dir.join("manifest.json")) else {
        return false;
    };
    manifest.get("key").and_then(Value::as_str) == Some(key)
        && outputs(dir).is_ok_and(|hashes| {
            !hashes.as_object().unwrap().is_empty() && manifest.get("classes") == Some(&hashes)
        })
}

/// Whether the version ships a matching, intact bundle and needs only a Java runtime.
pub fn available(root: &Path) -> bool {
    key(root).is_ok_and(|key| valid(&root.join("exporters"), &key))
}

pub(crate) fn classes(run: &Run) -> Result<PathBuf, String> {
    let key = key(&run.root)?;
    let shipped = run.root.join("exporters");
    if valid(&shipped, &key) {
        return Ok(shipped);
    }
    let cached = run.local.join("exporters").join(&key);
    if valid(&cached, &key) {
        return Ok(cached);
    }
    // Build privately: an interrupted compiler or another preparer cannot publish partial classes.
    let destination = cached;
    let cached = run
        .local
        .join("exporters")
        .join(format!(".{key}-{}", std::process::id()));
    if cached.exists() {
        fs::remove_dir_all(&cached).map_err(|e| e.to_string())?;
    }
    mkdir(&cached)?;
    let jdk = java::find(run.java_major, &[run.jdk.home()])?;
    // The two sides compile at once: neither reads the other's classes.
    let sides = [
        ("server", SERVER, &run.server_classpath),
        ("client", CLIENT, &run.client_classpath),
    ];
    crate::pool::map(&sides, |(side, mains, cp)| {
        (run.log)(&format!("Compiling {side} exporters (shared helpers once)"));
        let started = std::time::Instant::now();
        let classes = cached.join(side);
        mkdir(&classes)?;
        let log = run.log_file(&cached.join(format!("{side}.log")))?;
        let mut args = vec![
            "-encoding".into(),
            "UTF-8".into(),
            "-cp".into(),
            cp.to_string(),
            "-sourcepath".into(),
            arg(&run.root.join("mods/horizon/src/test/java")),
            "-d".into(),
            arg(&classes),
        ];
        args.extend(mains.iter().map(|n| arg(&run.oracle(&format!("{n}.java")))));
        jdk.javac(run.java_major, &args, &log)?;
        let validate = vec![
            "-Xverify:all".into(),
            "-cp".into(),
            format!(
                "{}{}{}",
                classes.display(),
                if cfg!(windows) { ";" } else { ":" },
                cp
            ),
            "dev.horizonlab.parity.PrepareWorker".into(),
            "validate".into(),
            arg(&classes),
        ];
        // No exporter executes here: resolve every class and its signatures against pinned jars.
        run.jdk.java(&validate, Some(&run.local), &log)?;
        (run.log)(&format!(
            "Compiled {side} exporters{}",
            crate::took(started)
        ));
        Ok(())
    })?;
    let manifest = Value::Object(vec![
        ("key".into(), Value::Str(key.clone())),
        ("classes".into(), outputs(&cached)?),
    ]);
    write_json(&cached.join("manifest.json"), &manifest)?;
    if valid(&destination, &key) {
        fs::remove_dir_all(&cached).map_err(|e| e.to_string())?;
    } else {
        if destination.exists() {
            fs::remove_dir_all(&destination).map_err(|e| e.to_string())?;
        }
        fs::rename(&cached, &destination).map_err(|e| e.to_string())?;
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_invalidate_sources_pins_and_damaged_classes() {
        let root = std::env::temp_dir().join(format!("prepare-bundle-{}", std::process::id()));
        let source = root.join(PARITY).join("VanillaHeightmapOracle.java");
        mkdir(source.parent().unwrap()).unwrap();
        write(&source, b"original\n").unwrap();
        let compiler = root.join("crates/horizon-prepare/src/exporters.rs");
        mkdir(compiler.parent().unwrap()).unwrap();
        write(&compiler, b"--release 25 -proc:none").unwrap();
        write(&root.join("research-lock.json"), b"pins").unwrap();
        let original = key(&root).unwrap();
        write(&compiler, b"--release 26 -proc:none").unwrap();
        assert_ne!(key(&root).unwrap(), original);
        write(&compiler, b"--release 25 -proc:none").unwrap();
        write(&source, b"original\r\n").unwrap();
        assert_eq!(key(&root).unwrap(), original);
        write(&source, b"changed").unwrap();
        assert_ne!(key(&root).unwrap(), original);
        write(&source, b"original\n").unwrap();
        write(&root.join("research-lock.json"), b"new dependencies").unwrap();
        assert_ne!(key(&root).unwrap(), original);
        write(&root.join("research-lock.json"), b"pins").unwrap();
        let bundle = root.join("exporters");
        let class = bundle.join("server/dev/horizonlab/parity/Helper.class");
        mkdir(class.parent().unwrap()).unwrap();
        write(&class, b"compiled").unwrap();
        write_json(
            &bundle.join("manifest.json"),
            &Value::Object(vec![
                ("key".into(), Value::Str(original)),
                ("classes".into(), outputs(&bundle).unwrap()),
            ]),
        )
        .unwrap();
        assert!(available(&root));
        write(&class, b"damaged").unwrap();
        assert!(!available(&root));
        fs::remove_file(&class).unwrap();
        assert!(!available(&root));
        fs::remove_dir_all(root).unwrap();
    }
}
