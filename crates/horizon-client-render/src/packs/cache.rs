//! Overlay reuse validated by source topology and filesystem stamps.
//! Published output is an owned cache artifact; hits do not traverse its tree.
use super::{Pack, Source, build_overlay_uncached};
use crate::zip::Zip;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::{collections::BTreeSet, fs, io, path::Path, path::PathBuf};

// Bump when overlay/merge semantics change. Markers are disposable cache data.
// Version 4 mirrored every untouched directory file by file on Windows.
// Version 3 could omit untouched vanilla directories on Windows (#1649, #1688).
// Version 5 read every vanilla file to stamp it on Windows.
// Version 6 stamps vanilla's files by their metadata on Windows too.
const VERSION: u32 = 6;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Dependencies {
    entries: Vec<(PathBuf, bool, String)>,
    #[serde(skip)]
    failed: bool,
    #[serde(skip)]
    tracking: bool,
    /// The vanilla root (absolute): its files are stamped without reading them.
    #[serde(skip)]
    vanilla: PathBuf,
}
impl Dependencies {
    fn record(&mut self, path: &Path, directory: bool) {
        if !self.tracking {
            return;
        }
        match dependency_signature(path, directory, &self.vanilla) {
            Ok(signature) => self.entries.push((path.into(), directory, signature)),
            Err(_) => self.failed = true,
        }
    }
    pub(super) fn file(&mut self, path: &Path) {
        self.record(path, false);
    }
    pub(super) fn directory(&mut self, path: &Path) {
        self.record(path, true);
    }
    fn valid(&self) -> bool {
        !self.failed
            && self.entries.iter().all(|(path, directory, signature)| {
                dependency_signature(path, *directory, &self.vanilla).is_ok_and(|s| s == *signature)
            })
    }
}

#[derive(Serialize, Deserialize)]
struct Marker {
    version: u32,
    vanilla: PathBuf,
    packs: String,
    dependencies: Dependencies,
}

fn finish(hash: Sha256) -> String {
    format!("{:x}", hash.finalize())
}
// Length framing keeps distinct paths/configurations unambiguous.
fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn content(hash: &mut Sha256, path: &Path) -> io::Result<()> {
    let mut file = fs::File::open(path)?;
    hash.update(file.metadata()?.len().to_le_bytes());
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(())
}
/// The largest zip hashed by its bytes: vanilla's limit on a server pack.
const WHOLE_ZIP: u64 = 250 << 20;
/// A zip up to `whole` by its bytes, which also catches an edit its checksums
/// don't show (a pack may leave them empty, nothing reads them). A larger one
/// would take minutes on every check, so by its stamp and its directory: what a
/// local pack changes when it is replaced, and the directory reads in a moment.
fn zip_content(hash: &mut Sha256, path: &Path, whole: u64) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    match Zip::open(path) {
        Ok(zip) if meta.len() > whole => {
            hash.update(meta.len().to_le_bytes());
            field(hash, format!("{:?}", meta.modified()?).as_bytes());
            hash.update(zip.directory());
            Ok(())
        }
        _ => content(hash, path),
    }
}
/// `path`'s stamp; `read` lets platforms without a change timestamp hash a file's bytes.
fn stamp(hash: &mut Sha256, path: &Path, read: bool) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    hash.update(meta.len().to_le_bytes());
    field(hash, format!("{:?}", meta.modified()?).as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The change time stands in for the bytes here.
        let _ = read;
        hash.update(meta.dev().to_le_bytes());
        hash.update(meta.ino().to_le_bytes());
        hash.update(meta.ctime().to_le_bytes());
        hash.update(meta.ctime_nsec().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        // Portable metadata lacks a change timestamp. Preserve content-based
        // validation for the packs' materialized files on those platforms;
        // vanilla's (the launcher's, tens of thousands) keep to the size and
        // the write and creation times: reading each costs minutes on Windows,
        // where every open is scanned by the antivirus.
        if read && meta.is_file() {
            return content(hash, path);
        }
        if let Ok(created) = meta.created() {
            field(hash, format!("{created:?}").as_bytes());
        }
    }
    Ok(())
}
fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}
fn dependency_signature(path: &Path, directory: bool, vanilla: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    let read = !(path.starts_with(vanilla) || absolute(path).is_ok_and(|p| p.starts_with(vanilla)));
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => field(&mut hash, b"missing"),
        Err(e) => return Err(e),
        Ok(meta) if directory && meta.is_dir() => {
            field(&mut hash, b"directory");
            stamp(&mut hash, path, read)?;
        }
        Ok(_) if directory => field(&mut hash, b"not-directory"),
        Ok(_) => stamp(&mut hash, path, read)?,
    }
    Ok(finish(hash))
}
fn tree(hash: &mut Sha256, path: &Path, pack: Option<&Pack>) -> io::Result<()> {
    tree_inner(hash, path, pack, &mut BTreeSet::new())
}

