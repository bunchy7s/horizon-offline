//! A shader pack on disk: a folder holding `horizon-shaders.toml`, or the
//! same files zipped as a `.hshader` (what the registry and the launcher
//! carry). Both read into one [`Pack`].
use crate::manifest::{MANIFEST, Manifest};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

/// A packaged pack's extension.
pub const EXTENSION: &str = "hshader";
/// Bytes per WGSL file, per other file, and in all.
pub const MAX_WGSL: u64 = 256 << 10;
const MAX_OTHER: u64 = 1 << 20;
pub const MAX_TOTAL: u64 = 8 << 20;
/// Files in a `.hshader`.
const MAX_ENTRIES: usize = 512;
/// Files a pack may hold beside its manifest and WGSL.
const EXTRAS: [&str; 5] = [
    "README.md",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "icon.png",
];

/// A pack read from a folder or a `.hshader`: its manifest and every file,
/// by path from the pack's root (`/`-separated).
#[derive(Debug)]
pub struct Pack {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// A relative path made of plain parts: no root, `..`, `\`, empty or
/// hidden parts.
pub fn plain_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

/// Whether `path` is a pack of this format: a folder holding
/// `horizon-shaders.toml`, or a `.hshader` file.
pub fn is_pack(path: &Path) -> bool {
    path.join(MANIFEST).is_file()
        || (path.is_file() && path.extension().is_some_and(|e| e == EXTENSION))
}

impl Pack {
    /// Reads `path`: a `.hshader` file, or a folder (its hidden files and
    /// folders, such as `.git`, are skipped).
    pub fn read(path: &Path) -> Result<Pack, String> {
        let mut files = BTreeMap::new();
        if path.is_dir() {
            read_dir(path, "", &mut files)?;
        } else {
            let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut zip = zip::ZipArchive::new(file)
                .map_err(|e| format!("{}: not a .{EXTENSION} (zip): {e}", path.display()))?;
            if zip.len() > MAX_ENTRIES {
                return Err(format!("over {MAX_ENTRIES} files in the package"));
            }
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().map_err(|e| e.to_string())?.into_owned();
                let mut bytes = Vec::new();
                (&mut entry)
                    .take(MAX_OTHER.max(MAX_WGSL) + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("{name}: {e}"))?;
                if files.insert(name.clone(), bytes).is_some() {
                    return Err(format!("{name} is in the package twice"));
                }
            }
        }
        Pack::from_files(files)
    }

    /// Checks a pack's files: a manifest, the WGSL it names (as UTF-8),
    /// nothing it doesn't use, and sizes within bounds.
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Result<Pack, String> {
        let text = files
            .get(MANIFEST)
            .ok_or_else(|| format!("{MANIFEST}: missing at the pack's root"))?;
        let manifest = Manifest::parse(
            std::str::from_utf8(text).map_err(|_| format!("{MANIFEST}: not UTF-8"))?,
        )?;
        let used = manifest.files();
        let mut total = 0;
        for (name, bytes) in &files {
            if !plain_path(name) {
                return Err(format!("{name:?} is not a plain path"));
            }
            let size = bytes.len() as u64;
            total += size;
            let max = if name.ends_with(".wgsl") {
                if !used.contains(&name.as_str()) {
                    return Err(format!("{name} is not used by any stage or in shared"));
                }
                if std::str::from_utf8(bytes).is_err() {
                    return Err(format!("{name}: not UTF-8"));
                }
                MAX_WGSL
            } else if name == MANIFEST
                || EXTRAS.contains(&name.as_str())
                || manifest.textures.iter().any(|t| &t.file == name)
            {
                MAX_OTHER
            } else {
                return Err(format!(
                    "{name}: a pack holds only {MANIFEST}, .wgsl files, its [[textures]] and {}",
                    EXTRAS.join(", ")
                ));
            };
            if size > max {
                return Err(format!("{name} is over {} KiB", max >> 10));
            }
        }
        if total > MAX_TOTAL {
            return Err(format!("the pack is over {} MiB", MAX_TOTAL >> 20));
        }
        if let Some(t) = manifest
            .textures
            .iter()
            .find(|t| !files.contains_key(&t.file))
        {
            return Err(format!("{}: named in {MANIFEST} but missing", t.file));
        }
        if let Some(missing) = used.iter().find(|f| !files.contains_key(**f)) {
            return Err(format!("{missing}: named in {MANIFEST} but missing"));
        }
        Ok(Pack { manifest, files })
    }

    /// The bytes of the pack's file `file` (a `[[textures]]` image).
    pub fn source_bytes(&self, file: &str) -> Option<&[u8]> {
        self.files.get(file).map(Vec::as_slice)
    }

    /// A WGSL file's text (every file the manifest names is checked UTF-8).
    pub fn source(&self, file: &str) -> Option<&str> {
        self.files
            .get(file)
            .and_then(|b| std::str::from_utf8(b).ok())
    }

    /// The `.hshader` bytes: the files in path order, with fixed times and
    /// permissions, so the same pack always gives the same bytes (and the
    /// same SHA-256).
    pub fn package(&self) -> Result<Vec<u8>, String> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        let err = |e: zip::result::ZipError| e.to_string();
        for (name, bytes) in &self.files {
            zip.start_file(name.as_str(), options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
        Ok(zip.finish().map_err(err)?.into_inner())
    }

    /// The SHA-256 of [`Pack::package`], in lowercase hex: the pack's
    /// identity in the registry.
    pub fn sha256(&self) -> Result<String, String> {
        Ok(sha256_hex(&self.package()?))
    }
}

/// SHA-256 in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read_dir(dir: &Path, prefix: &str, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let rel = format!("{prefix}{name}");
        if path.is_dir() {
            read_dir(&path, &format!("{rel}/"), files)?;
        } else {
            let size = entry.metadata().map_err(|e| format!("{rel}: {e}"))?.len();
            if size > MAX_TOTAL {
                return Err(format!("{rel} is over {} MiB", MAX_TOTAL >> 20));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
            files.insert(rel, bytes);
        }
    }
    Ok(())
}
