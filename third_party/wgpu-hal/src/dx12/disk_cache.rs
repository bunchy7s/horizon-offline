//! Horizon: compiled shaders kept on disk between runs.
//!
//! FXC takes about 2.6 s to compile Horizon's ~65 pipelines on every start
//! (DXC about 1 s), while the in-memory [`super::ShaderCache`] only helps
//! within one device. The application names one file with [`set_file`]. A
//! shader compiled once is appended there, and the next run reads its bytecode
//! back as [`super::CompiledShader::Precompiled`] instead of compiling it.
//!
//! The key is a 128-bit FNV-1a hash of the compiler, the shader model, the
//! stage, the entry point and the HLSL source naga wrote. Any change to
//! naga's output, the shader, or the compiler choice is a different key, so
//! the file never needs explicit invalidation. Each record carries a checksum
//! of its bytes; reading stops at the first record that is short or does not
//! match (a run killed mid-write, two games writing at once), and the next
//! append starts there. A file past [`MAX_BYTES`] is started over.
//!
//! File: `MAGIC`, then records of `key: u128 LE`, `len: u32 LE`,
//! `check: u64 LE` (FNV-1a 64 of the bytes), `bytes`.

use alloc::vec::Vec;
use std::{
    fs,
    io::{Seek as _, SeekFrom, Write as _},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use hashbrown::HashMap;

const MAGIC: &[u8; 8] = b"HZDXSC1\n";
const HEADER: usize = 16 + 4 + 8;
/// Past this the file is started over: Horizon's shaders take about 1 MB.
pub const MAX_BYTES: u64 = 64 << 20;

static FILE: OnceLock<PathBuf> = OnceLock::new();
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

/// Keep compiled shaders in `path` for the rest of the run. Only the first
/// call counts; without one nothing is kept on disk.
pub fn set_file(path: PathBuf) {
    let _ = FILE.set(path);
}

pub(super) struct Cache {
    path: PathBuf,
    entries: HashMap<u128, Vec<u8>>,
    /// Where the next record goes: the end of the last good record.
    end: u64,
}

fn cache() -> Option<&'static Mutex<Cache>> {
    let path = FILE.get()?;
    Some(CACHE.get_or_init(|| Mutex::new(Cache::load(path.clone()))))
}

/// The bytecode compiled for `key` in an earlier run.
pub(super) fn get(key: u128) -> Option<Vec<u8>> {
    let cache = cache()?.lock().ok()?;
    cache.entries.get(&key).cloned()
}

/// Keep `bytes` for `key` for the next runs.
pub(super) fn put(key: u128, bytes: &[u8]) {
    let Some(cache) = cache() else { return };
    let Ok(mut cache) = cache.lock() else { return };
    if let Err(error) = cache.append(key, bytes) {
        log::warn!("DX12 shader cache {}: {error}", cache.path.display());
    }
}

/// The key of one compile.
pub(super) fn key(
    compiler: &str,
    shader_model: &str,
    stage: &str,
    entry: &str,
    source: &str,
) -> u128 {
    let mut hash = Fnv128::new();
    for part in [compiler, shader_model, stage, entry, source] {
        hash.write(&(part.len() as u64).to_le_bytes());
        hash.write(part.as_bytes());
    }
    hash.0
}

impl Cache {
    fn load(path: PathBuf) -> Self {
        let bytes = match fs::metadata(&path) {
            Ok(meta) if meta.len() <= MAX_BYTES => fs::read(&path).unwrap_or_default(),
            _ => Vec::new(),
        };
        let (entries, end) = parse(&bytes);
        Self { path, entries, end }
    }

    fn append(&mut self, key: u128, bytes: &[u8]) -> std::io::Result<()> {
        if self.entries.contains_key(&key) {
            return Ok(());
        }
        let len = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
        if self.end + (HEADER + bytes.len()) as u64 > MAX_BYTES {
            self.end = 0;
            self.entries.clear();
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.path)?;
        let mut record = Vec::with_capacity(MAGIC.len() + HEADER + bytes.len());
        if self.end == 0 {
            record.extend_from_slice(MAGIC);
        }
        record.extend_from_slice(&key.to_le_bytes());
        record.extend_from_slice(&len.to_le_bytes());
        record.extend_from_slice(&fnv64(bytes).to_le_bytes());
        record.extend_from_slice(bytes);
        // Drop whatever follows the last good record, then write after it.
        file.set_len(self.end)?;
        file.seek(SeekFrom::Start(self.end))?;
        file.write_all(&record)?;
        self.end += record.len() as u64;
        self.entries.insert(key, bytes.to_vec());
        Ok(())
    }
}