fn needs_content(path: &Path, pack: Option<&Pack>) -> bool {
    let Some(pack) = pack.filter(|_| cfg!(unix)) else {
        return true;
    };
    let Source::Dir(root) = &pack.source else {
        return true;
    };
    let relative = path
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    // Ordinary folder resources are live Unix file symlinks. Rewrites and
    // replacements at that path are visible without rebuilding. Merged JSON
    // and configuration bytes are materialized and need stamp validation.
    relative == "pack.mcmeta"
        || super::merge_kind(&relative).is_some()
        || pack.overlays.iter().any(|overlay| {
            path.strip_prefix(root.join(overlay.trim_end_matches('/')))
                .ok()
                .is_some_and(|name| {
                    super::merge_kind(&name.to_string_lossy().replace('\\', "/")).is_some()
                })
        })
}
fn tree_inner(
    hash: &mut Sha256,
    path: &Path,
    pack: Option<&Pack>,
    active: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        field(hash, b"link");
        field(hash, fs::read_link(path)?.as_os_str().as_encoded_bytes());
        if pack.is_none() {
            return Ok(());
        }
    }
    if path.is_dir() {
        let canonical = if pack.is_some() {
            let canonical = fs::canonicalize(path)?;
            if !active.insert(canonical.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pack symlink cycle",
                ));
            }
            Some(canonical)
        } else {
            None
        };
        field(hash, b"directory");
        for entry in entries(path)? {
            field(hash, entry.file_name().unwrap().as_encoded_bytes());
            tree_inner(hash, &entry, pack, active)?;
        }
        field(hash, b"end-directory");
        if let Some(canonical) = canonical {
            active.remove(&canonical);
        }
    } else {
        field(hash, b"file");
        if needs_content(path, pack) {
            #[cfg(test)]
            if pack.is_none() {
                return content(hash, path);
            }
            stamp(hash, path, true)?;
        }
    }
    Ok(())
}
#[cfg(test)]
fn output_signature(out: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    tree(&mut hash, out, None)?;
    Ok(finish(hash))
}
fn pack_signature(packs: &[Pack]) -> io::Result<String> {
    let mut hash = Sha256::new();
    field(&mut hash, &[u8::from(cfg!(unix))]);
    for pack in packs {
        if matches!(pack.source, Source::Dir(_))
            && pack.overlays.iter().any(|overlay| {
                Path::new(overlay).components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            })
        {
            // The builder historically accepts external overlay roots. Preserve
            // that behavior, but don't cache dependencies outside this pack tree.
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "external pack overlay",
            ));
        }
        field(&mut hash, pack.id.as_bytes());
        // Includes source kind/path, overlay order and compiled filter settings.
        field(&mut hash, format!("{:?}", pack.source).as_bytes());
        field(&mut hash, format!("{:?}", pack.overlays).as_bytes());
        field(&mut hash, format!("{:?}", pack.filter).as_bytes());
        match &pack.source {
            Source::Dir(path) => {
                field(&mut hash, absolute(path)?.as_os_str().as_encoded_bytes());
                tree(&mut hash, path, Some(pack))?;
            }
            Source::Zip(path) => {
                field(&mut hash, absolute(path)?.as_os_str().as_encoded_bytes());
                // By its bytes, not its stamp: a server pack found in the download cache is
                // marked used on every join (`HttpUtil.updateModificationTime`), and a new
                // time rebuilt the whole overlay each time.
                zip_content(&mut hash, path, WHOLE_ZIP)?;
            }
        }
    }
    Ok(finish(hash))
}

pub(super) fn marker_path(out: &Path) -> PathBuf {
    // Outside the resource tree: no resource-name collisions or mirror changes.
    let mut name = out.as_os_str().to_os_string();
    name.push(".overlay-cache.json");
    PathBuf::from(name)
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.into())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path))
    }
}

fn lock_path(out: &Path) -> PathBuf {
    marker_path(out).with_extension("lock")
}

