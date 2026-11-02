//! Reusable, immutable mixin output. API packages are staged after this snapshot.
use super::{Bundle, Profile, FEATURES, MANIFEST};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Whether a manifest value names a path that overlaps the API package.
fn overlaps_api(value: &toml::Value, root: &Path, api: &Path) -> bool {
    match value {
        toml::Value::String(s) => root
            .join(s)
            .canonicalize()
            .is_ok_and(|p| p.starts_with(api) || api.starts_with(&p)),
        toml::Value::Array(a) => a.iter().any(|v| overlaps_api(v, root, api)),
        toml::Value::Table(t) => t.values().any(|v| overlaps_api(v, root, api)),
        _ => false,
    }
}

/// Hash all mixin inputs but not an API crate that mixins do not read.
pub(super) fn key(
    revision: &str,
    mixins: &str,
    profile: Profile,
    mods: &[Bundle],
) -> Result<String, String> {
    let mut hash = Sha256::new();
    for part in [
        // Bump when launcher generation or validation changes the snapshot's contents.
        "trait-injection-v2",
        revision,
        mixins,
        profile.name(),
        FEATURES,
        super::rust_toolchain::VERSION,
    ] {
        field(&mut hash, part.as_bytes());
    }
    for bundle in mods {
        field(&mut hash, b"bundle");
        field(&mut hash, bundle.id.as_bytes());
        let api = bundle
            .api
            .as_ref()
            .map(|p| bundle.dir.join(p).canonicalize())
            .transpose()
            .map_err(|e| e.to_string())?;
        let manifest_path = bundle.dir.join(MANIFEST);
        let mut manifest: toml::Table = fs::read_to_string(&manifest_path)
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        // Mixins also reads data/ implicitly, without a manifest entry.
        manifest.remove("api");
        let data = toml::Value::String("data".into());
        let manifest = toml::Value::Table(manifest);
        let exclude = api.as_ref().filter(|a| {
            !overlaps_api(&manifest, &bundle.dir, a) && !overlaps_api(&data, &bundle.dir, a)
        });
        for (name, path) in super::bundle_files(bundle)? {
            if exclude.is_some_and(|a| path.starts_with(a)) {
                continue;
            }
            field(&mut hash, b"file");
            field(&mut hash, name.as_bytes());
            field(
                &mut hash,
                &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn snapshot(root: &Path, key: &str) -> PathBuf {
    root.join("patched").join(key)
}

/// Record a hit and retain the three most recently used snapshots.
pub(super) fn touch_and_prune(root: &Path, key: &str) -> Result<(), String> {
    let dir = root.join("patched");
    fs::write(dir.join(key).join(".last-used"), b"").map_err(|e| e.to_string())?;
    let mut entries = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| {
        std::cmp::Reverse((
            e.file_name() == key,
            fs::metadata(e.path().join(".last-used"))
                .and_then(|m| m.modified())
                .ok(),
        ))
    });
    for entry in entries.into_iter().skip(3) {
        fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