/// The records of a cache file and where its good part ends.
fn parse(bytes: &[u8]) -> (HashMap<u128, Vec<u8>>, u64) {
    let mut entries = HashMap::new();
    if bytes.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return (entries, 0);
    }
    let mut at = MAGIC.len();
    while let Some(header) = bytes.get(at..at + HEADER) {
        let key = u128::from_le_bytes(header[..16].try_into().unwrap());
        let len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
        let check = u64::from_le_bytes(header[20..28].try_into().unwrap());
        let Some(body) = bytes.get(at + HEADER..at + HEADER + len) else {
            break;
        };
        if fnv64(body) != check {
            break;
        }
        entries.insert(key, body.to_vec());
        at += HEADER + len;
    }
    (entries, at as u64)
}

struct Fnv128(u128);

impl Fnv128 {
    fn new() -> Self {
        Self(0x6c62272e07bb014262b821756295c58d)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u128::from(b);
            self.0 = self.0.wrapping_mul(0x0000000001000000000000000000013b);
        }
    }
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hz-dx12-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("cache/dx12-shaders.bin")
    }

    #[test]
    fn a_shader_kept_in_one_run_is_read_back_in_the_next() {
        let path = scratch("next");
        let vs = key(
            "fxc",
            "5_1",
            "vs",
            "main",
            "float4 main() : SV_Position { return 0; }",
        );
        let fs_ = key(
            "fxc",
            "5_1",
            "ps",
            "main",
            "float4 main() : SV_Target { return 1; }",
        );
        let mut run = Cache::load(path.clone());
        run.append(vs, b"DXBC vertex").unwrap();
        run.append(fs_, b"DXBC fragment").unwrap();
        let next = Cache::load(path.clone());
        assert_eq!(
            next.entries.get(&vs).map(Vec::as_slice),
            Some(&b"DXBC vertex"[..])
        );
        assert_eq!(
            next.entries.get(&fs_).map(Vec::as_slice),
            Some(&b"DXBC fragment"[..])
        );
        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn each_part_of_the_compile_changes_the_key() {
        let base = key("fxc", "5_1", "vs", "main", "src");
        assert_ne!(base, key("dxc", "5_1", "vs", "main", "src"));
        assert_ne!(base, key("fxc", "6_0", "vs", "main", "src"));
        assert_ne!(base, key("fxc", "5_1", "ps", "main", "src"));
        assert_ne!(base, key("fxc", "5_1", "vs", "main2", "src"));
        assert_ne!(base, key("fxc", "5_1", "vs", "main", "src "));
        // Lengths are hashed, so moving a byte between parts changes the key.
        assert_ne!(key("ab", "c", "", "", ""), key("a", "bc", "", "", ""));
    }

    #[test]
    fn a_torn_record_is_dropped_and_the_next_one_written_over_it() {
        let path = scratch("torn");
        let a = key("fxc", "5_1", "vs", "a", "a");
        let b = key("fxc", "5_1", "vs", "b", "b");
        let c = key("fxc", "5_1", "vs", "c", "c");
        let mut run = Cache::load(path.clone());
        run.append(a, b"first").unwrap();
        run.append(b, b"second").unwrap();
        // The run is killed while it writes `b`: its last bytes never land.
        let full = fs::read(&path).unwrap();
        fs::write(&path, &full[..full.len() - 3]).unwrap();
        let mut next = Cache::load(path.clone());
        assert_eq!(next.entries.len(), 1);
        assert!(next.entries.contains_key(&a));
        next.append(c, b"third").unwrap();
        let last = Cache::load(path.clone());
        assert_eq!(last.entries.len(), 2);
        assert_eq!(last.entries.get(&c).map(Vec::as_slice), Some(&b"third"[..]));

        // A flipped byte fails the checksum the same way.
        let mut bytes = fs::read(&path).unwrap();
        let at = bytes.len() - 1;
        bytes[at] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert_eq!(Cache::load(path.clone()).entries.len(), 1);
        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_file_that_is_not_a_cache_is_started_over() {
        let path = scratch("foreign");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not a shader cache at all").unwrap();
        let mut run = Cache::load(path.clone());
        assert!(run.entries.is_empty());
        let k = key("dxc", "6_0", "cs", "main", "x");
        run.append(k, b"DXIL").unwrap();
        let next = Cache::load(path.clone());
        assert_eq!(next.entries.get(&k).map(Vec::as_slice), Some(&b"DXIL"[..]));
        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