pub(super) fn build(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // Keep this inode persistent: deleting it would let waiters and new callers
    // acquire different locks. Closing the file releases the advisory OS lock.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(out))?;
    lock.lock()?;
    let vanilla_key = absolute(vanilla)?;
    let marker_path = marker_path(out);
    let signature = pack_signature(packs);
    if let Ok(signature) = &signature
        && let Ok(bytes) = fs::read(&marker_path)
        && let Ok(mut marker) = serde_json::from_slice::<Marker>(&bytes)
        && marker.version == VERSION
        && marker.vanilla == vanilla_key
        && marker.packs == *signature
        && {
            marker.dependencies.vanilla = vanilla_key.clone();
            marker.dependencies.valid()
        }
        && out.is_dir()
    {
        return Ok(());
    }
    // An interrupted/failed rebuild must never leave a reusable marker.
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut dependencies = Dependencies {
        tracking: signature.is_ok(),
        vanilla: vanilla_key.clone(),
        ..Dependencies::default()
    };
    build_overlay_uncached(vanilla, packs, out, &mut dependencies)?;
    if let Ok(signature) = signature
        && dependencies.valid()
        && pack_signature(packs).is_ok_and(|s| s == signature)
    {
        let marker = Marker {
            version: VERSION,
            vanilla: vanilla_key,
            packs: signature,
            dependencies,
        };
        // PathBuf serialization requires UTF-8; unusual resource root names
        // can still build normally without a reusable marker.
        let Ok(bytes) = serde_json::to_vec(&marker) else {
            return Ok(());
        };
        // Publish only a complete marker, after all output writes succeeded.
        let temporary = marker_path.with_extension("tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, &marker_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{packs, text::Text};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "horizon-overlay-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        /// Waits for the filesystem's clock to move on, as it does between a build and a
        /// person's next edit. On Unix the cache compares stamps (length, mtime, ctime, inode),
        /// and Linux takes them from a coarse clock (a few milliseconds a tick): an edit in the
        /// tick of the build before kept them all, and the tests failed at random.
        fn next_tick(&self) {
            let probe = self.0.with_extension("tick");
            // New bytes each time: NTFS keeps the time of a write that changes nothing.
            let stamp = |n: u32| {
                fs::write(&probe, n.to_le_bytes()).unwrap();
                fs::metadata(&probe).unwrap().modified().unwrap()
            };
            let start = stamp(0);
            // A second at most: no clock is that coarse, and a test never hangs on one.
            for n in 1..1000 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                if stamp(n) != start {
                    break;
                }
            }
            let _ = fs::remove_file(&probe);
        }
        fn dir_pack(&self, name: &str) -> Pack {
            self.write(
                &format!("{name}/pack.mcmeta"),
                br#"{"pack":{"description":"test","pack_format":97}}"#,
            );
            packs::pack(name.into(), Text::empty(), Source::Dir(self.0.join(name))).unwrap()
        }
        fn rebuild_matches(&self, selected: &[Pack]) {
            let vanilla = self.0.join("vanilla");
            let cached = self.0.join("cached");
            packs::build_overlay(&vanilla, selected, &cached).unwrap();
            build_overlay_uncached(
                &vanilla,
                selected,
                &self.0.join("reference"),
                &mut Dependencies::default(),
            )
            .unwrap();
            assert_eq!(
                output_signature(&cached).unwrap(),
                output_signature(&self.0.join("reference")).unwrap()
            );
            self.next_tick();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Stored/deflated ZIP fixture using the production reader, no zip dependency.
    fn archive(files: &[(&str, Vec<u8>)], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in files {
            let data = if deflate {
                miniz_oxide::deflate::compress_to_vec(body, 6)
            } else {
                body.clone()
            };
            let offset = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            out.extend([0; 8]);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&data);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            central.extend([0; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        out
    }

    #[test]
    #[cfg(not(unix))]
    fn texture_only_packs_rebuild_incomplete_windows_v3_overlays() {
        let f = Fixture::new();
        let blockstate = "assets/minecraft/blockstates/oak_stairs.json";
        let model = "assets/minecraft/models/block/oak_stairs.json";
        let texture = "assets/minecraft/textures/block/stone.png";
        let font = "assets/minecraft/textures/font/ascii.png";
        f.write(&format!("vanilla/{blockstate}"), br#"{"variants":{}}"#);
        f.write(&format!("vanilla/{model}"), br#"{"elements":[]}"#);
        f.write(&format!("vanilla/{texture}"), b"vanilla texture");
        f.write(&format!("vanilla/{font}"), b"vanilla font");
        let folder = f.dir_pack("textures");
        f.write(&format!("textures/{texture}"), b"pack texture");
        f.write(
            "textures.zip",
            &archive(
                &[
                    (
                        "pack.mcmeta",
                        br#"{"pack":{"description":"test","pack_format":97}}"#.to_vec(),
                    ),
                    (texture, b"pack texture".to_vec()),
                ],
                true,
            ),
        );
        let zip = packs::pack(
            "zip".into(),
            Text::empty(),
            Source::Zip(f.0.join("textures.zip")),
        )
        .unwrap();
        let vanilla = f.0.join("vanilla");
        for pack in [folder, zip] {
            let out = f.0.join(format!("cached-{}", pack.id));
            let selected = std::slice::from_ref(&pack);
            packs::build_overlay(&vanilla, selected, &out).unwrap();
            let check = || {
                for name in [blockstate, model, font] {
                    assert_eq!(
                        fs::read(out.join(name)).unwrap(),
                        fs::read(vanilla.join(name)).unwrap()
                    );
                }
                assert_eq!(fs::read(out.join(texture)).unwrap(), b"pack texture");
            };
            check();
            // Before #1688, Windows silently omitted untouched directories.
            // A v3 marker still validates because it checks only source files.
            let marker_path = marker_path(&out);
            let mut marker: Marker =
                serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
            marker.version = 3;
            for dir in ["blockstates", "models", "textures/font"] {
                fs::remove_dir_all(out.join("assets/minecraft").join(dir)).unwrap();
            }
            fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
            assert!(marker.dependencies.valid());
            packs::build_overlay(&vanilla, selected, &out).unwrap();
            check();
            // The repaired overlay also remains complete on a cache hit.
            packs::build_overlay(&vanilla, selected, &out).unwrap();
            check();
        }
    }

    #[test]
    fn directory_inputs_config_and_owned_output_recovery() {
        let f = Fixture::new();
        let resource = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{resource}"), br#"{"v":"a"}"#);
        f.write("vanilla/assets/minecraft/textures/untouched/a", b"live");
        let mut low = f.dir_pack("low");
        let high = f.dir_pack("high");
        f.write(&format!("low/{resource}"), br#"{"p":"a"}"#);
        f.write(&format!("high/{resource}"), br#"{"p":"b"}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        let out = f.0.join("cached");
        let marker = marker_path(&out);
        // A hit keeps the actual merged file (and marker) in place.
        let time = fs::metadata(out.join(resource))
            .unwrap()
            .modified()
            .unwrap();
        let marker_time = fs::metadata(&marker).unwrap().modified().unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        assert_eq!(
            fs::metadata(out.join(resource))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
        assert_eq!(
            fs::metadata(&marker).unwrap().modified().unwrap(),
            marker_time
        );
        // Same-length rewrites cannot hide behind file size or preserved mtime.
        let path = f.0.join(format!("high/{resource}"));
        let original_time = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, br#"{"p":"c"}"#).unwrap();
        // Windows requires a writable handle to restore the timestamp.
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write(&format!("vanilla/{resource}"), br#"{"v":"b"}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write("vanilla/assets/minecraft/lang/new.json", b"new");
        f.rebuild_matches(&[low.clone(), high.clone()]);
        fs::remove_file(f.0.join("vanilla/assets/minecraft/lang/new.json")).unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.write("high/assets/minecraft/lang/added.json", br#"{"a":1}"#);
        f.rebuild_matches(&[low.clone(), high.clone()]);
        fs::remove_file(f.0.join("high/assets/minecraft/lang/added.json")).unwrap();
        f.rebuild_matches(&[low.clone(), high.clone()]);
        f.rebuild_matches(&[high.clone(), low.clone()]);
        f.rebuild_matches(std::slice::from_ref(&low));
        low.overlays.push("extra".into());
        f.write(&format!("low/extra/{resource}"), br#"{"p":"overlay"}"#);
        f.rebuild_matches(std::slice::from_ref(&low));
        low.filter = packs::filter_of(&serde_json::json!({"filter":{"block":[{"path":"lang"}]}}));
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::write(out.join(resource), b"corrupt").unwrap();
        // Published output is trusted; clearing the marker explicitly repairs
        // a cache tree changed outside the builder.
        packs::build_overlay(&f.0.join("vanilla"), std::slice::from_ref(&low), &out).unwrap();
        assert_eq!(fs::read(out.join(resource)).unwrap(), b"corrupt");
        let mut old: Marker = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        old.version = VERSION - 1;
        fs::write(&marker, serde_json::to_vec(&old).unwrap()).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_file(out.join(resource)).unwrap();
        fs::remove_file(&marker).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::write(&marker, b"corrupt").unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_file(&marker).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        fs::remove_dir_all(&out).unwrap();
        f.rebuild_matches(std::slice::from_ref(&low));
        #[cfg(unix)]
        {
            low.filter.clear();
            f.rebuild_matches(std::slice::from_ref(&low));
            // Unrelated vanilla content remains live through the directory symlink.
            let before = fs::read(&marker).unwrap();
            f.write("vanilla/assets/minecraft/textures/untouched/a", b"new live");
            f.write(
                "vanilla/assets/minecraft/textures/untouched/b",
                b"added live",
            );
            f.rebuild_matches(std::slice::from_ref(&low));
            assert_eq!(fs::read(&marker).unwrap(), before);
            assert_eq!(
                fs::read(out.join("assets/minecraft/textures/untouched/b")).unwrap(),
                b"added live"
            );
        }
        // Missing selected source disables reuse; the original directory builder
        // still tolerates it, and does not publish a marker for an incomplete input.
        fs::remove_dir_all(f.0.join("low")).unwrap();
        f.rebuild_matches(&[low]);
        assert!(!marker.exists());
    }

    #[test]
    fn a_large_zip_is_stamped_by_its_directory_and_time() {
        let f = Fixture::new();
        let path = f.0.join("large.zip");
        let at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
        let write = |name: &str, first: u8, modified| {
            let mut body = vec![0u8; 1 << 18];
            body[0] = first;
            fs::write(&path, archive(&[(name, body)], false)).unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(modified).unwrap();
        };
        let signature = |whole| {
            let mut hash = Sha256::new();
            zip_content(&mut hash, &path, whole).unwrap();
            finish(hash)
        };
        write("a", 0, at);
        let (bytes, directory) = (signature(u64::MAX), signature(0));
        // Above the limit the entries' bytes go unread; at or under it they count.
        write("a", 1, at);
        assert_eq!(signature(0), directory);
        assert_ne!(signature(u64::MAX), bytes);
        write("b", 0, at);
        assert_ne!(signature(0), directory);
        write("a", 0, at + std::time::Duration::from_secs(60));
        assert_ne!(signature(0), directory);
        assert_eq!(signature(u64::MAX), bytes);
    }

    #[test]
    fn zip_content_and_failed_build_invalidate() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let name = "assets/minecraft/textures/a";
        let files = |data: &[u8]| {
            vec![
                (
                    "pack.mcmeta",
                    br#"{"pack":{"description":"test","pack_format":97}}"#.to_vec(),
                ),
                (name, data.to_vec()),
            ]
        };
        f.write("pack.zip", &archive(&files(b"aaa"), false));
        let pack = packs::pack(
            "zip".into(),
            Text::empty(),
            Source::Zip(f.0.join("pack.zip")),
        )
        .unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        f.rebuild_matches(std::slice::from_ref(&pack));
        f.write("pack.zip", &archive(&files(b"bbb"), false));
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(f.0.join("cached").join(name)).unwrap(), b"bbb");
        // A central directory whose second name runs past the archive fails
        // the build (an unreadable entry alone only costs that file). No
        // reusable marker survives.
        let mut broken = archive(&files(b"ccc"), false);
        let central = broken
            .windows(4)
            .position(|s| s == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        let second = central + 46 + "pack.mcmeta".len();
        broken[second + 28..second + 30].copy_from_slice(&u16::MAX.to_le_bytes());
        f.write("pack.zip", &broken);
        let out = f.0.join("cached");
        assert!(
            packs::build_overlay(&f.0.join("vanilla"), std::slice::from_ref(&pack), &out).is_err()
        );
        assert!(!marker_path(&out).exists());
        f.write("pack.zip", &archive(&files(b"ddd"), true));
        f.rebuild_matches(std::slice::from_ref(&pack));
        fs::remove_file(f.0.join("pack.zip")).unwrap();
        assert!(
            packs::build_overlay(&f.0.join("vanilla"), std::slice::from_ref(&pack), &out).is_err()
        );
        assert!(!marker_path(&out).exists());
        assert!(
            packs::resource_root(&f.0.join("vanilla"), &[pack], &f.0.join("selection-cache"))
                .is_err()
        );
    }

    /// A ZIP whose bytes are the same keeps its overlay when only its time changes: a server
    /// pack found in the download cache is marked used on every join
    /// (`HttpUtil.updateModificationTime`), and rebuilding the overlay took most of a minute.
    #[test]
    fn a_zip_touched_but_unchanged_keeps_its_overlay() {
        let f = Fixture::new();
        f.write("vanilla/assets/minecraft/textures/b", b"vanilla");
        let files = vec![
            (
                "pack.mcmeta",
                br#"{"pack":{"description":"test","pack_format":97}}"#.to_vec(),
            ),
            ("assets/minecraft/textures/a", b"aaa".to_vec()),
        ];
        f.write("pack.zip", &archive(&files, false));
        let zip = f.0.join("pack.zip");
        let pack = packs::pack("zip".into(), Text::empty(), Source::Zip(zip.clone())).unwrap();
        let (vanilla, out) = (f.0.join("vanilla"), f.0.join("cached"));
        packs::build_overlay(&vanilla, std::slice::from_ref(&pack), &out).unwrap();
        // A rebuild empties the output first: this file survives a cache hit only.
        fs::write(out.join("kept"), b"").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        fs::OpenOptions::new()
            .append(true)
            .open(&zip)
            .unwrap()
            .set_modified(later)
            .unwrap();
        packs::build_overlay(&vanilla, std::slice::from_ref(&pack), &out).unwrap();
        assert!(out.join("kept").exists(), "the overlay was rebuilt");
    }

    #[cfg(unix)]
    #[test]
    fn unrelated_directory_pack_symlink_cycle_disables_reuse() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let pack = f.dir_pack("pack");
        f.write("pack/assets/minecraft/textures/a", b"pack");
        fs::create_dir_all(f.0.join("pack/docs")).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack"), f.0.join("pack/docs/back")).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert!(!marker_path(&f.0.join("cached")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_pack_bytes_stay_live_and_topology_invalidates() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let pack = f.dir_pack("pack");
        let name = "assets/minecraft/textures/a";
        f.write(&format!("pack/{name}"), b"aaa");
        f.rebuild_matches(std::slice::from_ref(&pack));
        let out = f.0.join("cached");
        let marker = marker_path(&out);
        let before = fs::metadata(&marker).unwrap().modified().unwrap();
        let path = f.0.join("pack").join(name);
        let original_time = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"bbb").unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"bbb");
        assert_eq!(fs::metadata(&marker).unwrap().modified().unwrap(), before);
        // Replacing a regular file at the same linked path is also already live.
        f.write("replacement", b"ccc");
        fs::rename(f.0.join("replacement"), &path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"ccc");
        assert_eq!(fs::metadata(&marker).unwrap().modified().unwrap(), before);
        // A changed source symlink identity is fingerprinted. Output is trusted
        // until its marker is cleared explicitly.
        fs::remove_file(&path).unwrap();
        f.write("other", b"ddd");
        std::os::unix::fs::symlink(f.0.join("other"), &path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), b"ddd");
        fs::remove_file(out.join(name)).unwrap();
        std::os::unix::fs::symlink(f.0.join("other"), out.join(name)).unwrap();
        fs::remove_file(&marker).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read_link(out.join(name)).unwrap(), path);
        fs::remove_file(&path).unwrap();
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert!(fs::symlink_metadata(out.join(name)).is_err());
    }

    #[test]
    fn normalized_and_external_overlay_roots_keep_merged_bytes_current() {
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{name}"), br#"{"v":1}"#);
        let mut pack = f.dir_pack("pack");
        f.write(&format!("pack/extra/{name}"), br#"{"p":1}"#);
        for overlay in ["extra/", "./extra", "extra/./", "extra//"] {
            pack.overlays = vec![overlay.into()];
            f.rebuild_matches(std::slice::from_ref(&pack));
            let marker = marker_path(&f.0.join("cached"));
            assert!(marker.exists());
            f.write(&format!("pack/extra/{name}"), br#"{"p":2}"#);
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert_eq!(
                fs::read(f.0.join("cached").join(name)).unwrap(),
                br#"{"p":2,"v":1}"#
            );
            f.write(&format!("pack/extra/{name}"), br#"{"p":1}"#);
        }
        f.write(&format!("outside/{name}"), br#"{"p":3}"#);
        for overlay in [
            "../outside".into(),
            f.0.join("outside").to_string_lossy().into_owned(),
        ] {
            pack.overlays = vec![overlay];
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert!(!marker_path(&f.0.join("cached")).exists());
            f.write(&format!("outside/{name}"), br#"{"p":4}"#);
            f.rebuild_matches(std::slice::from_ref(&pack));
            assert_eq!(
                fs::read(f.0.join("cached").join(name)).unwrap(),
                br#"{"p":4,"v":1}"#
            );
            f.write(&format!("outside/{name}"), br#"{"p":3}"#);
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_aliases_stay_live_and_retargeted_merges_invalidate() {
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write("vanilla-a/facts.json", b"a");
        f.write("vanilla-b/facts.json", b"b");
        f.write(&format!("vanilla-a/{name}"), br#"{"v":"a"}"#);
        f.write(&format!("vanilla-b/{name}"), br#"{"v":"b"}"#);
        let mut pack = f.dir_pack("pack-a");
        f.dir_pack("pack-b");
        f.write("pack-a/assets/minecraft/textures/a", b"a");
        f.write("pack-b/assets/minecraft/textures/a", b"b");
        f.write(&format!("pack-a/{name}"), br#"{"p":"a"}"#);
        f.write(&format!("pack-b/{name}"), br#"{"p":"b"}"#);
        let vanilla = f.0.join("vanilla");
        let source = f.0.join("pack");
        std::os::unix::fs::symlink(f.0.join("vanilla-a"), &vanilla).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack-a"), &source).unwrap();
        pack.source = Source::Dir(source.clone());
        f.rebuild_matches(std::slice::from_ref(&pack));
        fs::remove_file(&vanilla).unwrap();
        fs::remove_file(&source).unwrap();
        std::os::unix::fs::symlink(f.0.join("vanilla-b"), &vanilla).unwrap();
        std::os::unix::fs::symlink(f.0.join("pack-b"), &source).unwrap();
        let out = f.0.join("cached");
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"b");
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/a")).unwrap(),
            b"b"
        );
        f.rebuild_matches(std::slice::from_ref(&pack));
        assert_eq!(fs::read(out.join(name)).unwrap(), br#"{"p":"b","v":"b"}"#);
    }

    #[test]
    fn concurrent_selections_wait_and_never_publish_mismatched_markers() {
        use std::{sync::mpsc, time::Duration};
        let f = Fixture::new();
        let name = "assets/minecraft/lang/en_us.json";
        f.write(&format!("vanilla/{name}"), br#"{"v":1}"#);
        let a = f.dir_pack("a");
        let b = f.dir_pack("b");
        f.write(&format!("a/{name}"), br#"{"p":"a"}"#);
        f.write(&format!("b/{name}"), br#"{"p":"b"}"#);
        let out = f.0.join("cached");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&out))
            .unwrap();
        held.lock().unwrap();
        let (started, starts) = mpsc::channel();
        let (done, finishes) = mpsc::channel();
        std::thread::scope(|scope| {
            for pack in [&a, &b] {
                let started = started.clone();
                let done = done.clone();
                let vanilla = f.0.join("vanilla");
                let out = &out;
                scope.spawn(move || {
                    started.send(()).unwrap();
                    for _ in 0..20 {
                        packs::build_overlay(&vanilla, std::slice::from_ref(pack), out).unwrap();
                    }
                    done.send(()).unwrap();
                });
            }
            starts.recv().unwrap();
            starts.recv().unwrap();
            assert!(matches!(
                finishes.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            drop(held);
        });
        finishes.recv().unwrap();
        finishes.recv().unwrap();
        for pack in [&a, &b, &a, &b] {
            f.rebuild_matches(std::slice::from_ref(pack));
            let expected = if pack.id == "a" {
                br#"{"p":"a","v":1}"#
            } else {
                br#"{"p":"b","v":1}"#
            };
            assert_eq!(fs::read(out.join(name)).unwrap(), expected);
        }
        assert!(lock_path(&out).exists());
    }

    #[cfg(unix)]
    #[test]
    fn relative_sources_are_readable_through_cached_and_uncached_overlays() {
        let f = Fixture::new();
        f.write("vanilla/facts.json", b"[]");
        let mut pack = f.dir_pack("pack");
        let name = "assets/minecraft/textures/a";
        f.write(&format!("pack/{name}"), b"pack");
        let cwd = std::env::current_dir().unwrap();
        let mut relative = PathBuf::new();
        for component in cwd.components() {
            if matches!(component, std::path::Component::Normal(_)) {
                relative.push("..");
            }
        }
        relative.push(f.0.strip_prefix("/").unwrap());
        pack.source = Source::Dir(relative.join("pack"));
        let vanilla = relative.join("vanilla");
        let out = f.0.join("cached");
        let reference = f.0.join("reference");
        packs::build_overlay(&vanilla, std::slice::from_ref(&pack), &out).unwrap();
        build_overlay_uncached(
            &vanilla,
            std::slice::from_ref(&pack),
            &reference,
            &mut Dependencies::default(),
        )
        .unwrap();
        for root in [&out, &reference] {
            assert!(fs::read_link(root.join(name)).unwrap().is_absolute());
            assert_eq!(fs::read(root.join(name)).unwrap(), b"pack");
            assert_eq!(fs::read(root.join("facts.json")).unwrap(), b"[]");
        }
        packs::build_overlay(&vanilla, &[pack], &out).unwrap();
        assert_eq!(fs::read(out.join(name)).unwrap(), b"pack");
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"[]");
    }

    #[test]
    #[ignore = "requires prepared resources and real pack paths in environment"]
    fn benchmark_real_resource_root() {
        use std::{hint::black_box, time::Instant};
        let vanilla = PathBuf::from(std::env::var("HORIZON_OVERLAY_BENCH_VANILLA").unwrap());
        assert!(vanilla.join("horizon-resources.json").is_file());
        let paths: Vec<PathBuf> =
            serde_json::from_str(&std::env::var("HORIZON_OVERLAY_BENCH_PACKS").unwrap()).unwrap();
        // The original resource_root selection/cache-key path, with its original builder.
        fn original(vanilla: &Path, packs: &[Pack], cache: &Path) -> PathBuf {
            let key = packs
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
            });
            let out = cache.join(format!("{hash:016x}"));
            build_overlay_uncached(vanilla, packs, &out, &mut Dependencies::default()).unwrap();
            out
        }
        let median = |mut times: Vec<f64>| {
            times.sort_by(f64::total_cmp);
            times[times.len() / 2]
        };
        for path in paths {
            let f = Fixture::new();
            let pack = packs::pack(
                path.to_string_lossy().into_owned(),
                Text::empty(),
                Source::Zip(path.clone()),
            )
            .unwrap();
            let selected = std::slice::from_ref(&pack);
            let reference_cache = f.0.join("reference");
            let cached_cache = f.0.join("cached");
            fs::create_dir_all(&reference_cache).unwrap();
            let reference = original(&vanilla, selected, &reference_cache);
            let out = packs::resource_root(&vanilla, selected, &cached_cache).unwrap();
            assert_ne!(out, vanilla, "resource_root must not fall back to vanilla");
            assert!(marker_path(&out).exists());
            let expected = output_signature(&reference).unwrap();
            assert_eq!(output_signature(&out).unwrap(), expected);
            for _ in 0..2 {
                original(&vanilla, selected, &reference_cache);
                assert_eq!(
                    packs::resource_root(&vanilla, selected, &cached_cache).unwrap(),
                    out
                );
            }
            let mut baseline = Vec::new();
            let mut cold = Vec::new();
            let mut hit = Vec::new();
            for iteration in 0..11 {
                for operation in if iteration % 2 == 0 {
                    [0, 1, 2]
                } else {
                    [1, 2, 0]
                } {
                    if operation == 1 {
                        fs::remove_file(marker_path(&out)).unwrap();
                    }
                    let started = Instant::now();
                    let result = if operation == 0 {
                        original(
                            black_box(&vanilla),
                            black_box(selected),
                            black_box(&reference_cache),
                        )
                    } else {
                        packs::resource_root(
                            black_box(&vanilla),
                            black_box(selected),
                            black_box(&cached_cache),
                        )
                        .unwrap()
                    };
                    black_box(&result);
                    let ms = started.elapsed().as_secs_f64() * 1000.0;
                    match operation {
                        0 => baseline.push(ms),
                        1 => cold.push(ms),
                        _ => hit.push(ms),
                    }
                    if operation != 0 {
                        assert_eq!(result, out);
                    }
                }
                // Verify full materialized bytes and symlink identities outside timing.
                assert_eq!(output_signature(&out).unwrap(), expected);
                assert_eq!(output_signature(&reference).unwrap(), expected);
            }
            let baseline = median(baseline);
            let cold = median(cold);
            let hit = median(hit);
            println!(
                "real pack={} ZIP={} bytes: original={baseline:.3} ms cold={cold:.3} ms hit={hit:.3} ms speedup={:.2}x saved={:.3} ms cold_overhead={:.2}% (11 medians, 2 warmups)",
                path.display(),
                fs::metadata(&path).unwrap().len(),
                baseline / hit,
                baseline - hit,
                (cold / baseline - 1.0) * 100.0
            );
        }
    }
}
