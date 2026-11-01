//! Downloads that reach the game folder only when their checksum matches the source's.
use super::{safe_name, FileRef};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Hosts a file may come from: Modrinth's and CurseForge's file servers, and the release assets
/// of Horizon's registry.
const HOSTS: [&str; 5] = [
    "https://cdn.modrinth.com/",
    "https://edge.forgecdn.net/",
    "https://mediafilez.forgecdn.net/",
    horizon_mods::index::FILE_HOST,
    // The registry's files at their address before the move to horizon-rust (GitHub redirects it).
    "https://github.com/ludovic111/horizon-mods/releases/download/",
];
/// Bigger than any pack these sources serve (Modrinth caps uploads at 500 MiB).
const MAX: u64 = 1 << 30;

/// The HTTP agent the sources share, reaching `url` through the player's proxy when one is set
/// (`proxy`). Modrinth asks for a user agent that names the project.
///
/// It is kept between downloads: a fresh agent starts a new connection pool, so one per file would
/// be a handshake per file, and a modpack brings hundreds.
pub(super) fn agent(url: &str) -> ureq::Agent {
    let spec = crate::proxy::spec_for(url);
    horizon_port_pack::net::keep("browser", spec.as_deref(), |proxy| {
        let builder = ureq::AgentBuilder::new()
            .user_agent(&format!(
                "horizon-rust/horizon horizon-launcher/{} (github.com/horizon-rust/horizon)",
                crate::backend::VERSION
            ))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60));
        match proxy {
            Some(proxy) => builder.proxy(proxy).build(),
            None => builder.build(),
        }
    })
}

/// Why a file can't be downloaded at all, before any byte is read.
pub fn refuse(file: &FileRef) -> Option<String> {
    if !HOSTS.iter().any(|h| file.url.starts_with(h)) {
        return Some(format!(
            "{} isn't on Modrinth's, CurseForge's or Horizon's file servers.",
            file.url
        ));
    }
    refuse_file(file)
}

/// [`refuse`] without the host: the name, the checksum and the size.
fn refuse_file(file: &FileRef) -> Option<String> {
    if safe_name(&file.name).is_none() {
        return Some(format!("{:?} isn't a plain file name.", file.name));
    }
    if file.checksum().is_none() {
        return Some("Its source gives no checksum, so it can't be checked.".into());
    }
    if file.size > MAX {
        return Some("It is larger than any pack should be.".into());
    }
    None
}

/// Hashes as they are written: the strongest one the file lists.
enum Hasher {
    Sha512(Sha512),
    Sha256(Sha256),
    Sha1(Sha1),
}

impl Hasher {
    fn for_file(file: &FileRef) -> Self {
        match file.checksum() {
            Some(("SHA-512", _)) => Hasher::Sha512(Sha512::new()),
            Some(("SHA-256", _)) => Hasher::Sha256(Sha256::new()),
            _ => Hasher::Sha1(Sha1::new()),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Sha512(h) => h.update(bytes),
            Hasher::Sha256(h) => h.update(bytes),
            Hasher::Sha1(h) => h.update(bytes),
        }
    }
    fn hex(self) -> String {
        let bytes: Vec<u8> = match self {
            Hasher::Sha512(h) => h.finalize().to_vec(),
            Hasher::Sha256(h) => h.finalize().to_vec(),
            Hasher::Sha1(h) => h.finalize().to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Copies `reader` into `<folder>/<file.name>`, hashing on the way, and keeps it only when the
/// size and checksum match: until then it is `.<name>.part`, removed on any failure.
/// `progress` gets the fraction done.
pub fn save(
    file: &FileRef,
    reader: impl Read,
    folder: &Path,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    if let Some(why) = refuse(file) {
        return Err(why);
    }
    save_from_anywhere(file, reader, folder, progress)
}

/// [`save`] for a file that comes from no host: a local test registry's.
pub(super) fn save_from_anywhere(
    file: &FileRef,
    mut reader: impl Read,
    folder: &Path,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    if let Some(why) = refuse_file(file) {
        return Err(why);
    }
    let (algo, want) = file.checksum().expect("refuse checks for one");
    fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let target = folder.join(&file.name);
    let part = folder.join(format!(".{}.part", file.name));
    let result = (|| {
        let mut out = fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut hasher = Hasher::for_file(file);
        let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > MAX || (file.size > 0 && done > file.size) {
                return Err("The download is larger than its source says.".to_string());
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            progress(done as f32 / file.size.max(1) as f32);
        }
        if file.size > 0 && done != file.size {
            return Err(format!("Got {done} bytes of {}.", file.size));
        }
        let got = hasher.hex();
        if !got.eq_ignore_ascii_case(want) {
            return Err(format!(
                "The {algo} checksum doesn't match: the file isn't the one its source lists. \
                 Nothing was installed."
            ));
        }
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result.map(|()| target)
}

/// Downloads `file` into `folder`, checked as [`save`] does.
pub fn fetch(file: &FileRef, folder: &Path, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    if let Some(why) = refuse(file) {
        return Err(why);
    }
    let response = agent(&file.url)
        .get(&file.url)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("{} answered {code}.", file.url),
            ureq::Error::Transport(t) => format!("No connection: {t}"),
        })?;
    save(file, response.into_reader(), folder, progress)
}
